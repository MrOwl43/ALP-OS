#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# T-FORMAT-01: the format test set (docs/qa/formats.md). One boot: create the
# archive, upload one small file of every picture format and video codec in
# tests/vm/formats/make-samples.sh as a phone would (HTTPS on 443), then ask
# for each: was it kept byte for byte, does the gallery have a small picture
# of it, and (videos) does it play. Prints one FORMAT line per file.
# Every file must be kept exactly. A format listed in $must must also get its
# small picture and play; the others are reported only (the table in
# docs/qa/formats.md says what the gallery shows). Camera RAW files
# (raw-*, real files from tests/vm/formats/raw-samples.lock) are reported
# only: whether a RAW file gets a preview depends on Immich's LibRaw.
# Runs only with ALP_ALLOW_IMMICH=1, like T-IMMICH-01. Needs ffmpeg.
#
# Usage: tests/vm/t-format-01.sh <disk-image>
set -euo pipefail
here=$(dirname "$(realpath "$0")")
# shellcheck disable=SC1091,SC2153
. "$here/../lib/qa.sh"
image=${1:?usage: t-format-01.sh <disk-image>}
[[ -f $image ]] || { echo "FAIL: image not found: $image" >&2; exit 2; }

if [[ ${ALP_ALLOW_IMMICH:-0} != 1 ]]; then
    echo "SKIP: T-FORMAT-01 runs only with ALP_ALLOW_IMMICH=1 (CI sets it; it can crash a small VM)"
    exit 0
fi
qa_image_has "$image" 'podman' || qa_skip "T-FORMAT-01: no podman/Immich in the image manifest yet"
command -v ffmpeg >/dev/null || qa_skip "T-FORMAT-01: no ffmpeg on this machine"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
export T_WORK=$work
disk="$work/external.raw"
part=$(qa_make_disk "$disk")
pw=alpha-test-password-3
origin=http://127.0.0.1:8411
"$here/formats/make-samples.sh" "$work/samples" >/dev/null
( cd "$work/samples" && sha256sum -- * >../hashes.txt )
echo "ok:   $(wc -l <"$work/hashes.txt") sample files made"

# What the gallery must show today. The rest is reported.
export MUST="photo-jpeg.jpg photo-png.png photo-webp.webp photo-tiff.tiff photo-gif.gif photo-heic.heic photo-avif.avif
video-mp4-h264.mp4 video-mp4-h265.mp4 video-mkv-h264.mkv video-webm-vp9.webm video-mov-prores.mov video-mov-mjpeg.mov"

