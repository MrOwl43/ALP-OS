// SPDX-License-Identifier: AGPL-3.0-or-later
//! The event log behind the error and event manager (T58 audit, review
//! R5): what alpd itself did to a disk, in a fixed form.
//!
//! The events live in alpd's memory (the last [`KEEP`]), so nothing outside
//! alpd can add one: a local process could write a journal line tagged
//! like ours, but not into this list. Each event is also written to alpd's
//! own journal (`ALP-EVENT …`, unit alpd.service) for the boot log.
//! Never a password, a token, a file name or a label: only the fields of
//! [`Event`].

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

/// How many events are kept.
pub const KEEP: usize = 200;

/// One thing alpd did to a disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// Seconds since 1970 (UTC).
    pub at: u64,
    /// `disk.format`, `disk.check` or `disk.eject`.
    pub kind: &'static str,
    /// The disk's model, made safe for the screen.
    pub disk: String,
    pub size_bytes: u64,
    /// For a format: `quick` or `secure`, and the file system.
    pub mode: Option<&'static str>,
    pub fs: Option<&'static str>,
    /// `started`, `done`, `failed` or `stopped`.
    pub result: &'static str,
    /// The catalog code of a failure.
    pub code: Option<String>,
}

impl Event {
    pub fn json(&self) -> Value {
        json!({
            "at": self.at, "kind": self.kind, "disk": self.disk, "size_bytes": self.size_bytes,
            "mode": self.mode, "fs": self.fs, "result": self.result, "code": self.code,
        })
    }

    /// The journal line (fixed words only).
    pub fn line(&self) -> String {
        format!(
            "ALP-EVENT {} {} disk=\"{}\" size={} mode={} fs={} code={}",
            self.kind,
            self.result,
            self.disk,
            self.size_bytes,
            self.mode.unwrap_or("-"),
            self.fs.unwrap_or("-"),
            self.code.as_deref().unwrap_or("-"),
        )
    }
}

static LOG: Mutex<VecDeque<Event>> = Mutex::new(VecDeque::new());

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Add an event (the oldest goes beyond [`KEEP`]) and write its line.
pub fn record(mut e: Event) {
    e.disk = crate::fail::display_safe(&e.disk, 64).replace('"', "'");
    eprintln!("{}", e.line());
    let mut log = LOG
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if log.len() >= KEEP {
        log.pop_front();
    }
    log.push_back(e);
}

/// `GET live/events`: oldest first.
pub fn recent_json() -> Value {
    let log = LOG
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    json!({ "events": log.iter().map(Event::json).collect::<Vec<_>>() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(result: &'static str) -> Event {
        Event {
            at: 1,
            kind: "disk.format",
            disk: "SanDisk \"Extreme\"\u{202e}".into(),
            size_bytes: 10,
            mode: Some("quick"),
            fs: Some("exfat"),
            result,
            code: None,
        }
    }

    #[test]
    fn events_keep_a_fixed_form_and_the_last_200() {
        for _ in 0..(KEEP + 5) {
            record(ev("started"));
        }
        record(ev("done"));
        let v = recent_json();
        let list = v["events"].as_array().unwrap();
        assert_eq!(list.len(), KEEP);
        let last = list.last().unwrap();
        assert_eq!(last["result"], "done");
        // Device text is made safe; quotes cannot break the journal line.
        assert_eq!(last["disk"], "SanDisk 'Extreme'?");
        let keys: Vec<&String> = last.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            [
                "at",
                "code",
                "disk",
                "fs",
                "kind",
                "mode",
                "result",
                "size_bytes"
            ]
        );
        assert!(
            ev("failed")
                .line()
                .starts_with("ALP-EVENT disk.format failed disk=")
        );
    }
}
