#!/usr/bin/env bash
# Starts the local server (if it is not running) and the game. Run setup/setup.sh first.
# In game: J climbs into / out of the kart (PS5, PS4 or Xbox pad, or keyboard), K swaps the chase camera.
set -euo pipefail
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
cd "$ROOT"
if [ -e /etc/NIXOS ] && [ -z "${IN_NIX_SHELL:-}" ]; then
    exec nix-shell "$ROOT/shell.nix" --run "$(printf '%q ' "$0" "$@")"
fi
[ -x target/release/benilla ] || { echo "Not built yet: run setup/setup.sh first."; exit 1; }
docker compose -p "${KARTCRAFT_SERVER_PROJECT:-world-of-kartcraft}" --project-directory server -f server/compose.yaml up -d
export WOW_HOST=localhost
export WOW_KART_DATA="$ROOT/kart-data"
exec target/release/benilla "$@"
