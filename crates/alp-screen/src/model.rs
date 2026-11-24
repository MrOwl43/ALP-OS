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

/// One alert as the screen lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct Alert {
    pub level: Level,
    pub key: String,
    pub message: String,
    pub code: Option<String>,
}

/// The wire `alerts` list, worst first (same level: alpd's order).
pub fn alert_list(w: &Value) -> Vec<Alert> {
    let mut list: Vec<Alert> = w["alerts"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|v| v.is_object())
                .map(|v| Alert {
                    level: Level::from_wire(v["level"].as_str().unwrap_or("ok")),
                    key: v["key"].as_str().unwrap_or_default().to_string(),
                    message: v["message"].as_str().unwrap_or_default().to_string(),
                    code: v["code"].as_str().map(str::to_string),
                })
                .collect()
        })
        .unwrap_or_default();
    list.sort_by_key(|a| std::cmp::Reverse(a.level));
    list
}

/// What the settings page shows (read-only; actions go to alpd).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Settings {
    pub release: Option<String>,
    pub build: Option<String>,
    /// The computer's clock, seconds since 1970 (UTC), and whether the
    /// network set it.
    pub clock: Option<(u64, bool)>,
    pub update_supported: bool,
    pub update_current: Option<String>,
    /// A newer version that can be installed, if one was found.
    pub update_newer: Option<String>,
    /// Paired phones: (serial, name, model).
    pub devices: Vec<(String, String, String)>,
}

impl Settings {
    pub fn from_state(w: &Value) -> Self {
        let s = |v: &Value| v.as_str().map(str::to_string);
        let u = &w["update"];
        Settings {
            release: s(&w["system"]["release"]),
            build: s(&w["system"]["build"]),
            clock: w["clock"]["now"]
                .as_u64()
                .map(|n| (n, w["clock"]["synced"].as_bool().unwrap_or(false))),
            update_supported: u["supported"].as_bool().unwrap_or(false),
            update_current: s(&u["current"]),
            update_newer: s(&u["newer"]["version"]),
            devices: w["pairing"]["devices"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter(|d| !d["revoked"].as_bool().unwrap_or(false))
                        .filter_map(|d| {
                            Some((
                                s(&d["serial"])?,
                                s(&d["name"]).unwrap_or_default(),
                                s(&d["model"]).unwrap_or_default(),
                            ))
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn alerts_worst_first_keeping_order() {
        let w = json!({"alerts": [
            {"level": "note", "key": "a", "message": "A"},
            "junk",
            {"level": "now", "key": "b", "message": "B", "code": "ALP-INST-001"},
            {"level": "note", "key": "c", "message": "C"},
            {"level": "strange", "key": "d"}
        ]});
        let l = alert_list(&w);
        let keys: Vec<&str> = l.iter().map(|a| a.key.as_str()).collect();
        assert_eq!(keys, ["b", "a", "c", "d"]);
        assert_eq!(l[0].code.as_deref(), Some("ALP-INST-001"));
        assert_eq!(l[3].level, Level::Ok);
        assert!(alert_list(&json!({})).is_empty());
    }

    #[test]
    fn settings_from_state() {
        let w = json!({
            "system": {"release": "ALP-OS 0.2.8", "build": "abc 2026-11-24"},
            "clock": {"now": 1_795_000_000u64, "synced": true},
            "update": {"supported": true, "current": "0.2.8", "newer": {"version": "0.2.9", "state": "ready"}},
            "pairing": {"devices": [
                {"serial": "10AB", "name": "Phone", "model": "Pixel"},
                {"serial": "10CD", "name": "Old", "revoked": true},
                {"name": "no serial"}
            ]}
        });
        let s = Settings::from_state(&w);
        assert_eq!(s.release.as_deref(), Some("ALP-OS 0.2.8"));
        assert_eq!(s.clock, Some((1_795_000_000, true)));
        assert_eq!(s.update_newer.as_deref(), Some("0.2.9"));
        assert_eq!(
            s.devices,
            vec![("10AB".into(), "Phone".into(), "Pixel".into())]
        );
        assert_eq!(Settings::from_state(&json!({})), Settings::default());
    }

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
