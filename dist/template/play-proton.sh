#!/usr/bin/env bash
# Linux and Steam Deck stand-in for "Play Kartcraft.bat": Proton has no PowerShell, so the update
# and first-run steps run here in bash and only the .exe files go through Proton.
#   bash play-proton.sh         the game        bash play-proton.sh demo     the offline demo
# The first run asks for your WoW 1.12.1 folder and your CTR disc image and keeps them in
# kartcraft-linux.cfg (delete it to be asked again). PROTON=/path/to/proton picks a Proton build.
# Needs curl, jq and sha256sum.
set -euo pipefail
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
cd "$ROOT"
CFG=$ROOT/kartcraft-linux.cfg

# KEY=value lines of the keys we know; read, never sourced: defaults.cfg comes with updates.
read_cfg() {
    local key value
    [ -f "$1" ] || return 0
    while IFS='=' read -r key value; do
        value=${value%$'\r'}
        case $key in
            SERVER | UPDATE_URL | REGISTER_URL | WOW_DIR | CTR_DISC | PROTON | ACCOUNT_SEEN)
                printf -v "$key" '%s' "$value" ;;
        esac
    done <"$1"
}
SERVER='' UPDATE_URL='' REGISTER_URL='' WOW_DIR='' CTR_DISC='' ACCOUNT_SEEN=''
PROTON=${PROTON:-}
read_cfg defaults.cfg
read_cfg "$CFG"

for tool in curl jq sha256sum; do
    command -v "$tool" >/dev/null || { echo "Please install $tool first."; exit 1; }
done

# The last match wins: Steam's own Proton builds, then GE-Proton (the newest by name).
if [ -z "$PROTON" ]; then
    for p in "$HOME"/.local/share/Steam/steamapps/common/Proton*/proton \
             "$HOME"/.steam/steam/steamapps/common/Proton*/proton \
             "$HOME"/.steam/root/compatibilitytools.d/*/proton \
             "$HOME"/.local/share/Steam/compatibilitytools.d/*/proton; do
        if [ -x "$p" ]; then PROTON=$p; fi
    done
fi
[ -n "$PROTON" ] && [ -x "$PROTON" ] || { echo "No Proton found: install one through Steam, or set PROTON=/path/to/proton."; exit 1; }
STEAM_DIR=$HOME/.local/share/Steam
[ -d "$STEAM_DIR" ] || STEAM_DIR=$HOME/.steam/steam
export STEAM_COMPAT_DATA_PATH=$ROOT/.proton-prefix
export STEAM_COMPAT_CLIENT_INSTALL_PATH=${STEAM_COMPAT_CLIENT_INSTALL_PATH:-$STEAM_DIR}
mkdir -p "$STEAM_COMPAT_DATA_PATH"
# Vulkan, which Proton passes straight through, unless set: left to itself the game may take DX12
# (a Steam Deck does), whose shader compiler under Proton fails on its compute shaders.
export WGPU_BACKEND=${WGPU_BACKEND:-vulkan}
# Wine's WGI hands a DS4 (054c:09cc) its axes as RY RX LY LX R2 L2, not the order SDL's Windows
# mapping expects (measured under GE-Proton 10). One line per pad that needs it.
export SDL_GAMECONTROLLERCONFIG="030000004c050000cc09000000000000,PS4 Controller,a:b1,b:b2,back:b8,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,guide:b12,leftshoulder:b4,leftstick:b10,lefttrigger:a5,leftx:a3,lefty:a2,rightshoulder:b5,rightstick:b11,righttrigger:a4,rightx:a1,righty:a0,start:b9,touchpad:b13,x:b0,y:b3,platform:Windows,"

winpath() { echo "Z:$(realpath "$1" | tr / '\\')"; }
convert() {
    "$PROTON" run "$ROOT/ctr-extract.exe" "$(winpath "$CTR_DISC")" "$(winpath "$ROOT")\\kart-data" --all
}

