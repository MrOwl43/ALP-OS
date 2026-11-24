// SPDX-License-Identifier: AGPL-3.0-or-later
//! What the screen needs from alpd's state (`GET live/state`), read from the
//! same wire fields the web console normalises (`ui/live/api.js`). Missing
//! fields mean an older alpd and fall back to safe defaults.

use serde_json::Value;

/// The worst alert level (ADR 0023): ok < note < action < now < critical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Ok,
    Note,
    Action,
    Now,
    Critical,
}

impl Level {
    fn from_wire(s: &str) -> Self {
        match s {
            "note" => Level::Note,
            "action" => Level::Action,
            "now" => Level::Now,
            "critical" => Level::Critical,
            _ => Level::Ok,
        }
    }

    /// The user must do something (not only read a note).
    pub fn needs_user(self) -> bool {
        self >= Level::Action
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Screen {
    /// The console session is open (signed in).
    pub open: bool,
    /// The index waits for the ALP password (first unlock after start).
    pub index_locked: bool,
    /// Seconds to wait after wrong passwords.
    pub wait: u64,
    pub setup_finished: bool,
    pub level: Level,
    /// alpd's plain message for the worst alert, if any.
    pub message: Option<String>,
    pub alerts: usize,
}

impl Screen {
    pub fn from_state(w: &Value) -> Self {
        let status = &w["status"];
        Screen {
            open: w["console"]["state"].as_str().unwrap_or("open") == "open",
            index_locked: w["lock"]["state"].as_str() == Some("locked"),
            wait: w["lock"]["retry_after_seconds"].as_u64().unwrap_or(0),
            setup_finished: w["setup"]["finished"].as_bool().unwrap_or(false),
            level: Level::from_wire(status["level"].as_str().unwrap_or("ok")),
            message: status["message"].as_str().map(str::to_string),
            alerts: w["alerts"].as_array().map_or(0, Vec::len),
        }
    }

    /// Text keys for the home title and line: "All good" unless the user
    /// must act (owner, 2026-11-24).
    pub fn home_keys(&self) -> (&'static str, Option<&'static str>) {
        if self.level.needs_user() {
            ("home.title.attn", None)
        } else {
            ("lock.msg.ok", Some("home.sub.ok"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn older_alpd_means_safe_defaults() {
        let s = Screen::from_state(&json!({}));
        assert!(s.open);
        assert_eq!(s.level, Level::Ok);
        assert!(!s.setup_finished);
        assert_eq!(s.home_keys().0, "lock.msg.ok");
    }

    #[test]
    fn reads_lock_and_worst_alert() {
        let s = Screen::from_state(&json!({
            "console": {"state": "locked"},
            "lock": {"state": "locked", "retry_after_seconds": 30},
            "setup": {"finished": true},
            "status": {"level": "action", "message": "Connect the archive disk."},
            "alerts": [{}, {}]
        }));
        assert!(!s.open && s.index_locked);
        assert_eq!(s.wait, 30);
        assert_eq!(s.level, Level::Action);
        assert_eq!(s.message.as_deref(), Some("Connect the archive disk."));
        assert_eq!(s.alerts, 2);
        assert_eq!(s.home_keys().0, "home.title.attn");
    }

    #[test]
    fn a_note_is_still_all_good() {
        let s = Screen::from_state(&json!({"status": {"level": "note"}}));
        assert!(!s.level.needs_user());
        assert_eq!(s.home_keys().0, "lock.msg.ok");
    }
}
