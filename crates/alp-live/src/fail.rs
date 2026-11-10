// SPDX-License-Identifier: AGPL-3.0-or-later
//! Errors the user sees: a stable key (docs/plans/live-alpha-api.md) and a
//! plain sentence, and a catalog code (errors/catalog.toml) where one exists.

use std::collections::HashMap;
use std::fmt;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fail {
    /// Stable machine key, e.g. `wifi.password_refused`.
    pub key: &'static str,
    /// Plain sentence ready to show.
    pub message: String,
    /// 1-based position of the wrong item (chain words), if any.
    pub index: Option<usize>,
    /// Seconds to wait before trying again (attempt limits).
    pub retry_after: Option<u64>,
    /// Catalog code (`errors/catalog.toml`), e.g. `ALP-STO-100`.
    pub code: Option<&'static str>,
}

impl Fail {
    pub fn new(key: &'static str, message: impl Into<String>) -> Self {
        Self {
            key,
            message: message.into(),
            index: None,
            retry_after: None,
            code: None,
        }
    }

    /// The same error with its catalog code.
    pub fn with_code(mut self, code: &'static str) -> Self {
        self.code = Some(code);
        self
    }
}

/// The four levels of the (i) icon (ADR 0023), least to most serious.
pub const LEVELS: [&str; 4] = ["note", "action", "now", "critical"];

/// The level of each catalog code, read once from the catalog compiled in.
fn catalog_levels() -> &'static HashMap<String, &'static str> {
    static MAP: OnceLock<HashMap<String, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| {
        let t: toml::Table = include_str!("../../../errors/catalog.toml")
            .parse()
            .unwrap_or_default();
        t.iter()
            .filter_map(|(code, v)| {
                let sev = v.get("severity")?.as_str()?;
                let lvl = LEVELS.iter().find(|l| **l == sev)?;
                Some((code.clone(), *lvl))
            })
            .collect()
    })
}

/// The level of a catalog code (None: not in the catalog).
pub fn code_level(code: &str) -> Option<&'static str> {
    catalog_levels().get(code).copied()
}

impl Fail {
    /// How serious this error is for the (i) icon: the catalog code's level,
    /// else a level by key (an answer to what the person just typed is only
    /// a note), else `action`.
    pub fn level(&self) -> &'static str {
        if let Some(l) = self.code.and_then(code_level) {
            return l;
        }
        match self.key {
            "password.wrong"
            | "password.wait"
            | "request.json"
            | "console.locked"
            | "format.confirm_stale"
            | "archive.busy"
            | "chain.wait"
            | "recovery.wrong" => "note",
            "space.full" => "now",
            "archive.critical" => "critical",
            _ => "action",
        }
    }
}

impl fmt::Display for Fail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Fail {}

/// Text from devices or the air (Wi-Fi names, disk models, labels) made
/// safe for the screen (review S12): control characters (C0, C1), bidi
/// overrides and zero-width characters become `?`; at most `max` chars.
pub fn display_safe(s: &str, max: usize) -> String {
    s.chars()
        .map(|c| {
            let invisible = matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}');
            if c.is_control() || invisible { '?' } else { c }
        })
        .take(max)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_safe_removes_tricks() {
        assert_eq!(
            super::display_safe("Home\x1b[2J\u{202E}x", 40),
            "Home?[2J?x"
        );
        assert_eq!(super::display_safe("abcdef", 3), "abc");
    }

    #[test]
    fn levels_come_from_the_catalog_then_the_key() {
        assert_eq!(code_level("ALP-STO-111"), Some("critical"));
        assert_eq!(code_level("ALP-STO-124"), Some("action"));
        assert_eq!(code_level("not-a-code"), None);
        // Every catalog entry has a known level.
        assert!(catalog_levels().len() > 60);
        let f = Fail::new("archive.refused", "x").with_code("ALP-STO-101");
        assert_eq!(f.level(), "action");
        assert_eq!(Fail::new("password.wrong", "x").level(), "note");
        assert_eq!(Fail::new("space.full", "x").level(), "now");
        assert_eq!(Fail::new("archive.critical", "x").level(), "critical");
        assert_eq!(Fail::new("whatever", "x").level(), "action");
    }
}
