#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# T-UI-01: the kiosk shows the welcome screen (docs/plans/live-ux.md).
# Boots the image with a virtual display, waits until the picture is not blank,
# and saves it as a PNG under out/qa/ (QEMU screendump through the monitor).
# Passes when the picture has content (several colours, not one flat colour).
# ALP_UI_FORCE=1 runs it on an image without a kiosk (checks the harness itself).
# SKIPs while the repo has no kiosk build config.
#
# Usage: tests/vm/t-ui-01.sh <disk-image>
set -euo pipefail
here=$(dirname "$(realpath "$0")")
# shellcheck disable=SC1091
. "$here/../lib/qa.sh"
image=${1:?usage: t-ui-01.sh <disk-image>}
[[ -f $image ]] || { echo "FAIL: image not found: $image" >&2; exit 2; }

# Is there a kiosk? The image has no .manifest file any more, so decide from the
# repo's kiosk build config (the image is built from it). A wrong guess shows up
# as a blank screen: use ALP_UI_FORCE=1 to run regardless.
repo=$(realpath "$here/../..")
has_kiosk() { [[ -f $repo/image/mkosi.conf.d/a-kiosk.conf && -f $repo/image/units/alp-kiosk.service ]]; }
[[ ${ALP_UI_FORCE:-0} == 1 ]] || has_kiosk || qa_skip "T-UI-01: no kiosk in this image yet"

outdir=${ALP_QA_OUT:-$(dirname "$(realpath "$image")")}
mkdir -p "$outdir"
png="$outdir/t-ui-01-welcome.png"
ppm=$(mktemp --suffix=.ppm)
trap 'rm -f "$ppm"' EXIT
export ppm png here
rm -f "$png"

shoot() {
    command -v socat >/dev/null || { echo "FAIL: socat is not installed (needed for the QEMU screendump)" >&2; return 1; }
    # 300 s: on hosted runners the first start (setup) reaches the kiosk after
    # ~130 s (CI run 39270104858).
    local deadline=$((SECONDS + ${ALP_UI_WAIT:-300})) rc=1 stats
    while (( SECONDS < deadline )); do
        printf 'screendump %s\n' "$ppm" | socat - "UNIX-CONNECT:$ALP_MONITOR_SOCK" >/dev/null 2>&1 || true
        if [[ -s $ppm ]] && stats=$(python3 -I "$here/../lib/ppm2png.py" "$ppm" "$png"); then
            echo "ok:   welcome screen is not blank ($stats), saved $png"; return 0
        fi
        sleep 3
    done
    echo "FAIL: screen stayed blank for ${ALP_UI_WAIT:-300}s ${stats:-}" >&2
    [[ -s $ppm ]] && python3 -I "$here/../lib/ppm2png.py" "$ppm" "$png" >&2 || true
    return "$rc"
}
export -f shoot

# Extra guest RAM for a browser; the display is a plain VGA (screendump works on it).
status=0
ALP_RAM=${ALP_RAM:-2G} ALP_HOST_HOOK=shoot "$here/run-vm.sh" "$image" 420 || status=$?
(( status == 0 )) || { echo "FAIL: T-UI-01" >&2; exit 1; }
echo "PASS: T-UI-01"
