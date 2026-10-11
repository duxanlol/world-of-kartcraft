#!/usr/bin/env bash
# World of Kartcraft setup (Linux). Run from anywhere:
#
#   setup/setup.sh                       # everything, asking for the WoW folder and the CTR disc
#   setup/setup.sh --wow DIR --ctr DISC  # same, without asking
#   setup/setup.sh --doctor              # only check this machine
#
# Every step is skipped when its result is already there, so after a failure fix the reported
# problem and run it again. Everything is logged to setup.log.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT"

# NixOS has no system libraries for the build: re-run inside the repo's nix shell.
if [ -e /etc/NIXOS ] && [ -z "${IN_NIX_SHELL:-}" ]; then
    exec nix-shell "$ROOT/shell.nix" --run "$(printf '%q ' "$0" "$@")"
fi

STATE="$ROOT/.setup"
mkdir -p "$STATE/bin"
export PATH="$STATE/bin:$PATH"
exec > >(tee -a "$ROOT/setup.log") 2>&1

# Pinned upstreams.
SERVER_URL=https://github.com/mserajnik/vmangos-deploy
SERVER_REV=f0692574b7955e78d68fea39372e0045360ce5ee
SERVER_IMAGE=ghcr.io/mserajnik/vmangos-server:5875

step() { printf '\n==> %s\n' "$*"; }
die() {
    printf '\nSETUP STOPPED: %s\n' "$1"
    [ $# -gt 1 ] && printf 'Fix: %s\n' "$2"
    printf 'Then run setup/setup.sh again; finished steps are skipped.\n'
    exit 1
}

WOW_DIR="" CTR_DISC="" DOCTOR_ONLY=0
while [ $# -gt 0 ]; do
    case "$1" in
        --wow) WOW_DIR=$2; shift 2 ;;
        --ctr) CTR_DISC=$2; shift 2 ;;
        --doctor) DOCTOR_ONLY=1; shift ;;
        -h|--help) sed -n 2,10p "$0"; exit 0 ;;
        *) die "unknown option $1" "see setup/setup.sh --help" ;;
    esac
done

# ── Doctor ─────────────────────────────────────────────────────────────────────────────────────
distro() {
    . /etc/os-release 2>/dev/null || true
    case " ${ID:-} ${ID_LIKE:-} " in
        *" nixos "*) echo nixos ;;
        *" arch "*) echo arch ;;
        *" fedora "*|*" rhel "*) echo fedora ;;
        *" debian "*|*" ubuntu "*) echo debian ;;
        *) echo other ;;
    esac
}
packages_hint() {
    case "$(distro)" in
        debian) echo "sudo apt install build-essential pkg-config libasound2-dev libudev-dev git curl python3 python3-venv docker.io docker-compose-v2" ;;
        arch) echo "sudo pacman -S --needed base-devel pkgconf alsa-lib systemd-libs git curl python docker docker-compose" ;;
        fedora) echo "sudo dnf install gcc pkgconf-pkg-config alsa-lib-devel systemd-devel git curl python3 docker docker-compose-plugin" ;;
        nixos) echo "enable Docker in configuration.nix (virtualisation.docker.enable = true;); everything else comes from shell.nix" ;;
        *) echo "install: a C compiler, pkg-config, ALSA and libudev development files, git, curl, python3 with venv, Docker with the compose plugin" ;;
    esac
}

