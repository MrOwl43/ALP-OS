#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Machine-wide resource limits for the dev VM, shared by every worktree
# (NEXT_TASK: at most one image build and one test VM at the same time).
#
# Usage: tests/lib/locks.sh <resource> <command...>
#   resource: image-build (1 slot), vm (1 slot), cargo (2 slots)
# Waits for a free slot, runs the command while holding it, and releases the
# slot when the command ends (also on crash: the kernel drops flock locks).
# Override the lock directory with ALP_LOCK_DIR (tests).
set -euo pipefail

resource=${1:?usage: locks.sh <image-build|vm|cargo> <command...>}
shift
(( $# > 0 )) || { echo "locks.sh: no command given" >&2; exit 2; }
case $resource in
    image-build) slots=1 ;;
    vm) slots=1 ;;  # one VM at a time on the small dev VM (coordinator, 2026-10-19)
    cargo) slots=2 ;;
    *) echo "locks.sh: unknown resource '$resource'" >&2; exit 2 ;;
esac

dir=${ALP_LOCK_DIR:-${XDG_RUNTIME_DIR:-/tmp}/alp-os-locks}
mkdir -p "$dir"
waited=0
while :; do
    for ((i = 1; i <= slots; i++)); do
        exec 9>"$dir/$resource.$i.lock"
        if flock -n 9; then
            (( waited == 0 )) || echo "locks.sh: got $resource slot $i after ${waited}s" >&2
            "$@"
            exit $?
        fi
        exec 9>&-
    done
    (( waited % 60 == 0 )) && echo "locks.sh: waiting for a free $resource slot ($slots in use)" >&2
    sleep 2
    waited=$((waited + 2))
done
