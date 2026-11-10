<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# 0016 — Live protection levels: an unprotected file pipe up to strong encryption

- **Status:** accepted (owner, 2026-10-28, task T35); levels 1, 2 and 4 implemented 2026-10-28, level 3's password reopening 2026-10-30 (`ALPArchive/password.conf`, bound at creation and after every correct password; T-LIVE-02 boot 4)
- **Date:** 2026-10-28
- **Spec reference:** overview #3 ("data is always encrypted at rest"),
  blocks 4, 5; ADR 0011, 0012, 0013, 0014

## Context

The owner (2026-10-28): ALP-OS Live is a short-term **file pipe**: it
collects photos from many devices now, and the files later move to their
final place in the home archive (the full install). For that use the
backup disk may also be **unencrypted**, so the files can be read on any
computer. The full install stays always encrypted; its encryption already
exists in the Live code and stays there, only switched off by the choice
below. So spec overview #3 holds for the full install, and Live gets this
exception (owner: "no conflict, it means the Live version only").

Owner's rules:

1. The first protection screen asks one question: "Do you want to encrypt
   your backup disk?" Every other protection setting comes only after
   "Yes".
2. The UI pushes level 3 (below) as the default. A user who types no
   password and slides to confirm gets level 1: "we have to go with it".
3. Unprotected levels show a clear **UNPROTECTED** screen before the
   slide-to-confirm: anyone who plugs the disk into another computer sees
   the files. No recovery code is made.
4. The 15-word recovery code belongs mostly to the full install (the
   archive admin's way back after a crash). In Live it is only for the
   strong level.
5. Open blocker (not in this change): strong encryption asks for the
   password at every start. A safe shortcut (TPM, or the ALP password when
   we can be sure the hardware is the same) must be found later.

## Decision

The level is chosen once per stick at setup and kept in the plain stick
config (`protection` in the index config: `none`, `console`, `password`,
`strong`). It cannot be changed later in the alpha (a new setup is needed);
changing it is a later task.

| Level | Console | Stick index | Backup disk | Recovery code |
|---|---|---|---|---|
| 1 `none` | no password | plain Btrfs | plain folder | none |
| 2 `console` | ALP password | plain Btrfs | plain folder | none |
| 3 `password` (default) | ALP password | LUKS2 (+TPM) | gocryptfs | none: the ALP password itself reopens the disk |
| 4 `strong` | ALP password | LUKS2, no TPM | gocryptfs | 15 words + extra passphrase (ADR 0013 #6) |

- **Plain levels (1, 2).** The index partition is made the same way but
  formatted Btrfs without LUKS and mounted at every start without asking.
  The archive is `ALPArchive/files/` on the backup disk, a normal folder,
  bind-mounted at `/var/lib/alp-pool/files` (same path as the gocryptfs
  view, so Immich and the rest do not change). `ALPArchive/README.txt`
  says it is not encrypted. Level 2 keeps a password hash for the console
  in the index (`alp/console.hash`, `openssl passwd -6`, SHA-512 crypt
  with a random salt: the system's own tool, no new code or crate); it
  guards the console only, not the data, and the UI says so. Phones need
  a password for Immich: level 2 uses the ALP password; level 1 gets a
  random phone password shown on the phone screen (the pre-T23 family
  password path, still in the UI).
- **Level 3.** As today (index LUKS2 opened by the password or the TPM,
  gocryptfs archive with the random archive password from the index),
  plus `ALPArchive/recovery.conf` bound to a secret derived from the ALP
  password through the existing `recovery::bind` seam (ADR 0014), so a
  new stick reopens the disk with the ALP password; gocryptfs' scrypt
  protects it. Changing the ALP password re-binds it. No recovery code
  step; Settings still offers "Make a recovery code".
- **Level 4.** Today's "Encryption" option (ADR 0013 #6) with its 15
  words and extra passphrase, unchanged.
- **Old sticks** without `protection` are level 3 (a recovery code they
  made stays valid).

Setup flow (UI): welcome, network, **"Encrypt your backup disk?"** (Yes,
recommended / No). Yes: ALP password (required), backup disk with the
"Strong protection" option (renamed from "Encryption"; the screen says the
archive is always encrypted at this level), recovery code only for strong.
No: the UNPROTECTED screen with slide-to-confirm, an optional console
password (level 2, can be skipped), backup disk, phone.

## Level 3 safeguards (storage review, 2026-10-30)

- `password.conf` uses scrypt cost 2^18 (gocryptfs `-scryptn 18`), four
  times the default, because a family password guards it on the disk.
- A strong archive carries `ALPArchive/strong` and never gets one, on any
  stick. Wrong passwords on reopening count with the password back-off.
- If the binding is missing (it failed, or the archive was made outside
  setup), Settings' home reminders ask for the password once.

## Safeguards (storage review, 2026-10-28)

- `protection` in the plain config is believed only when the index
  partition holds no LUKS header (`cryptsetup isLuks`): editing the config
  of a stolen encrypted stick cannot skip its password or wipe it.
- Recovery never mixes levels: an encrypted stick refuses an unencrypted
  archive and the other way round.
- Level 1 always uses its own phone password; typed text never replaces it.
- The console hash and the phone password are mode 0600.

## Consequences

- Levels 1 and 2 give up confidentiality of the data on purpose; the
  screens say it plainly before the choice and on the About page after.
- `archive`, `index`, `immich` and alpd each gain a branch on the level;
  the encrypted paths stay as they are (the full install reuses them).
- Tests: unit tests for the config, the hash check and the flow; T-LIVE-02
  gets a plain-level run; the strong level keeps its current tests.
