@echo off
rem Builds the friends' client and publishes it as an update (see publish.ps1).
rem "publish.bat -NoBuild" reuses the last build.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0publish.ps1" %*
