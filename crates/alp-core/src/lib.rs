// SPDX-License-Identifier: AGPL-3.0-or-later
//! ALP-OS daemon (alpd): event loop, API server, orchestration glue.
//!
//! Today: the contracts in [`state`] (ADR 0004) and, for ALP-OS Live, the
//! console listener ([`console`]) with the live API ([`live_api`]).

pub mod console;
pub mod firstboot;
pub mod live_api;
pub mod live_media;
pub mod state;
