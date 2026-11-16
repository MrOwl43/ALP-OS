#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# T-UPD-01: the stick's layout for updates in place (ADR 0026, step 1 of
# docs/plans/live-update.md). Two first starts:
#   a 32 GiB stick gets two empty root slots (with their verity slots) and the
#   alp-boot partition (FAT, for the start files of new versions) before the
#   index, and the system it was written with is untouched;
#   an 8 GiB stick gets none of them: all its room stays for the index.
# Also: systemd-sysupdate is in the image, its timers are off and the image
# knows its version. (That files signed by an unknown key are refused is
# proven in T-UPD-02, which has a network and an update server.)
#
# Usage: tests/vm/t-upd-01.sh <disk-image>
set -euo pipefail
here=$(dirname "$(realpath "$0")")
# shellcheck disable=SC1091
. "$here/../lib/qa.sh"
image=${1:?usage: t-upd-01.sh <disk-image>}
[[ -f $image ]] || { echo "FAIL: image not found: $image" >&2; exit 2; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
origin=http://127.0.0.1:8411
probe="-/bin/bash -c 'exec 3<>/dev/tcp/127.0.0.1/8411 && echo API-LISTENING || echo API-NOT-LISTENING'
$(QA_HTTP_TIMEOUT=120 qa_http_probe SETUP POST /api/v1/live/password/setup "$origin" '{"password":"alpha-test-password-4"}')
-/bin/bash -c 'echo PARTS-BEGIN; lsblk -rno PARTLABEL,SIZE,FSTYPE /dev/\$(lsblk -rno PKNAME \$(findmnt -no SOURCE /var/lib/alp) | head -n 1); echo PARTS-END'
-/bin/bash -c 'ls /usr/lib/systemd/systemd-sysupdate /usr/lib/systemd/systemd-pull >/dev/null 2>&1 && echo SYSUPDATE-yes || echo SYSUPDATE-no; echo TIMERS-\$(systemctl is-enabled systemd-sysupdate.timer systemd-sysupdate-reboot.timer 2>&1 | paste -sd- -); echo \$(grep ^IMAGE_VERSION= /usr/lib/os-release || echo IMAGE_VERSION=none); echo KEYRING-\$(stat -c %s /usr/lib/systemd/import-pubring.gpg)'
-/bin/bash -c 'echo CHECK-BEGIN; alpctl live update check 2>&1 | tail -n 5; echo CHECK-END'"
fail=0
for size in 32G 8G; do
    status=0
    out=$(ALP_OVERLAY="$work/stick-$size.qcow2" ALP_OVERLAY_SIZE=$size ALP_RAM=${ALP_RAM:-2G} ALP_PROBE="$probe" ALP_PROBE_DELAY=15 \
        "$here/run-vm.sh" "$image" 420) || status=$?
    (( status == 0 )) || { echo "$out" | tail -n 60; echo "FAIL: T-UPD-01 (VM run failed, $size stick)" >&2; exit 1; }
    grep -q 'API-LISTENING' <<<"$out" || qa_skip "T-UPD-01: live API is not in this image yet"
    parts=$(sed -n '/PARTS-BEGIN/,/PARTS-END/p' <<<"$out" | tr -d '\r')
    echo "$parts"
    grep -o 'SYSUPDATE-[a-z]*' <<<"$out" | head -n 1
    empty=$(grep -c '^_empty ' <<<"$parts" || true)
    index=$(grep -c '^alp-index ' <<<"$parts" || true)
    boot=$(grep -c '^alp-boot 1G vfat' <<<"$parts" || true)
    want=0; [[ $size == 32G ]] && want=4
    if [[ $empty == "$want" && $index == 1 && $boot == $(( want / 4 )) ]]; then
        echo "ok:   $size stick: $empty empty update slots, $boot alp-boot partition and the index"
    else
        echo "FAIL: $size stick: $empty empty slots (want $want), $boot alp-boot (want $(( want / 4 ))), $index index partitions (want 1)" >&2; fail=1
    fi
    check=$(sed -n '/CHECK-BEGIN/,/CHECK-END/p' <<<"$out" | tr -d '\r')
    if [[ $size == 32G ]]; then
        # Two 6 GiB root slots and two 96 MiB verity slots.
        [[ $(grep -c '^_empty 6G' <<<"$parts" || true) == 2 && $(grep -c '^_empty 96M' <<<"$parts" || true) == 2 ]] \
            || { echo "FAIL: the update slots do not have the planned sizes" >&2; fail=1; }
        grep -q 'SYSUPDATE-yes' <<<"$out" || { echo "FAIL: systemd-sysupdate or systemd-pull is not in the image" >&2; fail=1; }
        grep -q 'TIMERS-masked-masked' <<<"$out" \
            || { echo "FAIL: systemd's own update timers are not masked: $(grep -o 'TIMERS-[a-z-]*' <<<"$out" | head -n 1)" >&2; fail=1; }
        grep -Eq 'IMAGE_VERSION="?[0-9]+\.[0-9]+\.[0-9]{2}"?' <<<"$out" \
            || { echo "FAIL: the image does not know its version: $(grep -o 'IMAGE_VERSION=[^ ]*' <<<"$out" | head -n 1)" >&2; fail=1; }
    else
        grep -q 'no room for a new version' <<<"$check" \
            || { echo "FAIL: the 8G stick did not say it cannot be updated in place: $check" >&2; fail=1; }
    fi
done
(( fail == 0 )) || { echo "FAIL: T-UPD-01" >&2; exit 1; }
echo "PASS: T-UPD-01"
