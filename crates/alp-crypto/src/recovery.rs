// SPDX-License-Identifier: AGPL-3.0-or-later
//! Recovery key and encrypted state backup (spec blocks 3 and 5).
//! Implementations arrive with block 5 (VERIFY the word-list crate).

use std::fmt;
use std::path::Path;

use zeroize::Zeroizing;

use crate::Result;

/// The recovery key (random 256-bit): second LUKS keyslot and key for the
/// encrypted `state` backup (HKDF with distinct contexts). Never printed.
pub struct RecoveryKey {
    _private: (),
}

impl fmt::Debug for RecoveryKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RecoveryKey(<redacted>)")
    }
}

/// The recovery key as shown to the user: seed-phrase words and QR.
/// Showing it is an explicit, ledger-recorded action (`export_recovery`).
pub struct RecoverySheet {
    words: Zeroizing<Vec<String>>,
}

impl RecoverySheet {
    pub fn words(&self) -> &[String] {
        &self.words
    }

    /// The same key as a QR code (SVG), for the printable sheet.
    pub fn qr_svg(&self) -> String {
        todo!("block 5")
    }
}

impl fmt::Debug for RecoverySheet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "RecoverySheet {{ words: <{} redacted> }}",
            self.words.len()
        )
    }
}

/// Create the recovery key on first boot (block 5).
pub fn create_recovery() -> Result<RecoveryKey> {
    todo!("block 5")
}

/// The words and QR for the recovery sheet. Callers must have the user's
/// explicit confirmation and record `RecoveryExported` in the ledger.
pub fn export_recovery(_key: &RecoveryKey) -> RecoverySheet {
    todo!("block 5")
}

/// Rebuild the recovery key from the words the user typed; fails with a
/// catalog code if a word or the checksum is wrong.
pub fn verify_recovery(_words: &[String]) -> Result<RecoveryKey> {
    todo!("block 5")
}

/// Encrypt the contents of the `state` directory into `out` (on the pool's
/// `@system-backup` subvolume) with a key derived from the recovery key.
pub fn backup_state(_state_dir: &Path, _key: &RecoveryKey, _out: &Path) -> Result<()> {
    todo!("blocks 3 and 5")
}

/// Restore `state` from a backup written by [`backup_state`] (block 3
/// acceptance: same node identity and a mountable pool).
pub fn restore_state(_backup: &Path, _key: &RecoveryKey, _state_dir: &Path) -> Result<()> {
    todo!("blocks 3 and 5")
}
