<!-- Generated from docs/spec.md (Version 0.5) by tools/split-spec.py. Do not edit; edit docs/spec.md and rerun the tool. -->

### Block 2 — Boot chain, partitions and A/B updates

**Goal.** Safe boot and updates that cannot brick the machine.

**Decisions.** Disk layout via `systemd-repart` using Discoverable Partitions (**VERIFY** the exact partition type UUIDs and the verity pairing rules): `ESP` (1 GB, FAT32), `root-a` + `root-a-verity`, `root-b` + `root-b-verity`, `state` (small, persistent). Bootloader `systemd-boot` with Boot Loader Specification Type 1/2 entries and **boot counting** (`+3` tries). Health is the systemd target `alp-healthy.target`; `systemd-bless-boot` marks the slot good. Updates through `systemd-sysupdate` with signed `SHA256SUMS`. Do not write a custom watchdog or updater.

**Constraints.** Secure Boot: sign the UKI and support enrollment of project keys when firmware allows. When Secure Boot is off, the system still boots, and the UI shows a one-line, non-alarming note that boot-time authenticity is not enforced. dm-verity protects root integrity in both cases. `alp-healthy.target` requires: `alpd` responding on its socket, networking up, time sync attempted. It must **not** require the data pool.

**Interfaces.** `BootBackend` (Section 5). `alpd` exposes update status and last update result to the UI. Updates are applied in a configurable maintenance window (default 03:00 local) or on user request, and only when no backup upload is mid-flight (otherwise defer).

**Acceptance (VM).** Update to a good image is blessed after reboot. A deliberately broken image (service fails) rolls back to the previous slot automatically. Power cut (QEMU kill) during write of the inactive slot leaves the active slot untouched and bootable. A tampered root image fails verity and does not boot.

**Out of scope.** Delta updates, multiple update channels, other bootloaders.
