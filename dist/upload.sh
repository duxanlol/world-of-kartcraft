#!/usr/bin/env bash
# Puts one channel's updates (publish.ps1's or package.sh's updates/ folder) on the download hosts
# (hosts.sh) under a prefix, then the download files named after it. The order is publish.ps1's:
# new objects first, the manifest after them, so a launcher never reads a manifest naming files
# not there yet; stale objects last, keeping those the replaced manifest names, since a launcher
# mid-download still reads that one.
#
#   DL_HOST=dl@<host> [ACCOUNT=<storage account> KEY=<its key>] \
#       dist/upload.sh <updates dir> <prefix> [<file> <name>]...
#
# <prefix> is the channel's folder: "" (the players'), "staging/", "macos/", "candidate/", ...
# Paths are given relative to where it runs: the Azure CLI on Windows takes no /c/... path.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=hosts.sh
. "$here/hosts.sh"
updates="$1" prefix="$2"
shift 2

names() { grep -o '"gz": *"files/[0-9a-f]*\.gz"' | sed 's#.*"files/##; s#"$##'; }
new="$(names <"$updates/manifest.json")"
echo "== ${prefix:-the root}: $(wc -l <<<"$new") files, version $(grep -o '"version": *"[^"]*"' "$updates/manifest.json" | cut -d'"' -f4) =="

for host in "${hosts[@]}"; do
    old="$(host_cat "$host" "${prefix}manifest.json" | names || true)"
    remote="$(host_list "$host" "${prefix}files/")"
    for gz in $new; do
        grep -qxF "${prefix}files/$gz" <<<"$remote" && continue
        echo "  $host: ${prefix}files/$gz"
        host_put "$host" "$updates/files/$gz" "${prefix}files/$gz"
    done
    host_put "$host" "$updates/manifest.json" "${prefix}manifest.json"
    while IFS= read -r name; do
        [ -n "$name" ] || continue
        gz="${name#"${prefix}"files/}"
        grep -qxF "$gz" <<<"$new" || grep -qxF "$gz" <<<"$old" || host_rm "$host" "$name"
    done <<<"$remote"

    # The files a new player downloads: the Windows launcher, the Mac zip, the Linux AppImage.
    args=("$@")
    while [ ${#args[@]} -ge 2 ]; do
        echo "  $host: ${args[1]}"
        host_put "$host" "${args[0]}" "${args[1]}"
        args=("${args[@]:2}")
    done
done
