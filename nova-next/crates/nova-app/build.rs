// The exe's own resources: Nova's icon (resource 1: Explorer, the taskbar, the windows) and its
// version info (name, publisher, version), from build/icon.ico and this package's version.
use std::{env, fs, path::PathBuf};

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../..");
    let icon = root.join("build").join("icon.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    // (1.8.7) the main thread's stack: 8 MB instead of Windows' 1 MB. Running out of it ends the
    // app with no panic and no log line (Partner Center's FAIL_FAST c0000409); the pages, the
    // widgets and the COM callbacks all run deep on this one thread. It's only reserved, not used.
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu") {
        println!("cargo:rustc-link-arg-bins=-Wl,--stack,8388608");
    } else {
        println!("cargo:rustc-link-arg-bins=/STACK:8388608");
    }
    let v = env::var("CARGO_PKG_VERSION").unwrap();
    let n: Vec<u32> = v.split('.').map(|x| x.parse().unwrap_or(0)).collect();
    let (a, b, c) = (n.first().copied().unwrap_or(0), n.get(1).copied().unwrap_or(0), n.get(2).copied().unwrap_or(0));
    // rc takes forward slashes
    let icon_path = icon.to_string_lossy().replace('\\', "/");
    let rc = format!(
        r#"1 ICON "{icon_path}"
1 VERSIONINFO
FILEVERSION {a},{b},{c},0
PRODUCTVERSION {a},{b},{c},0
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "AmerTech"
      VALUE "FileDescription", "Nova Island"
      VALUE "FileVersion", "{v}"
      VALUE "InternalName", "Nova Island"
      VALUE "LegalCopyright", "Copyright (C) 2026 AmerTech. GPL v3."
      VALUE "OriginalFilename", "Nova Island.exe"
      VALUE "ProductName", "Nova Island"
      VALUE "ProductVersion", "{v}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
    );
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("app.rc");
    fs::write(&out, rc).unwrap();
    embed_resource::compile(&out, embed_resource::NONE).manifest_optional().unwrap();
}
