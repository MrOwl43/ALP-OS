#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Guest commands are single-quoted on purpose: they expand in the VM.
# shellcheck disable=SC2016
# T-INST-02: install with two data disks and the encrypted pool on the
# first start (plan docs/plans/0.2.8-first-start.md). CI only. Three sparse
# 40 GiB files in this script's own work directory (never a disk of this
# machine): ALPTARGET01 system disk; ALPDATA02 and ALPDATA03 dedicated data
# disks (with dedicated data disks the system disk is not in the pool).
# 1. The installer image installs with --system <target> --data <a> --data <b>.
# 2. The installed disk starts (stick out) with both data disks and a TPM;
#    password chosen through the API, then archive/create-pool. Passes when
#    the pool is RAID1 for data and metadata, the seed is gone, the record is
#    written, and a 200 MB test file lands in the archive.
# 3. Restart: the index opens with the TPM, the pool opens by itself, the
#    file is intact.
# 4. Restart without the second data disk: read-only (degraded),
#    the file is still readable.
# Test data stays under 1 GB (owner rule until the beta).
#
# Usage: tests/vm/t-inst-02.sh <disk-image>
set -euo pipefail
here=$(dirname "$(realpath "$0")")
# shellcheck disable=SC1091
. "$here/../lib/qa.sh"
image=${1:?usage: t-inst-02.sh <disk-image>}
[[ -f $image ]] || { echo "FAIL: image not found: $image" >&2; exit 2; }

work=$(mktemp -d -p "$(dirname "$(realpath "$image")")" t-inst-02.XXXXXX)
trap 'rm -rf "$work"' EXIT
target="$work/target.raw" data="$work/data.raw" data2="$work/data2.raw"
truncate -s 40G "$target"
truncate -s 40G "$data"
truncate -s 40G "$data2"

origin=http://127.0.0.1:8411
pw=alpha-test-password-2
post() { qa_http_probe "$1" POST "$2" "$origin" "$3"; }

fail=0
expect() { # label, ERE, output
    if grep -qE -- "$2" <<<"$3"; then echo "ok:   $1"; else echo "FAIL: $1" >&2; fail=1; fi
}

# Part 1: install.
args="-drive if=none,id=tgt,format=raw,file=$target -device virtio-blk-pci,drive=tgt,serial=ALPTARGET01"
args+=" -drive if=none,id=dat,format=raw,file=$data -device virtio-blk-pci,drive=dat,serial=ALPDATA02"
args+=" -drive if=none,id=da2,format=raw,file=$data2 -device virtio-blk-pci,drive=da2,serial=ALPDATA03"
probe='-/usr/bin/sh -c "n() { basename $(readlink -f /dev/disk/by-id/virtio-$1); }; t=$(n ALPTARGET01); a=$(n ALPDATA02); b=$(n ALPDATA03); /usr/bin/alp-installer install --system $t --data $a --data $b --confirm-erase $t,$a,$b"'
status=0
out=$(ALP_RAM=${ALP_RAM:-2G} ALP_PROBE="$probe" ALP_PROBE_DELAY=10 ALP_QEMU_ARGS="$args" \
    "$here/run-vm.sh" "$image" 900) || status=$?
echo "$out"
(( status == 0 )) || { echo "FAIL: T-INST-02 (installer VM run failed)" >&2; exit 1; }
expect "installer finished" '^RESULT ok' "$out"
(( fail == 0 )) || { echo "FAIL: T-INST-02" >&2; exit 1; }

# Parts 2-4 boot the installed disk; its overlay and the TPM state are kept.
export ALP_OVERLAY="$work/installed.qcow2" ALP_OVERLAY_SIZE=40G ALP_TPM=1 ALP_TPM_STATE="$work/tpm"
one_data="-drive if=none,id=dat,format=raw,file=$data -device virtio-blk-pci,drive=dat,serial=ALPDATA02"
with_data="$one_data -drive if=none,id=da2,format=raw,file=$data2 -device virtio-blk-pci,drive=da2,serial=ALPDATA03"
wait_open='-/bin/bash -c "for i in $(seq 150); do mountpoint -q /var/lib/alp-pool/files && break; sleep 2; done; mountpoint -q /var/lib/alp-pool/files && echo POOL-OPEN=yes || echo POOL-OPEN=no"'
show='-/bin/bash -c "btrfs filesystem df /run/alp/pool | sed s/^/DF=/"
-/bin/bash -c "findmnt -no OPTIONS /run/alp/pool | tr , \"\\n\" | grep -qx ro && echo POOL-RO=yes || echo POOL-RO=no"
-/bin/bash -c "test -e /var/lib/alp/firstboot-seed.toml && echo SEED=yes || echo SEED=no"
-/bin/bash -c "sed -n s/^data_profile/RECORD-PROFILE/p /var/lib/alp/pool.toml"
-/bin/bash -c "findmnt -no SOURCE /var/lib/alp-index/immich | grep -q @immich && echo IMMICH-ON-POOL=yes || echo IMMICH-ON-POOL=no"'

