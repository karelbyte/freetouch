@echo off
:: PASO 1: activa firmas de prueba y reinicia. Ejecutar como administrador.
net session >nul 2>&1
if %errorlevel% neq 0 (
  powershell -Command "Start-Process '%~f0' -Verb RunAs"
  exit /b
)
bcdedit /set testsigning on
echo.
echo Modo de prueba ACTIVADO. Reinicia el PC ahora (veras "Modo de prueba" en el escritorio).
pause
shutdown /r /t 5
