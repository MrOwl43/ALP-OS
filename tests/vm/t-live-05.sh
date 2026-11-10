#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# T-LIVE-05 (task T49, disk side): Advanced > Disks "Quick format" and
# "Secure format" (POST live/disks/format, disks/format/estimate);
# docs/plans/live-alpha-api.md; owner 2026-11-03; Security and Network
# Expert conditions (ux-0.2.4-security-review.md section 4, review W2/W3).
#
# Disks: the stick (overlay), the computer's disk (virtio, ext4, must stay
# byte-for-byte unchanged), external disk A (usb-storage, ext4) that becomes
# the archive, and external disk B (usb-storage, serial ALPFMTB, ext4 with a
# marker file) that is formatted.
#
# One boot (ok:/FAIL: lines):
#   password setup (level 3) and archive on A;
#   live/disks (T58, ADR 0022): the computer's disk is listed locked
#     (internal_live), disk B external and free;
#   the computer's disk is refused with 403 disk.locked by format, estimate,
#     prepare and archive/create, also under a forged disk id;
#   format refused: the archive's disk A (422), a wrong password (403), a
#     changed partition list (409), a bad mode (400), no confirmation and a
#     used confirmation (409 format.confirm_stale, review R4);
#   estimate for B: secure.seconds >= 60, both texts, a confirmation;
#   quick format B as ext4 (label QUICK): job done, ext4 labelled QUICK, the result
#     names the method, B read-only again, the archive still open and its
#     partition still writable;
#   secure format B: job done, "overwritten with zeros" in the result, B
#     read-only again;
#   secure format B again, cancelled while overwriting: job "cancelled",
#     "partly erased", B read-only again, no allowance left;
#   live/events: the formats are there, without the password; a journal line
#     written by another user (logger) is not (review R5).
# Host: the marker is nowhere on B's raw image; the computer's disk is
# unchanged.
#
# Usage: tests/vm/t-live-05.sh <disk-image>
set -euo pipefail
here=$(dirname "$(realpath "$0")")
# shellcheck disable=SC1091
. "$here/../lib/qa.sh"
image=${1:?usage: t-live-05.sh <disk-image>}
[[ -f $image ]] || { echo "FAIL: image not found: $image" >&2; exit 2; }

work=$(mktemp -d -p "$(dirname "$(realpath "$image")")" t-live-05.XXXXXX)
trap 'rm -rf "$work"' EXIT
pw=alpha-test-password-1
a_uuid=4a1d2b3c-0000-4000-8000-0000000005a1
b_uuid=4a1d2b3c-0000-4000-8000-0000000005b1
int_uuid=4a1d2b3c-0000-4000-8000-0000000005c1
marker="ALP-T-LIVE-05-MARKER-$RANDOM$RANDOM"

# The computer's disk: a small GPT disk with one ext4 partition.
internal="$work/internal.raw"
truncate -s 1G "$internal"
sfdisk -q "$internal" <<SFDISK
label: gpt
start=2048, size=2000000, type=0FC63DAF-8483-4772-8E79-3D69D8477DE4, uuid=$int_uuid
SFDISK
mkfs.ext4 -q -F -E offset=$((2048 * 512)) "$internal" $((2000000 / 2))k
before=$(sha256sum "$internal" | cut -d' ' -f1)

# Disk A (the archive): sparse 24 GiB, one ext4 partition.
disk_a="$work/a.raw"
truncate -s 24G "$disk_a"
sfdisk -q "$disk_a" <<SFDISK
label: gpt
start=2048, size=46137344, type=0FC63DAF-8483-4772-8E79-3D69D8477DE4, uuid=$a_uuid
SFDISK
mkfs.ext4 -q -F -E offset=$((2048 * 512)) "$disk_a" $((46137344 / 2))k

