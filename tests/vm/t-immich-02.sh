#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# T-IMMICH-02: the gallery comes back on a new stick (ADR 0025).
# Boot 1 (stick 1): create the archive, upload photos as a phone through the
# HTTPS proxy on 443, wait until alpd asked Immich for a backup of its
# database and the file is on the backup disk.
# Boot 2 (stick 2: a new overlay, so an empty index; the same disk): set the
# password, reopen the disk with it. The photo service must come up with the
# old gallery: the same photo ids answer with the same bytes, a small picture
# is made again, and the console's own key works (the family account is
# listed in the state).
# Runs only with ALP_ALLOW_IMMICH=1, like T-IMMICH-01.
#
# Usage: tests/vm/t-immich-02.sh <disk-image>
set -euo pipefail
here=$(dirname "$(realpath "$0")")
# shellcheck disable=SC1091,SC2153
. "$here/../lib/qa.sh"
image=${1:?usage: t-immich-02.sh <disk-image>}
[[ -f $image ]] || { echo "FAIL: image not found: $image" >&2; exit 2; }

if [[ ${ALP_ALLOW_IMMICH:-0} != 1 ]]; then
    echo "SKIP: T-IMMICH-02 runs only with ALP_ALLOW_IMMICH=1 (CI sets it; it can crash a small VM)"
    exit 0
fi
qa_image_has "$image" 'podman' || qa_skip "T-IMMICH-02: no podman/Immich in the image manifest yet"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
export T_WORK=$work
disk="$work/external.raw"
part=$(qa_make_disk "$disk")
pw=alpha-test-password-2
pwjson() { printf '{"password":"%s"}' "$1"; }
origin=http://127.0.0.1:8411

# Photos: real 32x32 PNG pictures with random pixels, so every file is unique
# and the photo service can make a small picture of each.
mkdir -p "$work/photos"
python3 -I - "$work/photos" <<'PNG'
import os, struct, sys, zlib
def chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
for i in (1, 2, 3):
    rows = b"".join(b"\0" + os.urandom(32 * 3) for _ in range(32))
    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 32, 32, 8, 2, 0, 0, 0)) \
        + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")
    open(os.path.join(sys.argv[1], f"IMG_000{i}.png"), "wb").write(png)
