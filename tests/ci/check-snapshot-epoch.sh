#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Check that SourceDateEpoch= in mkosi.conf is the same moment as Snapshot=,
# so a snapshot bump cannot forget the epoch (reproducible builds).
# Usage: tests/ci/check-snapshot-epoch.sh image/mkosi.conf
set -euo pipefail
conf=${1:?usage: check-snapshot-epoch.sh <mkosi.conf>}
snapshot=$(sed -n 's/^Snapshot=//p' "$conf")
epoch=$(sed -n 's/^SourceDateEpoch=//p' "$conf")
[[ $snapshot =~ ^([0-9]{4})([0-9]{2})([0-9]{2})T([0-9]{2})([0-9]{2})([0-9]{2})Z$ ]] \
    || { echo "FAIL: unexpected Snapshot= value: '$snapshot'" >&2; exit 1; }
m=("${BASH_REMATCH[@]}")
expected=$(date -u -d "${m[1]}-${m[2]}-${m[3]}T${m[4]}:${m[5]}:${m[6]}Z" +%s)
if [[ $epoch != "$expected" ]]; then
    echo "FAIL: SourceDateEpoch=$epoch but Snapshot=$snapshot means $expected" >&2
    exit 1
fi
echo "ok: Snapshot= and SourceDateEpoch= match ($snapshot = $epoch)"