# Disk B (formatted): 1 GiB, one ext4 partition holding the marker.
disk_b="$work/b.raw"
truncate -s 1G "$disk_b"
sfdisk -q "$disk_b" <<SFDISK
label: gpt
start=2048, size=2000000, type=0FC63DAF-8483-4772-8E79-3D69D8477DE4, uuid=$b_uuid
SFDISK
mkdir -p "$work/seed"
for _ in $(seq 256); do echo "$marker"; done >"$work/seed/private.txt"
mkfs.ext4 -q -F -d "$work/seed" -E offset=$((2048 * 512)) "$disk_b" $((2000000 / 2))k
grep -a -q -z -F "$marker" "$disk_b" || { echo "FAIL: T-LIVE-05 setup: marker not on disk B" >&2; exit 2; }

args="-drive if=virtio,format=raw,file=$internal -device qemu-xhci"
args+=" -drive if=none,id=da,format=raw,file=$disk_a -device usb-storage,drive=da,serial=ALPFMTA"
args+=" -drive if=none,id=db,format=raw,file=$disk_b -device usb-storage,drive=db,serial=ALPFMTB"
export ALP_OVERLAY="$work/stick.qcow2"

read -r -d '' guest <<GUEST || true
set -u
A=http://127.0.0.1:8411/api/v1/live
c() { curl -s -m 120 -b /run/fmt-jar -c /run/fmt-jar -H 'Origin: http://127.0.0.1:8411' -H 'Content-Type: application/json' "\$@"; }
code() { curl -s -m 120 -o /run/fmt-body -w '%{http_code}' -b /run/fmt-jar -c /run/fmt-jar -H 'Origin: http://127.0.0.1:8411' -H 'Content-Type: application/json' "\$@"; }
ok() { echo "ok:   \$1"; }
bad() { echo "FAIL: \$1"; }
# Search a raw device for text. -z: NUL ends a "line", so a zeroed disk
# never becomes one huge line (CI run 41638245800: grep was OOM-killed).
# Exit 0 found, 1 not found, 2+ the search itself failed.
has() { grep -a -q -z -F "\$1" "\$2"; }
snap() { echo "SNAP-\$1 \$(c \$A/state | tr -d '\n')"; }
# Disk by USB serial, its id (name-diskseq) and its partitions' PARTUUIDs as JSON.
disk() { lsblk -dnro NAME,SERIAL | awk -v s="\$1" '\$2==s{print \$1}' | head -1; }
did() { echo "\$1-\$(cat /sys/block/\$1/diskseq)"; }
parts() { lsblk -nro PARTUUID "/dev/\$1" | awk 'NF{printf "%s\"%s\"", (n++?",":""), \$1}'; }
# A fresh one-time confirmation for a disk, from the estimate (review R4).
confirm() { c -X POST \$A/disks/format/estimate -d "{\"disk_id\":\"\$(did \$1)\",\"partuuids\":[\$(parts \$1)]}" | tr -d '\n' | grep -o '"confirm": *"[0-9a-f]*"' | grep -o '[0-9a-f]\{32\}'; }
body() { # disk, mode, password, label, [confirm], [fs]
  echo "{\"disk_id\":\"\$(did \$1)\",\"partuuids\":[\$(parts \$1)],\"mode\":\"\$2\",\"password\":\"\$3\",\"label\":\"\$4\",\"confirm\":\"\${5:-}\",\"fs\":\"\${6:-exfat}\"}"
}
# The archive job's state and step (serde_json sorts keys: they come last).
jstate() { c \$A/state | tr -d '\n' | grep -o '"state": *"[a-z]*", *"step": *"[a-z]*"' | head -1; }
waitjob() { # until the step is \$1 or the job ended; at most \$2 x 2 s
  for i in \$(seq "\$2"); do
    s=\$(jstate)
    echo "\$s" | grep -q "\"step\": *\"\$1\"" && break
    echo "\$s" | grep -qE '"state": *"(done|failed|cancelled)"' && break
    sleep 2
  done
}
allro() { # every partition and the disk itself read-only
  for d in /dev/\$1 \$(lsblk -nrpo NAME "/dev/\$1" | tail -n +2); do
    [ "\$(blockdev --getro "\$d")" = 1 ] || { echo "\$d"; return 1; }
  done
}
echo FMT-BEGIN

