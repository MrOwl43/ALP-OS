#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# T-LIVE-06 (Live UI 0.2.5 stage F, server side): read-only Files on an
# external disk (POST live/media/open, media/list, media/eject; GET
# live/media/file); docs/plans/live-ui-0.2.5-stage-f.md, crates/alp-core/
# src/live_media.rs, Security and Network Expert review F2-F4
# (live-ui-stage-f-review-2026-11-04.md).
#
# Disk: an external usb-storage disk (one ext4 partition, label ALPMEDIA)
# holding readme.txt, big.log (300000 bytes), photo.jpg, evil.svg, page.html, sub/inner.txt and a
# symbolic link link.txt -> /etc/os-release.
#
# One boot (ok:/FAIL: lines):
#   state.media lists the disk (feature media_files on);
#   list before open: 409 media.not_open; open: mounted read-only, noexec,
#     nosuid, nodev, and the partition is read-only in the kernel;
#   list: files and the folder, never the link; "..", "." and "a/b" names: 400;
#   file readme.txt: text/plain, sandbox CSP, inline, nosniff, no-store,
#     Accept-Ranges; Range 0-3: 206 with Content-Range and 4 bytes; a range
#     past the end: 416;
#   big.log: the UI's 256 KiB preview (206, text/plain); text capped at
#     256 KiB even for bytes=0-; whole without Range: 413;
#   evil.svg and page.html: 422 media.type; link.txt: refused (no link
#     followed); Sec-Fetch-Dest: document: 403;
#   eject: unmounted; the old id: 404 for file and list.
# Host: the disk image is unchanged byte for byte.
#
# Usage: tests/vm/t-live-06.sh <disk-image>
set -euo pipefail
here=$(dirname "$(realpath "$0")")
# shellcheck disable=SC1091
. "$here/../lib/qa.sh"
image=${1:?usage: t-live-06.sh <disk-image>}
[[ -f $image ]] || { echo "FAIL: image not found: $image" >&2; exit 2; }

work=$(mktemp -d -p "$(dirname "$(realpath "$image")")" t-live-06.XXXXXX)
trap 'rm -rf "$work"' EXIT
pw=alpha-test-password-1

disk="$work/media.raw"
truncate -s 1G "$disk"
sfdisk -q "$disk" <<SFDISK
label: gpt
start=2048, size=2000000, type=0FC63DAF-8483-4772-8E79-3D69D8477DE4
SFDISK
mkdir -p "$work/seed/sub"
printf 'Hello from the camera card.\n<script>alert(1)</script>\n' >"$work/seed/readme.txt"
printf '\xff\xd8\xff\xe0 not really a photo' >"$work/seed/photo.jpg"
printf '<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>' >"$work/seed/evil.svg"
printf '<html><script>alert(1)</script></html>' >"$work/seed/page.html"
echo inner >"$work/seed/sub/inner.txt"
head -c 300000 /dev/zero | tr '\0' 'a' >"$work/seed/big.log"
ln -s /etc/os-release "$work/seed/link.txt"
mkfs.ext4 -q -F -L ALPMEDIA -d "$work/seed" -E offset=$((2048 * 512)) "$disk" $((2000000 / 2))k
before=$(sha256sum "$disk" | cut -d' ' -f1)
size=$(stat -c %s "$work/seed/readme.txt")

args="-device qemu-xhci -drive if=none,id=md,format=raw,file=$disk -device usb-storage,drive=md,serial=ALPMEDIA"
export ALP_OVERLAY="$work/stick.qcow2"

read -r -d '' guest <<GUEST || true
set -u
A=http://127.0.0.1:8411/api/v1/live
J=(-b /run/md-jar -c /run/md-jar -H 'Origin: http://127.0.0.1:8411' -H 'Content-Type: application/json')
code() { curl -s -m 60 -o /run/md-body -w '%{http_code}' "\${J[@]}" "\$@"; }
# GET a file: headers to /run/md-head, body to /run/md-body, status printed.
fget() { curl -s -m 60 -D /run/md-head -o /run/md-body -w '%{http_code}' -b /run/md-jar "\$@"; }
hdr() { tr -d '\r' </run/md-head | grep -i "^\$1:" | head -1 | cut -d' ' -f2-; }
ok() { echo "ok:   \$1"; }
bad() { echo "FAIL: \$1"; }
enc() { printf '%s' "\$1" | od -An -tx1 | tr -d ' \n' | sed 's/../%&/g'; }
furl() { echo "\$A/media/file?media=\$1&path=\$(enc "\$2")"; }
echo MD-BEGIN
[ "\$(code -X POST \$A/password/setup -d '{"password":"$pw"}')" = 200 ] && ok "password setup" || bad "password setup: \$(cat /run/md-body)"
st=
for i in \$(seq 30); do
  st=\$(curl -s -m 30 "\${J[@]}" \$A/state | tr -d '\n')
  echo "\$st" | grep -q '"label": *"ALPMEDIA"' && break
  sleep 2
