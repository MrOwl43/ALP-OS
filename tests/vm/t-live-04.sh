#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# T-LIVE-04 (task T08): "Delete everything and start over" (POST live/reset)
# and cancelling an archive that is being made (POST live/archive/cancel).
# docs/plans/live-alpha-api.md (reset, archive/cancel); ADR 0014, ADR 0016.
#
# Disks: the stick (overlay of the image), the computer's disk (virtio, one
# ext4 partition, must stay byte-for-byte unchanged) and an EXTERNAL disk
# (usb-storage, one ext4 partition) that holds a file of the user's own,
# `keep-me.txt`, at its top and inside `ALPArchive/`.
#
# Boot 1 (one guest script, ok:/FAIL: lines):
#   password setup (level 3, encrypted index);
#   archive/create, then at once archive/cancel: the job ends "cancelled",
#     archive.state is "none", no ALPArchive data is left, the user's files
#     are unchanged, the disk is read-only again;
#   archive/create again: the archive opens; a test file is written;
#   reset with a wrong password: 403, nothing deleted;
#   reset with the right password: 202, job "reset" ends "done"; lock.state "setup", archive.state
#     "none"; the index partition holds no LUKS header any more; no key
#     file (password.conf, recovery.conf, recovery.conf.new) is left on the
#     disk after cancel or reset; the stick holds no
#     index or archive config (no secrets); ALPArchive's own files are gone,
#     the user's files stay.
# Boot 2 (same stick): starts at setup (nothing was remembered).
# Boot 3 (a new stick, level 1 "no password", ADR 0016): a marker written
#   into the plain index (where the gallery database lives), then reset:
#   the job erases the partition for real, and the marker cannot be found
#   anywhere on the raw partition afterwards (owner, 2026-11-03).
# Always: the computer's disk is unchanged.
#
# Usage: tests/vm/t-live-04.sh <disk-image>
set -euo pipefail
here=$(dirname "$(realpath "$0")")
# shellcheck disable=SC1091
. "$here/../lib/qa.sh"
image=${1:?usage: t-live-04.sh <disk-image>}
[[ -f $image ]] || { echo "FAIL: image not found: $image" >&2; exit 2; }

work=$(mktemp -d -p "$(dirname "$(realpath "$image")")" t-live-04.XXXXXX)
trap 'rm -rf "$work"' EXIT
pw=alpha-test-password-1
ext_uuid=4a1d2b3c-0000-4000-8000-0000000004e1
int_uuid=4a1d2b3c-0000-4000-8000-0000000004a1

# The computer's disk: a small GPT disk with one ext4 partition.
internal="$work/internal.raw"
truncate -s 1G "$internal"
sfdisk -q "$internal" <<SFDISK
label: gpt
start=2048, size=2000000, type=0FC63DAF-8483-4772-8E79-3D69D8477DE4, uuid=$int_uuid
SFDISK
mkfs.ext4 -q -F -E offset=$((2048 * 512)) "$internal" $((2000000 / 2))k
before=$(sha256sum "$internal" | cut -d' ' -f1)

# The external disk: sparse 24 GiB, one ext4 partition with the user's files.
external="$work/external.raw"
truncate -s 24G "$external"
sfdisk -q "$external" <<SFDISK
label: gpt
start=2048, size=46137344, type=0FC63DAF-8483-4772-8E79-3D69D8477DE4, uuid=$ext_uuid
SFDISK
mkdir -p "$work/seed/ALPArchive"
echo keep-top >"$work/seed/keep-me.txt"
echo keep-inside >"$work/seed/ALPArchive/keep-me.txt"
mkfs.ext4 -q -F -d "$work/seed" -E offset=$((2048 * 512)) "$external" $((46137344 / 2))k

args="-drive if=virtio,format=raw,file=$internal -drive if=none,id=ext,format=raw,file=$external -device qemu-xhci -device usb-storage,drive=ext"
export ALP_OVERLAY="$work/stick.qcow2"