PNG
( cd "$work/photos" && sha256sum ./*.png >../hashes.txt )

# Runs on the host once the guest is up (ALP_HOST_HOOK).
# shellcheck disable=SC2153
hook() {
    local base="https://127.0.0.1:$PORT" c="curl -sk -m 60" tries=0 code
    mark() { # word, seconds: wait for "RESTAGENT <word>" on the console
        local i
        for ((i = 0; i < $2; i++)); do
            grep -a "RESTAGENT $1" "$ALP_SERIAL_LOG" 2>/dev/null | tail -n 1 | tr -d '\r' | grep . && return 0
            sleep 1
        done
        return 1
    }
    small() { # base, token, id: wait for the asset's small picture; sets code
        local _t
        for _t in $(seq 48); do
            code=$(curl -sk -m 60 -H "Authorization: Bearer $2" -o /dev/null -w '%{http_code}' "$1/api/assets/$3/thumbnail")
            [[ $code == 200 ]] && return 0
            sleep 5
        done
        return 1
    }
    until code=$($c -o /dev/null -w '%{http_code}' "$base/api/server/ping"); [[ $code == 200 ]]; do
        (( ++tries < 150 )) || { echo "FAIL: Immich does not answer on 443 (last answer: $code)" >&2; return 1; }
        sleep 2
    done
    echo "ok:   Immich answers through the proxy on 443 (round $ROUND)"
    if [[ $ROUND == 2 ]]; then
        # The restore: Immich restarts twice; alpd then makes its key.
        local back
        back=$(mark BACK 900) || { echo "FAIL: the restore did not end in 15 minutes" >&2; grep -a 'RESTAGENT' "$ALP_SERIAL_LOG" | tr -d '\r' | tail -n 30 >&2; return 1; }
        grep -a 'RESTAGENT LOG' "$ALP_SERIAL_LOG" | tr -d '\r' | tail -n 30
        [[ $back == *"key=1"* ]] || { echo "FAIL: alpd has no key for the restored gallery (${back#*RESTAGENT })" >&2; return 1; }
        echo "ok:   the restore ended and alpd has its key again"
    fi
    local token _try
    for _try in $(seq 30); do
        token=$($c -H 'Content-Type: application/json' -d "$(jq -nc --arg p "$PW" '{email:"family@alparchive.local",password:$p}')" \
            "$base/api/auth/login" | jq -r .accessToken 2>/dev/null || true)
        [[ -n $token && $token != null ]] && break
        sleep 10
    done
    [[ -n $token && $token != null ]] || { echo "FAIL: phone login refused (round $ROUND)" >&2; return 1; }
    echo "ok:   phone login works (round $ROUND)"
    local f id n=0
    if [[ $ROUND == 1 ]]; then
        for f in "$T_WORK"/photos/*.png; do
            id=$($c -H "Authorization: Bearer $token" -F deviceAssetId="$(basename "$f")" -F deviceId=alp-qa \
                -F fileCreatedAt=2026-01-01T00:00:00.000Z -F fileModifiedAt=2026-01-01T00:00:00.000Z \
                -F assetData=@"$f" "$base/api/assets" | jq -r .id)
            [[ -n $id && $id != null ]] || { echo "FAIL: upload of $f refused" >&2; return 1; }
            echo "$id $(basename "$f")" >>"$T_WORK/ids.txt"; n=$((n + 1))
        done
        echo "ok:   $n photos uploaded"
        id=$(head -n1 "$T_WORK/ids.txt" | cut -d' ' -f1)
        small "$base" "$token" "$id" || { echo "FAIL: the first stick made no small picture (answer $code)" >&2; return 1; }
        echo "ok:   the first stick made the small pictures"
        local dump
        dump=$(mark DUMP 420) || { echo "FAIL: no backup of the gallery's database on the disk after 7 minutes" >&2; return 1; }
        echo "ok:   alpd asked for a backup after the upload (${dump#*DUMP })"
        return 0
    fi
    local want got bad=0
    while read -r id f; do
        want=$(grep " \./$f\$" "$T_WORK/hashes.txt" | cut -d' ' -f1)
        got=$($c -H "Authorization: Bearer $token" "$base/api/assets/$id/original" | sha256sum | cut -d' ' -f1)
        [[ $want == "$got" ]] || { echo "FAIL: $f is not in the gallery of the new stick (id $id)" >&2; bad=1; }
    done <"$T_WORK/ids.txt"
    (( bad == 0 )) || return 1
    echo "ok:   the gallery is back: the same photos under the same ids"
    n=$($c -H "Authorization: Bearer $token" "$base/api/assets/statistics" | jq -r '.images // .total')
    [[ $n == 3 ]] || { echo "FAIL: the gallery holds $n photos, not 3 (uploaded again, or lost)" >&2; return 1; }
    echo "ok:   3 photos, none twice"
    id=$(head -n1 "$T_WORK/ids.txt" | cut -d' ' -f1)
    small "$base" "$token" "$id" || { echo "FAIL: no small picture after 4 minutes (answer $code)" >&2; return 1; }
    echo "ok:   the small pictures are made again"
}
export -f hook

# In the guest: say on the console when the backup file is there (boot 1),
# and when the restore ended (boot 2), with alpd's own lines about it.
read -r -d '' agent <<'AGENT' || true
say() { echo "RESTAGENT $*" >/dev/console; }
dir=/var/lib/alp-pool/files/immich/backups
if [ "$1" = 1 ]; then
  for i in $(seq 240); do
    f=$(ls "$dir" 2>/dev/null | grep '^immich-db-backup-.*\.sql\.gz$' | tail -n 1)
    [ -n "$f" ] && [ -s "$dir/$f" ] && break
    sleep 2
  done
  sync
  say "DUMP ${f:-none}"
  [ -n "$f" ] || journalctl -b -u alpd --no-pager | grep -i gallery | tail -n 20 | sed 's/^/RESTAGENT LOG /' >/dev/console
else
  mark=/var/lib/alp-index/alp/immich/restore-pending
  seen=0
  for i in $(seq 440); do
    [ -e "$mark" ] && seen=1
    [ "$seen" = 1 ] && [ ! -e "$mark" ] && break
    sleep 2
  done
  sleep 20
  journalctl -b --no-pager | grep -i 'gallery' | tail -n 25 | sed 's/^/RESTAGENT LOG /' >/dev/console
  key=0; [ -s /var/lib/alp-index/alp/immich/admin-api-key ] && key=1
  say "BACK seen=$seen key=$key"
fi
AGENT
agent_b64=$(base64 -w0 <<<"$agent")

status=0
for round in 1 2; do
    port=$(qa_free_port)
    args="-drive if=none,id=ext,format=raw,file=$disk -device qemu-xhci -device usb-storage,drive=ext -nic user,model=virtio-net-pci,hostfwd=tcp:127.0.0.1:$port-:443"
    probe="$(QA_HTTP_TIMEOUT=90 qa_http_probe SETUP POST /api/v1/live/password/setup "$origin" "$(pwjson "$pw")")
"
    if [[ $round == 1 ]]; then
        probe+="$(qa_http_probe CREATE POST /api/v1/live/archive/create "$origin" "{\"partuuid\":\"$part\",\"size_bytes\":1073741824,\"alp_password\":\"$pw\"}")
"
    else
        probe+="$(qa_http_probe REOPEN POST /api/v1/live/archive/recover "$origin" "{\"partuuid\":\"$part\",\"alp_password\":\"$pw\"}")
"
    fi
    probe+="-/usr/bin/systemd-run --unit=alp-restore-agent /bin/bash -c 'echo $agent_b64 | base64 -d | bash -s $round'
/bin/bash -c 'sleep 60'
-/bin/bash -c 'exec 3<>/dev/tcp/127.0.0.1/8411 && echo API-LISTENING || echo API-NOT-LISTENING'"
    export ALP_OVERLAY="$work/stick$round.qcow2"
    out=$(ROUND=$round PORT=$port PW=$pw ALP_RAM=${ALP_IMMICH_RAM:-4G} ALP_PROBE="$probe" ALP_PROBE_DELAY=60 ALP_QEMU_ARGS="$args" \
        ALP_HOST_HOOK=hook "$here/run-vm.sh" "$image" 1500) || status=$?
    echo "$out"
    if grep -q 'API-NOT-LISTENING' <<<"$out"; then qa_skip "T-IMMICH-02: live API is not in this image yet"; fi
    (( status == 0 )) || { echo "FAIL: T-IMMICH-02 round $round" >&2; exit 1; }
done
echo "PASS: T-IMMICH-02"
