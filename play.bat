@echo off
rem Starts the local server (if this PC has one and it is not running) and the game. Run setup.bat first.
rem On a friend's server instead: set WOW_HOST=<their address> before running this.
rem In game: J climbs into / out of the kart (PS5, PS4 or Xbox pad, or keyboard), K swaps the chase camera,
rem F5 hosts / joins / starts a race at a course's start.
cd /d "%~dp0"
if not exist target\release\benilla.exe (
    echo Not built yet: run setup.bat first.
    pause
    exit /b 1
)
if "%KARTCRAFT_SERVER_PROJECT%"=="" set KARTCRAFT_SERVER_PROJECT=world-of-kartcraft
if "%WOW_HOST%"=="" (
    set WOW_HOST=localhost
    if exist server\compose.yaml docker compose -p %KARTCRAFT_SERVER_PROJECT% --project-directory server -f server\compose.yaml up -d
)
set WOW_KART_DATA=%~dp0kart-data
target\release\benilla.exe %*
