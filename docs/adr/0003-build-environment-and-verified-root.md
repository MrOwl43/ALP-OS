# 0003 — Build environment, CI image job and verified root (block 1)

- **Status:** accepted
- **Date:** 2026-10-16
- **Spec reference:** block 1, block 2 (verity, Secure Boot), Section 8; review of PR #3

## Context

Block 1 needs reproducible images, a verified read-only root and automatic VM
tests. Several choices were made that are not obvious from the spec.

## Decisions

1. **Build tools and pinning.** Images are built with the host's mkosi 26 and
   systemd 259 tools on Ubuntu 26.04 (dev VM and the `ubuntu-26.04` GitHub
   runner). Exact versions are listed in `tests/ci/build-tools.lock`; CI
   installs exactly those and `just check` verifies the dev VM. A pinned mkosi
   tools tree (same Debian snapshot as the image) remains the target, because
   mkosi 26 cannot pin a tools tree to a snapshot. Debian packages in the
   image come from snapshot.debian.org; `SourceDateEpoch=` must equal the
   snapshot moment (checked by `tests/ci/check-snapshot-epoch.sh`).
2. **Reproducibility scope.** Acceptance (two clean builds of one commit give
   identical hashes) is met on one machine (`just repro-check`, nightly CI).
   Builds on the dev VM and on CI currently differ in the UKI, initrd and
   disk image; tracked as an open item.
3. **Ubuntu AppArmor.** Ubuntu blocks unprivileged user namespaces. The
   profile `tests/ci/mkosi.apparmor` allows them for `/usr/bin/mkosi` only. It
   is installed on CI runners and, with the owner's approval, on the dev VM.
4. **KVM on CI.** The image job makes `/dev/kvm` world-accessible with GitHub's
   documented udev rule. Acceptable only on ephemeral GitHub-hosted runners;
   never on the dev VM or a self-hosted runner.
5. **Verified root.** Root is EROFS with a dm-verity hash partition
   (`image/repart.d/`); mkosi puts the roothash on the UKI command line.
   `loader.conf` sets `editor no`.
   **Limit:** with an unsigned UKI and Secure Boot off (the beta), this
   protects against corruption and offline changes to the root, not against
   an attacker who can replace the UKI or ESP. UKI and verity signing follow
   with block 1 step 8 / block 2 and need the key-custody decision (spec
   Section 10).
6. **Temporary `/var` in memory.** `systemd.volatile=state` keeps the root
   read-only and puts `/var` on tmpfs until the state partition (block 3). The
   journal is capped at 64 MB and is lost at reboot until then.
7. **Test-only units through SMBIOS credentials.** VM tests inject probe units
   with `systemd.extra-unit.*` credentials (systemd >= 256), so the tested
   image is unchanged. This also shows that the image accepts units from the
   hypervisor's SMBIOS data (read by systemd only inside a VM). To be covered
   by the block 2 threat model (Secure Boot, signed images).
8. **Installer medium for the beta: USB disk image.** The owner decided
   (2026-10-16) that a USB disk image (`.raw`, written to a USB stick) is
   enough for the beta; no ISO. This departs from the spec's "installer ISO"
   (block 1 Goal, Outputs and Acceptance; Section 1.2). The owner keeps the
   deviation recorded here and will update the spec (ADR 0005). Block 1's
   "ISO boots in QEMU with OVMF" is met by the USB image booting in QEMU
   with OVMF (T-BOOT-01).

## Open items

- Identical partition and ESP UUIDs on every copy of the image (fixed
  `Seed=`; root/verity UUIDs derive from the roothash). Before block 7 the
  installer must give each installation new UUIDs, and a test must boot the
  installer USB with an installed disk attached (blocks 2 and 7).
- Cross-machine reproducibility (decision 2).
- Image size budget: 2 GB for the system image (owner, ADR 0005; T-BUDGET-01).
