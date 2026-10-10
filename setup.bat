@echo off
rem World of Kartcraft setup (Windows). See INSTALL.md.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0setup\setup.ps1" %*
pause
