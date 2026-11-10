// SPDX-License-Identifier: AGPL-3.0-or-later
//! The recovery code (owner decision 2026-10-24, ADR 0013): 15 BIP-39
//! English words (160 random bits + 5-bit checksum) that reopen the archive
//! when the stick is lost. Shown once, never stored on the stick.
//!
//! No own cryptography: word list, checksum and the BIP-39 seed (PBKDF2-
//! HMAC-SHA512 over the words, salt "mnemonic" + passphrase) come from the
//! `bip39` crate; the entropy comes from the kernel. With serious encryption
//! the extra passphrase is the BIP-39 passphrase, so the same words with
//! another passphrase give an unrelated secret.
//!
//! Storage seam: this module does not lock anything itself. The storage
//! layer (storage v2) receives [`Code::unlock_secret`] and hands it to its
//! own mature tool as a passphrase (gocryptfs' scrypt password of
//! `ALPArchive/recovery.conf`, ADR 0014), which applies its own slow key
//! derivation; that slow step is what protects a weak passphrase. [`bind`]
//! and [`open`] are the two calls storage v2 implements (in `archive`).

use std::fs;
use std::io::Read;

use bip39::{Language, Mnemonic};
use zeroize::Zeroizing;

use crate::fail::Fail;

pub const WORDS: usize = 15;
const ENTROPY_BYTES: usize = 20;
const QR_PREFIX: &str = "alparchive-recovery:1:";
pub const MIN_PASSPHRASE: usize = 8;
pub const MAX_PASSPHRASE: usize = 128;

/// A valid recovery code (checked words).
pub struct Code {
    words: Zeroizing<Vec<String>>,
}

impl std::fmt::Debug for Code {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("recovery::Code(<redacted>)")
    }
}

impl Code {
    pub fn words(&self) -> &[String] {
        &self.words
    }

    /// The words with single spaces, for showing.
    pub fn phrase(&self) -> Zeroizing<String> {
        Zeroizing::new(self.words.join(" "))
    }

    /// What the QR code holds.
    pub fn qr_text(&self) -> Zeroizing<String> {
        Zeroizing::new(format!("{QR_PREFIX}{}", self.words.join("-")))
    }

    /// The secret the storage layer gives its own tool as a passphrase:
    /// the BIP-39 seed (64 bytes) of these words with `passphrase` (empty
    /// unless serious encryption), as 128 lowercase hex characters.
    pub fn unlock_secret(&self, passphrase: &str) -> Zeroizing<String> {
        let m = mnemonic(&self.words);
        let seed = Zeroizing::new(m.to_seed(passphrase));
        Zeroizing::new(seed.iter().map(|b| format!("{b:02x}")).collect())
    }
}

fn mnemonic(words: &[String]) -> Mnemonic {
    let phrase = Zeroizing::new(words.join(" "));
    // Only built from checked words (generate/parse).
    Mnemonic::parse_in(Language::English, phrase.as_str()).unwrap_or_else(|_| unreachable!())
}

/// A new code from 160 bits of kernel randomness.
pub fn generate() -> Result<Code, Fail> {
    let mut entropy = Zeroizing::new([0u8; ENTROPY_BYTES]);
    fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut entropy[..]))
        .map_err(|e| Fail::new("internal", format!("No random numbers: {e}")))?;
    from_entropy(&entropy[..])
}

fn from_entropy(entropy: &[u8]) -> Result<Code, Fail> {
    let m = Mnemonic::from_entropy_in(Language::English, entropy)
        .map_err(|e| Fail::new("internal", e.to_string()))?;
    Ok(Code {
        words: Zeroizing::new(m.words().map(str::to_string).collect()),
    })
}

