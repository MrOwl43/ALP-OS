#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# T-LIVE-02: ALP-OS Live storage after the 2026-10-19 redesign
# (docs/plans/live-alpha-decisions-2026-10-19.md, ADR 0011, docs/plans/live-alpha-api.md).
#
# Disks: the stick (overlay of the image), the "computer's disk" (virtio, GPT
# with ESP + NTFS-type partition, like Windows) and an EXTERNAL disk
# (`-device usb-storage`, one empty ext4 partition).
#
# Boot 1: first start (lock.state "setup"); the admin password is set through
#   the API; the internal disk is not offered and creating an archive on its
#   NTFS partition is refused; the external disk is offered; the archive is
#   created on it; a file is written; the recovery key is read if the API has it.
# Boot 2 (same stick, overlay kept):
#   case "tpm" (swtpm, state kept across boots): the archive reopens without
#     the password;
#   case "password" (no TPM): the state says the password is needed; wrong
#     passwords are refused (403) and back off (429); POST unlock opens it.
# Boot 3 (case "password" only, needs the recovery key from boot 1): a fresh
#   stick (new overlay: state and index gone) with the same external disk; the
#   recovery key reopens the archive. SKIPs this part while the API has no
#   recovery endpoint.
# Always: the internal disk is read-only and byte-for-byte unchanged.
# SKIPs while the live API is missing (nothing listens on 8411).
#
# Recovery (docs/plans/live-alpha-api.md, third change): POST live/recovery/create
# returns the key once; on a re-written stick POST live/archive/recover
# {partuuid, recovery_key} (202 job) opens the archive. The console lock
# (`console.state`) is not tested here: the probes run without a session cookie,
# so only `state`, `unlock`, `password/setup` are used on a locked console.
#
# Usage: tests/vm/t-live-02.sh <disk-image>
set -euo pipefail
here=$(dirname "$(realpath "$0")")
# shellcheck disable=SC1091
. "$here/../lib/qa.sh"
image=${1:?usage: t-live-02.sh <disk-image>}
[[ -f $image ]] || { echo "FAIL: image not found: $image" >&2; exit 2; }

origin=http://127.0.0.1:8411
pw=alpha-test-password-1
ext_uuid=4a1d2b3c-0000-4000-8000-0000000000e1
ntfs_uuid=4a1d2b3c-0000-4000-8000-0000000000a2

block() { sed -n "/$2-BEGIN/,/$2-END/p" <<<"$1"; }
body_json() { sed -n '/^{/,$p' <<<"$1" | sed '/-END$/d'; }
pwjson() { printf '{"password":"%s"}' "$1"; }
post() { qa_http_probe "$1" POST "$2" "$origin" "$3"; }
get() { qa_http_probe "$1" GET "$2"; }
# Sleep, but not when the API is not there (keeps the SKIP case fast).
sl() { echo "/bin/bash -c 'exec 3<>/dev/tcp/127.0.0.1/8411 2>/dev/null || exit 0; sleep $1'"; }