[ "\$(code -X POST \$A/password/setup -d '{"password":"$pw"}')" = 200 ] && ok "password setup" || bad "password setup: \$(cat /run/fmt-body)"
c -X POST \$A/archive/create -d '{"partuuid":"$a_uuid","alp_password":"$pw"}' >/dev/null
for i in \$(seq 90); do mountpoint -q /var/lib/alp-pool/files && break; sleep 5; done
mountpoint -q /var/lib/alp-pool/files && ok "archive open on disk A" || bad "archive did not open"
echo keep-me > /var/lib/alp-pool/files/keep.txt; sync
da=\$(disk ALPFMTA); db=\$(disk ALPFMTB)
# The computer's disk by its partition; the stick by the index partition type.
vi=\$(lsblk -nro PKNAME,PARTUUID | awk -v u=$int_uuid 'tolower(\$2)==u{print \$1}' | head -1)
st=\$(lsblk -nro PKNAME,PARTTYPE | awk 'toupper(\$2)=="A3E1C6B2-5B7E-4F0A-9C1D-6B2F0E8A4D11"{print \$1}' | head -1)
apart=\$(lsblk -nrpo NAME,PARTUUID | awk -v u=$a_uuid 'tolower(\$2)==u{print \$1}')
[ -n "\$da" ] && [ -n "\$db" ] && [ -n "\$vi" ] && [ -n "\$st" ] && ok "disks found (A \$da, B \$db, computer \$vi, stick \$st)" || bad "disks not found: A '\$da' B '\$db' computer '\$vi' stick '\$st'"

# The disk list (T58): checked on the host with jq.
echo "DISKS \$vi \$db \$(c \$A/disks | tr -d '\n')"

# The computer's own disk is locked in Live (403 disk.locked), whatever
# the call and also under a forged id (another diskseq).
locked() { [ "\$1" = 403 ] && grep -q 'disk.locked' /run/fmt-body; }
r=\$(code -X POST \$A/disks/format -d "\$(body \$vi quick $pw x)")
locked "\$r" && ok "format of the computer's disk: 403 disk.locked" || bad "computer's disk format: \$r \$(cat /run/fmt-body)"
r=\$(code -X POST \$A/disks/format -d "{\"disk_id\":\"\$vi-999999\",\"partuuids\":[],\"mode\":\"quick\",\"password\":\"$pw\",\"confirm\":\"\",\"fs\":\"exfat\"}")
locked "\$r" && ok "format under a forged id: 403 disk.locked" || bad "forged id format: \$r \$(cat /run/fmt-body)"
r=\$(code -X POST \$A/disks/format/estimate -d "{\"disk_id\":\"\$(did \$vi)\",\"partuuids\":[\$(parts \$vi)]}")
locked "\$r" && ok "estimate of the computer's disk: 403 disk.locked" || bad "computer's disk estimate: \$r \$(cat /run/fmt-body)"
r=\$(code -X POST \$A/disks/prepare -d "{\"disk_id\":\"\$(did \$vi)\",\"partuuids\":[\$(parts \$vi)],\"password\":\"$pw\"}")
locked "\$r" && ok "prepare of the computer's disk: 403 disk.locked" || bad "computer's disk prepare: \$r \$(cat /run/fmt-body)"
r=\$(code -X POST \$A/archive/create -d '{"partuuid":"$int_uuid","alp_password":"$pw"}')
case "\$r" in 403|409) grep -qE 'disk.locked|archive.exists' /run/fmt-body && ok "archive on the computer's disk refused (\$r)" || bad "computer's disk archive: \$r \$(cat /run/fmt-body)" ;; *) bad "computer's disk archive: \$r \$(cat /run/fmt-body)" ;; esac
# The stick is never formatted.
r=\$(code -X POST \$A/disks/format -d "\$(body \$st quick $pw x)")
case "\$r" in 403|404|422) ok "the stick is refused (\$r)" ;; *) bad "stick: \$r \$(cat /run/fmt-body)" ;; esac

