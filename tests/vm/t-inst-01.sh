#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# T-INST-01: one-disk install (plan docs/plans/0.2.8-installer.md, VM tests).
# CI only. Boots the installer image with two virtual disks, both sparse
# files in this script's own work directory (never a disk of this machine):
#   64 GiB, empty: the install target (serial ALPTARGET01);
#   40 GiB, a fake Windows disk (GPT: ESP with a Windows boot loader
#   file, Microsoft reserved, NTFS; serial ALPWINDOWS01).
# Kernel names (vda, vdb, ...) follow PCI order and are not fixed, so the
# guest finds the target by its serial (/dev/disk/by-id/virtio-ALPTARGET01)
# and the checks match disks by size. The guest runs:
#   alp-installer list
#   alp-installer install ... --confirm-token <wrong>  (must refuse: ALP-INST-401)
#   alp-installer plan --system <target>   (prints confirm_token)
#   alp-installer install --system <target> --confirm-erase <target> --confirm-token <token>
# Passes when:
#   - the Windows disk is listed as not usable (holds_windows),
#   - the Windows disk is bit-for-bit unchanged afterwards,
#   - the installer reports RESULT ok and the target holds exactly the partitions of
#     the disk-layout contract: esp, slot 1 filled (alp-os_<version> and its
#     _verity), slot 2 _empty (root and verity), alp-state, alp-index, alp-data;
#   - the installed disk then starts on its own (stick removed): root from
#     slot 1, the first-boot seed on its alp-state, Windows disk unchanged.
# Until block 4's pool plan digest exists the installer refuses before any
# write with ALP-INST-407; the script then checks that the target is unchanged too
# and exits 3 (not ready), not 0.
# The image must ship /usr/bin/alp-installer, /usr/lib/alp/install-layout and
# the install-time sysupdate definitions (stream A); without the binary the
# script exits 3 as well.
#
# Usage: tests/vm/t-inst-01.sh <disk-image>
set -euo pipefail
here=$(dirname "$(realpath "$0")")
image=${1:?usage: t-inst-01.sh <disk-image>}
[[ -f $image ]] || { echo "FAIL: image not found: $image" >&2; exit 2; }

work=$(mktemp -d -p "$(dirname "$(realpath "$image")")" t-inst-01.XXXXXX)
trap 'rm -rf "$work"' EXIT
target="$work/target.raw" windows="$work/windows.raw"

truncate -s 64G "$target"
target_before=$(sha256sum "$target" | cut -d' ' -f1)

# Fake Windows disk (same recipe as T-LIVE-02, plus Microsoft reserved).
truncate -s 40G "$windows"
sfdisk -q "$windows" <<SFDISK
label: gpt
start=2048, size=131072, type=C12A7328-F81F-11D2-BA4B-00A0C93EC93B
start=133120, size=32768, type=E3C9E316-0B5C-4DB8-817D-F92DF00215AE
start=165888, size=1900000, type=EBD0A0A2-B9E5-4433-87C0-68B6B72699C7
SFDISK
mkfs.vfat -F 32 --offset 2048 "$windows" 65536 >/dev/null
mmd -i "$windows@@$((2048 * 512))" ::/EFI ::/EFI/Microsoft ::/EFI/Microsoft/Boot
echo MZ >"$work/bootmgfw.efi"
mcopy -i "$windows@@$((2048 * 512))" "$work/bootmgfw.efi" ::/EFI/Microsoft/Boot/bootmgfw.efi
dd if=/dev/zero of="$work/ntfs.img" bs=512 count=1900000 conv=sparse status=none
mkfs.ntfs -q -F -L Windows "$work/ntfs.img"
dd if="$work/ntfs.img" of="$windows" bs=512 seek=165888 conv=notrunc status=none
rm -f "$work/ntfs.img"
windows_before=$(sha256sum "$windows" | cut -d' ' -f1)

