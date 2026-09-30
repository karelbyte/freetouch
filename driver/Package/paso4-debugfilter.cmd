@echo off
:: Activa los mensajes del kernel y reinicia.
net session >nul 2>&1
if %errorlevel% neq 0 (
  powershell -Command "Start-Process '%~f0' -Verb RunAs"
  exit /b
)
reg add "HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Debug Print Filter" /v DEFAULT /t REG_DWORD /d 0xFFFFFFFF /f
echo Filtro activado. Reiniciando en 5 segundos...
shutdown /r /t 5
