#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# T-LIVE-01: ALP-OS Live hardware report (ADR 0006, ADR 0007).
# Boots the image with a second virtual disk that stands in for the
# computer's internal disk, filled with random data. Passes when:
#   - the hardware report is produced and lists both disks with their roles,
#   - the report file is saved on the stick's ALP-REPORT partition,
#   - the internal disk is read-only in the kernel and a write attempt fails,
#   - the internal disk is bit-for-bit unchanged after the run,
#   - the kiosk (alp-kiosk.service, cage + Chromium on tty1) is running; the text
#     screen alp-live-screen.service is only the fallback and is NOT expected,
#   - the wired network gets an address by DHCP and the firewall is on.
#
# Usage: tests/vm/t-live-01.sh <disk-image>
set -euo pipefail
here=$(dirname "$(realpath "$0")")
# shellcheck disable=SC1091
. "$here/../lib/qa.sh"
image=${1:?usage: t-live-01.sh <disk-image>}
[[ -f $image ]] || { echo "FAIL: image not found: $image" >&2; exit 2; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
internal="$work/internal.raw"
head -c $((64 * 1024 * 1024)) /dev/urandom >"$internal"
before=$(sha256sum "$internal" | cut -d' ' -f1)

# shellcheck disable=SC2016  # the $(...) must reach the guest literally
probe='/usr/sbin/blockdev --getro /dev/vdb
-/usr/bin/dd if=/dev/zero of=/dev/vdb bs=512 count=1 oflag=direct
-/usr/bin/systemctl is-active alp-live-protect.service alp-live-report.service nftables.service systemd-networkd.service iwd.service
-/usr/bin/sh -c "echo KIOSK=$(systemctl is-active alp-kiosk.service) SCREEN=$(systemctl is-active alp-live-screen.service) KIOSKTTY=$(systemctl show -p TTYPath --value alp-kiosk.service)"
/usr/bin/cat /run/alp/hwreport.txt
/usr/bin/mount -o ro /dev/disk/by-partlabel/ALP-REPORT /mnt
/usr/bin/ls /mnt
/usr/sbin/nft list chain inet alp input
/usr/bin/sh -c "ls /usr/lib/firmware/iwlwifi-*.ucode* | head -1"
-/usr/bin/sh -c "echo PROTECTRUNS=$(systemctl list-units --all --no-legend --plain alp-live-protect-disk@* | wc -l) PROTECTFAILED=$(systemctl list-units --failed --no-legend --plain alp-live-protect-disk@* | wc -l) PROTECTERR=$(journalctl -b --no-pager | grep -c could.not.make)"'

probe+=$'\n'"$(qa_http_probe NETSTATE GET /api/v1/live/state)"

status=0
out=$(ALP_PROBE="$probe" ALP_PROBE_DELAY=10 \
    ALP_QEMU_ARGS="-drive if=virtio,format=raw,file=$internal -nic user,model=virtio-net-pci" \
    "$here/run-vm.sh" "$image" 240) || status=$?
echo "$out"
(( status == 0 )) || { echo "FAIL: T-LIVE-01 (VM run failed)" >&2; exit 1; }

fail=0
expect() {
    if grep -qE -- "$2" <<<"$out"; then echo "ok:   $1"; else echo "FAIL: $1" >&2; fail=1; fi
}
expect "internal disk is read-only" '^1\s*$'
expect "write to the internal disk is refused" 'Operation not permitted|Read-only'
expect "report services ran" '^active'
if grep -qE '^(inactive|failed|activating)\s*$' <<<"$out"; then echo "FAIL: a Live service is not running" >&2; fail=1; else echo "ok:   all Live services running"; fi
# Kiosk on tty1 (ADR 0010). Older images without the unit: say so, do not fail.
if grep -qE 'KIOSK=(active|activating)' <<<"$out"; then
    echo "ok:   kiosk is running (tty1)"
    expect "kiosk owns tty1" 'KIOSKTTY=/dev/tty1'
    if grep -qE 'SCREEN=active' <<<"$out"; then echo "FAIL: text screen runs next to the kiosk" >&2; fail=1; else echo "ok:   text screen is not running (kiosk is up)"; fi
elif grep -qE 'KIOSKTTY=/dev/tty1' <<<"$out"; then
    echo "FAIL: kiosk did not start (SCREEN fallback: $(grep -o 'SCREEN=[a-z]*' <<<"$out" | head -1)); graphics failed?" >&2; fail=1
else
    echo "note: no kiosk unit in this image; kiosk check skipped"
fi
expect "report has a verdict" 'This computer (can|cannot) run ALPArchive'
expect "stick is recognised" 'vda .*this stick \(ALP-OS\)'
expect "internal disk is listed as read-only" 'vdb .*internal, read-only'
expect "home network has an address (DHCP)" 'Home network: connected on .*address 10\.0\.2\.15|"address": ?"10\.0\.2\.15"'
expect "firewall drops unrequested traffic" 'policy drop'
expect "Wi-Fi firmware is in the image" '/usr/lib/firmware/iwlwifi-'
# The udev rule starts alp-live-protect-disk@ for each disk; none may fail
# (0.2.4: the rule's own run failed on every disk).
expect "the disk protection runs for each disk without errors" 'PROTECTRUNS=[1-9][0-9]* PROTECTFAILED=0 PROTECTERR=0'
expect "report saved on the stick" 'hw-.*-[0-9]{8}-[0-9]{4}\.txt'

after=$(sha256sum "$internal" | cut -d' ' -f1)
if [[ $before == "$after" ]]; then echo "ok:   internal disk unchanged"; else echo "FAIL: internal disk changed" >&2; fail=1; fi

(( fail == 0 )) || { echo "FAIL: T-LIVE-01" >&2; exit 1; }
echo "PASS: T-LIVE-01"