# Runs in the guest as root; ok:/FAIL: lines between RST-BEGIN/END.
read -r -d '' guest <<GUEST || true
set -u
A=http://127.0.0.1:8411/api/v1/live
c() { curl -s -m 90 -b /run/rst-jar -c /run/rst-jar -H 'Origin: http://127.0.0.1:8411' -H 'Content-Type: application/json' "\$@"; }
code() { curl -s -m 90 -o /run/rst-body -w '%{http_code}' -b /run/rst-jar -c /run/rst-jar -H 'Origin: http://127.0.0.1:8411' -H 'Content-Type: application/json' "\$@"; }
ok() { echo "ok:   \$1"; }
bad() { echo "FAIL: \$1"; }
# The archive job's block in live/state: "job":{... "id":"archive.create", ... "state":"…"}.
job() { c \$A/state | grep -o '"job": *{[^}]*"id": *"archive.create"[^}]*}' | head -1; }
# live/state as one line for the host to check with jq (serde_json sorts
# keys, so archive.state comes after the nested job and place objects).
snap() { echo "SNAP-\$1 \$(c \$A/state | tr -d '\n')"; }
lstate() { c \$A/state | grep -o '"lock": *{[^}]*}' | grep -o '"state": *"[a-z_]*"' | head -1; }
ext() { lsblk -nrpo NAME,PARTUUID | awk -v u=$ext_uuid 'tolower(\$2)==u{print \$1}' | head -1; }
# Look at the external disk read-only, from a private mount point.
peek() { mkdir -p /run/rst-peek; mount -o ro "\$(ext)" /run/rst-peek 2>/dev/null || mount -o ro,norecovery "\$(ext)" /run/rst-peek; }
unpeek() { umount /run/rst-peek 2>/dev/null || true; }
# The wrapped master keys are the most sensitive files on the disk.
nokeys() {
  left=
  for f in password.conf recovery.conf recovery.conf.new gocryptfs.conf; do
    [ -e "/run/rst-peek/ALPArchive/\$f" ] && left="\$left \$f"
  done
  [ -z "\$left" ] && ok "no archive key files on the disk \$1" || bad "key files left \$1:\$left"
}
echo RST-BEGIN

[ "\$(code -X POST \$A/password/setup -d '{"password":"$pw"}')" = 200 ] && ok "password setup" || bad "password setup: \$(cat /run/rst-body)"

# 1. Cancel while the archive is being made (the disk checks take seconds).
c -X POST \$A/archive/create -d '{"partuuid":"$ext_uuid","alp_password":"$pw"}' >/run/rst-create
r=\$(code -X POST \$A/archive/cancel -d '{}')
[ "\$r" = 202 ] && ok "cancel accepted while creating" || bad "cancel answered \$r: \$(cat /run/rst-body) (create: \$(cat /run/rst-create))"
j=
for i in \$(seq 60); do j=\$(job); echo "\$j" | grep -qE '"state": *"(cancelled|done|failed)"' && break; sleep 2; done
echo "\$j" | grep -q '"state": *"cancelled"' && ok "job ends cancelled" || bad "job did not end cancelled: \$j"
snap CANCELLED
d=\$(ext)
[ "\$(blockdev --getro "\$d" 2>/dev/null)" = 1 ] && ok "backup disk is read-only again after cancel" || bad "backup disk is writable after cancel (\$d)"
peek
[ ! -e /run/rst-peek/ALPArchive/data ] && [ ! -e /run/rst-peek/ALPArchive/data.partial ] && [ ! -e /run/rst-peek/ALPArchive/gocryptfs.conf ] \
  && ok "no archive left on the disk after cancel" || bad "archive files left after cancel: \$(ls -A /run/rst-peek/ALPArchive | tr '\n' ' ')"
