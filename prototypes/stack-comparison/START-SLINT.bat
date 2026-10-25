@echo off
setlocal
set "SLINT_STYLE=fluent"
if not exist "%~dp0artifacts\slint\Quote.Slint.exe" (
  echo First build the prototypes with build.ps1.
  pause
  exit /b 1
)
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0start.ps1" -Stack slint
if errorlevel 1 pause
