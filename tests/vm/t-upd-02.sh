#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# T-UPD-02: a new version without erasing the stick (ADR 0026, steps 2 to 4
# of docs/plans/live-update.md), with the real parts: systemd-sysupdate,
# systemd-boot's boot counting and the dm-verity root of the new version.
#
#   start 1  a stick written with <image>: the ALP password is chosen; the
#            update files of <next-image> are offered by a local web server
#            and signed with a key made for this test; `alpctl live update`
#            finds the new version, writes it and says it is installed.
#   start 2  the same stick starts the new version by itself, the start is
#            marked good (the boot counter is gone), and the index opens
#            with the ALP password chosen before the update.
#
# T-UPD-03, on a copy of the stick taken after start 1: the new version's
# root is damaged (as T-BOOT-03 does). Three starts fail on dm-verity, each
# using one try of the boot counter; the fourth start is the old version
# again, with its index, and it says that the new version did not start.
#
# T-UPD-05, in start 2: the update server offers a list of files that was
# changed after it was signed; the stick refuses it.
#
# The image is not changed for the test: its update source and (empty)
# keyring are covered inside the guest by bind mounts made as root.
#
# Usage: tests/vm/t-upd-02.sh <image> <next-image> <next-version>
set -euo pipefail
here=$(dirname "$(realpath "$0")")
# shellcheck disable=SC1091
. "$here/../lib/qa.sh"
image=${1:?usage: t-upd-02.sh <image> <next-image> <next-version>}
next=${2:?usage: t-upd-02.sh <image> <next-image> <next-version>}
version=${3:?usage: t-upd-02.sh <image> <next-image> <next-version>}
for f in "$image" "$next"; do [[ -f $f ]] || { echo "FAIL: image not found: $f" >&2; exit 2; }; done

work=$(mktemp -d)
srv_pid=
trap '[[ -z $srv_pid ]] || kill "$srv_pid" 2>/dev/null; rm -rf "$work"' EXIT
origin=http://127.0.0.1:8411
pw=alpha-test-password-4
port=$(qa_free_port)

# The update files of the next version, signed with a key that exists only here.
mkdir -m 700 "$work/gnupg"
mkdir "$work/srv"
"$here/../../tools/update-files.sh" "$next" "$version" "$work/srv" >/dev/null
export GNUPGHOME="$work/gnupg"
gpg --batch --quiet --passphrase '' --quick-generate-key 'T-UPD-02 test key' ed25519 sign never
gpg --batch --yes --detach-sign -o "$work/srv/SHA256SUMS.gpg" "$work/srv/SHA256SUMS"
# T-UPD-05: the same folder with a list that was changed after signing (it
# offers a version 9.9.99 that the signature does not cover).
mkdir "$work/srv/bad"
cp "$work/srv/SHA256SUMS.gpg" "$work/srv/bad/"
{ cat "$work/srv/SHA256SUMS"; sed "s/_${version}/_9.9.99/g" "$work/srv/SHA256SUMS"; } > "$work/srv/bad/SHA256SUMS"
pubring=$(gpg --export | base64 -w0)
gpgconf --kill all 2>/dev/null || true
unset GNUPGHOME
(cd "$work/srv" && exec python3 -I -m http.server "$port" --bind 127.0.0.1 >/dev/null 2>&1) &
srv_pid=$!