nokeys "after cancel"
[ "\$(cat /run/rst-peek/keep-me.txt)" = keep-top ] && [ "\$(cat /run/rst-peek/ALPArchive/keep-me.txt)" = keep-inside ] \
  && ok "the user's files are unchanged after cancel" || bad "the user's files changed after cancel"
unpeek

# 2. Make the archive for real, then reset.
c -X POST \$A/archive/create -d '{"partuuid":"$ext_uuid","alp_password":"$pw"}' >/dev/null
for i in \$(seq 90); do mountpoint -q /var/lib/alp-pool/files && break; sleep 5; done
mountpoint -q /var/lib/alp-pool/files && ok "archive open after the second create" || bad "archive did not open: \$(job)"
# The mount comes before the create job ends; until it ends the state
# says "creating" (seen once in CI), so wait for the job before the snapshot.
for i in \$(seq 60); do c \$A/state | tr -d '\n' | grep -q '"state": *"creating"' || break; sleep 1; done
snap OPEN
echo reset-me > /var/lib/alp-pool/files/test.txt; sync

r=\$(code -X POST \$A/reset -d '{"password":"wrong-password-1"}')
[ "\$r" = 403 ] && ok "reset with a wrong password refused (403)" || bad "reset with a wrong password answered \$r"
mountpoint -q /var/lib/alp-pool/files && [ "\$(cat /var/lib/alp-pool/files/test.txt)" = reset-me ] \
  && ok "nothing deleted after the refused reset" || bad "archive gone after the refused reset"
# The wrong try counts with the back-off; wait it out before the right one.
sleep 3
r=\$(code -X POST \$A/reset -d '{"password":"$pw"}')
[ "\$r" = 202 ] && ok "reset with the right password started (202)" || bad "reset answered \$r: \$(cat /run/rst-body)"
rj=
for i in \$(seq 120); do
  rj=\$(c \$A/state | grep -o '"job": *{[^}]*"id": *"reset"[^}]*}' | head -1)
  echo "\$rj" | grep -qE '"state": *"(done|failed)"' && break
  sleep 5
done
echo "\$rj" | grep -q '"state": *"done"' && ok "reset job done" || bad "reset job did not finish: \$rj"
# The cookie went with the console session; state is free to read.
lstate | grep -q '"setup"' && ok "lock.state is setup after reset" || bad "lock.state after reset: \$(lstate)"
snap RESET
idx=\$(lsblk -nrpo NAME,PARTTYPE | awk 'toupper(\$2)=="A3E1C6B2-5B7E-4F0A-9C1D-6B2F0E8A4D11"{print \$1}' | head -1)
[ -b "\$idx" ] && ! cryptsetup isLuks "\$idx" && ok "index header is gone (no key slot left: password, TPM)" || bad "index \$idx still holds a LUKS header"
[ ! -e /var/lib/alp/live/index.toml ] && [ ! -e /var/lib/alp/live/archive.toml ] \
  && ok "the stick keeps no index or archive config" || bad "config left on the stick: \$(ls /var/lib/alp/live 2>&1 | tr '\n' ' ')"
grep -qE '/var/lib/alp-index|/var/lib/alp-pool' /proc/self/mountinfo && bad "index or archive still mounted" || ok "index and archive unmounted"
peek
[ ! -e /run/rst-peek/ALPArchive/data ] && [ ! -e /run/rst-peek/ALPArchive/gocryptfs.conf ] && [ ! -e /run/rst-peek/ALPArchive/README.txt ] \
  && ok "ALPArchive's own files are gone" || bad "ALPArchive files left after reset: \$(ls -A /run/rst-peek/ALPArchive | tr '\n' ' ')"
nokeys "after reset"
[ "\$(cat /run/rst-peek/keep-me.txt)" = keep-top ] && [ "\$(cat /run/rst-peek/ALPArchive/keep-me.txt)" = keep-inside ] \
  && ok "the user's files stay after reset" || bad "the user's files were touched by reset"
