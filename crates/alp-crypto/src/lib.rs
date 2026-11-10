// SPDX-License-Identifier: AGPL-3.0-or-later
//! Identity, key hierarchy, sealing, recovery, ledger primitives.
//!
//! Contract (ADR 0004). This crate is the only one that creates, derives or
//! reads key material (spec Section 2 rule 6). Other crates hold keys only as
//! the opaque types below and pass them back to this crate's functions.
//! Modules are split so that streams B (keys, recovery) and C (ledger) do not
//! edit the same files. Implementations arrive with blocks 5 and 6.

use std::fmt;

pub mod keys;
pub mod ledger;
pub mod recovery;
pub mod seal;

pub use keys::{Identity, Passphrase, PoolKey, Secret};
pub use seal::{
    KeySealBackend, PassphraseOrKeyfileSeal, SealBackendKind, SealCapability, SealedBlob, Tpm2Seal,
};

/// Error from a key or ledger operation. `code` is a user-facing error code
/// from `errors/catalog.toml`; `detail` is technician detail and must never
/// contain key material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CryptoError {
    pub code: &'static str,
    pub detail: String,
}

impl fmt::Display for CryptoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.detail)
    }
}

impl std::error::Error for CryptoError {}

pub type Result<T> = std::result::Result<T, CryptoError>;
