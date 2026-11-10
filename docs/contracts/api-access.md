<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Contract: who can reach which API

Source of truth: `x-alp-listeners` and `security` on every operation in
`api/alpd.openapi.yaml`; checked by `tests/contracts/check-openapi.py`.

## Listeners

| Listener | Where | Serves |
|---|---|---|
| console | `http://127.0.0.1:8411`, localhost only | The shared UI and `/api/v1` for the server's own screen (kiosk). |
| gallery | `http://localhost:8412`, loopback only | Immich web UI for the console. A different host name from the console (`127.0.0.1`), so it is a different *site*: browsers do not send the console cookie with its requests, and Immich's scripts cannot read it. |
| lan | `https://<server>:443`, home network, local CA | **Beta: only the Immich reverse proxy** (the phone app's server URL), passing only the Immich API paths the mobile app needs (allow-list: stream E VERIFY). No `/api/v1` on the LAN in the beta. Admin sign-up and admin routes are blocked. Starts only after alpd has created the Immich admin. |
| socket | `unix:/run/alp/alpd.sock`, mode 0600, root | `/api/v1` for `alpctl`, the health check and tests. |
| installer | `http://127.0.0.1:8410`, localhost only, live USB | `api/installer.openapi.yaml`. |

An operation not listed for a listener returns 404 there. Immich's own port
is never reachable from the LAN (only through alpd, spec block 8).

## Console lock (spec block 8, local console rule 2)

- Locked console shows only `GET /status` (overall state and error codes).
- Unlock with the PIN (6–8 digits) or the recovery words. Attempt limits,
  shared and stored in the database: 5 free attempts, then 30 s wait,
  doubling per further failure up to 1 h; success resets. Every attempt is
  in the ledger.
- Session cookie `alp_console`: HttpOnly, SameSite=Strict, ends on lock,
  idle timeout or screen blank.
- First boot: the PIN may be set without a session only while none is set,
  and only on the console listener.
- Recovery sheet: unlocked session **and** the PIN entered again; ledger
  entry `RecoveryExported`.
- `POST /storage/unlock` (recovery words) works while locked: the recovery
  key is stronger than the PIN. Same attempt limits.

## Forged requests (CSRF)

Every state-changing request (POST, PUT, PATCH, DELETE) on the console
listener must carry `Origin: http://127.0.0.1:8411`, or it is refused. Every
POST, PUT and PATCH also has a JSON body (possibly `{}`) and must be sent as
`Content-Type: application/json` (checked by `check-openapi.py`).
Any `/api/` request a browser marks `Sec-Fetch-Site` other than `same-origin`
or `none` is refused too: the Immich gallery framed in the kiosk
(`http://127.0.0.1:2283`) is the same site, so the browser would send it the
console's cookie, also on GETs.

## Immich admin (beta)

alpd creates Immich's admin account before the LAN listener starts: API key
only (kept sealed in `paths().service_secrets`), no password login. Nobody
on the LAN can claim it, because the proxy blocks admin sign-up and admin
routes, and it is not running yet while alpd sets Immich up.

## Devices

- Revoking a device disables its login and sessions only. It never deletes
  the Immich user, albums or photos. alpd never calls Immich's user-delete
  endpoints (deletion purges a user's photos). How to block a user without
  deleting them (e.g. random password plus revoking sessions and API keys)
  is a stream E VERIFY item, with a test that photos survive a revoke.
- Beta phone method (deviation from spec block 8 rule 2, allowed for the
  beta by NEXT_TASK): alpd creates an Immich account and shows the details
  once. VERIFY (stream E/G): how the Immich app trusts the local CA.
