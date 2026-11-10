#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Shared boot for T-BOOT-01 and T-BUDGET-01: one boot (4 GiB guest, probe 30 s
# after boot, as T-BUDGET-01 needs) gives the output both scenarios check, so
# `just test-vm` boots the healthy image once instead of twice. Both scenarios
# still run their own checks (ALP_SHARED_OUT); they also work on their own.
#
# Usage: tests/vm/t-core.sh <disk-image>
set -euo pipefail
here=$(dirname "$(realpath "$0")")
image=${1:?usage: t-core.sh <disk-image>}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

probe="$("$here/t-boot-01.sh" "$image" --print-probe)
$("$here/t-budget-01.sh" "$image" --print-probe)"
status=0
out=$(ALP_RAM=4G ALP_PROBE_DELAY=30 ALP_PROBE="$probe" "$here/run-vm.sh" "$image" 240) || status=$?
echo "$out"
(( status == 0 )) || { echo "FAIL: T-CORE shared boot" >&2; exit 1; }
printf '%s\n' "$out" >"$work/out.txt"
export ALP_SHARED_OUT="$work/out.txt"
"$here/t-boot-01.sh" "$image"
"$here/t-budget-01.sh" "$image"
