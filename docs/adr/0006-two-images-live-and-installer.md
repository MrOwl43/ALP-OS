# 0006 — Two images: ALP-OS Live and ALP-OS Installer

- **Status:** accepted (owner decision, 2026-10-17)
- **Date:** 2026-10-17
- **Spec reference:** block 1 (outputs), block 7 (installer); refines ADR 0003
  decision 8 and ADR 0005 item 2

## Decision

ALP-OS ships as two images, built from the same mkosi configuration and the
same verified system:

1. **ALP-OS Live**: a raw USB disk image (`.raw`, written to a USB stick).
   It boots and runs from the stick and **never writes to the computer's
   internal disks**. It is the **only focus now**. Its first form is the
   **hardware-test image**: it shows a plain hardware report on the screen
   and saves it to the stick (stream A's first deliverable).
2. **ALP-OS Installer**: a **hybrid ISO** (boots from a USB stick and from
   an optical/virtual drive), containing the graphical installer (block 7).
   **Later**, after the Live image works on real hardware.

## Consequences

- ADR 0003 decision 8 ("USB disk image, no ISO, for the beta") now applies
  to ALP-OS Live; the installer medium will be a hybrid ISO. mkosi 26 has no
  ISO output, so producing the hybrid ISO is a VERIFY item for stream A
  when the installer work starts (e.g. wrapping the ESP/UKI with `xorriso`).
- Spec block 1's "installer ISO" output is restored, as ALP-OS Installer.
- Until the installer exists, beta testing is done with ALP-OS Live.
