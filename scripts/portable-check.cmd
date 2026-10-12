@echo off
chcp 65001 >nul
setlocal
"%~dp0zzz-auto-engine.exe" --root "%~dp0." --data "%~dp0data" doctor > "%~dp0diagnostic.log" 2>&1
set "ZZZ_AUTO_DIAGNOSTIC_EXIT=%ERRORLEVEL%"
type "%~dp0diagnostic.log"
echo.
echo Diagnostic saved to diagnostic.log
pause
exit /b %ZZZ_AUTO_DIAGNOSTIC_EXIT%
