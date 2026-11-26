//! (1.8.17) High-effectiveness Windows Cache Cleaner & Real-Time RAM Booster.
//! - Deep, safe multi-category cache cleanup: Windows Temp, CrashDumps, WER, INetCache, stale
//!   Explorer thumbcaches, GPU shader caches (D3D/AMD/NVIDIA/Intel), Chromium/Firefox disposable
//!   caches (Cache_Data, Code Cache, GPUCache, GrShaderCache — never cookies/logins/history),
//!   and heavy desktop app caches (Discord, VS Code, Steam, Spotify).
//! - Largest-first progressive process working-set trimming (`SetProcessWorkingSetSizeEx` +
//!   `K32EmptyWorkingSet`) streaming real `GlobalMemoryStatusEx` snapshots on every batch so the
//!   island's circular RAM ring drains in live synchronization with real OS memory reclamation.
//! - DNS resolver cache flush (`DnsFlushResolverCache`), heap compaction (`HeapCompact`), and
//!   kernel Standby/Modified memory list purge + memory combining (`NtSetSystemInformation`) when
//!   running with `SeProfileSingleProcessPrivilege`.
//! - Zero idle threads or timers: runs strictly on user demand.
use std::{
    collections::VecDeque,
    ffi::c_void,
    fs,
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Mutex,
    },
    time::{Duration, Instant, SystemTime},
};

use serde_json::{json, Value};
use windows::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::{
        LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW},
        SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX},
        Threading::{GetCurrentProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA},
    },
};

const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
const MAX_SCAN_ENTRIES: usize = 28_000;
const MAX_CLEAN_ENTRIES: usize = 65_000;
const MAX_DEPTH: usize = 6;
const TOKEN_ADJUST_PRIVILEGES: u32 = 0x0020;
const TOKEN_QUERY: u32 = 0x0008;
const SE_PRIVILEGE_ENABLED: u32 = 0x0000_0002;
const SYSTEM_MEMORY_LIST_INFORMATION: u32 = 80;
const SYSTEM_COMBINE_PHYSICAL_MEMORY_INFORMATION: u32 = 130;

