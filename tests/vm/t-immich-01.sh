#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# T-IMMICH-01: phone backup through Immich over the LAN proxy (docs/plans/live-immich.md).
# Boot 1: create the archive, get the phone login from the console API, upload
# photos through the HTTPS proxy on 443 (reached with a QEMU user-networking
# host forward, like a phone on the home network), read them back, try to
# delete them (must be refused). Boot 2 (same stick and disk): restart, log in
# again, hashes of all photos still match and the photos are still there.
# SKIPs while there is no Immich or no live API in the image.
# Immich is heavy and has crashed the local dev VM (owner instruction 2026-10-19):
# this test runs ONLY when ALP_ALLOW_IMMICH=1 is set (CI sets it on hosted
# runners; never on the dev VM); otherwise it skips (also under
# ALP_QA_STRICT=1, so the local release gate does not demand it). Nothing here
# depends on the dev VM: only /dev/kvm or ALP_ALLOW_TCG=1, QEMU, swtpm-free,
# curl, jq, socat and free local ports.
#
# Usage: tests/vm/t-immich-01.sh <disk-image>
# NOTE: field names of live/phone/pair follow docs/plans/live-alpha-api.md.
set -euo pipefail
here=$(dirname "$(realpath "$0")")
# shellcheck disable=SC1091,SC2153
. "$here/../lib/qa.sh"
image=${1:?usage: t-immich-01.sh <disk-image>}
[[ -f $image ]] || { echo "FAIL: image not found: $image" >&2; exit 2; }

if [[ ${ALP_ALLOW_IMMICH:-0} != 1 ]]; then
    echo "SKIP: T-IMMICH-01 runs only with ALP_ALLOW_IMMICH=1 (CI sets it; it can crash a small VM)"
    exit 0
fi
qa_image_has "$image" 'podman' || qa_skip "T-IMMICH-01: no podman/Immich in the image manifest yet"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
export work
# The hook runs inside run-vm.sh, which has its own $work: pass ours by
# another name.
export T_WORK=$work
# External disk (USB) that gets the archive; the same disk is used in both rounds.
disk="$work/external.raw"
part=$(qa_make_disk "$disk")
pw=alpha-test-password-1
pwjson() { printf '{"password":"%s"}' "$1"; }
export ALP_OVERLAY="$work/stick.qcow2"
origin=http://127.0.0.1:8411

# Photos: a valid 1x1 JPEG plus a random COM segment, so every file is unique.
mkdir -p "$work/photos"
tiny='/9j/4AAQSkZJRgABAQEASABIAAD/2wBDAP//////////////////////////////////////////////////////////////////////////////////////wgALCAABAAEBAREA/8QAFBABAAAAAAAAAAAAAAAAAAAAAP/aAAgBAQABPxA='
for i in 1 2 3 4 5; do
    { printf '\xff\xd8'; printf '\xff\xfe\x20\x02'; head -c 512 /dev/urandom; base64 -d <<<"$tiny" | tail -c +3; } >"$work/photos/IMG_000$i.jpg"