# Refusals of an external disk: nothing written.
# The archive's disk: refused at the estimate already (no confirmation is
# given), and a format without one is refused too.
r=\$(code -X POST \$A/disks/format/estimate -d "{\"disk_id\":\"\$(did \$da)\",\"partuuids\":[\$(parts \$da)]}")
[ "\$r" = 422 ] && grep -q 'backup disk' /run/fmt-body && ok "the archive's disk is refused at the estimate (422)" || bad "archive disk estimate: \$r \$(cat /run/fmt-body)"
r=\$(code -X POST \$A/disks/format -d "\$(body \$da quick $pw x)")
case "\$r" in 409|422) ok "the archive's disk is refused (\$r)" ;; *) bad "archive disk: \$r \$(cat /run/fmt-body)" ;; esac
r=\$(code -X POST \$A/disks/format -d "\$(body \$db quick $pw x)")
[ "\$r" = 409 ] && grep -q 'format.confirm_stale' /run/fmt-body && ok "no confirmation is refused (409)" || bad "no confirmation: \$r \$(cat /run/fmt-body)"
# Refusals before the confirmation is checked leave it valid.
t=\$(confirm \$db)
r=\$(code -X POST \$A/disks/format -d "\$(body \$db fast $pw x \$t)")
[ "\$r" = 400 ] && ok "an unknown mode is refused (400)" || bad "bad mode: \$r"
r=\$(code -X POST \$A/disks/format -d "\$(body \$db quick $pw 'bad/label' \$t)")
[ "\$r" = 400 ] && ok "a bad label is refused (400)" || bad "bad label: \$r"
r=\$(code -X POST \$A/disks/format -d "\$(body \$db quick wrong-password-1 x \$t)")
[ "\$r" = 403 ] && ok "a wrong password is refused (403)" || bad "wrong password: \$r"
sleep 3 # the wait after a wrong password
r=\$(code -X POST \$A/disks/format -d "{\"disk_id\":\"\$(did \$db)\",\"partuuids\":[],\"mode\":\"quick\",\"password\":\"$pw\",\"confirm\":\"\$t\"}")
[ "\$r" = 409 ] && grep -q 'disk.changed' /run/fmt-body && ok "a changed partition list is refused (409)" || bad "changed list: \$r \$(cat /run/fmt-body)"
# That attempt used the confirmation up.
r=\$(code -X POST \$A/disks/format -d "\$(body \$db quick $pw x \$t)")
[ "\$r" = 409 ] && grep -q 'format.confirm_stale' /run/fmt-body && ok "a used confirmation is refused (409)" || bad "used confirmation: \$r \$(cat /run/fmt-body)"
sleep 3
has "$marker" "/dev/\$db" && ok "disk B untouched by the refusals" || bad "disk B changed by a refusal"

# Estimate (nothing written).
r=\$(code -X POST \$A/disks/format/estimate -d "{\"disk_id\":\"\$(did \$db)\",\"partuuids\":[\$(parts \$db)]}")
[ "\$r" = 200 ] && grep -qE '"confirm": *"[0-9a-f]{32}"' /run/fmt-body && ok "estimate answered with a confirmation" || bad "estimate: \$r \$(cat /run/fmt-body)"
echo "ESTIMATE \$(tr -d '\n' </run/fmt-body)"

