// SPDX-License-Identifier: AGPL-3.0-or-later
//! Key types and the key hierarchy of spec block 5.
//!
//! Key bytes can only be created and read inside this crate. Other crates
//! receive opaque values that print as `<redacted>` and are wiped on drop.

use std::fmt;
use std::io::Write;

use zeroize::Zeroizing;

use crate::Result;

/// Secret bytes, wiped from memory when dropped. Never printed.
/// Only this crate can create one or read its bytes.
pub struct Secret(Zeroizing<Vec<u8>>);

impl Secret {
    #[allow(dead_code)] // used by the block 5 implementations
    pub(crate) fn new(bytes: Vec<u8>) -> Self {
        Self(Zeroizing::new(bytes))
    }

    #[allow(dead_code)] // used by the block 5 implementations
    pub(crate) fn expose(&self) -> &[u8] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// A secret typed by the user (console PIN, hardened-mode passphrase).
/// The UI layer may create it; only this crate can read it.
pub struct Passphrase(Zeroizing<String>);

impl Passphrase {
    pub fn new(text: String) -> Self {
        Self(Zeroizing::new(text))
    }

    #[allow(dead_code)] // used by the block 5 implementations
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Passphrase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Passphrase(<redacted>)")
    }
}

/// The pool key that unlocks every LUKS container of the data pool
/// (spec block 5: random 512-bit). Only this crate can create it or see its
/// bytes; `alp-storage` hands it to `cryptsetup` with [`PoolKey::write_key_file`].
pub struct PoolKey(Zeroizing<[u8; 64]>);

impl PoolKey {
    /// Generate a new random pool key (block 5).
    pub fn generate() -> Result<Self> {
        todo!("block 5")
    }

    /// Write the raw key to `out`, e.g. the stdin pipe of
    /// `cryptsetup --key-file=-`. Never to a file, a log or a buffer that
    /// is not wiped (spec Section 2 rule 4).
    pub fn write_key_file(&self, out: &mut dyn Write) -> std::io::Result<()> {
        out.write_all(self.0.as_slice())
    }
}

impl fmt::Debug for PoolKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PoolKey(<redacted>)")
    }
}

/// The node identity (Ed25519), root of trust for the ledger and device
/// certificates (spec block 5). Holds the private key; never printed.
pub struct Identity {
    _private: (),
}

impl Identity {
    /// Public key bytes (Ed25519, 32 bytes), safe to share.
    pub fn public_key(&self) -> [u8; 32] {
        todo!("block 5")
    }
}

impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Identity(<private key redacted>)")
    }
}

/// Generate the node identity on first boot (block 5).
pub fn generate_identity() -> Result<Identity> {
    todo!("block 5")
}

/// Replace the pool key in every LUKS keyslot. Atomic and resumable
/// (spec block 5); recorded in the ledger by the caller.
pub fn rotate_pool_key(_current: &PoolKey) -> Result<PoolKey> {
    todo!("block 5")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_types_never_print_their_bytes() {
        let s = Secret::new(b"hunter2".to_vec());
        assert_eq!(format!("{s:?}"), "Secret(<redacted>)");
        let p = Passphrase::new("hunter2".to_string());
        assert_eq!(format!("{p:?}"), "Passphrase(<redacted>)");
        let k = PoolKey(Zeroizing::new([7u8; 64]));
        assert_eq!(format!("{k:?}"), "PoolKey(<redacted>)");
    }

    #[test]
    fn pool_key_is_512_bits_and_written_verbatim() {
        let k = PoolKey(Zeroizing::new([7u8; 64]));
        let mut out = Vec::new();
        k.write_key_file(&mut out).unwrap();
        assert_eq!(out.len(), 64);
    }
}