# T42: the recovery code in two steps, run in the guest (curl, the console
# cookie of the probes). A code made and confirmed opens; a second create
# without replace is refused; a replacement that is cancelled leaves the old
# code working; a confirmed replacement makes the old code stop. The last
# code's answer is printed as the RECOVERY block for boot 3.
rcycle() { # password
local g
read -r -d '' g <<GUEST || true
A=http://127.0.0.1:8411/api/v1/live
c() { curl -s -m 120 -H "Cookie: \$(cat /run/qa-cookie)" -H 'Origin: http://127.0.0.1:8411' -H 'Content-Type: application/json' "\$@"; }
words() { tr -d '\n' | grep -o '"words": *\[[^]]*\]' | sed 's/^"words"//' | grep -o '"[a-z]*"' | tr -d '"'; }
phr() { echo "\$1" | words | tr '\n' ' '; }
commit() {
  local ws ck b n
  ws=(\$(echo "\$1" | words)); ck=\$(echo "\$1" | tr -d '\n' | grep -o '"check": *\[[0-9, ]*\]' | grep -o '[0-9][0-9]*')
  b=; for n in \$ck; do b="\$b\"\$n\":\"\${ws[\$((n-1))]}\","; done
  c -o /dev/null -w '%{http_code}' -X POST \$A/recovery/commit -d "{\"words\":{\${b%,}}}"
}
opens() { c -X POST \$A/recovery/check -d "{\"recovery_key\":\"\$1\"}" | grep -q '"opens": *true' && echo yes || echo no; }
t() { [ "\$2" = "\$3" ] && echo "ok:   \$1" || echo "FAIL: \$1 (\$2)"; }
echo RCYCLE-BEGIN
ra=\$(c -X POST \$A/recovery/create -d '{"password":"$1"}'); a=\$(phr "\$ra")
t "a new code is confirmed with three words" "\$(commit "\$ra")" 200
t "the confirmed code opens the archive" "\$(opens "\$a")" yes
t "a second code needs replace" "\$(c -o /dev/null -w '%{http_code}' -X POST \$A/recovery/create -d '{"password":"$1"}')" 409
t "a wrong password does not sign in" "\$(c -o /dev/null -w '%{http_code}' -X POST \$A/console/signin -d '{"password":"wrong"}')" 403
t "a wrong password makes no code" "\$(c -o /dev/null -w '%{http_code}' -X POST \$A/recovery/create -d '{"password":"wrong","replace":true}')" 403
# Security review F2: every screen that asks for the password refuses a
# wrong one (on a TPM stick cryptsetup once let any password through). At most
# four wrong ones in a row, so the back-off (5) never answers instead; a right
# password in between resets it.
wrong() { c -o /dev/null -w '%{http_code}' -X POST \$A/\$1 -d '{"password":"wrong"}'; }
t "a wrong password is refused by password/check" "\$(wrong password/check)" 403
t "a wrong password resets nothing" "\$(wrong reset)" 403
rb=\$(c -X POST \$A/recovery/create -d '{"password":"$1","replace":true}'); b=\$(phr "\$rb")
c -X POST \$A/recovery/cancel -d '{}' >/dev/null
t "after cancel the old code still opens" "\$(opens "\$a")" yes
t "the cancelled code leaves no pending file" "\$(ls /run/alp/host/ALPArchive/recovery.conf.pending 2>/dev/null | wc -l)" 0
rc=\$(c -X POST \$A/recovery/create -d '{"password":"$1","replace":true}'); cw=\$(phr "\$rc")
t "the replacement is confirmed" "\$(commit "\$rc")" 200
t "the new code opens" "\$(opens "\$cw")" yes
t "the old code stopped working" "\$(opens "\$a")" no
t "the cancelled code never worked" "\$(opens "\$b")" no
t "a wrong password does not empty the trash" "\$(wrong gallery/trash/empty)" 403
t "a wrong password pairs no phone" "\$(wrong phone/pair)" 403
echo RCYCLE-END
echo RECOVERY-BEGIN; echo "HTTP/1.1 200 OK"; echo; echo "\$rc"; echo RECOVERY-END
GUEST
echo "-/bin/bash -c 'echo $(base64 -w0 <<<"$g") | base64 -d | bash'"
}

