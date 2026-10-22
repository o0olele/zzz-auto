@echo off
chcp 65001 >nul
setlocal
"%~dp0zzz-auto-engine.exe" --root "%~dp0." --data "%~dp0data" doctor > "%~dp0diagnostic.log" 2>&1
set "ZZZ_AUTO_DIAGNOSTIC_EXIT=%ERRORLEVEL%"
if not "%ZZZ_AUTO_DIAGNOSTIC_EXIT%"=="0" goto show_result
"%~dp0zzz-auto-engine.exe" --root "%~dp0." mail-bundle-check >> "%~dp0diagnostic.log" 2>&1
set "ZZZ_AUTO_DIAGNOSTIC_EXIT=%ERRORLEVEL%"
if not "%ZZZ_AUTO_DIAGNOSTIC_EXIT%"=="0" goto show_result
"%~dp0zzz-auto-engine.exe" --root "%~dp0." map-bundle-check --bundle "%~dp0resources\generated\map-preview" >> "%~dp0diagnostic.log" 2>&1
set "ZZZ_AUTO_DIAGNOSTIC_EXIT=%ERRORLEVEL%"
if not "%ZZZ_AUTO_DIAGNOSTIC_EXIT%"=="0" goto show_result
"%~dp0zzz-auto-engine.exe" --root "%~dp0." coffee-run-bundle-check --bundle "%~dp0resources\generated\coffee-run" >> "%~dp0diagnostic.log" 2>&1
set "ZZZ_AUTO_DIAGNOSTIC_EXIT=%ERRORLEVEL%"
if not "%ZZZ_AUTO_DIAGNOSTIC_EXIT%"=="0" goto show_result
"%~dp0zzz-auto-engine.exe" --root "%~dp0." basic-daily-bundle-check --bundle "%~dp0resources\generated\basic-daily" >> "%~dp0diagnostic.log" 2>&1
set "ZZZ_AUTO_DIAGNOSTIC_EXIT=%ERRORLEVEL%"
if not "%ZZZ_AUTO_DIAGNOSTIC_EXIT%"=="0" goto show_result
"%~dp0zzz-auto-engine.exe" --root "%~dp0." combat-bundle-check --bundle "%~dp0resources\generated\combat" >> "%~dp0diagnostic.log" 2>&1
set "ZZZ_AUTO_DIAGNOSTIC_EXIT=%ERRORLEVEL%"
if not "%ZZZ_AUTO_DIAGNOSTIC_EXIT%"=="0" goto show_result
"%~dp0zzz-auto-engine.exe" --root "%~dp0." suibian-temple-bundle-check --bundle "%~dp0resources\generated\suibian-temple" >> "%~dp0diagnostic.log" 2>&1
set "ZZZ_AUTO_DIAGNOSTIC_EXIT=%ERRORLEVEL%"
:show_result
type "%~dp0diagnostic.log"
echo.
echo Diagnostic saved to diagnostic.log
pause
exit /b %ZZZ_AUTO_DIAGNOSTIC_EXIT%
