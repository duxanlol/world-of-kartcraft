#!/usr/bin/env bash
# Puts one channel's updates (publish.ps1's or package.sh's updates/ folder) into the storage
# account's static website under a prefix, then the download files named after it. The order is
# publish.ps1's: new objects first, the manifest after them, so a launcher never reads a manifest
# naming files not there yet; stale objects last, keeping those the replaced manifest names, since
# a launcher mid-download still reads that one.
#
#   ACCOUNT=<storage account> KEY=<its key> WEB=<its website URL> \
#       dist/upload.sh <updates dir> <prefix> [<file> <blob name>]...
#
# <prefix> is the channel's folder: "" (the players'), "staging/", "macos/", "candidate/", ...
# Paths are given relative to where it runs: the Azure CLI on Windows takes no /c/... path.
set -euo pipefail
updates="$1" prefix="$2"
shift 2
auth=(--account-name "$ACCOUNT" --account-key "$KEY" --only-show-errors)

names() { grep -o '"gz": *"files/[0-9a-f]*\.gz"' | sed 's#.*"files/##; s#"$##'; }
new="$(names <"$updates/manifest.json")"
old="$(curl -fsS "$WEB/${prefix}manifest.json" 2>/dev/null | names || true)"
remote="$(az storage blob list -c '$web' --prefix "${prefix}files/" --query '[].name' -o tsv "${auth[@]}" | tr -d '\r')"

echo "== ${prefix:-the root}: $(wc -l <<<"$new") files, version $(grep -o '"version": *"[^"]*"' "$updates/manifest.json" | cut -d'"' -f4) =="
for gz in $new; do
    grep -qxF "${prefix}files/$gz" <<<"$remote" && continue
    echo "  ${prefix}files/$gz"
    az storage blob upload -c '$web' -n "${prefix}files/$gz" -f "$updates/files/$gz" --content-type application/gzip --overwrite --no-progress "${auth[@]}" -o none
done
az storage blob upload -c '$web' -n "${prefix}manifest.json" -f "$updates/manifest.json" --content-type application/json --content-cache-control no-cache --overwrite --no-progress "${auth[@]}" -o none
while IFS= read -r name; do
    [ -n "$name" ] || continue
    gz="${name#"${prefix}"files/}"
    grep -qxF "$gz" <<<"$new" || grep -qxF "$gz" <<<"$old" || az storage blob delete -c '$web' -n "$name" "${auth[@]}" -o none
done <<<"$remote"

# The files a new player downloads: the Windows launcher, the Mac zip, the Linux AppImage.
while [ $# -ge 2 ]; do
    file="$1" blob="$2"
    shift 2
    echo "  $blob"
    case "$blob" in
    *.exe) az storage blob upload -c '$web' -n "$blob" -f "$file" --content-type application/vnd.microsoft.portable-executable --content-disposition "attachment; filename*=UTF-8''Kartcraft%20Launcher.exe" --content-cache-control no-cache --overwrite --no-progress "${auth[@]}" -o none ;;
    *.AppImage) az storage blob upload -c '$web' -n "$blob" -f "$file" --content-type application/octet-stream --content-cache-control no-cache --overwrite --no-progress "${auth[@]}" -o none ;;
    *.zip) az storage blob upload -c '$web' -n "$blob" -f "$file" --content-type application/zip --content-cache-control no-cache --overwrite --no-progress "${auth[@]}" -o none ;;
    *) echo "upload.sh: no content type for $blob" >&2 && exit 1 ;;
    esac
done
