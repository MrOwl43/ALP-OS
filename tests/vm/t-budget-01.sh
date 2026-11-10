#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# T-BUDGET-01: record image sizes and idle memory, and check them against
# their budgets: idle memory within 2 GB on a 4 GB machine (spec Section 3);
# system image (root + verity + UKI, what one update ships) within 4 GB
# (owner decision 2026-10-18 for Live, ADR 0009; was 2 GB in ADR 0005).
# "4 GB" means 4,000,000,000 bytes = 3814 MiB.
# Results go to <image dir>/budget.json.
#
# Usage: tests/vm/t-budget-01.sh <disk-image> [--print-probe]
# Env:   ALP_SHARED_OUT=<file>  use this probe output instead of booting
# Env:   ALP_BUDGET_IDLE_MIB (default 1907), ALP_BUDGET_SYSTEM_MIB (default 3814)
set -euo pipefail
here=$(dirname "$(realpath "$0")")
image=${1:?usage: t-budget-01.sh <disk-image>}
idle_budget=${ALP_BUDGET_IDLE_MIB:-1907}
system_budget=${ALP_BUDGET_SYSTEM_MIB:-3814}
dir=$(dirname "$(realpath "$image")")
uki="$dir/$(basename "$image" .raw).efi"

[[ -f $image ]] || { echo "FAIL: image not found: $image" >&2; exit 2; }
[[ -f $uki ]] || { echo "FAIL: UKI not found next to the image: $uki" >&2; exit 2; }

status=0
mem_probe='/bin/grep -E ^(MemTotal|MemAvailable): /proc/meminfo'
if [[ ${2:-} == --print-probe ]]; then printf '%s\n' "$mem_probe"; exit 0; fi
if [[ -n ${ALP_SHARED_OUT:-} ]]; then
    # Shared boot (tests/vm/t-core.sh): 4G guest, probe run 30 s after boot.
    out=$(<"$ALP_SHARED_OUT")
else
    out=$(ALP_RAM=4G ALP_PROBE_DELAY=30 ALP_PROBE=$mem_probe \
        "$here/run-vm.sh" "$image" 240) || status=$?
fi
echo "$out"
(( status == 0 )) || { echo "FAIL: T-BUDGET-01 (VM run failed)" >&2; exit 1; }

ALP_PROBE_OUT=$out python3 -I - "$image" "$uki" "$idle_budget" "$system_budget" "$dir/budget.json" <<'PY' || status=1
import json, os, re, subprocess, sys
image, uki, idle_budget, system_budget, dest = (
    sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4]), sys.argv[5])
out = os.environ["ALP_PROBE_OUT"]

def kib(key):
    # Not anchored: on the serial console kernel output can share the line.
    m = re.search(key + r":\s+([0-9]+) kB", out)
    if not m:
        sys.exit(f"FAIL: probe output has no {key}")
    return int(m.group(1))

mib = lambda n: round(n / 1024 / 1024, 1)
table = json.loads(subprocess.run(["sfdisk", "--json", image], check=True,
                                  capture_output=True, text=True).stdout)
parts = {p["type"].upper(): p["size"] * 512 for p in table["partitiontable"]["partitions"]}
used_mib = (kib("MemTotal") - kib("MemAvailable")) // 1024
result = {
    "image_file_mib": mib(os.path.getsize(image)),
    "uki_mib": mib(os.path.getsize(uki)),
    "root_partition_mib": mib(parts["4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709"]),
    "root_verity_partition_mib": mib(parts["2C7357ED-EBD2-46D9-AEC1-23D437EC2BF5"]),
    "idle_memory_used_mib": used_mib,
    "idle_memory_budget_mib": idle_budget,
    "guest_ram": "4G",
}
result["system_image_mib"] = round(result["uki_mib"] + result["root_partition_mib"]
                                   + result["root_verity_partition_mib"], 1)
result["system_image_budget_mib"] = system_budget
with open(dest, "w") as f:
    json.dump(result, f, indent=2); f.write("\n")
for k, v in result.items():
    print(f"  {k}: {v}")
print(f"  written to {dest}")
failed = []
if used_mib > idle_budget:
    failed.append("idle memory over budget")
if result["system_image_mib"] > system_budget:
    failed.append("system image over budget")
if failed:
    sys.exit("FAIL: " + "; ".join(failed))
PY
(( status == 0 )) && echo "PASS: T-BUDGET-01"
exit "$status"
