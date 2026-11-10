// SPDX-License-Identifier: AGPL-3.0-or-later
//! ALP-OS Live (ADR 0006, 0007, 0009, 0010): the logic behind `alpctl live …`
//! and the console API of `alpd` (`docs/plans/live-alpha-api.md`).

pub mod admin;
pub mod archive;
pub mod chain;
pub mod disks;
pub mod display;
pub mod events;
pub mod fail;
pub mod format;
pub mod http;
pub mod immich;
pub mod index;
pub mod job;
pub mod live;
pub mod media;
pub mod pairing;
pub mod prefs;
pub mod qr;
pub mod recovery;
pub mod tls;
pub mod vault;
pub mod wifi;
pub mod wipe;
