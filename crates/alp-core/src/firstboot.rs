// SPDX-License-Identifier: AGPL-3.0-or-later
//! First-boot seed contract (ADR 0004, docs/contracts/firstboot-seed.md).
//!
//! Written by `alp-installer` to `paths().firstboot_seed` on the installed
//! `state` partition; read and then deleted by `alpd` when first boot
//! completes. TOML; serialization arrives with streams C and D.

/// One pool member as the user confirmed it in the installer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedMember {
    /// `/dev/disk/by-id` name at install time (display, first lookup).
    pub by_id: String,
    pub serial: Option<String>,
    pub size_bytes: u64,
    /// GPT partition UUID of the member's `alp-data` partition (created by
    /// the installer on dedicated data disks too, so it is known here).
    pub partition_uuid: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirstbootSeed {
    /// Schema version of this file; first boot refuses unknown versions.
    pub format_version: u32,
    pub installer_version: String,
    /// Hardware profile name, e.g. `generic-x86_64`.
    pub profile: String,
    /// `standard` in the beta (archive encryption is out of beta scope).
    pub protection_tier: String,
    /// The pool the user confirmed on the installer's Confirm screen.
    pub members: Vec<SeedMember>,
    /// When the user confirmed (RFC 3339, UTC).
    pub confirmed_at: String,
    /// `alp_hw::PlanDigest` of the confirmed pool plan.
    pub plan_digest: [u8; 32],
}

/// Current seed schema version.
pub const SEED_FORMAT_VERSION: u32 = 1;
