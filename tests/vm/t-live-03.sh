#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# T-LIVE-03: encrypt later (ADR 0018, T41). An unprotected stick (ADR 0016
# level 1) with a plain archive and a test photo is encrypted afterwards:
#   boot 1: setup without password, archive, a file; archive/encrypt; the job
#           ends "done"; the index partition is LUKS, the backup disk holds
#           ALPArchive/data and no ALPArchive/files, the file reads back;
#   boot 2: the stick asks for the password; unlock opens the archive and
#           the file is there.
# Needs Immich (the job waits for the photo service before deleting the
# unencrypted copy), so it runs where T-IMMICH-01 runs (ALP_ALLOW_IMMICH=1).
#
# Usage: tests/vm/t-live-03.sh <disk-image>
set -euo pipefail
here=$(dirname "$(realpath "$0")")
# shellcheck disable=SC1091
. "$here/../lib/qa.sh"
image=${1:?usage: t-live-03.sh <disk-image>}
[[ -f $image ]] || { echo "FAIL: image not found: $image" >&2; exit 2; }
if [[ ${ALP_ALLOW_IMMICH:-0} != 1 ]]; then
    echo "SKIP: T-LIVE-03 runs only with ALP_ALLOW_IMMICH=1 (CI sets it; it needs the photo service)"
    exit 0
fi

work=$(mktemp -d -p "$(dirname "$(realpath "$image")")" t-live-03.XXXXXX)
trap 'rm -rf "$work"' EXIT
disk="$work/external.raw"
part=$(qa_make_disk "$disk")
pw=alpha-test-password-1
args="-drive if=none,id=ext,format=raw,file=$disk -device qemu-xhci -device usb-storage,drive=ext"
export ALP_OVERLAY="$work/stick.qcow2"

# Runs in the guest as root; ok:/FAIL: lines between ENC-BEGIN/END.
read -r -d '' guest <<GUEST || true
set -u
A=http://127.0.0.1:8411/api/v1/live
c() { curl -s -m 60 -b /run/enc-jar -c /run/enc-jar -H 'Origin: http://127.0.0.1:8411' -H 'Content-Type: application/json' "\$@"; }
ok() { echo "ok:   \$1"; }
bad() { echo "FAIL: \$1"; }
echo ENC-BEGIN
c -X POST \$A/password/setup -d '{"password":"","protection":"none"}' >/dev/null
c -X POST \$A/archive/create -d '{"partuuid":"$part"}' >/dev/null
# The archive is made (its id file is on the disk) and mounted.
for i in \$(seq 90); do [ -f /run/alp/host/ALPArchive/plain-id.txt ] && mountpoint -q /var/lib/alp-pool/files && break; sleep 5; done
echo encrypt-me > /var/lib/alp-pool/files/test.txt; sync
r=\$(c -X POST \$A/archive/encrypt -d '{"password":"$pw"}')
echo "\$r" | grep -q '"archive.encrypt"' && ok "encrypt job started" || bad "encrypt job did not start: \$r"
# The archive job's own block: "job":{... "id":"archive.encrypt", ... "state":"done"}.
job=; lost=0; st=
for i in \$(seq 150); do
  st=\$(c -m 20 \$A/state)
  # The job's keys come sorted: a failed job's nested "error" object comes
  # before "id", and "id" to "state" holds no braces.
  job=\$(echo "\$st" | grep -o '"id": *"archive.encrypt"[^{}]*"state": *"[a-z]*"' | head -1)
  echo "\$job" | grep -qE '"state": *"(done|failed)"' && break
  # The job is gone from the state (alpd restarted, or no answer): waiting
  # longer only burns CI time; the diagnosis below says why.
  if [ -z "\$job" ]; then lost=\$((lost+1)); [ \$lost -ge 6 ] && break; else lost=0; fi
  sleep 10
