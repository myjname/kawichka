@echo off
setlocal
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0Start-preview.ps1" %*
set "kavychkaExitCode=%errorlevel%"
if not "%kavychkaExitCode%"=="0" pause
exit /b %kavychkaExitCode%
