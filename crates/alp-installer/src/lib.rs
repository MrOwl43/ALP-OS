// SPDX-License-Identifier: AGPL-3.0-or-later
//! Installer logic (library) for the 0.2.8 installer (spec block 7,
//! ADR 0028, plan `docs/plans/0.2.8-installer.md`).
//!
//! Disk discovery lives in `alp_hw::inspect::blockdev` (stream rule 12).
//! This crate turns the disk list and the user's choice into an explicit
//! install plan, and the executor carries it out through [`run::Runner`]
//! (unit tests use a recording mock; real disks only in the VM tests).

pub mod capacity;
pub mod exec;
pub mod plan;
pub mod run;
pub mod seed;
