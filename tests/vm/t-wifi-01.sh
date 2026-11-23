#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# T-WIFI-01: Live joins a Wi-Fi network through the console API, and the
# WPA3 switch-off kept on the stick reaches iwd (0.2.7, WPA3 report).
# Two simulated radios (mac80211_hwsim): iwd runs a WPA2 access point on
# the second one and Live tries to join it on the first; that join is
# reported only (see below). Passes when:
#   - live/state reports "wpa3_off" (false after a first-try join),
#   - a switch-off file on the stick (/var/lib/iwd/.alp-main.conf) is what
#     iwd sees as /etc/iwd/main.conf after a restart (unit bind mount), and
#     iwd lists SaeDisable among the driver's flags when the driver is known.
# The real case (a driver that claims WPA3 and fails its sign-in) cannot be
# simulated; the second-try logic is covered by wifi.rs unit tests.
#
# Usage: tests/vm/t-wifi-01.sh <disk-image>
set -euo pipefail
here=$(dirname "$(realpath "$0")")
# shellcheck disable=SC1091
. "$here/../lib/qa.sh"
image=${1:?usage: t-wifi-01.sh <disk-image>}
[[ -f $image ]] || { echo "FAIL: image not found: $image" >&2; exit 2; }

# The access point: second radio, WPA2 passphrase, iwd's own AP mode.
ap_script=$(base64 -w0 <<'GUEST'
# iwd keeps the kernel's own interfaces (otherwise it deletes and makes
# them again, and one came back as "Too many open files in system").
printf '[DriverQuirks]\nDefaultInterface=mac80211_hwsim\n' > /var/lib/iwd/.alp-main.conf
# Radios first, then iwd starts and finds both (when they came while iwd
# was running, it took only the second one).
modprobe mac80211_hwsim radios=2 && echo HWSIM-OK || echo HWSIM-MISSING
for i in $(seq 20); do
    [ "$(ls -d /sys/class/net/*/phy80211 2>/dev/null | wc -l)" -ge 2 ] && break
    sleep 1
done
systemctl restart iwd.service
sleep 6
radios=$(for d in /sys/class/net/*; do [ -e "$d/phy80211" ] && basename "$d"; done | sort)
echo "RADIOS=$(echo $radios)"
ap=$(echo "$radios" | tail -n 1)
# Both radios powered (iwd may bring a new adapter up powered off).
for a in $(timeout 10 iwctl adapter list | sed 's/\x1b\[[0-9;]*m//g' | grep -o 'phy[0-9]*' | sort -u); do
    timeout 10 iwctl adapter "$a" set-property Powered on 2>&1 | sed 's/^/IWCTL /'
done
for d in $radios; do timeout 10 iwctl device "$d" set-property Powered on 2>&1 | sed 's/^/IWCTL /'; done
sleep 2
mkdir -p /var/lib/iwd/ap
printf '[Security]\nPassphrase=alp-test-pass-1\n' > /var/lib/iwd/ap/ALPTEST.ap
timeout 10 iwctl device "$ap" set-property Mode ap 2>&1 | sed 's/^/IWCTL /'
sleep 3
timeout 10 iwctl ap "$ap" start-profile ALPTEST 2>&1 | sed 's/^/IWCTL /'
sleep 3
timeout 10 iwctl ap "$ap" show 2>&1 | sed 's/\x1b\[[0-9;]*m//g; s/^/IWCTL /'
timeout 10 iwctl ap "$ap" show 2>&1 | grep -q 'Started *yes' && echo AP-STARTED || echo AP-FAILED
timeout 10 iwctl device list 2>&1 | sed 's/\x1b\[[0-9;]*m//g; s/^/IWCTL /'
sta=$(echo "$radios" | head -n 1)
timeout 10 iwctl station "$sta" scan 2>&1 | sed 's/^/IWCTL /'
sleep 5
timeout 10 iwctl station "$sta" get-networks 2>&1 | sed 's/\x1b\[[0-9;]*m//g; s/^/IWCTL /'
journalctl -b -u iwd.service --no-pager -o cat | tail -n 15 | sed 's/^/IWDJ /'
sleep 4
GUEST
)
quirk_script=$(base64 -w0 <<'GUEST'
sta=$(for d in /sys/class/net/*; do [ -e "$d/phy80211" ] && basename "$d"; done | sort | head -n 1)
drv=$(basename "$(readlink "/sys/class/net/$sta/device/driver" 2>/dev/null)" 2>/dev/null)
echo "DRIVER=$drv"
printf '[DriverQuirks]\nSaeDisable=%s\n' "${drv:-none}" > /var/lib/iwd/.alp-main.conf
systemctl restart iwd.service
sleep 6
pid=$(systemctl show -p MainPID --value iwd.service)
echo "IWDCONF=$(grep -c '^SaeDisable=' "/proc/$pid/root/etc/iwd/main.conf" 2>/dev/null)"
journalctl -b -u iwd.service --no-pager -o cat | grep -E 'Driver Flags' | tail -n 2 | sed 's/^/IWDLOG /'
rm -f /var/lib/iwd/.alp-main.conf
systemctl restart iwd.service
sleep 4
pid=$(systemctl show -p MainPID --value iwd.service)
echo "IWDCONF2=$(grep -c '^SaeDisable=' "/proc/$pid/root/etc/iwd/main.conf" 2>/dev/null)"
GUEST
)

probe="-/bin/bash -c 'echo $ap_script | base64 -d > /run/t-wifi-ap.sh; bash /run/t-wifi-ap.sh'
$(qa_http_probe CONNECT POST /api/v1/live/wifi/connect http://127.0.0.1:8411 '{"ssid":"ALPTEST","password":"alp-test-pass-1"}')
-/usr/bin/sleep 75
$(QA_HTTP_MAX=200000 qa_http_probe STATE GET /api/v1/live/state)
-/bin/bash -c 'echo $quirk_script | base64 -d > /run/t-wifi-quirk.sh; bash /run/t-wifi-quirk.sh'"

status=0
out=$(ALP_RAM=${ALP_RAM:-2G} ALP_PROBE="$probe" ALP_PROBE_DELAY=15 \
    ALP_QEMU_ARGS="-nic user,model=virtio-net-pci" \
    "$here/run-vm.sh" "$image" 420) || status=$?
echo "$out"
(( status == 0 )) || { echo "FAIL: T-WIFI-01 (VM run failed)" >&2; exit 1; }

grep -q 'HWSIM-OK' <<<"$out" || qa_skip "T-WIFI-01: mac80211_hwsim is not in this image's kernel"

fail=0
expect() { if grep -qE -- "$2" <<<"$3"; then echo "ok:   $1"; else echo "FAIL: $1" >&2; fail=1; fi; }
block() { sed -n "/$1-BEGIN/,/$1-END/p" <<<"$out"; }

# The join on simulated radios is a best effort: in CI iwd kept losing one
# of the two simulated interfaces ("No default interface for wiphy 1",
# 2026-11-22, four runs). It is reported, not judged; the checks below on
# the switch-off reaching iwd are the ones that must pass.
note_or_ok() { if grep -qE -- "$2" <<<"$3"; then echo "ok:   $1"; else echo "note: $1: not seen (simulated radios)"; fi; }
note_or_ok "two simulated radios" 'RADIOS=[^ ]+ [^ ]+' "$out"
note_or_ok "the access point started" 'AP-STARTED' "$out"
note_or_ok "connect is accepted" '^HTTP/1\.[01] 20[02]' "$(block CONNECT)"
state=$(sed -n '/^{/,$p' <<<"$(block STATE)" | sed '/STATE-END/d')
if jq -e '.network.ports[] | select(.kind == "wifi" and .ssid == "ALPTEST")' >/dev/null 2>&1 <<<"$state"; then
    echo "ok:   Live joined the simulated network (port shows ALPTEST)"
    if jq -e '.network.wpa3_off == false' >/dev/null 2>&1 <<<"$state"; then echo "ok:   WPA3 stays on after a first-try join"; else echo "FAIL: wpa3_off is not false after a first-try join" >&2; fail=1; fi
else
    echo "note: Live did not join the simulated network: $(jq -c '.network.job' <<<"$state" 2>/dev/null)"
fi
if jq -e '.network | has("wpa3_off")' >/dev/null 2>&1 <<<"$state"; then echo "ok:   live/state reports wpa3_off"; else echo "FAIL: live/state has no wpa3_off" >&2; fail=1; fi
expect "iwd reads the switch-off from the stick" 'IWDCONF=1' "$out"
expect "without the file iwd has its defaults" 'IWDCONF2=0' "$out"
if grep -qE '^DRIVER=.+' <<<"$out" && ! grep -q '^DRIVER=$' <<<"$out"; then
    expect "iwd turns WPA3 off for the driver" 'IWDLOG .*SaeDisable' "$out"
else
    echo "note: the simulated radio has no driver name; the SaeDisable flag check is skipped"
fi

(( fail == 0 )) || { echo "FAIL: T-WIFI-01" >&2; exit 1; }
echo "PASS: T-WIFI-01"
