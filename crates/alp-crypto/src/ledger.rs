// SPDX-License-Identifier: AGPL-3.0-or-later
//! Audit ledger (spec block 6): `append`, `verify`, `export_range`.
//!
//! Append-only table in the `alpd` SQLite database on `state`. Each entry:
//! `seq`, `timestamp`, `event_type`, canonical versioned `payload`,
//! `prev_hash`, `hash = SHA3-256(prev_hash || canonical_entry)`, and an
//! Ed25519 signature by the node identity. Events never contain secrets,
//! file names or file contents. Implementation: block 6 (stream C).

use std::path::Path;

use crate::{Identity, Result};

/// Events recorded in v1 (spec block 6 plus the console lock of block 8).
/// Adding an event is not a contract change; changing one is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerEvent {
    FirstBoot,
    DevicePaired {
        device_id: String,
    },
    DeviceRevoked {
        device_id: String,
    },
    ServiceEnabled {
        service: String,
    },
    ServiceDisabled {
        service: String,
    },
    UpdateApplied {
        version: String,
    },
    UpdateRolledBack {
        from_version: String,
        to_version: String,
    },
    PoolCreated {
        pool_id: String,
    },
    PoolExpanded {
        pool_id: String,
    },
    DiskReplaced {
        pool_id: String,
    },
    KeyRotated,
    RecoveryExported,
    AdvancedModeToggled {
        enabled: bool,
    },
    AdminLogin {
        via: String,
    },
    ConsoleUnlocked {
        via: String,
    },
    ConsoleUnlockFailed {
        attempt: u32,
    },
    /// Written when the user confirms a destructive operation, before it
    /// runs (spec Section 2 rule 8). `action` is e.g. "create-pool",
    /// "expand-pool", "replace-disk"; `plan_digest` is
    /// `alp_hw::PlanDigest::plan_digest` of the confirmed plan.
    OperationConfirmed {
        action: String,
        plan_digest: [u8; 32],
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerEntry {
    pub seq: u64,
    pub timestamp_unix: i64,
    pub event: LedgerEvent,
    pub prev_hash: [u8; 32],
    pub hash: [u8; 32],
    pub signature: [u8; 64],
}

/// Result of [`verify`]: the first bad sequence number, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyReport {
    pub entries_checked: u64,
    pub first_bad_seq: Option<u64>,
}

/// Handle to the ledger table in the `alpd` database
/// (`alp_core::state::paths().database`).
#[derive(Debug)]
pub struct Ledger {
    _private: (),
}

impl Ledger {
    pub fn open(_database: &Path) -> Result<Self> {
        todo!("block 6")
    }
}

/// Append an event, signed by the node identity. Safe under concurrent
/// appends (block 6 acceptance). Returns the new sequence number.
pub fn append(_ledger: &Ledger, _identity: &Identity, _event: &LedgerEvent) -> Result<u64> {
    todo!("block 6")
}

/// Check the whole chain and every signature against the node's public key.
pub fn verify(_ledger: &Ledger, _public_key: &[u8; 32]) -> Result<VerifyReport> {
    todo!("block 6")
}

/// Entries with `from <= seq <= to`, e.g. for the daily mirror to the pool.
pub fn export_range(_ledger: &Ledger, _from: u64, _to: u64) -> Result<Vec<LedgerEntry>> {
    todo!("block 6")
}