done
echo "\$st" | grep -q '"media_files"' && ok "feature media_files is on" || bad "feature media_files missing"
id=\$(echo "\$st" | grep -o '"id": *"[0-9a-f]\{24\}"[^}]*"label": *"ALPMEDIA"' | grep -o '[0-9a-f]\{24\}' | head -1)
[ -n "\$id" ] && ok "state.media lists the disk" || bad "disk not in state.media: \$(echo "\$st" | grep -o '"media": *\[[^]]*\]')"
part=\$(lsblk -nrpo NAME,LABEL | awk '\$2=="ALPMEDIA"{print \$1}' | head -1)

r=\$(code -X POST \$A/media/list -d "{\"media\":\"\$id\",\"path\":[]}")
[ "\$r" = 409 ] && grep -q media.not_open /run/md-body && ok "list before open: 409 media.not_open" || bad "list before open: \$r \$(cat /run/md-body)"
r=\$(code -X POST \$A/media/open -d "{\"media\":\"\$id\"}")
[ "\$r" = 200 ] && ok "open" || bad "open: \$r \$(cat /run/md-body)"
mnt=/run/alp/media/\$id
opts=\$(findmnt -no OPTIONS "\$mnt")
case ",\$opts," in *,ro,*) ok "mounted read-only" ;; *) bad "not read-only: \$opts" ;; esac
for o in nosuid nodev noexec; do case ",\$opts," in *,\$o,*) ok "mounted \$o" ;; *) bad "not \$o: \$opts" ;; esac; done
[ "\$(blockdev --getro "\$part")" = 1 ] && ok "partition read-only in the kernel" || bad "partition writable (\$part)"

r=\$(code -X POST \$A/media/list -d "{\"media\":\"\$id\",\"path\":[]}")
l=\$(tr -d '\n' </run/md-body)
[ "\$r" = 200 ] && echo "\$l" | grep -q '"readme.txt"' && echo "\$l" | grep -q '"sub"' && ok "list shows files and folders" || bad "list: \$r \$l"
echo "\$l" | grep -q '"link.txt"' && bad "the link is listed" || ok "links are not listed"
r=\$(code -X POST \$A/media/list -d "{\"media\":\"\$id\",\"path\":[\"sub\"]}")
grep -q '"inner.txt"' /run/md-body && ok "list a folder" || bad "list sub: \$r \$(cat /run/md-body)"
for p in '[".."]' '["."]' '["sub/.."]' '["sub",".."]' '["/etc"]'; do
  r=\$(code -X POST \$A/media/list -d "{\"media\":\"\$id\",\"path\":\$p}")
  [ "\$r" = 400 ] && ok "path \$p refused (400)" || bad "path \$p answered \$r"
done

