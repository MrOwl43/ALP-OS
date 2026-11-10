#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# T-TERM-01 (Live UI stage F, server and system side): the Terminal
# (docs/plans/live-terminal.md revision 2; Security and Network Expert
# review T1-T6 and the round-2 build notes).
#
# Guest, through the console API:
#   the feature `terminal`; ttyd's version (VERIFY); no cron or at, no
#     lingering user manager for alp-term;
#   terminal/start: a wrong password is 403 and counts; the right one gives
#     http://127.0.0.1:8413/<32 hex>/; a second start is 409; the state says
#     running and never holds the address; the session file is gone; the
#     address is not in the journal or /var/log; ttyd runs as alp-term and
#     listens on 127.0.0.1 only; another path, or /, answers 404;
#   WebSocket (when this curl speaks it): another origin is refused; one
#     client is held, a second is refused; when the client leaves, the
#     session ends by itself (--once);
#   console/lock ends a session and leaves no alp-term process; the old
#     address answers nothing; terminal/stop ends one and answers 200 again
#     when none runs;
#   five wrong passwords lock terminal/start and sign-in alike (429).
# Guest, inside the unit's own sandbox (its ExecStart swapped for a probe
# by a drop-in in /run, everything else unchanged):
#   uid 58413 in no other group; new connections to 8411 (with faked
#   browser headers), 8410, 8412, 2283, 5432, 6379, [::1]:8411 and this
#   computer's own address on 80 and 443 all fail, and the firewall rule
#   counts them (T1); /run/alp, /run/podman, /run/dbus, the stick's state,
#   the index, the archive and Immich cannot be read; other users'
#   processes are hidden; the system is read-only; no user namespaces; the
#   log page is readable (T6); only ttyd's port 8413 can be bound; a
#   command line typed in the real shell is
#   in the journal (T4) and TMOUT is 600 and read-only.
# Guest, whole system: no abstract unix listener outside an empty
#   allowlist (review R1: the sandbox does not hide them).
#
# Usage: tests/vm/t-term-01.sh <disk-image>
set -euo pipefail
here=$(dirname "$(realpath "$0")")
# shellcheck disable=SC1091
. "$here/../lib/qa.sh"
image=${1:?usage: t-term-01.sh <disk-image>}
[[ -f $image ]] || { echo "FAIL: image not found: $image" >&2; exit 2; }

work=$(mktemp -d -p "$(dirname "$(realpath "$image")")" t-term-01.XXXXXX)
trap 'rm -rf "$work"' EXIT
pw=alpha-test-password-1
export ALP_OVERLAY="$work/stick.qcow2"

# Runs as alp-term in the unit's sandbox. Its lines go to the journal of
# alp-terminal.service, each with the marker TP-PROBE.
read -r -d '' probe <<'PROBE' || true
M=TP-PROBE
ok() { echo "$M ok:   $1"; }
bad() { echo "$M FAIL: $1"; }
[ "$(id -u)" = 58413 ] && ok "runs as uid 58413" || bad "runs as uid $(id -u)"
[ "$(id -G)" = 58413 ] && ok "in no group but its own" || bad "groups: $(id -G)"
lan_t=
[ -n "$ALP_LAN" ] && lan_t="$ALP_LAN:80 $ALP_LAN:443"
# shellcheck disable=SC2086 # lan_t is a list of words
for t in 127.0.0.1:8411 127.0.0.1:8410 127.0.0.1:8412 127.0.0.1:2283 127.0.0.1:5432 127.0.0.1:6379 "[::1]:8411" $lan_t; do
  curl -s -m 5 -o /dev/null -H 'Host: 127.0.0.1:8411' -H 'Origin: http://127.0.0.1:8411' \
    -H 'Sec-Fetch-Site: same-origin' "http://$t/api/v1/live/state"
  rc=$?
  [ "$rc" != 0 ] && ok "no connection to $t (curl $rc)" || bad "reached $t"
done
for p in /run/alp /run/alp/media /run/podman /run/dbus /run/user /var/lib/alp /var/lib/alp-index /var/lib/alp-pool /var/lib/immich /var/log/journal; do
  ls "$p" >/dev/null 2>&1 && bad "can read $p" || ok "cannot read $p"
