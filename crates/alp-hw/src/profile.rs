// SPDX-License-Identifier: AGPL-3.0-or-later
//! Hardware profile schema (contract, ADR 0004): the TOML files in
//! `profiles/` (v1: `generic-x86_64.toml`). The loader arrives with the
//! `alpd` skeleton; this struct is the schema every stream reads.

use crate::SnapshotPolicy;

/// Implementation names per trait, e.g. `boot = "UefiSystemdBoot"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backends {
    pub boot: String,
    pub key_seal: Vec<String>,
    pub storage: String,
    pub power: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HardwareProfile {
    pub name: String,
    pub architecture: String,
    pub backends: Backends,
    /// Installer minimums (spec Section 3; owner decision for single disk).
    pub min_ram_bytes: u64,
    pub min_system_disk_bytes: u64,
    pub min_single_disk_bytes: u64,
    /// Snapshot retention (spec block 4).
    pub snapshots: SnapshotPolicy,
    /// Raise `ALP-STO-003` below this much free space, in percent.
    pub capacity_low_percent: u8,
    /// Days between scheduled scrubs (spec block 4: monthly).
    pub scrub_interval_days: u32,
}