# Serials give the disks /dev/disk/by-id names; the installer refuses a
# disk without one (ALP-INST-403). VERIFY: virtio-blk-pci serial= property.
args="-drive if=none,id=tgt,format=raw,file=$target -device virtio-blk-pci,drive=tgt,serial=ALPTARGET01"
args+=" -drive if=none,id=win,format=raw,file=$windows -device virtio-blk-pci,drive=win,serial=ALPWINDOWS01"

probe='-/usr/bin/sh -c "test -x /usr/bin/alp-installer && echo INSTALLER=yes || echo INSTALLER=no"
-/usr/bin/alp-installer list
-/usr/bin/sh -c "t=$(basename $(readlink -f /dev/disk/by-id/virtio-ALPTARGET01)); /usr/bin/alp-installer install --system $t --confirm-erase $t --confirm-token 0000000000000000000000000000000000000000000000000000000000000000 | sed s/^RESULT/STALE-RESULT/"
-/usr/bin/sh -c "t=$(basename $(readlink -f /dev/disk/by-id/virtio-ALPTARGET01)); tok=$(/usr/bin/alp-installer plan --system $t | grep -o confirm_token.:.[0-9a-f]* | cut -c17-); /usr/bin/alp-installer install --system $t --confirm-erase $t --confirm-token $tok"'

status=0
out=$(ALP_RAM=${ALP_RAM:-2G} ALP_PROBE="$probe" ALP_PROBE_DELAY=10 ALP_QEMU_ARGS="$args" \
    "$here/run-vm.sh" "$image" 900) || status=$?
echo "$out"
(( status == 0 )) || { echo "FAIL: T-INST-01 (VM run failed)" >&2; exit 1; }

fail=0
expect() {
    if grep -qE -- "$2" <<<"$out"; then echo "ok:   $1"; else echo "FAIL: $1" >&2; fail=1; fi
}
same() { # label, file, hash before
    if [[ $(sha256sum "$2" | cut -d' ' -f1) == "$3" ]]; then echo "ok:   $1 unchanged"; else echo "FAIL: $1 changed" >&2; fail=1; fi
}

if grep -q 'INSTALLER=no' <<<"$out"; then
    echo "NOT READY: the image has no /usr/bin/alp-installer (stream A)" >&2
    same "Windows disk" "$windows" "$windows_before"
    same "target disk" "$target" "$target_before"
    exit 3
fi

expect "Windows disk is greyed out" '^DISK vd[a-z] 42949672960 .*not usable: holds_windows'
expect "target disk is offered" '^DISK vd[a-z] 68719476736 .*usable \[SystemAndData'
same "Windows disk" "$windows" "$windows_before"

if grep -q '^RESULT ALP-INST-407' <<<"$out"; then
    same "target disk" "$target" "$target_before"
    (( fail == 0 )) || { echo "FAIL: T-INST-01" >&2; exit 1; }
    echo "NOT READY: installer refused before writing (block 4 plan digest missing)" >&2
    exit 3
fi
expect "a plan not confirmed is refused before writing" '^STALE-RESULT ALP-INST-401'
expect "installer finished" '^RESULT ok'
expect "progress reached the end" '^PROGRESS 100 .*install\.done'

if python3 -I - "$target" <<'PY'
import json, re, subprocess, sys
parts = json.loads(subprocess.run(["sfdisk", "--json", sys.argv[1]], check=True,
                                  capture_output=True, text=True).stdout)["partitiontable"]["partitions"]
want = [r"esp", r"alp-os_[0-9.]+", r"alp-os_[0-9.]+_verity", r"_empty", r"_empty",
        r"alp-state", r"alp-index", r"alp-data"]
got = [p.get("name", "") for p in parts]
print("partitions:", got)
ok = len(got) == len(want) and all(re.fullmatch(w, g) for w, g in zip(want, got))
sys.exit(0 if ok else 1)
PY
then echo "ok:   target has the contract layout"; else echo "FAIL: target layout" >&2; fail=1; fi

