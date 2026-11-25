#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# T-SCREEN-01 (part 1): the native screen (alp-screen, GTK 4 under cage) on
# the installed-profile stick, first start, nothing set up.
#   - alp-screen.service is active and did not restart (no crash loop);
#   - alpd says the first setup step is the place (setup.place "choose":
#     install on a disk or keep running from the stick, owner 2026-11-25);
#   - the lock screen facts are in the state (system.temperature_c key);
#   - the screen is not blank (QEMU screendump, saved as t-screen-01.png
#     next to the image or in ALP_QA_OUT).
# Driving the pages through the accessibility tree (AT-SPI) is part 2.
#
# Usage: tests/vm/t-screen-01.sh <disk-image>
set -euo pipefail
here=$(dirname "$(realpath "$0")")
# shellcheck disable=SC1091
. "$here/../lib/qa.sh"
image=${1:?usage: t-screen-01.sh <disk-image>}
[[ -f $image ]] || { echo "FAIL: image not found: $image" >&2; exit 2; }
command -v socat >/dev/null || { echo "FAIL: socat is not installed (needed for the QEMU screendump)" >&2; exit 2; }
outdir=${ALP_QA_OUT:-$(dirname "$(realpath "$image")")}
mkdir -p "$outdir"
export png="$outdir/t-screen-01.png" here

# shellcheck disable=SC2329  # runs as ALP_HOST_HOOK (exported)
shoot() {
    local deadline=$((SECONDS + ${ALP_UI_WAIT:-240})) stats ppm
    ppm=$(mktemp --suffix=.ppm)
    while (( SECONDS < deadline )); do
        printf 'screendump %s\n' "$ppm" | socat - "UNIX-CONNECT:$ALP_MONITOR_SOCK" >/dev/null 2>&1 || true
        if [[ -s $ppm ]] && stats=$(python3 -I "$here/../lib/ppm2png.py" "$ppm" "$png"); then
            echo "ok:   native screen is not blank ($stats), saved $png"; rm -f "$ppm"; return 0
        fi
        sleep 3
    done
    echo "FAIL: native screen stayed blank for ${ALP_UI_WAIT:-240}s ${stats:-}, last picture in $png" >&2
    [[ -s $ppm ]] && python3 -I "$here/../lib/ppm2png.py" "$ppm" "$png" >/dev/null 2>&1 || true
    rm -f "$ppm"
    return 1
}
export -f shoot

# The probe waits so the screen had time to start (and to crash, if it does).
probe="/bin/sleep 30
-/usr/bin/sh -c 'systemctl is-active alp-screen.service | sed s/^/SCREEN-ACTIVE=/'
-/usr/bin/sh -c 'systemctl show -p NRestarts --value alp-screen.service | sed s/^/SCREEN-RESTARTS=/'
-/usr/bin/sh -c 'curl -s -m 20 http://127.0.0.1:8411/api/v1/live/state | grep -o place.:.[a-z]* | sed s/^/PLACE=/'
-/usr/bin/sh -c 'curl -s -m 20 http://127.0.0.1:8411/api/v1/live/state | grep -q temperature_c && echo TEMP-KEY=yes || echo TEMP-KEY=no'
-/usr/bin/sh -c 'journalctl -b -u alp-screen.service -o cat --no-pager | tail -15 | sed s/^/SCREEN-LOG=/'
-/usr/bin/sh -c 'journalctl -b _UID=988 -o cat --no-pager | tail -25 | sed s/^/SCREEN-USER-LOG=/'
-/usr/bin/sh -c 'ps -eo pid,user,args | grep -E \"cage|alp-screen\" | grep -v grep | sed s/^/SCREEN-PS=/'
-/usr/bin/sh -c '{ ls -l /dev/dri; dmesg | grep -iE \"drm|bochs|fb0\" | tail -8; } | sed s/^/SCREEN-DRM=/'"

status=0
out=$(ALP_RAM=${ALP_RAM:-2G} ALP_PROBE="$probe" ALP_PROBE_DELAY=5 ALP_HOST_HOOK=shoot \
    "$here/run-vm.sh" "$image" 600 2>&1) || status=$?

fail=0
expect() {
    if grep -qE -- "$2" <<<"$out"; then echo "ok:   $1"; else echo "FAIL: $1" >&2; fail=1; fi
}
grep -E '^(ok|FAIL):' <<<"$out" || true
expect "the native screen runs" '^SCREEN-ACTIVE=active'
expect "the native screen did not restart" '^SCREEN-RESTARTS=0'
expect "the first setup step is the place" '^PLACE=place.:.choose'
expect "the lock screen facts are in the state" '^TEMP-KEY=yes'
(( status == 0 )) || fail=1
if (( fail )); then
    grep -E '^(SCREEN-|PLACE=|TEMP-KEY=|TIMING:)' <<<"$out" | tr -d '\r' || true
    echo "FAIL: T-SCREEN-01" >&2
    exit 1
fi
echo "PASS: T-SCREEN-01"
