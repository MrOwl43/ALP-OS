<!-- Generated from docs/spec.md (Version 0.5) by tools/split-spec.py. Do not edit; edit docs/spec.md and rerun the tool. -->

## 11. Design hooks for future work (do not implement in v1)

- **Accountabilibuddy distributed backup:** identity key, signed ledger, and the `alp-crypto` crate boundary are designed to host chunking, encryption, erasure coding and fragment placement later. Do not couple any v1 code to the storage format on disk beyond the `StorageBackend` trait.
- **Relay and remote access:** `alp-network` keeps TLS and device certificates independent of the LAN assumption.
- **Local AI indexing:** will be an optional container reading from `@archive` through a read-only mount; reserve a service slot and a profile flag, nothing else.
- **ARM and ALP-Box:** new `BootBackend`, `KeySealBackend` and `PowerBackend` implementations plus a new hardware profile. No v1 code above the traits may assume x86 or UEFI.
- **ZFS:** a second `StorageBackend` implementation, chosen at pool creation and stored as a pool property.
- **Headless install:** a second front end for the installer library (remote or companion-app driven).
- **Shamir secret sharing and delegated recovery:** extends the recovery key mechanism in block 5.
