<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# nginx LAN reverse proxy and TLS for ALP-OS Live alpha

Config: `image/extra/etc/nginx/nginx.conf` (owner: security). It replaces
Debian's `nginx.conf` entirely: no `sites-enabled` (Debian's default site on
port 80 is never loaded), no dynamic modules, no port 80.

VERIFIED against the pinned snapshot 20261016T023245Z: nginx
1.26.3-3+deb13u7 (built with ssl, realip, http_v2; `--error-log-path=stderr`;
temp paths default to `/var/lib/nginx/*`, which do not exist on Live's
tmpfs `/var`, so the config moves all five to `/run`); Immich OpenAPI v3.3.3
(`open-api/immich-openapi-specs.json`, servers `/api`); Immich's own nginx
example (`docs/docs/administration/reverse-proxy.md` @ v3.3.3).

## What it does

- HTTPS on 443 (IPv4 and IPv6), TLS 1.3 only, session tickets off,
  `server_tokens off` (no version), `X-Powered-By` removed.
- Proxies `/api/` (with websockets) and `/.well-known/immich` to
  `127.0.0.1:2283`. Everything else is 404: no web UI on the LAN.
- Uploads: `client_max_body_size 50000m`, bodies streamed
  (`proxy_request_buffering off`, `proxy_max_temp_file_size 0`) so nothing is
  spooled into RAM; 600 s body/read/send timeouts.
- `X-Forwarded-For` is set to the real client address (a client cannot
  choose it); method-override headers are removed.
- Login rate limit 10/min per address (burst 5, then 429); 64 connections
  per address.
- Logs: only 4xx/5xx to the journal, path without query string, never
  headers or bodies.

## Delete guard (403)

Matched on nginx's normalised path, case-insensitive (Immich's Express
routes ignore case):
- every method on `/api/admin`, `/api/system-config`, `/api/system-metadata`,
  `/api/jobs`, `/api/queues`, `/api/libraries`, `/api/trash`,
  `/api/auth/admin-sign-up`;
- every `DELETE`, except `DELETE /api/sync/ack` (sync checkpoints) and
  `DELETE /api/assets/{id}/video/stream/{session}` (end a video stream);
  neither removes a photo, video or album;
- every method other than GET, HEAD, POST, PUT, PATCH, OPTIONS.

Tested 2026-10-19 with the pinned Debian nginx binary against an echo
upstream (scratch test, not in the repo): allowed: ping, well-known,
POST/PUT `/api/assets`, the two DELETE exceptions, a 2 GB streamed upload.
Refused 403: `DELETE /api/assets` (also as `/API/Assets`, `/api//assets`,
`/api/%61ssets`), `DELETE /api/albums/1`, `POST /api/trash/empty` (any case),
admin sign-up (with and without slash), `/api/admin/users` (any case,
`%2F`), `PUT /api/system-config`, `PUT /api/jobs/x`, `PROPFIND`. A POST with
`X-HTTP-Method-Override: DELETE` reached the upstream without the header;
a spoofed `X-Forwarded-For` was replaced. TLS 1.2 refused (alert 70).
QA: port this matrix to T-IMMICH-02 against the real Immich.

VERIFY (QA, real phone): the Immich app works through this allow-list
(backup, timeline, video playback) and shows a sane message on 403.

## Certificate: per-device, self-signed, made on first start (lead)

Stored on the stick's state partition so it survives a new archive and
the phone keeps trusting the same server: `/var/lib/alp/live/tls/`
(dir 0700, `server.key` 0600 root, `server.crt` 0644). Deviation from
live-immich.md section 7 (archive `.alp`): nginx must not depend on the
archive being open, and the identity belongs to the device. Never in the
repo or the image (QA: image contains no `server.key`).

Package: add `openssl` (main has 3.5.7-1~deb13u2; the image already takes
3.5.7-1~deb13u3 libraries from the security pocket, VERIFY in the manifest).