doctor() {
    step "Checking this machine ($(distro))"
    local bad=0
    check() { # name, test command, fix
        if eval "$2" >/dev/null 2>&1; then printf '  ok       %s\n' "$1"
        else printf '  MISSING  %s\n           fix: %s\n' "$1" "$3"; bad=1; fi
    }
    check "Rust (rustup/cargo)" "command -v cargo" "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh   (then open a new terminal)"
    check "C compiler" "command -v cc" "$(packages_hint)"
    check "pkg-config + ALSA + libudev" "pkg-config --exists alsa libudev" "$(packages_hint)"
    check "git" "command -v git" "$(packages_hint)"
    check "curl" "command -v curl" "$(packages_hint)"
    check "python3 with venv" "python3 -c 'import venv, ensurepip'" "$(packages_hint)"
    check "Docker" "command -v docker" "$(packages_hint)"
    check "Docker compose plugin" "docker compose version" "$(packages_hint)"
    check "Docker running and usable by you" "docker info" \
        "start Docker (sudo systemctl enable --now docker) and add yourself to its group: sudo usermod -aG docker \$USER, then log out and back in"
    local free
    free=$(df -P -BG "$ROOT" | awk 'NR==2 {gsub("G","",$4); print $4}')
    check "20 GB free disk (${free} GB)" "[ ${free:-0} -ge 20 ]" "free up disk space"
    if [ -e /dev/input/by-id ] && ! ls /dev/input/by-id/*event-joystick >/dev/null 2>&1; then
        printf '  note     no controller found right now; plug in an Xbox-style pad before playing\n'
    fi
    return $bad
}

if [ "$DOCTOR_ONLY" = 1 ]; then
    doctor && echo "All good." || exit 1
    exit 0
fi
doctor || die "this machine is missing the tools listed above" "install them, then run setup again"

ask_dir() { # var, prompt
    local value=${!1}
    while [ -z "$value" ]; do
        read -r -p "$2: " value </dev/tty
        value=${value/#\~/$HOME}
    done
    printf -v "$1" '%s' "$(realpath -m "$value")"
}

# ── WoW client ─────────────────────────────────────────────────────────────────────────────────
if [ ! -e "$ROOT/WoW/Data" ]; then
    step "WoW 1.12.1 client"
    ask_dir WOW_DIR "Path to your WoW 1.12.1 (English, build 5875) folder, the one with Data/ in it"
    [ "$(basename "$WOW_DIR")" = Data ] && WOW_DIR=$(dirname "$WOW_DIR")
    for mpq in dbc.MPQ patch.MPQ model.MPQ terrain.MPQ; do
        ls "$WOW_DIR/Data/" 2>/dev/null | grep -qix "$mpq" ||
            die "$WOW_DIR/Data has no $mpq" "point setup at an untouched English 1.12.1 client folder"
    done
    ln -sfn "$WOW_DIR" "$ROOT/WoW"
    echo "  linked WoW -> $WOW_DIR"
fi

# ── The kart, from the player's CTR disc ───────────────────────────────────────────────────────
CTR_FILE="$STATE/ctr-path"
[ -z "$CTR_DISC" ] && [ -f "$CTR_FILE" ] && CTR_DISC=$(cat "$CTR_FILE")
if [ ! -f "$ROOT/kart-data/kart.json" ]; then
    step "Converting the kart from your Crash Team Racing disc"
    while [ -z "$CTR_DISC" ]; do
        read -r -p "Path to your Crash Team Racing (PS1, USA, SCUS-94426) disc image, the .bin file: " CTR_DISC </dev/tty
    done
    CTR_DISC=${CTR_DISC%.cue}
    CTR_DISC=${CTR_DISC%.bin}.bin
    [ -f "$CTR_DISC" ] || die "$CTR_DISC does not exist" "point setup at the .bin of a raw (2352-byte sector) CTR NTSC-U image"
    echo "$CTR_DISC" > "$CTR_FILE"
    cargo run --release --quiet -p ctr-assets --bin ctr-extract -- "$CTR_DISC" "$ROOT/kart-data" --all ||
        die "the kart conversion failed (see the lines above)" "check that the image is the NTSC-U disc"
    [ -f "$ROOT/kart-data/kart.json" ] || die "the conversion finished without kart.json"
fi

# ── Local server ───────────────────────────────────────────────────────────────────────────────
SERVER="$ROOT/server"
# One fixed project name, so the server is the same Docker project whatever the folder is called.
export KARTCRAFT_SERVER_PROJECT=${KARTCRAFT_SERVER_PROJECT:-world-of-kartcraft}
compose() { docker compose -p "$KARTCRAFT_SERVER_PROJECT" --project-directory "$SERVER" -f "$SERVER/compose.yaml" "$@"; }
if [ ! -f "$SERVER/compose.yaml" ]; then
    step "Setting up the local vmangos server"
    [ -d "$SERVER/.git" ] || git clone --quiet "$SERVER_URL" "$SERVER"
    git -C "$SERVER" checkout --quiet "$SERVER_REV"
    cp "$SERVER/config/mangosd.conf.example" "$SERVER/config/mangosd.conf"
    cp "$SERVER/config/realmd.conf.example" "$SERVER/config/realmd.conf"
    # Horde and Alliance race together: a PvP realm lets an account have both only with this.
    sed -i 's/^AllowTwoSide\.Accounts = 0/AllowTwoSide.Accounts = 1/' "$SERVER/config/mangosd.conf"
    TZ_NAME=$(timedatectl show -p Timezone --value 2>/dev/null || cat /etc/timezone 2>/dev/null || echo Etc/UTC)
    sed -e "s#TZ=Etc/UTC#TZ=${TZ_NAME}#" -e "s#user: 1000:1000#user: $(id -u):$(id -g)#" \
        "$SERVER/compose.yaml.example" > "$SERVER/compose.yaml"
fi
if [ ! -d "$SERVER/storage/mangosd/extracted-data/maps" ]; then
    step "Extracting server maps from your WoW client (this takes HOURS; leave it running)"
    docker pull --quiet "$SERVER_IMAGE" >/dev/null
    docker run --rm --user "$(id -u):$(id -g)" \
        -v "$(realpath "$ROOT/WoW")":/opt/vmangos/storage/client-data \
        -v "$SERVER/storage/mangosd/extracted-data":/opt/vmangos/storage/extracted-data \
        "$SERVER_IMAGE" extract-client-data </dev/null ||
        die "map extraction failed (see the lines above)"
fi
step "Building the server with the Karting profession (a few minutes)"
"$ROOT/setup/build-server.sh" "$SERVER" || die "the server build failed (see the lines above)"
# The Race Marshals (setup/vmangos/sql/kartcraft.sql): the database container runs custom-sql/ on
# every start, so a changed file restarts it, with the world server stopped meanwhile.
CUSTOM_SQL="$SERVER/storage/database/custom-sql/kartcraft.sql"
if ! cmp -s "$ROOT/setup/vmangos/sql/kartcraft.sql" "$CUSTOM_SQL"; then
    mkdir -p "$(dirname "$CUSTOM_SQL")"
    cp "$ROOT/setup/vmangos/sql/kartcraft.sql" "$CUSTOM_SQL"
    echo "  Race Marshals: storage/database/custom-sql/kartcraft.sql"
    if [ -n "$(compose ps -q database 2>/dev/null)" ]; then
        compose stop mangosd
        compose restart database
    fi
fi
step "Starting the server"
compose up -d
printf '  waiting for the world server'
for _ in $(seq 120); do
    [ "$(docker inspect -f '{{.State.Health.Status}}' "$(compose ps -q mangosd)" 2>/dev/null)" = healthy ] && break
    printf '.'; sleep 5
done
echo
[ "$(docker inspect -f '{{.State.Health.Status}}' "$(compose ps -q mangosd)" 2>/dev/null)" = healthy ] ||
    die "the world server did not come up within 10 minutes" "look at: docker compose -p world-of-kartcraft -f server/compose.yaml logs mangosd"

# ── Account ────────────────────────────────────────────────────────────────────────────────────
if [ ! -f "$STATE/account" ]; then
    step "Game account"
    read -r -p "Account name (letters and digits): " ACCOUNT </dev/tty
    read -r -s -p "Password: " PASSWORD </dev/tty; echo
    python3 "$ROOT/tools/make_account.py" "$ACCOUNT" "$PASSWORD" --server "$SERVER" ||
        die "could not create the account"
    echo "$ACCOUNT" > "$STATE/account"
fi

# ── Build ──────────────────────────────────────────────────────────────────────────────────────
step "Building the game (first time: several minutes)"
cargo build --release -p benilla -p kart-demo || die "the build failed (see the lines above)" "$(packages_hint)"

step "Done"
echo "  Start the game with:  ./play.sh      (the offline kart demo, no server: ./target/release/kartcraft-demo)"
echo "  Log in as $(cat "$STATE/account"), create a character, press J to drive (PS5, PS4 or Xbox pad, or keyboard)."
