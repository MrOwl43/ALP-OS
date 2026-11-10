#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# T-UI-02: the second start (T28). Alpha 0.2.1 on the owner's laptop showed
# an empty kiosk page on the start after setup: alpd took the port but did
# not answer. This boots the same stick twice.
#
# Boot 1: password set and archive created through the API (as the setup
#   screens do), on an external disk (`-device usb-storage`).
# Boot 2 (overlay kept), for two cases: "tpm" (swtpm, state kept: the index
#   opens by itself, then the archive and Immich start), "plain" (ADR 0016
#   level 1: no password, the archive is the plain folder ALPArchive/files)
#   and "password" (no
#   TPM: the console asks for the password):
#   - alpd answers the page, a UI file and the state within 5 s each;
#   - the kiosk screen is not blank (QEMU screendump, saved as
#     t-ui-02-<case>.png next to the image or in ALP_QA_OUT).
# Output is short on purpose: "ok:"/"FAIL:" lines, and on a failure the
# start's waiting jobs, failed units and alpd/kiosk messages.
# SKIPs while the repo has no kiosk build config (like T-UI-01).
#
# Usage: tests/vm/t-ui-02.sh <disk-image>
set -euo pipefail
here=$(dirname "$(realpath "$0")")
# shellcheck disable=SC1091
. "$here/../lib/qa.sh"
image=${1:?usage: t-ui-02.sh <disk-image>}
[[ -f $image ]] || { echo "FAIL: image not found: $image" >&2; exit 2; }
repo=$(realpath "$here/../..")
[[ -f $repo/image/mkosi.conf.d/a-kiosk.conf && -f $repo/image/units/alp-kiosk.service ]] ||
    qa_skip "T-UI-02: no kiosk in this image yet"
command -v socat >/dev/null || { echo "FAIL: socat is not installed (needed for the QEMU screendump)" >&2; exit 2; }

origin=http://127.0.0.1:8411
pw=alpha-test-password-1
outdir=${ALP_QA_OUT:-$(dirname "$(realpath "$image")")}
mkdir -p "$outdir"

block() { sed -n "/$2-BEGIN/,/$2-END/p" <<<"$1"; }
pwjson() { printf '{"password":"%s"}' "$1"; }
# How long alpd takes for one path, measured in the guest.
# Retries only while alpd has not bound the port yet (Type=simple: the probe
# can come first); the time is that of the answer.
timed() { # tag, path
    echo "-/usr/bin/curl -s -o /dev/null -m 20 --retry 10 --retry-delay 1 --retry-connrefused -w '$1 %{http_code} %{time_total}\\n' http://127.0.0.1:8411$2"
}

# shellcheck disable=SC2329  # runs as ALP_HOST_HOOK (exported)
shoot() {
    local deadline=$((SECONDS + ${ALP_UI_WAIT:-300})) stats ppm
    ppm=$(mktemp --suffix=.ppm)
    while (( SECONDS < deadline )); do
        printf 'screendump %s\n' "$ppm" | socat - "UNIX-CONNECT:$ALP_MONITOR_SOCK" >/dev/null 2>&1 || true
        if [[ -s $ppm ]] && stats=$(python3 -I "$here/../lib/ppm2png.py" "$ppm" "$png"); then
            echo "ok:   kiosk screen is not blank ($stats), saved $png"; rm -f "$ppm"; return 0
        fi
        sleep 3
    done
    echo "FAIL: kiosk screen stayed blank for ${ALP_UI_WAIT:-300}s ${stats:-}, last picture in $png" >&2
    [[ -s $ppm ]] && python3 -I "$here/../lib/ppm2png.py" "$ppm" "$png" >/dev/null 2>&1 || true
    rm -f "$ppm"
    return 1
}
export -f shoot
export here

