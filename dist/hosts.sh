# The download hosts the upload scripts write to (sourced by upload.sh, android-upload.sh and
# promote.sh). Each one configured is written, the same names in the same order:
#
#   DL_HOST=dl@host  the game's own host (dl.worldofkartcraft.com): its `dl` user's SSH key runs only
#                    kartcraft-dl (put, cat, list, rm, cp under the folder Caddy serves), and Caddy
#                    sets the content types and caching from the names. DL_KEY is the key file
#                    (default ~/.ssh/kartcraft_dl); DL_KNOWN_HOSTS, when set, pins the host's key.
#   ACCOUNT, KEY     an Azure storage account's static website ($web), with each object's type and
#                    caching set on upload. Kept while launchers built before the move still update
#                    from it.
#
# Names are relative to the host's root: "manifest.json", "staging/files/<sha256>.gz", ...
hosts=()
[ -z "${DL_HOST:-}" ] || hosts+=(dl)
[ -z "${ACCOUNT:-}" ] || hosts+=(azure)
[ ${#hosts[@]} -gt 0 ] || { echo "no download host: set DL_HOST, or ACCOUNT and KEY" >&2; exit 1; }

dl_ssh() {
    local opts=(-i "${DL_KEY:-$HOME/.ssh/kartcraft_dl}" -o BatchMode=yes -o ConnectTimeout=30)
    if [ -n "${DL_KNOWN_HOSTS:-}" ]; then
        local known="${TMPDIR:-/tmp}/kartcraft-dl-known.$$"
        printf '%s\n' "$DL_KNOWN_HOSTS" >"$known"
        opts+=(-o StrictHostKeyChecking=yes -o "UserKnownHostsFile=$known")
    else
        opts+=(-o StrictHostKeyChecking=accept-new)
    fi
    ssh "${opts[@]}" "$DL_HOST" "$@"
}

azure_type() { # the object's type, download name and caching, from its name
    case "$1" in
    */files/*.gz | files/*.gz) echo --content-type application/gzip ;;
    *.json) echo --content-type application/json --content-cache-control no-cache ;;
    *.exe) echo --content-type application/vnd.microsoft.portable-executable --content-cache-control no-cache ;;
    *.apk) echo --content-type application/vnd.android.package-archive --content-cache-control no-cache ;;
    *.AppImage) echo --content-type application/octet-stream --content-cache-control no-cache ;;
    *.zip) echo --content-type application/zip --content-cache-control no-cache ;;
    *) echo "hosts.sh: no content type for $1" >&2; return 1 ;;
    esac
}
azure_auth() { echo --account-name "$ACCOUNT" --account-key "$KEY" --only-show-errors; }

# host_put <host> <local file> <name>
host_put() {
    case "$1" in
    dl) dl_ssh "put $3" <"$2" ;;
    azure)
        local extra=()
        case "$3" in
        *.exe) extra=(--content-disposition "attachment; filename*=UTF-8''Kartcraft%20Launcher.exe") ;;
        *.apk) extra=(--content-disposition "attachment; filename=WorldOfKartcraft.apk") ;;
        esac
        # shellcheck disable=SC2046
        az storage blob upload -c '$web' -n "$3" -f "$2" $(azure_type "$3") "${extra[@]}" --overwrite --no-progress $(azure_auth) -o none ;;
    esac
}
# host_list <host> <prefix>: the names starting with <prefix>, one per line
host_list() {
    case "$1" in
    dl) dl_ssh "list $2" ;;
    # shellcheck disable=SC2046
    azure) az storage blob list -c '$web' --prefix "$2" --num-results '*' --query '[].name' -o tsv $(azure_auth) | tr -d '\r' ;;
    esac
}
# host_cat <host> <name>: its contents, or a failure when it is not there
host_cat() {
    case "$1" in
    dl) dl_ssh "cat $2" 2>/dev/null ;;
    # shellcheck disable=SC2046
    azure) az storage blob download -c '$web' -n "$2" -f /dev/stdout --no-progress $(azure_auth) -o none 2>/dev/null ;;
    esac
}
# host_rm <host> <name>
host_rm() {
    case "$1" in
    dl) dl_ssh "rm $2" ;;
    # shellcheck disable=SC2046
    azure) az storage blob delete -c '$web' -n "$2" $(azure_auth) -o none ;;
    esac
}