done
[ -e /proc/1 ] && bad "sees process 1 (root)" || ok "other users' processes are hidden"
touch /etc/alp-term-x 2>/dev/null && bad "can write /etc" || ok "the system is read-only"
touch /tmp/alp-term-x && ok "own /tmp is writable" || bad "no /tmp"
unshare -U true 2>/dev/null && bad "can make a user namespace" || ok "no new namespaces"
# SocketBindAllow=tcp:8413, SocketBindDeny=any (security review 2026-11-04).
if command -v perl >/dev/null; then
  b() { perl -MIO::Socket::INET -e 'exit(IO::Socket::INET->new(LocalAddr => "127.0.0.1", LocalPort => $ARGV[0], Proto => $ARGV[1], ($ARGV[1] eq "tcp" ? (Listen => 1) : ())) ? 0 : 1)' "$1" "$2" 2>/dev/null; }
  b 8499 tcp && bad "can listen on tcp 8499" || ok "cannot listen on tcp 8499"
  b 8499 udp && bad "can bind udp 8499" || ok "cannot bind udp 8499"
  b 8413 tcp && ok "can bind ttyd's port 8413" || bad "cannot bind ttyd's port 8413"
  b 0 tcp && echo "$M note: a random free port can still be bound (port 0, not filtered by systemd)"
else
  bad "no perl to test SocketBind"
fi
{ touch /run/alp-terminal/x || echo x >>/run/alp-terminal/logs.txt; } 2>/dev/null && bad "can write the log page" || ok "cannot write the log page"
head -1 /run/alp-terminal/logs.txt 2>/dev/null | grep -q 'ALPArchive boot log' && ok "log page readable (T6)" || bad "no log page"
out=$(printf 'echo "t=$TMOUT"\nTMOUT=0\necho cmd-TP-PROBE-typed\nexit\n' | /usr/lib/alp/alp-term-shell 2>&1)
echo "$out" | grep -qx 't=600' && ok "TMOUT is 600" || bad "TMOUT: $out"
echo "$out" | grep -q 'TMOUT: readonly' && ok "TMOUT is read-only" || bad "TMOUT can be changed"
echo "$M END"
PROBE
probe_b64=$(base64 -w0 <<<"$probe")

read -r -d '' guest <<GUEST || true
set -u
A=http://127.0.0.1:8411/api/v1/live
J=(-b /run/tt-jar -c /run/tt-jar -H 'Origin: http://127.0.0.1:8411' -H 'Content-Type: application/json')
code() { curl -s -m 60 -o /run/tt-body -w '%{http_code}' "\${J[@]}" "\$@"; }
ok() { echo "ok:   \$1"; }
bad() { echo "FAIL: \$1"; }
note() { echo "note: \$1"; }
start() { code -X POST \$A/terminal/start -d "{\"password\":\"\$1\"}"; }
active() { systemctl is-active --quiet alp-terminal.service; }
# Wait up to \$1 seconds for the unit to end.
ended() { for _ in \$(seq "\$1"); do active || return 0; sleep 1; done; return 1; }
none_left() { ! pgrep -u 58413 >/dev/null; }
ws() { curl -s -m "\$1" -o /dev/null -H "Origin: \$2" -H 'Sec-WebSocket-Protocol: tty' "\$3"; }
echo TT-BEGIN
note "ttyd: \$(ttyd --version 2>&1 | head -1)"
for b in cron crond atd at anacron; do command -v "\$b" >/dev/null && bad "\$b is in the image"; done
ok "checked: no cron or at"
[ -e /var/lib/systemd/linger/alp-term ] && bad "alp-term lingers" || ok "no lingering user manager"
[ "\$(id -u alp-term)" = 58413 ] && ok "user alp-term is 58413" || bad "alp-term: \$(id alp-term 2>&1)"

[ "\$(code -X POST \$A/password/setup -d '{"password":"$pw"}')" = 200 ] && ok "password setup" || bad "password setup: \$(cat /run/tt-body)"
st=\$(curl -s -m 30 "\${J[@]}" \$A/state | tr -d '\n ')
echo "\$st" | grep -q '"terminal"' && echo "\$st" | grep -q 'features":\[[^]]*"terminal"' && ok "feature terminal is on" || bad "feature terminal missing"
echo "\$st" | grep -q '"terminal":{"state":"off"}' && ok "state: off" || bad "state: \$(echo "\$st" | grep -o '"terminal":{[^}]*}')"

