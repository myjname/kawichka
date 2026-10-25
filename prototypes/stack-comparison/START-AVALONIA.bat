@echo off
setlocal
if not exist "%~dp0artifacts\avalonia\Quote.Avalonia.exe" (
  echo First build the prototypes with build.ps1.
  pause
  exit /b 1
)
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0start.ps1" -Stack avalonia
if errorlevel 1 pause