unpeek
echo RST-END
GUEST

b64=$(base64 -w0 <<<"$guest")
probe="-/bin/bash -c 'echo $b64 | base64 -d | bash'"
status=0
out=$(ALP_RAM=${ALP_RAM:-2G} ALP_PROBE="$probe" ALP_PROBE_DELAY=15 ALP_QEMU_ARGS="$args" \
    "$here/run-vm.sh" "$image" 900 2>&1) || status=$?
if ! grep -q 'RST-BEGIN' <<<"$out"; then
    grep -q 'ALP-PROBE-BEGIN' <<<"$out" || { tail -n 40 <<<"$out"; echo "FAIL: T-LIVE-04 boot 1" >&2; exit 1; }
fi
res=$(sed -n '/RST-BEGIN/,/RST-END/p' <<<"$out" | tr -d '\r' | grep -E '^(ok|FAIL):' || true)
echo "$res"
if (( status != 0 )) || [[ -z $res ]]; then tail -n 40 <<<"$out"; echo "FAIL: T-LIVE-04 boot 1" >&2; exit 1; fi

# Boot 2: the same stick starts at setup.
probe2="$(qa_http_probe STATE2 GET /api/v1/live/state)"
status=0
out2=$(ALP_RAM=${ALP_RAM:-2G} ALP_PROBE="$probe2" ALP_PROBE_DELAY=15 ALP_QEMU_ARGS="$args" \
    "$here/run-vm.sh" "$image" 600 2>&1) || status=$?
(( status == 0 )) || { tail -n 40 <<<"$out2"; echo "FAIL: T-LIVE-04 boot 2" >&2; exit 1; }

# Boot 3: a new stick at level 1; its plain index is erased for real.
marker="ALP-T-LIVE-04-MARKER-$RANDOM$RANDOM"
read -r -d '' guest3 <<GUEST || true
set -u
A=http://127.0.0.1:8411/api/v1/live
c() { curl -s -m 90 -b /run/w-jar -c /run/w-jar -H 'Origin: http://127.0.0.1:8411' -H 'Content-Type: application/json' "\$@"; }
ok() { echo "ok:   \$1"; }
bad() { echo "FAIL: \$1"; }
# Search a raw device for text. -z: NUL ends a "line", so a zeroed disk
# never becomes one huge line (CI run 41638245800: grep was OOM-killed).
# Exit 0 found, 1 not found, 2+ the search itself failed.
has() { grep -a -q -z -F "\$1" "\$2"; }
echo WIPE-BEGIN
c -X POST \$A/password/setup -d '{"password":"","protection":"none"}' >/dev/null
for i in \$(seq 60); do mountpoint -q /var/lib/alp-index && break; sleep 2; done
idx=\$(lsblk -nrpo NAME,PARTTYPE | awk 'toupper(\$2)=="A3E1C6B2-5B7E-4F0A-9C1D-6B2F0E8A4D11"{print \$1}' | head -1)
# Where the gallery database lives (ADR 0011): plain on a level 1 stick.
mkdir -p /var/lib/alp-index/immich
for i in \$(seq 64); do echo "$marker"; done > /var/lib/alp-index/immich/marker.txt
sync
has "$marker" "\$idx" && ok "marker is on the raw partition before reset" || bad "marker not found before reset (\$idx)"
c -X POST \$A/reset -d '{"password":""}' >/run/w-reset
rj=
for i in \$(seq 240); do
  rj=\$(c \$A/state | grep -o '"job": *{[^}]*"id": *"reset"[^}]*}' | head -1)
  echo "\$rj" | grep -qE '"state": *"(done|failed)"' && break
  sleep 5
done
echo "\$rj" | grep -q '"state": *"done"' && ok "level 1: reset job done" || bad "level 1: reset job did not finish: \$rj \$(cat /run/w-reset)"
echo 3 > /proc/sys/vm/drop_caches
has "$marker" "\$idx"; g=\$?
case \$g in
  1) ok "level 1: the marker is gone from the raw partition" ;;
  0) bad "level 1: the marker is still on the raw partition" ;;
  *) bad "level 1: the raw partition could not be searched (grep \$g)" ;;
