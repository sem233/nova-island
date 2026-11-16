Unicode true
RequestExecutionLevel user
SetCompressor /SOLID lzma
SilentInstall silent

Name "Nova Island Portable"
OutFile "${OUT}"
Icon "${ICON}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "Nova Island Portable"
VIAddVersionKey "CompanyName" "AmerTech"
VIAddVersionKey "FileDescription" "Nova Island Portable"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "Copyright (C) 2026 AmerTech. GPL v3."

Section
  InitPluginsDir
  SetOutPath "$PLUGINSDIR"
  File "/oname=Nova Island.exe" "${EXE}"
  File "${DLL}"
  CreateDirectory "$PLUGINSDIR\tools"
  File "/oname=$PLUGINSDIR\tools\yt-dlp.exe" "${YTDLP}"
  ExecWait '"$PLUGINSDIR\Nova Island.exe"'
SectionEnd
