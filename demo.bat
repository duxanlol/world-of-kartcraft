@echo off
rem World of Kartcraft: the offline kart demo (no server). Needs kart-data\ and WoW\ here.
cd /d "%~dp0"
if exist target\release\kartcraft-demo.exe (
    target\release\kartcraft-demo.exe
) else (
    if not exist target\play\kartcraft-demo.exe cargo build --profile play -p kart-demo
    target\play\kartcraft-demo.exe
)