Script `/usr/lib/alp/alp-live-tls` (lead's paths), POSIX sh:

```sh
#!/bin/sh
set -eu
umask 077
d=/var/lib/alp/live/tls
mountpoint -q /var/lib/alp || { echo "state partition not mounted" >&2; exit 1; }
mkdir -p "$d"; chmod 0700 "$d"
ok() {
  [ -s "$d/server.key" ] && [ -s "$d/server.crt" ] &&
  openssl x509 -in "$d/server.crt" -noout -checkend 2592000 >/dev/null &&
  [ "$(openssl x509 -in "$d/server.crt" -noout -pubkey)" = \
    "$(openssl pkey -in "$d/server.key" -pubout)" ]
}
if ! ok; then
  openssl req -x509 -new -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 \
    -noenc -keyout "$d/server.key.tmp" -out "$d/server.crt.tmp" -days 825 \
    -subj "/CN=ALPArchive" -addext "subjectAltName=DNS:alparchive.local" \
    -addext "basicConstraints=critical,CA:FALSE" \
    -addext "keyUsage=critical,digitalSignature" \
    -addext "extendedKeyUsage=serverAuth" 2>/dev/null
  chmod 0644 "$d/server.crt.tmp"
  sync "$d/server.key.tmp" "$d/server.crt.tmp"
  mv "$d/server.key.tmp" "$d/server.key"
  mv "$d/server.crt.tmp" "$d/server.crt"
  sync "$d"
fi
mkdir -p /run/alp
openssl x509 -in "$d/server.crt" -noout -fingerprint -sha256 > /run/alp/tls-fingerprint
```

(The `openssl req` line was tested on OpenSSL 3.5.5: EC P-256, CA:FALSE,
serverAuth, SAN, 825 days.) 825 days keeps within Apple's limit for TLS
server certificates; renewal within 30 days of expiry. The phone app's
"allow self-signed certificates" setting does not pin, so a renewed
certificate keeps working (VERIFY on Android and iOS). The screen shows the
SHA-256 fingerprint from `/run/alp/tls-fingerprint`.

Unit `alp-live-tls.service`: `Type=oneshot`, `RemainAfterExit=yes`,
`Requires=`/`After=alp-live-state.service`, `Before=nginx.service`,
`UMask=0077`, `ExecStart=/usr/lib/alp/alp-live-tls`.

Drop-in `nginx.service.d/50-alp-live.conf` (lead):

```ini
[Unit]
Requires=alp-live-tls.service
After=alp-live-tls.service
[Service]
Restart=on-failure
NoNewPrivileges=yes
ProtectHome=yes
PrivateTmp=yes
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectControlGroups=yes
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6
RestrictNamespaces=yes
LockPersonality=yes
MemoryMax=128M
```

Test the hardening in the VM before enabling it; drop a line that breaks
start-up rather than weakening anything else. The admin sign-up route is
blocked at the proxy, so nginx may start before Immich is set up.

## Console gallery (8412), not in this config

If the beta serves Immich's web UI to the kiosk through nginx, add a second
server block `listen 127.0.0.1:8412; listen [::1]:8412;` without TLS, with
the same `$alp_deny` guard and `location /` proxied. Not added now because
alpd may own 8412, and a port clash would stop nginx and phone backup.

## The console's gallery on 127.0.0.1:8412 (0.2.6)

- A second server in the same nginx, bound to loopback only; nftables drops
  8412 from the network and from the Terminal user.
- Its own origin: pages of the photo service's web gallery can never call
  the console API on 8411 (alpd refuses `Sec-Fetch-Site: same-site`).
- The same delete guard as on 443; the page's own sign-in and sign-out
  routes are refused.
- No credential in the browser. For each `/api/` request nginx asks alpd
  (`auth_request` to `GET gallery/gate`) with the console's cookie; while
  the console is signed in alpd answers with the family account's key and
  nginx adds it as `x-api-key`. Everything the page sends that could choose
  another account (cookies, bearer, session and share headers) is dropped,
  and the photo service's `Set-Cookie` is hidden.
- alpd refuses the gate to every browser request (`Sec-Fetch-Site` or
  `Origin` present), so a page cannot read the key from alpd either.
- Locking the console (or the idle lock) closes the gallery at once; there
  is no session to end and no file to remove.
- The kiosk can no longer open the photo service's own port 2283 (URL
  allow-list and `frame-src`), which passed by the delete guard.
- Test: T-GAL-01 in `tests/vm/t-immich-01.sh`.
- Open for the security review: the earlier condition asked for a session
  token in an HttpOnly cookie; this design keeps the credential out of the
  browser entirely and uses the family account's existing key instead.
