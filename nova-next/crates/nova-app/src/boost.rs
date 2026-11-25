//! (1.8.16) Windows Cache Cleaner & RAM Booster: safe, fast, zero-bloat system cleanup on demand.
//! - Cleans strictly temporary user cache directories (%TEMP%, CrashDumps, D3DSCache, INetCache,
//!   best-effort C:\Windows\Temp) while skipping reparse points, locked files, and recent files (<30s).
//! - Trims bloated process working sets via Win32 `K32EmptyWorkingSet` across accessible user
//!   processes + trims Nova Island's own process.
//! - Flushes the Windows DNS resolver cache via `dnsapi.dll!DnsFlushResolverCache`.
//! Zero background threads or timers: runs purely on user demand from the island's Boost sheet.
use std::{
    fs,
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

use serde_json::{json, Value};
use windows::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::{
        LibraryLoader::{GetProcAddress, LoadLibraryW},
        SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX},
        Threading::{GetCurrentProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA},
    },
};

const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
const MAX_SCAN_ENTRIES: usize = 6000;
const MAX_DEPTH: usize = 5;
const MIN_AGE_SECS: u64 = 30;

windows::core::link!("kernel32.dll" "system" fn K32EnumProcesses(lpidprocess: *mut u32, cb: u32, lpcbneeded: *mut u32) -> i32);
windows::core::link!("kernel32.dll" "system" fn K32EmptyWorkingSet(hprocess: HANDLE) -> i32);

#[derive(Clone, Copy, Debug)]
pub struct MemInfo {
    pub pct: u32,
    pub used_bytes: u64,
    pub total_bytes: u64,
    pub avail_bytes: u64,
}

pub fn mem_snapshot() -> MemInfo {
    let mut ms = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    unsafe {
        if GlobalMemoryStatusEx(&mut ms).is_ok() && ms.ullTotalPhys > 0 {
            let used = ms.ullTotalPhys.saturating_sub(ms.ullAvailPhys);
            return MemInfo {
                pct: ms.dwMemoryLoad,
                used_bytes: used,
                total_bytes: ms.ullTotalPhys,
                avail_bytes: ms.ullAvailPhys,
            };
        }
    }
    MemInfo { pct: 0, used_bytes: 0, total_bytes: 0, avail_bytes: 0 }
}

fn cache_roots() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut push_unique = |p: PathBuf| {
        if p.is_dir() && !dirs.iter().any(|d: &PathBuf| d.as_os_str().eq_ignore_ascii_case(p.as_os_str())) {
            dirs.push(p);
        }
    };
    push_unique(std::env::temp_dir());
    if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        push_unique(local.join("Temp"));
        push_unique(local.join("CrashDumps"));
        push_unique(local.join("D3DSCache"));
        push_unique(local.join("Microsoft").join("Windows").join("INetCache").join("IE"));
    }
    push_unique(PathBuf::from(r"C:\Windows\Temp"));
    dirs
}

