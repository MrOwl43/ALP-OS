#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Download the container images pinned in images.lock into <out-dir>, one
# directory per image in containers/image's "dir:" format (manifest.json,
# blobs named by their sha256, version). Every download is checked against
# its digest; the index must list the pinned linux/amd64 manifest. Blobs
# already present and correct are not downloaded again.
#
# Usage: image/immich/fetch.sh <out-dir>
set -euo pipefail
here=$(dirname "$(realpath "$0")")
out=${1:?usage: fetch.sh <out-dir>}
mkdir -p "$out"

accept='application/vnd.oci.image.index.v1+json,application/vnd.docker.distribution.manifest.list.v2+json,application/vnd.oci.image.manifest.v1+json,application/vnd.docker.distribution.manifest.v2+json'

token() { # <registry> <repo>
    case $1 in
        ghcr.io) curl -fsSL "https://ghcr.io/token?scope=repository:$2:pull" | jq -r .token ;;
        docker.io) curl -fsSL "https://auth.docker.io/token?service=registry.docker.io&scope=repository:$2:pull" | jq -r .token ;;
        *) echo "unknown registry $1" >&2; return 1 ;;
    esac
}

api() { case $1 in docker.io) echo registry-1.docker.io ;; *) echo "$1" ;; esac; }

check() { # <file> <sha256:hex>
    [[ $(sha256sum "$1" | cut -d' ' -f1) == "${2#sha256:}" ]]
}

fetch() { # <url> <file> <digest> <token> [accept]
    if [[ -f $2 ]] && check "$2" "$3"; then return 0; fi
    curl -fsSL --retry 6 --retry-delay 20 --retry-all-errors -H "Authorization: Bearer $4" ${5:+-H "Accept: $5"} -o "$2.part" "$1"
    check "$2.part" "$3" || { echo "FAIL: digest mismatch for $1" >&2; rm -f "$2.part"; return 1; }
    mv "$2.part" "$2"
}

grep -v '^\s*#' "$here/images.lock" | while read -r name ref tag index amd64; do
    [[ -n $name ]] || continue
    registry=${ref%%/*}; repo=${ref#*/}
    host=$(api "$registry"); t=$(token "$registry" "$repo")
    dir="$out/$name"; mkdir -p "$dir"
    fetch "https://$host/v2/$repo/manifests/$index" "$dir/index.json" "$index" "$t" "$accept"
    jq -e --arg d "$amd64" '.manifests[] | select(.digest == $d and .platform.os == "linux" and .platform.architecture == "amd64")' \
        "$dir/index.json" >/dev/null || { echo "FAIL: $name: index does not list $amd64 for linux/amd64" >&2; exit 1; }
    fetch "https://$host/v2/$repo/manifests/$amd64" "$dir/manifest.json" "$amd64" "$t" "$accept"
    for d in $(jq -r '.config.digest, .layers[].digest' "$dir/manifest.json"); do
        fetch "https://$host/v2/$repo/blobs/$d" "$dir/${d#sha256:}" "$d" "$t"
    done
    printf 'Directory Transport Version: 1.1\n' > "$dir/version"
    rm -f "$dir/index.json"
    echo "ok: $name $tag ($amd64)"
done
