; Nova Island 1.8 installer (NSIS, Modern UI 2), the same install as 1.7's electron-builder one:
; for the current user only, the same "Installed apps" entry (its GUID), the same Start-menu and
; desktop shortcuts, "Run Nova Island" at the end, and the data in %APPDATA%\dynamic-nova never
; touched. Over a 1.7 install it closes the app, removes Electron's files and installs into the
; same folder. It takes electron-updater's switches too, so 1.7 updates itself into 1.8:
;   /S             silent
;   --updated      an update (keeps the user's shortcuts as they are)
;   --force-run    start Nova Island when done
; Built by build.ps1:  makensis /DVERSION=1.8.0 /DEXE=<release exe> /DOUT=<setup exe> nova-island.nsi

Unicode true
ManifestDPIAware true
RequestExecutionLevel user
SetCompressor /SOLID lzma

!include "MUI2.nsh"
!include "FileFunc.nsh"
!include "WordFunc.nsh"
!include "LogicLib.nsh"

!define APP "Nova Island"
!define PUBLISHER "AmerTech"
; electron-builder's UUID v5 of the appId com.dynamicnova.app: keeps one entry in Installed apps
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\a9c08701-fc32-5535-8c18-9dda7436a24c"
!define RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"
!define RUN_VALUE "electron.app.Nova Island"
!define WEBVIEW2 "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"
!define ROOT "..\..\.."

Name "${APP}"
OutFile "${OUT}"
InstallDir "$LOCALAPPDATA\Programs\${APP}"
BrandingText "${APP} ${VERSION} by ${PUBLISHER}"
ShowInstDetails nevershow
ShowUninstDetails nevershow

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${APP}"
VIAddVersionKey "CompanyName" "${PUBLISHER}"
VIAddVersionKey "FileDescription" "${APP} Setup"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "Copyright (C) 2026 ${PUBLISHER}. GPL v3."

!define MUI_ICON "${ROOT}\build\icon.ico"
!define MUI_UNICON "${ROOT}\build\icon.ico"
!define MUI_WELCOMEFINISHPAGE_BITMAP "art\sidebar.bmp"
!define MUI_UNWELCOMEFINISHPAGE_BITMAP "art\sidebar.bmp"
!define MUI_HEADERIMAGE
!define MUI_HEADERIMAGE_RIGHT
!define MUI_HEADERIMAGE_BITMAP "art\header.bmp"
!define MUI_HEADERIMAGE_UNBITMAP "art\header.bmp"
!define MUI_ABORTWARNING

!define MUI_WELCOMEPAGE_TITLE "Welcome to ${APP} ${VERSION}"
!define MUI_WELCOMEPAGE_TEXT "A living island for your Windows desktop: music, prayer times, downloads, apps and more, right at the top of the screen.$\r$\n$\r$\nYour settings are kept if ${APP} is already installed.$\r$\n$\r$\nClick Next to continue."
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${ROOT}\LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN
!define MUI_FINISHPAGE_RUN_TEXT "Run ${APP}"
!define MUI_FINISHPAGE_RUN_FUNCTION RunApp
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

Var IsUpdate

; Starts Nova Island as the signed-in user, also from an installer that has admin rights (a 1.7
; install in Program Files): Explorer starts it, so it never runs as admin.
!macro StartApp
  Exec '"$WINDIR\explorer.exe" "$INSTDIR\${APP}.exe"'
!macroend

Function RunApp
  !insertmacro StartApp
FunctionEnd

; $0 = 1 when this installer may write in $INSTDIR (made if it's missing)
Function CanWrite
  StrCpy $0 0
  ClearErrors
  CreateDirectory "$INSTDIR"
  FileOpen $9 "$INSTDIR\.nova-write-test" w
  ${IfNot} ${Errors}
    FileClose $9
    Delete "$INSTDIR\.nova-write-test"
    StrCpy $0 1
  ${EndIf}
FunctionEnd

; Nova Island is running (1.7 or 1.8): close it, with the WebView2 / Electron processes it started
!macro CloseApp
  nsExec::Exec 'taskkill /F /T /IM "${APP}.exe"'
  Pop $0
  Sleep 800
  ; (1.8.7) until the old exe can be written (up to 10 s): a copy still closing kept it locked,
  ; the File below then failed without a word in a silent update, and the old version stayed
  StrCpy $R8 0
  ${DoWhile} ${FileExists} "$INSTDIR\${APP}.exe"
    ClearErrors
    FileOpen $R9 "$INSTDIR\${APP}.exe" a
    ${IfNot} ${Errors}
      FileClose $R9
      ${Break}
    ${EndIf}
    IntOp $R8 $R8 + 1
    ${If} $R8 >= 40
      ${Break}
    ${EndIf}
    Sleep 250
  ${Loop}
!macroend

Function .onInit
  ${GetParameters} $R0
  StrCpy $IsUpdate 0
  ClearErrors
  ${GetOptions} $R0 "--updated" $R1
  ${IfNot} ${Errors}
    StrCpy $IsUpdate 1
  ${EndIf}

  ; where it is installed already: 1.8 writes InstallLocation; 1.7 only an UninstallString whose
  ; folder holds the uninstaller, with the app in it or in its "Nova Island" subfolder
  ReadRegStr $0 HKCU "${UNINST_KEY}" "InstallLocation"
  ${If} $0 != ""
  ${AndIf} ${FileExists} "$0\${APP}.exe"
    StrCpy $INSTDIR $0
  ${Else}
    ReadRegStr $0 HKCU "${UNINST_KEY}" "UninstallString"
    ${If} $0 != ""
      ${WordFind} $0 '"' "+1" $1
      ${GetParent} $1 $2
      ${If} ${FileExists} "$2\${APP}\${APP}.exe"
        StrCpy $INSTDIR "$2\${APP}"
      ${ElseIf} ${FileExists} "$2\${APP}.exe"
        StrCpy $INSTDIR $2
      ${EndIf}
    ${EndIf}
  ${EndIf}

  ; 1.7 let people install into a folder of their choice, often C:\Program Files\Nova Island,
  ; which this installer (no admin rights) can't write to: 1.8.0's update closed 1.7 there and
  ; then couldn't put itself in. Now it asks Windows for admin rights once, for that folder.
  Call CanWrite
  ${If} $0 == 0
    ClearErrors
    ${GetOptions} $R0 "--elevated" $R1
    ${If} ${Errors}
      ClearErrors
      ExecShell "runas" "$EXEPATH" "$R0 --elevated"
      ${If} ${Errors}
        ; no admin rights: the version that's there runs again, as before
        ClearErrors
        ${GetOptions} $R0 "--force-run" $R1
        ${IfNot} ${Errors}
          !insertmacro StartApp
        ${ElseIfNot} ${Silent}
          MessageBox MB_OK|MB_ICONEXCLAMATION "${APP} is installed in $INSTDIR, and updating it there needs administrator rights.$\r$\n$\r$\nRun the installer again and choose Yes when Windows asks."
        ${EndIf}
      ${EndIf}
      Quit
    ${EndIf}
  ${EndIf}

  ; WebView2 draws Nova's pages; Windows 11 has it, an old Windows 10 may not
  ReadRegStr $0 HKLM "SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\${WEBVIEW2}" "pv"
  ${If} $0 == ""
  ${OrIf} $0 == "0.0.0.0"
    ReadRegStr $0 HKCU "Software\Microsoft\EdgeUpdate\Clients\${WEBVIEW2}" "pv"
  ${EndIf}
  ${If} $0 == ""
  ${OrIf} $0 == "0.0.0.0"
    ${IfNot} ${Silent}
      MessageBox MB_YESNO|MB_ICONINFORMATION "${APP} needs Microsoft Edge WebView2, a free part of Windows that is missing on this PC.$\r$\n$\r$\nOpen Microsoft's download page now? (${APP} installs either way.)" IDNO +2
      ExecShell "open" "https://go.microsoft.com/fwlink/p/?LinkId=2124703"
    ${EndIf}
  ${EndIf}
FunctionEnd

Section "Install"
  !insertmacro CloseApp
  SetOutPath "$INSTDIR"

  ; over 1.7: Electron's files go (the app's data lives in %APPDATA% and stays)
  ${If} ${FileExists} "$INSTDIR\resources\app.asar"
    RMDir /r "$INSTDIR\resources"
    RMDir /r "$INSTDIR\locales"
    Delete "$INSTDIR\*.pak"
    Delete "$INSTDIR\*.dll"
    Delete "$INSTDIR\*.bin"
    Delete "$INSTDIR\*.dat"
    Delete "$INSTDIR\vk_swiftshader_icd.json"
    Delete "$INSTDIR\LICENSE.electron.txt"
    Delete "$INSTDIR\LICENSES.chromium.html"
  ${EndIf}
  Delete "$INSTDIR\uninstallerIcon.ico" ; 1.7's (the entry now uses the exe's own icon)
  ; 1.7 kept its uninstaller one folder up ("…\Programs\Dynamic Nova")
  ${GetParent} "$INSTDIR" $0
  ${If} ${FileExists} "$0\Uninstall ${APP}.exe"
    Delete "$0\Uninstall ${APP}.exe"
    Delete "$0\uninstallerIcon.ico"
  ${EndIf}
  ; 1.7's update downloads (the last one is often ~110 MB)
  RMDir /r "$LOCALAPPDATA\nova-island-updater\pending"

  ; (1.8.7) a write that fails is tried again, then said: never the old version left in place
  ; quietly as if updated
  ClearErrors
  File "/oname=${APP}.exe" "${EXE}"
  ${If} ${Errors}
    Sleep 2000
    ClearErrors
    File "/oname=${APP}.exe" "${EXE}"
  ${EndIf}
  ${If} ${Errors}
    SetErrorLevel 2
    MessageBox MB_ICONSTOP "Nova Island is still running and couldn't be replaced. Close it and run the installer again." /SD IDOK
    Abort
  ${EndIf}
  File "${DLL}"
  CreateDirectory "$INSTDIR\tools"
  File "/oname=$INSTDIR\tools\yt-dlp.exe" "${YTDLP}"
  WriteUninstaller "$INSTDIR\Uninstall ${APP}.exe"

  ; Installed apps
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "${APP}"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "${PUBLISHER}"
  WriteRegStr HKCU "${UNINST_KEY}" "Comments" "A living island for your Windows desktop"
  WriteRegStr HKCU "${UNINST_KEY}" "URLInfoAbout" "https://github.com/NOVA-TECH-KIT/nova-island"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\${APP}.exe,0"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\Uninstall ${APP}.exe"'
  WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\Uninstall ${APP}.exe" /S'
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegDWORD HKCU "${UNINST_KEY}" "EstimatedSize" $0

  ; shortcuts: the Start menu always; the desktop on a new install, and kept as it is on an update
  CreateShortcut "$SMPROGRAMS\${APP}.lnk" "$INSTDIR\${APP}.exe" "" "$INSTDIR\${APP}.exe" 0
  ${If} $IsUpdate == 0
  ${OrIf} ${FileExists} "$DESKTOP\${APP}.lnk"
    CreateShortcut "$DESKTOP\${APP}.lnk" "$INSTDIR\${APP}.exe" "" "$INSTDIR\${APP}.exe" 0
  ${EndIf}

  ; Start with Windows, if it was on: the new path (1.7's could point at a folder that moved)
  ReadRegStr $0 HKCU "${RUN_KEY}" "${RUN_VALUE}"
  ${If} $0 != ""
    WriteRegStr HKCU "${RUN_KEY}" "${RUN_VALUE}" '"$INSTDIR\${APP}.exe"'
  ${EndIf}

  ; electron-updater's "--force-run": start it again after a silent update
  ${If} ${Silent}
    ${GetParameters} $R0
    ClearErrors
    ${GetOptions} $R0 "--force-run" $R1
    ${IfNot} ${Errors}
      !insertmacro StartApp
    ${EndIf}
  ${EndIf}
SectionEnd

Function un.onInit
  ; nothing to ask in a silent uninstall
FunctionEnd

Section "Uninstall"
  !insertmacro CloseApp
  Delete "$INSTDIR\${APP}.exe"
  Delete "$INSTDIR\WebView2Loader.dll"
  Delete "$INSTDIR\tools\yt-dlp.exe"
  RMDir "$INSTDIR\tools"
  Delete "$INSTDIR\Uninstall ${APP}.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${APP}.lnk"
  Delete "$DESKTOP\${APP}.lnk"
  DeleteRegValue HKCU "${RUN_KEY}" "${RUN_VALUE}"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "${RUN_VALUE}"
  DeleteRegKey HKCU "${UNINST_KEY}"
  ; WebView2's cache; the settings in %APPDATA%\dynamic-nova stay, like 1.7
  RMDir /r "$LOCALAPPDATA\${APP}\EBWebView"
  RMDir "$LOCALAPPDATA\${APP}"
SectionEnd
