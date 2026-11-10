<!-- Generated from docs/spec.md (Version 0.5) by tools/split-spec.py. Do not edit; edit docs/spec.md and rerun the tool. -->

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
