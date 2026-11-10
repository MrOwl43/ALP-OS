// SPDX-License-Identifier: AGPL-3.0-or-later
//! Console preferences (T56): a few plain settings kept on the stick, next
//! to `setup-done`, because the lock screen needs the scale before anything
//! is unlocked. Not secret. Only listed values are taken, both from the API
//! and when the file is read back (a hand-edited file can only choose a
//! listed value). Start over removes the file.

use std::fs;
use std::io::Write;
use std::path::Path;

use serde_json::{Value, json};

/// Where the preferences are kept (the stick's plain state).
pub const PATH: &str = "/var/lib/alp/live/prefs.json";

/// The UI scales offered, in percent (`ui_scale` 1, 1.25 … 2).
pub const SCALES: [u16; 5] = [100, 125, 150, 175, 200];

/// "Lock after" choices in seconds (no "never": Security, 2026-11-08).
pub const IDLE_LOCKS: [u32; 4] = [60, 300, 900, 1800];
pub const IDLE_LOCK_DEFAULT: u32 = 300;

/// The home screen backgrounds. Only names of images shipped in the image,
/// never a URL (review, T56).
pub const BACKGROUNDS: [&str; 1] = ["blue"];

/// A screen mode: pixels and refresh rate in millihertz.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mode {
    pub width: u32,
    pub height: u32,
    pub refresh_mhz: u32,
}

impl Mode {
    /// Sane bounds for a mode read back from the file.
    fn sane(self) -> bool {
        (320..=16384).contains(&self.width)
            && (200..=16384).contains(&self.height)
            && (1_000..=1_000_000).contains(&self.refresh_mhz)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prefs {
    pub natural_scrolling: bool,
    /// Percent from [`SCALES`]; None: automatic.
    pub ui_scale: Option<u16>,
    pub home_background: &'static str,
    /// Seconds without use before the console locks, from [`IDLE_LOCKS`].
    pub idle_lock_seconds: u32,
    /// The screen mode the user kept, with the identity hash of the screen
    /// it was kept on (review N-T56-2): applied only on that screen.
    pub mode: Option<(Mode, String)>,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            natural_scrolling: true,
            ui_scale: None,
            home_background: BACKGROUNDS[0],
            idle_lock_seconds: IDLE_LOCK_DEFAULT,
            mode: None,
        }
    }
}

/// `ui_scale` from JSON: null is automatic, else one of [`SCALES`].
fn scale_of(v: &Value) -> Option<Option<u16>> {
    if v.is_null() {
        return Some(None);
    }
    let f = v.as_f64()? * 100.0;
    SCALES
        .iter()
        .find(|s| (f - f64::from(**s)).abs() < 0.01)
        .map(|s| Some(*s))
}

fn idle_lock_of(v: &Value) -> Option<u32> {
    let n = u32::try_from(v.as_u64()?).ok()?;
    IDLE_LOCKS.contains(&n).then_some(n)
}

fn background_of(v: &Value) -> Option<&'static str> {
    let s = v.as_str()?;
    BACKGROUNDS.iter().find(|b| **b == s).copied()
}

fn mode_of(v: &Value) -> Option<(Mode, String)> {
    let n = |k: &str| v.get(k)?.as_u64().and_then(|n| u32::try_from(n).ok());
    let m = Mode {
        width: n("width")?,
        height: n("height")?,
        refresh_mhz: n("refresh_mhz")?,
    };
    let screen = v.get("screen")?.as_str()?;
    let hex = screen.len() == 64 && screen.bytes().all(|b| b.is_ascii_hexdigit());
    (m.sane() && hex).then(|| (m, screen.to_ascii_lowercase()))
}

impl Prefs {
    /// Read back from the file's text: each field is checked again, and a
    /// wrong or missing one takes its default.
    pub fn parse(text: &str) -> Self {
        let d = Self::default();
        let Ok(v) = serde_json::from_str::<Value>(text) else {
            return d;
        };
        Self {
            natural_scrolling: v
                .get("natural_scrolling")
                .and_then(Value::as_bool)
                .unwrap_or(d.natural_scrolling),
            ui_scale: v.get("ui_scale").and_then(scale_of).unwrap_or(d.ui_scale),
            home_background: v
                .get("home_background")
                .and_then(background_of)
                .unwrap_or(d.home_background),
            idle_lock_seconds: v
                .get("idle_lock_seconds")
                .and_then(idle_lock_of)
                .unwrap_or(d.idle_lock_seconds),
            mode: v.get("mode").and_then(mode_of),
        }
    }

    /// `state.prefs`.
    pub fn json(&self) -> Value {
        json!({
            "natural_scrolling": self.natural_scrolling,
            "ui_scale": self.ui_scale.map(|s| f64::from(s) / 100.0),
            "home_background": self.home_background,
            "idle_lock_seconds": self.idle_lock_seconds,
        })
    }

