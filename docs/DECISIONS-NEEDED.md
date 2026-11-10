# Decisions needed from the owner

Open questions that block or shape work. Each has options, consequences and a
recommendation. Answered items move to an ADR.

Items 1–8 were answered on 2026-10-17 (`docs/adr/0005-owner-decisions-2026-10-17.md`).

## 9. Spec v0.5 wording: what the installer boots from (low priority)

Spec v0.5 says in Section 1.2 and block 1 that ALP-OS Installer is a hybrid
ISO, but Section 6 ("Install") and block 7 ("Installer") still say "A
bootable USB image" / "The USB image boots into the kiosk session". Per ADR
0006 the installer is the hybrid ISO and the USB image is ALP-OS Live. No
work is blocked: streams follow ADR 0006. Please adjust those two sentences
in the next spec edit.
