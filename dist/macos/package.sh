#!/usr/bin/env bash
# Builds the World of Kartcraft client for macOS and packages it, the Mac counterpart of
# dist\publish.ps1 (bash only: no PowerShell on a Mac).
#
#   dist/macos/package.sh            build both architectures, bundle, sign, zip, publish folder
#   dist/macos/package.sh --no-build  reuse target-dist's binaries
#   dist/macos/package.sh --launcher-only  reuse target-dist's game binaries, build the launcher
#                                     (another channel's package from the same build)
#
# PUBLISH_CFG names the presets file (default .setup/publish.cfg), KARTCRAFT_OUT the folder it
# leaves its three things in (default dist/macos/out), VERSION the stamp (default now): GitHub's
# Release workflow packages one build for both channels with them.
#
# It leaves three things in dist/macos/out/:
#   World of Kartcraft.app   the bundle: a universal (Apple Silicon + Intel) client, the launcher,
#                            the demo and the disc converter, ad-hoc signed
#   World of Kartcraft.zip   the bundle zipped with ditto: what a new Mac player downloads
#   updates/                 what Mac launchers download from: manifest.json (each file of the
#                            bundle with its SHA-256) and the files gzip-compressed, named by
#                            their hash. Serve it as the `macos/` folder beside the Windows
#                            updates (a Mac launcher asks for <UPDATE_URL>/macos/manifest.json),
#                            so a Mac launcher never fetches a Windows file.
# No game data goes into any of them: the kart data is converted on each player's Mac.
#
# .setup/publish.cfg (yours, never committed) presets the package as on Windows, one KEY=value
# per line: SERVER, UPDATE_URL, REGISTER_URL. The launcher carries them in itself and a
# defaults.cfg in the bundle's Resources repeats them.
#
# Signing: the bundle is ad-hoc signed (`codesign -s -`), which Apple Silicon needs to run it at
# all. Without a Developer ID, Gatekeeper blocks the first open; dist/macos/README.txt tells
# players how to open it once. The DEVELOPER_ID step below is written but off until the owner
# has an Apple Developer account.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
dist="$repo/dist/macos"
out="${KARTCRAFT_OUT:-$dist/out}"
mkdir -p "$out" && out="$(cd "$out" && pwd)"
app="$out/World of Kartcraft.app"
updates="$out/updates"
target_dir="$repo/target-dist"
targets=(aarch64-apple-darwin x86_64-apple-darwin)
# macOS 11 is the first with Apple Silicon and the oldest wgpu's Metal backend serves.
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}"

build=1
[ "${1:-}" = "--no-build" ] && build=0
[ "${1:-}" = "--launcher-only" ] && build=launcher
cfg="${PUBLISH_CFG:-$repo/.setup/publish.cfg}"

# ── Presets ──────────────────────────────────────────────────────────────────────────────────────
SERVER="" UPDATE_URL="" REGISTER_URL="" DEVELOPER_ID=""
if [ -f "$cfg" ]; then
    while IFS='=' read -r key value; do
        value="${value%$'\r'}"
        case "$key" in
        SERVER | UPDATE_URL | REGISTER_URL | DEVELOPER_ID) printf -v "$key" '%s' "$value" ;;
        esac
    done <"$cfg"
fi

# ── Build ────────────────────────────────────────────────────────────────────────────────────────
if [ "$build" != 0 ]; then
    cd "$repo"
    export CARGO_TARGET_DIR="$target_dir"
    for t in "${targets[@]}"; do
        rustup target add "$t" >/dev/null 2>&1 || true
        if [ "$build" = 1 ]; then
            echo "== Building for $t (player build) =="
            cargo build --release --target "$t" -p benilla --no-default-features
            cargo build --release --target "$t" -p kart-demo -p ctr-assets
        fi
        echo "== Building the launcher for $t (server ${SERVER:-none}) =="
        # The launcher carries the presets in itself: it is one file, with no defaults.cfg beside it.
        KARTCRAFT_SERVER="$SERVER" KARTCRAFT_UPDATE_URL="$UPDATE_URL" KARTCRAFT_REGISTER_URL="$REGISTER_URL" \
            cargo build --release --target "$t" -p kartcraft-launcher
    done
fi

# ── The bundle ───────────────────────────────────────────────────────────────────────────────────
echo "== Assembling $app =="
rm -rf "$app"
macos="$app/Contents/MacOS"
resources="$app/Contents/Resources"
mkdir -p "$macos" "$resources"

