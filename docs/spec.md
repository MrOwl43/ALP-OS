# ALP-OS v1 — Implementation Specification for Claude Code

Version 0.5 · Working document · Audience: Claude Code and human reviewers · Scope: the base operating system and the v1 product (phone backup) · New in 0.5: owner decisions of 2026-10-17 applied (ADR 0005): two separate images (ALP-OS Live as a raw USB image, the current focus; ALP-OS Installer as a hybrid ISO, later), 1 GB ESP, 64 GB single-disk minimum, 2 GB system image limit, Immich inside the system image, beta managed from the server screen only; technical wording corrections in ADR 0004 (where this spec and ADRs 0004–0005 differ, the ADRs win). New in 0.4: graphical installer, local console on the server's own screen, one shared UI for every screen, numbered error codes, AGPL-3.0-or-later license

## 0. How to use this document

- Work one building block at a time, in the order given in Section 9. Do not start block N+1 until the acceptance tests of block N pass in CI.
- Before writing code for a block: read the block, write a short plan (files, crates, dependencies, test harness), and record non-obvious choices as ADRs in `docs/adr/`.
- Prefer existing, mature components over new code. If this spec names a tool, use it. Do not write a replacement for it.
- Anything marked **VERIFY** must be checked against current upstream documentation or the real Debian Stable package versions. Do not rely on memory for versions, flags or partition type UUIDs.
- If this spec contradicts itself or reality, stop and report the conflict. Do not resolve it silently.
- Every block ends with acceptance tests. A block is done only when those tests run automatically in CI.

## 1. Product context

ALPArchive is an open source, sovereign personal archive. ALP-OS is its operating system: an immutable appliance Linux that turns an ordinary PC (later, a dedicated ALP device) into a private archive server for a household. It is a standalone operating system that takes over the machine, not an app inside another OS (a lighter app-style edition may come later). The first product is automatic backup of household phones to this server, with a friendly Google Photos style gallery served by the server itself. Complete PC backup is the second product. The user is non-technical and must never be asked to make a technical decision. Advanced users get an explicit opt-in advanced mode where they can review and override automatic choices.

Guiding principles, in priority order when they conflict:

1. **Stability and reliability above everything.** The archive must keep working and stay available for years, and problems must be easy to recover from. Without this the project has no reason to exist.
2. Absolute simplicity for the non-technical user.
3. Privacy and security by construction, adapted to the hardware: data is always encrypted at rest, keys belong to the user, the project never holds a key, and nothing leaves the LAN in v1.
4. Build on mature, proven open source projects instead of writing new code, and never take their updates untested (Section 2.1).
5. Runs on ordinary and older hardware: use TPM2, Secure Boot and other hardware features when present, never require them.
6. Foundations that later features (distributed backup, relay, AI search, ARM, ALP-Box) can build on without rewrites.

### 1.1 Locked strategic decisions

- **First hardware target:** x86_64 PCs that the user dedicates to ALP-OS. The dedicated ALP-Box (ARM) comes later.
- **v1 product:** automatic backup of household phones (photos and videos) to the home server over the local network, done perfectly and simply, plus a gallery app to browse them. **Next product:** complete PC backup.
- **Standalone OS:** ALP-OS installs as the machine's operating system and erases it after explicit confirmation. A lite edition that runs inside an existing OS is a possible later product, not v1.
- **Hardware-adaptive installation:** the installer inspects the machine (UEFI, TPM2, Secure Boot, AES-NI, RAM, disks) and selects the configuration automatically. Advanced users may review and override these choices.
- **Graphical setup and a local console.** Installation is a friendly graphical flow on the machine's own screen (mouse, keyboard or touch), as simple as setting up a new consumer device. After installation, attaching a screen to the server opens a graphical local console for status, settings, re-pairing devices and browsing the archive. The phone is never the only way in: if a phone is lost or cannot connect, everything can be checked and fixed on the server itself. Headless installation remains a later front end.
- **The chain:** the household's set of trusted devices joined to its server. Installation ends with a QR code on screen; scanning it with a phone joins that phone to the chain. Every join and revocation is recorded in the signed ledger (block 6), and any device can be revoked. Other household devices join the same chain later.
- **Two protection tiers** (block 5): Standard, the default, where the user has nothing to remember, and Archive encryption, an advanced opt-in protected by the user's seed phrase.
- **Photo app: integrate Immich** as the phone backup engine and gallery instead of building one (block 8), under the licensing constraints stated there.
- **Foundations are built now** even though their features ship later: cryptographic identity, key hierarchy, signed audit ledger, A/B updates, hardware abstraction traits.

### 1.2 In scope for v1

