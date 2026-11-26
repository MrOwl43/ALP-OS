#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Guest commands are single-quoted on purpose: they expand in the VM.
# shellcheck disable=SC2016
# T-IMMICH-03: the gallery comes back after the system disk is lost, on the
# installed system (plan docs/plans/0.2.8-reopen-archive.md, the VERIFY on
# Immich's admin account; restore flow of ADR 0025). CI only.
# 1. Install from the stick on a system disk with one data disk.
# 2. First start: password, pool, a recovery code (15 words); upload photos
#    as a phone through the HTTPS proxy on 443; wait for the database backup
#    on the pool.
# 3. Install again on a NEW blank system disk with --reopen (the data disk is
#    never erased).
# 4. First start: password, the 15 words open the pool. The photo service
#    must come up with the old gallery: the same photo ids answer with the
#    same bytes, small pictures are made again, alpd has its key again.
# The hook and the in-guest agent are T-IMMICH-02's (same checks).
# Runs only with ALP_ALLOW_IMMICH=1. Test data stays far under 1 GB.
#
# Usage: tests/vm/t-immich-03.sh <disk-image>
set -euo pipefail
here=$(dirname "$(realpath "$0")")
# shellcheck disable=SC1091,SC2153
. "$here/../lib/qa.sh"
image=${1:?usage: t-immich-03.sh <disk-image>}
[[ -f $image ]] || { echo "FAIL: image not found: $image" >&2; exit 2; }
if [[ ${ALP_ALLOW_IMMICH:-0} != 1 ]]; then
    echo "SKIP: T-IMMICH-03 runs only with ALP_ALLOW_IMMICH=1 (CI sets it; it can crash a small VM)"
    exit 0
fi
qa_image_has "$image" 'podman' || qa_skip "T-IMMICH-03: no podman/Immich in the image manifest yet"

work=$(mktemp -d -p "$(dirname "$(realpath "$image")")" t-immich-03.XXXXXX)
trap 'rm -rf "$work"' EXIT
export T_WORK=$work
target="$work/target.raw" target2="$work/target2.raw" data="$work/data.raw"
truncate -s 40G "$target"
truncate -s 40G "$target2"
truncate -s 40G "$data"
pw=alpha-test-password-2
origin=http://127.0.0.1:8411
dat="-drive if=none,id=dat,format=raw,file=$data -device virtio-blk-pci,drive=dat,serial=ALPDATA02"

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
        (( ++tries < 450 )) || { echo "FAIL: Immich does not answer on 443 in 15 minutes (last answer: $code)" >&2
            grep -a 'IMMDIAG' "$ALP_SERIAL_LOG" | tr -d '\r' | tail -n 120 >&2; return 1; }
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

do_install() { # <target file> <serial> <installer arguments, with $t and $a>
    local args="-drive if=none,id=tgt,format=raw,file=$1 -device virtio-blk-pci,drive=tgt,serial=$2 $dat" out st=0
    local probe="-/usr/bin/sh -c \"n() { basename \$(readlink -f /dev/disk/by-id/virtio-\$1); }; t=\$(n $2); a=\$(n ALPDATA02); tok=\$(/usr/bin/alp-installer plan $3 | grep -o confirm_token.:.[0-9a-f]* | cut -c17-); /usr/bin/alp-installer install $3 --confirm-erase $4 --confirm-token \$tok\""
    out=$(ALP_OVERLAY='' ALP_OVERLAY_SIZE='' ALP_TPM='' ALP_RAM=2G ALP_PROBE="$probe" ALP_PROBE_DELAY=10 ALP_QEMU_ARGS="$args" "$here/run-vm.sh" "$image" 900) || st=$?
    echo "$out"
    if (( st != 0 )) || ! grep -q '^RESULT ok' <<<"$out"; then echo "FAIL: T-IMMICH-03 (install on $2)" >&2; exit 1; fi
    echo "ok:   installed on $2"
}

