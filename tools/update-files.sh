#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Make the update files of one version from a built stick image (ADR 0026):
#
#   alp-os_<version>_<root partition UUID>.root.raw.xz
#   alp-os_<version>_<verity partition UUID>.verity.raw.xz
#   alp-os_<version>.efi
#   SHA256SUMS            (of the three files; signed by the caller)
#
# They are cut out of the image as it is: the same root, hash partition and
# start file (UKI) a freshly written stick has. systemd-sysupdate on a stick
# reads the names (image/extra/usr/lib/sysupdate.d): the version, and the
# partition UUIDs the UKI finds its root by.
#
# Usage: tools/update-files.sh <image> <version> <out-dir>
# Needs sfdisk, python3, mtools (mcopy), xz.
set -euo pipefail
image=${1:?usage: update-files.sh <image> <version> <out-dir>}
version=${2:?usage: update-files.sh <image> <version> <out-dir>}
out=${3:?usage: update-files.sh <image> <version> <out-dir>}
[[ -f $image ]] || { echo "update-files: image not found: $image" >&2; exit 2; }
[[ $version =~ ^[0-9]+(\.[0-9]+)*$ ]] || { echo "update-files: '$version' is not a version" >&2; exit 2; }
mkdir -p "$out"

# "<type> <start sector> <sectors> <uuid>" of the ESP, the root and its
# verity partition (the first of each type; the image has one of each).
parts=$(sfdisk --json "$image" | python3 -I -c '
import json, sys
types = {
    "C12A7328-F81F-11D2-BA4B-00A0C93EC93B": "esp",
    "4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709": "root",
    "2C7357ED-EBD2-46D9-AEC1-23D437EC2BF5": "verity",
}
seen = set()
for p in json.load(sys.stdin)["partitiontable"]["partitions"]:
    kind = types.get(p["type"].upper())
    if kind and kind not in seen:
        seen.add(kind)
        print(kind, p["start"], p["size"], p["uuid"].lower())
if seen != set(types.values()):
    sys.exit("update-files: the image has no ESP, root or verity partition")
')
cut_out() { # kind, file name (without .xz)
    local start size
    read -r _ start size _ < <(grep "^$1 " <<<"$parts")
    dd if="$image" of="$out/$2" bs=1M iflag=skip_bytes,count_bytes skip=$((start * 512)) count=$((size * 512)) status=none
    xz -T0 -1 -f "$out/$2"
}
uuid_of() { awk -v k="$1" '$1 == k {print $4}' <<<"$parts"; }

root="alp-os_${version}_$(uuid_of root).root.raw"
verity="alp-os_${version}_$(uuid_of verity).verity.raw"
uki="alp-os_${version}.efi"
cut_out root "$root"
cut_out verity "$verity"

# The one UKI on the ESP.
read -r _ esp_start _ _ < <(grep '^esp ' <<<"$parts")
esp="$image@@$((esp_start * 512))"
mapfile -t ukis < <(MTOOLS_SKIP_CHECK=1 mdir -b -i "$esp" '::/EFI/Linux' | tr -d '\r' | grep -i '\.efi$')
(( ${#ukis[@]} == 1 )) || { echo "update-files: expected one UKI on the ESP, found ${#ukis[@]}" >&2; exit 1; }
MTOOLS_SKIP_CHECK=1 mcopy -n -i "$esp" "${ukis[0]}" "$out/$uki"

(cd "$out" && sha256sum "$root.xz" "$verity.xz" "$uki" > SHA256SUMS && cat SHA256SUMS)
