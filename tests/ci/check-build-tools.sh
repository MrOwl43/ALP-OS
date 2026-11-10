#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Verify that the installed build tools match tests/ci/build-tools.lock.
# Usage: tests/ci/check-build-tools.sh tests/ci/build-tools.lock
set -euo pipefail
lock=${1:?usage: check-build-tools.sh <lockfile>}
status=0
while IFS='=' read -r pkg want; do
    [[ -z $pkg || $pkg == \#* ]] && continue
    # ALP_LOCK_SKIP=<ERE>: packages not checked here (the dev VM no longer runs
    # VM tests; QEMU and friends must match only on CI).
    if [[ -n ${ALP_LOCK_SKIP:-} && $pkg =~ $ALP_LOCK_SKIP ]]; then continue; fi
    have=$(dpkg-query -W -f='${Version}' "$pkg" 2>/dev/null || echo "not installed")
    if [[ $have != "$want" ]]; then
        echo "MISMATCH: $pkg is $have, lock says $want" >&2
        status=1
    fi
done <"$lock"
(( status == 0 )) && echo "ok: build tools match $lock"
exit "$status"