# The sandbox, with the unit's own settings and a probe in place of ttyd.
lan=\$(ip -4 -o addr show scope global | awk '{print \$4}' | cut -d/ -f1 | head -1)
# The test VM may have no network address: give it one on a dummy
# interface (review). A connection to any own address leaves through lo,
# the same as on real hardware.
if [ -z "\$lan" ]; then
  ip link add alpt0 type dummy && ip addr add 192.0.2.10/24 dev alpt0 && ip link set alpt0 up && lan=192.0.2.10
fi
[ -n "\$lan" ] && ok "own address \$lan" || bad "no own address, not even a dummy one"
tries=9
before=\$(nft list chain inet alp output | grep -o 'packets [0-9]*' | head -1 | cut -d' ' -f2)
mkdir -p /run/systemd/system/alp-terminal.service.d
printf 'ALP_TERM_PATH=probe\n' >/run/alp/terminal.env
alpctl live term-logs || bad "the log page was not written"
[ "\$(stat -c '%U %a' /run/alp-terminal-logs /run/alp-terminal-logs/logs.txt | tr '\n' ' ')" = "root 755 root 644 " ] && ok "log page: root 0755/0644" || bad "log page owner/mode: \$(stat -c '%n %U %a' /run/alp-terminal-logs /run/alp-terminal-logs/logs.txt)"
cat >/run/systemd/system/alp-terminal.service.d/zz-probe.conf <<UNIT
[Service]
Environment=ALP_LAN=\$lan
StandardOutput=journal
StandardError=journal
ExecStart=
ExecStart=/bin/sh -c "echo $probe_b64 | base64 -d | sh"
UNIT
systemctl daemon-reload
systemctl start alp-terminal.service; rc=\$?
[ \$rc = 0 ] && ok "the sandbox starts (the firewall check passed)" || { bad "the sandbox did not start"; journalctl -b -u alp-terminal.service --no-pager -o short -n 40 | sed 's/^/diag: /'; systemctl show alp-terminal.service -p Result -p ExecMainStatus -p ActiveState --no-pager | sed 's/^/diag: /'; }
ended 60 || bad "the probe did not end"
for _ in 1 2 3; do journalctl -u alp-terminal.service -o cat | grep -q 'TP-PROBE END' && break; sleep 2; done
journalctl -u alp-terminal.service -o cat | grep '^TP-PROBE \(ok\|FAIL\|note\):' | sed 's/^TP-PROBE //; s/^/sandbox /' | sed 's/^sandbox ok:   /ok:   sandbox: /; s/^sandbox FAIL: /FAIL: sandbox: /' | sed 's/^sandbox note: /note: sandbox: /'
journalctl -u alp-terminal.service -o cat | grep -q 'TP-PROBE END' || bad "the probe did not finish: \$(journalctl -u alp-terminal.service -o cat | tail -5)"
after=\$(nft list chain inet alp output | grep -o 'packets [0-9]*' | head -1 | cut -d' ' -f2)
[ "\$((after - before))" -ge "\$tries" ] && ok "the firewall rule refused them (\$before -> \$after packets)" || bad "firewall counter \$before -> \$after"
journalctl -b -t alp-terminal -o cat | grep -q 'command: echo cmd-TP-PROBE-typed' && ok "a typed command line is in the journal (T4)" || bad "command not logged"
rm -rf /run/systemd/system/alp-terminal.service.d /run/alp/terminal.env
systemctl daemon-reload
systemctl reset-failed alp-terminal.service 2>/dev/null
none_left && ok "nothing of the probe is left" || bad "alp-term processes left"

# R1: abstract unix sockets are not hidden by the unit's filesystem
# sandbox, and the unit shares the host's network namespace, so every
# abstract listener here is reachable from the session. Allowed: none
# (an X server such as Xwayland on @/tmp/.X11-unix/X0 would let the
# session draw over the kiosk).
abs_allow='^$'
abs=\$(ss -xlH | awk '{print \$5}' | grep '^@' | sort -u)
note "abstract unix listeners: \${abs:-none}"
n=0
for s in \$abs; do echo "\$s" | grep -Eq "\$abs_allow" || { bad "abstract listener reachable from the session: \$s"; n=1; }; done
[ \$n = 0 ] && ok "no abstract unix listener outside the allowlist (R1)"
[ -x /usr/bin/Xwayland ] && bad "Xwayland can start (cage would open an X server)" || ok "no Xwayland binary in the image"
note "DNS: \$(grep -m1 '^nameserver' /etc/resolv.conf 2>/dev/null || echo 'no nameserver line')"

