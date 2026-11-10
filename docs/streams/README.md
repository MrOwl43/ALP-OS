<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Workstreams (NEXT_TASK step 1)

Each stream works in its own git worktree and branch and keeps its notes in
`docs/streams/<stream>.md`. The lead reviews plans, resolves conflicts and
merges.

| Stream | Scope | Spec blocks | Notes file |
|---|---|---|---|
| A — Image and updates | reproducible build, EROFS + verity, A/B, boot counting, rollback; images per ADR 0006: **ALP-OS Live first (hardware-test image)**, ALP-OS Installer (hybrid ISO) later | 1, 2 | `a-image.md` |
| B — Storage and keys | state partition, Btrfs on LUKS, snapshots, scrub, sealing, recovery key | 3, 4, 5 | `b-storage-keys.md` |
| C — Core | `alpd`, API, ledger, health aggregation, error catalog tooling | 6, 10, parts of 8 | `c-core.md` |
| D — Installer | installer library, hardware check, disk plan, kiosk session | 7 | `d-installer.md` |
| E — Photos | Immich Quadlet units, pinned digests, migration/rollback, beta phone connection | 8 | `e-photos.md` |
| F — UI | shared web UI: installer screens, status, devices, console lock screen, RTL-ready | 2.2, 8 | `f-ui.md` |
| G — Network | nftables, networkd, mDNS, local CA and TLS | 9 | `g-network.md` |

## Rules for every stream

1. **Read first (only what you need):** `CLAUDE.md`, `docs/PROGRESS.md`,
   `docs/spec/overview.md`, `docs/spec/rules.md`, the `docs/spec/` files of
   your blocks, `docs/contracts/README.md` and the contracts your scope
   touches, ADRs 0003–0006. Do not read the whole spec.
2. **Plan first:** before code, write the plan at the top of your notes
   file: files, crates (one-line justification each, pinned, license
   checked), tests, and the VERIFY items you must research.
3. **Contracts are fixed.** Use them as they are. If a contract is wrong or
   missing something, stop that part, describe the change in your notes
   file under "Contract change requests", and continue with other work.
   Never change a contract file in a stream PR.
4. **VERIFY items** are checked against primary sources and recorded in an
   ADR or `docs/research/<topic>.md`. Never rely on memory for versions,
   flags, UUIDs or licenses.
5. **Tests:** every acceptance test of your spec block runs automatically.
   Unit tests in Rust; VM scenarios in `tests/vm/` named `T-<AREA>-NN`.
6. **Machine limits:** use `just build-image`, `tests/vm/run-vm.sh` and
   `just check`; they wait for free slots (one image build, two VMs, two
   cargo builds) via `tests/lib/locks.sh`. Never run mkosi, QEMU or heavy
   cargo builds around them.
7. **Safety:** destructive commands only against virtual disks inside the
   test harness, never the dev VM's own disks. No secrets in logs or
   commits.
8. **Commits and PRs:** small commits; identity `ALP-OS
   <noreply@alparchive.local>` (already set in the repo); PR to `main` with
   a one-line justification per new dependency. Do not merge: the lead
   merges after CI is green and, for encryption, keys, updates, rollback,
   storage, pairing or firewall, after one independent review round (a
   second round only to check blocker fixes).
9. **Notes file** (`docs/streams/<stream>.md`): plan, status, open
   problems, contract change requests, what is next. Keep it current; the
   next session resumes from it.
10. **Image changes:** never edit `image/mkosi.conf` for stream work. Add
    packages, settings and trees in your own drop-in,
    `image/mkosi.conf.d/<stream letter>-<topic>.conf` (e.g.
    `g-network.conf`), and your units under unique names in `image/units/`.
11. **Programs that run in the image** are built on this Ubuntu 26.04 VM
    but run on Debian 13 trixie, whose system C library (glibc 2.41) is
    older than the VM's. A binary linked against the VM's glibc will not
    start in the image. Before shipping one, VERIFY and record the build
    environment (for example a static `x86_64-unknown-linux-musl` build, or
    building inside Debian trixie) and test that the binary runs in the
    booted image.
12. **Hardware detection** (CPU, RAM, disks, network, TPM, Secure Boot,
    graphics) lives in `alp-hw` (module `alp_hw::inspect`), so the live
    hardware report (stream A), the installer's hardware check (stream D)
    and `alpd` share one implementation.
13. **New error codes** (adding codes is not a contract change): to avoid
    two streams taking the same number, each stream uses only its own
    range of numbers, in any area: A 100–199, B 200–299, C 300–399,
    D 400–499, E 500–599, F 600–699, G 700–799 (e.g. stream B's first new
    storage code is number 200 in the STO area). Codes below 100 belong to
    the contracts.
    Add every new code to both `errors/catalog.toml` and the end of
    `errors/codes.lock`.
14. **Report:** end with what works, what was tested (with results), what
    is open, and anything the owner must decide (plain English).
