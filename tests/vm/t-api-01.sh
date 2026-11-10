#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# T-API-01: the Live console API (docs/plans/live-alpha-api.md, docs/contracts/api-access.md).
# Boots the image with the home network on QEMU user networking and passes when
#   - the API on 127.0.0.1:8411 answers GET /api/v1/live/state with JSON,
#   - a state-changing request with a foreign Origin is refused (403),
#   - the same API is NOT reachable from the LAN side (the guest's own network
#     address; QEMU forwards a host port to it), nor is Immich's port 2283.
# SKIPs while the API is missing (no listener on 8411).
#
# Usage: tests/vm/t-api-01.sh <disk-image>
set -euo pipefail
here=$(dirname "$(realpath "$0")")
# shellcheck disable=SC1091
. "$here/../lib/qa.sh"
image=${1:?usage: t-api-01.sh <disk-image>}
[[ -f $image ]] || { echo "FAIL: image not found: $image" >&2; exit 2; }

p_api=$(qa_free_port)
p_immich=$(qa_free_port)
nic="user,model=virtio-net-pci,hostfwd=tcp:127.0.0.1:$p_api-:8411,hostfwd=tcp:127.0.0.1:$p_immich-:2283"

probe="-/bin/bash -c 'exec 3<>/dev/tcp/127.0.0.1/8411 && echo API-LISTENING || echo API-NOT-LISTENING'
$(qa_http_probe STATE GET /api/v1/live/state)
$(qa_http_probe BADORIGIN POST /api/v1/live/wifi/connect http://evil.example '{}')
$(qa_http_probe NOORIGIN POST /api/v1/live/wifi/connect '' '{}')"

status=0
out=$(ALP_RAM=${ALP_RAM:-2G} ALP_PROBE="$probe" ALP_PROBE_DELAY=15 ALP_QEMU_ARGS="-nic $nic" \
    "$here/run-vm.sh" "$image" 300) || status=$?
echo "$out"
(( status == 0 )) || { echo "FAIL: T-API-01 (VM run failed)" >&2; exit 1; }

grep -q 'API-LISTENING' <<<"$out" || qa_skip "T-API-01: nothing listens on 127.0.0.1:8411 yet (live API not built into this image)"

fail=0
expect() { if grep -qE -- "$2" <<<"$3"; then echo "ok:   $1"; else echo "FAIL: $1" >&2; fail=1; fi; }
block() { sed -n "/$1-BEGIN/,/$1-END/p" <<<"$out"; }

state=$(block STATE)
expect "GET live/state answers 200" '^HTTP/1\.[01] 200' "$state"
if jq -e . >/dev/null 2>&1 <<<"$(sed -n '/^{/,$p' <<<"$state" | sed '/STATE-END/d')"; then echo "ok:   live/state body is JSON"; else echo "FAIL: live/state body is not JSON" >&2; fail=1; fi
expect "foreign Origin is refused (403)" '^HTTP/1\.[01] 403' "$(block BADORIGIN)"
expect "missing Origin is refused (403)" '^HTTP/1\.[01] 403' "$(block NOORIGIN)"

# LAN side: the host port reaches the guest's network address, not its loopback.
for pair in "console API:$p_api" "Immich:$p_immich"; do
    name=${pair%%:*}; port=${pair##*:}
    got=$(curl -s -m 5 -o /dev/null -w '%{http_code}' "http://127.0.0.1:$port/" || true)
    if [[ $got == 000 || $got == 000000 ]]; then echo "ok:   $name not reachable from the LAN side"; else echo "FAIL: $name answered $got on the LAN side" >&2; fail=1; fi
done

(( fail == 0 )) || { echo "FAIL: T-API-01" >&2; exit 1; }
echo "PASS: T-API-01"