# A real session through the API.
r=\$(start wrong-password-1)
[ "\$r" = 403 ] && grep -q password.wrong /run/tt-body && ok "wrong password: 403" || bad "wrong password: \$r \$(cat /run/tt-body)"
active && bad "a wrong password started a session" || ok "no session after a wrong password"
r=\$(start "$pw")
url=\$(grep -o '"url": *"[^"]*"' /run/tt-body | sed 's/.*"\(http[^"]*\)"/\1/')
[ "\$r" = 200 ] && ok "start: 200" || bad "start: \$r \$(cat /run/tt-body)"
echo "\$url" | grep -Eq '^http://127\.0\.0\.1:8413/[0-9a-f]{32}/\$' && ok "address: loopback, 8413, 128-bit path" || bad "address: \$url"
path=\$(echo "\$url" | cut -d/ -f4)
[ -e /run/alp/terminal.env ] && bad "the session file is left" || ok "the session file is gone"
st=\$(curl -s -m 30 "\${J[@]}" \$A/state | tr -d '\n ')
echo "\$st" | grep -q '"terminal":{"state":"running"}' && ok "state: running" || bad "state: \$(echo "\$st" | grep -o '"terminal":{[^}]*}')"
echo "\$st" | grep -q "\$path" && bad "the state holds the path" || ok "the state never holds the path"
[ "\$(ps -o user= -C ttyd | sort -u)" = alp-term ] && ok "ttyd runs as alp-term" || bad "ttyd runs as: \$(ps -o user= -C ttyd)"
# Every TCP and UDP socket alp-term listens on: exactly one, on loopback
# (libwebsockets binds to 0.0.0.0 when it cannot read the interfaces).
l=\$(ss -Hltune | grep -w 'uid:58413' | awk '{print \$1" "\$5}' | sort -u | tr '\n' ' ')
[ "\$l" = "tcp 127.0.0.1:8413 " ] && ok "the session's only listener is 127.0.0.1:8413" || bad "alp-term listens on: \${l:-nothing}"
r=\$(curl -s -m 10 -o /run/tt-page -w '%{http_code}' "\$url")
[ "\$r" = 200 ] && ok "the address answers 200" || bad "the address: \$r"
for u in http://127.0.0.1:8413/ "http://127.0.0.1:8413/0123456789abcdef0123456789abcdef/"; do
  r=\$(curl -s -m 10 -o /dev/null -w '%{http_code}' "\$u")
  [ "\$r" = 404 ] && ok "another path: 404 (\${u#http://127.0.0.1:8413})" || bad "\$u answered \$r"
