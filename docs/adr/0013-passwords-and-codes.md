<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# 0013 — Passwords and codes: one password, 15-word recovery code, 8-word chain code, serious encryption

- **Status:** accepted (owner decision 2026-10-24, task T02; storage seam
  per coordinator 2026-10-24: storage v2 replaces the LUKS archive file)
- **Date:** 2026-10-24
- **Spec reference:** block 5 (recovery), ADR 0009 #6 and ADR 0010 (chain),
  ADR 0011 (index on the stick)

## Decisions

1. **One main password.** The admin password opens the index on the stick
   (and with it the archive key) and the console. With a TPM 2.0 chip the
   index opens by itself (unchanged), except with serious encryption (5).
2. **Recovery code: 15 BIP-39 English words** (160 random bits from
   `/dev/urandom` + 5-bit checksum), made by the pinned `bip39` crate.
   Shown once (words + QR `alparchive-recovery:1:w1-…-w15`), never stored.
   Module `alp_live::recovery`.
3. **What the storage layer receives** (`Code::unlock_secret(passphrase)`):
   the BIP-39 seed of the words, i.e. PBKDF2-HMAC-SHA512, 2048 rounds,
   salt `"mnemonic" + passphrase` (standard BIP-39 passphrase semantics), as
   128 hex characters. The storage layer hands it to its own mature tool as
   a passphrase (a LUKS key slot with argon2id, or gocryptfs' scrypt
   password) and never wraps keys itself. That tool's slow KDF is what
   protects a weak extra passphrase when the words are stolen; BIP-39's 2048
   rounds alone would not. Seam: `recovery::bind(secret)` (make the secret
   open the archive, replacing the previous code) and
   `recovery::open(partuuid, secret)` (reopen from another stick). Storage
   v2 implements both; until then they answer `NOT_READY` and the API says
   `recovery.not_ready` (no code is shown that opens nothing).
   *Rejected:* a `wrap_master_key/unwrap` pair in our code (needs an AEAD
   and key-wrapping format of our own: own cryptography); concatenating
   words and passphrase into one LUKS passphrase (ad hoc; BIP-39's own
   passphrase rule is the documented, tested way).
4. **Chain code: 8 words from the BIP-39 English list = 88 bits:** 80
   random bits + an 8-bit checksum = the first byte of SHA-256 over the 80
   bits (`bitcoin_hashes`, already in the tree as `bip39`'s dependency).
   Words are the 88 bits read 11 at a time, first bits first. A single
   wrong word is caught with probability 255/256 (unit-tested). The code is
   never an encryption key. Public chain ID = first 8 bytes of
   SHA-256("alparchive-chain-id:2:" ‖ 80 bits); recovering the code from the
   ID means trying 2^80 codes. QR `alparchive-chain:2:w1-…-w8`. Joins are
   rate-limited: after 5 wrong checksums each try waits 30 s, doubling up
   to 1 hour (in memory, like the password back-off). Chains made before
   keep their 24 BIP-39 words (`:1:` QR) and still join and show.
5. **Word counts tell the codes apart.** The chain field refuses 15 words
   (`chain.recovery_code`), the recovery field refuses 8 words
   (`recovery.chain_code`), each with a plain hint; server and UI both check.
6. **Serious encryption** (optional, at archive creation, needs the admin
   password): the choice is stored inside the encrypted index
   (`settings.toml`, so editing the plain stick config cannot undo it), the
   index's TPM key slot is removed with
   `systemd-cryptenroll --wipe-slot=tpm2` (password on stdin) before the
   archive is made, `tpm_refresh` never seals a new one, and `lock.tpm`
   reads `disabled`: the password is typed at every start. The recovery
   code then needs the extra passphrase (8–128 characters) as its BIP-39
   passphrase.
7. **Migration.** Old archives keep their systemd 64-letter recovery key
   (`archive/recover` still accepts it). Making a new recovery code is the
   storage layer's `bind`, which replaces any previous code.

## Consequences

- Until storage v2 lands, a new 15-word code cannot be made
  (`recovery.not_ready`); the setup recovery step shows that message and
  T-LIVE-02's recovery part SKIPs.
- 80-bit chain codes are short enough to type; they rely on the rate limit
  and on never being a key. If the chain ever authenticates over the
  network, its protocol must add its own rate limit on the serving side.