run_case() { # tpm | password | plain
    local mode=$1 tpm=0 work status=0 fail=0 args out1 out2 ext_uuid setup_body
    echo "== case: $mode"
    [[ $mode == tpm ]] && tpm=1
    work=$(mktemp -d -p "$(dirname "$(realpath "$image")")" t-ui-02.XXXXXX)
    # shellcheck disable=SC2064
    trap "rm -rf '$work'" RETURN
    ext_uuid=$(qa_make_disk "$work/external.raw")
    args="-drive if=none,id=ext,format=raw,file=$work/external.raw -device qemu-xhci -device usb-storage,drive=ext"
    export ALP_OVERLAY="$work/stick.qcow2" ALP_TPM=$tpm ALP_TPM_STATE="$work/tpm"

    local probe1
    # plain (ADR 0016 level 1): no password, nothing encrypted.
    setup_body=$(pwjson "$pw")
    [[ $mode == plain ]] && setup_body='{"password":"","protection":"none"}'
    probe1="$(QA_HTTP_TIMEOUT=90 qa_http_probe SETUP POST /api/v1/live/password/setup "$origin" "$setup_body")
$(qa_http_probe CREATE POST /api/v1/live/archive/create "$origin" "{\"partuuid\":\"$ext_uuid\"}")
/bin/sleep 60
$(qa_http_probe STATE1 GET /api/v1/live/state)"
    out1=$(ALP_RAM=${ALP_RAM:-2G} ALP_PROBE="$probe1" ALP_PROBE_DELAY=15 ALP_QEMU_ARGS="$args" "$here/run-vm.sh" "$image" 600 2>&1) || status=$?
    if (( status != 0 )); then echo "$out1" | tail -n 40; echo "FAIL: T-UI-02 boot 1 ($mode)" >&2; return 1; fi
    grep -qE '^HTTP/1\.[01] 200' <<<"$(block "$out1" SETUP)" || { block "$out1" SETUP; echo "FAIL: password setup ($mode)" >&2; return 1; }
    grep -qE '"state": ?"open"' <<<"$(block "$out1" STATE1)" || echo "note: archive not open yet after boot 1 ($mode); boot 2 still tests the start"

    # Boot 2: no delay, so the answer times are those right after the start.
    local probe2
    probe2="$(timed UI-PAGE /)
$(timed UI-FILE /live/i18n/en.json)
$(timed UI-STATE /api/v1/live/state)
-/bin/sh -c 'echo DIAG-BEGIN; cat /proc/uptime; systemctl list-jobs --no-pager; systemctl --failed --no-pager; journalctl -b --no-pager -o short-monotonic -u alpd -u alp-kiosk -n 60; echo DIAG-END'"
    [[ $mode == plain ]] && probe2+=$'\n'"-/bin/sh -c 'echo PLAIN-BEGIN; sleep 20; ls -1 /run/alp/host/ALPArchive; grep -c fuse.gocryptfs /proc/self/mountinfo; echo PLAIN-END'"
    export png="$outdir/t-ui-02-$mode.png"
    out2=$(ALP_RAM=${ALP_RAM:-2G} ALP_PROBE="$probe2" ALP_HOST_HOOK=shoot ALP_QEMU_ARGS="$args" "$here/run-vm.sh" "$image" 600 2>&1) || status=$?
    grep -E '^(ok|FAIL):' <<<"$out2" || true
    for tag in UI-PAGE UI-FILE UI-STATE; do
        local line code secs
        line=$(grep -m1 "^$tag " <<<"$out2" | tr -d '\r' || true)
        read -r _ code secs <<<"$line"
        if [[ $code =~ ^[23] ]] && awk -v s="${secs:-99}" 'BEGIN { exit !(s < 5) }'; then
            echo "ok:   $tag answers ($code in ${secs}s)"
        else
            echo "FAIL: $tag: ${line:-no answer} (expected 2xx/3xx within 5 s)" >&2; fail=1
        fi
    done
    if [[ $mode == plain ]]; then
        local pb
        pb=$(block "$out2" PLAIN | tr -d '\r')
        if grep -qx files <<<"$pb" && ! grep -qx data <<<"$pb" && grep -qx 0 <<<"$pb"; then
            echo "ok:   unprotected archive is the plain folder ALPArchive/files, nothing encrypted mounted"
        else
            echo "FAIL: unprotected archive: expected ALPArchive/files and no gocryptfs mount:" >&2; echo "$pb" >&2; fail=1
        fi
    fi
    (( status == 0 )) || fail=1
    if (( fail )); then
        echo "--- second start, for the diagnosis:"
        if grep -q DIAG-BEGIN <<<"$out2"; then block "$out2" DIAG | tr -d '\r'
        else echo "(the probe did not run: the start did not reach alp-healthy.target)"; tail -n 30 <<<"$out2"; fi
        grep -E '^TIMING:' <<<"$out2" || true
    fi
    return "$fail"
}

rc=0
# ALP_UI02_CASES="tpm plain" runs only those.
for c in ${ALP_UI02_CASES:-tpm password plain}; do
    run_case "$c" || rc=1
done
(( rc == 0 )) || { echo "FAIL: T-UI-02" >&2; exit 1; }
echo "PASS: T-UI-02"