done
( cd "$work/photos" && sha256sum ./*.jpg >../hashes.txt )

# Runs on the host once the guest is up (ALP_HOST_HOOK). Argument: round (1|2).
# shellcheck disable=SC2153
hook() {
    local base="https://127.0.0.1:$PORT" c="curl -sk -m 60" tries=0
    local code
    until code=$($c -o /dev/null -w '%{http_code}' "$base/api/server/ping"); [[ $code == 200 ]]; do
        if (( ++tries >= 150 )); then
            # 403 without a client certificate = "Only paired phones" is on (T-PAIR-02 turns it off in round 1).
            [[ $code == 403 ]] && echo "FAIL: strict mode is on at this start (round $1): turning it off in round 1 did not last" >&2
            echo "FAIL: Immich does not answer on 443 (last answer: $code)" >&2; return 1
        fi
        sleep 2
    done
    echo "ok:   Immich answers through the proxy on 443 (round $ROUND)"
    # T34: / shows that the certificate works; nothing else outside the API.
    if [[ $ROUND == 1 ]]; then
        $c "$base/" | grep -q 'The connection is safe' || { echo "FAIL: no ALPArchive page at / on 443" >&2; return 1; }
        [[ $($c -o /dev/null -w '%{http_code}' "$base/photos") == 404 ]] || { echo "FAIL: /photos on 443 is not 404" >&2; return 1; }
        echo "ok:   / on 443 is the ALPArchive page, other paths 404"
    fi
    local json email password token
    json=$(sed -n '/PAIR-BEGIN/,/PAIR-END/p' "$ALP_SERIAL_LOG" | tr -d '\r' | sed '1,/^$/d;/PAIR-END/d' | tail -n 1)
    email=$(jq -r '.email // .user // .account.email' <<<"$json")
    password=$(jq -r '.password // .account.password' <<<"$json")
    # Since T23 phones sign in with the ALP password (the answer says
    # uses_main_password and carries no password).
    if [[ $password == null || -z $password ]] && jq -e '.uses_main_password == true' <<<"$json" >/dev/null 2>&1; then
        password=$PW
    fi
    [[ -n $email && $email != null && -n $password && $password != null ]] \
        || { echo "FAIL: live/phone/pair gave no login" >&2; return 1; }
    # The family account takes the ALP password a moment after Immich is up.
    local _try
    for _try in $(seq 18); do
        token=$($c -H 'Content-Type: application/json' -d "$(jq -nc --arg e "$email" --arg p "$password" '{email:$e,password:$p}')" \
            "$base/api/auth/login" | jq -r .accessToken)
        [[ -n $token && $token != null ]] && break
        sleep 10
    done
    [[ -n $token && $token != null ]] || { echo "FAIL: phone login refused" >&2; return 1; }
    echo "ok:   phone login works"
    local f id n=0
    if [[ $ROUND == 1 ]]; then
        for f in "$T_WORK"/photos/*.jpg; do
            id=$($c -H "Authorization: Bearer $token" -F deviceAssetId="$(basename "$f")" -F deviceId=alp-qa \
                -F fileCreatedAt=2026-01-01T00:00:00.000Z -F fileModifiedAt=2026-01-01T00:00:00.000Z \
                -F assetData=@"$f" "$base/api/assets" | jq -r .id)
            [[ -n $id && $id != null ]] || { echo "FAIL: upload of $f refused" >&2; return 1; }
            echo "$id $(basename "$f")" >>"$T_WORK/ids.txt"; n=$((n + 1))
        done
        echo "ok:   $n photos uploaded"
    fi
    local want got bad=0
    while read -r id f; do
        want=$(grep " \./$f\$" "$T_WORK/hashes.txt" | cut -d' ' -f1)
        got=$($c -H "Authorization: Bearer $token" "$base/api/assets/$id/original" | sha256sum | cut -d' ' -f1)
        [[ $want == "$got" ]] || { echo "FAIL: $f differs after download" >&2; bad=1; }
    done <"$T_WORK/ids.txt"
    (( bad == 0 )) || return 1
    echo "ok:   all photos downloaded with matching hashes (round $ROUND)"
    if [[ $ROUND == 1 ]]; then
        local ids code
        ids=$(awk '{printf "%s\"%s\"", sep, $1; sep=","}' "$T_WORK/ids.txt")
        for code in "DELETE /api/assets {\"ids\":[$ids],\"force\":true}" "DELETE /api/assets {\"ids\":[$ids]}" "POST /api/trash/empty {}"; do
            read -r method path data <<<"$code"
            got=$($c -X "$method" -H "Authorization: Bearer $token" -H 'Content-Type: application/json' -d "$data" -o /dev/null -w '%{http_code}' "$base$path")
            [[ $got == 403 || $got == 404 ]] || { echo "FAIL: $method $path answered $got (delete must be refused)" >&2; return 1; }
        done
        id=$(head -n1 "$T_WORK/ids.txt" | cut -d' ' -f1)
        [[ $($c -H "Authorization: Bearer $token" -o /dev/null -w '%{http_code}' "$base/api/assets/$id") == 200 ]] \
            || { echo "FAIL: photo vanished after refused delete" >&2; return 1; }
        echo "ok:   delete is refused and the photo is still there"
        pair02 "$base" "$token" || return 1
    fi
}

# T-PAIR-02 (ADR 0017), here because nginx runs only while Immich does: a
# phone paired through nginx's /alp/pair/v1/ uses its device certificate;
# strict mode refuses phones without one; revoking closes the phone's open
# WebSocket within seconds and refuses it afterwards.
# shellcheck disable=SC2329,SC2153  # called by hook; PW comes from its environment
pair02() { # base, immich token
    # alpd listens on the guest's 127.0.0.1 only, so the console side is the
    # guest's pair agent (started by the probe); it writes PAIRAGENT lines.
    local base=$1 tok=$2 c="curl -sk -m 30"
    local r uri ptok csr rid s wspid t0
    mark() { # word, seconds: wait for "PAIRAGENT <word>" on the console
        local i
        for ((i = 0; i < $2; i++)); do
            grep -a "PAIRAGENT $1" "$ALP_SERIAL_LOG" 2>/dev/null | tail -n 1 | tr -d '\r' | grep . && return 0
            sleep 1
        done
        return 1
    }
    local san sip sfile sstale safter
    san=$(mark SAN 180) || { echo "FAIL: T-PAIR-SAN: no result from the guest" >&2; return 1; }
    sip=$(sed -n 's/.* ip=\([^ ]*\).*/\1/p' <<<"$san")
    sfile=$(sed -n 's/.* file=\([^ ]*\).*/\1/p' <<<"$san")
    sstale=$(sed -n 's/.* stale=\([^ ]*\).*/\1/p' <<<"$san")
    safter=$(sed -n 's/.* after=\([^ ]*\).*/\1/p' <<<"$san")
    [[ -n $sfile && -n $sstale && $sfile != "$sstale" ]] \
        || { echo "FAIL: T-PAIR-SAN: could not make nginx serve an old certificate ($san)" >&2; return 1; }
    [[ $safter == "$sfile" && $san == *"opened=1"* ]] \
        || { echo "FAIL: T-PAIR-SAN: pair/open did not make nginx send the new certificate ($san)" >&2; return 1; }
    [[ $san == *"IPAddress:$sip"* && $san == *'conn="ok":true'* ]] \
        || { echo "FAIL: T-PAIR-SAN: the certificate sent does not name $sip, or phone/connection is not ok ($san)" >&2; return 1; }
    echo "ok:   T-PAIR-SAN: a stale nginx certificate is replaced before the QR; it names $sip"
    uri=$(mark URI 120 | sed 's/.*PAIRAGENT URI //')
    ptok=$(sed -n 's/.*[?&]t=\([^&]*\).*/\1/p' <<<"$uri")
    [[ -n $ptok ]] || { echo "FAIL: T-PAIR-02: no pairing window: $(grep -a PAIRAGENT "$ALP_SERIAL_LOG" | tr -d '\r' | head -3)" >&2; return 1; }
    openssl req -new -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -noenc -keyout "$T_WORK/phone.key" \
        -subj /CN=phone -out "$T_WORK/phone.csr" 2>/dev/null
    csr=$(awk '{printf "%s\\n", $0}' "$T_WORK/phone.csr")
    r=$($c -H 'Content-Type: application/json' -X POST "$base/alp/pair/v1/request" \
        -d "{\"token\":\"$ptok\",\"name\":\"CI phone\",\"csr\":\"$csr\"}")
    rid=$(jq -r .id <<<"$r")
    [[ -n $rid && $rid != null ]] || { echo "FAIL: T-PAIR-02: pairing request through nginx: $r" >&2; return 1; }
    mark APPROVED 120 >/dev/null || { echo "FAIL: T-PAIR-02: the console did not approve" >&2; return 1; }
    $c -H 'Content-Type: application/json' -X POST "$base/alp/pair/v1/status" -d "{\"id\":\"$rid\"}" | jq -r .certificate >"$T_WORK/phone.crt"
    sleep 5   # nginx reload
    local m=(--cert "$T_WORK/phone.crt" --key "$T_WORK/phone.key")
    [[ $($c "${m[@]}" -o /dev/null -w '%{http_code}' "$base/api/server/ping") == 200 ]] \
        || { echo "FAIL: T-PAIR-02: paired phone refused" >&2; return 1; }
    echo "ok:   phone paired through nginx; its certificate is accepted"
    mark STRICT 120 >/dev/null || { echo "FAIL: T-PAIR-02: strict mode was not turned on" >&2; return 1; }
    sleep 5
    [[ $($c -o /dev/null -w '%{http_code}' "$base/api/server/ping") == 403 ]] \
        || { echo "FAIL: T-PAIR-02: strict mode lets a phone without certificate in" >&2; return 1; }
    [[ $($c "${m[@]}" -o /dev/null -w '%{http_code}' "$base/api/server/ping") == 200 ]] \
        || { echo "FAIL: T-PAIR-02: strict mode refuses the paired phone" >&2; return 1; }
    echo "ok:   strict mode: only the paired phone gets in"
    # An open WebSocket (Immich's socket.io) of the paired phone.
    curl -sk -N --http1.1 -m 180 "${m[@]}" -H "Authorization: Bearer $tok" -H "Connection: Upgrade" -H "Upgrade: websocket" \
        -H "Sec-WebSocket-Version: 13" -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" \
        "$base/api/socket.io/?EIO=4&transport=websocket" >/dev/null 2>&1 &
    wspid=$!
    mark REVOKED 120 >/dev/null || { kill "$wspid" 2>/dev/null; echo "FAIL: T-PAIR-02: the phone was not revoked" >&2; return 1; }
    t0=$SECONDS
    while kill -0 "$wspid" 2>/dev/null && (( SECONDS - t0 < 30 )); do sleep 1; done
    s=$((SECONDS - t0))
    if kill -0 "$wspid" 2>/dev/null; then kill "$wspid"; echo "FAIL: T-PAIR-02: WebSocket still open ${s}s after revoke" >&2; return 1; fi
    (( s <= 15 )) || { echo "FAIL: T-PAIR-02: WebSocket closed only after ${s}s" >&2; return 1; }
    [[ $($c "${m[@]}" -o /dev/null -w '%{http_code}' "$base/api/server/ping") == 403 ]] \
        || { echo "FAIL: T-PAIR-02: revoked phone still gets in" >&2; return 1; }
    echo "ok:   revoked phone is refused; its WebSocket closed within ${s}s"
    # Strict mode off again, so round 2's phone (no certificate) gets in.
    local off
    off=$(mark DONE 120) || { echo "FAIL: T-PAIR-02: strict mode was not turned off again" >&2; return 1; }
    [[ $off == *"DONE {}"* ]] || { echo "FAIL: T-PAIR-02: turning strict mode off failed: ${off#*DONE }" >&2; return 1; }
    sleep 5
    echo "ok:   T-PAIR-02"
}
export -f pair02
export -f hook

