<!-- Generated from docs/spec.md (Version 0.5) by tools/split-spec.py. Do not edit; edit docs/spec.md and rerun the tool. -->

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
