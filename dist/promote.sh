#!/usr/bin/env bash
# Hands the players what the testers have: GitHub's Release workflow packages each build twice,
# for staging (live under staging/) and for production (waiting under candidate/, the same game
# files with the players' launcher and presets). This copies candidate/ to the players' folders,
# Windows, Mac and Linux, through dist/upload.sh; nothing is built. It refuses when candidate/ is not
# the build staging serves.
#
#   ACCOUNT=<storage account> KEY=<its key> WEB=<its website URL> dist/promote.sh
#
# FROM and TO (default candidate/ and the root) name other folders, to try it out of the players' way.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FROM="${FROM-candidate/}" TO="${TO-}"
version() { curl -fsS "$WEB/$1manifest.json" | grep -o '"version": *"[^"]*"' | cut -d'"' -f4; }

for os in "" macos/ linux/; do
    staged="$(version "staging/$os")"
    waiting="$(version "$FROM$os")"
    [ "$staged" = "$waiting" ] || { echo "$FROM$os is $waiting but staging/$os serves $staged: build again"; exit 1; }
    echo "${os:-windows/} $waiting (${TO:-the root}$os has $(version "$TO$os" 2>/dev/null || echo none))"
done

work="$(mktemp -d "${TMPDIR:-/tmp}/kartcraft-promote.XXXXXX")"
trap 'cd / && rm -rf "$work"' EXIT
cd "$work"
for os in "" macos/ linux/; do
    dir="updates-${os%/}"
    mkdir -p "$dir/files"
    curl -fsS -o "$dir/manifest.json" "$WEB/$FROM${os}manifest.json"
    for gz in $(grep -o '"gz": *"files/[0-9a-f]*\.gz"' "$dir/manifest.json" | sed 's#.*"files/##; s#"$##'); do
        curl -fsS -o "$dir/files/$gz" "$WEB/$FROM${os}files/$gz"
    done
done
curl -fsS -o launcher.exe "$WEB/${FROM}KartcraftLauncher.exe"
curl -fsS -o mac.zip "$WEB/${FROM}macos/World%20of%20Kartcraft.zip"
curl -fsS -o linux.AppImage "$WEB/${FROM}linux/Kartcraft%20Launcher.AppImage"

bash "$here/upload.sh" updates-linux "${TO}linux/" linux.AppImage "${TO}linux/Kartcraft Launcher.AppImage"
bash "$here/upload.sh" updates-macos "${TO}macos/" mac.zip "${TO}macos/World of Kartcraft.zip"
bash "$here/upload.sh" updates- "$TO" launcher.exe "${TO}KartcraftLauncher.exe"
echo "promoted"
