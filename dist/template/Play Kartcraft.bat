@echo off
rem World of Kartcraft. The first run asks for your WoW 1.12.1 folder, your Crash Team Racing disc
rem image and the server's address, converts the kart from the disc, and remembers the answers in
rem kartcraft.cfg (delete it to be asked again). Each start first fetches what changed from the
rem server's update host (UPDATE_URL, by default port 8080 of the server). defaults.cfg, which
rem comes with updates, presets SERVER and UPDATE_URL; kartcraft.cfg overrides it.
rem "Play Kartcraft.bat demo" runs the offline demo.
rem Before asking, it looks for both games itself: WoW in this folder, a folder in it or the one
rem above it (Kartcraft unzipped into the WoW folder); the CTR image anywhere under this folder.
setlocal EnableExtensions
cd /d "%~dp0"
set "HERE=%~dp0"
set "CFG=%~dp0kartcraft.cfg"
if exist "%~dp0defaults.cfg" for /f "usebackq tokens=1,* delims==" %%a in ("%~dp0defaults.cfg") do set "%%a=%%b"
set "DEF_SERVER=%SERVER%"
set "DEF_UPDATE_URL=%UPDATE_URL%"
if exist "%CFG%" for /f "usebackq tokens=1,* delims==" %%a in ("%CFG%") do set "%%a=%%b"

:wow
if not defined WOW_DIR if not defined WOW_SEARCHED call :findwow
if not defined WOW_DIR (
    echo.
    echo Where is your World of Warcraft 1.12.1 folder? The one with Data in it,
    echo for example C:\Games\World of Warcraft
    set /p "WOW_DIR=WoW folder: "
)
set "WOW_DIR=%WOW_DIR:"=%"
if /i "%WOW_DIR:~-5%"=="\Data" set "WOW_DIR=%WOW_DIR:~0,-5%"
if not exist "%WOW_DIR%\Data\dbc.MPQ" (
    echo   That folder has no Data\dbc.MPQ: it needs to be an English 1.12.1 client.
    set "WOW_DIR="
    goto wow
)

if /i "%~1"=="demo" goto update

:server
if not defined SERVER (
    echo.
    echo What is the server's address? Ask whoever runs it ^(an IP like 84.12.34.56 or a name^).
    set /p "SERVER=Server address: "
)
if not defined SERVER goto server

:update
set "UPDATES=%UPDATE_URL%"
if not defined UPDATES if defined SERVER set "UPDATES=http://%SERVER%:8080"
if defined UPDATES if exist "%~dp0update.ps1" (
    powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0update.ps1" -Base "%UPDATES%" -Root "%~dp0." -Disc "%CTR_DISC%"
    if errorlevel 1 (
        echo   The update failed; starting the version you have.
        del /q "%~dp0Play Kartcraft.bat.new" >nul 2>&1
    )
)
rem A new launcher replaces this one and runs in its place, all on one line: cmd reads a batch
rem file as it goes, so nothing of the old one may be read after the move.
if exist "%~dp0Play Kartcraft.bat.new" (move /y "%~dp0Play Kartcraft.bat.new" "%~f0" >nul & "%~f0" %*)

:ctr
if not exist "kart-data\kart.json" if not defined CTR_DISC if not defined CTR_SEARCHED call :findctr
if not exist "kart-data\kart.json" (
    if not defined CTR_DISC (
        echo.
        echo Where is your Crash Team Racing disc image? The US release, as a .bin file
        echo ^(with its .cue beside it^), for example C:\Games\CTR\CTR - Crash Team Racing ^(USA^).bin
        set /p "CTR_DISC=CTR .bin file: "
    )
)
if not exist "kart-data\kart.json" (
    set "CTR_DISC=%CTR_DISC:"=%"
    call :convert
    if not exist "kart-data\kart.json" (
        set "CTR_DISC="
        goto ctr
    )
)

(
    echo WOW_DIR=%WOW_DIR%
    if defined SERVER if not "%SERVER%"=="%DEF_SERVER%" echo SERVER=%SERVER%
    if defined CTR_DISC echo CTR_DISC=%CTR_DISC%
    if defined UPDATE_URL if not "%UPDATE_URL%"=="%DEF_UPDATE_URL%" echo UPDATE_URL=%UPDATE_URL%
    if defined ACCOUNT_SEEN echo ACCOUNT_SEEN=1
) > "%CFG%"

set "WOW_DATA=%WOW_DIR%\Data"
set "WOW_KART_DATA=%~dp0kart-data"
if /i "%~1"=="demo" (
    start "" "%~dp0kartcraft-demo.exe"
    exit /b 0
)
if defined REGISTER_URL if not defined ACCOUNT_SEEN (
    echo.
    echo No account yet? Make one at %REGISTER_URL%
    >> "%CFG%" echo ACCOUNT_SEEN=1
    pause
)
set "WOW_HOST=%SERVER%"
start "" "%~dp0WorldOfKartcraft.exe"
exit /b 0

rem A WoW 1.12.1 folder (one with Data\dbc.MPQ) here, in a folder here, or above here. Once: a
rem found folder always passes the check, so the question only comes when none is found.
:findwow
set "WOW_SEARCHED=1"
for %%d in ("%HERE%." "%HERE%..") do if exist "%%~fd\Data\dbc.MPQ" (
    set "WOW_DIR=%%~fd"
    echo Found World of Warcraft in %%~fd
    exit /b 0
)
for /d %%d in ("%HERE%*") do if exist "%%~fd\Data\dbc.MPQ" (
    set "WOW_DIR=%%~fd"
    echo Found World of Warcraft in %%~fd
    exit /b 0
)
exit /b 0

rem A CTR disc image under this folder: a .bin or .iso of a disc's size (the US disc is about 500 MB).
rem Once: if it does not convert, the question comes as before.
:findctr
set "CTR_SEARCHED=1"
for /r "%HERE%." %%f in (*.bin *.iso) do if %%~zf GTR 300000000 (
    set "CTR_DISC=%%~ff"
    echo Found a Crash Team Racing image: %%~ff
    exit /b 0
)
exit /b 0

:convert
set "DISC=%CTR_DISC%"
if /i "%DISC:~-4%"==".cue" set "DISC=%DISC:~0,-4%.bin"
if not exist "%DISC%" (
    echo   %DISC% does not exist.
    exit /b 1
)
set "CTR_DISC=%DISC%"
echo.
echo Converting the kart, its sounds, effects, HUD and music from your disc (a few seconds)...
"%~dp0ctr-extract.exe" "%DISC%" "%~dp0kart-data" --all
if errorlevel 1 (
    echo   The conversion failed: the image must be the US disc ^(SCUS-94426^), as a raw .bin.
    exit /b 1
)
rem Which converter made kart-data: the updater converts again when a new one arrives.
powershell -NoProfile -Command "(Get-FileHash -Algorithm SHA256 -LiteralPath '%~dp0ctr-extract.exe').Hash.ToLower() | Set-Content -LiteralPath '%~dp0kart-data\.converter'"
exit /b 0
