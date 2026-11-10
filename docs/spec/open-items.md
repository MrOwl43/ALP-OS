<!-- Generated from docs/spec.md (Version 0.5) by tools/split-spec.py. Do not edit; edit docs/spec.md and rerun the tool. -->

## 10. Open items to resolve during implementation

- **Verify toolchain versions** on the current Debian Stable: kernel, systemd, btrfs-progs, mkosi, Podman, Samba.
- **TPM sealing policy** that survives A/B updates (signed PCR policy versus a stable PCR subset). Needs an ADR with a tested design.
- **Immich integration:** chain-to-Immich sign-in route, database migration and rollback procedure, RAM thresholds, deletion behaviour, and face-recognition licensing (block 8).
- **Browser TLS experience:** the local console uses localhost and shows no warnings; the phone app trusts the node CA through pairing. For a laptop browser on the home network, decide between guided CA install and access only from paired devices. It must not teach users to click through warnings.
- **Legacy BIOS demand:** measure how many target machines are excluded before finalizing the UEFI-only decision.
- **Update server operation and signing key custody:** process, rotation and revocation policy (non-code, but required before the first public release).
- **Licensing and trademark** review of the final component list before publishing images or selling hardware (InsightFace models, Immich AGPL obligations, ZFS if ever added).
- **Resolved in 0.3:** the single-disk default favours protection (data `dup`) with an advanced switch to `single`; the PC backup method is decided when the PC phase starts.
