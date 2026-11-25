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

/// The installer as `alpd` passes it on (`alp_installer::view`).
pub mod install {
    use serde_json::Value;

    fn s(v: &Value) -> String {
        v.as_str().unwrap_or_default().to_string()
    }

    fn lines(v: &Value) -> Vec<String> {
        v.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|l| l.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[derive(Debug, Clone, PartialEq, Default)]
    pub struct Disk {
        pub name: String,
        pub model: String,
        pub size: String,
        pub usb: bool,
        /// False: greyed out, with [`Disk::reason`].
        pub usable: bool,
        pub can_be_system: bool,
        pub can_be_data: bool,
        /// What is on it now (erased if chosen); empty when blank.
        pub contents: Vec<String>,
        /// Why it is greyed out: translation key and English text.
        pub reason: Option<(String, String)>,
    }

    /// `GET install/disks`: every disk, in alpd's order.
    pub fn disks(w: &Value) -> Vec<Disk> {
        w["disks"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter(|d| d.is_object())
                    .map(|d| {
                        let roles = lines(&d["roles"]);
                        let has = |r: &str| roles.iter().any(|x| x == r);
                        let usable = d["usable"].as_bool().unwrap_or(false);
                        Disk {
                            name: s(&d["name"]),
                            model: s(&d["model"]),
                            size: s(&d["size"]),
                            usb: d["transport"] == "usb",
                            usable,
                            can_be_system: usable && (has("system") || has("system_and_data")),
                            can_be_data: usable && has("data"),
                            contents: lines(&d["contents"]),
                            reason: (!usable)
                                .then(|| (s(&d["reason"]["key"]), s(&d["reason"]["text"]))),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    #[derive(Debug, Clone, PartialEq, Default)]
    pub struct Plan {
        /// (name, model, size, contents) of every disk that is erased.
        pub erases: Vec<(String, String, String, Vec<String>)>,
        /// Some erased disk holds files: strong warning and the slide.
        pub strong_warning: bool,
        pub usable: String,
        /// `none`, `bit_rot_only` or `disk_failure`.
        pub protection: String,
    }

    /// `POST install/plan`.
    pub fn plan(w: &Value) -> Plan {
        Plan {
            erases: w["erases"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|e| {
                            (
                                s(&e["name"]),
                                s(&e["model"]),
                                s(&e["size"]),
                                lines(&e["contents"]),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
            // Unknown means warn: never skip the slide by mistake.
            strong_warning: w["strong_warning"].as_bool().unwrap_or(true),
            usable: s(&w["pool"]["usable"]),
            protection: s(&w["pool"]["protection"]),
        }
    }

    #[derive(Debug, Clone, PartialEq, Default)]
    pub struct Job {
        pub running: bool,
        pub percent: u8,
        pub key: String,
        pub message: String,
        /// `ok` or an `ALP-INST-*` code once finished.
        pub result: Option<String>,
        pub detail: String,
    }

    /// `state.install`: whether this system can install, and the job.
    pub fn state(w: &Value) -> (bool, Option<Job>) {
        let i = &w["install"];
        let job = i["job"].as_object().map(|_| {
            let j = &i["job"];
            Job {
                running: j["running"].as_bool().unwrap_or(false),
                percent: j["percent"].as_u64().unwrap_or(0).min(100) as u8,
                key: s(&j["key"]),
                message: s(&j["message"]),
                result: j["result"].as_str().map(str::to_string),
                detail: s(&j["detail"]),
            }
        });
        (i["available"].as_bool().unwrap_or(false), job)
    }
}

/// First start of the installed system (plan `0.2.8-first-start.md`,
/// step 4): password, then the archive is made on the installer's disks,
/// then the recovery code, then a phone. Which step the state asks for.
pub mod setup {
    use serde_json::Value;

    #[derive(Debug, Clone, PartialEq)]
    pub enum Step {
        /// No password yet (`lock.state` `setup`).
        Password,
        /// The archive is being made on the chosen disks (or must be).
        Pool {
            /// Start `archive/create-pool` (nothing running, nothing failed).
            start: bool,
            percent: u8,
            message: String,
            /// Why it stopped; a retry resumes (alpd's error message).
            failed: Option<String>,
        },
        Recovery,
        Phone,
    }

    pub fn step(w: &Value) -> Option<Step> {
        if w["lock"]["state"].as_str() == Some("setup") {
            return Some(Step::Password);
        }
        let finished = w["setup"]["finished"].as_bool().unwrap_or(false)
            || w["setup"]["complete"].as_bool().unwrap_or(false);
        if finished || w["console"]["state"].as_str() != Some("open") {
            return None;
        }
        let a = &w["archive"];
        let job = &a["job"];
        if job["id"].as_str() == Some("archive.create") || w["pool"].as_str() == Some("waiting") {
            let state = job["state"].as_str().unwrap_or("");
            let running = matches!(state, "running" | "cancelling");
            let failed = (state == "failed").then(|| {
                job["error"]["message"]
                    .as_str()
                    .or(job["message"].as_str())
                    .unwrap_or_default()
                    .to_string()
            });
            if running || failed.is_some() || w["pool"].as_str() == Some("waiting") {
                return Some(Step::Pool {
                    start: !running && failed.is_none() && w["pool"].as_str() == Some("waiting"),
                    percent: job["percent"].as_u64().unwrap_or(0).min(100) as u8,
                    message: job["message"].as_str().unwrap_or_default().to_string(),
                    failed,
                });
            }
        }
        let skipped = |name: &str| {
            w["setup"]["skipped"]
                .as_array()
                .is_some_and(|s| s.iter().any(|v| v.as_str() == Some(name)))
        };
        if a["state"].as_str() == Some("open")
            && a["recovery"].as_str() != Some("set")
            && !skipped("recovery")
        {
            return Some(Step::Recovery);
        }
        Some(Step::Phone)
    }

    /// The three word positions `recovery/create` asks to type back.
    pub fn asked(answer: &Value) -> Option<[usize; 3]> {
        let a = answer["check"].as_array()?;
        let v: Vec<usize> = a
            .iter()
            .filter_map(|n| n.as_u64())
            .map(|n| n as usize)
            .collect();
        match v.as_slice() {
            [x, y, z] if v.iter().all(|n| (1..=15).contains(n)) => Some([*x, *y, *z]),
            _ => None,
        }
    }
}

/// The home screen's tiles (owner's 0.2.7 note 6: spread out, an uneven
/// grid). Same facts and wire fields as the web console's home tiles.
pub mod home {
    use serde_json::Value;

    #[derive(Debug, Clone, PartialEq, Default)]
    pub struct Tiles {
        /// The archive is open and holds something (else no tiles).
        pub show: bool,
        pub photos: u64,
        pub free_bytes: u64,
        /// `ok`, `low` or `full`.
        pub space: String,
        pub phones: u64,
        /// The last upload, as alpd sent it (RFC 3339).
        pub last_upload: Option<String>,
        /// `ok`, `warn` or `bad`.
        pub health: String,
        /// Where the archive is kept (disk label or name).
        pub place: Option<String>,
    }

    impl Tiles {
        pub fn from_state(w: &Value) -> Self {
            let ar = &w["archive"];
            let ph = &w["phone"];
            let state = ar["state"].as_str().unwrap_or("none");
            let space = ar["space"].as_str().unwrap_or("ok").to_string();
            let index_space = w["index"]["space"].as_str().unwrap_or("ok");
            let health = match ar["health"].as_str() {
                Some(h) => h.to_string(),
                None if state != "open"
                    || space == "full"
                    || index_space == "full"
                    || w["immich"]["state"].as_str() == Some("failed") =>
                {
                    "bad".into()
                }
                None if space == "low" || index_space == "low" => "warn".into(),
                None => "ok".into(),
            };
            let place = &ar["place"];
            Tiles {
                show: !matches!(state, "none" | "failed" | "creating" | "missing"),
                photos: ph["photos"].as_u64().unwrap_or(0),
                free_bytes: ar["free_bytes"].as_u64().unwrap_or(0),
                space,
                phones: ph["phones"].as_u64().unwrap_or(0),
                last_upload: ph["last_upload"].as_str().map(str::to_string),
                health,
                place: place["label"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .or_else(|| place["name"].as_str())
                    .map(str::to_string),
            }
        }
    }

    /// `12,345` (English digit groups; translation comes after beta).
    pub fn number(n: u64) -> String {
        let s = n.to_string();
        let mut out = String::new();
        for (i, c) in s.chars().enumerate() {
            if i > 0 && (s.len() - i).is_multiple_of(3) {
                out.push(',');
            }
            out.push(c);
        }
        out
    }

    /// `850 GB`, `12.5 TB` (decimal units, as the web console).
    pub fn size(b: u64) -> String {
        if b >= 1_000_000_000_000 {
            let tb = format!("{:.1}", b as f64 / 1e12);
            format!("{} TB", tb.strip_suffix(".0").unwrap_or(&tb))
        } else {
            format!("{} GB", (b as f64 / 1e9).round() as u64)
        }
    }

    /// About how many photos fit in `b` bytes (4.3 MB a photo, rounded to
    /// thousands), as the web console says it.
    pub fn room_photos(b: u64) -> String {
        number(((b as f64 / 4.3e6 / 1000.0).round() as u64) * 1000)
    }
}

/// The gallery view (ADR 0028: Immich in a locked embedded viewer).
pub mod gallery {
    use serde_json::Value;

    /// The only origin the gallery view may show: the gallery proxy on
    /// loopback (nginx 127.0.0.1:8412, its own origin, never the console's).
    pub const ORIGIN: &str = "http://127.0.0.1:8412";

    /// `immich.gallery_url` when alpd says the gallery is ready and it is
    /// the gallery proxy; anything else is ignored.
    pub fn url(w: &Value) -> Option<String> {
        w["immich"]["gallery_url"]
            .as_str()
            .filter(|u| allowed(u))
            .map(str::to_string)
    }

    /// May the view load `uri`? Only pages of [`ORIGIN`] (no other host,
    /// port, scheme or user part); `about:blank` for an empty frame.
    pub fn allowed(uri: &str) -> bool {
        if uri == "about:blank" {
            return true;
        }
        uri.strip_prefix(ORIGIN)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    }
}

/// Phone pairing on this screen (ADR 0017): the QR the ALPArchive app scans
/// and the approval of a phone that scanned it.
pub mod pairing {
    use serde_json::Value;

    /// A phone waiting for approval (`state.pairing.pending`).
    #[derive(Debug, Clone, PartialEq)]
    pub struct Pending {
        pub id: String,
        /// The phone's name, else its model, else empty.
        pub name: String,
        /// The code the phone shows too.
        pub code: String,
    }

    pub fn pending(w: &Value) -> Vec<Pending> {
        w["pairing"]["pending"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|p| {
                        let s = |k: &str| p[k].as_str().unwrap_or_default().trim().to_string();
                        let (id, code) = (s("id"), s("code"));
                        if id.is_empty() || code.is_empty() {
                            return None;
                        }
                        let name = Some(s("name"))
                            .filter(|n| !n.is_empty())
                            .unwrap_or_else(|| s("model"));
                        Some(Pending { id, name, code })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The QR modules of `text` (row by row, `true` = dark) and the side
    /// length, without the quiet zone. Same encoder and error correction
    /// as alpd's SVG (`alp_live::qr`).
    pub fn qr(text: &str) -> Option<(usize, Vec<bool>)> {
        use qrcodegen::{QrCode, QrCodeEcc};
        let q = QrCode::encode_text(text, QrCodeEcc::Medium).ok()?;
        let n = q.size();
        let cells = (0..n)
            .flat_map(|y| (0..n).map(move |x| (x, y)))
            .map(|(x, y)| q.get_module(x, y))
            .collect();
        Some((usize::try_from(n).ok()?, cells))
    }

    /// "m:ss" for the time left.
    pub fn left(seconds: u64) -> (String, String) {
        ((seconds / 60).to_string(), format!("{:02}", seconds % 60))
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

    #[test]
    fn install_disks_and_plan() {
        let w = json!({"disks": [
            {"name": "sda", "model": "Disk", "size": "500 GB", "transport": "sata",
             "usable": true, "roles": ["system_and_data", "system", "data"], "contents": []},
            {"name": "sdb", "size": "1 TB", "usable": true, "roles": ["data"],
             "contents": ["ntfs \"Photos\" (1000 GB)"]},
            {"name": "nvme0n1", "usable": false,
             "reason": {"key": "holds_windows", "text": "Holds Windows."}},
            "junk"
        ]});
        let d = install::disks(&w);
        assert_eq!(d.len(), 3);
        assert!(d[0].can_be_system && d[0].can_be_data && d[0].contents.is_empty());
        assert!(!d[1].can_be_system && d[1].can_be_data);
        assert!(!d[2].usable && !d[2].can_be_data);
        assert_eq!(d[2].reason.as_ref().unwrap().0, "holds_windows");

        let p = install::plan(&json!({
            "erases": [{"name": "sdb", "model": "M", "size": "1 TB", "contents": ["x"]}],
            "strong_warning": true, "pool": {"usable": "1 TB", "protection": "bit_rot_only"}
        }));
        assert!(p.strong_warning);
        assert_eq!(p.erases[0].3, ["x"]);
        // A plan without the flag never skips the warning.
        assert!(install::plan(&json!({})).strong_warning);

        let (avail, job) = install::state(&json!({"install": {"available": true,
            "job": {"running": true, "percent": 40, "key": "k", "message": "m", "result": null}}}));
        assert!(avail);
        assert_eq!(job.unwrap().percent, 40);
        assert_eq!(install::state(&json!({})), (false, None));
    }

    #[test]
    fn home_tiles_read_the_wire() {
        use super::home::{Tiles, number, room_photos, size};
        let t = Tiles::from_state(&json!({
            "archive": {"state": "open", "space": "low", "free_bytes": 2_500_000_000_000u64,
                        "place": {"label": "", "name": "sdb"}},
            "phone": {"photos": 1234567, "phones": 2, "last_upload": "2026-11-24T10:00:00Z"}
        }));
        assert!(t.show);
        assert_eq!(
            (t.photos, t.phones, t.health.as_str()),
            (1234567, 2, "warn")
        );
        assert_eq!(t.place.as_deref(), Some("sdb"));
        assert_eq!(number(1234567), "1,234,567");
        assert_eq!(number(999), "999");
        assert_eq!(size(2_500_000_000_000), "2.5 TB");
        assert_eq!(size(3_000_000_000_000), "3 TB");
        assert_eq!(size(850_400_000_000), "850 GB");
        assert_eq!(room_photos(43_000_000_000), "10,000");
        let none = Tiles::from_state(&json!({}));
        assert!(!none.show);
        assert_eq!(none.health, "bad");
        let ok = Tiles::from_state(&json!({"archive": {"state": "open", "health": "ok"}}));
        assert_eq!(ok.health, "ok");
    }

    #[test]
    fn the_gallery_view_stays_on_the_gallery_origin() {
        use super::gallery::{allowed, url};
        assert!(allowed("http://127.0.0.1:8412/"));
        assert!(allowed("http://127.0.0.1:8412/photos/abc?x=1"));
        assert!(allowed("http://127.0.0.1:8412"));
        assert!(allowed("about:blank"));
        for bad in [
            "http://127.0.0.1:8411/api/v1/live/state",
            "http://127.0.0.1:84120/",
            "http://127.0.0.1:8412.evil.example/",
            "http://127.0.0.1:8412@evil.example/",
            "https://127.0.0.1:8412/",
            "http://localhost:8412/",
            "https://immich.app/",
            "file:///etc/passwd",
            "javascript:alert(1)",
        ] {
            assert!(!allowed(bad), "{bad}");
        }
        assert_eq!(
            url(&json!({"immich": {"gallery_url": "http://127.0.0.1:8412/"}})).as_deref(),
            Some("http://127.0.0.1:8412/")
        );
        assert_eq!(
            url(&json!({"immich": {"gallery_url": "http://evil/"}})),
            None
        );
        assert_eq!(url(&json!({})), None);
    }

    #[test]
    fn pairing_reads_pending_phones_and_draws_the_qr() {
        use super::pairing::{left, pending, qr};
        let p = pending(&json!({"pairing": {"pending": [
            {"id": "r1", "name": "", "model": "Pixel 8", "code": "123 456"},
            {"id": "r2", "name": "Dana's phone", "code": "999 000"},
            {"id": "", "code": "1"},
            {"id": "r3"}
        ]}}));
        assert_eq!(p.len(), 2);
        assert_eq!(
            (p[0].name.as_str(), p[0].code.as_str()),
            ("Pixel 8", "123 456")
        );
        assert_eq!(p[1].name, "Dana's phone");
        assert!(pending(&json!({})).is_empty());
        // Version 2 for a short address, as alpd's SVG (25 modules).
        let (n, cells) = qr("https://192.168.1.23").unwrap();
        assert_eq!((n, cells.len()), (25, 625));
        // The three finder patterns: top-left corner dark.
        assert!(cells[0] && cells[n - 1] && cells[n * (n - 1)]);
        assert_eq!(left(125), ("2".to_string(), "05".to_string()));
    }
    #[test]
    fn setup_steps_in_order() {
        use super::setup::{Step, asked, step};
        assert_eq!(
            step(&json!({"lock": {"state": "setup"}})),
            Some(Step::Password)
        );
        let open = |extra: Value| {
            let mut w = json!({"lock": {"state": "open"}, "console": {"state": "open"},
                "setup": {"finished": false, "skipped": []}, "pool": "made",
                "archive": {"state": "open", "recovery": "none", "job": null}});
            if let (Some(o), Some(e)) = (w.as_object_mut(), extra.as_object()) {
                for (k, v) in e {
                    o.insert(k.clone(), v.clone());
                }
            }
            w
        };
        // Signed out, or finished: no setup page.
        assert_eq!(step(&open(json!({"console": {"state": "locked"}}))), None);
        assert_eq!(step(&open(json!({"setup": {"finished": true}}))), None);
        // The pool still to make: start it once.
        let w = open(json!({"pool": "waiting", "archive": {"state": "none", "job": null}}));
        assert!(matches!(step(&w), Some(Step::Pool { start: true, .. })));
        let w = open(json!({"pool": "waiting", "archive": {"state": "creating",
            "job": {"id": "archive.create", "state": "running", "percent": 50, "message": "Locking"}}}));
        assert_eq!(
            step(&w),
            Some(Step::Pool {
                start: false,
                percent: 50,
                message: "Locking".into(),
                failed: None
            })
        );
        let w = open(json!({"pool": "waiting", "archive": {"state": "failed",
            "job": {"id": "archive.create", "state": "failed", "percent": 50, "message": "m",
                    "error": {"message": "A disk changed."}}}}));
        assert!(
            matches!(step(&w), Some(Step::Pool { start: false, failed: Some(ref f), .. }) if f == "A disk changed.")
        );
        // Made: the recovery code, unless skipped; then the phone.
        assert_eq!(step(&open(json!({}))), Some(Step::Recovery));
        assert_eq!(
            step(&open(
                json!({"setup": {"finished": false, "skipped": ["recovery"]}})
            )),
            Some(Step::Phone)
        );
        assert_eq!(
            step(&open(
                json!({"archive": {"state": "open", "recovery": "set"}})
            )),
            Some(Step::Phone)
        );
        assert_eq!(asked(&json!({"check": [2, 9, 14]})), Some([2, 9, 14]));
        assert_eq!(asked(&json!({"check": [0, 9, 14]})), None);
        assert_eq!(asked(&json!({"check": [2, 9]})), None);
    }
}
