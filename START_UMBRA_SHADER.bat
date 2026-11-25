@echo off
setlocal
cd /d "%~dp0"
set "RUNTIME=%~dp0target\release\victory.exe"
if not exist "%RUNTIME%" set "RUNTIME=%~dp0target\diag\victory.exe"
if not exist "%RUNTIME%" (
 echo Patched runtime not found. Build victory first.
 pause
 exit /b 1
)
py -3 "%~dp0umbra_launcher\launcher.py" --runtime "%RUNTIME%" %*
if errorlevel 1 pause
