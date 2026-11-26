// SPDX-License-Identifier: AGPL-3.0-or-later
//! What the settings pages and the flows behind them read from alpd's state
//! (the 0.2.7 console's `normalize()` and `settings()`, without any GTK):
//! phones, password and recovery, the archive disk, display preferences,
//! the family chain, the disk list, the files on other disks, reminders for
//! skipped setup steps, and the list of messages behind the (i). Kept free
//! of drawing so it can be tested; the pages only show what it gives.

use serde_json::Value;

/// A server capability announced in `features` (`has("clock_sync")`).
pub fn has(w: &Value, feature: &str) -> bool {
    w["features"]
        .as_array()
        .is_some_and(|a| a.iter().any(|v| v.as_str() == Some(feature)))
}

fn s(v: &Value) -> Option<String> {
    v.as_str().map(str::to_string)
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().filter_map(s).collect())
        .unwrap_or_default()
}

/// A signed-in phone (the photo service's device list).
#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub os: String,
    pub kind: String,
    /// RFC 3339, when the phone last backed up.
    pub last_seen: Option<String>,
}

/// A phone paired with the ALPArchive app.
#[derive(Debug, Clone, PartialEq)]
pub struct Paired {
    pub serial: String,
    pub name: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Phones {
    pub devices: Vec<Device>,
    pub paired: Vec<Paired>,
    /// "Only paired phones".
    pub strict: bool,
}

impl Phones {
    pub fn from_state(w: &Value) -> Self {
        let devices = w["phone"]["devices"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|d| {
                        Some(Device {
                            id: s(&d["id"])?,
                            name: s(&d["name"]).unwrap_or_default(),
                            os: s(&d["os"]).unwrap_or_default(),
                            kind: s(&d["kind"]).unwrap_or_else(|| "phone".into()),
                            last_seen: s(&d["last_seen"]),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let paired = w["pairing"]["devices"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter(|d| !d["revoked"].as_bool().unwrap_or(false))
                    .filter_map(|d| {
                        Some(Paired {
                            serial: s(&d["serial"])?,
                            name: s(&d["name"]).unwrap_or_default(),
                            model: s(&d["model"]).unwrap_or_default(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Phones {
            devices,
            paired,
            strict: w["pairing"]["strict"].as_bool().unwrap_or(false),
        }
    }
}

/// "Dana's phone · iOS": the name, then the system; `unknown` when alpd
/// knows neither.
pub fn device_line(name: &str, os: &str, unknown: &str) -> String {
    let first = if name.is_empty() { unknown } else { name };
    if os.is_empty() {
        first.to_string()
    } else {
        format!("{first} · {os}")
    }
}

/// The password, recovery code and protection level.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Account {
    /// `none`, `console`, `password` or `strong`; unknown before the
    /// password is chosen.
    pub protection: Option<String>,
    pub archive_open: bool,
    pub recovery_set: bool,
    /// RFC 3339.
    pub recovery_made: Option<String>,
    /// Strong protection with an extra passphrase.
    pub serious: bool,
    pub password_reopen: bool,
}

impl Account {
    pub fn from_state(w: &Value) -> Self {
        Account {
            protection: s(&w["lock"]["protection"]),
            archive_open: w["archive"]["state"].as_str() == Some("open"),
            recovery_set: w["archive"]["recovery"].as_str() == Some("set"),
            recovery_made: s(&w["archive"]["recovery_made"]),
            serious: w["archive"]["serious"].as_bool().unwrap_or(false),
            password_reopen: w["archive"]["password_reopen"].as_bool().unwrap_or(true),
        }
    }

    /// The archive is encrypted with the ALP password (levels 3 and 4).
    pub fn encrypted(&self) -> bool {
        matches!(self.protection.as_deref(), Some("password" | "strong"))
    }

    /// Strong protection: the check also asks for the extra passphrase.
    pub fn strong(&self) -> bool {
        self.protection.as_deref() == Some("strong") || self.serious
    }

    /// Encrypt later is offered for the two unencrypted levels.
    pub fn can_encrypt_later(&self) -> bool {
        matches!(self.protection.as_deref(), Some("none" | "console"))
    }

    /// The text key for the protection line ("about.protection.*").
    pub fn protection_key(&self) -> String {
        format!(
            "about.protection.{}",
            self.protection.as_deref().unwrap_or("none")
        )
    }
}

/// The archive disk in Settings > Disk.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ArchiveDisk {
    /// `none`, `creating`, `open`, `missing`, `failed`.
    pub state: String,
    pub place: Option<String>,
    pub size: u64,
    pub free: u64,
    /// `ok`, `warn` or `bad`.
    pub health: String,
}

impl ArchiveDisk {
    pub fn from_state(w: &Value) -> Self {
        let t = crate::model::home::Tiles::from_state(w);
        let a = &w["archive"];
        ArchiveDisk {
            state: s(&a["state"]).unwrap_or_else(|| "none".into()),
            place: t.place,
            size: a["size_bytes"].as_u64().unwrap_or(0),
            free: a["free_bytes"].as_u64().unwrap_or(0),
            health: t.health,
        }
    }

    pub fn used(&self) -> u64 {
        self.size.saturating_sub(self.free)
    }

    /// Percent used, 0 to 100.
    pub fn percent(&self) -> u32 {
        if self.size == 0 {
            0
        } else {
            ((self.used() as f64 / self.size as f64) * 100.0).round() as u32
        }
    }

    /// Without an archive the row offers "Set up".
    pub fn can_resume(&self) -> bool {
        matches!(self.state.as_str(), "none" | "failed")
    }
}

/// The text size steps: smaller, default (automatic), larger, larger
/// still, largest.
pub const SCALE_STEPS: [Option<f64>; 5] = [Some(1.0), None, Some(1.5), Some(1.75), Some(2.0)];
pub const SCALE_NAMES: [&str; 5] = [
    "disp.s.smaller",
    "disp.s.default",
    "disp.s.larger",
    "disp.s.larger2",
    "disp.s.largest",
];
/// The lock screen's items the person can hide.
pub const LOCK_ITEMS: [&str; 4] = ["phones", "network", "space", "backup"];
/// The idle lock choices, in seconds ("never" is not offered).
pub const IDLE_CHOICES: [u64; 4] = [60, 300, 900, 1800];

/// The step the slider shows for the saved scale (`null` = automatic).
pub fn scale_index(scale: Option<f64>) -> usize {
    SCALE_STEPS
        .iter()
        .position(|s| match (s, scale) {
            (None, None) => true,
            (Some(a), Some(b)) => (a - b).abs() < 0.001,
            _ => false,
        })
        .unwrap_or(1)
}

#[derive(Debug, Clone, PartialEq)]
pub struct DisplayPrefs {
    pub scale: Option<f64>,
    /// Seconds until the screen locks by itself.
    pub idle: u64,
    pub lock_hide: Vec<String>,
    /// `None`: this alpd does not offer the switch.
    pub natural_scrolling: Option<bool>,
    pub idle_pref: bool,
    pub lock_hide_pref: bool,
    pub modes: bool,
    pub natural_pref: bool,
    pub auto_scale: Option<f64>,
}

impl DisplayPrefs {
    pub fn from_state(w: &Value) -> Self {
        let p = &w["prefs"];
        DisplayPrefs {
            scale: p["ui_scale"].as_f64(),
            idle: w["console"]["idle_lock_seconds"].as_u64().unwrap_or(300),
            lock_hide: strings(&p["lock_hide"]),
            natural_scrolling: p["natural_scrolling"].as_bool(),
            idle_pref: has(w, "idle_lock_pref"),
            lock_hide_pref: has(w, "lock_hide"),
            modes: has(w, "display_modes"),
            natural_pref: has(w, "natural_scrolling"),
            auto_scale: w["display"]["auto_scale"].as_f64(),
        }
    }
}

/// The lock-screen items still shown after switching `item` to `shown`.
pub fn lock_hide_after(current: &[String], item: &str, shown: bool) -> Vec<String> {
    LOCK_ITEMS
        .iter()
        .filter(|x| {
            if **x == item {
                !shown
            } else {
                current.iter().any(|c| c == **x)
            }
        })
        .map(|x| x.to_string())
        .collect()
}

/// One screen resolution (`display/modes`).
#[derive(Debug, Clone, PartialEq)]
pub struct Mode {
    pub id: u64,
    pub width: u64,
    pub height: u64,
    pub refresh: Option<f64>,
    pub current: bool,
    pub native: bool,
}

impl Mode {
    pub fn text(&self) -> String {
        match self.refresh.filter(|r| *r > 0.0) {
            Some(r) => format!("{} × {} · {} Hz", self.width, self.height, r.round() as u64),
            None => format!("{} × {}", self.width, self.height),
        }
    }
}

pub fn modes(v: &Value) -> Vec<Mode> {
    v["modes"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|m| {
                    Some(Mode {
                        id: m["id"].as_u64()?,
                        width: m["width"].as_u64()?,
                        height: m["height"].as_u64()?,
                        refresh: m["refresh"].as_f64(),
                        current: m["current"].as_bool().unwrap_or(false),
                        native: m["native"].as_bool().unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Seconds to wait before a new resolution goes back by itself (5 to 30).
pub fn revert_in(answer: &Value) -> u64 {
    answer["revert_in"].as_u64().unwrap_or(15).clamp(5, 30)
}

/// The family chain: `member`, or anything else (none).
pub fn chain_member(w: &Value) -> bool {
    w["chain"]["state"].as_str() == Some("member")
}

/// A skipped or open setup step that shows on home ("Do it now").
/// `(step, text key)`, in the order of setup.
pub fn reminders(w: &Value) -> Vec<(&'static str, &'static str)> {
    let mut r = Vec::new();
    let a = &w["archive"]["state"];
    let a = a.as_str().unwrap_or("none");
    if !w["network"]["online"].as_bool().unwrap_or(false) {
        r.push(("network", "rem.network"));
    }
    if w["lock"]["state"].as_str() == Some("setup") {
        r.push(("password", "rem.password"));
    }
    if matches!(a, "none" | "failed") {
        r.push(("where", "rem.where"));
    }
    let open = a == "open";
    let ph = &w["phone"];
    if open && !ph["paired"].as_bool().unwrap_or(false) && ph["server_url"].is_null() {
        r.push(("phone", "rem.phone"));
    }
    let acc = Account::from_state(w);
    if open && acc.protection.as_deref() == Some("strong") && !acc.recovery_set {
        r.push(("recovery", "rem.recovery"));
    }
    if open && acc.protection.as_deref() == Some("password") && !acc.password_reopen {
        r.push(("pwreopen", "rem.pwreopen"));
    }
    r
}

// ---------------------------------------------------------------- disks

/// One partition of a disk.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub partuuid: String,
    pub label: String,
    pub name: String,
    pub fs: String,
    pub size_bytes: u64,
}

/// One disk in Advanced > Disks.
#[derive(Debug, Clone, PartialEq)]
pub struct DiskItem {
    pub id: String,
    pub name: String,
    pub model: String,
    pub size_bytes: u64,
    /// `stick`, `internal` or `external`.
    pub kind: String,
    /// `system`, `backup` or `other`.
    pub role: String,
    pub parts: Vec<Part>,
    pub health: Option<String>,
    /// The legend level of its health (`action`, `now`, `critical`).
    pub level: Option<String>,
    pub temperature_c: Option<f64>,
    pub safe_to_remove: bool,
    /// Why it cannot be changed (`stick`, `internal_live`, `backup`,
    /// `write_protected`, `open`).
    pub lock: Option<String>,
}

impl DiskItem {
    /// The computer's own disk: listed, never chosen.
    pub fn offline(&self) -> bool {
        self.kind == "internal"
    }

    /// The name to show: the model, else the kernel name.
    pub fn title(&self) -> String {
        if !self.model.is_empty() && self.model != self.name {
            self.model.clone()
        } else if !self.name.is_empty() {
            self.name.clone()
        } else {
            self.id.clone()
        }
    }

    /// Only an unlocked external disk with an id can be formatted.
    pub fn can_format(&self) -> bool {
        self.lock.is_none() && self.kind == "external" && !self.id.is_empty()
    }

    /// The partition ids the format call must name (exactly what was shown).
    pub fn part_ids(&self) -> Vec<String> {
        self.parts.iter().map(|p| p.partuuid.clone()).collect()
    }

    /// The text key that says why a locked disk is locked.
    pub fn lock_hint_key(&self) -> &'static str {
        match self.lock.as_deref() {
            Some("stick" | "backup") => "disks.lock.startover",
            Some("write_protected") => "disks.lock.wp",
            Some("open") => "disks.lock.open",
            _ if !self.id.is_empty() => "disks.lock.other",
            _ => "disks.refused.empty",
        }
    }

    /// Eject: an external disk that is not the archive's.
    pub fn can_eject(&self) -> bool {
        self.kind == "external" && !matches!(self.lock.as_deref(), Some("backup" | "open"))
    }

    /// The label to start with: the first partition's, if it is valid.
    pub fn start_label(&self) -> String {
        self.parts
            .first()
            .map(|p| p.label.clone())
            .filter(|l| label_ok(l))
            .unwrap_or_default()
    }
}

/// A disk name for formatting: 1 to 11 of letters, digits, space, `_`, `-`.
pub fn label_ok(l: &str) -> bool {
    (1..=11).contains(&l.chars().count())
        && l.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '-'))
}

/// `live/disks` as the list: open disks first, the computer's own last.
pub fn disks(v: &Value) -> Vec<DiskItem> {
    let mut list: Vec<DiskItem> = v["disks"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|d| {
                    let kind = s(&d["kind"]).unwrap_or_else(|| "external".into());
                    let lock = s(&d["lock"])
                        .or_else(|| (kind == "internal").then(|| "internal_live".into()));
                    DiskItem {
                        id: s(&d["id"]).unwrap_or_default(),
                        name: s(&d["name"]).unwrap_or_default(),
                        model: s(&d["model"]).unwrap_or_default(),
                        size_bytes: d["size_bytes"].as_u64().unwrap_or(0),
                        kind,
                        role: s(&d["role"]).unwrap_or_else(|| "other".into()),
                        parts: d["parts"]
                            .as_array()
                            .map(|p| {
                                p.iter()
                                    .map(|p| Part {
                                        partuuid: s(&p["partuuid"]).unwrap_or_default(),
                                        label: s(&p["label"]).unwrap_or_default(),
                                        name: s(&p["name"]).unwrap_or_default(),
                                        fs: s(&p["fs"]).unwrap_or_default(),
                                        size_bytes: p["size_bytes"].as_u64().unwrap_or(0),
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                        health: s(&d["health"]),
                        level: s(&d["level"]),
                        temperature_c: d["temperature_c"].as_f64(),
                        safe_to_remove: d["safe_to_remove"].as_bool().unwrap_or(false),
                        lock,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    list.sort_by_key(DiskItem::offline);
    list
}

/// What `disks/format/estimate` says.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Estimate {
    pub size_bytes: u64,
    pub bytes_used: u64,
    pub files: Option<u64>,
    pub confirm: Value,
    pub warnings: Vec<String>,
    pub quick_text: Option<String>,
    pub secure_text: Option<String>,
    pub secure_seconds: u64,
    pub secure_about: bool,
    pub photos_not_in_archive: u64,
}

pub fn estimate(v: &Value) -> Estimate {
    Estimate {
        size_bytes: v["size_bytes"].as_u64().unwrap_or(0),
        bytes_used: v["bytes_used"].as_u64().unwrap_or(0),
        files: v["files"].as_u64(),
        confirm: v["confirm"].clone(),
        warnings: strings(&v["warnings"]),
        quick_text: s(&v["quick"]["text"]),
        secure_text: s(&v["secure"]["text"]),
        secure_seconds: v["secure"]["seconds"].as_u64().unwrap_or(60),
        secure_about: v["secure"]["about"].as_bool().unwrap_or(false),
        photos_not_in_archive: v["photos_not_in_archive"].as_u64().unwrap_or(0),
    }
}

impl Estimate {
    /// Minutes a secure erase takes, at least 1.
    pub fn secure_minutes(&self) -> u64 {
        (self.secure_seconds as f64 / 60.0).round().max(1.0) as u64
    }
}

/// The `disks/format` body: the checked partitions, the mode, the password
/// and the server's `confirm` token (the label only when given).
pub fn format_body(
    disk: &DiskItem,
    mode: &str,
    password: &str,
    label: &str,
    fs: &str,
    est: &Estimate,
) -> Value {
    let mut b = serde_json::json!({
        "disk_id": disk.id,
        "partuuids": disk.part_ids(),
        "mode": mode,
        "password": password,
        "fs": fs,
        "confirm": est.confirm,
    });
    if !label.is_empty() {
        b["label"] = Value::String(label.to_string());
    }
    b
}

/// The format or reset job in the state: `(percent, message, seconds_left,
/// can_cancel_while_erasing, state)`; `None` when another job is shown.
#[derive(Debug, Clone, PartialEq)]
pub struct JobView {
    pub percent: u64,
    pub message: String,
    pub seconds_left: Option<u64>,
    pub can_stop: bool,
    /// `running`, `done`, `failed`, `cancelled`.
    pub state: String,
    pub error: Option<String>,
}

pub fn job(w: &Value, id: &str) -> Option<JobView> {
    let j = &w["archive"]["job"];
    if j["id"].as_str() != Some(id) {
        return None;
    }
    Some(JobView {
        percent: j["percent"].as_u64().unwrap_or(0).min(100),
        message: s(&j["message"]).unwrap_or_default(),
        seconds_left: j["seconds_left"].as_u64(),
        can_stop: j["can_cancel"].as_bool().unwrap_or(false)
            && j["step"].as_str() == Some("erasing"),
        state: s(&j["state"]).unwrap_or_default(),
        error: s(&j["error"]["message"]),
    })
}

// ---------------------------------------------------------------- files

/// A file or folder listed on another disk.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Listing {
    pub entries: Vec<Entry>,
    pub truncated: bool,
    pub skipped: u64,
}

pub fn listing(v: &Value) -> Listing {
    Listing {
        entries: v["entries"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|e| {
                        Some(Entry {
                            name: s(&e["name"])?,
                            dir: e["dir"].as_bool().unwrap_or(false),
                            size_bytes: e["size_bytes"].as_u64(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
        truncated: v["truncated"].as_bool().unwrap_or(false),
        skipped: v["skipped"].as_u64().unwrap_or(0),
    }
}

/// A disk or stick in the Files view (`state.media`).
#[derive(Debug, Clone, PartialEq)]
pub struct Media {
    pub id: String,
    pub label: Option<String>,
    pub fs_type: Option<String>,
    pub size_bytes: Option<u64>,
    /// Why alpd will not open it (shown as text).
    pub refused: Option<String>,
}

pub fn media(w: &Value) -> Vec<Media> {
    w["media"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|m| {
                    Some(Media {
                        id: s(&m["id"])?,
                        label: s(&m["label"]).filter(|l| !l.is_empty()),
                        fs_type: s(&m["fs_type"]),
                        size_bytes: m["size_bytes"].as_u64(),
                        refused: s(&m["refused"]).filter(|r| !r.is_empty()),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// What a file name can be previewed as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Image,
    Video,
    Text,
    Other,
}

pub fn kind_of(name: &str) -> Kind {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" => Kind::Image,
        "mp4" | "m4v" | "mov" | "webm" => Kind::Video,
        "txt" | "log" | "md" | "csv" | "json" | "xml" | "ini" | "cfg" | "conf" | "nfo" => {
            Kind::Text
        }
        _ => Kind::Other,
    }
}

/// `850 B`, `12 KB`, `3.4 MB`, `1 GB` for a file size.
pub fn file_size(b: u64) -> String {
    if b < 1_000 {
        format!("{b} B")
    } else if b < 1_000_000 {
        format!("{} KB", (b as f64 / 1e3).round() as u64)
    } else if b < 1_000_000_000 {
        format!("{:.1} MB", b as f64 / 1e6)
    } else {
        crate::model::home::size(b)
    }
}

/// The Terminal frame is allowed only on its own local port.
pub fn terminal_url_ok(url: &str) -> bool {
    url.strip_prefix("http://127.0.0.1:8413/")
        .is_some_and(|rest| !rest.contains(['@', '\\', ' ']))
}

// ------------------------------------------------------- the (i) messages

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Lv {
    Info,
    Warn,
    Bad,
}

/// One message in the list behind the (i).
#[derive(Debug, Clone, PartialEq)]
pub struct Msg {
    pub id: String,
    /// Seconds since 1970.
    pub time: u64,
    pub level: Lv,
    pub text: String,
    pub code: Option<String>,
    pub key: Option<String>,
    pub step: Option<String>,
    pub detail: Option<String>,
    pub resolved: bool,
}

impl Msg {
    pub fn new(id: &str, level: Lv, text: &str) -> Self {
        Msg {
            id: id.to_string(),
            time: 0,
            level,
            text: text.to_string(),
            code: None,
            key: None,
            step: None,
            detail: None,
            resolved: false,
        }
    }
}

/// The messages: what the state says (an entry that goes away is marked
/// resolved), what this screen saw (a failed action) and alpd's own events.
#[derive(Debug, Default)]
pub struct ErrLog {
    pub list: Vec<Msg>,
    seen: u64,
    seq: u64,
}

impl ErrLog {
    /// Adds a message (or renews the one with the same id); returns whether
    /// anything changed.
    pub fn add(&mut self, mut m: Msg, now: u64) -> bool {
        if m.id.is_empty() {
            self.seq += 1;
            m.id = format!("c{}", self.seq);
        }
        m.time = if m.time == 0 { now } else { m.time };
        if let Some(old) = self.list.iter_mut().find(|e| e.id == m.id) {
            if old.resolved || old.text != m.text {
                *old = m;
                return true;
            }
            return false;
        }
        self.list.insert(0, m);
        self.list.truncate(200);
        true
    }

    pub fn resolve(&mut self, id: &str) -> bool {
        match self.list.iter_mut().find(|e| e.id == id && !e.resolved) {
            Some(e) => {
                e.resolved = true;
                true
            }
            None => false,
        }
    }

    /// Reads the state's problems; `t` gives the text of a key. Returns
    /// whether the list changed.
    pub fn sync(&mut self, w: &Value, now: u64, t: &dyn Fn(&str) -> String) -> bool {
        let mut changed = false;
        let mut present: Vec<String> = Vec::new();
        let mut put = |log: &mut ErrLog, m: Msg| -> bool {
            present.push(m.id.clone());
            log.add(m, now)
        };
        for (level, key) in warnings(w) {
            changed |= put(self, Msg::new(&format!("st:{key}"), level, &t(key)));
        }
        let err = |e: &Value| s(&e["message"]).or_else(|| s(&e["key"]));
        let a = &w["archive"]["error"];
        if let Some(detail) = err(a) {
            let mut m = Msg::new("st:arch.error", Lv::Bad, &t("about.err.archive"));
            m.code = s(&a["code"]);
            m.key = s(&a["key"]);
            m.detail = Some(detail);
            m.step = s(&w["archive"]["job"]["step"]);
            changed |= put(self, m);
        }
        let g = &w["immich"]["error"];
        if let Some(detail) = err(g) {
            let mut m = Msg::new("st:immich.error", Lv::Bad, &t("about.err.gallery"));
            m.code = s(&g["code"]);
            m.key = s(&g["key"]);
            m.detail = Some(detail);
            changed |= put(self, m);
        }
        for c in w["hardware"]["checks"].as_array().into_iter().flatten() {
            if c["status"].as_str() == Some("fail") {
                let item = s(&c["item"]).or_else(|| s(&c["id"])).unwrap_or_default();
                let id = format!("st:hw:{item}");
                let mut m = Msg::new(
                    &id,
                    Lv::Bad,
                    &format!("{item}: {}", s(&c["value"]).unwrap_or_default()),
                );
                m.code = s(&c["code"]);
                m.detail = s(&c["advice"]);
                changed |= put(self, m);
            }
        }
        for e in w["events"].as_array().into_iter().flatten() {
            let Some(id) = s(&e["id"]) else { continue };
            let level = match e["level"].as_str() {
                Some("error" | "bad") => Lv::Bad,
                Some("info") => Lv::Info,
                _ => Lv::Warn,
            };
            let id = format!("sv:{id}");
            let mut m = Msg::new(
                &id,
                level,
                &s(&e["message"])
                    .or_else(|| s(&e["key"]))
                    .unwrap_or_default(),
            );
            m.code = s(&e["code"]);
            m.key = s(&e["key"]);
            m.step = s(&e["step"]);
            m.detail = s(&e["detail"]);
            changed |= put(self, m);
            if e["resolved"].as_bool().unwrap_or(false) {
                changed |= self.resolve(&id);
            }
        }
        let gone: Vec<String> = self
            .list
            .iter()
            .filter(|e| {
                (e.id.starts_with("st:") || e.id.starts_with("sv:")) && !present.contains(&e.id)
            })
            .map(|e| e.id.clone())
            .collect();
        for id in gone {
            changed |= self.resolve(&id);
        }
        changed
    }

    /// The dot's colour: worst open message not yet seen.
    pub fn tone(&self) -> Option<Lv> {
        self.list
            .iter()
            .filter(|e| !e.resolved && e.time > self.seen && e.level != Lv::Info)
            .map(|e| e.level)
            .max()
    }

    pub fn mark_seen(&mut self, now: u64) {
        self.seen = now;
    }
}

/// The problems the state itself reports, as the web console listed them.
pub fn warnings(w: &Value) -> Vec<(Lv, &'static str)> {
    let a = &w["archive"];
    let mut out = Vec::new();
    if a["state"].as_str() == Some("missing") {
        out.push((Lv::Bad, "status.bad.archive"));
        return out;
    }
    if !w["network"]["online"].as_bool().unwrap_or(false) {
        out.push((Lv::Warn, "status.warn.network"));
    }
    let space = [
        a["space"].as_str().unwrap_or("ok"),
        w["index"]["space"].as_str().unwrap_or("ok"),
    ];
    if space.contains(&"full") || w["immich"]["state"].as_str() == Some("paused") {
        out.push((Lv::Bad, "status.bad.full"));
    } else if space.contains(&"low") {
        out.push((Lv::Warn, "status.warn.lowspace"));
    }
    if w["immich"]["state"].as_str() == Some("failed") {
        out.push((Lv::Bad, "status.bad.gallery"));
    }
    out
}

/// What needs the person, for the top of the messages: `(text key, button
/// key, action)` where the action is `disk`, `recovery` or a reminder step.
pub fn needs_you(w: &Value) -> Vec<(&'static str, &'static str, String)> {
    let mut out = Vec::new();
    let acc = Account::from_state(w);
    if w["archive"]["state"].as_str() == Some("missing") {
        out.push(("needs.disk", "needs.disk.do", "disk".to_string()));
    }
    if acc.archive_open && acc.encrypted() && !acc.recovery_set {
        out.push((
            "needs.recovery",
            "needs.recovery.do",
            "recovery".to_string(),
        ));
    }
    for (step, key) in reminders(w) {
        if step != "recovery" {
            out.push((key, "home.rem.do", step.to_string()));
        }
    }
    out
}

// ------------------------------------------------------- the screen saver

/// What this screen remembers by itself (alpd keeps no such setting):
/// the photo frame on or off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LocalPrefs {
    pub saver: bool,
}

impl LocalPrefs {
    pub fn parse(text: &str) -> Self {
        let v: Value = serde_json::from_str(text).unwrap_or(Value::Null);
        LocalPrefs {
            saver: v["saver"].as_bool().unwrap_or(false),
        }
    }

    pub fn to_text(self) -> String {
        serde_json::json!({ "saver": self.saver }).to_string()
    }

    pub fn load(path: &std::path::Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .filter(|t| t.len() <= 4096)
            .map(|t| Self::parse(&t))
            .unwrap_or_default()
    }

    /// Best effort: a screen that cannot write keeps the choice until it
    /// restarts.
    pub fn save(self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(path, self.to_text())
    }
}

/// Where the photo frame choice is kept.
pub fn prefs_path() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".local/state")))
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
    base.join("alp-screen").join("prefs.json")
}

/// `D•••'s iPhone` style shortening is the lock screen's; here only the
/// seconds a person has been idle decide the saver.
pub fn idle_over(idle_seconds: u64, since_last_touch: u64, in_gallery: bool) -> bool {
    let limit = if in_gallery {
        idle_seconds.max(30 * 60)
    } else {
        idle_seconds
    };
    since_last_touch >= limit
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn features_are_read_by_name() {
        let w = json!({"features": ["disks", "clock"]});
        assert!(has(&w, "disks"));
        assert!(!has(&w, "terminal"));
        assert!(!has(&json!({}), "disks"));
    }

    #[test]
    fn phones_skip_revoked_and_name_unknowns() {
        let w = json!({
            "phone": {"devices": [
                {"id": "a", "name": "Dana's phone", "os": "iOS", "kind": "phone", "last_seen": "2026-11-20T10:00:00Z"},
                {"name": "no id"}]},
            "pairing": {"strict": true, "devices": [
                {"serial": "1", "name": "Old", "revoked": true},
                {"serial": "2", "name": "Pixel", "model": "Pixel 8"}]}
        });
        let p = Phones::from_state(&w);
        assert_eq!(p.devices.len(), 1);
        assert_eq!(
            p.devices[0].last_seen.as_deref(),
            Some("2026-11-20T10:00:00Z")
        );
        assert_eq!(p.paired.len(), 1);
        assert_eq!(p.paired[0].serial, "2");
        assert!(p.strict);
        assert_eq!(device_line("", "", "A device"), "A device");
        assert_eq!(device_line("Dana", "iOS", "A device"), "Dana · iOS");
        assert_eq!(device_line("", "iOS", "A device"), "A device · iOS");
    }

    #[test]
    fn account_levels() {
        let a = Account::from_state(&json!({
            "lock": {"protection": "console"},
            "archive": {"state": "open", "recovery": "none"}
        }));
        assert!(!a.encrypted() && a.can_encrypt_later() && !a.strong());
        assert_eq!(a.protection_key(), "about.protection.console");
        let b = Account::from_state(&json!({
            "lock": {"protection": "password"},
            "archive": {"state": "open", "recovery": "set", "recovery_made": "2026-11-01T00:00:00Z"}
        }));
        assert!(b.encrypted() && b.recovery_set && !b.can_encrypt_later());
        assert_eq!(b.recovery_made.as_deref(), Some("2026-11-01T00:00:00Z"));
        let c = Account::from_state(&json!({"archive": {"serious": true}}));
        assert!(c.strong());
    }

    #[test]
    fn archive_disk_space() {
        let d = ArchiveDisk::from_state(&json!({
            "archive": {"state": "open", "size_bytes": 1000, "free_bytes": 250,
                        "place": {"label": "Photos"}}
        }));
        assert_eq!((d.used(), d.percent()), (750, 75));
        assert_eq!(d.place.as_deref(), Some("Photos"));
        assert!(!d.can_resume());
        assert!(ArchiveDisk::from_state(&json!({})).can_resume());
        assert_eq!(ArchiveDisk::default().percent(), 0);
    }

    #[test]
    fn scale_steps_map_both_ways() {
        assert_eq!(scale_index(None), 1);
        assert_eq!(scale_index(Some(1.0)), 0);
        assert_eq!(scale_index(Some(1.75)), 3);
        assert_eq!(scale_index(Some(1.25)), 1);
        assert_eq!(SCALE_STEPS[scale_index(Some(2.0))], Some(2.0));
    }

    #[test]
    fn lock_screen_choices_keep_one_order() {
        let cur = vec!["backup".to_string(), "phones".to_string()];
        assert_eq!(
            lock_hide_after(&cur, "network", false),
            ["phones", "network", "backup"]
        );
        assert_eq!(lock_hide_after(&cur, "phones", true), ["backup"]);
        assert!(lock_hide_after(&[], "space", true).is_empty());
    }

    #[test]
    fn display_prefs_follow_features() {
        let w = json!({
            "features": ["idle_lock_pref", "display_modes"],
            "prefs": {"ui_scale": 1.5, "lock_hide": ["space"], "natural_scrolling": true},
            "console": {"idle_lock_seconds": 900}
        });
        let p = DisplayPrefs::from_state(&w);
        assert_eq!((p.scale, p.idle), (Some(1.5), 900));
        assert!(p.idle_pref && p.modes && !p.lock_hide_pref && !p.natural_pref);
        assert_eq!(p.lock_hide, ["space"]);
        assert_eq!(DisplayPrefs::from_state(&json!({})).idle, 300);
    }

    #[test]
    fn modes_and_revert_time() {
        let m = modes(&json!({"modes": [
            {"id": 1, "width": 1920, "height": 1080, "refresh": 59.93, "current": true, "native": true},
            {"id": 2, "width": 1280, "height": 720},
            {"bad": true}]}));
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].text(), "1920 × 1080 · 60 Hz");
        assert_eq!(m[1].text(), "1280 × 720");
        assert!(m[0].current && m[0].native);
        assert_eq!(revert_in(&json!({"revert_in": 2})), 5);
        assert_eq!(revert_in(&json!({"revert_in": 99})), 30);
        assert_eq!(revert_in(&json!({})), 15);
    }

    #[test]
    fn reminders_follow_the_state() {
        let w = json!({
            "network": {"online": false},
            "lock": {"state": "ready", "protection": "strong"},
            "archive": {"state": "open", "recovery": "none"},
            "phone": {"paired": false}
        });
        let r: Vec<&str> = reminders(&w).iter().map(|x| x.0).collect();
        assert_eq!(r, ["network", "phone", "recovery"]);
        let done = json!({
            "network": {"online": true}, "lock": {"protection": "password"},
            "archive": {"state": "open", "recovery": "set", "password_reopen": true},
            "phone": {"paired": true}
        });
        assert!(reminders(&done).is_empty());
        let none = json!({"network": {"online": true}, "archive": {"state": "none"}});
        assert_eq!(reminders(&none)[0], ("where", "rem.where"));
        let reopen = json!({
            "network": {"online": true}, "lock": {"protection": "password"},
            "archive": {"state": "open", "password_reopen": false},
            "phone": {"paired": true}
        });
        assert_eq!(reminders(&reopen)[0].0, "pwreopen");
        // Recovery is listed by needs_you on its own, not twice.
        let n = needs_you(&w);
        assert!(n.iter().all(|x| x.2 != "recovery") || w["lock"]["protection"] == "strong");
    }

    #[test]
    fn disk_list_orders_and_locks() {
        let v = json!({"disks": [
            {"id": "i", "name": "nvme0n1", "model": "Internal", "kind": "internal", "role": "other"},
            {"id": "u", "name": "sdb", "model": "SanDisk", "kind": "external", "role": "other",
             "size_bytes": 32_000_000_000u64, "safe_to_remove": true,
             "parts": [{"partuuid": "p1", "label": "USB", "fs": "exfat", "size_bytes": 1}]},
            {"id": "b", "name": "sdc", "kind": "external", "role": "backup", "lock": "backup"}]});
        let d = disks(&v);
        assert_eq!(
            d.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(),
            ["u", "b", "i"]
        );
        assert!(d[0].can_format() && d[0].can_eject());
        assert_eq!(d[0].start_label(), "USB");
        assert_eq!(d[0].title(), "SanDisk");
        assert!(!d[1].can_format() && !d[1].can_eject());
        assert_eq!(d[1].lock_hint_key(), "disks.lock.startover");
        assert_eq!(d[1].title(), "sdc");
        assert!(d[2].offline() && d[2].lock.as_deref() == Some("internal_live"));
        assert_eq!(d[0].part_ids(), ["p1"]);
    }

    #[test]
    fn labels_are_checked() {
        assert!(label_ok("USB 2-a_b"));
        assert!(!label_ok(""));
        assert!(!label_ok("twelve chars"));
        assert!(!label_ok("bad/slash"));
    }

    #[test]
    fn format_body_names_what_was_shown() {
        let d = disks(
            &json!({"disks": [{"id": "u", "name": "sdb", "kind": "external",
            "parts": [{"partuuid": "p1"}, {"partuuid": "p2"}]}]}),
        )
        .remove(0);
        let est = estimate(&json!({"confirm": "tok", "secure": {"seconds": 150, "about": true}}));
        assert_eq!(est.secure_minutes(), 3);
        let b = format_body(&d, "secure", "pw", "MY DISK", "exfat", &est);
        assert_eq!(b["partuuids"], json!(["p1", "p2"]));
        assert_eq!(
            (&b["mode"], &b["confirm"], &b["label"]),
            (&json!("secure"), &json!("tok"), &json!("MY DISK"))
        );
        assert!(
            format_body(&d, "quick", "pw", "", "ext4", &est)
                .get("label")
                .is_none()
        );
    }

    #[test]
    fn jobs_are_found_by_id() {
        let w = json!({"archive": {"job": {"id": "disk.format", "state": "running", "percent": 40,
            "message": "Erasing", "seconds_left": 90, "can_cancel": true, "step": "erasing"}}});
        let j = job(&w, "disk.format").unwrap();
        assert_eq!(
            (j.percent, j.seconds_left, j.can_stop),
            (40, Some(90), true)
        );
        assert!(job(&w, "reset").is_none());
    }

    #[test]
    fn files_kinds_and_sizes() {
        assert_eq!(kind_of("IMG_1.JPG"), Kind::Image);
        assert_eq!(kind_of("a.mp4"), Kind::Video);
        assert_eq!(kind_of("notes.txt"), Kind::Text);
        assert_eq!(kind_of("archive.zip"), Kind::Other);
        assert_eq!(kind_of("noext"), Kind::Other);
        assert_eq!(file_size(850), "850 B");
        assert_eq!(file_size(12_400), "12 KB");
        assert_eq!(file_size(3_400_000), "3.4 MB");
        let l = listing(
            &json!({"entries": [{"name": "DCIM", "dir": true}, {"name": "a.txt", "size_bytes": 5}], "truncated": true, "skipped": 2}),
        );
        assert_eq!((l.entries.len(), l.truncated, l.skipped), (2, true, 2));
        assert!(l.entries[0].dir);
        let m = media(&json!({"media": [{"id": "m1", "label": "", "refused": "btrfs"}, {"x": 1}]}));
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].refused.as_deref(), Some("btrfs"));
        assert!(m[0].label.is_none());
    }

    #[test]
    fn terminal_address_must_be_the_local_port() {
        assert!(terminal_url_ok("http://127.0.0.1:8413/abc123/"));
        assert!(!terminal_url_ok("http://127.0.0.1:8411/abc/"));
        assert!(!terminal_url_ok("http://evil.example/"));
        assert!(!terminal_url_ok("http://127.0.0.1:8413@evil.example/"));
        assert!(!terminal_url_ok("https://127.0.0.1:8413/x"));
    }

    fn word(k: &str) -> String {
        format!("T:{k}")
    }

    #[test]
    fn messages_follow_the_state_and_resolve() {
        let mut log = ErrLog::default();
        let w = json!({"network": {"online": false}, "archive": {"state": "open"}});
        assert!(log.sync(&w, 100, &word));
        assert_eq!(log.list.len(), 1);
        assert_eq!(log.list[0].text, "T:status.warn.network");
        assert_eq!(log.tone(), Some(Lv::Warn));
        log.mark_seen(200);
        assert_eq!(log.tone(), None);
        // The same state again changes nothing.
        assert!(!log.sync(&w, 300, &word));
        // The problem goes away: the entry stays, marked resolved.
        let ok = json!({"network": {"online": true}, "archive": {"state": "open"}});
        assert!(log.sync(&ok, 400, &word));
        assert!(log.list[0].resolved);
        // It comes back: shown again as new.
        assert!(log.sync(&w, 500, &word));
        assert!(!log.list[0].resolved);
        assert_eq!(log.list[0].time, 500);
    }

    #[test]
    fn messages_take_errors_events_and_screen_failures() {
        let mut log = ErrLog::default();
        let w = json!({
            "network": {"online": true},
            "archive": {"state": "open", "error": {"key": "x", "code": "E123", "message": "boom"}},
            "events": [{"id": "e1", "level": "error", "message": "Disk failed", "resolved": true}]
        });
        log.sync(&w, 10, &word);
        let a = log.list.iter().find(|m| m.id == "st:arch.error").unwrap();
        assert_eq!(
            (a.code.as_deref(), a.detail.as_deref()),
            (Some("E123"), Some("boom"))
        );
        let e = log.list.iter().find(|m| m.id == "sv:e1").unwrap();
        assert!(e.resolved && e.level == Lv::Bad);
        assert!(log.add(Msg::new("", Lv::Bad, "An action failed"), 20));
        assert!(log.list[0].id.starts_with('c'));
        // Two failures of the same action make two entries (own ids).
        assert!(log.add(Msg::new("", Lv::Bad, "An action failed"), 21));
        assert_eq!(
            log.list
                .iter()
                .filter(|m| m.text == "An action failed")
                .count(),
            2
        );
    }

    #[test]
    fn the_saver_choice_is_kept_in_a_file() {
        assert!(!LocalPrefs::parse("garbage").saver);
        assert!(LocalPrefs::parse(r#"{"saver": true}"#).saver);
        let dir = std::env::temp_dir().join(format!("alp-screen-test-{}", std::process::id()));
        let file = dir.join("p").join("prefs.json");
        LocalPrefs { saver: true }.save(&file).unwrap();
        assert!(LocalPrefs::load(&file).saver);
        assert!(!LocalPrefs::load(&dir.join("none.json")).saver);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn idle_rule_waits_longer_in_the_gallery() {
        assert!(idle_over(300, 300, false));
        assert!(!idle_over(300, 299, false));
        assert!(!idle_over(300, 900, true));
        assert!(idle_over(300, 1800, true));
    }
}