# Quick format, as ext4.
r=\$(code -X POST \$A/disks/format -d "\$(body \$db quick $pw QUICK \$(confirm \$db) ext4)")
[ "\$r" = 202 ] && ok "quick format started (202)" || bad "quick format: \$r \$(cat /run/fmt-body)"
waitjob never 90
snap QUICK
udevadm settle
[ "\$(lsblk -nro FSTYPE,LABEL "/dev/\$db" | awk 'NF==2' | head -1)" = "ext4 QUICK" ] && ok "B is ext4 labelled QUICK" || bad "B after quick: \$(lsblk -nro NAME,FSTYPE,LABEL /dev/\$db | tr '\n' ' ')"
bro=\$(allro \$db) && ok "B is read-only again after quick format" || bad "B writable after quick format: \$bro"
mountpoint -q /var/lib/alp-pool/files && [ "\$(cat /var/lib/alp-pool/files/keep.txt)" = keep-me ] && ok "archive still open after quick format" || bad "archive closed by quick format"
[ "\$(blockdev --getro "\$apart")" = 0 ] && ok "archive partition still writable" || bad "archive partition read-only after format"
has "$marker" "/dev/\$db" && echo "info: quick format left old data readable (as the screen says)" || echo "info: quick format happened to cover the marker"

# Secure format.
r=\$(code -X POST \$A/disks/format -d "\$(body \$db secure $pw SECURE \$(confirm \$db))")
[ "\$r" = 202 ] && ok "secure format started (202)" || bad "secure format: \$r \$(cat /run/fmt-body)"
waitjob never 600
snap SECURE
udevadm settle
has "$marker" "/dev/\$db"; g=\$?
case \$g in
  1) ok "marker gone from B after secure format" ;;
  0) bad "marker still on B after secure format" ;;
  *) bad "B could not be searched (grep \$g)" ;;
esac
[ "\$(lsblk -nro FSTYPE,LABEL "/dev/\$db" | awk 'NF==2' | head -1)" = "exfat SECURE" ] && ok "B is exFAT labelled SECURE" || bad "B after secure: \$(lsblk -nro NAME,FSTYPE,LABEL /dev/\$db | tr '\n' ' ')"
bro=\$(allro \$db) && ok "B is read-only again after secure format" || bad "B writable after secure format: \$bro"

# Secure format, cancelled while overwriting.
r=\$(code -X POST \$A/disks/format -d "\$(body \$db secure $pw CANCEL \$(confirm \$db))")
[ "\$r" = 202 ] && ok "second secure format started (202)" || bad "second secure format: \$r \$(cat /run/fmt-body)"
waitjob erasing 60
r=\$(code -X POST \$A/archive/cancel -d '{}')
[ "\$r" = 202 ] && ok "cancel accepted while overwriting" || bad "cancel answered \$r: \$(cat /run/fmt-body)"
waitjob never 120
snap CANCELLED
bro=\$(allro \$db) && ok "B is read-only again after cancel" || bad "B writable after cancel: \$bro"
[ ! -e /run/alp/allowed-format ] && ok "no format allowance left" || bad "format allowance left: \$(cat /run/alp/allowed-format)"
journalctl -b -u alpd --no-pager | grep -E 'disk.format|wipe: ' | tail -8
# Events (review R5): another user's journal line, tagged like alpd's, is
# not an event; the list has the formats and never the password.
setpriv --reuid=65534 --regid=65534 --clear-groups logger -t alpd 'ALP-EVENT disk.format done disk="FORGED" size=1 mode=quick fs=exfat code=-'
sleep 1
ev=\$(c \$A/events | tr -d '\n')
echo "EVENTS \$ev"
echo "\$ev" | grep -q FORGED && bad "a forged journal line became an event" || ok "a forged journal line is not an event"
echo "\$ev" | grep -qF "$pw" && bad "the password is in the events" || ok "no password in the events"
echo FMT-END
GUEST

b64=$(base64 -w0 <<<"$guest")
status=0
out=$(ALP_RAM=${ALP_RAM:-2G} ALP_PROBE="-/bin/bash -c 'echo $b64 | base64 -d | bash'" ALP_PROBE_DELAY=15 ALP_QEMU_ARGS="$args" \
    "$here/run-vm.sh" "$image" 2400 2>&1) || status=$?
