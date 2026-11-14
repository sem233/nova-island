@echo off
rem Double-click: creates the desktop icon + start-with-Windows entry and starts Nova Island.
cd /d "%~dp0"
if not exist "node_modules\electron\dist\electron.exe" (
  echo Installing Nova Island dependencies (one-time)...
  call npm install
  if exist "node_modules\electron\install.js" call node "node_modules\electron\install.js"
)
if not exist "node_modules\electron\dist\electron.exe" (
  echo electron.exe not found. Make sure Node.js is installed and internet is connected.
  pause
  exit /b 1
)
start "" "node_modules\electron\dist\electron.exe" "%~dp0." --setup
echo Nova Island is starting. The result is written to setup-log.txt
timeout /t 4 >nul
