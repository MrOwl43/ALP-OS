<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# ALP-OS Live alpha — team plan (owner request 2026-10-18)

Goal: one ALP-OS Live image (`out/alp-os.raw`) a non-technical person writes
to a USB stick and uses to: check the hardware, join the network (wired or
Wi-Fi), choose where to keep the encrypted archive, connect a phone and back
up photos to Immich over the home network, and start a sync chain (seed =
chain only, like Brave Sync). Polished graphical UI in the style of Apple's
2016–2018 setup assistant, on the computer's own screen (kiosk). Alpha
quality. Owner decisions: `~/Downloads/ALPArchive-Project/decisions.md`,
ADR 0005–0009. Plans: `live-archive.md`, `live-immich.md`, `live-ux.md`.

## Roles and file ownership (one checkout, branch `stream-a/live-alpha` (renamed from stream-a/live-hwtest))

Nobody edits another role's files; ask the coordinator instead. Nobody
commits, pushes, switches branches or creates worktrees: the coordinator
reviews and commits. Machine limits (6 CPUs, ~7 GB RAM): image builds and
VMs only through `just` / `tests/vm/run-vm.sh` (they queue on locks).

| Role | Owns |
|---|---|
| Lead developer | `crates/**`, `image/**` (except files listed for security), `justfile`, `api/**`, `docs/adr/0010+`, `docs/plans/live-alpha-*.md` |
| UX | `ui/**`, `docs/plans/live-ux.md` |
| Security | `image/extra/etc/nftables.conf`, `image/extra/etc/nginx/**`, `image/mkosi.conf.d/g-*.conf`, `docs/security/**` |
| QA | `tests/**` except `tests/vm/run-vm.sh` (shared: QA only), `docs/qa/**`, builds into `out/qa/` only (`just _build out/qa`) |

## Shared interface: the console API

The UI talks only to `alpd` on `http://127.0.0.1:8411/api/v1` (contract
`api/alpd.openapi.yaml`, access rules `docs/contracts/api-access.md`).
Endpoints the contract lacks for Live are added by the lead as
`/api/v1/live/...` and documented in `docs/plans/live-alpha-api.md` FIRST
(before code), so UX and QA can build against it. Minimum set:
`GET live/state` (hardware checks, network lines, archive state + progress,
Immich state, chain state), `GET live/wifi/networks`, `POST live/wifi/connect`,
`GET live/archive/candidates`, `POST live/archive/create`,
`POST live/phone/pair` (server URL + QR + family account),
`POST live/chain/create`, `POST live/chain/join`, `POST live/power/off`.
