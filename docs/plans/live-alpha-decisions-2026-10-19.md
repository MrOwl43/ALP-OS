<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Owner decisions 2026-10-19 for the Live alpha (binding; newest wins)

1. Disks holding any operating system are refused; internal disks stay read-only. The backup
   (encrypted archive) goes on an EXTERNAL disk or USB stick the user plugs in.
2. The index (Immich DB + thumbnails + encoded video) lives on the ALP-OS stick, encrypted
   (LUKS2, Btrfs zstd), opened automatically via TPM2 when present; the admin password is the
   fallback keyslot (unlock screen only when there is no TPM or auto-unlock fails). Archive key
   inside the index.
3. Index/archive must never run out of space: reserve + early warnings; uploads pause before full;
   index location in one config value (moving it later = copy + config).
4. Admin password chosen by the user at first start (typed twice).
5. Setup order: Welcome + hardware check -> Network -> Admin password -> External disk ->
   Phone (pairing joins the chain behind the scenes; first photos arrive) -> Recovery page ->
   All set. No separate chain step; chain seed/join live in Settings.
6. Recovery page: a recovery key + QR to print or photograph, which reopens the external archive
   from a re-written stick (spec block 5; LUKS keyslot on the archive, never stored on the stick).
7. Phone: QR = server address only; one short readable shared family password typed once
   (e.g. "river-4821"); footnote "* For now ALPArchive uses Immich, a trusted open-source photo
   app. Our own ALPArchive app is on the way." Self-signed certificate: the server screen shows a QR that installs the certificate on the phone once (owner approved 02:35Z); security specified the cert in docs/security/nginx-live.md; a small certificate download page is needed (HTTP on the LAN, certificate file only).
8. Everyday screen: Windows-style lock screen with non-sensitive status (device icons with masked
   names, network, free space, disk health, last backup). One sign-in with the admin password
   opens the interface (incl. gallery: view, scroll, search). Deleting a file and emptying the
   trash ask the password again. Idle auto-lock. Photo-frame screensaver optional, off by default.
9. UI English only for the alpha; tone warm and short, one sentence per thing
   ("Your photos are home and safe").
10. Later (not alpha): pizza disk chart next to a serious bar with all numbers; own app; full QR.