esac
[ ! -e /var/lib/alp/live/index-wipe-pending ] && ok "level 1: no unfinished wipe left" || bad "level 1: wipe marker left"
how=\$(journalctl -b -u alpd --no-pager | grep -o 'wipe: [^ ]*: .*' | tail -1)
echo "\$how"
# Review W4: an edges-only wipe would also hide a marker in the first 16 MiB.
case "\$how" in
  *"secure: "*"overwritten with zeros"*) ok "level 1: the whole partition was overwritten with zeros" ;;
  *) bad "level 1: not a full overwrite: \$how" ;;
esac
c \$A/state | grep -q 'overwritten with zeros' && ok "level 1: the result names the method" || bad "level 1: the result does not name the method"
echo WIPE-END
GUEST
b64w=$(base64 -w0 <<<"$guest3")
status=0
export ALP_OVERLAY="$work/stick-level1.qcow2"
out3=$(ALP_RAM=${ALP_RAM:-2G} ALP_PROBE="-/bin/bash -c 'echo $b64w | base64 -d | bash'" ALP_PROBE_DELAY=15 ALP_QEMU_ARGS="$args" \
    "$here/run-vm.sh" "$image" 1800 2>&1) || status=$?
res3=$(sed -n '/WIPE-BEGIN/,/WIPE-END/p' <<<"$out3" | tr -d '\r' | grep -E '^(ok|FAIL):|wipe: ' || true)
echo "$res3"
# Fail closed: every check of boot 3 must have printed (CI run 41622595568
# showed only the first two lines and still passed).
need=("the marker is gone from the raw partition" "no unfinished wipe left"
      "the whole partition was overwritten with zeros" "the result names the method")
missing=0
for n in "${need[@]}"; do grep -qF "ok:   level 1: $n" <<<"$res3" || missing=1; done
if (( status != 0 )) || (( missing )) || ! grep -q 'WIPE-END' <<<"$out3"; then
    tail -n 80 <<<"$out3" | tr -d '\r'; echo "FAIL: T-LIVE-04 boot 3 (a check did not run)" >&2; exit 1
fi

fail=0
grep -q '^FAIL:' <<<"$res" && fail=1
grep -q '^FAIL:' <<<"$res3" && fail=1
snapshot() { tr -d '\r' <<<"$out" | grep -m1 "^SNAP-$1 " | cut -d' ' -f2-; }
expect_jq() { # what, snapshot, jq test
    if jq -e "$3" >/dev/null 2>&1 <<<"$(snapshot "$2")"; then echo "ok:   $1"; else echo "FAIL: $1" >&2; fail=1; fi
}
expect_jq "archive.state is none after cancel" CANCELLED '.archive.state == "none"'
expect_jq "archive.state is open after the second create" OPEN '.archive.state == "open"'
expect_jq "archive.state is none after reset" RESET '.archive.state == "none"'
expect_jq "lock.state is setup after reset" RESET '.lock.state == "setup"'
if sed -n '/STATE2-BEGIN/,/STATE2-END/p' <<<"$out2" | grep -qE '"lock": ?\{[^}]*"state": ?"setup"'; then
    echo "ok:   after a reset the stick starts at setup"
else echo "FAIL: after a reset the stick did not start at setup" >&2; fail=1; fi
if [[ $(sha256sum "$internal" | cut -d' ' -f1) == "$before" ]]; then echo "ok:   computer's disk unchanged (byte for byte)"
else echo "FAIL: computer's disk changed" >&2; fail=1; fi
(( fail == 0 )) || { echo "FAIL: T-LIVE-04" >&2; exit 1; }
echo "PASS: T-LIVE-04"