# One universal binary per program, from the two builds.
join() { # $1 = cargo binary name, $2 = name in the bundle
    local inputs=()
    for t in "${targets[@]}"; do inputs+=("$target_dir/$t/release/$1"); done
    lipo -create "${inputs[@]}" -output "$macos/$2"
    # The symbol table goes (cargo's `release` keeps it on macOS): a third of the client's size.
    strip -S -x "$macos/$2"
    chmod 755 "$macos/$2"
}
join benilla WorldOfKartcraft
join kartcraft-demo kartcraft-demo
join ctr-extract ctr-extract
join kartcraft-launcher "Kartcraft Launcher"

version="${VERSION:-$(date +%Y.%m.%d-%H%M)}"
# The launcher is the bundle's executable: a double-click opens it, and it starts the game.
cat >"$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key>
	<string>en</string>
	<key>CFBundleExecutable</key>
	<string>Kartcraft Launcher</string>
	<key>CFBundleIdentifier</key>
	<string>com.worldofkartcraft.client</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>CFBundleName</key>
	<string>World of Kartcraft</string>
	<key>CFBundleDisplayName</key>
	<string>World of Kartcraft</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>$version</string>
	<key>CFBundleVersion</key>
	<string>$version</string>
	<key>CFBundleIconFile</key>
	<string>WorldOfKartcraft</string>
	<key>LSMinimumSystemVersion</key>
	<string>$MACOSX_DEPLOYMENT_TARGET</string>
	<key>LSApplicationCategoryType</key>
	<string>public.app-category.racing-games</string>
	<key>NSHighResolutionCapable</key>
	<true/>
	<key>NSSupportsAutomaticGraphicsSwitching</key>
	<true/>
	<key>GCSupportsControllerUserInteraction</key>
	<true/>
	<key>NSHumanReadableCopyright</key>
	<string>World of Kartcraft contributors. GPL-3.0-or-later; see NOTICE.txt.</string>
</dict>
</plist>
PLIST
printf 'APPL????' >"$app/Contents/PkgInfo"

# The icon: an .icns from dist/macos/icon.icns, or made from dist/macos/icon.png (square, 1024 px)
# or a Windows .ico anywhere under dist/. The repo ships none today, so the bundle gets macOS's
# generic icon until one is added here.
icon_src=""
for candidate in "$dist/icon.icns" "$dist/icon.png" "$dist/icon.ico" "$repo/dist/icon.ico"; do
    [ -f "$candidate" ] && icon_src="$candidate" && break
done
if [ -n "$icon_src" ]; then
    case "$icon_src" in
    *.icns) cp "$icon_src" "$resources/WorldOfKartcraft.icns" ;;
    *)
        iconset="$(mktemp -d "${TMPDIR:-/tmp}/kartcraft-iconset.XXXXXX")/WorldOfKartcraft.iconset"
        mkdir -p "$iconset"
        base="$(mktemp "${TMPDIR:-/tmp}/kartcraft-icon.XXXXXX").png"
        sips -s format png "$icon_src" --out "$base" >/dev/null
        for size in 16 32 128 256 512; do
            sips -z "$size" "$size" "$base" --out "$iconset/icon_${size}x${size}.png" >/dev/null
            sips -z "$((size * 2))" "$((size * 2))" "$base" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
        done
        iconutil -c icns "$iconset" -o "$resources/WorldOfKartcraft.icns"
        rm -rf "$(dirname "$iconset")" "$base"
        ;;
    esac
else
    echo "   (no icon: put icon.png or icon.icns in dist/macos/ to give the app one)"
fi

cp "$repo/LICENSE-MIT" "$resources/LICENSE-MIT.txt"
cp "$repo/LICENSE-APACHE" "$resources/LICENSE-APACHE.txt"
cp "$repo/LICENSE-GPL-3.0" "$resources/LICENSE-GPL-3.0.txt"
cp "$repo/NOTICE" "$resources/NOTICE.txt"
cp "$dist/README.txt" "$resources/README.txt"
# The launcher's presets; a player's own kartcraft.cfg overrides them.
{
    [ -n "$SERVER" ] && echo "SERVER=$SERVER"
    [ -n "$UPDATE_URL" ] && echo "UPDATE_URL=$UPDATE_URL"
    [ -n "$REGISTER_URL" ] && echo "REGISTER_URL=$REGISTER_URL"
    true
} >"$resources/defaults.cfg"

# GPL: the complete source of this build ships beside it, as the Windows package does: git's
# files when the checkout is one, minus build output and local state.
echo "== source.zip =="
src="$(mktemp -d "${TMPDIR:-/tmp}/kartcraft-source.XXXXXX")"
(
    cd "$repo"
    if [ -d .git ]; then git ls-files -co --exclude-standard -z; else find . -type f -print0; fi
) | grep -zEv '^(\./)?(target[^/]*|dist/macos/out|dist/WorldOfKartcraft|dist/updates|server|kart-data|benilla-config|WoW|extracted|out|\.setup|\.claude|\.git|wow-addons-vanilla)(/|$)|__pycache__|\.probe-identity$|^setup\.log$|^trace-.*\.json$' \
    | (cd "$repo" && tar --null -T - -cf - 2>/dev/null) | (cd "$src" && tar -xf -)