r=\$(fget "\$(furl \$id '["readme.txt"]')")
[ "\$r" = 200 ] && grep -q 'Hello from the camera card' /run/md-body && ok "text file served" || bad "readme: \$r"
[ "\$(hdr content-type)" = "text/plain; charset=utf-8" ] && ok "Content-Type text/plain" || bad "Content-Type: \$(hdr content-type)"
[ "\$(hdr content-security-policy)" = "sandbox; default-src 'none'" ] && ok "sandbox CSP" || bad "CSP: \$(hdr content-security-policy)"
[ "\$(hdr content-disposition)" = inline ] && ok "Content-Disposition inline" || bad "Content-Disposition: \$(hdr content-disposition)"
[ "\$(hdr x-content-type-options)" = nosniff ] && ok "nosniff" || bad "nosniff missing"
[ "\$(hdr cache-control)" = no-store ] && ok "no-store" || bad "Cache-Control: \$(hdr cache-control)"
[ "\$(hdr accept-ranges)" = bytes ] && ok "Accept-Ranges bytes" || bad "Accept-Ranges: \$(hdr accept-ranges)"
r=\$(fget -H 'Range: bytes=0-3' "\$(furl \$id '["readme.txt"]')")
[ "\$r" = 206 ] && [ "\$(cat /run/md-body)" = Hell ] && [ "\$(hdr content-range)" = "bytes 0-3/$size" ] && ok "Range 0-3: 206" || bad "range: \$r '\$(cat /run/md-body)' \$(hdr content-range)"
r=\$(fget -H 'Range: bytes=999999-' "\$(furl \$id '["readme.txt"]')")
[ "\$r" = 416 ] && ok "range past the end: 416" || bad "range past the end: \$r"
r=\$(fget -H 'Range: bytes=0-262143' "\$(furl \$id '["big.log"]')")
[ "\$r" = 206 ] && [ "\$(hdr content-type)" = "text/plain; charset=utf-8" ] && [ "\$(stat -c %s /run/md-body)" = 262144 ] && [ "\$(hdr content-range)" = "bytes 0-262143/300000" ] && ok "UI text preview: .log as text, 256 KiB" || bad "big.log preview: \$r \$(hdr content-type) \$(hdr content-range)"
r=\$(fget -H 'Range: bytes=0-' "\$(furl \$id '["big.log"]')")
[ "\$r" = 206 ] && [ "\$(stat -c %s /run/md-body)" = 262144 ] && ok "text answer capped at 256 KiB" || bad "text cap: \$r \$(stat -c %s /run/md-body)"
r=\$(fget "\$(furl \$id '["big.log"]')")
[ "\$r" = 413 ] && ok "text over 256 KiB without Range: 413" || bad "big.log whole: \$r"
r=\$(fget "\$(furl \$id '["photo.jpg"]')")
[ "\$r" = 200 ] && [ "\$(hdr content-type)" = image/jpeg ] && ok "photo served as image/jpeg" || bad "photo: \$r \$(hdr content-type)"
for f in evil.svg page.html; do
  r=\$(fget "\$(furl \$id "[\"\$f\"]")")
  [ "\$r" = 422 ] && grep -q media.type /run/md-body && ok "\$f refused (422)" || bad "\$f answered \$r"
done
r=\$(fget "\$(furl \$id '["link.txt"]')")
case "\$r" in 2*) bad "the link was followed (\$r)" ;; *) ok "link not followed (\$r)" ;; esac
grep -q 'ID=' /run/md-body && bad "os-release leaked through the link" || ok "nothing read through the link"
r=\$(fget -H 'Sec-Fetch-Dest: document' "\$(furl \$id '["readme.txt"]')")
[ "\$r" = 403 ] && ok "Sec-Fetch-Dest document refused (403)" || bad "Sec-Fetch-Dest document: \$r"
r=\$(fget -H 'Sec-Fetch-Dest: image' "\$(furl \$id '["photo.jpg"]')")
[ "\$r" = 200 ] && ok "Sec-Fetch-Dest image allowed" || bad "Sec-Fetch-Dest image: \$r"

r=\$(code -X POST \$A/media/eject -d "{\"media\":\"\$id\"}")
[ "\$r" = 200 ] && ok "eject" || bad "eject: \$r \$(cat /run/md-body)"
findmnt "\$mnt" >/dev/null && bad "still mounted after eject" || ok "unmounted after eject"
r=\$(fget "\$(furl \$id '["readme.txt"]')")
[ "\$r" = 404 ] && ok "old id after eject: file 404" || bad "old id after eject: file \$r"
r=\$(code -X POST \$A/media/list -d "{\"media\":\"\$id\",\"path\":[]}")
[ "\$r" = 404 ] && ok "old id after eject: list 404" || bad "old id after eject: list \$r"
echo MD-END
GUEST

b64=$(base64 -w0 <<<"$guest")
status=0
out=$(ALP_RAM=${ALP_RAM:-2G} ALP_PROBE="-/bin/bash -c 'echo $b64 | base64 -d | bash'" ALP_PROBE_DELAY=15 ALP_QEMU_ARGS="$args" \
    "$here/run-vm.sh" "$image" 900 2>&1) || status=$?
res=$(sed -n '/MD-BEGIN/,/MD-END/p' <<<"$out" | tr -d '\r' | grep -E '^(ok|FAIL):' || true)
echo "$res"
if (( status != 0 )) || ! grep -q 'MD-END' <<<"$out" || ! grep -q '^ok:' <<<"$res"; then
    tail -n 60 <<<"$out" | tr -d '\r'; echo "FAIL: T-LIVE-06 (did not finish)" >&2; exit 1
fi
fail=0
grep -q '^FAIL:' <<<"$res" && fail=1
if [[ $(sha256sum "$disk" | cut -d' ' -f1) == "$before" ]]; then echo "ok:   the disk is unchanged (byte for byte)"
else echo "FAIL: the disk changed" >&2; fail=1; fi
(( fail == 0 )) || { echo "FAIL: T-LIVE-06" >&2; exit 1; }
echo "PASS: T-LIVE-06"