run_case() { # tpm | password
local mode=$1 tpm=0 work status=0 fail=0 args
echo "== case: $mode"
[[ $mode == tpm ]] && tpm=1
work=$(mktemp -d -p "$(dirname "$(realpath "$image")")" t-live-02.XXXXXX)
# shellcheck disable=SC2064
trap "rm -rf '$work'" RETURN
local internal="$work/internal.raw" external="$work/external.raw"

# Internal disk: GPT, ESP (vfat, Windows boot loader file) + NTFS partition.
truncate -s 1G "$internal"
sfdisk -q "$internal" <<SFDISK
label: gpt
start=2048, size=131072, type=C12A7328-F81F-11D2-BA4B-00A0C93EC93B
start=133120, size=1900000, type=EBD0A0A2-B9E5-4433-87C0-68B6B72699C7, uuid=$ntfs_uuid
SFDISK
mkfs.vfat -F 32 --offset 2048 "$internal" 65536 >/dev/null
mmd -i "$internal@@$((2048 * 512))" ::/EFI ::/EFI/Microsoft ::/EFI/Microsoft/Boot
echo MZ >"$work/bootmgfw.efi"
mcopy -i "$internal@@$((2048 * 512))" "$work/bootmgfw.efi" ::/EFI/Microsoft/Boot/bootmgfw.efi
dd if=/dev/zero of="$work/ntfs.img" bs=512 count=1900000 conv=sparse status=none
mkfs.ntfs -q -F -L Windows "$work/ntfs.img"
dd if="$work/ntfs.img" of="$internal" bs=512 seek=133120 conv=notrunc status=none
rm -f "$work/ntfs.img"
local before
before=$(sha256sum "$internal" | cut -d' ' -f1)

# External disk: sparse 24 GiB, one ext4 partition.
truncate -s 24G "$external"
sfdisk -q "$external" <<SFDISK
label: gpt
start=2048, size=46137344, type=0FC63DAF-8483-4772-8E79-3D69D8477DE4, uuid=$ext_uuid
SFDISK
mkfs.ext4 -q -F -E offset=$((2048 * 512)) "$external" $((46137344 / 2))k

args="-drive if=virtio,format=raw,file=$internal -drive if=none,id=ext,format=raw,file=$external -device qemu-xhci -device usb-storage,drive=ext"
export ALP_OVERLAY="$work/stick.qcow2" ALP_TPM=$tpm ALP_TPM_STATE="$work/tpm"
local alive="-/bin/bash -c 'exec 3<>/dev/tcp/127.0.0.1/8411 && echo API-LISTENING || echo API-NOT-LISTENING'"
local readfile="-/usr/bin/cat /var/lib/alp-pool/files/test.txt"

local probe1
probe1="$alive
$(get STATE1 /api/v1/live/state)
$(QA_HTTP_TIMEOUT=90 post SETUP /api/v1/live/password/setup "$(pwjson "$pw")")
$(get STATE1B /api/v1/live/state)
$(get CAND /api/v1/live/archive/candidates)
$(post REFUSE /api/v1/live/archive/create "{\"partuuid\":\"$ntfs_uuid\",\"size_bytes\":1073741824}")
$(post CREATE /api/v1/live/archive/create "{\"partuuid\":\"$ext_uuid\",\"size_bytes\":1073741824,\"alp_password\":\"$pw\"}")
$(sl 60)
$(get STATE2 /api/v1/live/state)
-/usr/bin/sh -c 'echo archive-ok > /var/lib/alp-pool/files/test.txt; sync'
$(QA_HTTP_TIMEOUT=90 rcycle "$pw")
/usr/sbin/blockdev --getro /dev/vdb
-/usr/bin/dd if=/dev/zero of=/dev/vdb bs=512 count=1 oflag=direct"
local out1 out2 out3="" out4=""
out1=$(ALP_RAM=${ALP_RAM:-2G} ALP_PROBE="$probe1" ALP_PROBE_DELAY=15 ALP_QEMU_ARGS="$args" "$here/run-vm.sh" "$image" 600) || status=$?
echo "$out1"
(( status == 0 )) || { echo "FAIL: T-LIVE-02 boot 1 ($mode)" >&2; return 1; }
grep -q 'API-LISTENING' <<<"$out1" || qa_skip "T-LIVE-02: nothing listens on 127.0.0.1:8411 yet (live API not in this image)"
grep -q -E '"lock": ?\{' <<<"$(block "$out1" STATE1)" || qa_skip "T-LIVE-02: live/state has no lock block yet (password/TPM storage not in this image)"

local probe2
if [[ $mode == tpm ]]; then
probe2="$(sl 40)
$(get STATE3 /api/v1/live/state)
-/usr/bin/sh -c \"echo TPMDIAG-BEGIN; journalctl -b -u alpd --no-pager | grep -i 'security chip'; ls -l /dev/tpmrm0; echo TPMDIAG-END\"
$readfile"
else
probe2="$(get STATE3 /api/v1/live/state)
$(post BAD1 /api/v1/live/unlock "$(pwjson wrong-1)")
$(post BAD2 /api/v1/live/unlock "$(pwjson wrong-2)")
$(post BAD3 /api/v1/live/unlock "$(pwjson wrong-3)")
$(post BAD4 /api/v1/live/unlock "$(pwjson wrong-4)")
$(post BAD5 /api/v1/live/unlock "$(pwjson wrong-5)")
$(post BAD6 /api/v1/live/unlock "$(pwjson wrong-6)")
$(sl 35)
$(QA_HTTP_TIMEOUT=90 post UNLOCK /api/v1/live/unlock "$(pwjson "$pw")")
$(sl 40)
$(get STATE4 /api/v1/live/state)
$readfile"
fi
# With the chip the archive and Immich open by themselves at this start:
# Immich needs the memory of T-IMMICH-01, and the start takes longer (CI run
# 41015475111 ran out of its 600 s in 2 GB).
ram2=${ALP_RAM:-2G}; limit2=600
[[ $mode == tpm ]] && { ram2=${ALP_IMMICH_RAM:-4G}; limit2=900; }
out2=$(ALP_RAM=$ram2 ALP_PROBE="$probe2" ALP_PROBE_DELAY=15 ALP_QEMU_ARGS="$args" "$here/run-vm.sh" "$image" "$limit2") || status=$?
echo "$out2"
(( status == 0 )) || { echo "FAIL: T-LIVE-02 boot 2 ($mode)" >&2; return 1; }

# Boot 3: recovery key on a re-written stick (fresh overlay), password case only.
local rkey="" recov_ok=0
if [[ $mode == password ]]; then
    rkey=$(body_json "$(block "$out1" RECOVERY)" | jq -r '.recovery_key // empty' 2>/dev/null | head -1 || true)
    if [[ -n $rkey ]] && grep -qE '^HTTP/1\.[01] 200' <<<"$(block "$out1" RECOVERY)"; then
        recov_ok=1
        local probe3
        probe3="$(get STATE5 /api/v1/live/state)
$(QA_HTTP_TIMEOUT=90 post SETUP3 /api/v1/live/password/setup "$(pwjson "$pw")")
$(get CAND3 /api/v1/live/archive/candidates)
$(post RECOVER /api/v1/live/archive/recover "{\"partuuid\":\"$ext_uuid\",\"recovery_key\":\"$rkey\"}")
$(sl 60)
$(get STATE6 /api/v1/live/state)
$readfile"
        ALP_OVERLAY="$work/stick2.qcow2"
        out3=$(ALP_RAM=${ALP_RAM:-2G} ALP_PROBE="$probe3" ALP_PROBE_DELAY=15 ALP_QEMU_ARGS="$args" "$here/run-vm.sh" "$image" 600) || status=$?
        echo "$out3"
        (( status == 0 )) || { echo "FAIL: T-LIVE-02 boot 3 ($mode)" >&2; return 1; }
    else
        echo "SKIP: recovery key part (live/recovery/create gave no key in this image)"
    fi
    # Boot 4 (ADR 0016 level 3): another re-written stick reopens the archive
    # with the ALP password alone.
    local probe4
    probe4="$(QA_HTTP_TIMEOUT=90 post SETUP4 /api/v1/live/password/setup "$(pwjson "$pw")")
$(get CAND4 /api/v1/live/archive/candidates)
$(post REOPENPW /api/v1/live/archive/recover "{\"partuuid\":\"$ext_uuid\",\"alp_password\":\"$pw\"}")
$(sl 60)
$(get STATE7 /api/v1/live/state)
$readfile"
    ALP_OVERLAY="$work/stick3.qcow2"
    out4=$(ALP_RAM=${ALP_RAM:-2G} ALP_PROBE="$probe4" ALP_PROBE_DELAY=15 ALP_QEMU_ARGS="$args" "$here/run-vm.sh" "$image" 600) || status=$?
    echo "$out4"
    (( status == 0 )) || { echo "FAIL: T-LIVE-02 boot 4 ($mode)" >&2; return 1; }
fi

# The archive object nests job and place, so check it with jq, not a regex.
expect_archive_open() { if body_json "$2" | jq -e '.archive.state == "open"' >/dev/null 2>&1; then echo "ok:   $1"; else echo "FAIL: $1" >&2; fail=1; fi; }
expect() { if grep -qE -- "$3" <<<"$2"; then echo "ok:   $1"; else echo "FAIL: $1" >&2; fail=1; fi; }
expect "first start asks to choose a password" "$(block "$out1" STATE1)" '"state": ?"setup"'
expect "password setup works" "$(block "$out1" SETUP)" '^HTTP/1\.[01] 200'
expect "lock is open after the password was set" "$(block "$out1" STATE1B)" '"lock": ?\{[^}]*"state": ?"open"'
local cand
cand=$(block "$out1" CAND)
expect "external disk is offered and usable" "$cand" "$ext_uuid"
if grep -qE "vdb|nvme|\"disk\": ?\"vd|$ntfs_uuid" <<<"$cand"; then
    # Listed but flagged unusable is acceptable; offered as usable is not.
    if jq -e --arg u "$ntfs_uuid" '[.candidates[]? | select(.partuuid == $u and .usable == true)] | length == 0' >/dev/null 2>&1 <<<"$(body_json "$cand")"; then
        echo "ok:   internal OS disk is listed only as not usable"
    else echo "FAIL: the computer's internal disk is offered as usable" >&2; fail=1; fi
else echo "ok:   internal disk (ESP + NTFS) is not offered"; fi
if grep -qE '^HTTP/1\.[01] 4[0-9][0-9]' <<<"$(block "$out1" REFUSE)"; then echo "ok:   archive on the internal OS disk is refused"; else echo "FAIL: archive on the internal OS disk was not refused" >&2; fail=1; fi
expect "archive create on the external disk accepted" "$(block "$out1" CREATE)" '^HTTP/1\.[01] 202'
expect_archive_open "archive open after create" "$(block "$out1" STATE2)"
expect "internal disk is read-only" "$out1" '^1\s*$'
expect "write to the internal disk is refused" "$out1" 'Operation not permitted|Read-only'
if [[ $mode == tpm ]]; then
expect "TPM: state says the chip is used" "$(block "$out2" STATE3)" '"tpm": ?"on"'
expect "TPM: lock open after restart (no password asked)" "$(block "$out2" STATE3)" '"lock": ?\{[^}]*"state": ?"open"'
expect_archive_open "TPM: archive open after restart without a password" "$(block "$out2" STATE3)"
else
expect "no TPM: password needed after restart" "$(block "$out2" STATE3)" '"lock": ?\{[^}]*"state": ?"locked"'
for n in 1 2 3 4 5; do expect "wrong password $n refused (403)" "$(block "$out2" BAD$n)" '^HTTP/1\.[01] 403'; done
expect "sixth wrong password backs off (429)" "$(block "$out2" BAD6)" '^HTTP/1\.[01] 429'
expect "right password unlocks after the wait" "$(block "$out2" UNLOCK)" '^HTTP/1\.[01] 200'
expect_archive_open "archive open after unlock" "$(block "$out2" STATE4)"
fi
expect "file is still there after a restart" "$out2" '^archive-ok'
# T42: the two-step recovery code (boot 1).
rc_res=$(block "$out1" RCYCLE | tr -d "\r" | grep -E '^(ok|FAIL):' || true)
echo "$rc_res"
if [[ -z $rc_res ]] || grep -q '^FAIL:' <<<"$rc_res"; then echo "FAIL: recovery code in two steps" >&2; fail=1; fi
if (( recov_ok )); then
expect "re-written stick starts at setup" "$(block "$out3" STATE5)" '"lock": ?\{[^}]*"state": ?"setup"'
expect "password setup on the new stick works" "$(block "$out3" SETUP3)" '^HTTP/1\.[01] 200'
expect "existing archive is recognised on the external disk" "$(block "$out3" CAND3)" '"has_archive": ?true'
expect "recovery key accepted" "$(block "$out3" RECOVER)" '^HTTP/1\.[01] 202'
expect_archive_open "archive open again with the recovery key" "$(block "$out3" STATE6)"
expect "file is still there after recovery" "$out3" '^archive-ok'
if [[ $mode == password ]]; then
    expect "level 3: the ALP password reopens the archive on another stick" "$(block "$out4" REOPENPW)" '^HTTP/1\.[01] 202'
    expect_archive_open "archive open again with the ALP password" "$(block "$out4" STATE7)"
    expect "file is still there after reopening with the password" "$out4" '^archive-ok'
fi
fi
if [[ $(sha256sum "$internal" | cut -d' ' -f1) == "$before" ]]; then echo "ok:   internal disk unchanged (byte for byte)"; else echo "FAIL: internal disk changed" >&2; fail=1; fi
return "$fail"
}

rc=0
run_case tpm || rc=1
run_case password || rc=1
(( rc == 0 )) || { echo "FAIL: T-LIVE-02" >&2; exit 1; }
echo "PASS: T-LIVE-02"