# T-PAIR-02's console side in the guest: sign in, open a pairing window,
# approve the first phone, then strict mode, then revoke it (the host side,
# pair02, follows its PAIRAGENT lines on the console).
read -r -d '' agent <<AGENT || true
A=http://127.0.0.1:8411/api/v1/live
c() { curl -s -m 60 -b /run/pa-jar -c /run/pa-jar -H 'Origin: http://127.0.0.1:8411' -H 'Content-Type: application/json' "\$@"; }
say() { echo "PAIRAGENT \$*" >/dev/console; }
sleep 30
c -X POST \$A/console/signin -d '{"password":"$pw"}' >/dev/null
# T-PAIR-SAN (0.2.5 field report): nginx kept sending an old certificate
# without the computer's address. Make that happen (a new certificate file
# that nginx was not told about), then pair/open must notice and fix it
# before it gives a QR.
tls=/var/lib/alp-index/alp/tls
ip=\$(ip -4 -o addr show scope global | awk '{print \$4}' | cut -d/ -f1 | head -n 1)
fp() { openssl x509 -noout -fingerprint -sha256 2>/dev/null | cut -d= -f2; }
leaf() { timeout 5 openssl s_client -connect "\$ip:443" -noservername </dev/null 2>/dev/null | sed -n '/BEGIN CERT/,/END CERT/p'; }
# nginx runs only once Immich is up (its first start is slow).
for i in \$(seq 450); do [ -n "\$(leaf)" ] && break; sleep 2; done
rm -f \$tls/server.crt \$tls/server.key
/usr/lib/alp/alp-live-tls "\$ip" >/dev/null
file=\$(fp <\$tls/server.crt)
stale=\$(leaf | fp)
opened=\$(c -X POST \$A/pair/open -d '{}' | grep -c '"uri"')
after=\$(leaf | fp)
names=\$(leaf | openssl x509 -noout -ext subjectAltName 2>/dev/null | tr -d ' \n')
conn=\$(c \$A/phone/connection | tr -d ' \n' | grep -o '"ok":[a-z]*' | head -n 1)
say "SAN ip=\$ip file=\$file stale=\$stale after=\$after opened=\$opened conn=\$conn names=\$names"
# A fresh window every 4 minutes until the phone (the host) comes: a
# window lives 5 minutes, and the host reaches this after its photo checks.
id=
for w in \$(seq 5); do
  uri=\$(c -X POST \$A/pair/open -d '{}' | sed -n 's/.*"uri": *"\([^"]*\)".*/\1/p')
  say "URI \$uri"
  for i in \$(seq 120); do
    id=\$(c \$A/state | tr -d '\n' | sed -n 's/.*"pending": *\[ *{[^]]*"id": *"\([^"]*\)".*/\1/p')
    [ -n "\$id" ] && break 2
    sleep 2
  done
