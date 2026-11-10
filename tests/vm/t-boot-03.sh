#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# T-BOOT-03: a tampered root image fails dm-verity and does not boot.
# Two cases, each on a throwaway copy of the image:
#   data: change one byte in every MiB of the root partition, skipping the
#         first MiB (the EROFS superblock has its own checksum, which would
#         make the boot fail even without verity);
#   hash: change one byte in every 64 KiB of the verity hash partition,
#         skipping its first 4 KiB.
# Pass = the kernel reports a corrupted verity data/metadata block and
# alp-healthy.target is never reached.
#
# Usage: tests/vm/t-boot-03.sh <disk-image> [timeout-seconds]
set -euo pipefail
here=$(dirname "$(realpath "$0")")
image=${1:?usage: t-boot-03.sh <disk-image> [timeout-seconds]}
timeout_s=${2:-120}

[[ -f $image ]] || { echo "FAIL: image not found: $image" >&2; exit 2; }

# Discoverable Partitions type GUIDs, as reported by sfdisk for our image.
root_type=4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709
verity_type=2C7357ED-EBD2-46D9-AEC1-23D437EC2BF5

# Work next to the image (on disk), not in /tmp (a RAM-backed tmpfs).
work=$(mktemp -d -p "$(dirname "$(realpath "$image")")" t-boot-03.XXXXXX)
trap 'rm -rf "$work"' EXIT

tamper() { # file, partition type GUID, skip bytes, stride bytes
    python3 -I - "$@" <<'PY'
import json, subprocess, sys
path, ptype, skip, stride = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4])
table = json.loads(subprocess.run(["sfdisk", "--json", path], check=True,
                                  capture_output=True, text=True).stdout)
parts = [p for p in table["partitiontable"]["partitions"] if p["type"].upper() == ptype]
if len(parts) != 1:
    sys.exit(f"expected exactly one partition of type {ptype}, found {len(parts)}")
start, size = parts[0]["start"] * 512, parts[0]["size"] * 512
count = 0
with open(path, "r+b") as f:
    for off in range(start + skip, start + size, stride):
        f.seek(off); b = f.read(1); f.seek(off); f.write(bytes([b[0] ^ 0xFF])); count += 1
print(f"  changed {count} bytes in partition {ptype}")
PY
}

expect='device-mapper: verity: [^ ]+: (data|metadata) block [0-9]+ is corrupted'
status=0
for case in data hash; do
    copy="$work/$case.raw"
    cp --sparse=always "$image" "$copy"
    echo "Case '$case':"
    if [[ $case == data ]]; then
        tamper "$copy" "$root_type" $((1024 * 1024)) $((1024 * 1024))
    else
        tamper "$copy" "$verity_type" 4096 $((64 * 1024))
    fi
    if ALP_EXPECT_FAIL=$expect "$here/run-vm.sh" "$copy" "$timeout_s"; then
        echo "  ok: case '$case'"
    else
        echo "  FAIL: case '$case'" >&2; status=1
    fi
    rm -f "$copy"
done
(( status == 0 )) && echo "PASS: T-BOOT-03"
exit "$status"