done
echo "\$job"
if echo "\$job" | grep -q '"state": *"done"'; then ok "encrypt job done"; else
  bad "encrypt job did not finish: \${job:-no job in the state}"
  echo ENCDIAG-BEGIN
  echo "state: \$(echo "\$st" | head -c 3000)"
  systemctl show -p NRestarts -p ActiveState -p ExecMainStatus alpd.service
  journalctl -b -u alpd --no-pager -o short-monotonic | tail -n 80
  ps -eo pid,etimes,args | grep -E 'cryptsetup|btrfs|gocryptfs' | grep -v grep
  lsblk -o NAME,FSTYPE,SIZE,MOUNTPOINTS
  echo ENCDIAG-END
fi
idx=\$(lsblk -nrpo NAME,PARTTYPE | awk 'toupper(\$2)=="A3E1C6B2-5B7E-4F0A-9C1D-6B2F0E8A4D11"{print \$1}' | head -1)
cryptsetup isLuks "\$idx" && ok "index is encrypted now" || bad "index is not LUKS (\$idx)"
ls -1 /run/alp/host/ALPArchive >/run/enc-ls 2>&1
grep -qx data /run/enc-ls && ok "backup disk holds the encrypted archive" || bad "no ALPArchive/data: \$(tr '\n' ' ' </run/enc-ls)"
grep -qx files /run/enc-ls && bad "the unencrypted copy is still there" || ok "the unencrypted copy is gone"
grep -q fuse.gocryptfs /proc/self/mountinfo && ok "archive is mounted encrypted" || bad "no gocryptfs mount"
[ "\$(cat /var/lib/alp-pool/files/test.txt 2>/dev/null)" = encrypt-me ] && ok "the photo reads back after encrypting" || bad "the photo is missing after encrypting"
echo ENC-END
GUEST

b64=$(base64 -w0 <<<"$guest")
probe="-/bin/bash -c 'echo $b64 | base64 -d | bash'"
status=0
out=$(ALP_RAM=${ALP_IMMICH_RAM:-4G} ALP_PROBE="$probe" ALP_PROBE_DELAY=15 ALP_QEMU_ARGS="$args" \
    "$here/run-vm.sh" "$image" 3000 2>&1) || status=$?
res=$(sed -n '/ENC-BEGIN/,/ENC-END/p' <<<"$out" | tr -d '\r' | grep -E '^(ok|FAIL):' || true)
echo "$res"
sed -n '/ENCDIAG-BEGIN/,/ENCDIAG-END/p' <<<"$out" | tr -d '\r'
if (( status != 0 )) || [[ -z $res ]]; then tail -n 40 <<<"$out"; echo "FAIL: T-LIVE-03 boot 1" >&2; exit 1; fi

# Boot 2: the password is asked for and opens everything.
probe2="$(qa_http_probe STATE1 GET /api/v1/live/state)
$(QA_HTTP_TIMEOUT=90 qa_http_probe UNLOCK POST /api/v1/live/unlock http://127.0.0.1:8411 "{\"password\":\"$pw\"}")
/bin/sleep 60
-/usr/bin/cat /var/lib/alp-pool/files/test.txt"
status=0
out2=$(ALP_RAM=${ALP_IMMICH_RAM:-4G} ALP_PROBE="$probe2" ALP_PROBE_DELAY=15 ALP_QEMU_ARGS="$args" \
    "$here/run-vm.sh" "$image" 900 2>&1) || status=$?
(( status == 0 )) || { tail -n 40 <<<"$out2"; echo "FAIL: T-LIVE-03 boot 2" >&2; exit 1; }
fail=0
grep -q '^FAIL:' <<<"$res" && fail=1
if sed -n '/STATE1-BEGIN/,/STATE1-END/p' <<<"$out2" | grep -qE '"lock": ?\{[^}]*"state": ?"locked"'; then
    echo "ok:   after encrypting, the stick asks for the password"
else echo "FAIL: the stick did not ask for the password" >&2; fail=1; fi
if grep -q '^encrypt-me' <<<"$(tr -d '\r' <<<"$out2")"; then echo "ok:   the photo is there after unlocking"
else echo "FAIL: the photo is missing after unlocking" >&2; fail=1; fi
(( fail == 0 )) || { echo "FAIL: T-LIVE-03" >&2; exit 1; }
echo "PASS: T-LIVE-03"