#[repr(C)]
#[derive(Default)]
struct ProcessMemoryCounters {
    cb: u32,
    page_fault_count: u32,
    peak_working_set_size: usize,
    working_set_size: usize,
    quota_peak_paged_pool_usage: usize,
    quota_paged_pool_usage: usize,
    quota_peak_non_paged_pool_usage: usize,
    quota_non_paged_pool_usage: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Luid {
    low_part: u32,
    high_part: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct LuidAndAttributes {
    luid: Luid,
    attributes: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct TokenPrivileges {
    privilege_count: u32,
    privileges: [LuidAndAttributes; 1],
}

#[repr(C)]
#[derive(Default)]
struct MemoryCombineInformationEx {
    handle: usize,
    pages_combined: usize,
    flags: u32,
    _pad: u32,
}

windows::core::link!("kernel32.dll" "system" fn K32EnumProcesses(lpidprocess: *mut u32, cb: u32, lpcbneeded: *mut u32) -> i32);
windows::core::link!("kernel32.dll" "system" fn K32EmptyWorkingSet(hprocess: HANDLE) -> i32);
windows::core::link!("kernel32.dll" "system" fn K32GetProcessMemoryInfo(hprocess: HANDLE, ppsmemcounters: *mut ProcessMemoryCounters, cb: u32) -> i32);
windows::core::link!("kernel32.dll" "system" fn SetProcessWorkingSetSizeEx(hprocess: HANDLE, dwminimumworkingsetsize: usize, dwmaximumworkingsetsize: usize, flags: u32) -> i32);
windows::core::link!("kernel32.dll" "system" fn GetProcessHeap() -> HANDLE);
windows::core::link!("kernel32.dll" "system" fn HeapCompact(hheap: HANDLE, dwflags: u32) -> usize);
windows::core::link!("advapi32.dll" "system" fn OpenProcessToken(processhandle: HANDLE, desiredaccess: u32, tokenhandle: *mut HANDLE) -> i32);
windows::core::link!("advapi32.dll" "system" fn LookupPrivilegeValueW(lpsystemname: *const u16, lpname: *const u16, lpluid: *mut Luid) -> i32);
windows::core::link!("advapi32.dll" "system" fn AdjustTokenPrivileges(tokenhandle: HANDLE, disableallprivileges: i32, newstate: *const TokenPrivileges, bufferlength: u32, previousstate: *mut c_void, returnlength: *mut u32) -> i32);

#[derive(Clone, Copy, Debug)]
pub struct MemInfo {
    pub pct: u32,
    pub used_bytes: u64,
    pub total_bytes: u64,
    pub avail_bytes: u64,
}

#[derive(Clone)]
struct CacheTarget {
    path: PathBuf,
    min_age: Duration,
    name_filter: Option<fn(&str) -> bool>,
    skip_browser_meta: bool,
    group: &'static str,
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

fn is_thumb_or_icon_cache(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    (lower.starts_with("thumbcache_") || lower.starts_with("iconcache_")) && lower.ends_with(".db")
}

fn is_browser_meta_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "index" | "the-real-index" | "lock" | "current" | "data_0" | "data_1" | "data_2" | "data_3" | "data_4"
    ) || lower.starts_with("manifest-")
}

fn push_chromium_caches(user_data: &Path, add: &mut impl FnMut(CacheTarget)) {
    if !user_data.is_dir() {
        return;
    }
    if let Ok(rd) = fs::read_dir(user_data) {
        for entry in rd.flatten() {
            let p = entry.path();
            if !p.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == "Default" || name == "Guest Profile" || name.starts_with("Profile ") {
                add(CacheTarget {
                    path: p.join("Cache").join("Cache_Data"),
                    min_age: Duration::from_secs(30),
                    name_filter: None,
                    skip_browser_meta: true,
                    group: "browser",
                });
                add(CacheTarget {
                    path: p.join("Code Cache"),
                    min_age: Duration::from_secs(30),
                    name_filter: None,
                    skip_browser_meta: true,
                    group: "browser",
                });
                add(CacheTarget {
                    path: p.join("GPUCache"),
                    min_age: Duration::from_secs(60),
                    name_filter: None,
                    skip_browser_meta: true,
                    group: "browser",
                });
            }
        }
    }
    for top in ["GrShaderCache", "ShaderCache"] {
        add(CacheTarget {
            path: user_data.join(top),
            min_age: Duration::from_secs(60),
            name_filter: None,
            skip_browser_meta: true,
            group: "browser",
        });
    }
}

fn cache_targets() -> Vec<CacheTarget> {
    let mut targets: Vec<CacheTarget> = Vec::new();
    let mut push_unique = |t: CacheTarget| {
        if t.path.is_dir()
            && !targets
                .iter()
                .any(|e| e.path.as_os_str().eq_ignore_ascii_case(t.path.as_os_str()))
        {
            targets.push(t);
        }
    };

    // Lead with UserTemp (Worker 1), then Browser caches (Workers 2-3), then GPU shaders (Worker 4)
    // so all top-impact categories clean concurrently from millisecond 0 with zero starvation.
    push_unique(CacheTarget {
        path: std::env::temp_dir(),
        min_age: Duration::from_secs(15),
        name_filter: None,
        skip_browser_meta: false,
        group: "temp",
    });

    if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        // Chromium browser disposable caches (Chrome, Edge, Brave, Vivaldi — all profiles)
        for rel in [
            r"Google\Chrome\User Data",
            r"Microsoft\Edge\User Data",
            r"BraveSoftware\Brave-Browser\User Data",
            r"Vivaldi\User Data",
        ] {
            push_chromium_caches(&local.join(rel), &mut push_unique);
        }

        // GPU Shader Caches (AMD, NVIDIA, Intel, Direct3D)
        for rel in [
            r"AMD\DxCache",
            r"AMD\DxcCache",
            r"AMD\GLCache",
            r"NVIDIA\DXCache",
            r"NVIDIA\GLCache",
            r"NVIDIA Corporation\NV_Cache",
            r"Intel\ShaderCache",
            "D3DSCache",
        ] {
            push_unique(CacheTarget {
                path: local.join(rel),
                min_age: Duration::from_secs(30),
                name_filter: None,
                skip_browser_meta: false,
                group: "gpu",
            });
        }

        // Firefox disposable cache2/entries
        let ff = local.join(r"Mozilla\Firefox\Profiles");
        if let Ok(rd) = fs::read_dir(&ff) {
            for e in rd.flatten() {
                let p = e.path().join("cache2").join("entries");
                push_unique(CacheTarget {
                    path: p,
                    min_age: Duration::from_secs(30),
                    name_filter: None,
                    skip_browser_meta: false,
                    group: "browser",
                });
            }
        }

        // Remaining Windows Temp, CrashDumps, WER, INetCache, stale Explorer thumbnails
        for (rel, age) in [
            ("Temp", 15),
            ("CrashDumps", 15),
            (r"Microsoft\Windows\WER", 30),
            (r"Microsoft\Windows\INetCache", 30),
            (r"Microsoft\Windows\Caches", 60),
        ] {
            push_unique(CacheTarget {
                path: local.join(rel),
                min_age: Duration::from_secs(age),
                name_filter: None,
                skip_browser_meta: false,
                group: "temp",
            });
        }
        push_unique(CacheTarget {
            path: local.join(r"Microsoft\Windows\Explorer"),
            min_age: Duration::from_secs(300),
            name_filter: Some(is_thumb_or_icon_cache),
            skip_browser_meta: false,
            group: "temp",
        });

        // Steam & Spotify disposable caches
        for rel in [
            r"Steam\htmlcache\Cache\Cache_Data",
            r"Steam\htmlcache\Code Cache",
            r"Spotify\Browser\Cache\Cache_Data",
        ] {
            push_unique(CacheTarget {
                path: local.join(rel),
                min_age: Duration::from_secs(60),
                name_filter: None,
                skip_browser_meta: true,
                group: "apps",
            });
        }
    }

    if let Some(roaming) = std::env::var_os("APPDATA").map(PathBuf::from) {
        // Opera / Opera GX disposable caches
        for rel in [r"Opera Software\Opera Stable", r"Opera Software\Opera GX Stable"] {
            let base = roaming.join(rel);
            for sub in [r"Cache\Cache_Data", "Code Cache", "GPUCache"] {
                push_unique(CacheTarget {
                    path: base.join(sub),
                    min_age: Duration::from_secs(30),
                    name_filter: None,
                    skip_browser_meta: true,
                    group: "browser",
                });
            }
        }
        // Discord & VS Code disposable caches (never settings/sessions)
        for rel in [
            r"discord\Cache\Cache_Data",
            r"discord\Code Cache",
            r"discord\GPUCache",
            r"Code\Cache\Cache_Data",
            r"Code\CachedData",
            r"Code\GPUCache",
        ] {
            push_unique(CacheTarget {
                path: roaming.join(rel),
                min_age: Duration::from_secs(60),
                name_filter: None,
                skip_browser_meta: true,
                group: "apps",
            });
        }
    }

    // Best-effort Windows system temp / update download cache (cleaned when elevated)
    push_unique(CacheTarget {
        path: PathBuf::from(r"C:\Windows\Temp"),
        min_age: Duration::from_secs(30),
        name_filter: None,
        skip_browser_meta: false,
        group: "temp",
    });
    push_unique(CacheTarget {
        path: PathBuf::from(r"C:\Windows\SoftwareDistribution\Download"),
        min_age: Duration::from_secs(3600),
        name_filter: None,
        skip_browser_meta: false,
        group: "temp",
    });

    targets
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

#[allow(clippy::too_many_arguments)]
fn scan_dir(
    dir: &Path,
    depth: usize,
    now: SystemTime,
    target: &CacheTarget,
    deadline: Instant,
    seen: &mut usize,
    bytes: &mut u64,
    files: &mut u64,
    partial: &mut bool,
) {
    if depth > MAX_DEPTH {
        return;
    }
    if *seen >= MAX_SCAN_ENTRIES || Instant::now() >= deadline {
        *partial = true;
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut subdirs = Vec::new();
    for entry in rd.flatten() {
        if *seen >= MAX_SCAN_ENTRIES || Instant::now() >= deadline {
            *partial = true;
            return;
        }
        *seen += 1;
        let path = entry.path();
        let Ok(md) = fs::symlink_metadata(&path) else { continue };
        if is_reparse(&md) {
            continue;
        }
        if md.is_dir() {
            if target.name_filter.is_none() {
                subdirs.push(path);
            }
        } else if md.is_file() && is_old_enough(&md, now, target.min_age) {
            let fname = entry.file_name().to_string_lossy().into_owned();
            if let Some(f) = target.name_filter {
                if !f(&fname) {
                    continue;
                }
            }
            if target.skip_browser_meta && is_browser_meta_file(&fname) {
                continue;
            }
            *bytes = bytes.saturating_add(md.len());
            *files += 1;
        }
    }
    for sub in subdirs {
        if *seen >= MAX_SCAN_ENTRIES || Instant::now() >= deadline {
            *partial = true;
            return;
        }
        scan_dir(&sub, depth + 1, now, target, deadline, seen, bytes, files, partial);
    }
}

#[allow(clippy::too_many_arguments)]
fn clean_dir_inner(
    dir: &Path,
    depth: usize,
    now: SystemTime,
    target: &CacheTarget,
    deadline: Instant,
    seen: &mut usize,
    freed: &mut u64,
    files: &mut u64,
    on_tick: &mut impl FnMut(u64, u64),
) {
    if depth > MAX_DEPTH || *seen >= MAX_CLEAN_ENTRIES || Instant::now() >= deadline {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut subdirs = Vec::new();
    for entry in rd.flatten() {
        if *seen >= MAX_CLEAN_ENTRIES || Instant::now() >= deadline {
            break;
        }
        *seen += 1;
        let path = entry.path();
        let Ok(md) = fs::symlink_metadata(&path) else { continue };
        if is_reparse(&md) {
            continue;
        }
        if md.is_dir() {
            if target.name_filter.is_none() {
                subdirs.push((path, is_old_enough(&md, now, target.min_age)));
            }
        } else if md.is_file() && is_old_enough(&md, now, target.min_age) {
            let fname = entry.file_name().to_string_lossy().into_owned();
            if let Some(f) = target.name_filter {
                if !f(&fname) {
                    continue;
                }
            }
            if target.skip_browser_meta && is_browser_meta_file(&fname) {
                continue;
            }
            let len = md.len();
            if fs::remove_file(&path).is_ok() {
                *freed = freed.saturating_add(len);
                *files += 1;
                if (*files & 0x3F) == 0 {
                    on_tick(*freed, *files);
                }
            }
        }
    }
    for (sub, old) in subdirs {
        if *seen >= MAX_CLEAN_ENTRIES || Instant::now() >= deadline {
            break;
        }
        clean_dir_inner(&sub, depth + 1, now, target, deadline, seen, freed, files, on_tick);
        if old && !target.skip_browser_meta {
            let _ = fs::remove_dir(&sub);
        }
    }
}

/// Cleans stale files inside `dir` without ever deleting `dir` itself.
#[cfg(test)]
pub fn clean_dir_safe(dir: &Path, min_age: Duration) -> (u64, u64) {
    let target = CacheTarget {
        path: dir.to_path_buf(),
        min_age,
        name_filter: None,
        skip_browser_meta: false,
        group: "temp",
    };
    let (mut seen, mut freed, mut files) = (0usize, 0u64, 0u64);
    let deadline = Instant::now() + Duration::from_millis(1200);
    let mut noop = |_, _| {};
    clean_dir_inner(dir, 0, SystemTime::now(), &target, deadline, &mut seen, &mut freed, &mut files, &mut noop);
    (freed, files)
}

fn process_working_set(h: HANDLE) -> u64 {
    let mut pmc = ProcessMemoryCounters {
        cb: std::mem::size_of::<ProcessMemoryCounters>() as u32,
        ..Default::default()
    };
    let ok = unsafe { K32GetProcessMemoryInfo(h, &mut pmc, pmc.cb) };
    if ok != 0 {
        pmc.working_set_size as u64
    } else {
        0
    }
}

fn trim_handle(h: HANDLE) -> (bool, u64) {
    let before = process_working_set(h);
    let ok1 = unsafe { SetProcessWorkingSetSizeEx(h, usize::MAX, usize::MAX, 0) != 0 };
    let ok2 = unsafe { K32EmptyWorkingSet(h) != 0 };
    if !ok1 && !ok2 {
        return (false, 0);
    }
    let after = process_working_set(h);
    (true, before.saturating_sub(after))
}

fn collect_trimmable_pids() -> Vec<(u32, u64)> {
    let mut pids = vec![0u32; 2048];
    let mut needed = 0u32;
    let ok = unsafe {
        K32EnumProcesses(
            pids.as_mut_ptr(),
            (pids.len() * std::mem::size_of::<u32>()) as u32,
            &mut needed,
        )
    };
    if ok == 0 {
        return Vec::new();
    }
    let count = (needed as usize / std::mem::size_of::<u32>()).min(pids.len());
    let my_pid = std::process::id();
    let mut list = Vec::with_capacity(count);
    for &pid in &pids[..count] {
        if pid <= 4 || pid == my_pid {
            continue;
        }
        unsafe {
            if let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SET_QUOTA, false, pid) {
                let ws = process_working_set(h);
                let _ = CloseHandle(h);
                // Focus on processes holding meaningful working sets (>= 2 MB)
                if ws >= 2 * 1024 * 1024 {
                    list.push((pid, ws));
                }
            }
        }
    }
    // Trim heaviest memory consumers first so RAM visibly drops right away
    list.sort_unstable_by(|a, b| b.1.cmp(&a.1));
    list
}

fn enable_profile_privilege() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let name: Vec<u16> = "SeProfileSingleProcessPrivilege\0".encode_utf16().collect();
        let mut luid = Luid::default();
        let ok = if LookupPrivilegeValueW(std::ptr::null(), name.as_ptr(), &mut luid) != 0 {
            let tp = TokenPrivileges {
                privilege_count: 1,
                privileges: [LuidAndAttributes { luid, attributes: SE_PRIVILEGE_ENABLED }],
            };
            AdjustTokenPrivileges(token, 0, &tp, std::mem::size_of::<TokenPrivileges>() as u32, std::ptr::null_mut(), std::ptr::null_mut()) != 0
                && windows::Win32::Foundation::GetLastError().0 == 0
        } else {
            false
        };
        let _ = CloseHandle(token);
        ok
    }
}

/// Best-effort kernel standby/modified memory list purge + memory combining (active when elevated).
fn purge_standby_and_combine() -> bool {
    if !enable_profile_privilege() {
        return false;
    }
    unsafe {
        let Ok(ntdll) = GetModuleHandleW(windows::core::w!("ntdll.dll")) else { return false };
        let Some(proc) = GetProcAddress(ntdll, windows::core::s!("NtSetSystemInformation")) else {
            return false;
        };
        let nt_set: extern "system" fn(u32, *mut c_void, u32) -> i32 = std::mem::transmute(proc);
        let mut any = false;
        // 3 = MemoryFlushModifiedList, 5 = MemoryPurgeLowPriorityStandbyList, 4 = MemoryPurgeStandbyList
        for mut cmd in [3u32, 5u32, 4u32] {
            let status = nt_set(
                SYSTEM_MEMORY_LIST_INFORMATION,
                (&mut cmd as *mut u32).cast::<c_void>(),
                std::mem::size_of::<u32>() as u32,
            );
            if status == 0 {
                any = true;
            }
        }
        let mut combine = MemoryCombineInformationEx::default();
        if nt_set(
            SYSTEM_COMBINE_PHYSICAL_MEMORY_INFORMATION,
            (&mut combine as *mut MemoryCombineInformationEx).cast::<c_void>(),
            std::mem::size_of::<MemoryCombineInformationEx>() as u32,
        ) == 0
        {
            any = true;
        }
        any
    }
}

fn compact_self() -> u64 {
    unsafe {
        let heap = GetProcessHeap();
        if !heap.is_invalid() {
            let _ = HeapCompact(heap, 0);
        }
        let (_, freed) = trim_handle(GetCurrentProcess());
        freed
    }
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

/// Fast on-demand snapshot of live RAM usage and cleanable Windows/browser/GPU cache (<= 220ms).
pub fn stats() -> Value {
    let m = mem_snapshot();
    let now = SystemTime::now();
    let targets = cache_targets();
    let total_deadline = Instant::now() + Duration::from_millis(220);
    let (mut seen, mut cache_bytes, mut cache_files, mut partial) = (0usize, 0u64, 0u64, false);
    let total_targets = targets.len().max(1);
    for (idx, t) in targets.iter().enumerate() {
        let now_i = Instant::now();
        if now_i >= total_deadline {
            partial = true;
            break;
        }
        let left = (total_targets - idx) as u64;
        let rem_ms = total_deadline.saturating_duration_since(now_i).as_millis() as u64;
        let slice_ms = (rem_ms / left).clamp(10, 45);
        let slice_deadline = (now_i + Duration::from_millis(slice_ms)).min(total_deadline);
        scan_dir(&t.path, 0, now, t, slice_deadline, &mut seen, &mut cache_bytes, &mut cache_files, &mut partial);
    }
    json!({
        "ramPct": m.pct,
        "usedMb": mb_u64(m.used_bytes),
        "availMb": mb_u64(m.avail_bytes),
        "totalMb": mb_u64(m.total_bytes),
        "cacheMb": mb_round1(cache_bytes),
        "cacheFiles": cache_files,
        "cachePartial": partial,
    })
}

/// Executes deep multi-stage Windows/browser/GPU cache cleanup, DNS flush, progressive largest-first
/// RAM working-set reclamation, heap compaction, and kernel standby purge (if elevated), streaming
/// real live `boost-progress` snapshots to `emit` on every step.
pub fn run_with_progress<F: Fn(Value)>(emit: F) -> Value {
    let t0 = Instant::now();
    let before = mem_snapshot();
    let send = |stage: &str, phase: u32, m: MemInfo, cache_b: u64, files: u64, procs: u32, procs_total: u32, ws_b: u64| {
        let ram_freed = before.used_bytes.saturating_sub(m.used_bytes);
        emit(json!({
            "stage": stage,
            "phase": phase.min(100),
            "ramPct": m.pct,
            "usedMb": mb_u64(m.used_bytes),
            "availMb": mb_u64(m.avail_bytes),
            "totalMb": mb_u64(m.total_bytes),
            "ramFreedMb": mb_u64(ram_freed),
            "wsTrimmedMb": mb_u64(ws_b),
            "cacheFreedMb": mb_round1(cache_b),
            "filesCleaned": files,
            "procsTrimmed": procs,
            "procsTotal": procs_total,
        }));
    };

    send("ram", 4, before, 0, 0, 0, 0, 0);

    // Stage 1 (Instant RAM relief): trim top heaviest memory consumers first so the circular
    // RAM ring begins visibly draining in real time the instant the user clicks Boost.
    let pids = collect_trimmable_pids();
    let total_procs = (pids.len() as u32) + 1;
    let mut procs_trimmed = 0u32;
    let mut ws_trimmed_bytes = 0u64;
    let lead_count = 12usize.min(pids.len());
    let (lead_pids, rest_pids) = pids.split_at(lead_count);
    if !lead_pids.is_empty() {
        let lead_batches = 3usize.min(lead_pids.len());
        let chunk_size = lead_pids.len().div_ceil(lead_batches);
        for (bi, chunk) in lead_pids.chunks(chunk_size).enumerate() {
            for &(pid, _) in chunk {
                unsafe {
                    if let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SET_QUOTA, false, pid) {
                        let (ok, freed) = trim_handle(h);
                        let _ = CloseHandle(h);
                        if ok {
                            procs_trimmed += 1;
                            ws_trimmed_bytes = ws_trimmed_bytes.saturating_add(freed);
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(45));
            let phase = 8 + (((bi + 1) * 14) / lead_batches) as u32;
            send("ram", phase.min(24), mem_snapshot(), 0, 0, procs_trimmed, total_procs, ws_trimmed_bytes);
        }
    }

    // Stage 2: Parallel deep Windows, Browser, GPU & App cache cleanup (4 scoped worker threads,
    // fair per-target time slices so no single slow directory can starve Chrome/GPU/Temp caches).
    let targets = cache_targets();
    let n_targets = targets.len().max(1);
    let cache_deadline = Instant::now() + Duration::from_millis(5000);
    let now = SystemTime::now();
    send("browser", 26, mem_snapshot(), 0, 0, procs_trimmed, total_procs, ws_trimmed_bytes);

    let queue = Mutex::new(VecDeque::from(targets));
    let total_bytes = AtomicU64::new(0);
    let total_files = AtomicU64::new(0);
    let done_targets = AtomicUsize::new(0);
    let active_group = Mutex::new("browser");

    std::thread::scope(|scope| {
        let n_workers = 4usize.min(n_targets);
        let handles: Vec<_> = (0..n_workers)
            .map(|_| {
                scope.spawn(|| {
                    while Instant::now() < cache_deadline {
                        let next = queue.lock().unwrap().pop_front();
                        let Some(t) = next else { break };
                        *active_group.lock().unwrap() = t.group;
                        // Fair per-target slice (up to 1.8s per target) so every category runs
                        let target_deadline = (Instant::now() + Duration::from_millis(1800)).min(cache_deadline);
                        let (mut seen, mut freed, mut files) = (0usize, 0u64, 0u64);
                        let mut last_flushed_b = 0u64;
                        let mut last_flushed_f = 0u64;
                        let mut on_tick = |cur_b: u64, cur_f: u64| {
                            let db = cur_b.saturating_sub(last_flushed_b);
                            let df = cur_f.saturating_sub(last_flushed_f);
                            if db > 0 {
                                total_bytes.fetch_add(db, Ordering::Relaxed);
                                last_flushed_b = cur_b;
                            }
                            if df > 0 {
                                total_files.fetch_add(df, Ordering::Relaxed);
                                last_flushed_f = cur_f;
                            }
                        };
                        clean_dir_inner(&t.path, 0, now, &t, target_deadline, &mut seen, &mut freed, &mut files, &mut on_tick);
                        on_tick(freed, files);
                        done_targets.fetch_add(1, Ordering::Relaxed);
                    }
                })
            })
            .collect();

        // Stream live cache & RAM progress from the coordinator thread every ~95ms while workers run
        while done_targets.load(Ordering::Relaxed) < n_targets && Instant::now() < cache_deadline {
            if handles.iter().all(|h| h.is_finished()) {
                break;
            }
            std::thread::sleep(Duration::from_millis(95));
            let done_n = done_targets.load(Ordering::Relaxed);
            let cur_b = total_bytes.load(Ordering::Relaxed);
            let cur_f = total_files.load(Ordering::Relaxed);
            let grp = *active_group.lock().unwrap();
            let phase = 26 + ((done_n * 40) / n_targets) as u32;
            send(grp, phase.min(66), mem_snapshot(), cur_b, cur_f, procs_trimmed, total_procs, ws_trimmed_bytes);
        }
        for h in handles {
            let _ = h.join();
        }
    });

    let cache_bytes = total_bytes.load(Ordering::Relaxed);
    let files_cleaned = total_files.load(Ordering::Relaxed);
    send("cache", 67, mem_snapshot(), cache_bytes, files_cleaned, procs_trimmed, total_procs, ws_trimmed_bytes);

    // Stage 3: DNS resolver cache flush
    send("dns", 70, mem_snapshot(), cache_bytes, files_cleaned, procs_trimmed, total_procs, ws_trimmed_bytes);
    let dns_flushed = flush_dns_cache();

    // Stage 4: Remaining process working-set reclamation (5 live-synced batches)
    if !rest_pids.is_empty() {
        let batches = 5usize.min(rest_pids.len());
        let chunk_size = rest_pids.len().div_ceil(batches);
        for (bi, chunk) in rest_pids.chunks(chunk_size).enumerate() {
            for &(pid, _) in chunk {
                unsafe {
                    if let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SET_QUOTA, false, pid) {
                        let (ok, freed) = trim_handle(h);
                        let _ = CloseHandle(h);
                        if ok {
                            procs_trimmed += 1;
                            ws_trimmed_bytes = ws_trimmed_bytes.saturating_add(freed);
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(40));
            let phase = 72 + (((bi + 1) * 18) / batches) as u32;
            send("ram", phase.min(90), mem_snapshot(), cache_bytes, files_cleaned, procs_trimmed, total_procs, ws_trimmed_bytes);
        }
    }

    // Stage 5: Second pass on top hogs (reclaims pages faulted during cache cleanup) +
    // compact own heap + trim self + kernel standby/modified list purge (when elevated)
    send("memory", 93, mem_snapshot(), cache_bytes, files_cleaned, procs_trimmed, total_procs, ws_trimmed_bytes);
    for &(pid, _) in lead_pids {
        unsafe {
            if let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SET_QUOTA, false, pid) {
                let (_, freed) = trim_handle(h);
                let _ = CloseHandle(h);
                ws_trimmed_bytes = ws_trimmed_bytes.saturating_add(freed);
            }
        }
    }
    let self_freed = compact_self();
    procs_trimmed += 1;
    ws_trimmed_bytes = ws_trimmed_bytes.saturating_add(self_freed);
    let standby_purged = purge_standby_and_combine();
    std::thread::sleep(Duration::from_millis(90));

    let after = mem_snapshot();
    send("done", 100, after, cache_bytes, files_cleaned, procs_trimmed, total_procs, ws_trimmed_bytes);

    let ram_freed_bytes = before.used_bytes.saturating_sub(after.used_bytes);
    let elapsed_ms = t0.elapsed().as_millis() as u64;
    nova_core::log::info(&format!(
        "[boost] ram={}MB (ws={}MB, {}%->{}%), cache={}MB ({} files), procs={}, dns={}, standby={}, {}ms",
        mb_u64(ram_freed_bytes),
        mb_u64(ws_trimmed_bytes),
        before.pct,
        after.pct,
        mb_round1(cache_bytes),
        files_cleaned,
        procs_trimmed,
        dns_flushed,
        standby_purged,
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
        "wsTrimmedMb": mb_u64(ws_trimmed_bytes),
        "cacheFreedMb": mb_round1(cache_bytes),
        "filesCleaned": files_cleaned,
        "procsTrimmed": procs_trimmed,
        "dnsFlushed": dns_flushed,
        "standbyPurged": standby_purged,
        "elapsedMs": elapsed_ms,
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

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

        let (freed_fresh, count_fresh) = clean_dir_safe(&root, Duration::from_secs(60));
        assert_eq!(freed_fresh, 0);
        assert_eq!(count_fresh, 0);
        assert!(f1.exists() && f2.exists());

        let (freed, count) = clean_dir_safe(&root, Duration::ZERO);
        assert_eq!(freed, 6144);
        assert_eq!(count, 2);
        assert!(root.is_dir(), "root temp folder itself must never be deleted");
        assert!(!f1.exists() && !f2.exists() && !sub.exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn browser_meta_and_thumbcache_filters_are_strictly_safe() {
        assert!(is_browser_meta_file("index"));
        assert!(is_browser_meta_file("data_0"));
        assert!(is_browser_meta_file("MANIFEST-000001"));
        assert!(!is_browser_meta_file("f_0001a2"));
        assert!(!is_browser_meta_file("9f82b3a1_0"));

        assert!(is_thumb_or_icon_cache("thumbcache_256.db"));
        assert!(is_thumb_or_icon_cache("iconcache_idx.db"));
        assert!(!is_thumb_or_icon_cache("desktop.ini"));
        assert!(!is_thumb_or_icon_cache("important.db"));
    }

    #[test]
    fn process_memory_counters_layout_matches_win64_sdk() {
        assert_eq!(std::mem::size_of::<ProcessMemoryCounters>(), 72);
        let ws = unsafe { process_working_set(GetCurrentProcess()) };
        assert!(ws > 1024 * 1024, "current process working set should exceed 1 MB");
    }

    #[test]
    fn progress_stream_is_monotonic_and_reaches_done() {
        let events: RefCell<Vec<Value>> = RefCell::new(Vec::new());
        let target_dir = std::env::temp_dir().join(format!("nova-boost-meta-{}", std::process::id()));
        let _ = fs::remove_dir_all(&target_dir);
        fs::create_dir_all(&target_dir).unwrap();
        let meta = target_dir.join("index");
        let blob = target_dir.join("f_0001");
        fs::write(&meta, b"keep").unwrap();
        fs::write(&blob, vec![1u8; 2048]).unwrap();

        let t = CacheTarget {
            path: target_dir.clone(),
            min_age: Duration::ZERO,
            name_filter: None,
            skip_browser_meta: true,
            group: "browser",
        };
        let (mut seen, mut freed, mut files) = (0usize, 0u64, 0u64);
        let mut tick = |b, f| events.borrow_mut().push(json!({ "b": b, "f": f }));
        clean_dir_inner(&target_dir, 0, SystemTime::now(), &t, Instant::now() + Duration::from_secs(2), &mut seen, &mut freed, &mut files, &mut tick);
        assert!(meta.exists(), "browser index metadata must be preserved");
        assert!(!blob.exists(), "browser cache blob must be deleted");
        assert_eq!(freed, 2048);
        assert_eq!(files, 1);

        let _ = fs::remove_dir_all(&target_dir);
    }
}
