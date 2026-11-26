#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# The certificate maker (image/extra/usr/lib/alp/alp-live-tls) and the clock
# (0.2.8 security item): nothing is made on a clock before the image was
# built; the CA is never replaced because the clock says it expires; a CA
# "not valid yet" is replaced only once the network set the clock. Runs on
# the host with openssl, in a temporary folder.
# ok() never fails, so "check && ok || bad" is a plain if-else here.
# shellcheck disable=SC2015
set -euo pipefail
tool=$(realpath "$(dirname "$0")/../../image/extra/usr/lib/alp/alp-live-tls")
w=$(mktemp -d)
trap 'rm -rf "$w"' EXIT
export ALP_TLS_DIR=$w/tls ALP_TLS_PUB=$w/pub/tls ALP_TLS_FLOOR=$w/floor ALP_TLS_SYNCED=$w/synced
fail=0
ok() { echo "ok:   $1"; }
bad() { echo "FAIL: $1" >&2; fail=1; }
sha() { sha256sum "$ALP_TLS_DIR/$1" | cut -d' ' -f1; }
run() { "$tool" "$@" 2>"$w/err"; }

# A clock before the image was built: nothing is made.
touch -d '+3 days' "$w/floor"
if run 192.168.1.5; then bad "a fresh start on a wrong clock is refused"; else
    grep -q 'clock is wrong' "$w/err" && ok "a fresh start on a wrong clock is refused" || bad "clock message: $(cat "$w/err")"; fi
[[ ! -e $ALP_TLS_DIR/ca.key ]] && ok "no CA made on a wrong clock" || bad "a CA was made on a wrong clock"

# A right clock: CA and server certificate.
touch -d '-30 days' "$w/floor"
out=$(run 192.168.1.5) && [[ $out == changed ]] && ok "made on a right clock" || bad "first run: $out $(cat "$w/err")"
ca=$(sha ca.crt) srv=$(sha server.crt)
openssl verify -CAfile "$ALP_TLS_DIR/ca.crt" "$ALP_TLS_DIR/server.crt" >/dev/null && ok "server certificate verifies" || bad "verify"
out=$(run 192.168.1.5); [[ -z $out && $(sha server.crt) == "$srv" ]] && ok "nothing changes on a second run" || bad "second run changed: $out"

# The clock goes wrong (too early): a new address does not renew anything.
touch -d '+3 days' "$w/floor"
run 192.168.1.5 192.168.1.6 >/dev/null && [[ $(sha server.crt) == "$srv" && $(sha ca.crt) == "$ca" ]] &&
    ok "certificates kept on a wrong clock" || bad "changed on a wrong clock: $(cat "$w/err")"
touch -d '-30 days' "$w/floor"

# A CA close to its end (the clock says so): never replaced, phones keep it.
cd "$ALP_TLS_DIR"
cat >"$w/v3.cnf" <<CNF
[v3]
basicConstraints = critical,CA:TRUE,pathlen:0
keyUsage = critical,keyCertSign,cRLSign
subjectKeyIdentifier = hash
CNF
mkca() { # <start> <end>: re-sign the CA's own key with these dates
    mkdir -p "$w/ca" && : >"$w/ca/index"
    printf '[ca]\ndefault_ca=a\n[a]\ndatabase=%s/ca/index\nnew_certs_dir=%s/ca\nserial=%s/ca/serial\ndefault_md=sha256\npolicy=p\nunique_subject=no\n[p]\norganizationName=optional\ncommonName=supplied\n' "$w" "$w" "$w" >"$w/ca/ca.cnf"
    openssl req -new -key ca.key -subj "/O=ALPArchive/CN=test" -out "$w/ca.csr" 2>/dev/null
    openssl ca -batch -notext -config "$w/ca/ca.cnf" -rand_serial -selfsign -keyfile ca.key \
        -extfile "$w/v3.cnf" -extensions v3 -startdate "$1" -enddate "$2" -in "$w/ca.csr" -out ca.crt 2>/dev/null
}
mkca "$(date -u -d '9 years ago' +%Y%m%d%H%M%SZ)" "$(date -u -d '100 days' +%Y%m%d%H%M%SZ)"
ca=$(sha ca.crt)
run 192.168.1.5 >/dev/null && [[ $(sha ca.crt) == "$ca" ]] && ok "a CA near its end is not replaced" || bad "CA replaced near its end"
openssl verify -CAfile ca.crt server.crt >/dev/null && ok "server certificate made again under it" || bad "server not renewed under the old CA"

# A CA made while the clock was ahead ("not valid yet").
mkca "$(date -u -d '2 days' +%Y%m%d%H%M%SZ)" "$(date -u -d '10 years' +%Y%m%d%H%M%SZ)"
ca=$(sha ca.crt) srv=$(sha server.crt)
rm -f "$w/synced"
run 192.168.1.5 >/dev/null && [[ $(sha ca.crt) == "$ca" && $(sha server.crt) == "$srv" ]] &&
    ok "a future CA is kept until the network sets the clock" || bad "future CA changed without a network clock"
touch "$w/synced"
out=$(run 192.168.1.5) && [[ $out == changed && $(sha ca.crt) != "$ca" ]] && ok "a future CA is replaced once the clock is set" || bad "future CA not replaced: $out $(cat "$w/err")"
openssl verify -CAfile ca.crt server.crt >/dev/null && ok "and the server certificate verifies" || bad "verify after CA replace"

(( fail == 0 )) || { echo "FAIL: alp-live-tls clock checks" >&2; exit 1; }
echo "PASS: alp-live-tls clock checks"