res=$(sed -n '/FMT-BEGIN/,/FMT-END/p' <<<"$out" | tr -d '\r' | grep -E '^(ok|FAIL|info):|disk.format|wipe: |^ESTIMATE |^DISKS |^EVENTS ' || true)
echo "$res"
if (( status != 0 )) || ! grep -q '^ok:' <<<"$res"; then tail -n 40 <<<"$out"; echo "FAIL: T-LIVE-05" >&2; exit 1; fi

fail=0
grep -q '^FAIL:' <<<"$res" && fail=1
snapshot() { tr -d '\r' <<<"$out" | grep -m1 "^SNAP-$1 " | cut -d' ' -f2-; }
expect_jq() { # what, snapshot, jq test
    if jq -e "$3" >/dev/null 2>&1 <<<"$(snapshot "$2")"; then echo "ok:   $1"; else echo "FAIL: $1" >&2; fail=1; fi
}
est=$(grep -m1 '^ESTIMATE ' <<<"$res" | cut -d' ' -f2- || true)
if jq -e '.secure.seconds >= 60 and .secure.about == true and (.quick.text | test("brought back")) and (.secure.text | test("not a guarantee")) and .warnings == []' >/dev/null 2>&1 <<<"$est"; then
    echo "ok:   estimate gives minutes and both honest texts"
else echo "FAIL: estimate: $est" >&2; fail=1; fi
disks=$(grep -m1 '^DISKS ' <<<"$res" || true)
read -r _ vi_name db_name disks_json <<<"$disks"
if jq -e --arg v "$vi_name" --arg b "$db_name" '(.disks[] | select(.name == $v) | .kind == "internal" and .locked and .lock == "internal_live") and (.disks[] | select(.name == $b) | .kind == "external" and (.locked | not))' >/dev/null 2>&1 <<<"$disks_json"; then
    echo "ok:   live/disks: the computer's disk locked, disk B free"
else echo "FAIL: live/disks: $disks" >&2; fail=1; fi
evs=$(grep -m1 '^EVENTS ' <<<"$res" | cut -d' ' -f2- || true)
if jq -e '([.events[] | select(.kind == "disk.format" and .result == "done")] | length >= 2) and any(.events[]; .fs == "ext4") and all(.events[]; (keys | sort) == ["at","code","disk","fs","kind","mode","result","size_bytes"])' >/dev/null 2>&1 <<<"$evs"; then
    echo "ok:   live/events lists the formats in the fixed form"
else echo "FAIL: live/events: $evs" >&2; fail=1; fi
expect_jq "quick format done, method named" QUICK '.archive.job.id == "disk.format" and .archive.job.state == "done" and (.archive.job.message | test("quick"))'
expect_jq "archive still open after quick format" QUICK '.archive.state == "open"'
expect_jq "secure format done with a full zero overwrite" SECURE '.archive.job.state == "done" and (.archive.job.message | test("overwritten with zeros")) and (.archive.job.message | test("partial") | not)'
expect_jq "cancelled secure format says partly erased" CANCELLED '.archive.job.state == "cancelled" and (.archive.job.message | test("partly erased"))'
expect_jq "archive still open at the end" CANCELLED '.archive.state == "open"'
# On the host too: the marker is nowhere on disk B's raw image.
g=0; grep -a -q -z -F "$marker" "$disk_b" || g=$?
case $g in
    1) echo "ok:   marker is nowhere on disk B (host)" ;;
    0) echo "FAIL: marker still on disk B (host)" >&2; fail=1 ;;
    *) echo "FAIL: disk B could not be searched on the host (grep $g)" >&2; fail=1 ;;
esac
if [[ $(sha256sum "$internal" | cut -d' ' -f1) == "$before" ]]; then echo "ok:   computer's disk unchanged (byte for byte)"
else echo "FAIL: computer's disk changed" >&2; fail=1; fi
(( fail == 0 )) || { echo "FAIL: T-LIVE-05" >&2; exit 1; }
echo "PASS: T-LIVE-05"
