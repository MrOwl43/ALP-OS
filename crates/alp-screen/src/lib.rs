// SPDX-License-Identifier: AGPL-3.0-or-later
//! The native local screen's logic-free core (ADR 0028 #5): alpd client,
//! state model and texts. The GTK app (`src/main.rs`, feature `gui`) only
//! draws what these give and sends the user's actions back to alpd.

pub mod api;
pub mod i18n;
pub mod meter;
pub mod model;
pub mod paging;
pub mod start;
