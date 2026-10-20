#!/usr/bin/env bash
# Puts the Android app built by scripts/android-apk.sh on the update host, where the app updates
# itself from (crates/benilla-app/src/android.rs, update): the APK first, then the version.json
# naming it, so an app never reads a version.json whose APK is not there yet. Build with the same
# ANDROID_CHANNEL it is uploaded to: the channel is built into the app.
#
#   [ANDROID_CHANNEL=staging] [ACCOUNT=<storage account> KEY=<its key>] dist/android-upload.sh
#
# The players' folder is android/, the testers' staging/android/. The APK stays downloadable at
# <website>/android/WorldOfKartcraft.apk, the link to give a new player.
set -euo pipefail
cd "$(dirname "$0")/.."
channel="${ANDROID_CHANNEL:-}"
# ANDROID_PREFIX overrides the folder: the release workflow puts production's in candidate/android/,
# which dist/promote.sh copies to android/.
prefix="${ANDROID_PREFIX:-${channel:+$channel/}android/}"
# The account and its key as publish.ps1 finds them when not given: `.setup/publish.cfg`'s
# AZURE_STORAGE, and the key from the signed-in Azure CLI.
ACCOUNT="${ACCOUNT:-$(sed -n 's/^AZURE_STORAGE=//p' .setup/publish.cfg 2>/dev/null | tr -d '\r')}"
[ -n "$ACCOUNT" ] || { echo "no ACCOUNT, and no AZURE_STORAGE in .setup/publish.cfg"; exit 1; }
KEY="${KEY:-$(az storage account keys list -n "$ACCOUNT" --query '[0].value' -o tsv | tr -d '\r')}"
auth=(--account-name "$ACCOUNT" --account-key "$KEY" --only-show-errors)
# Relative paths: the Azure CLI on Windows takes no /c/... path.
# ANDROID_DIR names the build's folder when not the script's own (the workflow keeps two).
dir="${ANDROID_DIR:-target/android}"
apk="$dir/kartcraft.apk"
version="$dir/version.json"
[ -f "$apk" ] && [ -f "$version" ] || { echo "no APK: run scripts/android-apk.sh first"; exit 1; }
[ "$(sha256sum "$apk" | cut -d' ' -f1)" = "$(sed -n 's/.*"sha256": "\(.*\)".*/\1/p' "$version")" ] ||
    { echo "$version does not describe $apk: build again"; exit 1; }
built="$(sed -n 's/.*"channel": "\(.*\)".*/\1/p' "$version")"
[ "$built" = "${channel:-production}" ] ||
    { echo "the APK was built for $built, not ${channel:-production}: build it with this ANDROID_CHANNEL"; exit 1; }

az storage blob upload -c '$web' -n "${prefix}WorldOfKartcraft.apk" -f "$apk" \
    --content-type application/vnd.android.package-archive \
    --content-disposition "attachment; filename=WorldOfKartcraft.apk" \
    --content-cache-control no-cache --overwrite --no-progress "${auth[@]}" -o none
az storage blob upload -c '$web' -n "${prefix}version.json" -f "$version" \
    --content-type application/json --content-cache-control no-cache --overwrite --no-progress \
    "${auth[@]}" -o none
echo "uploaded build $(sed -n 's/.*"versionCode": \([0-9]*\).*/\1/p' "$version") to $prefix"
