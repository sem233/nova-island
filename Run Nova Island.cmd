@echo off
rem Runs Nova Island without modifying existing shortcuts or the old installation.
cd /d "%~dp0"
start "" "node_modules\electron\dist\electron.exe" "%~dp0."
