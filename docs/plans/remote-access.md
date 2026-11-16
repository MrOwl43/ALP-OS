<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Plan: reaching the archive away from home (alpha 0.2.7, one-week trial)

Status: 2026-11-16. **Paused by the owner (2026-11-16)** after trial step 1 (two programs and our own relay on
one machine). Nothing in the product yet. Open owner decisions are marked
**DECISION**.

## What the owner approved

Remote access through a tunnel hidden inside the ALPArchive app, built on
iroh, with our own relay as a paid service later. Off by default. The
existing locks stay inside the tunnel: the pinned server certificate and
the phone's own certificate (ADR 0017).

## Trial step 1: what was tried and what it showed

`spikes/iroh-tunnel` (iroh 1.4.1, MIT or Apache-2.0; not product code): a
"home" program that accepts iroh connections and forwards each stream to a
local TCP port, and a "phone" program that offers a local port and sends
each connection through the tunnel. With the open source relay server
(`iroh-relay` 1.4.1, same licence) on the same machine.

| Tried | Result |
|---|---|
| Dial by the home's public key, direct path | Connected; 300 MB arrived byte for byte |
| Relay only (the phone has no IP path at all) | Connected through our relay; 300 MB byte for byte |
| A phone whose key is not on the approved list | Refused by the home program before any byte reached the target |
| An endpoint the relay does not know (`access.allowlist`) | Could not use the relay at all |
| A phone removed from the list while connected | **Still connected**: the list is read when a connection starts |

Speeds on one machine say nothing about the internet and are not recorded.

What this settles:

- iroh's 1.x API does what the design needs with little code (about 120
  lines): own keys, own relay only (`RelayMode::Custom`), no use of the
  public n0 relays or their DNS (`presets::Minimal`).
- The relay has what a paid service needs built in: an allow list, a
  shared token, or an HTTP question to our own service for every endpoint
  that connects (`access.http`), and rate limits per client.
- iroh pulls in about 320 crates (alpd has 35 today). So the tunnel must
  not be linked into alpd, which runs as root.

## Design for the product (to be recorded in an ADR before building)

1. **`alp-tunnel`**, a small program of its own, as its own user with no
   rights: it holds the tunnel key, accepts only approved phone ids, and
   forwards to nginx on 127.0.0.1:443. It can read nothing else. alpd
   starts and stops it (the switch in Settings) and gives it the approved
   list. A break-in through iroh reaches only what a phone on the home
   network already reaches: the nginx door with its own two locks.
2. **Revoking a phone closes its tunnel at once**: alp-tunnel keeps its
   open connections by phone id and closes them when alpd says the list
   changed (the trial showed that reading the list at connect is not
   enough). Test: as T-PAIR-02's WebSocket check.
3. **Pairing**: the QR and the identity document gain the home's tunnel
   id and the relay address, signed by the identity key (as the
   certificate authority is today). The phone's tunnel key is made on the
   phone at pairing and its id is stored with the approved phone.
4. **Phone app**: home network first; if the home does not answer there,
   the same HTTPS connection through the tunnel (one place in the app's
   single HTTP client). Same server name, same pinned certificate, same
   phone certificate.
5. **Relay**: `iroh-relay` on one small server of ours, with
   `access.http` asking a tiny service of ours "is this id allowed?". The
   relay sees two IP addresses and encrypted packets, never a key or a
   photo. The address of the relay is a setting, so a family can run its
   own.
6. **Off by default**, one switch: "Reach my archive away from home".

## Still to verify before product code (trial steps 2 to 4)

- Static build of iroh for `x86_64-unknown-linux-musl` (how alpd and
  alpctl are built); not possible on the machine of step 1.
- Step 2, needs the owner's phone and a server on the internet: the
  Kotlin binding in the ALPArchive app on a real phone (size of the app,
  background upload, battery), from mobile data to the laptop at home;
  how often the path is direct and how often relayed; upload speed for a
  1 GB video.
- Step 3: relay cost per family per month, from step 2's numbers.
- Step 4: security review of alp-tunnel and of the pairing change.

## Decisions for the owner

- **DECISION 1**: where the trial relay runs. It needs a small server on
  the internet with a public address and a name (about the price of the
  smallest cloud server, a few dollars a month). Nothing of a family's
  data is stored on it.
- **DECISION 2** (later, with step 3's numbers): what the paid service
  covers and what stays free (for example: direct connections free,
  relayed traffic paid).

## Tests planned

- T-REMOTE-01 (VM): a phone stand-in outside the VM's network reaches the
  gallery only through the tunnel; an unapproved id is refused; a revoked
  phone's open tunnel closes within 10 seconds.
- T-REMOTE-02 (VM): with the switch off nothing listens and no packet
  leaves towards the relay.