# Inside the guest: this test's key and web server instead of the project's.
testsrc="-/bin/bash -c 'mkdir -p /run/alp-test/d && echo $pubring | base64 -d > /run/alp-test/pubring.gpg && for f in /usr/lib/sysupdate.d/*.transfer; do sed \"s|^Path=https://.*|Path=http://10.0.2.2:$port/|\" \$f > /run/alp-test/d/\$(basename \$f); done && mount --bind /run/alp-test/pubring.gpg /usr/lib/systemd/import-pubring.gpg && mount --bind /run/alp-test/d /usr/lib/sysupdate.d && echo TESTSRC-ok'"
# The same steps the Settings screen takes, through alpd's API (feature
# update), as a script run in the guest: look for updates, install, and
# follow state.update until the version is on the stick.
api_script=$(base64 -w0 <<'SCRIPT'
api() { curl -s -m 20 -H "Origin: http://127.0.0.1:8411" -H "Cookie: $(cat /run/qa-cookie)" "$@"; }
post() { api -X POST -H "Content-Type: application/json" -d "{}" "http://127.0.0.1:8411/api/v1/live/update/$1" | head -c 300; echo; }
newer() { api http://127.0.0.1:8411/api/v1/live/state | grep -o '"newer":{[^}]*}' | head -n 1; }
wait_for() { # state word, tries of 2 s
    for _ in $(seq "$2"); do
        n=$(newer); case $n in *"\"state\":\"$1\""*) break ;; esac; sleep 2
    done
    echo "API-$1 $n"
}
echo "API-POST-check $(post check)"; wait_for available 60
echo "API-POST-apply $(post apply)"; wait_for installed 800
api http://127.0.0.1:8411/api/v1/live/state | grep -o '"update":{.*' | head -c 600; echo
SCRIPT
)
check() { printf '%s' "-/bin/bash -c 'echo $1-BEGIN; alpctl live update check 2>&1 | tail -n 8; echo $1-END'"; }
between() { sed -n "/$1-BEGIN/,/$1-END/p" <<<"$out" | tr -d '\r'; }
boot() { # probe, seconds
    local status=0
    out=$(ALP_OVERLAY="$work/stick.qcow2" ALP_OVERLAY_SIZE=32G ALP_RAM=${ALP_RAM:-2G} ALP_PROBE="$1" ALP_PROBE_DELAY=15 \
        ALP_QEMU_ARGS="-nic user,model=virtio-net-pci" "$here/run-vm.sh" "$image" "$2") || status=$?
    (( status == 0 )) || { echo "$out" | tail -n 60 | cut -c1-300; echo "FAIL: T-UPD-02 (VM run failed)" >&2; exit 1; }
}
fail() { echo "FAIL: T-UPD-02: $*" >&2; exit 1; }

# Start 1: choose the password, find the update, write it.
boot "$(QA_HTTP_TIMEOUT=120 qa_http_probe SETUP POST /api/v1/live/password/setup "$origin" "{\"password\":\"$pw\"}")
$testsrc
-/bin/bash -c 'echo \$(grep ^IMAGE_VERSION= /usr/lib/os-release || echo IMAGE_VERSION=none)'
$(check CHECK1)
-/bin/bash -c 'echo APPLY-BEGIN; echo $api_script | base64 -d > /run/alp-test/api.sh; bash /run/alp-test/api.sh 2>&1; echo APPLY-END'
$(check CHECK2)
-/bin/bash -c 'echo STARTS-BEGIN; lsblk -rno PARTLABEL,START /dev/\$(lsblk -rno PKNAME \$(findmnt -no SOURCE /var/lib/alp) | head -n 1); echo STARTS-END'
-/bin/bash -c 'echo PARTS-BEGIN; lsblk -rno PARTLABEL /dev/\$(lsblk -rno PKNAME \$(findmnt -no SOURCE /var/lib/alp) | head -n 1); echo PARTS-END'" 1800
grep -q 'TESTSRC-ok' <<<"$out" || qa_skip "T-UPD-02: the update tools are not in this image yet"
old=$(grep -o 'IMAGE_VERSION="\?[0-9.]*' <<<"$out" | head -n 1 | tr -d '"' | cut -d= -f2)
[[ -n $old && $old != "$version" ]] || fail "the written image has no version of its own: '$old'"
between CHECK1 | grep -q "^newer $version available" || fail "the new version was not found: $(between CHECK1)"
echo "ok:   $old finds version $version (signature checked)"
shown=${version%0}   # what people see: 9.9.90 is shown as 9.9.9
between APPLY | grep -q "^API-available .*\"version\":\"$shown\"" || fail "alpd did not find the new version: $(between APPLY)"
between APPLY | grep -q "^API-installed .*\"state\":\"installed\"" || fail "alpd did not write the update: $(between APPLY)"
echo "ok:   alpd's update API finds and installs the version (as the Settings screen does)"
between CHECK2 | grep -q "^newer $version installed" || fail "the written version is not seen as installed: $(between CHECK2)"
between CHECK2 | grep -q "^start-file alp-os_$version+3-0.efi" || fail "the new start file has no boot counter of three tries: $(between CHECK2)"
parts=$(between PARTS)
if ! grep -q "^alp-root-$version\$" <<<"$parts" || ! grep -q "^alp-verity-$version\$" <<<"$parts"; then fail "the slots do not carry the new version: $parts"; fi
[[ $(grep -c '^_empty$' <<<"$parts") == 2 && $(grep -c '^root-x86-64$' <<<"$parts") == 1 ]] || fail "the other slots or the written root changed: $parts"
echo "ok:   version $version is written to a free slot; the written system is untouched"
# For T-UPD-03: a copy of the stick as it is now, before the new version starts.
cp --sparse=always "$work/stick.qcow2" "$work/stick-b.qcow2"
root_start=$(between STARTS | awk -v l="alp-root-$version" '$1 == l {print $2}')
[[ $root_start =~ ^[0-9]+$ ]] || fail "the new root slot's place on the stick is not known: $(between STARTS)"

