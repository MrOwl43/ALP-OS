#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Put the pinned ttyd (image/terminal/ttyd.lock) into the image tree.
# Usage: image/terminal/fetch.sh <cache-dir> <image-tree>
# Each file is downloaded once into <cache-dir> and checked against its
# SHA-256 every time; a wrong or partial file fails the build.
set -euo pipefail
cache=${1:?usage: fetch.sh <cache-dir> <image-tree>}
tree=${2:?usage: fetch.sh <cache-dir> <image-tree>}
lock="$(dirname "$(realpath "$0")")/ttyd.lock"
mkdir -p "$cache"
while read -r sum dest url; do
    [[ -z $sum || $sum == \#* ]] && continue
    f="$cache/$sum"
    if ! echo "$sum  $f" | sha256sum -c --status - 2>/dev/null; then
        rm -f "$f.part"
        curl -fsSL --retry 4 --retry-delay 2 -o "$f.part" "$url"
        echo "$sum  $f.part" | sha256sum -c --quiet - || { echo "fetch.sh: checksum mismatch for $url" >&2; rm -f "$f.part"; exit 1; }
        mv "$f.part" "$f"
    fi
    mode=0644
    [[ $dest == usr/bin/* ]] && mode=0755
    install -D -m "$mode" "$f" "$tree/$dest"
done <"$lock"
