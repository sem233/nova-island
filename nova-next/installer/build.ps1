# Builds the Nova Island installer for GitHub (NSIS), like `npm run dist` did for 1.7:
#   1. the app, release profile (cargo)
#   2. the installer pictures from build/icon.png (installer/nsis/make-art.py)
#   3. the installer (NSIS 3: electron-builder's cached copy, else an installed one)
# Output: <target>\dist\Nova-Island-Setup-<version>.exe (target: -Target, else CARGO_TARGET_DIR, else nova-next\target)
#   powershell -ExecutionPolicy Bypass -File nova-next\installer\build.ps1
#   -Target <folder>   build there instead (when the usual folder can't be written, e.g. an
#                      antivirus locked a file in it)
param([string]$Target = '')
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$next = Split-Path -Parent $here
if (-not $Target) { $Target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $next 'target' } }
$target = $Target
$env:CARGO_TARGET_DIR = $target

$version = (Select-String -Path "$next\crates\nova-app\Cargo.toml" -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
Write-Host "Nova Island $version"

Push-Location $next
cargo +stable-x86_64-pc-windows-gnu build --release -p nova-app
if ($LASTEXITCODE) { throw 'cargo build failed' }
Pop-Location

if (-not (Test-Path "$here\nsis\art\header.bmp") -or -not (Test-Path "$here\nsis\art\sidebar.bmp")) {
  python "$here\nsis\make-art.py"
  if ($LASTEXITCODE) { throw 'make-art failed' }
}

# NSIS 3: the copy electron-builder kept (what 1.7 was built with), else an installed one or one on PATH
$makensis = @(
  "$env:LOCALAPPDATA\electron-builder\Cache\nsis\nsis-3.0.4.1\makensis.exe",
  "${env:ProgramFiles(x86)}\NSIS\makensis.exe",
  "$env:ProgramFiles\NSIS\makensis.exe",
  (Get-Command makensis.exe -ErrorAction SilentlyContinue).Source
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if (-not $makensis) { throw 'NSIS 3 (makensis.exe) not found: install it from nsis.sourceforge.io' }
New-Item -ItemType Directory -Force "$target\dist" | Out-Null
$dll = "$target\release\WebView2Loader.dll"
if (-not (Test-Path $dll)) {
  $srcDll = "$next\crates\webview2-com-sys\x64\WebView2Loader.dll"
  if (Test-Path $srcDll) {
    Copy-Item $srcDll $dll -Force
  } else {
    $foundDll = Get-ChildItem -Path "$next" -Recurse -Filter "WebView2Loader.dll" | Where-Object { $_.FullName -like "*x64*" } | Select-Object -First 1
    if ($foundDll) { Copy-Item $foundDll.FullName $dll -Force }
  }
}
$ytdlp = "$target\release\tools\yt-dlp.exe"
if (-not (Test-Path $ytdlp)) {
  $foundYt = "$env:APPDATA\dynamic-nova\tools\yt-dlp.exe"
  if (Test-Path $foundYt) {
    New-Item -ItemType Directory -Force "$target\release\tools" | Out-Null
    Copy-Item $foundYt $ytdlp -Force
  }
}
$out = "$target\dist\Nova-Island-Setup-$version.exe"
$outPortable = "$target\dist\Nova-Island-$version-Portable.exe"
$icon = (Split-Path -Parent $next) + "\build\icon.ico"
Push-Location "$here\nsis"
& $makensis /V2 "/DVERSION=$version" "/DEXE=$target\release\nova-island.exe" "/DDLL=$dll" "/DYTDLP=$ytdlp" "/DOUT=$out" nova-island.nsi
if ($LASTEXITCODE) { throw 'makensis setup failed' }
& $makensis /V2 "/DVERSION=$version" "/DEXE=$target\release\nova-island.exe" "/DDLL=$dll" "/DYTDLP=$ytdlp" "/DICON=$icon" "/DOUT=$outPortable" portable.nsi
if ($LASTEXITCODE) { throw 'makensis portable failed' }
Pop-Location

# (1.8.7) the symbols of this very exe, kept with the installer: a crash's offsets (Partner Center,
# Windows Error Reporting) can then be read as function names and lines
New-Item -ItemType Directory -Force "$target\dist\symbols\$version" | Out-Null
Copy-Item "$target\release\nova_island.pdb" "$target\dist\symbols\$version\nova_island.pdb" -Force -ErrorAction SilentlyContinue
Copy-Item "$target\release\nova-island.exe" "$target\dist\symbols\$version\Nova Island.exe" -Force

$f = Get-Item $out
$sha = (Get-FileHash $out -Algorithm SHA256).Hash.ToLower()
Write-Host ("{0}  {1:N2} MB  sha256 {2}" -f $f.FullName, ($f.Length / 1MB), $sha)
