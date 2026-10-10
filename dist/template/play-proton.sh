#!/usr/bin/env bash
# Linux stand-in for "Play Kartcraft.bat": Proton has no PowerShell, so the update and first-run
# steps run here in bash and only the .exe files go through Proton. "./play-proton.sh demo" = offline demo.
set -euo pipefail
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
cd "$ROOT"

PROTON=${PROTON:-$HOME/.local/share/Steam/compatibilitytools.d/GE-Proton10-34/proton}
WOW_DIR=${WOW_DIR:-$ROOT/../World of Warcraft 1.12}
CTR_DISC=${CTR_DISC:-$ROOT/../CTR - Crash Team Racing (USA)/CTR - Crash Team Racing (USA).bin}
export STEAM_COMPAT_DATA_PATH=$ROOT/.proton-prefix
export STEAM_COMPAT_CLIENT_INSTALL_PATH=$HOME/.local/share/Steam
# EXE=WorldOfKartcraft-test.exe runs a local build. The published exe still crashes on a controller
# under Wine (gilrs unwraps NonRoamableId), so for it WGI is off: keyboard only until the fix ships.
EXE=${EXE:-WorldOfKartcraft.exe}
[ "$EXE" = WorldOfKartcraft.exe ] && export WINEDLLOVERRIDES="windows.gaming.input=d${WINEDLLOVERRIDES:+;$WINEDLLOVERRIDES}"
# Wine's WGI hands a DS4 (054c:09cc) its axes as RY RX LY LX R2 L2, not the order SDL's Windows
# mapping expects; measured with padprobe. ponytail: this pad only, add a line per pad that needs it.
export SDL_GAMECONTROLLERCONFIG="030000004c050000cc09000000000000,PS4 Controller,a:b1,b:b2,back:b8,dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,guide:b12,leftshoulder:b4,leftstick:b10,lefttrigger:a5,leftx:a3,lefty:a2,rightshoulder:b5,rightstick:b11,righttrigger:a4,rightx:a1,righty:a0,start:b9,touchpad:b13,x:b0,y:b3,platform:Windows,"
mkdir -p "$STEAM_COMPAT_DATA_PATH"
set -a; source <(tr -d '\r' < defaults.cfg); set +a

winpath() { echo "Z:$(realpath "$1" | tr / '\\')"; }
proton() { local exe=$1; shift; "$PROTON" run "$ROOT/$exe" "$@"; }
convert() { proton ctr-extract.exe "$(winpath "$CTR_DISC")" "$(winpath "$ROOT")\\kart-data" --all; }

# Same logic as update.ps1: fetch every manifest file whose SHA-256 differs here.
update() {
    local m; m=$(curl -fsS --max-time 8 "$UPDATE_URL/manifest.json") || { echo "(no update server; starting as is)"; return; }
    jq -r '.files[] | [.path, .sha256, .gz] | @tsv' <<<"$m" | while IFS=$'\t' read -r path sha gz; do
        [ "$(sha256sum "$path" 2>/dev/null | cut -d' ' -f1)" = "$sha" ] && continue
        echo "  updating $path"
        mkdir -p "$(dirname "$path")"
        curl -fsS "$UPDATE_URL/$gz" | gunzip > "$path.download"
        [ "$(sha256sum "$path.download" | cut -d' ' -f1)" = "$sha" ] || { rm -f "$path.download"; echo "  $path damaged, skipped"; continue; }
        mv "$path.download" "$path"
    done
    jq -r .version <<<"$m" > version.txt
    local conv; conv=$(jq -r .converter <<<"$m")
    if [ -f kart-data/kart.json ] && [ "$(cat kart-data/.converter 2>/dev/null)" != "$conv" ]; then
        echo "Converter updated, converting the disc again..."
        convert && echo "$conv" > kart-data/.converter
    fi
}

[ -f "$WOW_DIR/Data/dbc.MPQ" ] || { echo "No dbc.MPQ in $WOW_DIR/Data (set WOW_DIR)"; exit 1; }
update
if [ ! -f kart-data/kart.json ]; then
    echo "Converting the kart from your CTR disc..."
    convert
    sha256sum ctr-extract.exe | cut -d' ' -f1 > kart-data/.converter
fi

export WOW_DATA=$(winpath "$WOW_DIR/Data") WOW_KART_DATA=$(winpath kart-data) WOW_HOST=$SERVER
if [ "${1:-}" = demo ]; then exec "$PROTON" run "$ROOT/kartcraft-demo.exe"; fi
[ -f .account-seen ] || { echo "No account yet? Make one at $REGISTER_URL"; xdg-open "$REGISTER_URL" >/dev/null 2>&1 || true; touch .account-seen; }
exec "$PROTON" run "$ROOT/$EXE"