# The same as update.ps1: every manifest file whose SHA-256 differs here, gzip-compressed and
# hash-checked, swapped in with mv (so this script, if it is one of them, keeps running its old
# copy and is started again afterwards).
update() {
    local m self_before
    [ -n "$UPDATE_URL" ] || return 0
    m=$(curl -fsS --max-time 8 "$UPDATE_URL/manifest.json") || { echo "  (no update server at $UPDATE_URL; starting as is)"; return 0; }
    self_before=$(sha256sum play-proton.sh | cut -d' ' -f1)
    while IFS=$'\t' read -r path sha gz; do
        [ "$(sha256sum "$path" 2>/dev/null | cut -d' ' -f1)" = "$sha" ] && continue
        echo "  updating $path"
        mkdir -p "$(dirname "$path")"
        if ! curl -fsS "$UPDATE_URL/$gz" | gunzip >"$path.download" ||
            [ "$(sha256sum "$path.download" | cut -d' ' -f1)" != "$sha" ]; then
            rm -f "$path.download"
            echo "  the download of $path is damaged; try again"
            return 0
        fi
        mv "$path.download" "$path"
    done < <(jq -r '.files[] | [.path, .sha256, .gz] | @tsv' <<<"$m")
    jq -r .version <<<"$m" >version.txt
    local conv
    conv=$(jq -r .converter <<<"$m")
    if [ -f kart-data/kart.json ] && [ -n "$CTR_DISC" ] && [ "$(cat kart-data/.converter 2>/dev/null)" != "$conv" ]; then
        echo "Converting the kart data from your disc again (the converter was updated)..."
        convert && echo "$conv" >kart-data/.converter
    fi
    if [ "$(sha256sum play-proton.sh | cut -d' ' -f1)" != "$self_before" ]; then
        exec bash "$ROOT/play-proton.sh" "$@"
    fi
}

save() {
    {
        echo "WOW_DIR=$WOW_DIR"
        echo "CTR_DISC=$CTR_DISC"
        [ -n "$ACCOUNT_SEEN" ] && echo "ACCOUNT_SEEN=1"
        true
    } >"$CFG"
}

while [ ! -f "$WOW_DIR/Data/dbc.MPQ" ]; do
    [ -n "$WOW_DIR" ] && echo "  $WOW_DIR has no Data/dbc.MPQ: it needs to be an English 1.12.1 client."
    read -rp "Your World of Warcraft 1.12.1 folder (the one with Data in it): " WOW_DIR
    WOW_DIR=${WOW_DIR%/}
    WOW_DIR=${WOW_DIR%/Data}
done
# Kept before the update, which may start a new copy of this script.
save

update "$@"

while [ ! -f kart-data/kart.json ]; do
    if [ ! -f "$CTR_DISC" ]; then
        [ -n "$CTR_DISC" ] && echo "  $CTR_DISC does not exist."
        read -rp "Your Crash Team Racing disc image (the US release, .bin or .iso): " CTR_DISC
        [ "${CTR_DISC%.cue}" != "$CTR_DISC" ] && CTR_DISC=${CTR_DISC%.cue}.bin
        continue
    fi
    echo "Converting the kart, its sounds, effects, HUD and music from your disc (a few seconds)..."
    if convert; then
        sha256sum ctr-extract.exe | cut -d' ' -f1 >kart-data/.converter
    else
        echo "  The conversion failed: the image must be the US disc (SCUS-94426)."
        CTR_DISC=''
    fi
done
save

export WOW_DATA WOW_KART_DATA WOW_HOST=$SERVER
WOW_DATA=$(winpath "$WOW_DIR/Data")
WOW_KART_DATA=$(winpath kart-data)
if [ "${1:-}" = demo ]; then
    exec "$PROTON" run "$ROOT/kartcraft-demo.exe"
fi
if [ -n "$REGISTER_URL" ] && [ -z "$ACCOUNT_SEEN" ]; then
    echo "No account yet? Make one at $REGISTER_URL"
    xdg-open "$REGISTER_URL" >/dev/null 2>&1 || true
    ACCOUNT_SEEN=1
    save
fi
exec "$PROTON" run "$ROOT/WorldOfKartcraft.exe"