done
r=\$(start "$pw")
[ "\$r" = 409 ] && grep -q terminal.running /run/tt-body && ok "second start: 409" || bad "second start: \$r \$(cat /run/tt-body)"
wsurl="ws://127.0.0.1:8413/\$path/ws"
if curl -V | grep -qw ws; then
  ws 4 http://127.0.0.1:8411 "\$wsurl"; rc=\$?
  [ "\$rc" != 28 ] && ok "WebSocket from another origin refused (curl \$rc)" || bad "WebSocket from another origin was held"
  (ws 6 http://127.0.0.1:8413 "\$wsurl"; echo \$? >/run/tt-ws1) &
  sleep 2
  ws 3 http://127.0.0.1:8413 "\$wsurl"; rc=\$?
  [ "\$rc" != 28 ] && ok "a second WebSocket client is refused (curl \$rc)" || bad "a second client was held"
  wait
  [ "\$(cat /run/tt-ws1)" = 28 ] && ok "the first WebSocket client was held" || bad "the first client: curl \$(cat /run/tt-ws1)"
  ended 10 && ok "the session ended when its client left (--once)" || bad "the session went on after its client left"
  sleep 3
  curl -s -m 30 "\${J[@]}" \$A/state | tr -d '\n ' | grep -q '"terminal":{"state":"off"}' && ok "state: off again" || bad "state not off after --once"
else
  note "this curl has no WebSocket support: WebSocket checks skipped"
  code -X POST \$A/terminal/stop -d '{}' >/dev/null
fi
[ -n "\$path" ] && journalctl -b -o cat | grep -qF "\$path" && bad "the path is in the journal" || ok "the path is not in the journal"
grep -rqs "\$path" /var/log && bad "the path is in /var/log" || ok "the path is not in /var/log"

# The console lock ends a session.
r=\$(start "$pw")
[ "\$r" = 200 ] && ok "start again: 200" || bad "start again: \$r \$(cat /run/tt-body)"
old=\$(grep -o '"url": *"[^"]*"' /run/tt-body | sed 's/.*"\(http[^"]*\)"/\1/')
[ "\$(code -X POST \$A/console/lock -d '{}')" = 200 ] && ok "console lock" || bad "console lock: \$(cat /run/tt-body)"
ended 15 && ok "the lock ended the session" || bad "the session went on after the lock"
sleep 1
none_left && ok "no alp-term process left" || bad "left: \$(pgrep -u 58413 -a)"
[ -e /run/alp-terminal-logs ] && bad "the log page is left after the session" || ok "the log page is removed after the session"
r=\$(curl -s -m 5 -o /dev/null -w '%{http_code}' "\$old"); rc=\$?
[ "\$rc" != 0 ] && ok "the old address answers nothing (curl \$rc)" || bad "the old address answered \$r"
r=\$(start "$pw")
[ "\$r" = 401 ] && ok "locked console: start refused (401)" || bad "locked console: start \$r"

# terminal/stop.
[ "\$(code -X POST \$A/console/signin -d '{"password":"$pw"}')" = 200 ] && ok "sign in again" || bad "sign in: \$(cat /run/tt-body)"
[ "\$(start "$pw")" = 200 ] && ok "start: 200" || bad "start: \$(cat /run/tt-body)"
[ "\$(code -X POST \$A/terminal/stop -d '{}')" = 200 ] && ok "stop: 200" || bad "stop: \$(cat /run/tt-body)"
ended 15 && ok "stop ended the session" || bad "the session went on after stop"
none_left && ok "no alp-term process left" || bad "left: \$(pgrep -u 58413 -a)"
[ "\$(code -X POST \$A/terminal/stop -d '{}')" = 200 ] && ok "stop with no session: 200" || bad "stop again: \$(cat /run/tt-body)"

# Review question: ttyd compares the WebSocket path only up to the
# request's own length, so a client asking for / may pass its path check.
if curl -V | grep -qw ws && [ "\$(start "$pw")" = 200 ]; then
  ws 3 http://127.0.0.1:8413 ws://127.0.0.1:8413/; rc=\$?
  [ "\$rc" = 28 ] && note "ttyd held a WebSocket on / (path check is a prefix match)" || note "ttyd refused a WebSocket on / (curl \$rc)"
  code -X POST \$A/terminal/stop -d '{}' >/dev/null
  ended 15 || bad "the session went on after stop"
fi

# The lockout is shared with sign-in.
for i in 1 2 3 4 5; do r=\$(start "wrong-password-\$i"); done
r=\$(start "wrong-password-6")
[ "\$r" = 429 ] && grep -q retry_after_seconds /run/tt-body && ok "too many wrong passwords: 429" || bad "after 6 wrong: \$r \$(cat /run/tt-body)"
r=\$(code -X POST \$A/console/signin -d '{"password":"$pw"}')
[ "\$r" = 429 ] && ok "sign-in waits too (one counter)" || bad "sign-in after terminal tries: \$r"
active && bad "a session runs" || ok "no session at the end"
echo TT-END
GUEST

b64=$(base64 -w0 <<<"$guest")
status=0
out=$(ALP_RAM=${ALP_RAM:-2G} ALP_PROBE="-/bin/bash -c 'echo $b64 | base64 -d | bash'" ALP_PROBE_DELAY=15 \
    "$here/run-vm.sh" "$image" 900 2>&1) || status=$?
res=$(sed -n '/TT-BEGIN/,/TT-END/p' <<<"$out" | tr -d '\r' | grep -E '^(ok|FAIL|note|diag):' || true)
echo "$res"
if (( status != 0 )) || ! grep -q 'TT-END' <<<"$out" || ! grep -q '^ok:' <<<"$res"; then
    tail -n 60 <<<"$out" | tr -d '\r'; echo "FAIL: T-TERM-01 (did not finish)" >&2; exit 1
fi
grep -q '^FAIL:' <<<"$res" && { echo "FAIL: T-TERM-01" >&2; exit 1; }
echo "PASS: T-TERM-01"