# The recovery code (T42 flow, as in T-INST-02 part 3), printed for the host.
read -r -d '' rec_script <<'SH' || true
req() {
    exec 3<>/dev/tcp/127.0.0.1/8411
    printf '%s /api/v1/live/%s HTTP/1.0\r\nHost: 127.0.0.1:8411\r\nOrigin: http://127.0.0.1:8411\r\nCookie: %s\r\nContent-Type: application/json\r\nContent-Length: %s\r\n\r\n%s' \
        "$1" "$2" "$(cat /run/qa-cookie)" "${#3}" "$3" >&3
    timeout 180 cat <&3 | tr -d '\r' | sed '1,/^$/d'
    exec 3<&-
}
for i in $(seq 150); do mountpoint -q /var/lib/alp-pool/files && break; sleep 2; done
r=$(req POST recovery/create "{\"password\":\"$1\"}")
read -r -a w <<<"$(grep -o '"words":\[[^]]*\]' <<<"$r" | sed 's/^"words"://' | grep -o '"[a-z]*"' | tr -d '"' | tr '\n' ' ')"
body=""
for i in $(grep -o '"check":\[[0-9,]*\]' <<<"$r" | grep -o '[0-9]\+'); do
    body+="${body:+,}\"$i\":\"${w[i-1]}\""
done
echo "REC-COMMIT $(req POST recovery/commit "{\"words\":{$body}}")" >/dev/console
echo "REC-WORDLIST ${w[*]}" >/dev/console
SH
# Reopen with the words, waiting for the job (as in T-INST-02 part 5).
read -r -d '' reo_script <<'SH' || true
req() {
    exec 3<>/dev/tcp/127.0.0.1/8411
    printf '%s /api/v1/live/%s HTTP/1.0\r\nHost: 127.0.0.1:8411\r\nOrigin: http://127.0.0.1:8411\r\nCookie: %s\r\nContent-Type: application/json\r\nContent-Length: %s\r\n\r\n%s' \
        "$1" "$2" "$(cat /run/qa-cookie)" "${#3}" "$3" >&3
    timeout 180 cat <&3 | tr -d '\r' | sed '1,/^$/d'
    exec 3<&-
}
echo "REO-POST $(req POST archive/reopen-pool "{\"words\":\"$1\"}" | head -c 200)" >/dev/console
for i in $(seq 150); do mountpoint -q /var/lib/alp-pool/files && break; sleep 2; done
mountpoint -q /var/lib/alp-pool/files && echo REO-OPEN=yes >/dev/console || echo REO-OPEN=no >/dev/console
SH

# In the guest: every minute, the photo service's state on the console
# (printed by the hook when the gallery does not answer).
read -r -d '' diag <<'DIAG' || true
for t in 240 300; do
    sleep "$t"
    {
        echo "units $(systemctl is-active alp-immich.service alp-live-archive.service alpd.service user@50000.service | tr '\n' ' ')"
        for m in /var/lib/alp-pool/files /var/lib/alp-index/immich /run/alp/immich/originals; do
            echo "mount $m: $(findmnt -n -o SOURCE,FSTYPE "$m" | tr '\n' ' ')"
        done
        journalctl -b -o short-monotonic --no-pager -u alp-immich.service -u alp-live-archive.service -u user@50000.service | tail -n 25
        journalctl -b _UID=50000 -o short-monotonic --no-pager 2>/dev/null | grep -iE 'immich|condition|fail' | tail -n 12
        journalctl -b -u alpd -o short-monotonic --no-pager | grep -iE 'immich|gallery|pool|archive' | tail -n 12
    } | sed 's/^/IMMDIAG /' >/dev/console
done
DIAG
diag_b64=$(base64 -w0 <<<"$diag")

# 1. Install.
do_install "$target" ALPTARGET01 '--system $t --data $a' '$t,$a'

