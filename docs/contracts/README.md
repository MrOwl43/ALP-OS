<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Shared contracts

Every workstream depends on these. They were fixed before parallel work
started (NEXT_TASK step 0.4, ADR 0004).

| Contract | Source of truth | Checked by |
|---|---|---|
| Backend traits (spec Section 5), their types, storage events, `Confirmed<T>` | `crates/alp-hw/src/lib.rs` | `cargo build`/`clippy` |
| Pool-member safety rule | `crates/alp-hw/src/safety.rs` | unit tests in that file |
| Hardware profile schema | `crates/alp-hw/src/profile.rs`, `profiles/generic-x86_64.toml` | loader tests (with the `alpd` skeleton) |
| Key types, sealing, recovery, ledger APIs | `crates/alp-crypto/src/{keys,seal,recovery,ledger}.rs` | unit tests (secrets never printed) |
| `alpd` API and who can reach it | `api/alpd.openapi.yaml`, [api-access.md](api-access.md) | `tests/contracts/check-openapi.py` (refs; every operation names its listeners; nothing on the LAN in the beta) |
| Installer API | `api/installer.openapi.yaml` | `tests/contracts/check-openapi.py` |
| Error codes | `errors/catalog.toml`, `errors/codes.lock` (append-only) | `crates/alp-core/tests/error_catalog.rs` (format; never removed; every code in any tracked file valid, defined, not retired) |
| State paths, pool mount points, service secrets, first-boot seed | `alp_core::state::paths()` (`crates/alp-core/src/state.rs`) | unit tests in that file |
| Disk layout (installed system) | `image/install-layout/` + [disk-layout.md](disk-layout.md) | `tests/contracts/check-install-layout.sh` |
| systemd target and unit names | [systemd-targets.md](systemd-targets.md) | T-BOOT-01 (boot-complete drop-in, no repart); T-BOOT-02 (reboot and rollback, stream A) |
| Installer → first-boot hand-off | [firstboot-seed.md](firstboot-seed.md), `alp_core::firstboot` | stream C/D tests |
| Beta gate | `tests/e2e/t-e2e-01.sh` | `just test-e2e` (fails until the beta exists) |

## Changing a contract

A change to any file above is a **contract change**:

1. One PR that changes the contract **and** every stream's code that uses
   it, so `main` stays green.
2. The PR title starts with `contract:` and names the contract.
3. Error codes may be **added** by any stream's PR (that is not a contract
   change); codes are never renumbered or reused.
