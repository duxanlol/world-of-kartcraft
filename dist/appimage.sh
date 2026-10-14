#!/usr/bin/env bash
# Builds the Linux launcher as an AppImage, the Linux counterpart of dist/macos/package.sh. The
# AppImage holds only the launcher; the game it downloads (the Windows one, from the Windows
# manifest) lives beside it, in a WorldOfKartcraft folder, and runs through the player's own Proton.
#   dist/appimage.sh
# It leaves two things in dist/linux/out/:
#   Kartcraft Launcher.AppImage   what a new Linux player downloads
#   updates/                      what Linux launchers update themselves from: manifest.json (the
#                                 AppImage with its SHA-256) and the file gzip-compressed, named
#                                 by its hash. Serve it as the `linux/` folder beside the Windows
#                                 updates (a Linux launcher asks for <UPDATE_URL>/linux/manifest.json).
# .setup/publish.cfg presets it as dist/publish.ps1 presets the Windows one (SERVER, UPDATE_URL,
# REGISTER_URL). It builds in Docker; appimagetool comes from APPIMAGETOOL, the PATH, or a pinned
# download, cached.
set -euo pipefail
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo"

cfg=$repo/.setup/publish.cfg
if [ -f "$cfg" ]; then
    while IFS='=' read -r key value; do
        value=${value%$'\r'}
        case $key in
            SERVER) export KARTCRAFT_SERVER=$value ;;
            UPDATE_URL) export KARTCRAFT_UPDATE_URL=$value ;;
            REGISTER_URL) export KARTCRAFT_REGISTER_URL=$value ;;
        esac
    done <"$cfg"
fi

# Built in Debian 12 (glibc 2.36; the launcher then needs 2.35 at most), not on the machine's own
# distribution: a binary built on a rolling one needs that glibc and starts nowhere older (Ubuntu
# 22.04, SteamOS). The toolchain is the repo's (rust-toolchain.toml).
# Made here, as ours: one Docker made would be root's, and the build could not write to it.
mkdir -p "${CARGO_HOME:-$HOME/.cargo}/registry"
docker run --rm --user "$(id -u):$(id -g)" \
    -v "$repo:/src" -v "${CARGO_HOME:-$HOME/.cargo}/registry:/usr/local/cargo/registry" -w /src \
    -e CARGO_TARGET_DIR=/src/target-appimage -e CARGO_BUILD_JOBS \
    -e KARTCRAFT_SERVER -e KARTCRAFT_UPDATE_URL -e KARTCRAFT_REGISTER_URL \
    rust:1.98.1-bookworm cargo build --release -p kartcraft-launcher

tool=${APPIMAGETOOL:-$(command -v appimagetool || true)}
if [ -z "$tool" ]; then
    version=1.9.1
    tool=${XDG_CACHE_HOME:-$HOME/.cache}/kartcraft/appimagetool-$version
    if [ ! -x "$tool" ]; then
        mkdir -p "$(dirname "$tool")"
        curl -fsSL -o "$tool.part" \
            "https://github.com/AppImage/appimagetool/releases/download/$version/appimagetool-x86_64.AppImage"
        chmod +x "$tool.part" && mv "$tool.part" "$tool"
    fi
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
app=$work/AppDir
mkdir -p "$app/usr/bin"
cp target-appimage/release/kartcraft-launcher "$app/usr/bin/"
cp dist/linux/kartcraft-launcher.desktop "$app/"
# The WoK badge, the launcher's and the game's window icon too.
cp dist/icon.png "$app/kartcraft-launcher.png"
cat >"$app/AppRun" <<'RUN'
#!/bin/sh
exec "$APPDIR/usr/bin/kartcraft-launcher" "$@"
RUN
chmod +x "$app/AppRun"

out=$repo/dist/linux/out
image="$out/Kartcraft Launcher.AppImage"
mkdir -p "$out"
# The tool is an AppImage itself; where FUSE is missing it runs extracted.
ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 "$tool" --no-appstream "$app" "$image"

# ── updates/ ─────────────────────────────────────────────────────────────────────────────────────
# As package.sh writes the Mac one: the manifest written last and whole, and the objects no
# manifest names any more gone, the previous publish's kept for a launcher mid-download.
updates=$out/updates
objects=$updates/files
mkdir -p "$objects"
sha=$(sha256sum "$image" | cut -d' ' -f1)
gz=$objects/$sha.gz
[ -f "$gz" ] || gzip -9 -n -c "$image" >"$gz"
cat >"$updates/manifest.json.tmp" <<JSON
{
  "version": "$(date +%Y.%m.%d-%H%M)",
  "files": [
    {"path": "Kartcraft Launcher.AppImage", "sha256": "$sha", "size": $(stat -c %s "$image"), "gz": "files/$sha.gz", "gz_size": $(stat -c %s "$gz")}
  ]
}
JSON
mv -f "$updates/manifest.json.tmp" "$updates/manifest.json"
keep="$(echo "$sha.gz"; [ -f "$updates/previous.txt" ] && cat "$updates/previous.txt"; true)"
for obj in "$objects"/*.gz; do
    grep -qxF "$(basename "$obj")" <<<"$keep" || rm -f "$obj"
done
echo "$sha.gz" >"$updates/previous.txt"
echo "Built $image and $updates"
