// SPDX-License-Identifier: AGPL-3.0-or-later
//! The sync chain, alpha (ADR 0009 #6, ADR 0010, ADR 0013): a chain code
//! of 8 words from the BIP-39 English list identifies and joins the family
//! chain, like Brave Sync. The words belong only to the chain; they are
//! never an encryption key (the recovery code has 15 words, see
//! `recovery`). 8 words = 88 bits: 80 random bits from the kernel and an
//! 8-bit checksum, the first byte of SHA-256 over the 80 bits (ADR 0013);
//! a typo is caught with probability 255/256. Chains made before keep their
//! 24 BIP-39 words (`bip39` crate's checksum) and work as before.
//!
//! State inside the encrypted index on the stick (ADR 0011): the words
//! (mode 0600, so this computer can show them again, as Brave does) and a
//! small record.

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

use bip39::{Language, Mnemonic};
use bitcoin_hashes::{Hash, sha256};
use zeroize::Zeroizing;

use crate::fail::Fail;

pub const WORDS: usize = 8;
/// Chains made before 2026-10-24 (BIP-39, 256 bits).
pub const LEGACY_WORDS: usize = 24;
const ENTROPY_BYTES: usize = 10;
const QR_PREFIX: &str = "alparchive-chain:2:";
const LEGACY_QR_PREFIX: &str = "alparchive-chain:1:";
/// BIP-39 passphrase used only to derive the public ID of a legacy chain,
/// so the ID never reveals the words.
const ID_CONTEXT: &str = "alparchive-chain-id";
/// Prefix hashed with the 80 bits to give the public ID of a chain code.
const ID_CONTEXT_2: &[u8] = b"alparchive-chain-id:2:";

/// A checked chain code.
pub enum Phrase {
    /// 8 words: the 80 random bits.
    Short(Zeroizing<[u8; ENTROPY_BYTES]>),
    /// 24 BIP-39 words (chains made before).
    Legacy(Mnemonic),
}

impl std::fmt::Debug for Phrase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("chain::Phrase(<redacted>)")
    }
}

impl Phrase {
    pub fn words(&self) -> Vec<String> {
        match self {
            Self::Short(e) => encode(e).iter().map(|w| (*w).to_string()).collect(),
            Self::Legacy(m) => m.words().map(str::to_string).collect(),
        }
    }
}

/// The 8 words of 80 bits: 88 bits (the 80 bits, then the first byte of
/// their SHA-256), 11 bits per word, first bits first.
pub fn encode(entropy: &[u8; ENTROPY_BYTES]) -> [&'static str; WORDS] {
    let sum = sha256::Hash::hash(entropy).to_byte_array()[0];
    let mut n: u128 = 0;
    for b in entropy.iter().chain([&sum]) {
        n = (n << 8) | u128::from(*b);
    }
    let list = Language::English.word_list();
    std::array::from_fn(|i| list[((n >> (77 - 11 * i)) & 0x7ff) as usize])
}

/// The 80 bits of 8 known words, if their checksum fits.
fn decode(idx: &[u16; WORDS]) -> Option<Zeroizing<[u8; ENTROPY_BYTES]>> {
    let mut n: u128 = 0;
    for i in idx {
        n = (n << 11) | u128::from(*i);
    }
    let mut e = Zeroizing::new([0u8; ENTROPY_BYTES]);
    for (k, b) in e.iter_mut().enumerate() {
        *b = (n >> (80 - 8 * k)) as u8;
    }
    (sha256::Hash::hash(&e[..]).to_byte_array()[0] == n as u8).then_some(e)
}

/// The chain as this computer knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chain {
    pub chain_id: String,
    /// `created` or `joined`.
    pub role: String,
    /// RFC 3339 UTC.
    pub since: String,
}

/// What the screen shows to let another device join.
pub struct Code {
    pub chain_id: String,
    pub words: Zeroizing<Vec<String>>,
    pub qr_text: Zeroizing<String>,
}

impl std::fmt::Debug for Code {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Code {{ chain_id: {}, words: <redacted> }}",
            self.chain_id
        )
    }
}

pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The Live store: inside the encrypted index (the words are secret).
    pub fn live() -> Self {
        Self::new(crate::index::path("chain"))
    }

    fn words_file(&self) -> PathBuf {
        self.dir.join("chain.words")
    }

    fn record_file(&self) -> PathBuf {
        self.dir.join("chain.toml")
    }

    pub fn get(&self) -> Option<Chain> {
        let t: toml::Table = fs::read_to_string(self.record_file()).ok()?.parse().ok()?;
        let g = |k: &str| t.get(k)?.as_str().map(str::to_string);
        Some(Chain {
            chain_id: g("chain_id")?,
            role: g("role")?,
            since: g("since")?,
        })
    }

    /// Start a new chain with fresh words.
    pub fn create(&self, now: &str) -> Result<Code, Fail> {
        self.refuse_if_member()?;
        let mut entropy = Zeroizing::new([0u8; ENTROPY_BYTES]);
        fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut entropy[..]))
            .map_err(|e| Fail::new("internal", format!("No random numbers: {e}")))?;
        let p = Phrase::Short(entropy);
        self.save(&p, "created", now)?;
        Ok(code(&p))
    }

    /// Join a chain with words typed or scanned.
    pub fn join(&self, words: &[String], now: &str) -> Result<Chain, Fail> {
        self.refuse_if_member()?;
        let m = parse(words)?;
        self.save(&m, "joined", now)?;
        self.get()
            .ok_or_else(|| Fail::new("internal", "The chain could not be saved."))
    }

    /// The words and QR again ("view the chain code").
    pub fn code(&self) -> Result<Code, Fail> {
        let phrase = Zeroizing::new(fs::read_to_string(self.words_file()).map_err(|_| none())?);
        let p = parse(std::slice::from_ref(&*phrase))
            .map_err(|_| Fail::new("internal", "The saved chain words are damaged."))?;
        Ok(code(&p))
    }

    /// Forget the chain on this computer.
    pub fn leave(&self) -> Result<(), Fail> {
        if self.get().is_none() {
            return Err(none());
        }
        let _ = fs::remove_file(self.record_file());
        let _ = fs::remove_file(self.words_file());
        Ok(())
    }

    fn refuse_if_member(&self) -> Result<(), Fail> {
        if self.get().is_some() {
            return Err(Fail::new(
                "chain.exists",
                "This computer is already in a family chain.",
            ));
        }
        Ok(())
    }

    fn save(&self, m: &Phrase, role: &str, now: &str) -> Result<(), Fail> {
        let io =
            |e: std::io::Error| Fail::new("internal", format!("The chain could not be saved: {e}"));
        fs::create_dir_all(&self.dir).map_err(io)?;
        let phrase = Zeroizing::new(m.words().join(" "));
        let tmp = self.dir.join("chain.words.tmp");
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(io)?;
        f.write_all(phrase.as_bytes())
            .and_then(|()| f.sync_all())
            .map_err(io)?;
        fs::rename(&tmp, self.words_file()).map_err(io)?;
        let mut t = toml::Table::new();
        t.insert("chain_id".into(), chain_id(m).into());
        t.insert("role".into(), role.into());
        t.insert("since".into(), now.into());
        let rec = self.dir.join("chain.toml.tmp");
        fs::write(&rec, t.to_string()).map_err(io)?;
        fs::File::open(&rec)
            .and_then(|f| f.sync_all())
            .map_err(io)?;
        fs::rename(&rec, self.record_file()).map_err(io)?;
        fs::File::open(&self.dir)
            .and_then(|d| d.sync_all())
            .map_err(io)
    }
}

fn none() -> Fail {
    Fail::new("chain.none", "This computer is not in a family chain yet.")
}

/// Public ID of the chain, 16 hex characters. Chain code: the first 8
/// bytes of SHA-256 over a fixed prefix and the 80 bits (a one-way hash;
/// finding the code from it means trying 2^80 codes). Legacy chain: the
/// BIP-39 seed made with a passphrase used for nothing else.
pub fn chain_id(p: &Phrase) -> String {
    let id: Zeroizing<Vec<u8>> = match p {
        Phrase::Short(e) => {
            let mut data = Zeroizing::new(ID_CONTEXT_2.to_vec());
            data.extend_from_slice(&e[..]);
            Zeroizing::new(sha256::Hash::hash(&data).to_byte_array()[..8].to_vec())
        }
        Phrase::Legacy(m) => Zeroizing::new(m.to_seed(ID_CONTEXT)[..8].to_vec()),
    };
    id.iter().map(|b| format!("{b:02x}")).collect()
}