Two bootable images built from the same system: ALP-OS Live (raw USB image; runs entirely from the stick, never writes to internal disks, keeps its data pool on the stick; the first deliverable) and ALP-OS Installer (hybrid ISO for DVD or USB; installs onto the computer's disk; later), both with hardware detection; immutable A/B system with automatic rollback; always-encrypted Btrfs data pool with scrub, snapshots and disk expansion; protection tiers and key recovery; first-boot setup and QR pairing into the chain; LAN-only phone backup and gallery through an integrated, pinned Immich; local web management UI; health monitoring and notifications; minimal firewall and network stack. PC backup ingestion is prepared but ships only after phone backup is proven stable.

### 1.3 Explicitly out of scope for v1 (do not implement)

- Distributed backup to friends and family (Accountabilibuddy distribution), erasure coding, fragment placement, peer verification.
- Relay service and any access from outside the home network. No inbound ports exposed to the internet, no UPnP.
- Machine-learning features of the photo app (face recognition, smart search) are off by default, and models the project may not redistribute are never bundled (block 8). Any other AI indexing.
- ZFS. ARM, phones, ALP-Box hardware enablement. Legacy BIOS boot. 32-bit CPUs.
- Clustering of multiple boxes. Third-party app store.
- Built-in VPN engine. (Advanced users may configure WireGuard themselves in advanced mode.)

## 2. Global engineering rules

1. **Language and tooling.** All new daemon and tool code is Rust (stable toolchain pinned via `rust-toolchain.toml`). Use `tokio`, `serde`, `rusqlite` (bundled), `tracing`, `axum` with `rustls`, `clap`. Shell is allowed only for thin glue in image build hooks and must pass `shellcheck`.
2. **No unsafe Rust** outside narrow FFI wrappers, each justified in an ADR. CI runs `cargo clippy -- -D warnings`, `cargo fmt --check`, `cargo test`, `cargo deny check` and `cargo audit`.
3. **Dependencies are a security surface.** Adding a crate requires a one-line justification in the PR description. Prefer crates that are widely used and maintained. Pin versions with `Cargo.lock` committed.
4. **Cryptography: never invent primitives or protocols.** Use `ring` or RustCrypto crates, `ed25519-dalek`, `sha3`, `zeroize`. Use an established PAKE (`spake2`) for pairing. Secrets are wrapped in zeroizing types, never logged, never written to disk in plaintext except where a block explicitly says so.
5. **Hardware isolation.** Code above the abstraction layer (Section 5) must not assume UEFI, TPM, Btrfs or any specific hardware. Only backend implementations may.
6. **Single point of access.** Only `alp-storage` may invoke `btrfs`, `cryptsetup`, `mkfs`, `smartctl`. Only `alp-crypto` handles key material. Enforce with a CI grep check.
7. **Safe defaults, no questions.** Any decision a non-technical user could not evaluate is made automatically. The UI shows a result and a recommendation.
8. **Destructive operations** (partitioning, formatting, disk removal, key rotation) require explicit confirmation from the user and are always written to the ledger.
9. **No telemetry.** The only outbound connections allowed are to the signed update server, and to nothing else.
10. **Observable and testable.** Structured logs through `tracing` to journald. Every block has a VM-based acceptance test. Failure injection is a first-class test tool.
11. **Reproducible builds.** Same inputs produce the same image hash (see block 1).

### 2.1 Upstream component policy

- Prefer integrating mature, widely used projects over writing new code. Every integrated component (Debian packages, containers such as Immich, Rust crates) is listed with its version and license in `docs/components.md`.
- **No automatic upstream updates.** Every component is pinned to an exact version (Debian snapshot mirror, container image digest, committed `Cargo.lock`). A new upstream version enters a release only after it passes the full VM test suite and a beta period on test machines. On users' machines, only signed ALP-OS images update, and only through block 2.
- Each component update is one reviewable change that summarises the upstream changelog and tests its migration steps, including rollback to the previous image.
- A component with a database or on-disk format (Immich and its PostgreSQL) must survive rollback: snapshot its data before any migration and restore it if the image rolls back. **VERIFY** Immich's migration and downgrade behaviour and record the procedure in an ADR.
- Licenses are reviewed before a component is added. Anything restricted for redistribution or commercial use is not bundled.

### 2.2 User experience rules

- **The server is the family's photo album, not a computer.** Every screen speaks in plain words about photos, devices and safety, never about partitions, services or certificates. Technical detail is available behind an explicit "Technical details" control.
- **One UI for every screen.** A single web UI codebase (`ui/`), served by `alpd`, is used by the local console, by a laptop browser on the home network, and by the phone app. Layouts differ (large screen and phone), but the API, the data, the wording and the error codes are identical, so the code is shared. VERIFY in an ADR how the phone app reuses it when the mobile phase starts.
- **Numbered error codes.** Every problem the user can see has a stable code of the form `ALP-<AREA>-<NNN>` (areas: `INST`, `BOOT`, `UPD`, `STO`, `KEY`, `NET`, `PAIR`, `APP`, `PWR`, `HW`). One source file (`errors/catalog.toml`) defines for each code: severity, a plain-language title, what the user should do, and technician detail with the logs to check. It generates `docs/errors.md` and the UI strings. Codes are never reused or renumbered. The UI always shows the plain message and action first, then the code with a copy button, so the user can search for it or give it to a technician. The same code is written to the journal (structured field `ALP_ERROR`).
- **Ask nothing the system can find out.** Disks, protection level and capacity are decided automatically. Language defaults to English and can be changed on the first screen. Time zone defaults to UTC and is taken from the first paired phone, changeable in settings (no external geolocation lookup, see rule 9).
- **Translation-ready from day one.** All user-facing text lives in translation files; layouts must work right-to-left even though v1 ships in English.
- **Accessible and touch-friendly:** large targets, readable contrast, keyboard navigation.

## 3. Hardware baseline for v1

- Architecture x86_64 (64-bit only). UEFI firmware required. Legacy BIOS is unsupported in v1 (the installer must detect it and explain clearly).
- RAM: 4 GB minimum for the base system, which must stay within 2 GB at idle with photo-app machine learning off. With the integrated photo app, recommend 8 GB (**VERIFY** Immich's current minimum and recommended RAM and set the installer thresholds from it). The installer measures RAM and enables or disables optional features accordingly.
- System disk: separate from data disks, 32 GB or larger (SSD, small SATA, or even a good USB SSD). Single-disk machines need at least 64 GB, and the installer says in plain words how many photos fit. If the machine has only one disk, the installer partitions it into system and data areas and tells the user that protection is limited.
- TPM2 and Secure Boot: optional. Use them when present. Behavior without them is defined below and must be shown honestly to the user.
- AES-NI: optional. Detect it at install time and choose the LUKS cipher accordingly (block 5).
- Network: wired Ethernet. Wi-Fi only if needed by the reference hardware, and not a v1 requirement.
- Display: a screen with keyboard and mouse (or touch) is required during installation. Afterwards it is optional: the server runs without one, and attaching one at any time opens the local console. Use the firmware framebuffer or common open GPU drivers; never require a specific GPU.

## 4. Repository and workspace layout

```
alp/
  Cargo.toml                 # workspace
  rust-toolchain.toml
  LICENSE                    # AGPL-3.0-or-later
  crates/
    alp-core/                # daemon (alpd), event loop, API server, orchestration glue
    alp-storage/             # StorageBackend trait, BtrfsBackend, disk discovery, SMART
    alp-crypto/              # identity, key hierarchy, sealing, recovery, ledger primitives
    alp-network/             # mDNS announce, firewall rules, TLS/CA, device pairing transport
    alp-orchestrator/        # service lifecycle, Podman/Quadlet management, health aggregation
    alp-installer/           # installer logic (library) + local web front end for the graphical installer
    alp-hw/                  # Hardware Profile loader and backend traits (Section 5)
    alpctl/                  # CLI talking to alpd over a unix socket (technicians, tests)
  ui/                        # the one shared web UI: installer screens, console, management
  errors/
    catalog.toml             # single source of all user-facing error codes (Section 2.2)
  image/
    mkosi.conf, mkosi.*      # image definition (block 1)
    sysupdate.d/             # systemd-sysupdate transfer files
    repart.d/                # systemd-repart partition definitions
    units/                   # systemd units, Quadlet files, nftables ruleset, kiosk session
  profiles/
    generic-x86_64.toml      # the only v1 hardware profile
  tests/
    vm/                      # QEMU harness and scenarios (Section 8)
  docs/adr/                  # architecture decision records
```

The daemon is a single binary `alpd` built from `alp-core` that links the other library crates. `alpctl` and the installer are separate binaries.

## 5. Abstraction layer: traits and hardware profile

Four traits isolate hardware and platform differences. v1 ships exactly one implementation of each (two for key sealing). Future hardware adds implementations, not changes to callers.

```rust
pub trait BootBackend {            // v1: UefiSystemdBoot
    fn current_slot(&self) -> Slot;
    fn stage_update(&self, image: &VerifiedImage) -> Result<()>;
    fn mark_good(&self) -> Result<()>;
    fn rollback_pending(&self) -> bool;
}
pub trait KeySealBackend {         // v1: Tpm2Seal, PassphraseOrKeyfileSeal
    fn available(&self) -> SealCapability;
    fn seal(&self, secret: &Secret) -> Result<SealedBlob>;
    fn unseal(&self, blob: &SealedBlob) -> Result<Secret>;
}
pub trait StorageBackend {         // v1: BtrfsBackend
    fn discover_disks(&self) -> Result<Vec<DiskInfo>>;
    fn plan_pool(&self, disks: &[DiskInfo]) -> Result<PoolPlan>;
    fn create_pool(&self, plan: &PoolPlan, key: &PoolKey) -> Result<PoolId>;
    fn open_pool(&self, id: PoolId, key: &PoolKey) -> Result<MountedPool>;
    fn propose_expansion(&self, id: PoolId, new_disk: &DiskInfo) -> Result<ExpansionPlan>;
    fn apply_expansion(&self, plan: &ExpansionPlan) -> Result<()>;   // needs user confirmation token
    fn replace_disk(&self, id: PoolId, old: DiskId, new: &DiskInfo) -> Result<()>;
    fn snapshot(&self, id: PoolId, policy: &SnapshotPolicy) -> Result<()>;
    fn scrub(&self, id: PoolId) -> Result<ScrubHandle>;
    fn health(&self, id: PoolId) -> Result<PoolHealth>;
}
pub trait PowerBackend {           // v1: GenericPcPower (NUT-aware if a UPS is detected)
    fn status(&self) -> PowerStatus;
    fn on_event(&self, handler: Box<dyn Fn(PowerEvent) + Send>);
}
```

A hardware profile is a declarative TOML file selecting the implementation for each trait and setting numeric defaults (ARC-style limits do not apply to Btrfs, but scrub throttling, snapshot retention and container quotas do). v1 has only `generic-x86_64.toml`. Never scatter `if hardware == ...` checks in code.

## 6. System overview

**Boot.** UEFI loads systemd-boot, which picks a Unified Kernel Image (UKI) using boot counting. The UKI carries kernel, initrd and command line, including the dm-verity root hash. The initrd assembles the verified read-only root. systemd starts services and reaches `alp-healthy.target`, after which the slot is marked good. If the counter reaches zero first, the previous slot boots automatically.

**Storage after boot.** `alp-storage` unlocks and mounts the data pool after boot, not from the initrd. A pool that cannot be opened never blocks boot and never fails the health target. The system stays reachable and reports `storage unavailable`.

**Install.** A bootable USB image starts a minimal graphical kiosk session that shows the installer screens from `alp-installer`'s local web front end. The user sees a welcome screen, a plain hardware check, the disks with one recommendation, one clear confirmation, progress, and "done". After the first reboot the screen shows the first-boot steps and a QR to join the first phone. **Local console.** Whenever a screen is attached, the same kiosk stack shows the shared UI from `alpd` on localhost.

**Update.** `systemd-sysupdate` fetches signed images from a static HTTPS server, writes the inactive slot, and the box reboots into trial mode in a quiet window.

## 7. Building blocks

Each block lists: Goal, Decisions, Constraints, Interfaces, Acceptance tests, Out of scope.

### Block 1 — Image builder

**Goal.** Produce reproducible, signed images and the Live and Installer images from Debian Stable.

**Decisions.** Use `mkosi` (not live-build, not custom scripts). Outputs: UKI, root filesystem image (EROFS preferred, SquashFS acceptable) with a dm-verity hash partition image, the ALP-OS Live raw USB image now and the ALP-OS Installer hybrid ISO later, system image of at most 2 GB, update bundle (for sysupdate), SBOM. **VERIFY** the Debian Stable release, its kernel, and that `systemd` is recent enough for boot assessment, `systemd-sysupdate`, `systemd-repart` and UKI support; if not, use the newest supported source (for example backports) and record it in an ADR.

**Constraints.** Pin package versions through a snapshot mirror so rebuilds are identical. Build with `SOURCE_DATE_EPOCH`. Architecture is a build parameter (CI also builds aarch64 as a smoke test, no full image). Signing keys never live in the repo; CI signs with injected secrets. Include only packages needed: systemd, btrfs-progs, cryptsetup, nftables, avahi-daemon, podman, samba (in container only), smartmontools, nut-client (optional), ca-certificates, firmware needed for common NICs, storage controllers and display adapters, and the kiosk stack: a Wayland kiosk compositor (for example `cage`) plus a lightweight kiosk browser engine, fonts with wide script coverage (including right-to-left scripts). **VERIFY** the kiosk compositor and browser choice in an ADR (Debian availability, security update cadence, memory footprint, touch support); no full desktop environment.

**Interfaces.** `just build-image` produces `out/` containing all artifacts and a manifest with hashes.

**Acceptance.** Two clean builds of the same commit give identical artifact hashes. The USB image boots in QEMU with OVMF. Image size and idle memory are recorded and checked against budget.

**Out of scope.** ARM images, ZFS modules, a package manager on the target.

### Block 2 — Boot chain, partitions and A/B updates

**Goal.** Safe boot and updates that cannot brick the machine.

**Decisions.** Disk layout via `systemd-repart` using Discoverable Partitions (**VERIFY** the exact partition type UUIDs and the verity pairing rules): `ESP` (1 GB, FAT32), `root-a` + `root-a-verity`, `root-b` + `root-b-verity`, `state` (small, persistent). Bootloader `systemd-boot` with Boot Loader Specification Type 1/2 entries and **boot counting** (`+3` tries). Health is the systemd target `alp-healthy.target`; `systemd-bless-boot` marks the slot good. Updates through `systemd-sysupdate` with signed `SHA256SUMS`. Do not write a custom watchdog or updater.

**Constraints.** Secure Boot: sign the UKI and support enrollment of project keys when firmware allows. When Secure Boot is off, the system still boots, and the UI shows a one-line, non-alarming note that boot-time authenticity is not enforced. dm-verity protects root integrity in both cases. `alp-healthy.target` requires: `alpd` responding on its socket, networking up, time sync attempted. It must **not** require the data pool.

**Interfaces.** `BootBackend` (Section 5). `alpd` exposes update status and last update result to the UI. Updates are applied in a configurable maintenance window (default 03:00 local) or on user request, and only when no backup upload is mid-flight (otherwise defer).

**Acceptance (VM).** Update to a good image is blessed after reboot. A deliberately broken image (service fails) rolls back to the previous slot automatically. Power cut (QEMU kill) during write of the inactive slot leaves the active slot untouched and bootable. A tampered root image fails verity and does not boot.

**Out of scope.** Delta updates, multiple update channels, other bootloaders.

### Block 3 — State partition and configuration

**Goal.** One small, durable place for everything that is neither OS nor user data.

**Decisions.** `state` holds: node identity, wrapped keys, `alpd` SQLite database, ledger, configuration, pool metadata (`device UUIDs`, profile). `/etc` is an overlay over the read-only root with its upper layer on `state`. Canonical path `/var/lib/alp`. State is mounted early with `ProtectSystem=strict` semantics for all services except `alpd`.

**Constraints.** Write amplification matters on old SSDs and USB drives: batch writes, use SQLite WAL, avoid chatty logging to `state` (journald is volatile-capped or on its own small area). Daily encrypted backup of `state` into the data pool (see block 5 for the key) so loss of the system disk is recoverable.

**Interfaces.** `alp_core::state::paths()` is the only source of paths. No hard-coded paths elsewhere.

**Acceptance.** Replacing a slot does not change the state content. Wiping `state` and restoring from the pool backup using the recovery key yields the same node identity and a mountable pool (VM test).

**Out of scope.** Syncing state between nodes.

### Block 4 — Storage: encrypted Btrfs pool

**Goal.** A pool that protects user data against bit rot, survives disk failure when redundancy exists, and can grow easily.

**Decisions.** Btrfs on top of LUKS2, one LUKS container per data disk, all unlocked with the same pool key. Everything goes through `StorageBackend`/`BtrfsBackend` in `alp-storage`.

- Automatic planning: one disk: metadata `dup`, data `dup` (self-healing for bit rot, effective capacity is half; show the effective capacity and say it is partial protection and not protection against disk failure; provide an advanced setting to switch data to `single`). Two or more disks: `RAID1` (data and metadata). Allowed profiles: `dup`, `single` (only by explicit advanced setting), `RAID1`, `RAID1c3`, `RAID10`. **RAID5/6 are forbidden.**
- Subvolumes: `@archive` (user data), `@snapshots` (read-only snapshots), `@system-backup` (encrypted state backups). Mount with `compress=zstd:3` only if CPU budget allows (decide by benchmark on the reference machine, record as ADR) and `noatime`.
- Maintenance: monthly scrub (throttled, resumable across reboots), automated read-only snapshots (hourly 24, daily 14, weekly 8, tunable constants in the profile) as protection against deletion and ransomware, SMART monitoring via `smartmontools` with plain-language alerts.
- Expansion: `propose_expansion` computes the resulting usable capacity and shows it first. `apply_expansion` runs `device add` followed by a throttled background `balance`. Replacing with a larger disk uses `replace` then `resize`. No expansion without explicit confirmation. Disks of different sizes are allowed and the UI explains usable capacity honestly.
- USB disks are supported as data disks. Disconnect during operation is a normal event: the pool goes read-only or degraded, an alert is raised, nothing crashes, and reconnecting recovers automatically when safe.

**Constraints.** Parse command output only from machine-readable forms (`--json`, `-b`, `/sys`) wherever available. **VERIFY** `btrfs-progs` features in the shipped version. Never run `btrfs check --repair` automatically.

**Interfaces.** The `StorageBackend` methods in Section 5, plus events: `PoolDegraded`, `DiskMissing`, `ScrubFinished{errors_found, errors_corrected}`, `CapacityLow`.

**Acceptance (VM, virtual disks).** Create pool on 1 disk and on 2 disks. Pull one disk of a RAID1 pool: data stays readable, pool reports degraded. Reinsert or replace the disk and rebuild completes with no data loss. Corrupt a block on one mirror copy, run scrub, error is detected and repaired. Add a disk of a different size and confirm reported capacity matches the plan.

**Out of scope.** ZFS, RAID5/6, dedup, send/receive between nodes, quota per user.

### Block 5 — Encryption and key management

**Goal.** Data at rest is unreadable without user-owned keys, and a non-technical user can recover from hardware failure.

**Key hierarchy (all generated on first boot by ****`alp-crypto`****).**

- **Identity key** (Ed25519): the node identity and root of trust for the ledger and device certificates. Reserved so that future user and device identities attach to it.
- **Pool key** (random 512-bit): unlocks every LUKS container (stored in a LUKS keyslot as a keyfile).
- **Recovery key** (random 256-bit): added as a second LUKS keyslot and used to encrypt the `state` backup (derive keys with HKDF and distinct contexts). Presented as a seed phrase (word list; **VERIFY** a standard wordlist crate such as BIP39) plus QR. In the Standard tier it is escrowed automatically to the admin phone in the chain (encrypted for that device) and offered as a printable recovery sheet at the end of installation; installation does not block on it. In the Archive encryption tier the user must confirm they saved it (re-entry check of selected words) before the flow finishes.
- **Wrap key:** protects identity and pool keys stored on `state`.

**Unattended unlock (required, the box must come back after a power cut).**

- TPM2 present: the wrap key is sealed to TPM2 with a PCR policy that tolerates normal updates (**VERIFY** a policy based on signed PCR values or on a stable PCR set so that A/B updates do not break unsealing).
- No TPM: the wrap key is a keyfile on `state`. This protects against removed or resold data disks but not against theft of the whole machine. The UI states this plainly in one sentence. An optional hardened mode (passphrase or USB key required at each boot) exists but is never the default.

**Protection tiers.**

- **Standard (default).** Data disks are always encrypted, but the user has nothing to remember: the wrap key is sealed in TPM2 when present, otherwise kept as a keyfile on `state`. The server boots and unlocks unattended. This protects against removed, failed or resold disks, not against theft of the whole machine; the UI says so in one sentence. If the machine dies: install ALP-OS on new hardware, attach the data disks, and restore with the recovery key from the admin phone or the printed sheet.
- **Archive encryption (advanced, opt-in).** The archive is additionally protected by a key derived from the user's seed phrase, required to unlock (passphrase or USB key). Before enabling it, the UI states plainly that without the seed phrase nobody, including the project, can recover the data.
- **Premium recovery (future).** A paid convenience layer may make recovery easier, for example guided recovery between the user's own devices or shared among family members, but it must never give the project the ability to decrypt user data. No key escrow on project servers.

**LUKS parameters.** LUKS2. Cipher chosen at install: `aes-xts-plain64` if AES-NI is detected, otherwise `xchacha12,aes-adiantum-plain64` (**VERIFY** kernel support). Keyfile-based slots need no expensive KDF. The recovery slot uses a normal argon2id setting.

**Constraints.** Zeroize secrets after use. No key material in logs, crash dumps or the ledger (ledger records events, not secrets). Key rotation (`rotate_pool_key`) re-wraps keyslots and must be atomic and resumable. Provide `export_recovery` only behind explicit confirmation and ledger entry.

**Interfaces.** `alp-crypto`: `generate_identity`, `seal`, `unseal`, `rotate_pool_key`, `create_recovery`, `verify_recovery`, `backup_state`, `restore_state`.

**Acceptance (VM with swtpm).** First boot unlocks automatically after reboot with TPM and without TPM. Replacing the virtual TPM, the user can regain access with the recovery key only. Fresh scan of disk images and logs finds no plaintext key material. State restore test from block 3 passes.

**Out of scope.** Shamir secret sharing, key escrow with friends, multi-user key delegation (design hooks only, see Section 11).

### Block 6 — Audit ledger

**Goal.** A tamper-evident local record of important events that later becomes the base of the Accountabilibuddy protocol.

**Decisions.** Append-only table in SQLite on `state`. Each entry has `seq`, `timestamp`, `event_type`, `payload` (canonical serialization, defined and versioned), `prev_hash`, `hash = SHA3-256(prev_hash || canonical_entry)` and an Ed25519 `signature` by the identity key. The structure must be extendable to a Merkle tree with checkpoints later. Mirror the ledger to the pool daily.

**Events in v1.** First boot, device pairing and revocation, service enable/disable, update applied or rolled back, pool create/expand/replace, key rotation, recovery export, advanced mode toggled, admin login.

**Constraints.** Ledger events never contain secrets or file names or file contents. Verification runs at every boot and on demand, and a broken chain raises a clear alert but does not halt the system.

**Interfaces.** `alp-crypto::ledger::{append, verify, export_range}`.

**Acceptance.** Modifying any past row in the database makes `verify` fail and report the first bad sequence number. Concurrent appends keep a valid chain.

**Out of scope.** Cross-node verification, public anchoring, peer sync.

### Block 7 — Installer, first boot and pairing

**Goal.** From USB stick to first backup in minutes, with a screen but without technical knowledge.

**Installer.** The USB image boots into the kiosk session, which opens the installer UI served on localhost by `alp-installer`. The library crate holds all logic (hardware check, disk discovery, plan, repart, image copy, first-boot seed); the graphical screens are a thin front end built from the shared `ui/` code, so a headless or remote front end can be added later without changing logic. Screens, in order: **Welcome** (language picker, defaults to English) → **Your computer** (hardware check in plain words: memory, disks, security chip; if the machine cannot run ALP-OS, for example legacy BIOS only, explain kindly with an error code and stop) → **Where to install** (disks shown with model, size and a simple picture; the recommended layout preselected; a clear summary of exactly what will be erased) → **Confirm** (an unmistakable confirmation, such as typing a word or press-and-hold; never a single accidental click) → **Installing** (progress, no questions) → **Done** (remove the USB stick, restart). Never touch a disk the user did not confirm. Every failure shows a plain message, the action to take and an `ALP-INST-*` code.

**First boot.** Generates keys (block 5), creates the pool (block 4), creates the recovery key, creates the local CA and server certificate, and announces `alparchive.local` via mDNS (`avahi`). On the screen, in the same graphical style: set a short **console PIN** (protects the local console), offer the **printable recovery sheet** (Standard tier: optional; Archive encryption tier: required with re-entry check, see block 5), and show a **QR and short pairing code** to join the first phone. The QR contains the address and the CA fingerprint.

**Pairing protocol.** Use SPAKE2 (`spake2` crate) with the short code to authenticate the first device against man-in-the-middle, then exchange public keys. The server issues the device a certificate signed by the node CA; later connections use mutual TLS. The first paired device becomes the admin device. Pairing codes are single use, expire in minutes, and rate-limited. VERIFY the exact message flow with a written threat model in an ADR before implementing.

**Acceptance (VM).** Scripted install through the installer library on virtual hardware with 1 disk and with 2 disks, no manual input apart from the confirmation. One automated end-to-end test drives the graphical screens in the VM (screenshot or accessibility-tree based; **VERIFY** the tool in an ADR). An unsupported machine (legacy BIOS VM) shows the right `ALP-INST-*` code and changes nothing. A simulated client pairs using the code and fails with a wrong code, an expired code or a reused code. Installing never modifies a disk that was not selected.

**Out of scope.** Headless install, BLE or Wi-Fi AP provisioning, shipping languages other than English (the translation and right-to-left layer is in scope and must work).

### Block 8 — Backup ingestion and application runtime

**Goal.** Receive backups from household devices safely and reliably, and run supporting services isolated.

**Phone backup and gallery (Immich).** Immich provides the mobile app, the gallery and the user-side file management only. Connectivity, device identity and access control are ALPArchive's own: the chain, pairing (block 7) and mutual TLS. Immich runs as pinned rootless Podman containers shipped inside the signed system image, so it is updated and rolled back together with the system under Quadlet (server, its PostgreSQL and Redis; the machine-learning container only when enabled), with its library on `@archive`, and is never reachable except through `alpd`. Integration rules: (1) Use Immich unmodified wherever possible; publish any patches under its AGPL-3.0 license. (2) **Do not use Immich's own accounts and login as the user-facing access system.** `alpd` sits in front of Immich as the gateway: it admits only devices in the chain, and joining the chain provisions the Immich client automatically, so the user never types a server address, account or password. **VERIFY** how to do this with the least modification to the Immich app and server (for example a gateway that maps chain identities to Immich sessions or API keys, or the OAuth/OIDC hook) and record it in an ADR before implementing. (3) **Face recognition licensing:** Immich's default face models come from InsightFace, and the permission InsightFace gave Immich does not extend to redistribution or commercial use by third parties. ALP-OS images and ALP hardware must not bundle these models, and face recognition is off by default. Either adopt a permissively licensed open model or obtain written permission or a commercial license from InsightFace; get a legal review and record the decision in an ADR. (4) Deleting on the phone never deletes on the server (**VERIFY** the Immich settings that guarantee this). (5) No custom upload protocol in `alp``d` for v1: `alpd` manages and guards Immich, it does not replace it.

**Ingestion for PCs**** (second product, after phone backup is stable).** Samba in a rootless Podman container, LAN only, SMB3 minimum, signing required, guest access disabled, with `nftables` redirecting 445 to the container's high port. **VERIFY** when this phase starts whether an established open source backup client gives more reliable complete-PC backup than a network share, and record the decision.

**Runtime.** Services run as rootless Podman containers described by Quadlet units under systemd. Per-service CPU and memory limits through cgroups. Hardening: `NoNewPrivileges`, read-only root filesystems for containers, no host network except where required, only the archive path mounted.

**Shared UI.** One single-page web app (`ui/`) served by `alpd` over HTTPS (and on localhost for the console). v1 screens: status ("your album is safe" or what needs attention, last backup per device, storage, update state, alerts with codes), devices (join with QR, rename, revoke), storage and expansion wizard, recovery key status, updates, settings (language, time zone, console PIN), advanced mode, and a link into the gallery. Responsive layouts for large screens and phones; identical API, wording and error codes everywhere. UI copy is plain language, no technical terms.

**Local console.** When a display is connected, a kiosk session (block 1) starts automatically as a dedicated unprivileged user and shows the shared UI from localhost. Rules: (1) It is not part of `alp-healthy.target` and is supervised separately: a failing display, GPU driver or browser never affects backups, updates or boot health, and it restarts on its own. (2) Physical access is not full access: the console opens on a **lock screen** that shows only the overall status and any error codes. Unlocking requires the console PIN or the recovery key; attempts are rate-limited and logged in the ledger. Pairing a new device and browsing photos happen only after unlocking. (3) After unlocking, the console offers the same screens as the browser, plus troubleshooting: network check, re-show the pairing QR, revoke a lost phone, and the gallery (Immich web UI opened inside the same session). (4) It blanks the screen when idle and returns to the lock screen.

**Constraints.** Memory budget: `alpd` under 150 MB resident at idle. The console session runs only while a display is connected; record its memory use and keep the base system within budget with the console running (**VERIFY** on the reference machine). Backups must not starve the system: apply I/O and CPU weights.

**Acceptance.** Upload of 10,000 small files and one 20 GB file completes and verifies by hash, including after a forced network drop and resume. A container escape attempt test (container tries to write outside its mount) fails. Rebooting during an upload resumes cleanly.

**Out of scope.** Third-party apps other than Immich, remote access, photo-app machine learning enabled by default.

### Block 9 — Network and firewall

**Goal.** Minimal attack surface on the LAN, nothing exposed to the internet.

**Decisions.** `systemd-networkd` (DHCP by default), `avahi-daemon` for mDNS, `systemd-timesyncd` for time, `nftables` with default drop on input. Allow loopback, established traffic, required ICMP/ICMPv6, mDNS from the local link, HTTPS (443) and SMB (445, redirected) from private address ranges only. No SSH by default. **Advanced mode** is an explicit opt-in in the UI that enables SSH with key-only authentication, shows a persistent warning, and is recorded in the ledger. No UPnP, no NAT-PMP, no automatic port forwarding.

**Acceptance.** A port scan from outside the LAN range finds nothing. From the LAN only 443, 445 and mDNS answer. With advanced mode off, port 22 is closed.

**Out of scope.** Built-in VPN, relay client, IPv6 transition tooling.

### Block 10 — Health, monitoring and power

**Goal.** The user always knows whether everything is fine, in plain words.

**Decisions.** `alpd` aggregates: SMART status, scrub results, pool health, free space, slot state, last update result, ledger verification, time sync, power events. The UI maps them into the status legend (UX addendum 0.2.5, T54; ADR 0023): All fine (blue check), Note (white i, blue frame), Action needed (yellow i), Respond now (red i), Critical (red i with a yellow frame, pulsing: stop using the archive until fixed). Each level has its own shape and label, a one-sentence explanation and a recommended action. Notifications go to paired devices (through the app when connected; no external push service in v1). `journald` is persistent with a hard size cap. Hardware watchdog is enabled if present. `GenericPcPower` detects a UPS through NUT when available and triggers a clean shutdown on low battery. Unclean power loss is handled by Btrfs and by automatic recovery on next boot, and the UI recommends a UPS once, calmly.

**Error codes.** Every condition that reaches the user maps to a code from `errors/catalog.toml` (Section 2.2). Health states in the UI show the plain message, the recommended action and the code. CI checks that every code used in the source exists in the catalog, that no code is defined twice, and that every catalog entry has a title, a user action and technician detail.

**Acceptance (VM).** Injected disk error yields exactly one clear alert. Simulated power loss during upload leaves the pool consistent and the upload resumable. Alerts clear automatically when the condition resolves.

**Out of scope.** Email or cloud alerting, fleet monitoring.

## 8. Test infrastructure

- **Harness:** a QEMU/KVM runner (driven by a Rust `xtask` or `just` recipes) with OVMF firmware, `swtpm`, multiple virtual disks, and snapshot/restore of VM state.
- **Fault injection as standard tools:** kill VM mid-write, detach virtual disk, corrupt a sector in a disk image, break a service in a staged image, drop the virtual network.
- **Scenarios are named and numbered** (for example `T-BOOT-02 rollback on failed health`), referenced from each block's acceptance list, and run in CI on every merge to main. Slow scenarios (full scrub, 20 GB upload) run nightly.
- **Unit and property tests** for the ledger, planning logic (`plan_pool`, `propose_expansion`), and key hierarchy (including negative cases).
- **Hardware matrix:** after the VM suite passes, validate on a short list of real old PCs (record CPU, RAM, firmware, TPM presence) and keep results in `docs/hardware-matrix.md`.

## 9. Implementation order

1. Repository, CI skeleton, ADR template, test harness (Section 8).
2. Block 1 (image builder) and a booting empty image in a VM.
3. Block 2 (boot chain, A/B, rollback) with its fault-injection tests.
4. Block 3 (state), then Block 4 (storage) with disk-failure tests.
5. Block 5 (keys and recovery) and Block 6 (ledger).
6. Block 9 (network and firewall) and the `alpd` skeleton with API and UI shell.
7. Block 7 (installer, first boot, pairing).
8. Block 8 (ingestion and runtime), then Block 10 (health and power).
9. Hardening pass, hardware matrix, documentation, first alpha release.

## 10. Open items to resolve during implementation

- **Verify toolchain versions** on the current Debian Stable: kernel, systemd, btrfs-progs, mkosi, Podman, Samba.
- **TPM sealing policy** that survives A/B updates (signed PCR policy versus a stable PCR subset). Needs an ADR with a tested design.
- **Immich integration:** chain-to-Immich sign-in route, database migration and rollback procedure, RAM thresholds, deletion behaviour, and face-recognition licensing (block 8).
- **Browser TLS experience:** the local console uses localhost and shows no warnings; the phone app trusts the node CA through pairing. For a laptop browser on the home network, decide between guided CA install and access only from paired devices. It must not teach users to click through warnings.
- **Legacy BIOS demand:** measure how many target machines are excluded before finalizing the UEFI-only decision.
- **Update server operation and signing key custody:** process, rotation and revocation policy (non-code, but required before the first public release).
- **Licensing and trademark** review of the final component list before publishing images or selling hardware (InsightFace models, Immich AGPL obligations, ZFS if ever added).
- **Resolved in 0.3:** the single-disk default favours protection (data `dup`) with an advanced switch to `single`; the PC backup method is decided when the PC phase starts.

## 11. Design hooks for future work (do not implement in v1)

- **Accountabilibuddy distributed backup:** identity key, signed ledger, and the `alp-crypto` crate boundary are designed to host chunking, encryption, erasure coding and fragment placement later. Do not couple any v1 code to the storage format on disk beyond the `StorageBackend` trait.
- **Relay and remote access:** `alp-network` keeps TLS and device certificates independent of the LAN assumption.
- **Local AI indexing:** will be an optional container reading from `@archive` through a read-only mount; reserve a service slot and a profile flag, nothing else.
- **ARM and ALP-Box:** new `BootBackend`, `KeySealBackend` and `PowerBackend` implementations plus a new hardware profile. No v1 code above the traits may assume x86 or UEFI.
- **ZFS:** a second `StorageBackend` implementation, chosen at pool creation and stored as a pool property.
- **Headless install:** a second front end for the installer library (remote or companion-app driven).
- **Shamir secret sharing and delegated recovery:** extends the recovery key mechanism in block 5.