done
serial=\$(c -X POST \$A/pair/approve -d "{\"id\":\"\$id\"}" | sed -n 's/.*"serial": *"\([0-9A-F]*\)".*/\1/p')
say "APPROVED \$serial"
sleep 30; c -X POST \$A/pair/strict -d '{"on":true}' >/dev/null; say STRICT
sleep 40; c -X POST \$A/pair/revoke -d "{\"serial\":\"\$serial\"}" >/dev/null; say REVOKED
sleep 25; c -X POST \$A/console/signin -d '{"password":"$pw"}' >/dev/null
r=\$(c -X POST \$A/pair/strict -d '{"on":false}'); say "DONE \$r"
AGENT
agent_b64=$(base64 -w0 <<<"$agent")

status=0
for round in 1 2; do
    port=$(qa_free_port)
    args="-drive if=none,id=ext,format=raw,file=$disk -device qemu-xhci -device usb-storage,drive=ext -nic user,model=virtio-net-pci,hostfwd=tcp:127.0.0.1:$port-:443"
    probe=""
    if [[ $round == 1 ]]; then
        probe+="$(QA_HTTP_TIMEOUT=90 qa_http_probe SETUP POST /api/v1/live/password/setup "$origin" "$(pwjson "$pw")")
$(qa_http_probe CREATE POST /api/v1/live/archive/create "$origin" "{\"partuuid\":\"$part\",\"size_bytes\":1073741824}")
/bin/bash -c 'sleep 60'
"
    else
        probe+="$(QA_HTTP_TIMEOUT=90 qa_http_probe UNLOCK POST /api/v1/live/unlock "$origin" "$(pwjson "$pw")")
/bin/bash -c 'sleep 90'
"
    fi
    probe+="-/bin/bash -c 'exec 3<>/dev/tcp/127.0.0.1/8411 && echo API-LISTENING || echo API-NOT-LISTENING'
$(qa_http_probe PAIR POST /api/v1/live/phone/pair "$origin" '{}')"
    [[ $round == 1 ]] && probe+=$'\n'"-/usr/bin/systemd-run --unit=alp-pair-agent /bin/bash -c 'echo $agent_b64 | base64 -d | bash'"
    out=$(ROUND=$round PORT=$port PW=$pw ALP_RAM=${ALP_IMMICH_RAM:-4G} ALP_PROBE="$probe" ALP_PROBE_DELAY=60 ALP_QEMU_ARGS="$args" \
        ALP_HOST_HOOK=hook "$here/run-vm.sh" "$image" 600) || status=$?
    echo "$out"
    if grep -q 'API-NOT-LISTENING' <<<"$out"; then qa_skip "T-IMMICH-01: live API is not in this image yet"; fi
    (( status == 0 )) || { echo "FAIL: T-IMMICH-01 round $round" >&2; exit 1; }
done
echo "PASS: T-IMMICH-01"
