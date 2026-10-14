#!/usr/bin/env bash
# Builds the Kartcraft server image: vmangos-deploy's server Dockerfile at a pinned commit, with
# kartcraft.patch applied over vmangos at a pinned revision (VMANGOS_FAIL_ON_PATCH_ERROR=1, so a
# patch that no longer applies fails the build). The tag is the patch's SHA-256, first 12 hex, so
# a server is named by what it runs; GitHub's Release workflow builds it and the VMs pull it.
#
#   setup/vmangos/build-image.sh <vmangos-deploy checkout> <image name>
#   e.g. setup/vmangos/build-image.sh ../vmangos-deploy ghcr.io/duxanlol/world-of-kartcraft-server
#
# It prints the full image reference (<name>:5875-<tag>) on its last line. The image holds only
# the build: vmangos-deploy leaves the server's config folder out, and each VM mounts its own.
set -euo pipefail
deploy="$1" name="$2"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
patch="$here/kartcraft.patch"

# The pins: vmangos at the revision server/compose.yaml's database image is built from (0e3ff01e),
# and the vmangos-deploy commit the server's Dockerfile and scripts come from.
VMANGOS_REVISION=0e3ff01e76d4758e8a7c3108b2717cc785ed56fa
DEPLOY_REVISION=f0692574b7955e78d68fea39372e0045360ce5ee
[ "$(git -C "$deploy" rev-parse HEAD)" = "$DEPLOY_REVISION" ] ||
    { echo "$deploy is not vmangos-deploy at $DEPLOY_REVISION" >&2; exit 1; }

tag="$(sha256sum "$patch" | cut -c1-12)"
image="$name:5875-$tag"
# The patch with Unix line endings, which `git apply` in the build needs.
tr -d '\r' <"$patch" >"$deploy/docker/patches/zz-kartcraft.patch"
# The Dockerfile's `COPY --exclude` is BuildKit's labs syntax, which its first line has to name.
dockerfile="$deploy/Dockerfile.kartcraft"
{ echo '# syntax=docker/dockerfile:1.7-labs'; cat "$deploy/docker/server/Dockerfile"; } >"$dockerfile"
docker build --progress=plain -t "$image" -f "$dockerfile" \
    --label org.opencontainers.image.source=https://github.com/duxanlol/world-of-kartcraft \
    --label "org.opencontainers.image.revision=$(git -C "$here" rev-parse HEAD)" \
    --build-arg VMANGOS_REVISION=$VMANGOS_REVISION --build-arg VMANGOS_CLIENT_VERSION=5875 \
    --build-arg VMANGOS_FAIL_ON_PATCH_ERROR=1 "$deploy" >&2
rm -f "$deploy/docker/patches/zz-kartcraft.patch" "$dockerfile"
echo "$image"
