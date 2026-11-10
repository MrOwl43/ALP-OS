// SPDX-License-Identifier: AGPL-3.0-or-later
//! Wrap-key sealing: the `KeySealBackend` trait of spec Section 5.
//! Re-exported by `alp-hw`. Backend implementations arrive with block 5.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use zeroize::Zeroizing;

use crate::{CryptoError, Result, Secret};

/// Which backend sealed a blob.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SealBackendKind {
    Tpm2Seal,
    PassphraseOrKeyfileSeal,
}

impl SealBackendKind {
    fn tag(self) -> u8 {
        match self {
            Self::Tpm2Seal => 1,
            Self::PassphraseOrKeyfileSeal => 2,
        }
    }

    fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            1 => Some(Self::Tpm2Seal),
            2 => Some(Self::PassphraseOrKeyfileSeal),
            _ => None,
        }
    }
}

/// A secret sealed by a [`KeySealBackend`], as stored on `state`.
///
/// Treat as secret: with `PassphraseOrKeyfileSeal` in keyfile mode (no TPM)
/// the blob *is* the wrap key (spec block 5). Its bytes never leave this
/// crate: they are wiped on drop, never printed, and written or read only by
/// [`SealedBlob::store`] and [`SealedBlob::load`].
pub struct SealedBlob {
    kind: SealBackendKind,
    bytes: Zeroizing<Vec<u8>>,
}

const MAGIC: &[u8; 8] = b"ALPSEAL1";

/// Reading a stored blob failed: the server cannot open the pool unattended.
fn read_error(detail: String) -> CryptoError {
    CryptoError {
        code: "ALP-KEY-002",
        detail,
    }
}

/// Saving a blob failed (full or failing system disk, permissions).
fn write_error(detail: String) -> CryptoError {
    CryptoError {
        code: "ALP-KEY-004",
        detail,
    }
}

impl SealedBlob {
    #[allow(dead_code)] // used by the block 5 backends
    pub(crate) fn new(kind: SealBackendKind, bytes: Vec<u8>) -> Self {
        Self {
            kind,
            bytes: Zeroizing::new(bytes),
        }
    }

    /// Which backend sealed the blob.
    pub fn kind(&self) -> SealBackendKind {
        self.kind
    }

    /// Write the blob to `path` (mode 0600) crash-safely: temporary file in
    /// the same directory, fsync, rename over `path`, fsync the directory.
    pub fn store(&self, path: &Path) -> Result<()> {
        let dir = path
            .parent()
            .ok_or_else(|| write_error(format!("{} has no parent directory", path.display())))?;
        let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
        tmp_name.push(".tmp");
        let tmp = path.with_file_name(tmp_name);
        let write = || -> std::io::Result<()> {
            let mut f = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)?;
            f.write_all(MAGIC)?;
            f.write_all(&[self.kind.tag()])?;
            f.write_all(&self.bytes)?;
            f.sync_all()?;
            fs::rename(&tmp, path)?;
            File::open(dir)?.sync_all()
        };
        write().map_err(|e| {
            let _ = fs::remove_file(&tmp);
            write_error(format!("storing sealed blob at {}: {e}", path.display()))
        })
    }

    /// Read a blob written by [`SealedBlob::store`].
    pub fn load(path: &Path) -> Result<Self> {
        let read = || -> std::io::Result<Zeroizing<Vec<u8>>> {
            let mut f = File::open(path)?;
            let len = usize::try_from(f.metadata()?.len()).unwrap_or(usize::MAX);
            // Sized up front so reading never reallocates and leaves
            // unwiped copies of the key bytes behind.
            let mut raw = Zeroizing::new(Vec::with_capacity(len.saturating_add(1)));
            f.read_to_end(&mut raw)?;
            Ok(raw)
        };
        let raw = read()
            .map_err(|e| read_error(format!("reading sealed blob {}: {e}", path.display())))?;
        if raw.len() < MAGIC.len() + 1 || &raw[..MAGIC.len()] != MAGIC {
            return Err(read_error(format!(
                "{} is not a sealed blob",
                path.display()
            )));
        }
        let kind = SealBackendKind::from_tag(raw[MAGIC.len()])
            .ok_or_else(|| read_error(format!("{}: unknown sealing backend", path.display())))?;
        Ok(Self::new(kind, raw[MAGIC.len() + 1..].to_vec()))
    }
}

impl fmt::Debug for SealedBlob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "SealedBlob {{ kind: {:?}, len: {} }}",
            self.kind,
            self.bytes.len()
        )
    }
}

/// Whether a sealing backend can be used on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SealCapability {
    Available,
    /// Not usable here; `reason` is technician detail (e.g. "no TPM2 device").
    Unavailable {
        reason: String,
    },
}

/// Seals and unseals the wrap key (spec Section 5).
/// v1 implementations: [`Tpm2Seal`], [`PassphraseOrKeyfileSeal`].
pub trait KeySealBackend: Send + Sync {
    fn available(&self) -> SealCapability;
    fn seal(&self, secret: &Secret) -> Result<SealedBlob>;
    fn unseal(&self, blob: &SealedBlob) -> Result<Secret>;
}

/// v1 backend: wrap key sealed to TPM2 with a policy that survives A/B
/// updates (block 5; VERIFY the PCR policy in an ADR). Stub.
#[derive(Debug, Default)]
pub struct Tpm2Seal;

/// v1 backend: wrap key as a keyfile on `state`, or protected by a
/// passphrase in hardened mode (block 5). Stub.
#[derive(Debug, Default)]
pub struct PassphraseOrKeyfileSeal;

impl KeySealBackend for Tpm2Seal {
    fn available(&self) -> SealCapability {
        todo!("block 5")
    }
    fn seal(&self, _secret: &Secret) -> Result<SealedBlob> {
        todo!("block 5")
    }
    fn unseal(&self, _blob: &SealedBlob) -> Result<Secret> {
        todo!("block 5")
    }
}

impl KeySealBackend for PassphraseOrKeyfileSeal {
    fn available(&self) -> SealCapability {
        todo!("block 5")
    }
    fn seal(&self, _secret: &Secret) -> Result<SealedBlob> {
        todo!("block 5")
    }
    fn unseal(&self, _blob: &SealedBlob) -> Result<Secret> {
        todo!("block 5")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("alp-seal-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn sealed_blob_debug_shows_only_kind_and_length() {
        let b = SealedBlob::new(
            SealBackendKind::PassphraseOrKeyfileSeal,
            b"wrap-key-bytes".to_vec(),
        );
        let shown = format!("{b:?}");
        assert_eq!(
            shown,
            "SealedBlob { kind: PassphraseOrKeyfileSeal, len: 14 }"
        );
        assert!(!shown.contains("wrap-key"));
    }

    #[test]
    fn store_and_load_round_trip_with_mode_0600() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("wrap.sealed");
        let b = SealedBlob::new(SealBackendKind::Tpm2Seal, vec![1, 2, 3, 4]);
        b.store(&path).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(!dir.join("wrap.sealed.tmp").exists());
        let back = SealedBlob::load(&path).unwrap();
        assert_eq!(back.kind(), SealBackendKind::Tpm2Seal);
        assert_eq!(back.bytes.as_slice(), &[1, 2, 3, 4]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn load_refuses_files_that_are_not_sealed_blobs() {
        let dir = temp_dir("garbage");
        let path = dir.join("other");
        fs::write(&path, b"not a blob").unwrap();
        assert_eq!(SealedBlob::load(&path).unwrap_err().code, "ALP-KEY-002");
        fs::remove_dir_all(dir).unwrap();
    }
}
