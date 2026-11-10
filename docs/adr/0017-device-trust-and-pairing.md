<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# 0017 — Device trust and pairing: pin the server's identity key, QR plus on-screen approval

- **Status:** accepted (owner approved the 0.2.4 scope, 2026-10-30); not implemented yet
- **Date:** 2026-10-30
- **Spec reference:** block 7 (pairing; this ADR replaces its SPAKE2 step),
  block 8; ADR 0015 (Immich, the per-device CA), ADR 0016 (protection levels)
- **Research:** the device-trust research for 0.2.4 (project notes, not in the repository)
  and its English appendices A (connectivity) and B (security), with sources

## Context

Phones reach the home server over HTTPS on the LAN. Today (ADR 0015) the
device makes its own name-constrained CA and the user installs it on each
phone; the Immich app then trusts it as a user CA. That works poorly (the
owner's Pixel says "server unreachable", iOS needs a hidden trust switch) and
teaches people to install certificates. The owner ruled out plain HTTP.

Findings that decide it (research §3):

- The official Immich app trusts user-installed CAs since 2.5.4, so the
  Pixel failure has another cause (to diagnose, 0.2.4 item 1).
- Android's network security config is fixed in the APK, so a per-family
  pin must be code (`X509TrustManager`); Cronet cannot pin, so our app moves
  media to OkHttp (0.2.5).
- Apple's certificate rules apply to private CAs too: SAN, EKU serverAuth,
  at most 825 days.
- The `spake2` crate implements a draft, is unaudited and not constant time.
  A QR shown on the server's own screen is already an authenticated channel.
- Phone secure hardware supports P-256 only (no Ed25519) for TLS keys.

## Decision

1. **Pin the server's identity key, not its certificate.** K_id (Ed25519,
   per stick for Live, in the encrypted index) signs an endorsement of a
   P-256 CA (the device's existing CA, 10 years). The CA issues the server certificate (P-256, 90 days,
   renewed automatically; SAN = every current address and
   `alparchive.local`; EKU serverAuth) and device certificates. A phone
   accepts a server whose chain leads to the pinned K_id; names and
   addresses only find the server, they never establish trust.
   `/.well-known/alparchive/identity` serves K_id, the CA and the signed
   endorsement.
2. **QR, then approval on the server's screen; no SPAKE2.** The console
   (signed in) opens a 5-minute pairing window and shows
   `alparchive://pair?v=1&id=<K_id>&ca=<sha256 of CA key>&t=<128-bit token>&a=<addresses>&p=443&n=<name>&e=<family login>&exp=<time>` (`e` added for the app, T45: the non-admin family login, so the user types only the ALP password).
   The token is single use (only its hash is kept; the first request burns
   it) and rate limited. The phone generates a P-256 key in its keystore and
   sends `POST /alp/pair/v1/request` (token, device name/model/OS, CSR). The
   server answers a short code; the console shows "Approve Pixel 7? Code
   4821" and the user slides to approve. Then the server signs the device
   certificate, appends the device to a K_id-signed, append-only device
   register, and returns the certificate, the CA chain and the endorsement.
   A photographed QR is stopped by the single-use token plus the shared code
   on both screens.
3. **Enforcement in nginx, gradually.** `ssl_verify_client optional` against
   the CA and an allowlist of device certificate serials
   (`/run/alp/devices.map`). **Compatibility mode is the default in 0.2.4**:
   the official app with the installed CA keeps working (marked temporary).
   **Strict mode** (only paired devices on `/api/`, else 403) is an advanced
   switch now and the default in beta 0.3.
4. **Revocation** from Settings' device list (T33): remove from the
   allowlist, reload nginx with a short `worker_shutdown_timeout` so open
   connections and WebSockets close within seconds, delete the device's
   Immich sessions, record it in the register. Allowlist, not blocklist: a
   lost list lets nobody in.
5. **Tests without an app:** `alpctl pair-sim` is a simulated phone in Rust
   (pin check, CSR, request, approval wait, mTLS calls). VM tests T-PAIR-01
   (success, wrong pin, expired token, reused token, rejected on screen) and
   T-PAIR-02 (a revoked device is refused; its open WebSocket closes within
   10 s).

Not in 0.2.4: our Android app (0.2.5), iOS, the paper recovery key K_rec
(full install), remote access, pairing without a camera, the register in the
full signed journal (beta 0.3).

## Review of the implementation (2026-10-30)

- The allowlist and the mode live in /run: `alpctl live pair write-map`
  writes them from the register before nginx starts and on every reload,
  and nginx is strict when the mode file is missing (fail closed).
- Device certificates are valid 825 days until phones renew their own
  (our app, 0.2.5); the allowlist decides access, so a removed phone is
  refused whatever its certificate says. (An expired client certificate
  makes nginx answer 400 everywhere.)
- In compatibility mode a removed phone can still come in the temporary
  way (no certificate, the installed CA) with its Immich login: removal
  locks a phone out fully only in strict mode, and Settings says so.
  Deleting its Immich sessions comes with per-device Immich logins (0.2.5).
- The CSR check reads the key itself (P-256 only), device names drop
  bidirectional and invisible characters, one lock guards the register,
  its signing files and the allowlist.

## Consequences

- Certificates can be renewed and addresses can change without re-pairing.
- No third party (DNS service, public CA) sits between phone and home.
- Until our app exists, compatibility mode keeps the installed-CA path; its
  weaker properties are documented and it is labelled temporary.
- A stolen Live stick is a separate identity (its own K_id); copying the
  home server's K_id to a stick is never done.
- Unprotected Live sticks (ADR 0016 levels 1 and 2) keep K_id in their plain
  index: anyone with the stick can impersonate that stick's server. The UI
  says so with the other UNPROTECTED warnings.
