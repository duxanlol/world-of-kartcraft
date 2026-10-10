@echo off
rem Makes a game account for a friend on your Kartcraft server (see setup\account.ps1).
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0setup\account.ps1" %*
pause
