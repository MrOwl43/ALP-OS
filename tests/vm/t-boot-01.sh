#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# T-BOOT-01: the image boots headless (QEMU, OVMF) to alp-healthy.target, and
# the running root is the read-only EROFS filesystem on the dm-verity device
# with the roothash on the kernel command line. Before booting, it checks that
# the boot menu editor is disabled in the ESP's loader.conf.
#
# Usage: tests/vm/t-boot-01.sh <disk-image> [timeout-seconds | --print-probe]
# Env:   ALP_SHARED_OUT=<file>  use this probe output instead of booting
set -euo pipefail
here=$(dirname "$(realpath "$0")")
image=${1:?usage: t-boot-01.sh <disk-image> [timeout-seconds]}

probe=$'/usr/bin/findmnt -no SOURCE,FSTYPE,OPTIONS /\n/bin/cat /proc/cmdline'
probe+=$'\n/bin/systemctl show -p Requires boot-complete.target'
# The initrd is not probed: it has no repart.d definitions, so its
# systemd-repart (if present) has nothing to do.
probe+=$'\n-/bin/systemctl is-enabled systemd-repart.service'
if [[ ${2:-} == --print-probe ]]; then printf '%s\n' "$probe"; exit 0; fi

esp_offset=$(sfdisk --json "$image" | python3 -I -c '
import json, sys
parts = [p for p in json.load(sys.stdin)["partitiontable"]["partitions"]
         if p["type"].upper() == "C12A7328-F81F-11D2-BA4B-00A0C93EC93B"]
print(parts[0]["start"] * 512 if len(parts) == 1 else "")')
[[ -n $esp_offset ]] || { echo "FAIL: no single ESP in $image" >&2; exit 1; }
if mtype -i "$image@@$esp_offset" ::/loader/loader.conf | grep -qE '^editor[[:space:]]+no'; then
    echo "  ok: boot menu editor disabled (loader.conf)"
else
    echo "FAIL: loader.conf on the ESP does not say 'editor no'" >&2; exit 1
fi

if [[ -n ${ALP_SHARED_OUT:-} ]]; then
    # Shared boot (tests/vm/t-core.sh): the probe output is already there.
    out=$(<"$ALP_SHARED_OUT")
else
    out=$(ALP_PROBE=$probe \
        "$here/run-vm.sh" "$image" "${2:-180}") || { echo "$out"; echo "FAIL: T-BOOT-01" >&2; exit 1; }
fi
echo "$out"

status=0
check() { # description, ERE that must appear in the probe output
    if grep -qE "$2" <<<"$out"; then echo "  ok: $1"; else echo "  FAIL: $1" >&2; status=1; fi
}
check "root is /dev/mapper/root (dm-verity)" '/dev/mapper/root[[:space:]]+erofs'
check "root is mounted read-only" '/dev/mapper/root[[:space:]]+erofs[[:space:]]+ro[,[:space:]]'
check "kernel command line carries roothash" 'roothash=[0-9a-f]{64}'
check "boot-complete.target requires alp-healthy.target" 'Requires=.*alp-healthy\.target'
check "systemd-repart never runs at boot" 'masked|No such file|not-found'
(( status == 0 )) && echo "PASS: T-BOOT-01"
exit "$status"