/// Split typed or scanned text into lowercase words: spaces, commas and
/// dashes separate; the QR prefix is dropped.
pub fn split_words(text: &str) -> Vec<String> {
    let t = text.trim();
    let t = t.strip_prefix(QR_PREFIX).unwrap_or(t);
    t.split(|c: char| c.is_whitespace() || c == '-' || c == ',')
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Check a typed recovery code: count (an 8-word chain code gets its own
/// hint), each word in the list, then the checksum.
pub fn parse(words: &[String]) -> Result<Code, Fail> {
    let words: Vec<String> = words.iter().flat_map(|w| split_words(w)).collect();
    match words.len() {
        WORDS => {}
        crate::chain::WORDS => {
            return Err(Fail::new(
                "recovery.chain_code",
                "This is a chain code (8 words). The recovery code has 15 words; it is on the paper from the recovery page.",
            ));
        }
        n => {
            return Err(Fail::new(
                "recovery.word_count",
                format!("A recovery code has {WORDS} words; this has {n}."),
            ));
        }
    }
    if let Some(i) = words
        .iter()
        .position(|w| Language::English.find_word(w).is_none())
    {
        let mut f = Fail::new(
            "recovery.word_unknown",
            format!(
                "Word {} is not one of the recovery words. Please check its spelling.",
                i + 1
            ),
        );
        f.index = Some(i + 1);
        return Err(f);
    }
    let phrase = Zeroizing::new(words.join(" "));
    Mnemonic::parse_in(Language::English, phrase.as_str()).map_err(|_| {
        Fail::new(
            "recovery.checksum",
            "These words do not fit together. One of them is probably wrong; please check them against your paper.",
        )
    })?;
    Ok(Code {
        words: Zeroizing::new(words),
    })
}

/// The passphrase rules (serious encryption): 8 to 128 characters, no
/// control characters.
pub fn check_passphrase(p: &str) -> Result<(), Fail> {
    let n = p.chars().count();
    if !(MIN_PASSPHRASE..=MAX_PASSPHRASE).contains(&n) || p.chars().any(char::is_control) {
        return Err(Fail::new(
            "recovery.passphrase",
            format!(
                "The extra passphrase needs {MIN_PASSPHRASE} to {MAX_PASSPHRASE} letters, digits, spaces or punctuation."
            ),
        ));
    }
    Ok(())
}

/// Answer of the storage seam before storage v2 implemented it (kept for
/// clients that still map it; no longer returned).
pub const NOT_READY: &str = "recovery codes arrive with the next storage update";
/// [`bind`] without an open archive.
pub const NOT_OPEN: &str = "the archive is not open";
/// [`bind`] on an archive made before storage v2 (API `recovery.old_archive`).
pub const OLD_ARCHIVE: &str = "This archive was made with an earlier test version: its 64-letter recovery key still works. Recovery words need a new archive.";

/// [`bind`] or a check on an unencrypted archive (ADR 0016 levels 1 and 2):
/// nothing is locked, so no code could reopen it (API `recovery.not_encrypted`).
pub const NOT_ENCRYPTED: &str =
    "This backup disk is not encrypted, so a recovery code has nothing to reopen.";
/// [`commit`] without a pending code.
pub const NO_PENDING: &str = "no new recovery code is waiting";

/// First step of a new code (T42): make `secret` (from
/// [`Code::unlock_secret`]) open the open archive through a pending config
/// (storage v2, ADR 0014: `ALPArchive/recovery.conf.pending`). The current
/// code keeps working until [`commit`]. Errors: [`NOT_OPEN`],
/// [`OLD_ARCHIVE`], [`NOT_ENCRYPTED`], or what failed.
pub fn bind(secret: &str) -> Result<(), String> {
    crate::archive::bind_recovery(secret)
}

/// Second step: the pending code replaces the current one ([`NO_PENDING`]
/// when there is none).
pub fn commit() -> Result<(), String> {
    crate::archive::commit_recovery()
}

/// Forget the pending code; the current one stays valid.
pub fn cancel() {
    crate::archive::drop_pending_recovery()
}

/// Open the archive on `partuuid` (made with another stick) with `secret`
/// and give it a new password in this stick's index (storage v2). A wrong
/// code answers [`crate::archive::RECOVERY_WRONG`].
pub fn open(partuuid: &str, secret: &str, say: crate::job::Progress) -> Result<(), String> {
    crate::archive::recover_with_secret(partuuid, secret, say)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<String> {
        vec![s.to_string()]
    }

    // 15-word (160-bit) BIP-39 vectors. The published vector file has no
    // 160-bit cases, so these were computed independently of the crate
    // with Python's hashlib from the BIP-39 algorithm (checksum = first 5
    // bits of SHA-256; seed = PBKDF2-HMAC-SHA512, 2048 rounds, salt
    // "mnemonic" + passphrase), passphrase "TREZOR" as in the vector file.
    const V0: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon address";
    const V0_SEED_TREZOR: &str = "fa08713f46bf5cb48728ceb70e3aae1bc53c5cb7b4e29c5610261d1cbb7be3bed4d805256fec515754d2be35974fc5da678168e9d9bb0cb70948026923b0def3";
    const V1: &str = "legal winner thank year wave sausage worth useful legal winner thank year wave sausage wise";

    #[test]
    fn bip39_15_word_vectors() {
        // Entropy 00…00 (20 bytes) -> "abandon ×14 address".
        assert_eq!(from_entropy(&[0u8; 20]).unwrap().phrase().as_str(), V0);
        // Entropy 7f7f…7f -> "legal winner … wise".
        assert_eq!(from_entropy(&[0x7f; 20]).unwrap().phrase().as_str(), V1);
        assert_eq!(
            from_entropy(&[0x80; 20]).unwrap().phrase().as_str(),
            "letter advice cage absurd amount doctor acoustic avoid letter advice cage absurd amount doctor accident"
        );
        assert_eq!(
            from_entropy(&[0xff; 20]).unwrap().phrase().as_str(),
            "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrist"
        );
    }

    #[test]
    fn unlock_secret_follows_bip39_and_the_passphrase() {
        let c = parse(&w(V0)).unwrap();
        let plain = c.unlock_secret("");
        let serious = c.unlock_secret("TREZOR");
        assert_eq!(plain.len(), 128);
        assert_ne!(plain, serious);
        assert_eq!(serious.as_str(), V0_SEED_TREZOR);
        assert_eq!(c.unlock_secret("TREZOR"), serious, "stable");
    }

    #[test]
    fn generated_codes_have_15_words_and_parse() {
        let c = generate().unwrap();
        assert_eq!(c.words().len(), WORDS);
        let again = parse(&split_words(&c.qr_text())).unwrap();
        assert_eq!(again.words(), c.words());
        assert_ne!(generate().unwrap().words(), c.words());
    }

    #[test]
    fn typed_codes_are_checked() {
        assert!(parse(&w(&V0.to_uppercase())).is_ok(), "case is ignored");
        assert!(parse(&w(&V0.replace(' ', "-"))).is_ok(), "dashes work");
        assert!(parse(&w(&format!("  {}  ", V0.replace(' ', "  ")))).is_ok());
        let eight = "abandon abandon abandon abandon abandon abandon abandon abandon";
        assert_eq!(parse(&w(eight)).unwrap_err().key, "recovery.chain_code");
        let twelve: Vec<String> = V0.split(' ').take(12).map(String::from).collect();
        assert_eq!(parse(&twelve).unwrap_err().key, "recovery.word_count");
        let mut typo: Vec<String> = V0.split(' ').map(String::from).collect();
        typo[3] = "abandn".into();
        let e = parse(&typo).unwrap_err();
        assert_eq!((e.key, e.index), ("recovery.word_unknown", Some(4)));
        let mut bad: Vec<String> = V0.split(' ').map(String::from).collect();
        bad[14] = "abandon".into();
        assert_eq!(parse(&bad).unwrap_err().key, "recovery.checksum");
    }

    #[test]
    fn passphrase_rules() {
        assert!(check_passphrase("long enough").is_ok());
        assert_eq!(
            check_passphrase("short").unwrap_err().key,
            "recovery.passphrase"
        );
        assert!(check_passphrase("tab\there").is_err());
        assert!(check_passphrase(&"x".repeat(129)).is_err());
    }

    #[test]
    fn storage_seam_messages() {
        // No archive here (no record on a development machine): not open.
        if !std::path::Path::new("/var/lib/alp/live/archive.toml").exists() {
            assert_eq!(bind("x").unwrap_err(), NOT_OPEN);
        }
        assert!(OLD_ARCHIVE.contains("64-letter recovery key still works"));
    }
}