# 2. First start: pool, recovery code, photos, database backup.
export ALP_OVERLAY="$work/installed.qcow2" ALP_OVERLAY_SIZE=40G ALP_TPM=1 ALP_TPM_STATE="$work/tpm"
status=0
port=$(qa_free_port)
args="$dat -nic user,model=virtio-net-pci,hostfwd=tcp:127.0.0.1:$port-:443"
probe="$(QA_HTTP_TIMEOUT=90 qa_http_probe SETUP POST /api/v1/live/password/setup "$origin" "{\"password\":\"$pw\"}")
$(QA_HTTP_TIMEOUT=30 qa_http_probe POOL POST /api/v1/live/archive/create-pool "$origin" '')
-/bin/bash -c \"echo $(base64 -w0 <<<"$rec_script") | base64 -d > /run/t-rec.sh; bash /run/t-rec.sh $pw\"
-/usr/bin/systemd-run --unit=alp-immich-diag /bin/bash -c 'echo $diag_b64 | base64 -d | bash'
-/usr/bin/systemd-run --unit=alp-restore-agent /bin/bash -c 'echo $agent_b64 | base64 -d | bash -s 1'
/bin/bash -c 'sleep 60'"
out=$(ROUND=1 PORT=$port PW=$pw ALP_RAM=${ALP_IMMICH_RAM:-4G} ALP_PROBE="$probe" ALP_PROBE_DELAY=20 ALP_QEMU_ARGS="$args" \
    ALP_HOST_HOOK=hook "$here/run-vm.sh" "$target" 1500) || status=$?
words=$(sed -n 's/^.*REC-WORDLIST //p' <<<"$out" | tr -d '\r' | head -1)
grep -v 'REC-WORDLIST' <<<"$out"
(( status == 0 )) || { echo "FAIL: T-IMMICH-03 (first start)" >&2; exit 1; }
[[ $words =~ ^[a-z]+( [a-z]+){14}$ ]] || { echo "FAIL: no recovery code made" >&2; exit 1; }
echo "ok:   recovery code made"

# 3. The system disk is lost: install on a new one, the data disk reopened.
do_install "$target2" ALPTARGET04 '--system $t --reopen $a' '$t'

# 4. First start with the words: the gallery comes back.
export ALP_OVERLAY="$work/installed2.qcow2" ALP_TPM_STATE="$work/tpm2"
port=$(qa_free_port)
args="$dat -nic user,model=virtio-net-pci,hostfwd=tcp:127.0.0.1:$port-:443"
probe="$(QA_HTTP_TIMEOUT=90 qa_http_probe SETUP POST /api/v1/live/password/setup "$origin" "{\"password\":\"$pw\"}")
-/bin/bash -c \"echo $(base64 -w0 <<<"$reo_script") | base64 -d > /run/t-reo.sh; bash /run/t-reo.sh '$words'\"
-/usr/bin/systemd-run --unit=alp-immich-diag /bin/bash -c 'echo $diag_b64 | base64 -d | bash'
-/usr/bin/systemd-run --unit=alp-restore-agent /bin/bash -c 'echo $agent_b64 | base64 -d | bash -s 2'
/bin/bash -c 'sleep 60'"
out=$(ROUND=2 PORT=$port PW=$pw ALP_RAM=${ALP_IMMICH_RAM:-4G} ALP_PROBE="$probe" ALP_PROBE_DELAY=20 ALP_QEMU_ARGS="$args" \
    ALP_HOST_HOOK=hook "$here/run-vm.sh" "$target2" 1500) || status=$?
echo "${out//$words/<words>}"
(( status == 0 )) || { echo "FAIL: T-IMMICH-03 (reopen start)" >&2; exit 1; }
grep -q 'REO-OPEN=yes' <<<"$out" || { echo "FAIL: the words did not open the pool" >&2; exit 1; }
echo "ok:   the words opened the pool on the new system disk"
echo "PASS: T-IMMICH-03"