    /// The file's content (adds the kept mode).
    fn file_json(&self) -> Value {
        let mut v = self.json();
        v["mode"] = match &self.mode {
            Some((m, screen)) => json!({"width": m.width, "height": m.height,
                "refresh_mhz": m.refresh_mhz, "screen": screen}),
            None => Value::Null,
        };
        v
    }

    /// `POST prefs`: any subset of the known keys. Everything is checked
    /// first; one wrong key or value changes nothing.
    pub fn apply(&mut self, body: &Value) -> Result<(), String> {
        let obj = body.as_object().ok_or("The request is not valid JSON.")?;
        let mut next = self.clone();
        for (k, v) in obj {
            match k.as_str() {
                "natural_scrolling" => {
                    next.natural_scrolling = v
                        .as_bool()
                        .ok_or("\"natural_scrolling\" must be true or false.")?;
                }
                "ui_scale" => {
                    next.ui_scale =
                        scale_of(v).ok_or("\"ui_scale\" must be 1, 1.25, 1.5, 1.75, 2 or null.")?;
                }
                "home_background" => {
                    next.home_background = background_of(v)
                        .ok_or("\"home_background\" is not one of the backgrounds.")?;
                }
                "idle_lock_seconds" => {
                    next.idle_lock_seconds = idle_lock_of(v)
                        .ok_or("\"idle_lock_seconds\" must be 60, 300, 900 or 1800.")?;
                }
                _ => return Err("The request has a setting that does not exist.".into()),
            }
        }
        *self = next;
        Ok(())
    }

    pub fn load_from(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .filter(|t| t.len() <= 4096)
            .map_or_else(Self::default, |t| Self::parse(&t))
    }

    /// Write atomically: temporary file, fsync, rename, fsync the folder.
    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        let body = serde_json::to_string(&self.file_json()).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("json.tmp");
        let mut f = fs::File::create(&tmp).map_err(|e| e.to_string())?;
        f.write_all(body.as_bytes())
            .and_then(|()| f.sync_all())
            .map_err(|e| e.to_string())?;
        fs::rename(&tmp, path).map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent() {
            fs::File::open(dir)
                .and_then(|d| d.sync_all())
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

/// Start over: the preferences go too.
pub fn clear() {
    let _ = fs::remove_file(PATH);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_listed_values_are_taken() {
        let mut p = Prefs::default();
        assert_eq!(
            p.json(),
            json!({"natural_scrolling": true, "ui_scale": null, "home_background": "blue",
                   "idle_lock_seconds": 300})
        );
        p.apply(&json!({"ui_scale": 1.5})).unwrap();
        assert_eq!(p.ui_scale, Some(150));
        assert_eq!(p.json()["ui_scale"], 1.5);
        // One wrong value changes nothing.
        assert!(
            p.apply(&json!({"natural_scrolling": false, "ui_scale": 3}))
                .is_err()
        );
        assert!(p.natural_scrolling);
        assert!(
            p.apply(&json!({"home_background": "https://x/y.jpg"}))
                .is_err()
        );
        assert!(p.apply(&json!({"mode": {}})).is_err());
        // No "never" (0) and no other numbers.
        assert!(p.apply(&json!({"idle_lock_seconds": 0})).is_err());
        assert!(p.apply(&json!({"idle_lock_seconds": 301})).is_err());
        p.apply(&json!({"idle_lock_seconds": 900})).unwrap();
        assert_eq!(p.idle_lock_seconds, 900);
        assert!(p.apply(&json!([1])).is_err());
        p.apply(&json!({"ui_scale": null, "natural_scrolling": false}))
            .unwrap();
        assert_eq!((p.ui_scale, p.natural_scrolling), (None, false));
    }

    #[test]
    fn the_file_is_checked_again_when_read() {
        let p = Prefs::parse(
            r#"{"natural_scrolling": "yes", "ui_scale": 1.75, "home_background": "../x",
                "mode": {"width": 1920, "height": 1080, "refresh_mhz": 60000, "screen": "zz"}}"#,
        );
        assert!(p.natural_scrolling);
        assert_eq!(p.ui_scale, Some(175));
        assert_eq!(p.home_background, "blue");
        assert_eq!(p.mode, None);
        assert_eq!(
            Prefs::parse(r#"{"idle_lock_seconds": 0}"#).idle_lock_seconds,
            300
        );
        assert_eq!(Prefs::parse("not json"), Prefs::default());

        let dir = std::env::temp_dir().join(format!("alp-prefs-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let f = dir.join("prefs.json");
        let mut p = Prefs {
            ui_scale: Some(125),
            ..Prefs::default()
        };
        let m = Mode {
            width: 1920,
            height: 1080,
            refresh_mhz: 60000,
        };
        p.mode = Some((m, "a".repeat(64)));
        p.save_to(&f).unwrap();
        assert_eq!(Prefs::load_from(&f), p);
        let _ = fs::remove_dir_all(&dir);
    }
}
