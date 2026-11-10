<!-- Generated from docs/spec.md (Version 0.5) by tools/split-spec.py. Do not edit; edit docs/spec.md and rerun the tool. -->

## 8. Test infrastructure

- **Harness:** a QEMU/KVM runner (driven by a Rust `xtask` or `just` recipes) with OVMF firmware, `swtpm`, multiple virtual disks, and snapshot/restore of VM state.
- **Fault injection as standard tools:** kill VM mid-write, detach virtual disk, corrupt a sector in a disk image, break a service in a staged image, drop the virtual network.
- **Scenarios are named and numbered** (for example `T-BOOT-02 rollback on failed health`), referenced from each block's acceptance list, and run in CI on every merge to main. Slow scenarios (full scrub, 20 GB upload) run nightly.
- **Unit and property tests** for the ledger, planning logic (`plan_pool`, `propose_expansion`), and key hierarchy (including negative cases).
- **Hardware matrix:** after the VM suite passes, validate on a short list of real old PCs (record CPU, RAM, firmware, TPM presence) and keep results in `docs/hardware-matrix.md`.
