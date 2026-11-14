#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# T-PAIR-01: phone pairing (ADR 0017) with a simulated phone (curl and
# openssl in the guest, against alpd; no app and no Immich needed).
#   - the console opens a window; the QR carries K_id, the CA hash, the
#     addresses and a token; /identity's endorsement verifies with K_id;
#   - a wrong token is refused; the right one gives an id and a code; the
#     same token again is refused (single use);
#   - the console sees the request with the same code; after approval the
#     phone gets a clientAuth certificate that chains to the endorsed CA and
#     the device is in the register and the nginx allowlist;
#   - a second window, rejected on the console: the phone hears "rejected";
#   - revocation removes the device from the allowlist.
# The 5-minute expiry is covered by unit tests (pairing.rs), not here.
#
# Usage: tests/vm/t-pair-01.sh <disk-image>
set -euo pipefail
here=$(dirname "$(realpath "$0")")
# shellcheck disable=SC1091
. "$here/../lib/qa.sh"
image=${1:?usage: t-pair-01.sh <disk-image>}
[[ -f $image ]] || { echo "FAIL: image not found: $image" >&2; exit 2; }

# Runs in the guest as root. Prints ok:/FAIL: lines between PAIR-BEGIN/END.
read -r -d '' guest <<'GUEST' || true
set -u
A=http://127.0.0.1:8411/api/v1/live
J=/run/pair-test; mkdir -p $J; cd $J
c() { curl -s -m 30 -b jar -c jar -H 'Origin: http://127.0.0.1:8411' -H 'Content-Type: application/json' "$@"; }
ok() { echo "ok:   $1"; }
bad() { echo "FAIL: $1"; }
check() { if grep -qE -- "$2" <<<"$3"; then ok "$1"; else bad "$1: $3"; fi; }
echo PAIR-BEGIN
c -X POST $A/password/setup -d '{"password":"alpha-test-password-1"}' >/dev/null
open=$(c -X POST $A/pair/open -d '{}')
check "console opens a pairing window" '"uri": ?"alparchive://pair\?v=1&' "$open"
uri=$(sed -n 's/.*"uri": *"\([^"]*\)".*/\1/p' <<<"$open")
q() { sed -n "s/.*[?&]$1=\([^&]*\).*/\1/p" <<<"$uri"; }
tok=$(q t); kid=$(q id); cahash=$(q ca)
check "QR names an address" '^(10|192\.168|172)\.' "$(q a | sed 's/%2C/,/g')"
# Identity: K_id signs "alparchive-ca-endorsement-v1\n<id>\n<ca>\n".
idj=$(cat /run/alp/tls/identity.json)
id=$(sed -n 's/.*"id": *"\([^"]*\)".*/\1/p' <<<"$idj")
ca=$(sed -n 's/.*"ca": *"\([^"]*\)".*/\1/p' <<<"$idj")
[ "$(tr 'A-Z' 'a-z' <<<"$id")" = "$kid" ] && [ "$ca" = "$cahash" ] && ok "QR matches the published identity" || bad "QR does not match identity ($kid/$cahash vs $id/$ca)"
sed -n 's/.*"endorsement": *"\([^"]*\)".*/\1/p' <<<"$idj" | base64 -d >sig
printf 'alparchive-ca-endorsement-v1\n%s\n%s\n' "$id" "$ca" >msg
pad=$(( (8 - ${#id} % 8) % 8 )); eq=$(printf '%*s' $pad '' | tr ' ' '=')
{ printf '\x30\x2a\x30\x05\x06\x03\x2b\x65\x70\x03\x21\x00'; printf '%s%s' "$id" "$eq" | base32 -d; } >kid.der
openssl pkeyutl -verify -pubin -keyform DER -inkey kid.der -rawin -in msg -sigfile sig >/dev/null 2>&1 && ok "CA endorsement verifies with K_id" || bad "CA endorsement does not verify"
# Endorsement v2 (T62): the same, with the CA's sequence number signed too.
seq=$(sed -n 's/.*"seq": *\([0-9][0-9]*\).*/\1/p' <<<"$idj")
sed -n 's/.*"endorsement2": *"\([^"]*\)".*/\1/p' <<<"$idj" | base64 -d >sig2
printf 'alparchive-ca-endorsement-v2\n%s\n%s\n%s\n' "$id" "$ca" "$seq" >msg2
[[ $seq == 1 ]] && openssl pkeyutl -verify -pubin -keyform DER -inkey kid.der -rawin -in msg2 -sigfile sig2 >/dev/null 2>&1 \
    && ok "CA endorsement v2 verifies with K_id (seq 1)" || bad "CA endorsement v2 does not verify (seq '$seq')"
sed -n 's/.*"ca_cert": *"\([^"]*\)".*/\1/p' <<<"$idj" | sed 's/\\n/\n/g' >ca.pem
got=$(openssl x509 -in ca.pem -noout -pubkey | openssl pkey -pubin -outform DER | sha256sum | cut -d' ' -f1)
[ "$got" = "$ca" ] && ok "CA certificate matches the endorsed hash" || bad "CA hash differs"
# The phone: a P-256 key and CSR.
openssl req -new -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -noenc -keyout phone.key -subj /CN=phone 2>/dev/null >phone.csr
csr=$(awk '{printf "%s\\n", $0}' phone.csr)
r=$(c -X POST $A/pair/v1/request -d "{\"token\":\"wrong\",\"name\":\"Test phone\",\"csr\":\"$csr\"}")
check "wrong token refused" '"key": ?"pair.token"' "$r"
r=$(curl -s -m 30 -H 'Content-Type: application/json' -X POST $A/pair/v1/request -d "{\"token\":\"$tok\",\"name\":\"Test phone\",\"model\":\"QEMU\",\"os\":\"Linux\",\"csr\":\"$csr\"}")
check "phone request accepted (no cookie, no Origin)" '"code": ?"[0-9]{4}"' "$r"
rid=$(sed -n 's/.*"id": *"\([^"]*\)".*/\1/p' <<<"$r"); code=$(sed -n 's/.*"code": *"\([^"]*\)".*/\1/p' <<<"$r")
r2=$(curl -s -m 30 -H 'Content-Type: application/json' -X POST $A/pair/v1/request -d "{\"token\":\"$tok\",\"name\":\"Thief\",\"csr\":\"$csr\"}")
check "token is single use" '"key": ?"pair.token"' "$r2"
st=$(c $A/state)
check "console shows the phone with the same code" "\"code\": ?\"$code\"" "$st"
check "phone waits" '"state": ?"pending"' "$(curl -s -H 'Content-Type: application/json' -X POST $A/pair/v1/status -d "{\"id\":\"$rid\"}")"
c -X POST $A/pair/approve -d "{\"id\":\"$rid\"}" >appr
check "console approves" '"serial": ?"[0-9A-F]{32}"' "$(cat appr)"
serial=$(sed -n 's/.*"serial": *"\([0-9A-F]*\)".*/\1/p' appr)
s=$(curl -s -H 'Content-Type: application/json' -X POST $A/pair/v1/status -d "{\"id\":\"$rid\"}")
check "phone gets its certificate" '"state": ?"approved"' "$s"
sed -n 's/.*"certificate": *"\([^"]*\)".*/\1/p' <<<"$s" | sed 's/\\n/\n/g' >phone.crt
openssl verify -CAfile ca.pem phone.crt >/dev/null 2>&1 && ok "device certificate chains to the endorsed CA" || bad "device certificate does not chain"
openssl x509 -in phone.crt -noout -ext extendedKeyUsage 2>/dev/null | grep -q "Client Authentication" && ok "device certificate is for client authentication" || bad "no clientAuth"
[ "$(openssl x509 -in phone.crt -noout -pubkey)" = "$(openssl pkey -in phone.key -pubout)" ] && ok "certificate is for the phone's own key" || bad "certificate key differs"
grep -qx "$serial 1;" /run/alp/devices-map.conf && ok "device is on the nginx allowlist" || bad "device not on the allowlist"
grep -q "\"serial\":\"$serial\"" /var/lib/alp-index/alp/tls/devices.jsonl && ok "device is in the signed register" || bad "device not in the register"
# Rejected on the console.
open=$(c -X POST $A/pair/open -d '{}'); uri=$(sed -n 's/.*"uri": *"\([^"]*\)".*/\1/p' <<<"$open"); tok=$(q t)
r=$(curl -s -H 'Content-Type: application/json' -X POST $A/pair/v1/request -d "{\"token\":\"$tok\",\"name\":\"Other\",\"csr\":\"$csr\"}")
rid2=$(sed -n 's/.*"id": *"\([^"]*\)".*/\1/p' <<<"$r")
c -X POST $A/pair/reject -d "{\"id\":\"$rid2\"}" >/dev/null
check "rejected on the console: the phone hears it" '"state": ?"rejected"' "$(curl -s -H 'Content-Type: application/json' -X POST $A/pair/v1/status -d "{\"id\":\"$rid2\"}")"
# Revocation.
c -X POST $A/pair/revoke -d "{\"serial\":\"$serial\"}" >/dev/null
grep -q "$serial" /run/alp/devices-map.conf && bad "revoked device still on the allowlist" || ok "revoked device left the allowlist"
check "register keeps the revocation" '"revoked":true' "$(tail -n 1 /var/lib/alp-index/alp/tls/devices.jsonl)"
echo PAIR-END
GUEST

b64=$(base64 -w0 <<<"$guest")
probe="-/bin/bash -c 'echo $b64 | base64 -d | bash'"
status=0
out=$(ALP_RAM=${ALP_RAM:-2G} ALP_PROBE="$probe" ALP_PROBE_DELAY=15 \
    ALP_QEMU_ARGS="-nic user,model=virtio-net-pci" "$here/run-vm.sh" "$image" 600 2>&1) || status=$?
res=$(sed -n '/PAIR-BEGIN/,/PAIR-END/p' <<<"$out" | tr -d '\r' | grep -E '^(ok|FAIL):' || true)
echo "$res"
if (( status != 0 )) || [[ -z $res ]]; then
    tail -n 40 <<<"$out"; echo "FAIL: T-PAIR-01 (the VM run or the probe did not finish)" >&2; exit 1
fi
grep -q '^FAIL:' <<<"$res" && { echo "FAIL: T-PAIR-01" >&2; exit 1; }
echo "PASS: T-PAIR-01"
