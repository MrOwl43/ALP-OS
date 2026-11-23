#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Commit messages in this public repository name no author but the project
# and no tool: no "Co-Authored-By", no session links, no assistant or model
# names. Author and commit times are in UTC (+0000): a local time zone
# would hint where the work was done. Checks every commit made since
# 2026-11-11 that HEAD reaches (the 14 commits of the day before, when the
# repository was made public, are known and listed in docs/PROGRESS.md).
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
    # A merge made with the Merge button on GitHub: committed by GitHub
    # itself, with GitHub's own message (it names the branch). Its parents
    # are checked like every other commit.
    if [[ $(git log -1 --format='%P' "$hash" | wc -w) -ge 2 && $(git log -1 --format='%cn <%ce>' "$hash") == "GitHub <noreply@github.com>" ]] \
        && git log -1 --format=%s "$hash" | grep -Eq '^Merge pull request #[0-9]+ from '; then
        if git log -1 --format=%B "$hash" | grep -Eiq 'co-authored-by:|anthropic|session_[0-9A-Za-z]{8}|generated with|requested by'; then
            echo "FAIL: merge ${hash:0:7} names a tool, a session or a co-author" >&2
            bad=1
        fi
        if [[ $(git log -1 --format='%ad %cd' --date=format:%z "$hash") != "+0000 +0000" ]]; then
            echo "FAIL: merge ${hash:0:7} carries a local time zone; use Actions > Fast-forward main instead of the Merge button" >&2
            bad=1
        fi
        continue
    fi
    if git log -1 --format='%an <%ae>%n%cn <%ce>%n%B' "$hash" \
        | grep -Eiq 'co-authored-by:|claude|anthropic|session_[0-9A-Za-z]{8}|generated with|requested by'; then
        echo "FAIL: commit ${hash:0:7} names a tool, a session or a co-author: $(git log -1 --format=%s "$hash")" >&2
        bad=1
    fi
    if [[ $(git log -1 --format='%an <%ae> %cn <%ce>' "$hash") != "ALP-OS <noreply@alparchive.local> ALP-OS <noreply@alparchive.local>" ]]; then
        echo "FAIL: commit ${hash:0:7} is not authored and committed as ALP-OS <noreply@alparchive.local>" >&2
        bad=1
    fi
    if [[ $(git log -1 --format='%ad %cd' --date=format:%z "$hash") != "+0000 +0000" ]]; then
        echo "FAIL: commit ${hash:0:7} is not dated in UTC (+0000); commit with TZ=UTC" >&2
        bad=1
    fi
done < <(git log --since=2026-11-11T00:00:00Z --format=%H "$rev")
(( bad == 0 )) || exit 1
echo "ok:   commit messages, authors and time zones are clean"
