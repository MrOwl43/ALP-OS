<!-- Generated from docs/spec.md (Version 0.5) by tools/split-spec.py. Do not edit; edit docs/spec.md and rerun the tool. -->

### Block 5 — Encryption and key management

**Goal.** Data at rest is unreadable without user-owned keys, and a non-technical user can recover from hardware failure.

**Key hierarchy (all generated on first boot by ****`alp-crypto`****).**

- **Identity key** (Ed25519): the node identity and root of trust for the ledger and device certificates. Reserved so that future user and device identities attach to it.
- **Pool key** (random 512-bit): unlocks every LUKS container (stored in a LUKS keyslot as a keyfile).
- **Recovery key** (random 256-bit): added as a second LUKS keyslot and used to encrypt the `state` backup (derive keys with HKDF and distinct contexts). Presented as a seed phrase (word list; **VERIFY** a standard wordlist crate such as BIP39) plus QR. In the Standard tier it is escrowed automatically to the admin phone in the chain (encrypted for that device) and offered as a printable recovery sheet at the end of installation; installation does not block on it. In the Archive encryption tier the user must confirm they saved it (re-entry check of selected words) before the flow finishes.
- **Wrap key:** protects identity and pool keys stored on `state`.

**Unattended unlock (required, the box must come back after a power cut).**

- TPM2 present: the wrap key is sealed to TPM2 with a PCR policy that tolerates normal updates (**VERIFY** a policy based on signed PCR values or on a stable PCR set so that A/B updates do not break unsealing).
- No TPM: the wrap key is a keyfile on `state`. This protects against removed or resold data disks but not against theft of the whole machine. The UI states this plainly in one sentence. An optional hardened mode (passphrase or USB key required at each boot) exists but is never the default.

**Protection tiers.**

- **Standard (default).** Data disks are always encrypted, but the user has nothing to remember: the wrap key is sealed in TPM2 when present, otherwise kept as a keyfile on `state`. The server boots and unlocks unattended. This protects against removed, failed or resold disks, not against theft of the whole machine; the UI says so in one sentence. If the machine dies: install ALP-OS on new hardware, attach the data disks, and restore with the recovery key from the admin phone or the printed sheet.
- **Archive encryption (advanced, opt-in).** The archive is additionally protected by a key derived from the user's seed phrase, required to unlock (passphrase or USB key). Before enabling it, the UI states plainly that without the seed phrase nobody, including the project, can recover the data.
- **Premium recovery (future).** A paid convenience layer may make recovery easier, for example guided recovery between the user's own devices or shared among family members, but it must never give the project the ability to decrypt user data. No key escrow on project servers.

**LUKS parameters.** LUKS2. Cipher chosen at install: `aes-xts-plain64` if AES-NI is detected, otherwise `xchacha12,aes-adiantum-plain64` (**VERIFY** kernel support). Keyfile-based slots need no expensive KDF. The recovery slot uses a normal argon2id setting.

**Constraints.** Zeroize secrets after use. No key material in logs, crash dumps or the ledger (ledger records events, not secrets). Key rotation (`rotate_pool_key`) re-wraps keyslots and must be atomic and resumable. Provide `export_recovery` only behind explicit confirmation and ledger entry.

**Interfaces.** `alp-crypto`: `generate_identity`, `seal`, `unseal`, `rotate_pool_key`, `create_recovery`, `verify_recovery`, `backup_state`, `restore_state`.

**Acceptance (VM with swtpm).** First boot unlocks automatically after reboot with TPM and without TPM. Replacing the virtual TPM, the user can regain access with the recovery key only. Fresh scan of disk images and logs finds no plaintext key material. State restore test from block 3 passes.

**Out of scope.** Shamir secret sharing, key escrow with friends, multi-user key delegation (design hooks only, see Section 11).