fn code(p: &Phrase) -> Code {
    let words = p.words();
    let prefix = match p {
        Phrase::Short(_) => QR_PREFIX,
        Phrase::Legacy(_) => LEGACY_QR_PREFIX,
    };
    Code {
        chain_id: chain_id(p),
        qr_text: Zeroizing::new(format!("{prefix}{}", words.join("-"))),
        words: Zeroizing::new(words),
    }
}

/// Words from the QR text (`alparchive-chain:2:w1-w2-…`, or `:1:` for a
/// legacy chain).
pub fn words_from_qr(text: &str) -> Result<Vec<String>, Fail> {
    let t = text.trim();
    t.strip_prefix(QR_PREFIX)
        .or_else(|| t.strip_prefix(LEGACY_QR_PREFIX))
        .map(|w| w.split('-').map(str::to_string).collect())
        .ok_or_else(|| {
            Fail::new(
                "chain.word_count",
                "This picture is not an ALPArchive chain code.",
            )
        })
}

/// Check typed words: count (a 15-word recovery code gets its own hint),
/// each word in the list, then the checksum.
pub fn parse(words: &[String]) -> Result<Phrase, Fail> {
    let words: Vec<String> = words
        .iter()
        .flat_map(|w| w.split(|c: char| c.is_whitespace() || c == ','))
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    match words.len() {
        WORDS | LEGACY_WORDS => {}
        crate::recovery::WORDS => {
            return Err(Fail::new(
                "chain.recovery_code",
                "This is a recovery code (15 words). It is only for reopening your archive; a chain code has 8 words.",
            ));
        }
        n => {
            return Err(Fail::new(
                "chain.word_count",
                format!("A chain code has {WORDS} words; this has {n}."),
            ));
        }
    }
    let idx: Vec<Option<u16>> = words
        .iter()
        .map(|w| Language::English.find_word(w))
        .collect();
    if let Some(i) = idx.iter().position(Option::is_none) {
        let mut f = Fail::new(
            "chain.word_unknown",
            format!(
                "Word {} is not one of the chain words. Please check its spelling.",
                i + 1
            ),
        );
        f.index = Some(i + 1);
        return Err(f);
    }
    let checksum = || {
        Fail::new(
            "chain.checksum",
            "These words do not fit together. One of them is probably wrong; please check them against your paper.",
        )
    };
    if words.len() == WORDS {
        let idx: [u16; WORDS] = std::array::from_fn(|i| idx[i].unwrap_or_default());
        return decode(&idx).map(Phrase::Short).ok_or_else(checksum);
    }
    let phrase = Zeroizing::new(words.join(" "));
    Mnemonic::parse_in(Language::English, phrase.as_str())
        .map(Phrase::Legacy)
        .map_err(|_| checksum())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("alp-chain-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    // BIP-39 test vector (all-zero entropy, 24 words): a legacy chain.
    const ZERO24: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";
    // 8-word chain codes computed independently with Python's hashlib
    // (80 bits, then the first byte of their SHA-256; 11 bits per word).
    const ZERO8: &str = "abandon abandon abandon abandon abandon abandon abandon ability";
    const SEQ8: &str = "abandon amount liar amount expire adjust cage catalog";

    fn split(s: &str) -> Vec<String> {
        s.split(' ').map(String::from).collect()
    }

    #[test]
    fn eight_word_vectors() {
        assert_eq!(encode(&[0; 10]).join(" "), ZERO8);
        let seq: [u8; 10] = std::array::from_fn(|i| i as u8);
        assert_eq!(encode(&seq).join(" "), SEQ8);
        match parse(&[SEQ8.to_string()]).unwrap() {
            Phrase::Short(e) => assert_eq!(*e, seq),
            Phrase::Legacy(_) => panic!("8 words are a chain code"),
        }
    }

    #[test]
    fn checksum_round_trip_and_typos() {
        for seed in 0u8..50 {
            let e: [u8; 10] =
                std::array::from_fn(|i| seed.wrapping_mul(37).wrapping_add(i as u8 * 11));
            let words: Vec<String> = encode(&e).iter().map(|w| (*w).to_string()).collect();
            let p = parse(&words).unwrap();
            assert_eq!(p.words(), words, "round trip");
        }
        // Every single-word typo into another list word: caught unless the
        // 8-bit checksum matches by chance (1 in 256).
        let base = split(SEQ8);
        let list = Language::English.word_list();
        let (mut tried, mut missed) = (0u32, 0u32);
        for pos in 0..WORDS {
            for other in list.iter().step_by(7) {
                if *other == base[pos] {
                    continue;
                }
                let mut t = base.clone();
                t[pos] = (*other).to_string();
                tried += 1;
                if parse(&t).is_ok() {
                    missed += 1;
                }
            }
        }
        assert!(tried > 2000);
        assert!(missed * 100 < tried, "{missed} of {tried} typos passed");
        // Two words swapped.
        let mut sw = base.clone();
        sw.swap(1, 2);
        assert_eq!(parse(&sw).unwrap_err().key, "chain.checksum");
    }

    #[test]
    fn create_show_leave_join() {
        let s = Store::new(tmp("a"));
        assert_eq!(s.code().unwrap_err().key, "chain.none");
        let c = s.create("2026-10-19T09:41:00Z").unwrap();
        assert_eq!(c.words.len(), WORDS);
        assert!(c.qr_text.starts_with("alparchive-chain:2:"));
        assert_eq!(c.chain_id.len(), 16);
        assert_eq!(s.get().unwrap().role, "created");
        assert_eq!(s.create("x").unwrap_err().key, "chain.exists");
        let again = s.code().unwrap();
        assert_eq!(again.words, c.words);
        assert_eq!(again.chain_id, c.chain_id);
        let mode = fs::metadata(s.words_file()).unwrap();
        assert_eq!(
            std::os::unix::fs::PermissionsExt::mode(&mode.permissions()) & 0o777,
            0o600
        );

        // A second computer joins with the words (from the QR).
        let other = Store::new(tmp("b"));
        let words = words_from_qr(&c.qr_text).unwrap();
        let joined = other.join(&words, "2026-10-19T10:00:00Z").unwrap();
        assert_eq!(joined.chain_id, c.chain_id);
        assert_eq!(joined.role, "joined");
        s.leave().unwrap();
        assert!(s.get().is_none());
        assert_eq!(s.leave().unwrap_err().key, "chain.none");
    }

    #[test]
    fn legacy_24_word_chains_keep_working() {
        let s = Store::new(tmp("legacy"));
        let j = s
            .join(&[ZERO24.to_string()], "2026-10-19T10:00:00Z")
            .unwrap();
        let c = s.code().unwrap();
        assert_eq!(c.words.len(), LEGACY_WORDS);
        assert_eq!(c.chain_id, j.chain_id);
        assert!(c.qr_text.starts_with("alparchive-chain:1:"));
        let other = Store::new(tmp("legacy2"));
        let words = words_from_qr(&c.qr_text).unwrap();
        assert_eq!(other.join(&words, "x").unwrap().chain_id, j.chain_id);
    }

    #[test]
    fn checks_typed_words() {
        assert!(
            parse(&[ZERO8.to_uppercase()]).is_ok(),
            "case and spacing are ignored"
        );
        assert!(parse(&[ZERO24.to_uppercase()]).is_ok(), "legacy chains");
        let fifteen = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon address";
        assert_eq!(
            parse(&split(fifteen)).unwrap_err().key,
            "chain.recovery_code"
        );
        let short: Vec<String> = ZERO24.split(' ').take(12).map(String::from).collect();
        assert_eq!(parse(&short).unwrap_err().key, "chain.word_count");
        let mut typo = split(ZERO8);
        typo[6] = "abandn".into();
        let e = parse(&typo).unwrap_err();
        assert_eq!((e.key, e.index), ("chain.word_unknown", Some(7)));
        let mut bad = split(ZERO8);
        bad[7] = "abandon".into();
        assert_eq!(parse(&bad).unwrap_err().key, "chain.checksum");
        let mut bad24 = split(ZERO24);
        bad24[23] = "abandon".into();
        assert_eq!(parse(&bad24).unwrap_err().key, "chain.checksum");
        assert_eq!(words_from_qr("hello").unwrap_err().key, "chain.word_count");
    }

    #[test]
    fn id_does_not_change_and_hides_the_code() {
        let p = parse(&[ZERO8.to_string()]).unwrap();
        let id = chain_id(&p);
        assert_eq!(id, chain_id(&p));
        assert_eq!(id.len(), 16);
        assert!(!id.starts_with("00000000"));
        let m = parse(&[ZERO24.to_string()]).unwrap();
        let Phrase::Legacy(mm) = &m else { panic!() };
        let plain: String = mm.to_seed("")[..8]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_ne!(chain_id(&m), plain);
    }
}
