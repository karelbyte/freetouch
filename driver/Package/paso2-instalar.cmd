@echo off
:: PASO 2 (despues del reinicio): instala certificado, firma e instala los 2 cables.
net session >nul 2>&1
if %errorlevel% neq 0 (
  powershell -Command "Start-Process '%~f0' -Verb RunAs"
  exit /b
)
cd /d "%~dp0"
set SIGNTOOL=C:\Program Files (x86)\Windows Kits\10\bin\10.0.26100.0\x64\signtool.exe
set DEVCON=D:\project\freetouch\third_party\wdk-nuget\c\tools\10.0.26100.0\x64\devcon.exe
echo Instalando certificado de prueba...
certutil -addstore Root freetouch-test.cer
certutil -addstore TrustedPublisher freetouch-test.cer
echo Firmando catalogo y driver...
"%SIGNTOOL%" sign /fd SHA256 /f freetouch-test.pfx /p freetouch /tr http://timestamp.digicert.com /td SHA256 freetouch-cable.cat freetouchcable.sys
echo Instalando Cable A...
"%DEVCON%" install freetouch-cable.inf Root\FreeTouchCableA
echo Instalando Cable B...
"%DEVCON%" install freetouch-cable.inf Root\FreeTouchCableB
echo.
echo Listo. Revisa Sonido: deben aparecer FreeTouch A/B Entrada/Salida.
pause
