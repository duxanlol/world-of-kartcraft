#!/usr/bin/env bash
# Builds the vmangos server image with World of Kartcraft's patch (setup/vmangos/kartcraft.patch:
# the Karting profession) and points a vmangos-deploy checkout's compose.yaml at it.
# setup/setup.sh runs this for server/; for another vmangos-deploy checkout:
#   setup/build-server.sh ~/path/to/vmangos-deploy && docker compose -f ~/path/to/vmangos-deploy/compose.yaml up -d
# The first build compiles vmangos (a few minutes). KARTCRAFT_BUILD_JOBS sets its parallelism
# (default 4, so the PC stays usable).
set -euo pipefail
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
DEPLOY=$(cd "${1:-$ROOT/server}" && pwd)
PATCH="$ROOT/setup/vmangos/kartcraft.patch"
# The vmangos commit the patch is made against.
VMANGOS_REV=0e3ff01e76d4758e8a7c3108b2717cc785ed56fa
JOBS=${KARTCRAFT_BUILD_JOBS:-4}
# Tagged by the patch's contents, so a changed patch builds a new image.
IMAGE="world-of-kartcraft/vmangos-server:5875-$(sha256sum "$PATCH" | cut -c1-12)"

[ -f "$DEPLOY/compose.yaml" ] || { echo "no compose.yaml in $DEPLOY"; exit 1; }
if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
    echo "Building $IMAGE (vmangos $VMANGOS_REV with the Kartcraft patch, $JOBS jobs)"
    CONTEXT=$(mktemp -d)
    trap 'rm -rf "$CONTEXT"' EXIT
    cp -r "$DEPLOY/docker" "$CONTEXT/"
    cp "$PATCH" "$CONTEXT/docker/patches/zz-kartcraft.patch"
    # The frontend named first: older Dockers build without `COPY --exclude` otherwise.
    { echo "# syntax=docker/dockerfile:1.7-labs"
      sed "s/make -j\"\$(nproc)\"/make -j$JOBS/" "$CONTEXT/docker/server/Dockerfile"; } > "$CONTEXT/Dockerfile.kartcraft"
    docker build -t "$IMAGE" -f "$CONTEXT/Dockerfile.kartcraft" \
        --build-arg VMANGOS_REVISION="$VMANGOS_REV" \
        --build-arg VMANGOS_CLIENT_VERSION=5875 \
        --build-arg VMANGOS_FAIL_ON_PATCH_ERROR=1 \
        "$CONTEXT"
fi
sed -i -E "s#image: (ghcr\.io/mserajnik/vmangos-server:5875|world-of-(skate|kart)craft/vmangos-server:[^[:space:]]+)#image: $IMAGE#" \
    "$DEPLOY/compose.yaml"
# The database image floats on `latest`, which moved on to vmangos-deploy's version 1
# (2026-10-04): pin it to the build for our vmangos revision and give it the version it checks.
sed -i -E "s#image: ghcr\.io/mserajnik/vmangos-database(:[^[:space:]]+)?#image: ghcr.io/mserajnik/vmangos-database:$VMANGOS_REV#" \
    "$DEPLOY/compose.yaml"
grep -q VMANGOS_DEPLOY_VERSION "$DEPLOY/compose.yaml" ||
    sed -i '/^  database:/,/^    environment:/ s/^    environment:$/    environment:\n      - VMANGOS_DEPLOY_VERSION=1/' \
        "$DEPLOY/compose.yaml"
echo "$DEPLOY/compose.yaml now runs $IMAGE"