# Part 2: first start.
probe2="$(QA_HTTP_TIMEOUT=90 post SETUP /api/v1/live/password/setup "{\"password\":\"$pw\"}")
$(QA_HTTP_TIMEOUT=30 post POOL /api/v1/live/archive/create-pool '')
$wait_open
$show
-/bin/bash -c \"head -c 200M /dev/urandom > /var/lib/alp-pool/files/t.bin && sync && sha256sum /var/lib/alp-pool/files/t.bin | sed s/^/SUM=/\""
out=$(ALP_RAM=${ALP_RAM:-2G} ALP_PROBE="$probe2" ALP_PROBE_DELAY=5 ALP_QEMU_ARGS="$with_data" \
    "$here/run-vm.sh" "$target" 900) || status=$?
echo "$out"
(( status == 0 )) || { echo "FAIL: T-INST-02 (first start failed)" >&2; exit 1; }
expect "create-pool accepted" '"job"' "$(sed -n '/POOL-BEGIN/,/POOL-END/p' <<<"$out")"
expect "pool opened" '^POOL-OPEN=yes' "$out"
expect "data is RAID1" '^DF=Data, RAID1' "$out"
expect "metadata is RAID1" '^DF=Metadata, RAID1' "$out"
expect "seed removed" '^SEED=no' "$out"
expect "record written" '^RECORD-PROFILE = "raid1"' "$out"
expect "photo service data is on the pool" '^IMMICH-ON-POOL=yes' "$out"
sum=$(sed -n 's/^SUM=\([0-9a-f]\{64\}\).*/\1/p' <<<"$out" | head -1)
if [[ -n $sum ]]; then echo "ok:   test file written"; else echo "FAIL: test file written" >&2; fail=1; fi
(( fail == 0 )) || { echo "FAIL: T-INST-02" >&2; exit 1; }

# Part 3: restart; the TPM opens the index, the pool opens by itself.
probe3="$wait_open
$show
-/bin/bash -c \"sha256sum /var/lib/alp-pool/files/t.bin | sed s/^/SUM=/\""
out=$(ALP_RAM=${ALP_RAM:-2G} ALP_PROBE="$probe3" ALP_PROBE_DELAY=5 ALP_QEMU_ARGS="$with_data" \
    "$here/run-vm.sh" "$target" 900) || status=$?
echo "$out"
(( status == 0 )) || { echo "FAIL: T-INST-02 (restart failed)" >&2; exit 1; }
expect "pool opened after restart" '^POOL-OPEN=yes' "$out"
expect "read-write with both disks" '^POOL-RO=no' "$out"
expect "file intact after restart" "^SUM=$sum" "$out"

# Part 4: the second data disk is missing: read-only, degraded, file readable.
out=$(ALP_RAM=${ALP_RAM:-2G} ALP_PROBE="$probe3" ALP_PROBE_DELAY=5 ALP_QEMU_ARGS="$one_data" \
    "$here/run-vm.sh" "$target" 900) || status=$?
echo "$out"
(( status == 0 )) || { echo "FAIL: T-INST-02 (start without the data disk failed)" >&2; exit 1; }
expect "pool opened with one data disk missing" '^POOL-OPEN=yes' "$out"
expect "read-only when a disk is missing" '^POOL-RO=yes' "$out"
expect "file readable when a disk is missing" "^SUM=$sum" "$out"

(( fail == 0 )) || { echo "FAIL: T-INST-02" >&2; exit 1; }
echo "PASS: T-INST-02"
