@echo off
setlocal
chcp 65001 >nul
title Kavychka
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\start.ps1" %*
set "launchResult=%ERRORLEVEL%"
if "%launchResult%"=="0" exit /b 0
echo.
echo Launch failed. See the error above.
pause
exit /b %launchResult%