fn is_reparse(md: &fs::Metadata) -> bool {
    (md.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0
}

fn is_old_enough(md: &fs::Metadata, now: SystemTime, min_age: Duration) -> bool {
    if min_age.is_zero() {
        return true;
    }
    md.modified()
        .ok()
        .and_then(|m| now.duration_since(m).ok())
        .is_some_and(|age| age >= min_age)
}

fn scan_dir(
    dir: &Path,
    depth: usize,
    now: SystemTime,
    min_age: Duration,
    deadline: Instant,
    seen: &mut usize,
    bytes: &mut u64,
    files: &mut u64,
) {
    if depth > MAX_DEPTH || *seen >= MAX_SCAN_ENTRIES || Instant::now() >= deadline {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut subdirs = Vec::new();
    for entry in rd.flatten() {
        if *seen >= MAX_SCAN_ENTRIES || Instant::now() >= deadline {
            return;
        }
        *seen += 1;
        let path = entry.path();
        let Ok(md) = fs::symlink_metadata(&path) else { continue };
        if is_reparse(&md) {
            continue;
        }
        if md.is_dir() {
            subdirs.push(path);
        } else if md.is_file() && is_old_enough(&md, now, min_age) {
            *bytes = bytes.saturating_add(md.len());
            *files += 1;
        }
    }
    for sub in subdirs {
        if *seen >= MAX_SCAN_ENTRIES || Instant::now() >= deadline {
            return;
        }
        scan_dir(&sub, depth + 1, now, min_age, deadline, seen, bytes, files);
    }
}

fn clean_dir_inner(
    dir: &Path,
    depth: usize,
    now: SystemTime,
    min_age: Duration,
    deadline: Instant,
    seen: &mut usize,
    freed: &mut u64,
    files: &mut u64,
) {
    if depth > MAX_DEPTH || *seen >= MAX_SCAN_ENTRIES || Instant::now() >= deadline {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut subdirs = Vec::new();
    for entry in rd.flatten() {
        if *seen >= MAX_SCAN_ENTRIES || Instant::now() >= deadline {
            break;
        }
        *seen += 1;
        let path = entry.path();
        let Ok(md) = fs::symlink_metadata(&path) else { continue };
        if is_reparse(&md) {
            continue;
        }
        if md.is_dir() {
            subdirs.push((path, is_old_enough(&md, now, min_age)));
        } else if md.is_file() && is_old_enough(&md, now, min_age) {
            let len = md.len();
            if fs::remove_file(&path).is_ok() {
                *freed = freed.saturating_add(len);
                *files += 1;
            }
        }
    }
    for (sub, old) in subdirs {
        if *seen >= MAX_SCAN_ENTRIES || Instant::now() >= deadline {
            break;
        }
        clean_dir_inner(&sub, depth + 1, now, min_age, deadline, seen, freed, files);
        if old {
            let _ = fs::remove_dir(&sub);
        }
    }
}

/// Cleans stale files inside `dir` without ever deleting `dir` itself.
pub fn clean_dir_safe(dir: &Path, min_age: Duration) -> (u64, u64) {
    let (mut seen, mut freed, mut files) = (0usize, 0u64, 0u64);
    let deadline = Instant::now() + Duration::from_millis(1200);
    clean_dir_inner(dir, 0, SystemTime::now(), min_age, deadline, &mut seen, &mut freed, &mut files);
    (freed, files)
}

fn trim_working_sets() -> u32 {
    let mut pids = vec![0u32; 2048];
    let mut needed = 0u32;
    let ok = unsafe { K32EnumProcesses(pids.as_mut_ptr(), (pids.len() * std::mem::size_of::<u32>()) as u32, &mut needed) };
    let mut trimmed = 0u32;
    if ok != 0 {
        let count = (needed as usize / std::mem::size_of::<u32>()).min(pids.len());
        let my_pid = std::process::id();
        for &pid in &pids[..count] {
            if pid <= 4 || pid == my_pid {
                continue;
            }
            unsafe {
                if let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SET_QUOTA, false, pid) {
                    if K32EmptyWorkingSet(h) != 0 {
                        trimmed += 1;
                    }
                    let _ = CloseHandle(h);
                }
            }
        }
    }
    unsafe {
        if K32EmptyWorkingSet(GetCurrentProcess()) != 0 {
            trimmed += 1;
        }
    }
    trimmed
}

fn flush_dns_cache() -> bool {
    unsafe {
        let Ok(h) = LoadLibraryW(windows::core::w!("dnsapi.dll")) else { return false };
        let Some(p) = GetProcAddress(h, windows::core::s!("DnsFlushResolverCache")) else { return false };
        let f: extern "system" fn() -> i32 = std::mem::transmute(p);
        f() != 0
    }
}

fn mb_round1(bytes: u64) -> f64 {
    ((bytes as f64 / (1024.0 * 1024.0)) * 10.0).round() / 10.0
}

fn mb_u64(bytes: u64) -> u64 {
    (bytes as f64 / (1024.0 * 1024.0)).round() as u64
}

/// Fast on-demand snapshot of live RAM usage and cleanable Windows temp cache (time-budgeted <=120ms).
pub fn stats() -> Value {
    let m = mem_snapshot();
    let now = SystemTime::now();
    let min_age = Duration::from_secs(MIN_AGE_SECS);
    let deadline = Instant::now() + Duration::from_millis(120);
    let (mut seen, mut cache_bytes, mut cache_files) = (0usize, 0u64, 0u64);
    for dir in cache_roots() {
        scan_dir(&dir, 0, now, min_age, deadline, &mut seen, &mut cache_bytes, &mut cache_files);
    }
    json!({
        "ramPct": m.pct,
        "usedMb": mb_u64(m.used_bytes),
        "availMb": mb_u64(m.avail_bytes),
        "totalMb": mb_u64(m.total_bytes),
        "cacheMb": mb_round1(cache_bytes),
        "cacheFiles": cache_files,
    })
}

/// Executes Windows temp/cache cleanup, DNS cache flush, and RAM working-set optimization.
pub fn run() -> Value {
    let t0 = Instant::now();
    let before = mem_snapshot();
    let min_age = Duration::from_secs(MIN_AGE_SECS);
    let (mut cache_bytes, mut files_cleaned) = (0u64, 0u64);
    for dir in cache_roots() {
        let (b, f) = clean_dir_safe(&dir, min_age);
        cache_bytes = cache_bytes.saturating_add(b);
        files_cleaned = files_cleaned.saturating_add(f);
    }
    let dns_flushed = flush_dns_cache();
    let procs_trimmed = trim_working_sets();
    // Brief settle so Windows updates GlobalMemoryStatusEx counters cleanly
    std::thread::sleep(Duration::from_millis(80));
    let after = mem_snapshot();
    let ram_freed_bytes = before.used_bytes.saturating_sub(after.used_bytes);
    let elapsed_ms = t0.elapsed().as_millis() as u64;
    nova_core::log::info(&format!(
        "[boost] ram={}MB ({}%->{}%), cache={}MB ({} files), procs={}, dns={}, {}ms",
        mb_u64(ram_freed_bytes),
        before.pct,
        after.pct,
        mb_round1(cache_bytes),
        files_cleaned,
        procs_trimmed,
        dns_flushed,
        elapsed_ms
    ));
    json!({
        "beforePct": before.pct,
        "afterPct": after.pct,
        "beforeUsedMb": mb_u64(before.used_bytes),
        "afterUsedMb": mb_u64(after.used_bytes),
        "availMb": mb_u64(after.avail_bytes),
        "totalMb": mb_u64(after.total_bytes),
        "ramFreedMb": mb_u64(ram_freed_bytes),
        "cacheFreedMb": mb_round1(cache_bytes),
        "filesCleaned": files_cleaned,
        "procsTrimmed": procs_trimmed,
        "dnsFlushed": dns_flushed,
        "elapsedMs": elapsed_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mem_snapshot_reports_valid_physical_ram() {
        let m = mem_snapshot();
        assert!(m.total_bytes > 512 * 1024 * 1024, "total RAM should exceed 512 MB");
        assert!(m.pct <= 100);
        assert!(m.used_bytes <= m.total_bytes);
    }

    #[test]
    fn clean_dir_safe_deletes_stale_files_and_preserves_root_and_fresh_files() {
        let root = std::env::temp_dir().join(format!("nova-boost-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let sub = root.join("nested");
        fs::create_dir_all(&sub).unwrap();
        let f1 = root.join("a.tmp");
        let f2 = sub.join("b.tmp");
        fs::write(&f1, vec![0xAA; 4096]).unwrap();
        fs::write(&f2, vec![0xBB; 2048]).unwrap();

        // With 60s min_age, brand-new files are preserved
        let (freed_fresh, count_fresh) = clean_dir_safe(&root, Duration::from_secs(60));
        assert_eq!(freed_fresh, 0);
        assert_eq!(count_fresh, 0);
        assert!(f1.exists() && f2.exists());

        // With 0s min_age, all files + empty subdirs are cleaned while root stays intact
        let (freed, count) = clean_dir_safe(&root, Duration::ZERO);
        assert_eq!(freed, 6144);
        assert_eq!(count, 2);
        assert!(root.is_dir(), "root temp folder itself must never be deleted");
        assert!(!f1.exists() && !f2.exists() && !sub.exists());

        let _ = fs::remove_dir_all(&root);
    }
}
