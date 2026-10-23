#!/usr/bin/env bash
# The website replay viewer's game world (crates/benilla-web) for the browser: built for wasm32 and
# bound with wasm-bindgen (the CLI at the version Cargo.lock has), into the site's site/world/pkg.
# sccache is bypassed: web-sys's command line is past Windows' limit for it.
#
#   scripts/web-world.sh [site checkout, default ../worldofkartcraft-site]
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
site="${1:-$here/../worldofkartcraft-site}"
want="$(grep -A1 '^name = "wasm-bindgen"$' "$here/Cargo.lock" | sed -n 's/^version = "\(.*\)"/\1/p')"
have="$(wasm-bindgen --version 2>/dev/null | awk '{print $2}')"
[ "$have" = "$want" ] || { echo "wasm-bindgen CLI $have, Cargo.lock has $want: cargo install wasm-bindgen-cli --version $want --locked" >&2; exit 1; }
cd "$here"
RUSTC_WRAPPER= CARGO_BUILD_RUSTC_WRAPPER= cargo build -p benilla-web --target wasm32-unknown-unknown --release
out="$site/site/world/pkg"
mkdir -p "$out"
wasm-bindgen --target web --no-typescript --out-dir "$out" \
    "${CARGO_TARGET_DIR:-$here/target}/wasm32-unknown-unknown/release/benilla_web.wasm"
ls -la "$out"