# Start 2: the stick starts the new version, marks it good, and the index opens.
boot "$testsrc
-/bin/bash -c 'for i in \$(seq 90); do systemctl is-active -q systemd-bless-boot.service && break; sleep 2; done; echo BLESS-\$(systemctl is-active systemd-bless-boot.service)'
-/bin/bash -c 'echo \$(grep ^IMAGE_VERSION= /usr/lib/os-release || echo IMAGE_VERSION=none)'
$(QA_HTTP_TIMEOUT=90 qa_http_probe UNLOCK POST /api/v1/live/unlock "$origin" "{\"password\":\"$pw\"}")
$(check CHECK3)
-/bin/bash -c 'sed -i \"s|^Path=http://10.0.2.2:$port/|Path=http://10.0.2.2:$port/bad/|\" /run/alp-test/d/*.transfer; grep -c /bad/ /run/alp-test/d/10-root.transfer'
$(check CHECK5)" 600
now=$(grep -o 'IMAGE_VERSION="\?[0-9.]*' <<<"$out" | head -n 1 | tr -d '"' | cut -d= -f2)
[[ $now == "$version" ]] || fail "the second start runs '$now', not $version"
echo "ok:   the second start runs version $version"
grep -q 'BLESS-active' <<<"$out" || fail "the healthy start was not marked good: $(grep -o 'BLESS-[a-z]*' <<<"$out" | head -n 1)"
between CHECK3 | grep -q "^start-file alp-os_$version.efi" || fail "the start file still has its boot counter: $(between CHECK3)"
between CHECK3 | grep -q "^newer none" || fail "version $version is still offered after it started: $(between CHECK3)"
echo "ok:   the start is marked good (no boot counter left)"
between UNLOCK | grep -q '^HTTP/1.1 200' || fail "the index does not open with the password chosen before the update: $(between UNLOCK | head -n 3)"
echo "ok:   the index opens with the ALP password chosen before the update"
echo "PASS: T-UPD-02"

# T-UPD-05: a list of update files whose signature does not fit is refused.
if between CHECK5 | grep -q 'not signed by the ALP-OS project' && ! between CHECK5 | grep -q '^newer '; then
    echo "ok:   a changed list of update files is refused (signature)"
    echo "PASS: T-UPD-05"
else
    echo "FAIL: T-UPD-05: a changed list of update files was not refused: $(between CHECK5)" >&2; exit 1
fi

# T-UPD-03: damage the new root (1 MiB, 1 MiB into the slot) on the copy.
fail() { echo "FAIL: T-UPD-03: $*" >&2; exit 1; }
qemu-io -f qcow2 -c "write -q -P 0x5a $(( root_start * 512 + 1024 * 1024 )) 1M" "$work/stick-b.qcow2" >/dev/null
expect='device-mapper: verity: [^ ]+: (data|metadata) block [0-9]+ is corrupted'
for try in 1 2 3; do
    ALP_OVERLAY="$work/stick-b.qcow2" ALP_EXPECT_FAIL=$expect ALP_RAM=${ALP_RAM:-2G} "$here/run-vm.sh" "$image" 300 >/dev/null \
        || fail "try $try of the damaged version did not fail on dm-verity"
    echo "ok:   try $try: the damaged version does not start (dm-verity)"
done
status=0
out=$(ALP_OVERLAY="$work/stick-b.qcow2" ALP_RAM=${ALP_RAM:-2G} ALP_PROBE_DELAY=15 ALP_QEMU_ARGS="-nic user,model=virtio-net-pci" ALP_PROBE="$testsrc
-/bin/bash -c 'echo \$(grep ^IMAGE_VERSION= /usr/lib/os-release || echo IMAGE_VERSION=none)'
$(QA_HTTP_TIMEOUT=90 qa_http_probe UNLOCK POST /api/v1/live/unlock "$origin" "{\"password\":\"$pw\"}")
$(check CHECK4)" "$here/run-vm.sh" "$image" 600) || status=$?
(( status == 0 )) || { echo "$out" | tail -n 40 | cut -c1-300; fail "the fourth start did not come up"; }
back=$(grep -o 'IMAGE_VERSION="\?[0-9.]*' <<<"$out" | head -n 1 | tr -d '"' | cut -d= -f2)
[[ $back == "$old" ]] || fail "the fourth start runs '$back', not the old version $old"
between CHECK4 | grep -q "^newer $version failed" || fail "the old version does not say that $version did not start: $(between CHECK4)"
between UNLOCK | grep -q '^HTTP/1.1 200' || fail "the index does not open after the rollback: $(between UNLOCK | head -n 3)"
echo "ok:   after three failed tries the stick starts version $old again, and its index opens"
echo "PASS: T-UPD-03"
