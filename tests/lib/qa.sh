#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Helpers shared by the Live alpha VM scenarios (T-API-01, T-IMMICH-01, T-UI-01).
# Source it: . "$(dirname "$(realpath "$0")")/../lib/qa.sh"
#
# qa_skip <why>        print "SKIP: <why>" and stop with success, so a scenario
#                      for a feature that does not exist yet does not break
#                      `just test-vm`. With ALP_QA_STRICT=1 (release gate) a
#                      skip is a failure.
# qa_image_has <regex> true if a package matching <regex> is in the image
#                      manifest next to the image (no boot needed).
# qa_free_port         print an unused TCP port on 127.0.0.1
# qa_hmp <socket> <cmd>  send one QEMU monitor command
# qa_http_probe ...    one ALP_PROBE line that does an HTTP request on the guest's loopback

qa_skip() {
    echo "SKIP: $*"
    if [[ ${ALP_QA_STRICT:-0} == 1 ]]; then echo "FAIL: skipped under ALP_QA_STRICT=1" >&2; exit 1; fi
    exit 0
}

qa_image_has() { # image, ERE for a package name
    local manifest
    manifest="$(dirname "$(realpath "$1")")/$(basename "$1" .raw).manifest"
    if [[ ! -f $manifest ]]; then # no manifest next to the image: use the repo's package lists
        grep -rqhE "^[[:space:]]*($2)[[:space:]]*$" "$(dirname "${BASH_SOURCE[0]}")/../../image" --include='*.conf' 2>/dev/null
        return
    fi
    python3 -I -c '
import json, re, sys
names = [p["name"] for p in json.load(open(sys.argv[1]))["packages"]]
sys.exit(0 if any(re.fullmatch(sys.argv[2], n) for n in names) else 1)' "$manifest" "$2"
}

qa_free_port() {
    python3 -I -c '
import socket
s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])'
}

qa_hmp() { # socket, command
    printf '%s\n' "$2" | socat - "UNIX-CONNECT:$1" >/dev/null
}

# Guest-side HTTP request over bash's /dev/tcp (the image has no curl).
# Prints "<tag>: <status line>" and the body between BEGIN/END markers on the
# serial console. Use as one ALP_PROBE line:
#   $(qa_http_probe tag GET /api/v1/live/state [Origin-value] [json-body])
qa_http_probe() { # tag, method, path, origin, body
    local tag=$1 method=$2 path=$3 origin=${4:-} body=${5:-} hdr a b
    hdr="X-Alp-Test: 1"
    [[ -z $origin ]] || hdr="Origin: $origin"
    a=$(printf '%s %s HTTP/1.0\r\nHost: 127.0.0.1:8411\r\n%s\r\n' "$method" "$path" "$hdr" | base64 -w0)
    b=$(printf 'Content-Type: application/json\r\nContent-Length: %s\r\n\r\n%s' "${#body}" "$body" | base64 -w0)
    # The session cookie (alp_console, set by password/setup, unlock, console/signin) is kept in
    # /run/qa-cookie for the following requests of the same boot, like a browser's cookie jar.
    printf '%s' "-/bin/bash -c 'exec 3<>/dev/tcp/127.0.0.1/8411 || { echo ${tag}-NO-CONNECT; exit 0; }; { echo ${a} | base64 -d; [ -s /run/qa-cookie ] && printf \"Cookie: %s\\r\\n\" \"\$(cat /run/qa-cookie)\"; echo ${b} | base64 -d; } >&3; echo ${tag}-BEGIN; timeout ${QA_HTTP_TIMEOUT:-10} cat <&3 | tr -d \"\\r\" | tee /run/qa-resp | head -c ${QA_HTTP_MAX:-65536}; echo; grep -i \"^set-cookie: alp_console=\" /run/qa-resp | head -1 | sed \"s/^[^:]*: *//; s/;.*//\" > /run/qa-cookie.new; [ -s /run/qa-cookie.new ] && mv /run/qa-cookie.new /run/qa-cookie; echo ${tag}-END'"
}

# Make a sparse 24 GiB GPT disk with one big ext4 partition (stands in for the
# computer's disk) and print its partition UUID. Same layout as T-LIVE-02.
qa_make_disk() { # file
    local disk=$1 uuid=4a1d2b3c-0000-4000-8000-000000000001
    truncate -s 24G "$disk"
    sfdisk -q "$disk" <<SFDISK
label: gpt
start=2048, size=46137344, type=0FC63DAF-8483-4772-8E79-3D69D8477DE4, uuid=$uuid
SFDISK
    mkfs.ext4 -q -F -E offset=$((2048 * 512)) "$disk" $((46137344 / 2))k
    echo "$uuid"
}