# The seed must be on the target's alp-state (6th partition), read
# without mounting (debugfs reads the image at an offset).
read -r st_start st_size < <(sfdisk --json "$target" | python3 -Ic '
import json, sys
p = json.load(sys.stdin)["partitiontable"]["partitions"][5]
print(p["start"], p["size"])')
dd if="$target" of="$work/state.img" bs=512 skip="$st_start" count="$st_size" status=none
if debugfs -R "stat /firstboot-seed.toml" "$work/state.img" 2>/dev/null | grep -q '^Inode:'; then
    echo "ok:   the seed was written to the target's alp-state"
else
    echo "FAIL: no seed on the target's alp-state" >&2
    debugfs -R "ls -l /" "$work/state.img" 2>&1 | head -20 >&2
    fail=1
fi
rm -f "$work/state.img"

# The installed ESP gets the stick's loader.conf: menu editor off, a menu
# timeout (the "safe graphics" entry can be picked).
esp_start=$(sfdisk --json "$target" | python3 -Ic '
import json, sys
print(json.load(sys.stdin)["partitiontable"]["partitions"][0]["start"] * 512)')
loader=$(mtype -i "$target@@$esp_start" ::/loader/loader.conf 2>&1 || true)
if grep -qE '^editor[[:space:]]+no' <<<"$loader" && grep -qE '^timeout[[:space:]]+[1-9]' <<<"$loader"; then
    echo "ok:   the installed ESP has the stick's loader.conf"
else
    echo "FAIL: the installed ESP's loader.conf: $loader" >&2; fail=1
fi

(( fail == 0 )) || { echo "FAIL: T-INST-01" >&2; exit 1; }

# Part 2: start the installed disk on its own, the stick taken out (the
# Windows disk stays in). Fresh firmware variables, so the firmware finds
# systemd-boot by the fallback path EFI/BOOT/BOOTX64.EFI, as on a laptop
# whose boot entries were never written. The root must come from the
# installed slot 1, and the seed must be on the installed alp-state.
probe2='-/usr/bin/sh -c "lsblk -rnso PARTLABEL /dev/mapper/root | tr -s [:space:] , | sed s/^/ROOT-FROM=/; echo"
-/usr/bin/sh -c "test -s /var/lib/alp/firstboot-seed.toml && echo SEED=yes || echo SEED=no"
-/usr/bin/sh -c "findmnt -no SOURCE,TARGET /var/lib/alp | sed s/^/STATE-MNT=/; echo"
-/usr/bin/sh -c "systemctl is-active alp-live-state.service | sed s/^/STATE-UNIT=/"
-/usr/bin/sh -c "journalctl -b -u alp-live-state.service -o cat --no-pager | tail -5 | sed s/^/STATE-LOG=/"
-/usr/bin/sh -c "ls -la /var/lib/alp | sed s/^/STATE-LS=/"'
args2="-drive if=none,id=win,format=raw,file=$windows -device virtio-blk-pci,drive=win,serial=ALPWINDOWS01"
status=0
out=$(ALP_RAM=${ALP_RAM:-2G} ALP_PROBE="$probe2" ALP_PROBE_DELAY=5 ALP_QEMU_ARGS="$args2" ALP_OVERLAY_SIZE=64G \
    "$here/run-vm.sh" "$target" 600) || status=$?
echo "$out"
(( status == 0 )) || { echo "FAIL: T-INST-01 (the installed disk did not start)" >&2; exit 1; }
expect "the installed disk starts on its own" 'Reached alp-healthy.target'
expect "the root is the installed slot 1" '^ROOT-FROM=.*alp-os_[0-9.]+'
expect "the first-boot seed is on the installed alp-state" '^SEED=yes'
same "Windows disk" "$windows" "$windows_before"

(( fail == 0 )) || { echo "FAIL: T-INST-01" >&2; exit 1; }
echo "PASS: T-INST-01"
