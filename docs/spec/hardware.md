<!-- Generated from docs/spec.md (Version 0.5) by tools/split-spec.py. Do not edit; edit docs/spec.md and rerun the tool. -->

## 3. Hardware baseline for v1

- Architecture x86_64 (64-bit only). UEFI firmware required. Legacy BIOS is unsupported in v1 (the installer must detect it and explain clearly).
- RAM: 4 GB minimum for the base system, which must stay within 2 GB at idle with photo-app machine learning off. With the integrated photo app, recommend 8 GB (**VERIFY** Immich's current minimum and recommended RAM and set the installer thresholds from it). The installer measures RAM and enables or disables optional features accordingly.
- System disk: separate from data disks, 32 GB or larger (SSD, small SATA, or even a good USB SSD). Single-disk machines need at least 64 GB, and the installer says in plain words how many photos fit. If the machine has only one disk, the installer partitions it into system and data areas and tells the user that protection is limited.
- TPM2 and Secure Boot: optional. Use them when present. Behavior without them is defined below and must be shown honestly to the user.
- AES-NI: optional. Detect it at install time and choose the LUKS cipher accordingly (block 5).
- Network: wired Ethernet. Wi-Fi only if needed by the reference hardware, and not a v1 requirement.
- Display: a screen with keyboard and mouse (or touch) is required during installation. Afterwards it is optional: the server runs without one, and attaching one at any time opens the local console. Use the firmware framebuffer or common open GPU drivers; never require a specific GPU.