(cd "$src" && ditto -c -k --norsrc --noextattr . "$resources/source.zip")
rm -rf "$src"

# ── Signing ──────────────────────────────────────────────────────────────────────────────────────
# Ad hoc: every binary in the bundle gets a signature with no identity, which lets Apple Silicon
# run it. The nested programs are signed each on its own (`--deep`), and the launcher's update
# downloads these very files, so what it writes is signed already.
echo "== Signing (ad hoc) =="
codesign --force --deep -s - "$app"
# UNUSED until the owner has an Apple Developer account: a Developer ID signature with the
# hardened runtime, then notarization, so Gatekeeper opens it without the first-run dance.
# Set DEVELOPER_ID="Developer ID Application: Name (TEAMID)" in .setup/publish.cfg to turn it on,
# and `xcrun notarytool store-credentials kartcraft` once for the notarization step.
if [ -n "$DEVELOPER_ID" ]; then
    codesign --force --deep --options runtime --timestamp -s "$DEVELOPER_ID" "$app"
    # xcrun notarytool submit "$out/World of Kartcraft.zip" --keychain-profile kartcraft --wait
    # xcrun stapler staple "$app"
fi
codesign --verify --deep --strict "$app"

# ── The zip ──────────────────────────────────────────────────────────────────────────────────────
echo "== World of Kartcraft.zip =="
rm -f "$out/World of Kartcraft.zip"
(cd "$out" && ditto -c -k --keepParent "World of Kartcraft.app" "World of Kartcraft.zip")

# ── updates/ ─────────────────────────────────────────────────────────────────────────────────────
# Every file of the bundle but the seal (`_CodeSignature`), which the launcher never replaces:
# paths relative to the bundle, the way the Mac launcher joins them to its own bundle.
echo "== updates/ =="
objects="$updates/files"
mkdir -p "$objects"
manifest="$updates/manifest.json.tmp"
converter=""
{
    echo "{"
    echo "  \"version\": \"$version\","
    first=1
    entries=""
    while IFS= read -r -d '' file; do
        rel="${file#"$app"/}"
        case "$rel" in Contents/_CodeSignature/*) continue ;; esac
        sha="$(shasum -a 256 "$file" | cut -d' ' -f1)"
        size="$(stat -f %z "$file")"
        gz="$objects/$sha.gz"
        [ -f "$gz" ] || gzip -9 -n -c "$file" >"$gz"
        gz_size="$(stat -f %z "$gz")"
        [ "$rel" = "Contents/MacOS/ctr-extract" ] && converter="$sha"
        [ "$first" = 1 ] || entries+=","
        first=0
        entries+=$'\n'"    {\"path\": \"$rel\", \"sha256\": \"$sha\", \"size\": $size, \"gz\": \"files/$sha.gz\", \"gz_size\": $gz_size}"
        echo "$sha.gz" >>"$updates/current.txt.tmp"
    done < <(find "$app" -type f -print0 | sort -z)
    echo "  \"converter\": \"$converter\","
    echo "  \"files\": [$entries"
    echo "  ]"
    echo "}"
} >"$manifest"
# Written last and whole, so a launcher never reads a manifest naming files not there yet.
mv -f "$manifest" "$updates/manifest.json"
# Objects no manifest names any more go; a launcher mid-download still has the old manifest, so
# the previous publish's are kept until the next.
keep="$(cat "$updates/current.txt.tmp"; [ -f "$updates/previous.txt" ] && cat "$updates/previous.txt"; true)"
for obj in "$objects"/*.gz; do
    [ -e "$obj" ] || continue
    grep -qxF "$(basename "$obj")" <<<"$keep" || rm -f "$obj"
done
mv -f "$updates/current.txt.tmp" "$updates/previous.txt"

mb="$(du -m "$out/World of Kartcraft.zip" | cut -f1)"
echo
echo "Packaged $version."
echo "  New Mac players: dist/macos/out/World of Kartcraft.zip ($mb MB). The first open needs"
echo "  Gatekeeper's say-so (dist/macos/README.txt)."
echo "  Updates: publish dist/macos/out/updates as the macos/ folder beside the Windows updates"
echo "  (<UPDATE_URL>/macos/manifest.json). Nothing here uploads anything."
