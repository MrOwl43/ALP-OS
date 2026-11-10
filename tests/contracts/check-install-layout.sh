#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Contract check for the installed-system disk layout (image/install-layout,
# docs/contracts/disk-layout.md). systemd-repart lays the definitions out on
# throwaway sparse files (never a real disk) and the result is compared with
# the contract: partition order, types, labels and sizes.
# Usage: tests/contracts/check-install-layout.sh
set -euo pipefail
repo=$(cd "$(dirname "$0")/../.." && pwd)
layout=$repo/image/install-layout
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

run_repart() { # image, size, definition dirs...
    local img=$1 size=$2; shift 2
    local defs=()
    for d in "$@"; do defs+=(--definitions="$d"); done
    systemd-repart --dry-run=no --empty=create --size="$size" --offline=yes \
        --no-pager --json=off "${defs[@]}" "$img" >"$work/repart.log" 2>&1
}

check() { # image, expect-data (yes|no)
    python3 -I - "$1" "$2" <<'PY'
import json, subprocess, sys
img, want_data = sys.argv[1], sys.argv[2] == "yes"
parts = json.loads(subprocess.run(["sfdisk", "--json", img], check=True,
                                  capture_output=True, text=True).stdout)["partitiontable"]["partitions"]
G, M = 1024**3, 1024**2
ESP, ROOT, VERITY, GENERIC = ("C12A7328-F81F-11D2-BA4B-00A0C93EC93B",
    "4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709", "2C7357ED-EBD2-46D9-AEC1-23D437EC2BF5",
    "0FC63DAF-8483-4772-8E79-3D69D8477DE4")
want = [(ESP, "esp", 1*G), (ROOT, "_empty", 8*G), (VERITY, "_empty", 128*M),
        (ROOT, "_empty", 8*G), (VERITY, "_empty", 128*M), (GENERIC, "alp-state", 2*G)]
if want_data:
    want.append((GENERIC, "alp-data", None))
errors = []
if len(parts) != len(want):
    errors.append(f"expected {len(want)} partitions, found {len(parts)}")
for i, (p, (t, label, size)) in enumerate(zip(parts, want), 1):
    got = (p["type"].upper(), p.get("name", ""), p["size"] * 512)
    if got[0] != t: errors.append(f"partition {i}: type {got[0]}, want {t}")
    if got[1] != label: errors.append(f"partition {i}: label {got[1]!r}, want {label!r}")
    if size is not None and got[2] != size: errors.append(f"partition {i}: size {got[2]}, want {size}")
for e in errors: print("  FAIL:", e)
sys.exit(1 if errors else 0)
PY
}

status=0
echo "Single disk (64 GB): system + data"
run_repart "$work/single.raw" 64G "$layout/system" "$layout/single-disk-data" \
    || { cat "$work/repart.log"; exit 1; }
check "$work/single.raw" yes && echo "  ok" || status=1

echo "Dedicated system disk (32 GB): system only"
run_repart "$work/system.raw" 32G "$layout/system" || { cat "$work/repart.log"; exit 1; }
check "$work/system.raw" no && echo "  ok" || status=1

echo "Dedicated data disk (100 GB): one alp-data partition"
run_repart "$work/data.raw" 100G "$layout/data-disk" || { cat "$work/repart.log"; exit 1; }
python3 -I - "$work/data.raw" <<'PY' && echo "  ok" || status=1
import json, subprocess, sys
parts = json.loads(subprocess.run(["sfdisk", "--json", sys.argv[1]], check=True,
                                  capture_output=True, text=True).stdout)["partitiontable"]["partitions"]
ok = (len(parts) == 1 and parts[0]["type"].upper() == "0FC63DAF-8483-4772-8E79-3D69D8477DE4"
      and parts[0].get("name") == "alp-data")
print("  FAIL: expected one linux-generic alp-data partition" if not ok else "", end="")
sys.exit(0 if ok else 1)
PY

echo "Too small (16 GB): repart must refuse"
if run_repart "$work/small.raw" 16G "$layout/system"; then
    echo "  FAIL: layout fitted on a 16 GB disk" >&2; status=1
elif grep -q "Can't fit requested partitions" "$work/repart.log"; then
    echo "  ok (refused: does not fit)"
else
    echo "  FAIL: repart failed for another reason:" >&2; cat "$work/repart.log" >&2; status=1
fi
(( status == 0 )) && echo "PASS: install layout matches the contract"
exit "$status"
