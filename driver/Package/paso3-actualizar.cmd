@echo off
:: Actualiza los 2 cables con la ultima version (con telemetria).
net session >nul 2>&1
if %errorlevel% neq 0 (
  powershell -Command "Start-Process '%~f0' -Verb RunAs"
  exit /b
)
cd /d "%~dp0"
set DEVCON=D:\project\freetouch\third_party\wdk-nuget\c\tools\10.0.26100.0\x64\devcon.exe
"%DEVCON%" update freetouch-cable.inf Root\FreeTouchCableA
"%DEVCON%" update freetouch-cable.inf Root\FreeTouchCableB
echo.
echo Listo. Avisame para leer la telemetria.
pause
