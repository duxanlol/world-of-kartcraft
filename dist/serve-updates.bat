@echo off
rem Serves dist\updates over HTTP on port 8080, where your friends' launchers look for updates
rem (http://<the server address they entered>:8080). It runs in Docker, beside the game server,
rem and restarts with Docker, so this is needed once. Forward TCP 8080 on your router as well as
rem 3724 and 8085. "serve-updates.bat stop" stops it.
setlocal
cd /d "%~dp0"
docker rm -f kartcraft-updates >nul 2>&1
if /i "%~1"=="stop" (
    echo Update server stopped.
    exit /b 0
)
if not exist "%~dp0updates\manifest.json" (
    echo Nothing published yet: run publish.bat first.
    exit /b 1
)
docker run -d --name kartcraft-updates --restart unless-stopped -p 8080:80 ^
    -v "%~dp0updates:/usr/share/nginx/html:ro" nginx:1.27-alpine >nul
if errorlevel 1 (
    echo Docker could not start the update server. Is Docker Desktop running?
    exit /b 1
)
echo Serving updates at http://localhost:8080/manifest.json
