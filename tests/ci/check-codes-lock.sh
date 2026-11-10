#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# errors/codes.lock is append-only: the copy on <base> must be an exact prefix
# of the working copy (codes are never removed, renumbered or reordered).
# Usage: tests/ci/check-codes-lock.sh <base-ref, e.g. origin/main>
set -euo pipefail
base=${1:?usage: check-codes-lock.sh <base-ref>}
lock=errors/codes.lock
if ! old=$(git show "$base:$lock" 2>/dev/null); then
    echo "ok: $lock does not exist on $base yet"
    exit 0
fi
new=$(cat "$lock")
if [[ $new != "$old"* ]]; then
    echo "FAIL: $lock is append-only, but lines on $base were changed or removed:" >&2
    diff <(printf '%s\n' "$old") <(printf '%s\n' "$new") >&2 || true
    exit 1
fi
echo "ok: $lock only appends to $base"