# shellcheck disable=SC2153
hook() {
    local base="https://127.0.0.1:$PORT" c="curl -sk -m 120" tries=0 code
    until code=$($c -o /dev/null -w '%{http_code}' "$base/api/server/ping"); [[ $code == 200 ]]; do
        (( ++tries < 150 )) || { echo "FAIL: Immich does not answer on 443 (last answer: $code)" >&2; return 1; }
        sleep 2
    done
    local token _try
    for _try in $(seq 30); do
        token=$($c -H 'Content-Type: application/json' -d "$(jq -nc --arg p "$PW" '{email:"family@alparchive.local",password:$p}')" \
            "$base/api/auth/login" | jq -r .accessToken 2>/dev/null || true)
        [[ -n $token && $token != null ]] && break
        sleep 10
    done
    [[ -n $token && $token != null ]] || { echo "FAIL: phone login refused" >&2; return 1; }
    local auth="Authorization: Bearer $token" f name id r
    for f in "$T_WORK"/samples/*; do
        name=$(basename "$f")
        r=$($c -H "$auth" -F deviceAssetId="$name" -F deviceId=alp-qa \
            -F fileCreatedAt=2026-01-01T00:00:00.000Z -F fileModifiedAt=2026-01-01T00:00:00.000Z \
            -F assetData=@"$f" "$base/api/assets")
        id=$(jq -r '.id // empty' <<<"$r" 2>/dev/null || true)
        echo "${id:-refused} $name" >>"$T_WORK/ids.txt"
        [[ -n $id ]] || echo "note: $name was not taken: $(head -c 200 <<<"$r")"
    done
    echo "ok:   $(grep -vc '^refused ' "$T_WORK/ids.txt") of $(wc -l <"$T_WORK/ids.txt") files taken"
    # The gallery works through them in the background: ask until nothing
    # changes any more (at most 8 minutes).
    local _round done_n last=-1 still=0
    for _round in $(seq 48); do
        done_n=0
        while read -r id name; do
            [[ $id == refused ]] && continue
            [[ $($c -H "$auth" -o /dev/null -w '%{http_code}' "$base/api/assets/$id/thumbnail") == 200 ]] && done_n=$((done_n + 1))
        done <"$T_WORK/ids.txt"
        if (( done_n == last )); then still=$((still + 1)); else still=0; fi
        last=$done_n
        (( done_n == $(grep -vc '^refused ' "$T_WORK/ids.txt") || still >= 9 )) && break
        sleep 10
    done
    local bad=0 kept small play want got must
    while read -r id name; do
        kept=no; small=no; play=-
        if [[ $id != refused ]]; then
            want=$(grep "  $name\$" "$T_WORK/hashes.txt" | cut -d' ' -f1)
            got=$($c -H "$auth" "$base/api/assets/$id/original" | sha256sum | cut -d' ' -f1)
            [[ $want == "$got" ]] && kept=yes
            [[ $($c -H "$auth" -o /dev/null -w '%{http_code}' "$base/api/assets/$id/thumbnail") == 200 ]] && small=yes
            if [[ $name == video-* ]]; then
                play=no
                [[ $($c -H "$auth" -o /dev/null -w '%{http_code}' "$base/api/assets/$id/video/playback") == 200 ]] && play=yes
            fi
        fi
        must=no; grep -qwF -- "$name" <<<"$MUST" && must=yes
        echo "FORMAT $name taken=$([[ $id == refused ]] && echo no || echo yes) kept=$kept preview=$small play=$play must=$must"
        if [[ $id != refused && $kept != yes ]]; then echo "FAIL: $name came back changed" >&2; bad=1; fi
        if [[ $must == yes && ( $small != yes || $play == no ) ]]; then echo "FAIL: $name must show in the gallery (preview=$small play=$play)" >&2; bad=1; fi
    done <"$T_WORK/ids.txt"
    return "$bad"
}
export -f hook

port=$(qa_free_port)
args="-drive if=none,id=ext,format=raw,file=$disk -device qemu-xhci -device usb-storage,drive=ext -nic user,model=virtio-net-pci,hostfwd=tcp:127.0.0.1:$port-:443"
probe="$(QA_HTTP_TIMEOUT=90 qa_http_probe SETUP POST /api/v1/live/password/setup "$origin" "{\"password\":\"$pw\"}")
$(qa_http_probe CREATE POST /api/v1/live/archive/create "$origin" "{\"partuuid\":\"$part\",\"size_bytes\":1073741824,\"alp_password\":\"$pw\"}")
/bin/bash -c 'sleep 60'
-/bin/bash -c 'exec 3<>/dev/tcp/127.0.0.1/8411 && echo API-LISTENING || echo API-NOT-LISTENING'"
status=0
out=$(PORT=$port PW=$pw ALP_RAM=${ALP_IMMICH_RAM:-4G} ALP_PROBE="$probe" ALP_PROBE_DELAY=60 ALP_QEMU_ARGS="$args" \
    ALP_HOST_HOOK=hook "$here/run-vm.sh" "$image" 900) || status=$?
grep -E '^(FORMAT|ok:|note:) ' <<<"$out" || true
if grep -q 'API-NOT-LISTENING' <<<"$out"; then qa_skip "T-FORMAT-01: live API is not in this image yet"; fi
(( status == 0 )) || { tail -n 40 <<<"$out" >&2; echo "FAIL: T-FORMAT-01" >&2; exit 1; }
echo "PASS: T-FORMAT-01"
