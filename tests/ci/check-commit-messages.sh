#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Commit messages in this public repository name no author but the project
# and no tool: no "Co-Authored-By", no session links, no assistant or model
# names. Checks every commit made since 2026-11-11 that HEAD reaches (the
# 14 commits of the day before, when the repository was made public, are
# known and listed in docs/PROGRESS.md).
#
# Usage: tests/ci/check-commit-messages.sh [revision]   (default HEAD)
set -euo pipefail
rev=${1:-HEAD}
if [[ $(git rev-parse --is-shallow-repository) == true ]]; then
    echo "check-commit-messages: the checkout is shallow; fetch the full history first" >&2
    exit 2
fi
bad=0
while read -r hash; do
    if git log -1 --format='%an <%ae>%n%cn <%ce>%n%B' "$hash" \
        | grep -Eiq 'co-authored-by:|claude|anthropic|session_[0-9A-Za-z]{8}|generated with|requested by'; then
        echo "FAIL: commit ${hash:0:7} names a tool, a session or a co-author: $(git log -1 --format=%s "$hash")" >&2
        bad=1
    fi
    if [[ $(git log -1 --format='%an <%ae> %cn <%ce>' "$hash") != "ALP-OS <noreply@alparchive.local> ALP-OS <noreply@alparchive.local>" ]]; then
        echo "FAIL: commit ${hash:0:7} is not authored and committed as ALP-OS <noreply@alparchive.local>" >&2
        bad=1
    fi
done < <(git log --since=2026-11-11T00:00:00Z --format=%H "$rev")
(( bad == 0 )) || exit 1
echo "ok:   commit messages and authors are clean"
