// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/api/v1/live/...` for ALP-OS Live (contract: docs/plans/live-alpha-api.md).
//! The logic is in the `alp-live` crate, shared with `alpctl`.

use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use alp_live::fail::Fail;
use alp_live::{
    admin, archive, chain, disks, display, events, format, immich, index, job, live, pairing,
    prefs, qr, recovery, wifi,
};
use serde_json::{Value, json};

use crate::console::{Request, Response};
use crate::live_media;
use crate::live_terminal;

const PREFIX: &str = "/api/v1/live/";
/// Seconds an Immich answer is reused by `GET state` (it is polled often).
const IMMICH_CACHE: Duration = Duration::from_secs(5);
/// The screen locks by itself after this long without activity (owner
/// decision 8: idle auto-lock).
const COOKIE: &str = "alp_console";

/// The console session: one browser (the kiosk) signed in with the admin
/// password. The token lives in alpd's memory only; a restart locks.
#[derive(Default)]
struct Console {
    token: Option<zeroize::Zeroizing<String>>,
    last: Option<Instant>,
}

/// Seconds without progress after which a job says it is still waiting
/// for the disk.
const STILL_WORKING: u64 = 60;

#[derive(Debug, Clone)]
struct Job {
    id: &'static str,
    /// `running`, `cancelling`, `done`, `failed` or `cancelled`.
    state: &'static str,
    step: String,
    message: String,
    percent: Option<u8>,
    error: Option<Fail>,
    done_bytes: Option<u64>,
    total_bytes: Option<u64>,
    bytes_per_second: Option<u64>,
    seconds_left: Option<u64>,
    started: Instant,
    /// Last time the job moved (any update).
    moved: Instant,
    /// Set by `archive/cancel`; the job stops at its next check.
    cancel: Arc<AtomicBool>,
}

impl Job {
    fn start(id: &'static str, step: &str, message: &str, percent: Option<u8>) -> Self {
        Self {
            id,
            state: "running",
            step: step.into(),
            message: message.into(),
            percent,
            error: None,
            done_bytes: None,
            total_bytes: None,
            bytes_per_second: None,
            seconds_left: None,
            started: Instant::now(),
            moved: Instant::now(),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Still working (also while stopping after a cancel).
    fn running(j: &Option<Self>) -> bool {
        j.as_ref()
            .is_some_and(|j| j.state == "running" || j.state == "cancelling")
    }

    fn apply(&mut self, u: &job::Update) {
        self.step = u.step.into();
        self.message.clone_from(&u.message);
        self.percent = Some(u.percent);
        self.done_bytes = u.done_bytes;
        self.total_bytes = u.total_bytes;
        self.bytes_per_second = u.bytes_per_second;
        self.seconds_left = u.seconds_left();
        self.moved = Instant::now();
    }

    /// Computed when asked, so it changes every second even while the disk
    /// keeps the job waiting: `seconds_since_progress` grows then.
    fn json(&self) -> Value {
        let quiet = self.moved.elapsed().as_secs();
        let message = if self.state == "running"
            && quiet >= STILL_WORKING
            && !self.id.starts_with("power.")
        {
            format!(
                "Still working: the disk has not answered for {}. Keep it plugged in, or press Cancel.",
                job::about(quiet)
                    .replace("about ", "")
                    .replace("less than a minute", "a minute")
            )
        } else {
            self.message.clone()
        };
        json!({"id": self.id, "state": self.state, "step": self.step,
               "message": message, "percent": self.percent,
               "bytes_done": self.done_bytes, "bytes_total": self.total_bytes,
               "bytes_per_second": self.bytes_per_second, "seconds_left": self.seconds_left,
               "seconds_running": self.started.elapsed().as_secs(),
               "seconds_since_progress": self.moved.elapsed().as_secs(),
               "can_cancel": self.state == "running"
                   && (self.id == "archive.create"
                       || (self.id == "disk.format" && self.step == "erasing")),
               "error": self.error.as_ref().map(fail_json)})
    }

    /// Plain-text status for `alpctl live diag` (no secrets).
    fn text(&self) -> String {
        format!(
            "job: {}\nstate: {}\nstep: {}\npercent: {:?}\nmessage: {}\nbytes: {:?} of {:?}\nspeed (bytes/s): {:?}\nseconds running: {}\nseconds since progress: {}\nerror: {}\n",
            self.id,
            self.state,
            self.step,
            self.percent,
            self.message,
            self.done_bytes,
            self.total_bytes,
            self.bytes_per_second,
            self.started.elapsed().as_secs(),
            self.moved.elapsed().as_secs(),
            self.error.as_ref().map_or_else(
                || "none".to_string(),
                |e| format!("{} {} {}", e.key, e.code.unwrap_or("-"), e.message)
            ),
        )
    }
}

#[derive(Default, Clone)]
struct ImmichCache {
    at: Option<Instant>,
    state: Value,
    phone: Value,
    /// When alpd first saw Immich starting (T21).
    starting_since: Option<Instant>,
    /// Last time alpd started a stopped `alp-immich.service` itself.
    kicked: Option<Instant>,
}

/// The main password, kept in memory only until the family account uses
/// it (T23: phones sign in with the main password). Taken by the worker.
#[derive(Default)]
struct FamilySync {
    pending: Mutex<Option<zeroize::Zeroizing<String>>>,
    worker: AtomicBool,
    /// One change of the family account at a time (the worker and
    /// `phone/pair` would otherwise each make a key and delete the other's).
    busy: Mutex<()>,
}

impl FamilySync {
    /// Give the family account `pw` (once; nothing if it has it already).
    fn apply(&self, pw: &str) -> Result<(), Fail> {
        let _one = lock(&self.busy);
        if immich::family_uses_main_password() {
            return Ok(());
        }
        immich::set_family_password(pw)
    }
}

/// How long a new recovery code waits for its words to be confirmed.
const RECOVERY_PENDING_FOR: Duration = Duration::from_secs(15 * 60);
/// Wrong confirmations before a pending code is thrown away.
const RECOVERY_COMMIT_TRIES: u8 = 5;

/// A recovery code made by `recovery/create`, waiting for `recovery/commit`.
struct PendingRecovery {
    made: Instant,
    words: zeroize::Zeroizing<Vec<String>>,
    /// The positions (1 to 15) the user must type back, chosen here.
    ask: [usize; 3],
    tries: u8,
}

/// The storage seam of the recovery code (`archive` in alpd).
#[derive(Clone, Copy)]
struct RecoveryStore {
    has: fn() -> bool,
    bind: fn(&str) -> Result<(), String>,
    commit: fn() -> Result<(), String>,
    cancel: fn(),
}

const ARCHIVE_RECOVERY: RecoveryStore = RecoveryStore {
    has: archive::has_recovery,
    bind: recovery::bind,
    commit: recovery::commit,
    cancel: recovery::cancel,
};

/// Three different positions out of 15 (kernel randomness), in order.
fn recovery_positions() -> Result<[usize; 3], Fail> {
    use std::io::Read;
    let mut picked: Vec<usize> = Vec::with_capacity(3);
    let mut b = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .map_err(|e| Fail::new("internal", format!("No random numbers: {e}")))?;
    // 240 = 16 × 15: no bias. 16 bytes leave three usable ones but with a
    // chance of about 1e-9; draw again then.
    for &x in b.iter().filter(|&&x| x < 240) {
        let n = usize::from(x) % recovery::WORDS + 1;
        if !picked.contains(&n) {
            picked.push(n);
        }
        if picked.len() == 3 {
            picked.sort_unstable();
            return Ok([picked[0], picked[1], picked[2]]);
        }
    }
    recovery_positions()
}

pub struct Live {
    /// The hardware check, made in the background at start
    /// ([`Live::watch_hardware`]): alpd answers before it is done (T28).
    hardware: Arc<Mutex<Value>>,
    /// `system.battery`, read in the background: a laptop's battery
    /// (ACPI) can take long to answer, and the state must not wait for it.
    battery: Arc<Mutex<Value>>,
    /// The connected Wi-Fi network, asked of iwd in the background: a busy
    /// iwd held the state up to the `iwctl` limit (T28).
    ssid: Arc<Mutex<Option<String>>>,
    chain: chain::Store,
    wifi_job: Mutex<Option<Job>>,
    archive_job: Mutex<Option<Job>>,
    networks: Mutex<Vec<wifi::Network>>,
    immich: Mutex<ImmichCache>,
    tries: Mutex<Tries>,
    /// Wrong chain codes (owner decision 2026-10-24: joins are rate-limited).
    join_tries: Mutex<Tries>,
    /// Immich stopped because the index or the archive is full.
    paused: Mutex<bool>,
    low_percent: u64,
    /// Archive, phone and chain calls need the unlocked index, and most
    /// calls a signed-in console (false only in unit tests).
    require_index: bool,
    console: Mutex<Console>,
    /// Has the open archive a recovery key? (cached: `luksDump` per poll.)
    recovery: Mutex<Option<(Instant, bool)>>,
    /// A new recovery code shown but not yet confirmed (T42): its words
    /// live only here, so a restart forgets it and the old code stays.
    pending_recovery: Mutex<Option<PendingRecovery>>,
    /// One-time format confirmations (review R4), dropped at lock and sign-in.
    confirms: Mutex<format::Confirms>,
    /// The archive disk's own health from the last background check
    /// (`healthy`, `wear`, `failing`; None: unknown). Set and cleared only
    /// by that check (review R7).
    archive_health: Mutex<Option<&'static str>>,
    /// The storage calls behind the recovery code (replaced in unit tests).
    rec: RecoveryStore,
    /// Checks the admin password (`index::check`; replaced in unit tests).
    password_ok: fn(&str) -> Result<(), Fail>,
    /// Deletes the archive and the index (`reset_everything`; replaced in
    /// unit tests).
    wipe: fn(job::Progress) -> Result<String, Fail>,
    /// Formats an external disk (`format_disk`, T49; replaced in unit
    /// tests). Returns the method used.
    format: FormatFn,
    /// The read-only disk check (replaced in unit tests).
    check: CheckFn,
    /// Serious encryption chosen (`index::serious`; replaced in unit tests).
    serious: fn() -> bool,
    family: Arc<FamilySync>,
    /// The ordered shutdown (`power/off`, T15).
    power_job: Mutex<Option<Job>>,
    /// Phone pairing (ADR 0017): window, token and waiting requests.
    pairing: Mutex<pairing::Pairing>,
    /// Console preferences (T56), kept in `prefs_file` on the stick.
    prefs: Mutex<prefs::Prefs>,
    prefs_file: std::path::PathBuf,
    /// An unconfirmed screen mode change (T56).
    display_pending: Mutex<Option<DisplayPending>>,
    /// Runs `wlr-randr` (`display::randr`; replaced in unit tests).
    randr: fn(&[String]) -> Result<String, String>,
    /// How long a new screen mode waits for "keep" (15 s).
    revert_after: Duration,
}

/// A screen mode change waiting for `display/mode/keep`.
#[derive(Debug, Clone)]
struct DisplayPending {
    /// Tells the revert timer whether its change is still the pending one.
    serial: u64,
    output: String,
    back: prefs::Mode,
    new: prefs::Mode,
}

static DISPLAY_SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Wrong-password attempts (in memory; a restart resets them, which costs
/// the attacker a reboot each time).
#[derive(Default)]
struct Tries {
    failures: u32,
    wait_until: Option<Instant>,
}

impl Tries {
    /// Seconds to wait before the next try is allowed.
    fn wait(&self) -> u64 {
        self.wait_until
            .map_or(0, |t| t.saturating_duration_since(Instant::now()).as_secs())
    }

    fn failed(&mut self) {
        self.failures += 1;
        if self.failures >= 5 {
            let secs = 30u64
                .saturating_mul(1 << (self.failures - 5).min(7))
                .min(3600);
            self.wait_until = Some(Instant::now() + Duration::from_secs(secs));
        }
    }
}

fn locked() -> Fail {
    Fail::new(
        "locked",
        "Type your ALPArchive password first to open your archive.",
    )
}

fn fail_json(f: &Fail) -> Value {
    let mut v = json!({"key": f.key, "message": f.message, "code": f.code, "level": f.level()});
    if let Some(i) = f.index {
        v["index"] = i.into();
    }
    if let Some(w) = f.retry_after {
        v["retry_after_seconds"] = w.into();
    }
    v
}

fn status_for(key: &str) -> u16 {
    match key {
        "chain.none" | "wifi.not_found" | "archive.not_found" => 404,
        "chain.exists"
        | "wifi.busy"
        | "wifi.no_device"
        | "archive.exists"
        | "archive.busy"
        | "setup.busy"
        | "archive.not_creating"
        | "archive.not_open"
        | "network.offline" => 409,
        "archive.refused" => 422,
        "archive.forget" | "format.confirm_stale" | "archive.critical" => 409,
        "immich.starting" => 503,
        "locked" | "index.exists" | "state.not_ready" => 409,
        "password.wrong" | "recovery.wrong" | "disk.locked" => 403,
        "disk.open" => 409,
        "prefs.invalid" | "setup.invalid" => 422,
        "display.busy" | "display.no_pending" => 409,
        "display.not_found" => 404,
        "display.unavailable" => 503,
        "display.failed" => 500,
        "disk.eject_failed" => 500,
        "console.locked" => 401,
        "recovery.failed" => 500,
        "password.wait" | "chain.wait" => 429,
        "recovery.not_ready" | "recovery.old_archive" | "recovery.not_encrypted" => 409,
        "recovery.exists" | "recovery.no_pending" => 409,
        "recovery.expired" | "recovery.commit_gone" => 410,
        "recovery.commit_wrong" => 422,
        "disk.not_found" => 404,
        "disk.changed" => 409,
        "disk.refused" => 422,
        "media.not_found" => 404,
        "media.not_open" | "media.busy" => 409,
        "media.refused" | "media.type" => 422,
        "media.too_large" => 413,
        "media.failed" | "media.unreadable" => 500,
        "request.forbidden" => 403,
        "phone.reset_gone" => 410,
        "terminal.running" | "terminal.no_password" => 409,
        "terminal.failed" => 500,
        "index.missing" => 404,
        "index.failed" => 500,
        "internal" | "immich.failed" | "reset.failed" => 500,
        _ => 400,
    }
}

fn err(f: &Fail) -> Response {
    Response::json(status_for(f.key), &json!({"error": fail_json(f)}))
}

/// `state.setup` (T09): `complete` is `finished` under the name the
/// `setup_progress` feature uses.
fn setup_json() -> Value {
    let done = archive::setup_done();
    json!({"finished": done, "complete": done, "skipped": archive::setup_skipped()})
}

/// What a `setup/progress` request asks for: the new skipped list (if
/// given) and whether setup is complete. Checked before anything is kept.
fn setup_request(b: &Value) -> Result<(Option<Vec<&'static str>>, bool), Fail> {
    let bad = || Fail::new("setup.invalid", "This setup progress is not valid.");
    let mut skipped = None;
    let mut complete = false;
    for (k, v) in b.as_object().ok_or_else(bad)? {
        match k.as_str() {
            "complete" => complete = v.as_bool().filter(|c| *c).ok_or_else(bad)?,
            "skipped" => {
                let names: Option<Vec<&str>> = v
                    .as_array()
                    .ok_or_else(bad)?
                    .iter()
                    .map(Value::as_str)
                    .collect();
                skipped = Some(names.and_then(archive::setup_steps_of).ok_or_else(bad)?);
            }
            _ => return Err(bad()),
        }
    }
    Ok((skipped, complete))
}

fn setup_progress(b: &Value) -> Result<Response, Fail> {
    let (skipped, complete) = setup_request(b)?;
    if let Some(s) = skipped {
        archive::set_setup_skipped(&s).map_err(|e| Fail::new("internal", e))?;
    }
    if complete {
        archive::set_setup_done().map_err(|e| Fail::new("internal", e))?;
    }
    Ok(Response::json(200, &json!({"setup": setup_json()})))
}

fn body(req: &Request) -> Result<Value, Fail> {
    serde_json::from_slice::<Value>(&req.body)
        .ok()
        .filter(Value::is_object)
        .ok_or_else(|| Fail::new("request.json", "The request is not valid JSON."))
}

fn field<'a>(v: &'a Value, k: &str) -> Result<&'a Value, Fail> {
    v.get(k)
        .ok_or_else(|| Fail::new("request.json", format!("The request misses \"{k}\".")))
}

fn systemctl(args: &[&str]) {
    let _ = Command::new("systemctl").args(args).status();
}

/// Seconds the photo service may take to stop at shutdown (its database
/// gets a clean stop in a few seconds; a slow USB disk needs more).
const IMMICH_STOP_SECS: u64 = 35;

/// Run a command, waiting at most `secs`; true when it ended well in time
/// (a late one is killed; a `systemctl` job it started goes on).
fn bounded(cmd: &str, args: &[&str], secs: u64) -> bool {
    let Ok(mut child) = Command::new(cmd)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .spawn()
    else {
        return false;
    };
    let t = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(s)) => return s.success(),
            Ok(None) if t.elapsed() < Duration::from_secs(secs) => {
                std::thread::sleep(Duration::from_millis(200));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// Stable id of a hardware check (the English `item` may change).
pub fn check_id(item: &str) -> String {
    match item {
        "Processor" => "cpu".into(),
        "Memory" => "memory".into(),
        "Startup (UEFI)" => "uefi".into(),
        "Secure Boot" => "secure_boot".into(),
        "Security chip (TPM)" => "tpm".into(),
        "Storage" => "storage".into(),
        "Graphics" => "graphics".into(),
        "Wired network" => "wired_network".into(),
        other => other
            .to_ascii_lowercase()
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|w| !w.is_empty())
            .collect::<Vec<_>>()
            .join("_"),
    }
}

fn hardware_json() -> Value {
    let report_file = std::fs::read_to_string(live::REPORT_NAME_FILE)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    match live::hardware() {
        Ok((r, checks)) => {
            let word = |s: alp_hw::inspect::Status| match s {
                alp_hw::inspect::Status::Ok => "ok",
                alp_hw::inspect::Status::Note => "note",
                alp_hw::inspect::Status::Fail => "fail",
            };
            let worst = checks
                .iter()
                .map(|c| c.status)
                .max_by_key(|s| *s as u8)
                .map_or("ok", word);
            json!({
                "machine": r.machine,
                "verdict": worst,
                "checks": checks.iter().map(|c| json!({
                    "id": check_id(c.item), "code": live::check_code(c, &r), "item": c.item, "status": word(c.status), "value": c.value, "advice": c.advice
                })).collect::<Vec<_>>(),
                "report_file": report_file,
                "specs": specs_json(&r),
            })
        }
        Err(e) => {
            eprintln!("alpd: hardware check: {e}");
            json!({"machine": "", "verdict": "note", "checks": [], "report_file": report_file,
                   "specs": null})
        }
    }
}

/// `hardware` until the check at start is done (usually well under a
/// second): no checks yet, nothing failed.
fn hardware_pending() -> Value {
    json!({"machine": "", "verdict": "note", "checks": [], "report_file": null,
           "specs": null, "pending": true})
}

/// "This computer" (T25): the facts of the hardware report, for display.
pub fn specs_json(r: &alp_hw::inspect::HwReport) -> Value {
    use alp_hw::DiskTransport as T;
    use alp_hw::inspect::NicKind;
    json!({
        "cpu": {"model": r.cpu.model, "threads": r.cpu.threads},
        "memory_bytes": r.ram_bytes,
        "disks": r.disks.iter().map(|d| json!({
            "name": d.id.0, "model": d.model, "size_bytes": d.size_bytes,
            "kind": match d.transport {
                T::Nvme => "nvme", T::Sata => "sata", T::Usb => "usb",
                T::Virtio => "virtio", T::Other => "other",
            },
            "internal": alp_hw::inspect::is_internal(d), "system": d.is_system_disk,
        })).collect::<Vec<_>>(),
        "graphics": r.gpus.iter().map(|g| json!({"vendor": g.vendor, "pci_id": g.pci_id}))
            .collect::<Vec<_>>(),
        "network": r.nics.iter().map(|n| json!({
            "name": n.name, "kind": if n.kind == NicKind::Wireless { "wifi" } else { "wired" },
            "driver": n.driver, "usb": n.usb, "working": n.working,
        })).collect::<Vec<_>>(),
        "tpm": r.tpm_version.map(|v| if v >= 2 { "2.0" } else { "1.2" }),
        "uefi": r.uefi,
        "secure_boot": r.secure_boot,
        "battery": alp_hw::inspect::battery(Path::new("/sys"))
            .map(|b| json!({"model": b.model, "health_percent": b.health_percent})),
    })
}

/// `system.release` (T31): the image's name and version for Settings >
/// About, from `/usr/lib/alp/release` (set for each release); `null` if
/// missing.
fn release() -> Value {
    std::fs::read_to_string("/usr/lib/alp/release")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .into()
}

/// `system.build`: "<commit> <date>" written into the image by `just
/// build-image` (`/usr/lib/alp/build`); `null` if missing.
fn build() -> Value {
    std::fs::read_to_string("/usr/lib/alp/build")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .into()
}

/// `system.battery` (T18): `null` on a desktop.
fn battery_json(sys: &Path) -> Value {
    alp_hw::inspect::battery(sys).map_or(
        Value::Null,
        |b| json!({"percent": b.percent, "charging": b.charging}),
    )
}

/// `network.addresses` (T14): every home-network address, wired first.
fn addresses_json(ports: &[live::Port]) -> Value {
    let mut v: Vec<&live::Port> = ports.iter().filter(|p| p.address.is_some()).collect();
    v.sort_by_key(|p| p.wifi);
    v.iter()
        .filter_map(|p| {
            let a = p.address.as_deref()?;
            Some(json!({"address": a, "url": server_url(a), "port": p.name,
                        "kind": if p.wifi { "wifi" } else { "wired" }}))
        })
        .collect::<Vec<_>>()
        .into()
}

impl Live {
    pub fn new(chain: chain::Store) -> Arc<Self> {
        // No format job survives a restart (review G1).
        format::clear_stale();
        Arc::new(Self {
            hardware: Arc::new(Mutex::new(hardware_pending())),
            battery: Arc::new(Mutex::new(Value::Null)),
            ssid: Arc::new(Mutex::new(None)),
            chain,
            wifi_job: Mutex::new(None),
            archive_job: Mutex::new(None),
            networks: Mutex::new(Vec::new()),
            immich: Mutex::new(ImmichCache::default()),
            tries: Mutex::new(Tries::default()),
            join_tries: Mutex::new(Tries::default()),
            paused: Mutex::new(false),
            low_percent: live::capacity_low_percent(),
            require_index: true,
            console: Mutex::new(Console::default()),
            recovery: Mutex::new(None),
            pending_recovery: Mutex::new(None),
            confirms: Mutex::new(format::Confirms::default()),
            archive_health: Mutex::new(None),
            rec: ARCHIVE_RECOVERY,
            password_ok: index::check,
            wipe: reset_everything,
            format: format_disk,
            check: disks::check_disk,
            serious: index::serious,
            family: Arc::new(FamilySync::default()),
            power_job: Mutex::new(None),
            pairing: Mutex::new(pairing::Pairing::default()),
            prefs: Mutex::new(prefs::Prefs::load_from(Path::new(prefs::PATH))),
            prefs_file: prefs::PATH.into(),
            display_pending: Mutex::new(None),
            randr: display::randr,
            revert_after: Duration::from_secs(15),
        })
    }

    /// At start: let the TPM chip open the index (owner decision 2); the
    /// archive and Immich follow. Without a chip, or when it refuses, the
    /// screen asks for the password.
    pub fn auto_unlock(self: &Arc<Self>) {
        std::thread::spawn(|| {
            match index::tpm_unlock().inspect(|_| {
                // Encrypting was cut off before cryptsetup began (ADR 0018): the
                // stick stays unprotected and can start it again.
                if let Err(e) = index::abort_unstarted_encrypt() {
                    eprintln!("alpd: encrypt: {e}");
                }
            }) {
                Ok(true) => {
                    eprintln!(
                        "alpd: the index is open ({})",
                        if index::plain() {
                            "unprotected"
                        } else {
                            "the security chip opened it"
                        }
                    );
                    systemctl(&["start", "alp-live-index.service"]);
                    if archive::record().is_some() {
                        systemctl(&["start", "--no-block", "alp-live-archive.service"]);
                        systemctl(&["start", "--no-block", immich::UNIT]);
                    }
                }
                Ok(false) => {}
                Err(e) => eprintln!("alpd: the security chip did not open the index: {e}"),
            }
        });
    }

    /// How long the console stays signed in without use ("Lock after").
    fn idle_lock(&self) -> Duration {
        Duration::from_secs(u64::from(lock(&self.prefs).idle_lock_seconds))
    }

    /// Is the console signed in for this request? First start (no password
    /// yet) counts as signed in. Touches the idle timer when `touch`.
    fn console_open(&self, req: &Request, touch: bool) -> bool {
        if !self.require_index || index::lock_state() == "setup" {
            return true;
        }
        // Level 1 (ADR 0016): no password chosen, nothing to sign in with.
        if index::configured().is_some() && index::protection() == "none" {
            return true;
        }
        let mut c = lock(&self.console);
        let idle = self.idle_lock();
        let fresh = c.last.is_some_and(|t| t.elapsed() < idle);
        let ok = fresh
            && c.token
                .as_ref()
                .is_some_and(|t| cookie(req, COOKIE).is_some_and(|v| ct_eq(v, t)));
        if !fresh {
            c.token = None;
        }
        if ok && touch {
            c.last = Some(Instant::now());
        }
        ok
    }

    /// `GET gallery/gate`: nginx asks this before it passes a gallery
    /// request on 127.0.0.1:8412 to the photo service (`auth_request`). The
    /// answer carries the family account's key for nginx to add, only while
    /// the console is signed in: locking the console closes the gallery at
    /// once, with nothing to clean up. A browser can never read the key:
    /// every browser request carries `Sec-Fetch-Site` (nginx does not pass
    /// it on), and those are refused here. 401 for every refusal.
    fn gallery_gate(&self, req: &Request, get: bool) -> Response {
        let no = || Response::json(401, &json!({}));
        if !get || req.header("sec-fetch-site").is_some() || req.header("origin").is_some() {
            return no();
        }
        if !self.console_open(req, false) {
            return no();
        }
        let Some(key) = immich::family_key() else {
            return no();
        };
        let mut r = Response::json(200, &json!({}));
        r.headers
            .push(("X-Alp-Gallery-Key", key.as_str().to_string()));
        r
    }

    /// A new console session; the cookie goes on `r`.
    fn sign_in(&self, mut r: Response) -> Response {
        let token = random_hex();
        r.headers.push((
            "Set-Cookie",
            format!(
                "{COOKIE}={}; Path=/api/; HttpOnly; SameSite=Strict",
                token.as_str()
            ),
        ));
        let mut c = lock(&self.console);
        c.token = Some(token);
        c.last = Some(Instant::now());
        lock(&self.confirms).clear();
        r
    }

    /// Wait after several wrong passwords (shared by every password check).
    fn password_wait(&self) -> Result<(), Fail> {
        let wait = lock(&self.tries).wait();
        if wait > 0 {
            let mut f = Fail::new(
                "password.wait",
                format!("Too many tries. Please wait {wait} seconds, then try again."),
            );
            f.retry_after = Some(wait);
            return Err(f);
        }
        Ok(())
    }

    /// Check the admin password (sign-in, delete, empty trash).
    fn check_password(&self, pw: &str) -> Result<(), Fail> {
        self.password_wait()?;
        match (self.password_ok)(pw) {
            Ok(()) => {
                *lock(&self.tries) = Tries::default();
                // Level 1 has no password: whatever was typed must not
                // become the phones' password (review of T35).
                if index::protection() != "none" {
                    self.main_password_typed(pw);
                }
                Ok(())
            }
            Err(e) => {
                if e.key == "password.wrong" {
                    lock(&self.tries).failed();
                }
                Err(e)
            }
        }
    }

    /// The main password was typed and is right: the family account gets
    /// it as its password (T23) once Immich answers. Kept in memory only,
    /// at most 45 minutes; nothing when the family uses it already.
    fn main_password_typed(&self, pw: &str) {
        if !self.require_index {
            return;
        }
        // Level 3 (ADR 0016): the ALP password also reopens the disk. After an
        // unlock the archive opens a moment later: wait for it (at most 5 min).
        let typed = zeroize::Zeroizing::new(pw.to_string());
        std::thread::spawn(move || {
            let t = Instant::now();
            while !archive::is_open() && t.elapsed() < Duration::from_secs(300) {
                std::thread::sleep(Duration::from_secs(5));
            }
            if let Err(e) = archive::bind_password(&typed) {
                eprintln!("alpd: password reopening not set: {e}");
            }
        });
        if immich::family_uses_main_password() {
            return;
        }
        {
            // Under the lock: the worker clears `pending` and ends under it
            // too, so a password is never left without a worker to take it.
            let mut pending = lock(&self.family.pending);
            *pending = Some(zeroize::Zeroizing::new(pw.to_string()));
            if self.family.worker.swap(true, Ordering::SeqCst) {
                return;
            }
        }
        let fam = Arc::clone(&self.family);
        std::thread::spawn(move || {
            let t = Instant::now();
            while t.elapsed() < Duration::from_secs(45 * 60) && index::is_open() {
                if lock(&fam.pending).is_none() {
                    break;
                }
                if immich::ping() {
                    if let Some(pw) = lock(&fam.pending).take() {
                        match fam.apply(&pw) {
                            Ok(()) => eprintln!("alpd: phones now sign in with the main password"),
                            Err(e) => eprintln!("alpd: family account not changed: {}", e.message),
                        }
                    }
                    break;
                }
                std::thread::sleep(Duration::from_secs(10));
            }
            let mut pending = lock(&fam.pending);
            *pending = None;
            fam.worker.store(false, Ordering::SeqCst);
        });
    }

    /// Signed in now, without a request (the Terminal's watcher). Uses the
    /// same "Lock after" time as the console, so both lock together.
    fn console_fresh(&self) -> bool {
        let c = lock(&self.console);
        let idle = self.idle_lock();
        c.token.is_some() && c.last.is_some_and(|t| t.elapsed() < idle)
    }

    /// `POST terminal/start {password}` (docs/plans/live-terminal.md, T5):
    /// behind the console sign-in like every route, and the password again
    /// each time, in the same counter and back-off as sign-in. Not without
    /// a password (protection level 1 or setup): there is nothing to check.
    fn terminal_start(self: &Arc<Self>, req: &Request) -> Result<Response, Fail> {
        if self.require_index && (index::lock_state() == "setup" || index::protection() == "none") {
            return Err(Fail::new(
                "terminal.no_password",
                "Choose an ALPArchive password first to use the Terminal.",
            ));
        }
        let pw = password_field(req)?;
        self.check_password(&pw)?;
        let me = Arc::clone(self);
        let url = live_terminal::start(move || me.console_fresh())?;
        Ok(Response::json(200, &json!({"url": url})))
    }

    fn signin(&self, req: &Request) -> Result<Response, Fail> {
        let pw = password_field(req)?;
        self.check_password(&pw)?;
        Ok(self.sign_in(Response::json(200, &json!({}))))
    }

    /// `POST recovery/create {password, replace, passphrase}`: a new
    /// 15-word recovery code (ADR 0013), shown once. Two steps (T42,
    /// security review 2026-11-01): this binds it to the archive only as a
    /// pending config; the current code keeps working until
    /// `recovery/commit` checks three of the words. The ALP password is
    /// checked here (a wrong one counts in the back-off); an existing code
    /// is replaced only with `replace: true`. With serious encryption the
    /// extra passphrase is required.
    fn recovery_create(&self, req: &Request) -> Result<Response, Fail> {
        let b = body(req)?;
        if self.require_index && !archive::is_open() {
            return Err(not_open());
        }
        let pw = password_field(req)?;
        self.check_password(&pw)?;
        let passphrase = zeroize::Zeroizing::new(
            b.get("passphrase")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        );
        if (self.serious)() {
            recovery::check_passphrase(&passphrase)?;
        } else if !passphrase.is_empty() {
            return Err(Fail::new(
                "recovery.passphrase",
                "An extra passphrase is only used when Encryption is turned on.",
            ));
        }
        let replace = b.get("replace").and_then(Value::as_bool) == Some(true);
        let mut pending = lock(&self.pending_recovery);
        if !replace && (self.rec.has)() {
            return Err(Fail::new(
                "recovery.exists",
                "This archive has a recovery code already. Replacing it makes the old words stop working.",
            ));
        }
        // A code shown earlier and never confirmed is forgotten first.
        if pending.take().is_some() {
            (self.rec.cancel)();
        }
        let code = recovery::generate()?;
        let secret = code.unlock_secret(&passphrase);
        (self.rec.bind)(&secret).map_err(|e| recovery_fail(&e))?;
        let ask = recovery_positions()?;
        *pending = Some(PendingRecovery {
            made: Instant::now(),
            words: zeroize::Zeroizing::new(code.words().to_vec()),
            ask,
            tries: 0,
        });
        let phrase = code.phrase();
        let qr_text = code.qr_text();
        Ok(Response::json(
            200,
            &json!({"words": code.words(), "recovery_key": phrase.as_str(),
                    "qr_text": qr_text.as_str(), "qr_svg": qr::svg(&qr_text),
                    "check": ask, "expires_seconds": RECOVERY_PENDING_FOR.as_secs()}),
        ))
    }

    /// `POST recovery/commit {words: {"<position>": "<word>", …}}` (T42):
    /// the three words asked for by `recovery/create` are typed back; then
    /// the new code replaces the old one. Wrong words keep the old code;
    /// after a few tries, or 15 minutes, the new code is thrown away.
    fn recovery_commit(&self, req: &Request) -> Result<Response, Fail> {
        let b = body(req)?;
        let typed = b
            .get("words")
            .and_then(Value::as_object)
            .ok_or_else(|| Fail::new("request.json", "\"words\" must be an object."))?;
        let mut pending = lock(&self.pending_recovery);
        let Some(p) = pending.as_mut() else {
            return Err(Fail::new(
                "recovery.no_pending",
                "No new recovery code is waiting. Your current code still works.",
            ));
        };
        if p.made.elapsed() > RECOVERY_PENDING_FOR {
            *pending = None;
            (self.rec.cancel)();
            return Err(Fail::new(
                "recovery.expired",
                "The new code was not confirmed in time, so it was thrown away. Your current code still works.",
            ));
        }
        let right = p.ask.iter().fold(true, |ok, &i| {
            let w = typed
                .get(&i.to_string())
                .and_then(Value::as_str)
                .map(|w| w.trim().to_lowercase())
                .unwrap_or_default();
            ct_eq(&w, &p.words[i - 1]) & ok
        });
        if !right {
            p.tries += 1;
            if p.tries >= RECOVERY_COMMIT_TRIES {
                *pending = None;
                (self.rec.cancel)();
                return Err(Fail::new(
                    "recovery.commit_gone",
                    "Too many wrong words, so the new code was thrown away. Your current code still works; you can make a new one.",
                ));
            }
            return Err(Fail::new(
                "recovery.commit_wrong",
                "These words do not match the new code. Please check them against what you wrote down.",
            ));
        }
        (self.rec.commit)().map_err(|e| recovery_fail(&e))?;
        *pending = None;
        *lock(&self.recovery) = None;
        // The date only, never the words (the (i) log reads the journal).
        eprintln!("alpd: recovery code replaced");
        Ok(Response::json(200, &json!({})))
    }

    /// `POST recovery/cancel`: forget a new code that was not confirmed.
    fn recovery_cancel(&self) -> Response {
        if lock(&self.pending_recovery).take().is_some() {
            (self.rec.cancel)();
        }
        Response::json(200, &json!({}))
    }

    /// A pending code older than its time is thrown away (checked on every
    /// state poll).
    fn expire_pending_recovery(&self) {
        let mut p = lock(&self.pending_recovery);
        if p.as_ref()
            .is_some_and(|p| p.made.elapsed() > RECOVERY_PENDING_FOR)
        {
            *p = None;
            (self.rec.cancel)();
        }
    }

    /// `POST archive/encrypt` (ADR 0018, T41): an unprotected stick becomes
    /// encrypted with the ALP password (level 2: the console password).
    fn archive_encrypt(self: &Arc<Self>, req: &Request) -> Result<Response, Fail> {
        let b = body(req)?;
        let pw = zeroize::Zeroizing::new(
            field(&b, "password")?
                .as_str()
                .ok_or_else(|| Fail::new("request.json", "\"password\" must be text."))?
                .to_string(),
        );
        // A run that stopped (power cut, a failure) can be started again.
        let retry = index::encrypt_step().is_some() && !index::plain();
        if !index::plain() && !retry {
            return Err(Fail::new(
                "encrypt.not_plain",
                "This ALPArchive is encrypted already.",
            ));
        }
        if archive::record().is_none() {
            return Err(not_open());
        }
        if retry || index::protection() == "console" {
            self.check_password(&pw)?;
        } else {
            index::check_password(&pw)?;
        }
        if Job::running(&lock(&self.archive_job)) {
            return Err(Fail::new("archive.busy", "The archive is busy."));
        }
        let job = self.start_encrypt(pw);
        Ok(Response::json(202, &json!({"job": job})))
    }

    /// The encrypt-later job (ADR 0018); also resumes after a power cut.
    fn start_encrypt(self: &Arc<Self>, pw: zeroize::Zeroizing<String>) -> Value {
        let job = Job::start(
            "archive.encrypt",
            "stopping",
            "Stopping the photo service…",
            Some(0),
        );
        let answer = job.json();
        {
            // Check and take the job slot under one lock (two requests at once).
            let mut slot = lock(&self.archive_job);
            if Job::running(&slot) {
                return slot.as_ref().map_or(Value::Null, Job::json);
            }
            *slot = Some(job);
        }
        index::set_encrypting(true);
        let me = Arc::clone(self);
        std::thread::spawn(move || {
            let res = me.drive("archive.encrypt", |say| {
                let failed = |e: String| job::End::failed(archive::CODE_ENCRYPT, "encrypting", e);
                index::set_encrypt_step(Some("index")).map_err(failed)?;
                systemctl(&["stop", immich::UNIT]);
                systemctl(&["stop", "nginx.service"]);
                say(&job::Update::step("encrypting", "Locking the stick with your password…", 10));
                index::encrypt_in_place(&pw, &mut |m| say(&job::Update::step("encrypting", m, 15)))
                    .map_err(failed)?;
                index::set_encrypt_step(Some("archive")).map_err(failed)?;
                // Phones sign in with the ALP password from now on.
                immich::forget_family_password();
                if let Err(e) = admin::set_password(&pw) {
                    eprintln!("alpd: maintenance login not set: {e}");
                }
                archive::encrypt_archive(&pw, say).map_err(failed)?;
                index::set_encrypt_step(Some("cleanup")).map_err(failed)?;
                say(&job::Update::step("starting", "Starting the photo service on the encrypted archive…", 96));
                systemctl(&["start", immich::UNIT]);
                let t = Instant::now();
                while !immich::ping() && t.elapsed() < Duration::from_secs(15 * 60) {
                    std::thread::sleep(Duration::from_secs(5));
                }
                if !immich::ping() {
                    return Err(failed(
                        "the photo service did not start on the encrypted archive; the unencrypted copy was kept".into(),
                    ));
                }
                say(&job::Update::step("cleanup", "Deleting the unencrypted copy…", 98));
                archive::encrypt_cleanup().map_err(failed)?;
                index::set_encrypt_step(None).map_err(failed)?;
                // The security chip opens it from now on (only now: a cut-off
                // run must ask for the password to resume; storage review M1).
                index::tpm_refresh(&pw);
                say(&job::Update::step("done", "Everything is encrypted now.", 100));
                Ok(())
            });
            index::set_encrypting(false);
            if res.is_ok() {
                me.main_password_typed(&pw);
            } else {
                // Back to a working gallery on whatever archive is in place;
                // nothing is deleted, and it can be started again (M4).
                if let Err(e) = archive::encrypt_rollback() {
                    eprintln!("alpd: encrypt: the archive could not be put back: {e}");
                }
                systemctl(&["start", "--no-block", immich::UNIT]);
            }
            me.finish(&res);
        });
        answer
    }

    /// `POST recovery/check` (T40): do the typed words (and passphrase) open
    /// this archive? Nothing changes; wrong answers count like passwords.
    fn recovery_check(&self, req: &Request) -> Result<Response, Fail> {
        let b = body(req)?;
        let typed = zeroize::Zeroizing::new(
            b.get("recovery_key")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        );
        let passphrase = zeroize::Zeroizing::new(
            b.get("passphrase")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        );
        self.password_wait()?;
        let RecoveryInput::Secret(secret) = recovery_input(&typed, &passphrase)? else {
            return Err(Fail::new(
                "recovery.old_archive",
                "This is an old 64-letter key; it cannot be checked here.",
            ));
        };
        let ok = archive::check_recovery(&secret).map_err(|e| recovery_fail(&e))?;
        if ok {
            *lock(&self.tries) = Tries::default();
        } else {
            lock(&self.tries).failed();
        }
        Ok(Response::json(200, &json!({"opens": ok})))
    }

    fn has_recovery(&self) -> bool {
        self.expire_pending_recovery();
        let mut c = lock(&self.recovery);
        if let Some((t, v)) = *c
            && t.elapsed() < Duration::from_secs(10)
        {
            return v;
        }
        let v = (self.rec.has)();
        *c = Some((Instant::now(), v));
        v
    }

    fn gallery(&self, req: &Request, empty_trash: bool) -> Result<Response, Fail> {
        let b = body(req)?;
        let pw = zeroize::Zeroizing::new(
            field(&b, "password")?
                .as_str()
                .ok_or_else(|| Fail::new("request.json", "\"password\" must be text."))?
                .to_string(),
        );
        let ids: Vec<String> = if empty_trash {
            vec![]
        } else {
            let ids: Vec<String> = field(&b, "ids")?
                .as_array()
                .ok_or_else(|| Fail::new("request.json", "\"ids\" must be a list."))?
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect();
            if ids.is_empty() || ids.len() > 1000 {
                return Err(Fail::new("request.json", "Give 1 to 1000 ids."));
            }
            ids
        };
        self.check_password(&pw)?;
        if !archive::is_open() {
            return Err(not_open());
        }
        lock(&self.immich).at = None;
        if empty_trash {
            immich::empty_trash()?;
            Ok(Response::json(200, &json!({})))
        } else {
            let n = immich::trash_assets(&ids)?;
            Ok(Response::json(200, &json!({"deleted": n})))
        }
    }

    /// Background: the hardware check, the Wi-Fi network every 3 s and the
    /// battery every 30 s. Before
    /// alpha 0.2.2 the check ran before alpd listened, so anything slow in
    /// it left the kiosk on an empty page (T28). Each step is logged with
    /// its time, so a slow one shows in the boot log on the stick.
    pub fn watch_hardware(self: &Arc<Self>) {
        let hardware = Arc::clone(&self.hardware);
        std::thread::spawn(move || {
            let t = Instant::now();
            eprintln!("alpd: hardware check started");
            // A panic in the check must still end `pending`, or the welcome
            // screen waits for it forever.
            let v = std::panic::catch_unwind(hardware_json).unwrap_or_else(|_| {
                eprintln!("alpd: hardware check panicked");
                json!({"machine": "", "verdict": "note", "checks": [], "report_file": null,
                       "specs": null})
            });
            *lock(&hardware) = v;
            eprintln!(
                "alpd: hardware check done in {} ms",
                t.elapsed().as_millis()
            );
        });
        let ssid = Arc::clone(&self.ssid);
        std::thread::spawn(move || {
            loop {
                let v = wifi::device().as_deref().and_then(wifi::connected_ssid);
                *lock(&ssid) = v;
                std::thread::sleep(Duration::from_secs(3));
            }
        });
        let battery = Arc::clone(&self.battery);
        std::thread::spawn(move || {
            let mut first = true;
            loop {
                let t = Instant::now();
                let v = battery_json(Path::new("/sys"));
                if first || t.elapsed() > Duration::from_secs(5) {
                    eprintln!("alpd: battery read in {} ms", t.elapsed().as_millis());
                    first = false;
                }
                *lock(&battery) = v;
                std::thread::sleep(Duration::from_secs(30));
            }
        });
    }

    /// Background: pause Immich when the index or the archive is full,
    /// start it again when there is room (owner: never run full).
    /// The server certificate lives 90 days (ADR 0017): every 5 minutes, while
    /// the index is open, it is renewed when it expires within 30 days or
    /// misses one of this computer's home network addresses (Wi-Fi joined
    /// after the photo service started, a new address from the router), and
    /// what nginx really sends is checked (0.2.5 field report; cheap when
    /// nothing changed). The device allowlist is rewritten every 12 hours.
    /// Idle phone sessions are removed once a day (T63).
    pub fn watch_certificate(self: &Arc<Self>) {
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(60));
            let mut last_map: Option<Instant> = None;
            let mut last_idle: Option<Instant> = None;
            loop {
                // T63: once a day, phone sessions not used for 60 days are
                // signed out, but only while the clock was set from the
                // network: a clock that runs months ahead (0.2.5-test1 saw
                // one hours ahead) would sign every phone out.
                let synced = clock_json()["synced"] == json!(true);
                if index::is_open()
                    && synced
                    && last_idle.is_none_or(|t| t.elapsed() >= Duration::from_secs(24 * 3600))
                    && immich::state() == immich::State::Ready
                {
                    let now = clock_json()["now"].as_u64().unwrap_or(0);
                    match immich::remove_idle_sessions(now) {
                        Ok(n) if n > 0 => eprintln!("alpd: phone: {n} idle device(s) signed out"),
                        Ok(_) => {}
                        Err(e) => eprintln!("alpd: phone: idle devices not checked: {}", e.message),
                    }
                    last_idle = Some(Instant::now());
                }
                if index::is_open() {
                    // Failures are logged by tls::ensure; the phone screen shows them.
                    let _ = immich::certificate_for(&live::addresses(&live::ports()));
                    if last_map.is_none_or(|t| t.elapsed() >= Duration::from_secs(12 * 3600)) {
                        if let Err(e) = pairing::write_map() {
                            eprintln!("alpd: device allowlist: {}", e.message);
                        }
                        last_map = Some(Instant::now());
                    }
                }
                std::thread::sleep(Duration::from_secs(300));
            }
        });
    }

    /// Background: the Immich block for the state, every 2 s.
    pub fn watch_immich(self: &Arc<Self>) {
        let me = Arc::clone(self);
        std::thread::spawn(move || {
            loop {
                me.refresh_immich();
                std::thread::sleep(Duration::from_secs(2));
            }
        });
    }

    pub fn watch_space(self: &Arc<Self>) {
        let me = Arc::clone(self);
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(30));
                if !index::is_open() || !archive::is_open() {
                    continue;
                }
                let full = [index::MNT, archive::FILES_MNT].iter().any(|m| {
                    index::usage(m).is_some_and(|(size, free)| {
                        index::space_level(size, free, me.low_percent) == "full"
                    })
                });
                let mut paused = lock(&me.paused);
                if full && !*paused {
                    eprintln!("alpd: no room left; pausing Immich");
                    systemctl(&["stop", immich::UNIT]);
                    *paused = true;
                } else if !full && *paused {
                    systemctl(&["start", "--no-block", immich::UNIT]);
                    *paused = false;
                }
            }
        });
    }

    /// T54: the archive disk's health (SMART, every 5 minutes) and the
    /// phone-upload pause while the status is critical (review R6, R7: set
    /// by these checks only, cleared only when a later check passes).
    pub fn watch_status(self: &Arc<Self>) {
        let me = Arc::clone(self);
        std::thread::spawn(move || {
            let mut paused_uploads = false;
            let mut n = 0u32;
            loop {
                std::thread::sleep(Duration::from_secs(60));
                if n.is_multiple_of(5) {
                    let h = if archive::is_open() {
                        disks::archive_disk_health()
                    } else {
                        None
                    };
                    let mut cur = lock(&me.archive_health);
                    if *cur != h {
                        eprintln!("alpd: archive disk health: {}", h.unwrap_or("unknown"));
                        *cur = h;
                    }
                }
                n = n.wrapping_add(1);
                let critical = me.critical();
                if critical != paused_uploads {
                    eprintln!(
                        "alpd: status {}: phone uploads {}",
                        if critical {
                            "critical"
                        } else {
                            "no longer critical"
                        },
                        if critical { "paused" } else { "allowed again" }
                    );
                    if uploads_pause(critical) {
                        paused_uploads = critical;
                    }
                }
            }
        });
    }

    /// Is the status critical right now (from alpd's own checks)?
    fn critical(&self) -> bool {
        let v = self.full_state();
        alerts(&v, *lock(&self.archive_health))
            .iter()
            .any(|a| a["level"] == "critical")
    }

    /// Refuse what would change the archive while it is at risk (R6:
    /// formatting another disk, recovery, export, Files and check stay
    /// allowed).
    fn not_critical(&self) -> Result<(), Fail> {
        if self.critical() {
            return Err(Fail::new(
                "archive.critical",
                "The backup is at risk. Fix the problem shown first; until then nothing is deleted or added.",
            ));
        }
        Ok(())
    }

    fn need_open(&self) -> Result<(), Fail> {
        if !self.require_index || index::is_open() {
            Ok(())
        } else {
            Err(locked())
        }
    }

    fn password_setup(&self, req: &Request) -> Result<Response, Fail> {
        let b = body(req)?;
        // One setup at a time, and never while a reset is still erasing the
        // stick: setup may finish a cut-off wipe itself, and two wipes of the
        // same partition destroy the new index (QA review 0.2.5, finding 1).
        if Job::running(&lock(&self.archive_job)) {
            return Err(setup_busy());
        }
        let Ok(_one) = SETUP.try_lock() else {
            return Err(setup_busy());
        };
        // ADR 0016: "none" (no password) and "console" (a password for the
        // console only) leave everything unencrypted; anything else, or no
        // field (older UI), is the encrypted index.
        let protection = b
            .get("protection")
            .and_then(Value::as_str)
            .unwrap_or("password");
        let pw = match b.get("password") {
            Some(Value::String(p)) if protection == "none" && !p.is_empty() => {
                return Err(Fail::new(
                    "request.json",
                    "No password goes with \"protection\": \"none\".",
                ));
            }
            Some(Value::String(p)) if protection != "none" => {
                Some(zeroize::Zeroizing::new(p.clone()))
            }
            None | Some(Value::Null) | Some(Value::String(_)) if protection == "none" => None,
            None => {
                return Err(Fail::new(
                    "request.json",
                    "The request misses \"password\".",
                ));
            }
            Some(_) => return Err(Fail::new("request.json", "\"password\" must be text.")),
        };
        if matches!(protection, "none" | "console") {
            if protection == "console" && pw.is_none() {
                return Err(Fail::new(
                    "request.json",
                    "The request misses \"password\".",
                ));
            }
            let pw = if protection == "none" { None } else { pw };
            index::setup_plain(pw.as_deref().map(|p| p.as_str()), &mut |s| {
                eprintln!("alpd: {s}")
            })?;
            systemctl(&["start", "alp-live-index.service"]);
            match &pw {
                Some(p) => {
                    self.main_password_typed(p);
                    if let Err(e) = admin::set_password(p) {
                        eprintln!("alpd: maintenance login not set: {e}");
                    }
                }
                None => {
                    if let Some(p) = index::phone_password() {
                        self.main_password_typed(&p);
                    }
                }
            }
            return Ok(self.sign_in(Response::json(200, &json!({}))));
        }
        let pw = pw.ok_or_else(|| Fail::new("request.json", "The request misses \"password\"."))?;
        index::check_password(&pw)?;
        index::setup(&pw, &mut |s| eprintln!("alpd: {s}"))?;
        systemctl(&["start", "alp-live-index.service"]);
        self.main_password_typed(&pw);
        if let Err(e) = admin::set_password(&pw) {
            eprintln!("alpd: maintenance login not set: {e}");
        }
        Ok(self.sign_in(Response::json(200, &json!({}))))
    }

    fn unlock(self: &Arc<Self>, req: &Request) -> Result<Response, Fail> {
        let b = body(req)?;
        let pw = zeroize::Zeroizing::new(
            field(&b, "password")?
                .as_str()
                .ok_or_else(|| Fail::new("request.json", "\"password\" must be text."))?
                .to_string(),
        );
        self.password_wait()?;
        if index::is_open() {
            // The chip opened it already: only sign in.
            self.check_password(&pw)?;
            return Ok(self.sign_in(Response::json(200, &json!({}))));
        }
        match index::unlock(&pw) {
            Ok(()) => {
                *lock(&self.tries) = Tries::default();
                systemctl(&["start", "alp-live-index.service"]);
                if index::encrypt_step().is_some() {
                    // Encrypt later was cut off (ADR 0018): it goes on now,
                    // Immich only at its end.
                    self.start_encrypt(zeroize::Zeroizing::new(pw.to_string()));
                } else if archive::record().is_some() {
                    // Opens the archive with the key from the index; Immich
                    // follows (the archive unit wants it; T14).
                    systemctl(&["start", "--no-block", "alp-live-archive.service"]);
                    systemctl(&["start", "--no-block", immich::UNIT]);
                }
                self.main_password_typed(&pw);
                // Seal a new TPM slot for the next start (takes a few seconds).
                std::thread::spawn(move || index::tpm_refresh(&pw));
                Ok(self.sign_in(Response::json(200, &json!({}))))
            }
            Err(e) => {
                if e.key == "password.wrong" {
                    lock(&self.tries).failed();
                }
                Err(e)
            }
        }
    }

    pub fn handle(self: &Arc<Self>, req: &Request) -> Response {
        let Some(route) = req.path.strip_prefix(PREFIX) else {
            return Response::error(404, "request.not_found", "Not found.");
        };
        let get = req.method == "GET" || req.method == "HEAD";
        let post = req.method == "POST";
        // Asked by the gallery proxy, never by a page; it must not count as
        // use of the console (no keep-alive from the gallery frame).
        if route == "gallery/gate" {
            return self.gallery_gate(req, get);
        }
        let free = matches!(
            route,
            "state"
                | "unlock"
                | "password/setup"
                | "console/signin"
                | "console/lock"
                | "power/off"
                // A phone that is pairing (ADR 0017): its token is the credential.
                | "pair/v1/request"
                | "pair/v1/status"
        );
        if !free && !self.console_open(req, true) {
            return err(&Fail::new("console.locked", "Sign in first."));
        }
        let r: Result<Response, Fail> = match route {
            "state" if get => Ok(Response::json(
                200,
                &self.state(self.console_open(req, false)),
            )),
            "console/signin" if post => self.signin(req),
            "console/lock" if post => body(req).map(|_| {
                *lock(&self.console) = Console::default();
                lock(&self.confirms).clear();
                // The Terminal ends with the console (plan rule 5).
                live_terminal::stop();
                // Words shown on the screen are wiped there; the code
                // they belong to must not be confirmed by someone else.
                let _ = self.recovery_cancel();
                Response::json(200, &json!({}))
            }),
            "terminal/start" if post && live_terminal::available() => self.terminal_start(req),
            "terminal/stop" if post && live_terminal::available() => body(req).map(|_| {
                live_terminal::stop();
                Response::json(200, &json!({}))
            }),
            "console/activity" if post => body(req)
                .map(|_| Response::json(200, &json!({"idle_lock_seconds": self.idle_lock().as_secs()}))),
            // T09: setup finished or left; kept on the stick so a restart
            // does not start setup again.
            "setup/finish" if post => body(req).and_then(|_| {
                archive::set_setup_done().map_err(|e| Fail::new("internal", e))?;
                Ok(Response::json(200, &json!({})))
            }),
            "setup/progress" if post => body(req).and_then(|b| setup_progress(&b)),
            "recovery/create" if post => self.need_open().and_then(|()| self.recovery_create(req)),
            "recovery/commit" if post => self.need_open().and_then(|()| self.recovery_commit(req)),
            // `discard` is the Live UI developer's name for it (docs/plans/live-ui-0.2.4.md).
            "recovery/cancel" | "recovery/discard" if post => body(req).map(|_| self.recovery_cancel()),
            "recovery/check" if post => self.need_open().and_then(|()| self.recovery_check(req)),
            "archive/encrypt" if post => self.need_open().and_then(|()| self.archive_encrypt(req)),
            "archive/recover" if post => self.need_open().and_then(|()| self.archive_recover(req)),
            "gallery/delete" if post => self
                .need_open()
                .and_then(|()| self.not_critical())
                .and_then(|()| self.gallery(req, false)),
            "gallery/trash/empty" if post => self
                .need_open()
                .and_then(|()| self.not_critical())
                .and_then(|()| self.gallery(req, true)),
            "wifi/networks" if get => self.networks(),
            "wifi/connect" if post => self.wifi_connect(req),
            "password/setup" if post => self.password_setup(req),
            "unlock" if post => self.unlock(req),
            "archive/candidates" if get => self.need_open().and_then(|()| {
                // Looking mounts the disks read-only: never while one is
                // being written.
                if Job::running(&lock(&self.archive_job)) {
                    return Err(Fail::new("archive.busy", "The archive is being created."));
                }
                Ok(Response::json(200, &candidates()))
            }),
            "archive/create" if post => self.need_open().and_then(|()| self.archive_create(req)),
            "disks" if get => Ok(Response::json(200, &disks::list_json())),
            "events" if get => Ok(Response::json(200, &events::recent_json())),
            "disks/eject" if post => self.disks_eject(req),
            "prefs" if post => self.prefs_set(req),
            "display/modes" if get => self.display_modes(),
            "display/mode" if post => self.display_mode(req),
            "display/mode/keep" if post => self.display_keep(req),
            "display/mode/revert" if post => self.display_revert(req),
            "disks/check" if post => self.disks_check(req),
            "disks/prepare" if post => self.need_open().and_then(|()| self.disks_prepare(req)),
            "disks/format/estimate" if post => {
                self.need_open().and_then(|()| self.disks_format_estimate(req))
            }
            "disks/format" if post => self.need_open().and_then(|()| self.disks_format(req)),
            // Stage F Files (feature media_files): read only (live_media.rs).
            "media/open" if post => body(req).and_then(|b| live_media::open(&b)),
            "media/list" if post => body(req).and_then(|b| live_media::list(&b)),
            // GET only: a HEAD answer would carry the body (review N2).
            "media/file" if req.method == "GET" => live_media::file(req),
            "media/eject" if post => body(req).and_then(|b| live_media::eject(&b)),
            "password/check" if post => password_field(req)
                .and_then(|pw| self.check_password(&pw))
                .map(|()| Response::json(200, &json!({}))),
            "reset" if post => self.reset(req),
            "archive/cancel" if post => self
                .need_open()
                .and_then(|()| body(req))
                .and_then(|_| self.archive_cancel()),
            "phone/pair" if post => self.need_open().and_then(|()| self.pair(req)),
            // Connection check (0.2.5 field report): what a phone meets on
            // each address, live, for the screen and for support.
            "phone/connection" if get => Ok(Response::json(200, &connection_check())),
            "pair/v1/request" if post => body(req).and_then(|b| {
                let (id, code) = lock(&self.pairing).request(&b)?;
                Ok(Response::json(200, &json!({"id": id, "code": code})))
            }),
            "pair/v1/status" if post => body(req).map(|b| {
                let id = b["id"].as_str().unwrap_or("");
                Response::json(200, &lock(&self.pairing).status(id))
            }),
            "pair/open" if post => self.need_open().and_then(|()| body(req)).and_then(|_| {
                let addrs = live::addresses(&live::ports());
                if addrs.is_empty() {
                    return Err(Fail::new(
                        "pair.no_network",
                        "ALPArchive is not on your home network yet. Connect a cable or Wi-Fi first.",
                    ));
                }
                // Never a QR the phone cannot use (0.2.5 field report): nginx
                // must already send a certificate naming every address in it.
                immich::certificate_for(&addrs).map_err(|e| {
                    Fail::new("pair.certificate_not_served", e.message)
                })?;
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs());
                let uri = lock(&self.pairing).open(&addrs, "ALPArchive", now)?;
                Ok(Response::json(200, &json!({
                    "uri": uri, "qr_svg": qr::svg(&uri),
                    "expires_in": pairing::WINDOW.as_secs(),
                })))
            }),
            "pair/close" if post => body(req).map(|_| {
                lock(&self.pairing).close();
                Response::json(200, &json!({}))
            }),
            "pair/approve" if post => self.need_open().and_then(|()| body(req)).and_then(|b| {
                let id = field(&b, "id")?.as_str().unwrap_or_default().to_string();
                let d = lock(&self.pairing).approve(&id, &live::now_rfc3339())?;
                systemctl(&["try-reload-or-restart", "nginx.service"]);
                Ok(Response::json(200, &json!({"device": d.json()})))
            }),
            "pair/reject" if post => body(req).and_then(|b| {
                let id = field(&b, "id")?.as_str().unwrap_or_default().to_string();
                lock(&self.pairing).reject(&id)?;
                Ok(Response::json(200, &json!({})))
            }),
            "pair/revoke" if post => self.need_open().and_then(|()| body(req)).and_then(|b| {
                let serial = field(&b, "serial")?.as_str().unwrap_or_default().to_string();
                pairing::revoke(&serial, &live::now_rfc3339())?;
                Ok(Response::json(200, &json!({})))
            }),
            "pair/strict" if post => self.need_open().and_then(|()| body(req)).and_then(|b| {
                pairing::set_strict(b["on"].as_bool().unwrap_or(false))?;
                Ok(Response::json(200, &json!({})))
            }),
            "phone/devices/remove" if post => {
                self.need_open().and_then(|()| body(req)).and_then(|b| {
                    let id = field(&b, "id")?.as_str().unwrap_or_default().to_string();
                    immich::remove_device(&id)?;
                    lock(&self.immich).at = None;
                    Ok(Response::json(200, &json!({})))
                })
            }
            "phone/reset" if post => Err(Fail::new(
                "phone.reset_gone",
                "Phones now use your ALPArchive password.",
            )),
            "archive/forget" if post => self.need_open().and_then(|()| body(req)).and_then(|_| {
                if Job::running(&lock(&self.archive_job)) {
                    return Err(Fail::new("archive.busy", "The archive is being created."));
                }
                archive::forget(&live::now_rfc3339()).map_err(|e| {
                    Fail::new(
                        "archive.forget",
                        format!("ALPArchive cannot choose again now: {e}."),
                    )
                })?;
                *lock(&self.archive_job) = None;
                Ok(Response::json(200, &json!({})))
            }),
            "chain/create" if post => self
                .need_open()
                .and_then(|()| body(req).and_then(|_| self.chain_create())),
            "chain/code" if get => self
                .need_open()
                .and_then(|()| self.chain.code())
                .map(|c| Response::json(200, &code_json(&c))),
            "chain/join" if post => self.need_open().and_then(|()| self.chain_join(req)),
            "chain/leave" if post => self
                .need_open()
                .and_then(|()| body(req))
                .and_then(|_| self.chain.leave())
                .map(|()| Response::json(200, &json!({}))),
            "power/off" if post && index::encrypting() => Err(Fail::new(
                "encrypt.running",
                "ALPArchive is being encrypted. Switch off when it is done, or the next start goes on with it.",
            )),
            "power/off" if post => self.power_off(req),
            "state"
            | "password/setup"
            | "unlock"
            | "wifi/networks"
            | "wifi/connect"
            | "archive/candidates"
            | "archive/create"
            | "disks"
            | "events"
            | "disks/eject"
            | "prefs"
            | "display/modes"
            | "display/mode"
            | "display/mode/keep"
            | "display/mode/revert"
            | "disks/check"
            | "disks/prepare"
            | "disks/format/estimate"
            | "disks/format"
            | "media/open"
            | "media/list"
            | "media/file"
            | "media/eject"
            | "archive/cancel"
            | "password/check"
            | "reset"
            | "phone/pair"
            | "phone/connection"
            | "phone/devices/remove"
            | "pair/v1/request"
            | "pair/v1/status"
            | "pair/open"
            | "pair/close"
            | "pair/approve"
            | "pair/reject"
            | "pair/revoke"
            | "pair/strict"
            | "phone/reset"
            | "archive/forget"
            | "chain/create"
            | "chain/code"
            | "chain/join"
            | "chain/leave"
            | "power/off"
            | "console/signin"
            | "console/lock"
            | "console/activity"
            | "setup/finish"
            | "setup/progress"
            | "recovery/create"
            | "recovery/commit"
            | "recovery/cancel"
            | "recovery/discard"
            | "recovery/check"
            | "archive/encrypt"
            | "archive/recover"
            | "gallery/delete"
            | "gallery/trash/empty" => {
                return Response::error(405, "request.method", "Not allowed here.");
            }
            "terminal/start" | "terminal/stop" if live_terminal::available() => {
                return Response::error(405, "request.method", "Not allowed here.");
            }
            _ => return Response::error(404, "request.not_found", "Not found."),
        };
        r.unwrap_or_else(|f| err(&f))
    }

    fn state(&self, open: bool) -> Value {
        let mut v = self.full_state();
        let list = alerts(&v, *lock(&self.archive_health));
        v["status"] = status_of(&list);
        v["alerts"] = Value::Array(list);
        if !open {
            locked_view(&mut v);
        }
        // External disks for Files (stage F): only for a signed-in console.
        v["media"] = if open { live_media::state() } else { json!([]) };
        v
    }

    /// `POST prefs` (T56): any subset of the known settings; answers the
    /// whole set. Kept on the stick, so it needs the stick's own storage.
    fn prefs_set(&self, req: &Request) -> Result<Response, Fail> {
        let b = body(req)?;
        let mut p = lock(&self.prefs);
        let mut next = p.clone();
        next.apply(&b).map_err(|m| Fail::new("prefs.invalid", m))?;
        if self.require_index {
            archive::state_ready().map_err(|e| {
                Fail::new(
                    "state.not_ready",
                    format!("The setting was not saved: {e}."),
                )
            })?;
        }
        next.save_to(&self.prefs_file).map_err(|e| {
            eprintln!("alpd: prefs not saved: {e}");
            Fail::new("internal", "The setting was not saved. Please try again.")
        })?;
        *p = next;
        Ok(Response::json(200, &json!({"prefs": p.json()})))
    }

    /// The first screen as `wlr-randr` lists it.
    fn display_output(&self) -> Result<display::Output, Fail> {
        let unavailable = |e: String| {
            eprintln!("alpd: screen modes: {e}");
            Fail::new(
                "display.unavailable",
                "The screen settings cannot be read right now. Please try again in a moment.",
            )
        };
        let text = (self.randr)(&[]).map_err(unavailable)?;
        display::parse_outputs(&text)
            .map_err(|e| unavailable(e.to_string()))?
            .into_iter()
            .next()
            .ok_or_else(|| unavailable("no screen listed".into()))
    }

    fn display_set(&self, output: &str, m: prefs::Mode) -> Result<(), Fail> {
        (self.randr)(&display::set_args(output, m))
            .map(|_| ())
            .map_err(|e| {
                eprintln!("alpd: screen mode not set: {e}");
                Fail::new("display.failed", "The screen did not take this setting.")
            })
    }

    /// `GET display/modes`.
    fn display_modes(&self) -> Result<Response, Fail> {
        let o = self.display_output()?;
        Ok(Response::json(200, &display::modes_json(&o)))
    }

    /// `POST display/mode {id}`: switch, and switch back by itself after
    /// [`Live::revert_after`] unless `display/mode/keep` comes first. The
    /// way back is written to a marker first, so an alpd restart in the
    /// window still goes back (review N-T56-1).
    fn display_mode(self: &Arc<Self>, req: &Request) -> Result<Response, Fail> {
        let b = body(req)?;
        let id = field(&b, "id")?
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| Fail::new("request.json", "\"id\" must be a number from the list."))?;
        let mut pending = lock(&self.display_pending);
        if pending.is_some() {
            return Err(Fail::new(
                "display.busy",
                "Another screen setting is waiting for your answer.",
            ));
        }
        let o = self.display_output()?;
        let new = o.modes.get(id).ok_or_else(|| {
            Fail::new(
                "display.not_found",
                "This screen setting is no longer offered.",
            )
        })?;
        let back = o
            .modes
            .iter()
            .find(|m| m.current)
            .or_else(|| o.modes.iter().find(|m| m.preferred))
            .ok_or_else(|| {
                Fail::new(
                    "display.unavailable",
                    "The screen's current setting is unknown.",
                )
            })?
            .mode;
        if self.require_index {
            std::fs::write(display::PENDING, display::pending_line(&o.name, back)).map_err(
                |e| {
                    eprintln!("alpd: screen pending marker: {e}");
                    Fail::new(
                        "internal",
                        "The screen setting was not changed. Please try again.",
                    )
                },
            )?;
        }
        if let Err(e) = self.display_set(&o.name, new.mode) {
            if self.require_index {
                let _ = std::fs::remove_file(display::PENDING);
            }
            return Err(e);
        }
        let serial = DISPLAY_SERIAL.fetch_add(1, Ordering::SeqCst);
        *pending = Some(DisplayPending {
            serial,
            output: o.name,
            back,
            new: new.mode,
        });
        drop(pending);
        let me = Arc::clone(self);
        std::thread::spawn(move || {
            std::thread::sleep(me.revert_after);
            let mut p = lock(&me.display_pending);
            if let Some(d) = p.as_ref().filter(|d| d.serial == serial) {
                eprintln!("alpd: screen setting not confirmed; going back");
                let back = me.display_set(&d.output, d.back).is_ok();
                *p = None;
                // On a failed set the marker stays: the watcher tries the
                // way back again every few seconds.
                if back && me.require_index {
                    let _ = std::fs::remove_file(display::PENDING);
                }
            }
        });
        Ok(Response::json(
            200,
            &json!({"revert_in": self.revert_after.as_secs()}),
        ))
    }

    /// `POST display/mode/keep`: keep the new mode, also after a restart,
    /// on this same screen only (review N-T56-2).
    fn display_keep(&self, req: &Request) -> Result<Response, Fail> {
        body(req)?;
        let mut pending = lock(&self.display_pending);
        let d = pending.take().ok_or_else(|| {
            Fail::new("display.no_pending", "There is no screen setting waiting.")
        })?;
        if self.require_index {
            let _ = std::fs::remove_file(display::PENDING);
        }
        let screen = display::screen().and_then(|s| s.edid).map(|e| e.identity);
        let mut p = lock(&self.prefs);
        let mut next = p.clone();
        next.mode = screen.map(|s| (d.new, s));
        if next != *p {
            if let Err(e) = next.save_to(&self.prefs_file) {
                // The mode stays for now; it is only not kept for next time.
                eprintln!("alpd: kept screen mode not saved: {e}");
            }
            *p = next;
        }
        Ok(Response::json(200, &json!({})))
    }

    /// `POST display/mode/revert`: go back at once instead of waiting for
    /// the timer (the screen's Revert button).
    fn display_revert(&self, req: &Request) -> Result<Response, Fail> {
        body(req)?;
        let mut pending = lock(&self.display_pending);
        let d = pending.clone().ok_or_else(|| {
            Fail::new("display.no_pending", "There is no screen setting waiting.")
        })?;
        // The way back stays armed (timer and marker) until the screen
        // took the old mode: a failed set must not leave it black.
        self.display_set(&d.output, d.back)?;
        *pending = None;
        if self.require_index {
            let _ = std::fs::remove_file(display::PENDING);
        }
        Ok(Response::json(200, &json!({})))
    }

    /// In the background: go back from an unconfirmed mode change after an
    /// alpd restart, and set the kept mode each time the kiosk (cage)
    /// starts, when the same screen is connected.
    pub fn watch_display(self: &Arc<Self>) {
        if !display::modes_supported() {
            return;
        }
        let me = Arc::clone(self);
        std::thread::spawn(move || {
            let mut seen: Option<(u64, i64)> = None;
            loop {
                std::thread::sleep(Duration::from_secs(5));
                let Some((_, _, stamp)) = display::socket() else {
                    continue;
                };
                if lock(&me.display_pending).is_some() {
                    continue;
                }
                if let Some((output, back)) = std::fs::read_to_string(display::PENDING)
                    .ok()
                    .and_then(|t| display::parse_pending(&t))
                {
                    eprintln!("alpd: an unconfirmed screen setting was left; going back");
                    if me.display_set(&output, back).is_ok() {
                        let _ = std::fs::remove_file(display::PENDING);
                    }
                }
                if seen == Some(stamp) {
                    continue;
                }
                seen = Some(stamp);
                let kept = lock(&me.prefs).mode.clone();
                let Some((mode, screen)) = kept else { continue };
                let same = display::screen()
                    .and_then(|s| s.edid)
                    .is_some_and(|e| e.identity == screen);
                if !same {
                    continue;
                }
                if let Ok(o) = me.display_output()
                    && o.modes.iter().any(|m| m.mode == mode && !m.current)
                {
                    let _ = me.display_set(&o.name, mode);
                }
            }
        });
    }

    /// `pairing` (ADR 0017): the open window, phones waiting for approval,
    /// paired devices and the mode.
    fn pairing_json(&self) -> Value {
        let mut p = lock(&self.pairing);
        let open = index::is_open();
        json!({
            "window_seconds": p.window_left(),
            "pending": p.pending().iter().map(|r| json!({
                "id": r.id, "name": r.name, "model": r.model, "os": r.os, "code": r.code,
            })).collect::<Vec<_>>(),
            "devices": if open { pairing::devices().iter().map(pairing::Device::json).collect() } else { vec![] },
            "strict": open && pairing::strict(),
        })
    }

    fn full_state(&self) -> Value {
        let ports = live::ports();
        let dev = wifi::device();
        let ssid = lock(&self.ssid).clone();
        let ports_json: Vec<Value> = ports
            .iter()
            .map(|p| {
                json!({
                    "name": p.name,
                    "kind": if p.wifi { "wifi" } else { "wired" },
                    "state": match (&p.address, p.carrier) {
                        (Some(_), _) => "connected",
                        (None, true) => "waiting",
                        (None, false) => "off",
                    },
                    "address": p.address,
                    "ssid": if p.wifi { ssid.clone() } else { None },
                })
            })
            .collect();
        let address = live::main_address(&ports);
        let wifi_job = lock(&self.wifi_job).as_ref().map(Job::json);
        let (immich_state, phone) = self.immich_info(address.as_deref());
        json!({
            "api": 1,
            "hardware": lock(&self.hardware).clone(),
            "network": {
                "online": address.is_some(),
                "address": address,
                "ports": ports_json,
                "wifi_present": dev.is_some(),
                "addresses": addresses_json(&ports),
                "job": wifi_job,
            },
            "system": {"battery": lock(&self.battery).clone(), "release": release(), "build": build()},
            "pairing": self.pairing_json(),
            "power": match lock(&self.power_job).as_ref() {
                Some(j) => json!({"state": "stopping", "job": j.json()}),
                None => json!({"state": "on", "job": null}),
            },
            "lock": {"state": index::lock_state(), "retry_after_seconds": lock(&self.tries).wait(),
                     "tpm": index::tpm_state(),
                     "protection": if index::lock_state() == "setup" { Value::Null } else { index::protection().into() }},
            "console": {"state": "open", "idle_lock_seconds": self.idle_lock().as_secs()},
            "setup": setup_json(),
            // What this alpd can do, so the console (Live UI 0.2.4) shows
            // only the flows it has: two-step recovery code (T42).
            "features": features(),
            "prefs": lock(&self.prefs).json(),
            "clock": clock_json(),
            "display": display::state_json(),
            // Never the session's address (plan, build note 3).
            "terminal": live_terminal::state_json(),
            "index": self.space_json(index::MNT, index::is_open()),
            "archive": self.archive_state(),
            "immich": if *lock(&self.paused) {
                json!({"state": "paused", "error": fail_json(&Fail::new("space.full",
                    "There is no room left, so new photos are not saved for now. Make room or use a bigger disk."))})
            } else { immich_state },
            "phone": phone,
            "chain": match self.chain.get() {
                Some(c) => json!({"state": "member", "chain_id": c.chain_id, "role": c.role, "since": c.since}),
                None => json!({"state": "none", "chain_id": null, "role": null, "since": null}),
            },
        })
    }

    fn space_json(&self, at: &str, open: bool) -> Value {
        match index::usage(at).filter(|_| open) {
            Some((size, free)) => json!({"size_bytes": size, "free_bytes": free,
                "space": index::space_level(size, free, self.low_percent)}),
            None => json!({"size_bytes": 0, "free_bytes": 0, "space": "ok"}),
        }
    }

    fn archive_state(&self) -> Value {
        let job = lock(&self.archive_job).clone();
        let none_json = |state: &str, error: Option<Value>| {
            json!({"state": state, "place": null, "size_bytes": 0, "free_bytes": 0, "space": "ok",
                   "job": job.as_ref().map(Job::json), "error": error, "recovery": "none",
                   "serious": index::is_open() && (self.serious)()})
        };
        if Job::running(&job) {
            // Preparing a disk holds the shared job slot; no archive yet.
            let prep = job.as_ref().is_some_and(|j| j.id == "disk.prepare");
            return none_json(if prep { "none" } else { "creating" }, None);
        }
        if !index::is_open() {
            return none_json(
                if archive::record().is_some() {
                    "locked"
                } else {
                    "none"
                },
                None,
            );
        }
        match archive::status() {
            archive::Status::Open {
                place,
                size_bytes,
                free_bytes,
            } => json!({
                "state": "open",
                "place": place.map(|p| json!({"partuuid": p.partuuid, "name": p.name, "label": p.label})),
                "size_bytes": size_bytes, "free_bytes": free_bytes,
                "space": index::space_level(size_bytes, free_bytes, self.low_percent),
                "job": job.as_ref().map(Job::json), "error": null,
                "recovery": if self.has_recovery() { "set" } else { "none" },
                "password_reopen": archive::password_reopen(),
                "serious": (self.serious)(),
            }),
            archive::Status::Missing(why) => none_json(
                "missing",
                Some(fail_json(&Fail::new("archive.missing", why))),
            ),
            archive::Status::None => match job.as_ref().and_then(|j| j.error.clone()) {
                Some(e) => none_json("failed", Some(fail_json(&e))),
                None => none_json("none", None),
            },
        }
    }

    /// Refresh the Immich block (slow calls to Immich and systemd) without
    /// holding the cache while asking; at most every [`IMMICH_CACHE`].
    fn refresh_immich(&self) {
        // Work on a copy: the state answer reads the cache meanwhile.
        let mut c = {
            let g = lock(&self.immich);
            if g.at.is_some_and(|t| t.elapsed() <= IMMICH_CACHE) {
                return;
            }
            g.clone()
        };
        let st = immich::state();
        if st == immich::State::Starting {
            let since = *c.starting_since.get_or_insert_with(Instant::now);
            // Nobody started it (e.g. alpd restarted): start it, at
            // most once a minute, never while paused for space.
            if self.require_index
                && since.elapsed() > Duration::from_secs(20)
                && c.kicked
                    .is_none_or(|k| k.elapsed() > Duration::from_secs(60))
                && !*lock(&self.paused)
                && lock(&self.power_job).is_none()
                && immich::unit_inactive()
            {
                eprintln!("alpd: the photo service was not running; starting it");
                systemctl(&["start", "--no-block", immich::UNIT]);
                c.kicked = Some(Instant::now());
            }
        } else {
            c.starting_since = None;
        }
        let secs = c.starting_since.map(|t| t.elapsed().as_secs());
        let (word, error) = match &st {
            immich::State::Off => ("off", Value::Null),
            immich::State::Starting => ("starting", Value::Null),
            immich::State::Ready => ("ready", Value::Null),
            immich::State::Failed(m) => {
                ("failed", fail_json(&Fail::new("immich.failed", m.clone())))
            }
        };
        c.state = json!({
            "state": word, "error": error,
            "detail": secs.map(immich::detail),
            "seconds_starting": secs,
            "console_url": (st == immich::State::Ready).then_some(immich::CONSOLE_URL),
            // Feature gallery_session: set once the gallery proxy can answer.
            "gallery_url": (st == immich::State::Ready && immich::family_key().is_some())
                .then_some(immich::GALLERY_URL),
        });
        let fam = if st == immich::State::Ready {
            immich::family()
        } else {
            None
        };
        let devices: Vec<Value> = fam.as_ref().map_or(vec![], |f| {
            f.devices
                .iter()
                .map(|d| {
                    json!({
                        "id": d.id, "name": d.name, "os": d.os, "kind": d.kind,
                        "app_version": d.app_version, "last_seen": d.last_seen,
                    })
                })
                .collect()
        });
        c.phone = json!({
            "account": fam.as_ref().map(|_| immich::FAMILY_EMAIL),
            "paired": !devices.is_empty(),
            "phones": devices.len(),
            "photos": fam.as_ref().map_or(0, |f| f.photos),
            "last_upload": null,
            "devices": devices,
            "connected_now": if st == immich::State::Ready { immich::clients_now() } else { vec![] },
        });
        c.at = Some(Instant::now());
        *lock(&self.immich) = c;
    }

    /// Immich state and the phone block, as the background refresh left
    /// them ([`Live::watch_immich`]): the state never waits for Immich (an
    /// Immich that is starting can hold a request for its 30 s timeout; CI
    /// run 41011716279, T-LIVE-02 TPM case).
    fn immich_info(&self, address: Option<&str>) -> (Value, Value) {
        let c = lock(&self.immich);
        let mut phone = c.phone.clone();
        phone["server_url"] = if phone["account"].is_null() {
            Value::Null
        } else {
            address.map(server_url).into()
        };
        (c.state.clone(), phone)
    }

    fn networks(&self) -> Result<Response, Fail> {
        let dev = wifi::device().ok_or_else(no_wifi)?;
        let nets = wifi::scan(&dev);
        *lock(&self.networks) = nets.clone();
        Ok(Response::json(
            200,
            &json!({"networks": nets.iter().map(|n| json!({
                "ssid": n.ssid, "security": n.security, "signal": n.signal, "connected": n.connected
            })).collect::<Vec<_>>()}),
        ))
    }

    fn wifi_connect(self: &Arc<Self>, req: &Request) -> Result<Response, Fail> {
        let b = body(req)?;
        let ssid = field(&b, "ssid")?
            .as_str()
            .ok_or_else(|| Fail::new("request.json", "\"ssid\" must be text."))?
            .to_string();
        let password = zeroize::Zeroizing::new(
            b.get("password")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        );
        let dev = wifi::device().ok_or_else(no_wifi)?;
        let mut net = lock(&self.networks)
            .iter()
            .find(|n| n.ssid == ssid)
            .cloned();
        if net.is_none() {
            let nets = wifi::scan(&dev);
            net = nets.iter().find(|n| n.ssid == ssid).cloned();
            *lock(&self.networks) = nets;
        }
        let net = net.ok_or_else(|| {
            Fail::new(
                "wifi.not_found",
                "This network is not in range any more. Please look again.",
            )
        })?;
        wifi::check_request(&net.security, &password)?;
        let job = {
            let mut j = lock(&self.wifi_job);
            if Job::running(&j) {
                return Err(Fail::new(
                    "wifi.busy",
                    "ALPArchive is already joining a network.",
                ));
            }
            let job = Job::start(
                "wifi.connect",
                "joining",
                &format!("Connecting to {}…", net.ssid),
                None,
            );
            *j = Some(job.clone());
            job
        };
        let me = Arc::clone(self);
        std::thread::spawn(move || {
            let set = |f: &dyn Fn(&mut Job)| {
                if let Some(j) = lock(&me.wifi_job).as_mut() {
                    f(j);
                }
            };
            if let Err(e) = wifi::connect(&dev, &net, &password) {
                set(&|j| {
                    j.state = "failed";
                    j.message = e.message.clone();
                    j.error = Some(e.clone());
                });
                return;
            }
            set(&|j| {
                j.step = "address".into();
                j.message = "Connected. Waiting for an address from the router…".into();
            });
            for _ in 0..40 {
                if live::ports()
                    .iter()
                    .any(|p| p.name == dev && p.address.is_some())
                {
                    set(&|j| {
                        j.state = "done";
                        j.step = "done".into();
                        j.message = "Connected. ALPArchive is on your home network.".into();
                    });
                    return;
                }
                std::thread::sleep(Duration::from_secs(1));
            }
            let e = Fail::new(
                "wifi.no_address",
                "Connected, but the router did not give an address. Waiting again may help.",
            );
            set(&|j| {
                j.state = "failed";
                j.message = e.message.clone();
                j.error = Some(e.clone());
            });
        });
        Ok(Response::json(202, &json!({"job": job.json()})))
    }

    fn archive_create(self: &Arc<Self>, req: &Request) -> Result<Response, Fail> {
        let b = body(req)?;
        let partuuid = field(&b, "partuuid")?
            .as_str()
            .ok_or_else(|| Fail::new("request.json", "\"partuuid\" must be text."))?
            .to_string();
        // `size_bytes` may still be sent by old clients; it is ignored: the
        // archive grows into the disk (ADR 0014).
        let serious = b.get("serious").and_then(Value::as_bool).unwrap_or(false);
        let pw = if serious {
            Some(zeroize::Zeroizing::new(
                field(&b, "password")?
                    .as_str()
                    .ok_or_else(|| Fail::new("request.json", "\"password\" must be text."))?
                    .to_string(),
            ))
        } else {
            None
        };
        // Level 3 (ADR 0016): the ALP password typed at setup, so the new
        // archive can be reopened with it from another stick right away.
        let alp_pw = match b.get("alp_password").and_then(Value::as_str) {
            Some(p) if !p.is_empty() && !serious => {
                self.check_password(p)?;
                Some(zeroize::Zeroizing::new(p.to_string()))
            }
            _ => None,
        };
        if archive::record().is_some() {
            return Err(Fail::new(
                "archive.exists",
                "An archive is already set up with this stick.",
            ));
        }
        if Job::running(&lock(&self.archive_job)) {
            return Err(Fail::new(
                "archive.busy",
                "The archive is being created already.",
            ));
        }
        if let Some(pw) = &pw {
            self.check_password(pw)?;
        }
        let p = archive::partitions()
            .into_iter()
            .find(|p| p.partuuid == partuuid)
            .ok_or_else(|| {
                Fail::new(
                    "archive.not_found",
                    "This place is not on this computer any more.",
                )
            })?;
        if !p.external {
            return Err(disk_locked());
        }
        if let Some(why) = archive::refusal(&p, None) {
            return Err(Fail::new("archive.refused", why.reason).with_code(why.code));
        }
        let pr = archive::probe(&p).map_err(|e| {
            Fail::new(
                "archive.refused",
                format!("ALPArchive could not read this disk ({e}). Unplug it, plug it in again and retry."),
            )
            .with_code(archive::CODE_UNREADABLE)
        })?;
        if let Some(why) = archive::refusal(&p, Some(&pr)) {
            return Err(Fail::new("archive.refused", why.reason).with_code(why.code));
        }
        if pw.is_some() && index::plain() {
            return Err(Fail::new(
                "archive.serious_plain",
                "This ALPArchive was set up without encryption, so strong protection cannot be turned on. Start over to encrypt it.",
            ));
        }
        if let Some(pw) = &pw {
            // Serious encryption: auto-unlock off before anything is made.
            index::set_serious(pw)?;
        }
        let job = Job::start(
            "archive.create",
            "preparing",
            "Preparing the disk…",
            Some(0),
        );
        {
            let mut j = lock(&self.archive_job);
            if Job::running(&j) {
                return Err(Fail::new(
                    "archive.busy",
                    "The archive is being created already.",
                ));
            }
            *j = Some(job.clone());
        }
        let me = Arc::clone(self);
        let cancel = Arc::clone(&job.cancel);
        std::thread::spawn(move || {
            let res = me.drive("archive.create", |say| {
                archive::create(&partuuid, &cancel, say)
            });
            if res.is_ok() {
                if let Some(p) = &alp_pw
                    && let Err(e) = archive::bind_password(p)
                {
                    eprintln!("alpd: password reopening not set: {e}");
                }
                // The open-at-start unit and Immich follow the new archive.
                systemctl(&["start", "alp-live-archive.service"]);
                systemctl(&["start", "--no-block", immich::UNIT]);
            }
            me.finish(&res);
        });
        Ok(Response::json(202, &json!({"job": job.json()})))
    }

    /// `POST disks/prepare` (ADR 0014): erase a whole external disk and
    /// format it as exFAT, only with the admin password, the disk as
    /// plugged in now (`disk_id`) and exactly the partitions the user saw.
    /// Refuse a disk id that names the stick or an internal disk, whatever
    /// the screen sent: the class is looked up again here (ADR 0022). An
    /// unknown id is left to the caller's own "not plugged in" answer.
    /// (Unit tests run without real disks: `require_index` off.)
    /// What a format confirmation is bound to: the disk as it is now
    /// (unit tests have no real disks: the id alone).
    fn format_binding(&self, disk_id: &str) -> String {
        match archive::disk_of_id(disk_id) {
            Some(name) if self.require_index => format::binding(disk_id, name),
            _ => disk_id.to_string(),
        }
    }

    fn external_disk(&self, disk_id: &str) -> Result<(), Fail> {
        match archive::disk_of_id(disk_id) {
            Some(name) if self.require_index && !disks::is_external(name) => Err(disk_locked()),
            _ => Ok(()),
        }
    }

    /// `POST disks/eject` (T58): sync and switch off an external disk that
    /// is not open, so it can be pulled out.
    fn disks_eject(&self, req: &Request) -> Result<Response, Fail> {
        let b = body(req)?;
        let disk_id = text_field(&b, "disk_id")?;
        self.external_disk(&disk_id)?;
        if Job::running(&lock(&self.archive_job)) {
            return Err(busy());
        }
        if !self.require_index {
            return Ok(Response::json(200, &json!({})));
        }
        let (model, size_bytes) = disk_facts(&disk_id);
        disks::eject(&disk_id).map_err(|why| match why {
            "not_found" => format_refusal(archive::PrepareRefusal::NotFound),
            "locked" => disk_locked(),
            "open" => Fail::new("disk.open", "This disk is open. Close it in Files first, then eject it."),
            _ => Fail::new("disk.eject_failed", "The disk could not be switched off. Wait a moment and try again, or switch off the computer before unplugging it."),
        })?;
        events::record(events::Event {
            at: events::now(),
            kind: "disk.eject",
            disk: model,
            size_bytes,
            mode: None,
            fs: None,
            result: "done",
            code: None,
        });
        Ok(Response::json(200, &json!({})))
    }

    /// `POST disks/check` (T58): the job `disk.check`, a read-only check of
    /// an external disk's file systems in a sandbox (review R8).
    fn disks_check(self: &Arc<Self>, req: &Request) -> Result<Response, Fail> {
        let b = body(req)?;
        let disk_id = text_field(&b, "disk_id")?;
        self.external_disk(&disk_id)?;
        let job = Job::start("disk.check", "checking", "Checking the disk…", Some(0));
        {
            let mut j = lock(&self.archive_job);
            if Job::running(&j) {
                return Err(busy());
            }
            *j = Some(job.clone());
        }
        let (model, size_bytes) = disk_facts(&disk_id);
        let event = move |result, code: Option<String>| {
            events::record(events::Event {
                at: events::now(),
                kind: "disk.check",
                disk: model.clone(),
                size_bytes,
                mode: None,
                fs: None,
                result,
                code,
            });
        };
        event("started", None);
        let me = Arc::clone(self);
        let cancel = Arc::clone(&job.cancel);
        std::thread::spawn(move || {
            let mut words = String::new();
            let res = me.drive("disk.check", |say| {
                words = (me.check)(&disk_id, &cancel, say)?;
                Ok(())
            });
            if res.is_ok()
                && let Some(j) = lock(&me.archive_job).as_mut()
            {
                j.message = words;
            }
            match &res {
                Ok(()) => event("done", None),
                Err(job::End::Cancelled) => event("stopped", None),
                Err(job::End::Failed { code, .. }) => event("failed", Some(code.to_string())),
            }
            me.finish(&res);
        });
        Ok(Response::json(202, &json!({"job": job.json()})))
    }

    fn disks_prepare(self: &Arc<Self>, req: &Request) -> Result<Response, Fail> {
        let b = body(req)?;
        let disk_id = field(&b, "disk_id")?
            .as_str()
            .ok_or_else(|| Fail::new("request.json", "\"disk_id\" must be text."))?
            .to_string();
        let shown: Vec<String> = field(&b, "partuuids")?
            .as_array()
            .and_then(|a| a.iter().map(|v| v.as_str().map(str::to_string)).collect())
            .ok_or_else(|| Fail::new("request.json", "\"partuuids\" must be a list of text."))?;
        self.external_disk(&disk_id)?;
        let pw = zeroize::Zeroizing::new(
            field(&b, "password")?
                .as_str()
                .ok_or_else(|| Fail::new("request.json", "\"password\" must be text."))?
                .to_string(),
        );
        self.check_password(&pw)?;
        if archive::record().is_some() {
            return Err(Fail::new(
                "archive.exists",
                "An archive is already set up with this stick.",
            ));
        }
        if Job::running(&lock(&self.archive_job)) {
            return Err(Fail::new(
                "archive.busy",
                "Something is being done with a disk; wait until it is finished.",
            ));
        }
        archive::prepare_check(&disk_id, &shown).map_err(|r| match r {
            archive::PrepareRefusal::NotFound => Fail::new(
                "disk.not_found",
                "This disk is not plugged in any more. Look at the list of disks again.",
            ),
            archive::PrepareRefusal::Changed => Fail::new(
                "disk.changed",
                "What is on this disk changed. Look at the list of disks again.",
            ),
            archive::PrepareRefusal::Refused(m) => {
                Fail::new("disk.refused", m).with_code(archive::CODE_NO_PREPARE)
            }
        })?;
        let job = Job::start("disk.prepare", "checking", "Checking the disk…", Some(0));
        {
            let mut j = lock(&self.archive_job);
            if Job::running(&j) {
                return Err(Fail::new(
                    "archive.busy",
                    "Something is being done with a disk; wait until it is finished.",
                ));
            }
            *j = Some(job.clone());
        }
        let me = Arc::clone(self);
        std::thread::spawn(move || {
            let res = me.drive("disk.prepare", |say| {
                archive::prepare_disk(&disk_id, &shown, say)
            });
            me.finish(&res);
        });
        Ok(Response::json(202, &json!({"job": job.json()})))
    }

    /// `POST disks/format/estimate` (T49): the explanation of both modes and
    /// the minutes a secure format takes, for the disk as plugged in now.
    /// Nothing is written (the speed comes from a short read).
    fn disks_format_estimate(&self, req: &Request) -> Result<Response, Fail> {
        let b = body(req)?;
        let (disk_id, shown) = disk_and_parts(&b)?;
        self.external_disk(&disk_id)?;
        if Job::running(&lock(&self.archive_job)) {
            return Err(busy());
        }
        let e = format::estimate(&disk_id, &shown).map_err(format_refusal)?;
        let confirm = lock(&self.confirms)
            .issue(&disk_id, self.format_binding(&disk_id))
            .ok_or_else(|| {
                Fail::new("internal", "The disk could not be prepared for formatting.")
            })?;
        Ok(Response::json(
            200,
            &json!({
                "size_bytes": e.size_bytes,
                "bytes_used": e.bytes_used,
                "files": Value::Null,
                "confirm": confirm,
                "warnings": e.warnings,
                "quick": {"text": format::QUICK_TEXT, "seconds": 60},
                "secure": {"text": format::SECURE_TEXT, "seconds": e.secure_secs,
                           "about": true, "bytes_per_second": e.bytes_per_second},
            }),
        ))
    }

    /// `POST disks/format` (T49, Advanced > Disks): format an external disk
    /// that is not the archive's, quick or secure, with the admin password
    /// and exactly the partitions the user saw. The screen shows the
    /// warning and the slide (disk name on the last screen); the server
    /// checks the disk again here and in the job.
    fn disks_format(self: &Arc<Self>, req: &Request) -> Result<Response, Fail> {
        let b = body(req)?;
        let (disk_id, shown) = disk_and_parts(&b)?;
        self.external_disk(&disk_id)?;
        let mode = field(&b, "mode")?
            .as_str()
            .and_then(format::mode)
            .ok_or_else(|| {
                Fail::new("request.json", "\"mode\" must be \"quick\" or \"secure\".")
            })?;
        let label = b
            .get("label")
            .map(|v| v.as_str().ok_or(()))
            .transpose()
            .ok()
            .flatten()
            .unwrap_or("");
        let fs_kind = match b.get("fs") {
            None => format::Fs::Exfat,
            Some(v) => v.as_str().and_then(format::fs_named).ok_or_else(|| {
                Fail::new("request.json", "\"fs\" must be \"exfat\" or \"ext4\".")
            })?,
        };
        let label = format::label_for(label, fs_kind).ok_or_else(|| {
            Fail::new(
                "request.json",
                if fs_kind == format::Fs::Ext4 {
                    "The name may have up to 16 letters, digits, spaces, - or _."
                } else {
                    "The name may have up to 11 letters, digits, spaces, - or _."
                },
            )
        })?;
        let confirm = b
            .get("confirm")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let pw = zeroize::Zeroizing::new(
            field(&b, "password")?
                .as_str()
                .ok_or_else(|| Fail::new("request.json", "\"password\" must be text."))?
                .to_string(),
        );
        self.check_password(&pw)?;
        if Job::running(&lock(&self.archive_job)) {
            return Err(busy());
        }
        // The estimate the person saw, for exactly this disk state (R4).
        if !lock(&self.confirms).take(&disk_id, &self.format_binding(&disk_id), &confirm) {
            return Err(Fail::new(
                "format.confirm_stale",
                "The disk changed or too much time passed. Look at it again before formatting.",
            ));
        }
        if self.require_index {
            format::check(&disk_id, &shown).map_err(format_refusal)?;
        }
        let job = Job::start("disk.format", "checking", "Checking the disk…", Some(0));
        {
            let mut j = lock(&self.archive_job);
            if Job::running(&j) {
                return Err(busy());
            }
            *j = Some(job.clone());
        }
        // The audit line (T58, R5): what alpd does to which disk.
        let (model, size_bytes) = disk_facts(&disk_id);
        let event = move |result, code: Option<String>| {
            events::record(events::Event {
                at: events::now(),
                kind: "disk.format",
                disk: model.clone(),
                size_bytes,
                mode: Some(if mode == alp_live::wipe::Mode::Secure {
                    "secure"
                } else {
                    "quick"
                }),
                fs: Some(fs_kind.name()),
                result,
                code,
            });
        };
        event("started", None);
        let me = Arc::clone(self);
        let cancel = Arc::clone(&job.cancel);
        std::thread::spawn(move || {
            let req = FormatReq {
                disk_id,
                shown,
                mode,
                fs: fs_kind,
                label,
                cancel,
            };
            let mut how = String::new();
            let res = me.drive("disk.format", |say| {
                how = (me.format)(&req, say)?;
                Ok(())
            });
            if res.is_ok()
                && let Some(j) = lock(&me.archive_job).as_mut()
            {
                // Review W3: the method actually used.
                j.message = format!("Done: the disk is formatted ({how}).");
            }
            match &res {
                Ok(()) => event("done", None),
                Err(job::End::Cancelled) => event("stopped", None),
                Err(job::End::Failed { code, .. }) => event("failed", Some(code.to_string())),
            }
            me.finish(&res);
        });
        Ok(Response::json(202, &json!({"job": job.json()})))
    }

    /// Run an archive job's work on this thread: every update goes to the
    /// job at once (the API only copies it, so `GET state` never waits for
    /// the work), and to the journal with the `ALP-JOB` prefix at every
    /// step change and at least every 15 s.
    fn drive(
        &self,
        id: &'static str,
        work: impl FnOnce(job::Progress) -> Result<(), job::End>,
    ) -> Result<(), job::End> {
        let started = Instant::now();
        let mut gate = job::LogGate::default();
        let res = work(&mut |u| {
            let text = {
                let mut g = lock(&self.archive_job);
                let Some(j) = g.as_mut() else { return };
                j.apply(u);
                gate.pass(started.elapsed(), u.step, Duration::from_secs(15))
                    .then(|| j.text())
            };
            if let Some(t) = text {
                eprintln!("{}", job::log_line(id, u));
                if self.require_index {
                    job::save_last(&t);
                }
            }
        });
        eprintln!("{}", job::end_line(id, &res, started.elapsed().as_secs()));
        res
    }

    /// The end of an archive-creation job: `done`, `cancelled`, or `failed`
    /// with the catalog code and a plain message.
    fn finish(&self, res: &Result<(), job::End>) {
        let text = {
            let mut g = lock(&self.archive_job);
            let Some(j) = g.as_mut() else { return };
            match res {
                Ok(()) => j.state = "done",
                Err(job::End::Cancelled) => {
                    j.state = "cancelled";
                    j.message = if j.id == "disk.format" {
                        "Stopped. The disk is only partly erased and has no file system now; format it again to use it.".into()
                    } else {
                        "Stopped. The disk test file and the unfinished archive folder were removed.".into()
                    };
                }
                Err(job::End::Failed { code, message, .. }) => {
                    let f = if j.id == "reset" {
                        Fail::new(
                            "reset.failed",
                            format!(
                                "ALPArchive could not delete everything: {message}. Please try again."
                            ),
                        )
                    } else if j.id == "disk.prepare" {
                        Fail::new(
                            "disk.failed",
                            format!("ALPArchive could not prepare this disk ({message})."),
                        )
                    } else if j.id == "disk.format" {
                        Fail::new(
                            "disk.format_failed",
                            format!("The disk was not formatted completely: {message}."),
                        )
                    } else {
                        Fail::new(
                            "archive.failed",
                            format!(
                                "ALPArchive could not create the locked archive ({message}). Nothing was deleted; an ALPArchive folder may be left on that disk."
                            ),
                        )
                    }
                    .with_code(code);
                    j.state = "failed";
                    j.message = f.message.clone();
                    j.error = Some(f);
                }
            }
            j.moved = Instant::now();
            j.text()
        };
        if self.require_index {
            job::save_last(&text);
            if matches!(res, Err(job::End::Failed { .. })) {
                // Field logs for the owner, without asking (ALP-REPORT).
                std::thread::spawn(|| match live::diag_save() {
                    Ok(name) => {
                        eprintln!("{} diagnosis saved on the stick: {name}", job::LOG_PREFIX)
                    }
                    Err(e) => eprintln!("{} diagnosis not saved: {e}", job::LOG_PREFIX),
                });
            }
        }
    }

    /// `POST reset`: "Delete everything and start over" (owner 2026-10-24).
    /// Only with the admin password (same back-off as sign-in), never while
    /// an archive job runs. Deletes the archive's own files on the backup
    /// disk and erases the index on the stick; the stick is then at first
    /// start. Nothing else is touched.
    ///
    /// A job (`reset`, 202): on an unprotected stick the index is overwritten
    /// for real, which takes minutes on a large stick (owner, 2026-11-03).
    /// The console is signed out at once; `lock.state` is `setup` when the
    /// job is `done`, and the job's message says what happened on the disk.
    fn reset(self: &Arc<Self>, req: &Request) -> Result<Response, Fail> {
        let pw = password_field(req)?;
        self.check_password(&pw)?;
        let job = Job::start("reset", "deleting", "Deleting the archive…", Some(0));
        let answer = job.json();
        {
            // Check and take the job slot under one lock (two requests at once).
            let mut slot = lock(&self.archive_job);
            if Job::running(&slot) {
                return Err(Fail::new(
                    "archive.busy",
                    "The archive is being created. Cancel that first.",
                ));
            }
            *slot = Some(job);
        }
        eprintln!("alpd: reset requested (password checked)");
        *lock(&self.console) = Console::default();
        let me = Arc::clone(self);
        std::thread::spawn(move || {
            let mut what = String::new();
            let res = me.drive("reset", |say| {
                what = (me.wipe)(say)
                    .map_err(|f| job::End::failed(archive::CODE_RESET, "erasing", f.message))?;
                Ok(())
            });
            if res.is_ok() {
                eprintln!("alpd: reset done: {what}");
                *lock(&me.recovery) = None;
                *lock(&me.paused) = false;
                lock(&me.immich).at = None;
                if let Some(j) = lock(&me.archive_job).as_mut() {
                    j.message = format!("Done: {what}.");
                }
            }
            me.finish(&res);
        });
        Ok(Response::json(202, &json!({"job": answer})))
    }

    /// `POST archive/cancel`: ask the running creation to stop. It stops
    /// at its next check (within about a second while writing) and removes
    /// only its own `archive.alp.partial`. Never waits for it.
    fn archive_cancel(&self) -> Result<Response, Fail> {
        let mut g = lock(&self.archive_job);
        match g.as_mut() {
            Some(j)
                if j.state == "running"
                    && (j.id == "archive.create"
                        || (j.id == "disk.format" && j.step == "erasing")) =>
            {
                j.cancel.store(true, Ordering::Relaxed);
                j.state = "cancelling";
                j.message = "Stopping…".into();
                eprintln!("{} {}: cancel requested", job::LOG_PREFIX, j.id);
                Ok(Response::json(202, &json!({"job": j.json()})))
            }
            Some(j) if j.state == "cancelling" => {
                Ok(Response::json(202, &json!({"job": j.json()})))
            }
            Some(j) if j.state == "running" => Err(Fail::new(
                "archive.busy",
                if j.id == "disk.prepare" {
                    "Preparing a disk cannot be stopped; it takes about a minute."
                } else if j.id == "disk.format" {
                    "Only the overwrite of a secure format can be stopped; this part takes about a minute."
                } else {
                    "Opening an archive cannot be stopped; it takes only a moment."
                },
            )),
            _ => Err(Fail::new(
                "archive.not_creating",
                "No archive is being created.",
            )),
        }
    }

    fn archive_recover(self: &Arc<Self>, req: &Request) -> Result<Response, Fail> {
        let b = body(req)?;
        let partuuid = field(&b, "partuuid")?
            .as_str()
            .ok_or_else(|| Fail::new("request.json", "\"partuuid\" must be text."))?
            .to_string();
        // Level 3 (ADR 0016): the ALP password of the stick the archive was
        // made with, instead of a recovery code.
        let alp_pw = b
            .get("alp_password")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(|s| zeroize::Zeroizing::new(s.to_string()));
        let typed = zeroize::Zeroizing::new(match &alp_pw {
            Some(_) => String::new(),
            None => field(&b, "recovery_key")?
                .as_str()
                .ok_or_else(|| Fail::new("request.json", "\"recovery_key\" must be text."))?
                .to_string(),
        });
        let passphrase = zeroize::Zeroizing::new(
            b.get("passphrase")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        );
        self.password_wait()?;
        let key = match &alp_pw {
            Some(p) => {
                index::check_password(p)?;
                RecoveryInput::Password(p.clone())
            }
            None => recovery_input(&typed, &passphrase)?,
        };
        if archive::record().is_some() {
            return Err(Fail::new(
                "archive.exists",
                "An archive is already set up with this stick.",
            ));
        }
        if !archive::partitions().iter().any(|p| p.partuuid == partuuid) {
            return Err(Fail::new(
                "archive.not_found",
                "This place is not on this computer any more.",
            ));
        }
        let job = Job::start(
            "archive.recover",
            "checking",
            "Checking the recovery key…",
            Some(0),
        );
        {
            let mut j = lock(&self.archive_job);
            if Job::running(&j) {
                return Err(Fail::new("archive.busy", "The archive is busy."));
            }
            *j = Some(job.clone());
        }
        let me = Arc::clone(self);
        std::thread::spawn(move || {
            let mut say = |u: &job::Update| {
                if let Some(j) = lock(&me.archive_job).as_mut() {
                    j.apply(u);
                }
            };
            let res = match &key {
                RecoveryInput::Legacy(k) => archive::recover(&partuuid, k, &mut say),
                RecoveryInput::Secret(s) => recovery::open(&partuuid, s, &mut say),
                RecoveryInput::Password(p) => {
                    archive::recover_with_password(&partuuid, p, &mut say)
                }
            };
            if res.is_ok() {
                // Not under the job lock: `GET state` must never wait.
                systemctl(&["start", "alp-live-archive.service"]);
                systemctl(&["start", "--no-block", immich::UNIT]);
            }
            let mut j = lock(&me.archive_job);
            match res {
                Ok(()) => {
                    if let Some(j) = j.as_mut() {
                        j.state = "done";
                    }
                }
                Err(e) => {
                    if e == archive::RECOVERY_WRONG {
                        // Wrong words or password count like wrong passwords.
                        lock(&me.tries).failed();
                    }
                    let f = if [
                        archive::RECOVERY_WRONG,
                        archive::PASSWORD_NONE,
                        archive::RECOVERY_NEEDS_WORDS,
                        archive::RECOVERY_NEEDS_KEY,
                        archive::RECOVERY_NONE,
                    ]
                    .contains(&e.as_str())
                    {
                        Fail::new("recovery.wrong", e)
                    } else if e == recovery::NOT_READY {
                        recovery_fail(&e)
                    } else {
                        eprintln!("alpd: archive recover failed: {e}");
                        Fail::new(
                            "archive.failed",
                            format!(
                                "ALPArchive could not open this archive ({e}). Nothing was changed on it."
                            ),
                        )
                    };
                    if let Some(j) = j.as_mut() {
                        j.state = "failed";
                        j.message = f.message.clone();
                        j.error = Some(f);
                    }
                }
            }
        });
        Ok(Response::json(202, &json!({"job": job.json()})))
    }

    /// `POST phone/pair` (T14, T23): always the addresses to type, the
    /// family email (phones use the main password), and why phones cannot
    /// sign in yet when they cannot.
    fn pair(&self, req: &Request) -> Result<Response, Fail> {
        let b = body(req)?;
        let typed = match b.get("password") {
            None | Some(Value::Null) => None,
            Some(Value::String(p)) => Some(zeroize::Zeroizing::new(p.clone())),
            Some(_) => return Err(Fail::new("request.json", "\"password\" must be text.")),
        };
        if let Some(pw) = &typed {
            self.check_password(pw)?;
        }
        // Level 1 (ADR 0016): no ALP password; phones use the random phone
        // password made at setup, shown here.
        let phone_pw = index::phone_password().map(zeroize::Zeroizing::new);
        let typed = if phone_pw.is_some() {
            phone_pw.clone()
        } else {
            typed
        };
        let ports = live::ports();
        let urls: Vec<String> = addresses_json(&ports)
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v["url"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let address = live::main_address(&ports);
        let paused = *lock(&self.paused);
        let st = if paused {
            immich::State::Off
        } else {
            immich::state()
        };
        let mut uses_main = immich::family_uses_main_password();
        if st == immich::State::Ready
            && !uses_main
            && let Some(pw) = &typed
        {
            match self.family.apply(pw) {
                Ok(()) => {
                    uses_main = true;
                    *lock(&self.family.pending) = None;
                }
                Err(e) => eprintln!("alpd: family account not changed: {}", e.message),
            }
        }
        let waiting_for_worker = lock(&self.family.pending).is_some();
        let (word, reason): (&str, Option<String>) = match &st {
            _ if paused => (
                "paused",
                Some("There is no room left, so phones cannot save photos now. Make room or use a bigger disk.".into()),
            ),
            immich::State::Off if archive::record().is_none() => (
                "off",
                Some("Set up the archive first; your photos are kept there.".into()),
            ),
            immich::State::Off => (
                "off",
                Some("The archive is not open. Plug in the backup disk and wait a moment.".into()),
            ),
            immich::State::Starting => {
                let secs = lock(&self.immich)
                    .starting_since
                    .map_or(0, |t| t.elapsed().as_secs());
                (
                    "starting",
                    Some(format!(
                        "The photo service is still starting. {}",
                        immich::detail(secs)
                    )),
                )
            }
            immich::State::Failed(m) => ("failed", Some(m.clone())),
            immich::State::Ready if !uses_main && waiting_for_worker => (
                "ready",
                Some("Getting the family account ready. Please wait a moment.".into()),
            ),
            immich::State::Ready if !uses_main => (
                "ready",
                Some("Type your ALPArchive password once, so phones can use it.".into()),
            ),
            immich::State::Ready => ("ready", None),
        };
        let needs_password = st == immich::State::Ready && !uses_main && !waiting_for_worker;
        let reason = if address.is_none() {
            Some(
                "ALPArchive is not on your home network yet. Connect a cable or Wi-Fi first."
                    .to_string(),
            )
        } else {
            reason
        };
        lock(&self.immich).at = None;
        // The certificate nginx sends must name the addresses the phone uses
        // (0.2.5 field report): otherwise no QR, and the screen says why.
        let reason = if reason.is_none() && address.is_some() && index::is_open() {
            immich::certificate_for(&live::addresses(&ports))
                .err()
                .map(|e| e.message)
        } else {
            reason
        };
        let ready = reason.is_none();
        // No chain behind the scenes any more: the owner got a chain code he
        // never asked for (2026-10-30). A chain starts only from Settings.
        let url = address.as_deref().map(server_url);
        let cert_url = address.as_deref().map(|a| format!("http://{a}/"));
        Ok(Response::json(
            200,
            &json!({
                "ready": ready, "needs_password": needs_password,
                "server_url": url, "addresses": urls,
                "email": immich::FAMILY_EMAIL,
                "password": phone_pw.as_deref().map(|p| p.as_str()),
                "uses_main_password": phone_pw.is_none(),
                "immich": {"state": word, "reason": reason},
                "qr_text": url.as_deref().filter(|_| ready),
                "qr_svg": url.as_deref().filter(|_| ready).map(qr::svg),
                "certificate_sha256": immich::certificate_sha256(),
                "certificate_url": cert_url,
                "certificate_qr_svg": cert_url.as_deref().map(qr::svg),
            }),
        ))
    }

    /// `POST power/off` (T15): an ordered shutdown done by alpd itself,
    /// every step with a time limit, then the system's own power-off, with
    /// a forced one if the computer is still on 90 s later.
    fn power_off(self: &Arc<Self>, req: &Request) -> Result<Response, Fail> {
        let b = body(req)?;
        let restart = b.get("restart").and_then(Value::as_bool).unwrap_or(false);
        let mut pj = lock(&self.power_job);
        if let Some(j) = pj.as_ref() {
            return Ok(Response::json(202, &json!({"job": j.json()})));
        }
        let id = if restart {
            "power.restart"
        } else {
            "power.off"
        };
        let job = Job::start(id, "immich", "Stopping the photo service…", Some(10));
        let answer = job.json();
        *pj = Some(job);
        drop(pj);
        if let Some(j) = lock(&self.archive_job).as_ref() {
            j.cancel.store(true, Ordering::SeqCst);
        }
        let me = Arc::clone(self);
        std::thread::spawn(move || me.shut_down(restart));
        Ok(Response::json(202, &json!({"job": answer})))
    }

    fn power_step(&self, step: &str, message: &str, percent: u8) {
        if let Some(j) = lock(&self.power_job).as_mut() {
            j.step = step.into();
            j.message = message.into();
            j.percent = Some(percent);
            j.moved = Instant::now();
        }
    }

    fn shut_down(&self, restart: bool) {
        let mut log = Vec::new();
        let mut step = |me: &Self, name: &str, message: &str, percent: u8, f: &dyn Fn() -> bool| {
            me.power_step(name, message, percent);
            let t = Instant::now();
            let ok = f();
            log.push(format!(
                "{name} {}s {}",
                t.elapsed().as_secs(),
                if ok { "ok" } else { "not done" }
            ));
            if !ok {
                me.power_step(
                    name,
                    &format!(
                        "{} did not finish in time; going on.",
                        message.trim_end_matches('…')
                    ),
                    percent,
                );
            }
        };
        let stop = |unit: &str, secs: u64| bounded("systemctl", &["stop", unit], secs);
        step(self, "immich", "Stopping the photo service…", 10, &|| {
            stop(immich::UNIT, IMMICH_STOP_SECS) || {
                // The pod did not stop in time: end it, the database
                // recovers from its own log at the next start.
                bounded(
                    "systemctl",
                    &["kill", "--signal=SIGKILL", "user@50000.service"],
                    5,
                );
                stop(immich::UNIT, 5)
            }
        });
        step(
            self,
            "archive",
            "Closing the archive safely…",
            40,
            &|| {
                let unit = stop("alp-live-archive.service", 20);
                // An archive made in this session is open without its unit.
                // Not while the unit's own close may still run (timed out).
                if unit && archive::is_open() {
                    bounded("alpctl", &["live", "archive", "close"], 15);
                }
                unit && !archive::is_open()
            },
        );
        step(self, "index", "Locking the index…", 70, &|| {
            let unit = stop("alp-live-index.service", 10);
            if unit && index::is_open() {
                bounded("alpctl", &["live", "index", "close"], 10);
            }
            unit && !index::is_open()
        });
        step(
            self,
            "saving",
            "Saving everything to the disks…",
            85,
            &|| bounded("sync", &[], 10),
        );
        let verb = if restart { "reboot" } else { "poweroff" };
        let line = format!("{} {verb}: {}", live::now_rfc3339(), log.join(", "));
        eprintln!("alpd: shutdown: {line}");
        // On the stick's ALP-REPORT drive, for the owner (no names, no secrets).
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(live::shutdown_log(&line));
        });
        if let Ok(Err(e)) = rx.recv_timeout(Duration::from_secs(5)) {
            eprintln!("alpd: shutdown log not saved: {e}");
        }
        self.power_step(
            "off",
            if restart {
                "Restarting now…"
            } else {
                "Turning off now…"
            },
            100,
        );
        systemctl(&[verb]);
        // Watchdog: alpd is normally stopped by the shutdown before this
        // ends (then poweroff.target's own JobTimeoutSec forces it).
        std::thread::sleep(Duration::from_secs(90));
        eprintln!("alpd: still on 90 s after asking to {verb}; forcing it");
        let _ = bounded("sync", &[], 10);
        systemctl(&[verb, "--force"]);
        std::thread::sleep(Duration::from_secs(30));
        systemctl(&[verb, "--force", "--force"]);
    }

    fn chain_create(&self) -> Result<Response, Fail> {
        let c = self.chain.create(&live::now_rfc3339())?;
        Ok(Response::json(200, &code_json(&c)))
    }

    fn chain_join(&self, req: &Request) -> Result<Response, Fail> {
        let b = body(req)?;
        let words: Vec<String> = if let Some(t) = b.get("qr_text").and_then(Value::as_str) {
            chain::words_from_qr(t)?
        } else {
            field(&b, "words")?
                .as_array()
                .ok_or_else(|| Fail::new("request.json", "\"words\" must be a list."))?
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        };
        let wait = lock(&self.join_tries).wait();
        if wait > 0 {
            let mut f = Fail::new(
                "chain.wait",
                format!("Too many wrong codes. Please wait {wait} seconds, then try again."),
            );
            f.retry_after = Some(wait);
            return Err(f);
        }
        match self.chain.join(&words, &live::now_rfc3339()) {
            Ok(c) => {
                *lock(&self.join_tries) = Tries::default();
                Ok(Response::json(200, &json!({"chain_id": c.chain_id})))
            }
            Err(e) => {
                // A wrong checksum is what guessing looks like.
                if e.key == "chain.checksum" {
                    lock(&self.join_tries).failed();
                }
                Err(e)
            }
        }
    }
}

/// What `archive/recover` was given: an old 64-letter key (archives made
/// before 2026-10-24) or the secret of a 15-word code.
enum RecoveryInput {
    Legacy(zeroize::Zeroizing<String>),
    Secret(zeroize::Zeroizing<String>),
    /// Level 3 (ADR 0016): the ALP password the archive was used with.
    Password(zeroize::Zeroizing<String>),
}

/// Check a typed recovery code before any job starts: the old 64-letter
/// key first (its 8 dash-separated groups must not look like 8 words),
/// then 15 words (an 8-word chain code gets its own hint).
fn recovery_input(typed: &str, passphrase: &str) -> Result<RecoveryInput, Fail> {
    if let Some(k) = archive::normalize_recovery_key(typed) {
        return Ok(RecoveryInput::Legacy(zeroize::Zeroizing::new(k)));
    }
    let words = recovery::split_words(typed);
    if words.len() < 2 {
        return Err(Fail::new(
            "recovery.format",
            "A recovery code has 15 words. Please check it.",
        ));
    }
    let code = recovery::parse(&words)?;
    if !passphrase.is_empty() {
        recovery::check_passphrase(passphrase)?;
    }
    Ok(RecoveryInput::Secret(code.unlock_secret(passphrase)))
}

/// The storage seam's answers as API errors.
fn recovery_fail(e: &str) -> Fail {
    if e == recovery::NOT_OPEN {
        not_open()
    } else if e == recovery::OLD_ARCHIVE {
        Fail::new("recovery.old_archive", e)
    } else if e == recovery::NOT_ENCRYPTED {
        Fail::new("recovery.not_encrypted", e)
    } else if e == recovery::NO_PENDING {
        Fail::new(
            "recovery.no_pending",
            "No new recovery code is waiting. Your current code still works.",
        )
    } else if e == recovery::NOT_READY {
        Fail::new(
            "recovery.not_ready",
            "Recovery codes start working with the next ALPArchive storage update.",
        )
    } else {
        eprintln!("alpd: recovery code: {e}");
        Fail::new(
            "recovery.failed",
            "ALPArchive could not make a recovery code. Please try again.",
        )
    }
}

fn server_url(address: &str) -> String {
    format!("https://{address}")
}

fn no_wifi() -> Fail {
    Fail::new(
        "wifi.no_device",
        "This computer has no working Wi-Fi. You can plug in a network cable instead.",
    )
}

fn code_json(c: &chain::Code) -> Value {
    json!({"chain_id": c.chain_id, "words": *c.words, "qr_text": c.qr_text.as_str(),
           "qr_svg": qr::svg(&c.qr_text)})
}

fn candidates() -> Value {
    let all = archive::candidates();
    let mut list: Vec<Value> = all
        .iter()
        .map(|(p, v)| {
            let (usable, reason, code, free, has, dirty) = match v {
                Ok(pr) => (true, None, None, pr.free_bytes, pr.has_archive, pr.dirty),
                Err(why) => (false, Some(why.reason.clone()), Some(why.code), 0, false, false),
            };
            // The room the archive can grow into (ADR 0014; no size to choose).
            let max = if usable { archive::max_size(free) } else { 0 };
            let repair = usable && !has && dirty;
            let disk: Vec<_> = all.iter().filter(|(o, _)| o.disk == p.disk).collect();
            json!({
                "partuuid": p.partuuid, "name": p.name, "label": p.label, "fs": p.fs_type,
                "disk": p.disk, "disk_model": archive::disk_model(&p.disk),
                "disk_id": archive::disk_key(&p.disk),
                "size_bytes": p.size_bytes, "free_bytes": free,
                "usable": usable && !has, "reason": if has {
                    Some("This place already has an ALPArchive folder from another stick.".to_string())
                } else { reason },
                "code": code,
                "repair": repair,
                "note": if repair { Some(archive::REPAIR_NOTE) } else { None },
                "can_prepare": archive::preparable(&disk),
                "recommended": false,
                "default_size_bytes": max,
                "min_size_bytes": archive::MIN_BYTES, "max_size_bytes": max,
                "has_archive": has,
            })
        })
        .collect();
    list.sort_by_key(|c| {
        (
            !c["usable"].as_bool().unwrap_or(false),
            std::cmp::Reverse(c["free_bytes"].as_u64().unwrap_or(0)),
        )
    });
    if let Some(first) = list.first_mut().filter(|c| c["usable"] == true) {
        first["recommended"] = true.into();
    }
    json!({"candidates": list})
}

/// Everything "start over" deletes, in order: Immich stops, the archive's
/// own files on the backup disk are deleted (when it is ours and here),
/// then the index is erased. Stops at the first failure, before the index
/// is touched when the archive part fails.
/// What `disks/format` hands to the job.
pub(crate) struct FormatReq {
    disk_id: String,
    shown: Vec<String>,
    mode: alp_live::wipe::Mode,
    fs: format::Fs,
    label: String,
    cancel: Arc<AtomicBool>,
}

type FormatFn = fn(&FormatReq, job::Progress) -> Result<String, job::End>;
type CheckFn = fn(&str, &AtomicBool, job::Progress) -> Result<String, job::End>;

fn format_disk(r: &FormatReq, say: job::Progress) -> Result<String, job::End> {
    format::format(&r.disk_id, &r.shown, r.mode, r.fs, &r.label, &r.cancel, say)
        .map(|m| m.describe().to_string())
}

fn busy() -> Fail {
    Fail::new(
        "archive.busy",
        "Something is being done with a disk; wait until it is finished.",
    )
}

/// `disk_id` and `partuuids` of a disks/… request.
/// Model and size of the disk `disk_id`, for the event log ("" and 0 when
/// it is gone or in unit tests).
fn disk_facts(disk_id: &str) -> (String, u64) {
    let Some(name) = archive::disk_of_id(disk_id) else {
        return (String::new(), 0);
    };
    let r = alp_hw::inspect::inspect(&alp_hw::inspect::Roots::system());
    r.disks
        .iter()
        .find(|d| d.id.0 == name)
        .map_or((String::new(), 0), |d| (d.model.clone(), d.size_bytes))
}

/// The stick or an internal disk (ADR 0022): Live never changes it.
fn disk_locked() -> Fail {
    Fail::new(
        "disk.locked",
        "This is the computer's own disk (or the ALPArchive stick). ALPArchive Live never changes it, to keep what is on it safe.",
    )
    .with_code(disks::CODE_LOCKED)
}

/// A text field of a request body.
fn text_field(b: &Value, name: &str) -> Result<String, Fail> {
    field(b, name)?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| Fail::new("request.json", format!("\"{name}\" must be text.")))
}

fn disk_and_parts(b: &Value) -> Result<(String, Vec<String>), Fail> {
    let disk_id = field(b, "disk_id")?
        .as_str()
        .ok_or_else(|| Fail::new("request.json", "\"disk_id\" must be text."))?
        .to_string();
    let shown: Vec<String> = field(b, "partuuids")?
        .as_array()
        .and_then(|a| a.iter().map(|v| v.as_str().map(str::to_string)).collect())
        .ok_or_else(|| Fail::new("request.json", "\"partuuids\" must be a list of text."))?;
    Ok((disk_id, shown))
}

fn format_refusal(r: archive::PrepareRefusal) -> Fail {
    match r {
        archive::PrepareRefusal::NotFound => Fail::new(
            "disk.not_found",
            "This disk is not plugged in any more. Look at the list of disks again.",
        ),
        archive::PrepareRefusal::Changed => Fail::new(
            "disk.changed",
            "What is on this disk changed. Look at the list of disks again.",
        ),
        archive::PrepareRefusal::Refused(m) => {
            Fail::new("disk.refused", m).with_code(format::CODE_NO_FORMAT)
        }
    }
}

fn reset_everything(say: job::Progress) -> Result<String, Fail> {
    let failed = |e: String| {
        Fail::new(
            "reset.failed",
            format!(
                "ALPArchive could not delete everything: {e}. Nothing more was deleted; please try again."
            ),
        )
    };
    systemctl(&["stop", immich::UNIT]);
    systemctl(&["stop", "alp-live-archive.service"]);
    let what = archive::reset().map_err(failed)?;
    say(&job::Update::step("erasing", "Erasing the stick…", 10));
    systemctl(&["stop", "alp-live-index.service"]);
    let how = index::reset(&mut |done, total, speed| {
        let pct = done.saturating_mul(100) / total.max(1);
        let left = total.saturating_sub(done) / speed.max(1);
        let mut u = job::Update::step(
            "erasing",
            format!(
                "Erasing the stick… {pct}% (about {} min left)",
                left.div_ceil(60)
            ),
            10 + u8::try_from(pct * 89 / 100).unwrap_or(89),
        );
        u.done_bytes = Some(done);
        u.total_bytes = Some(total);
        say(&u);
    })
    .map_err(failed)?;
    archive::clear_setup_done();
    prefs::clear();
    // The method actually used goes into the result (review W3).
    Ok(format!("{what}; the stick: {how}"))
}

/// Held while `password/setup` runs (see there).
static SETUP: Mutex<()> = Mutex::new(());

fn setup_busy() -> Fail {
    Fail::new(
        "setup.busy",
        "The stick is still being prepared. Please wait a moment, then try again.",
    )
}

/// Server capabilities announced in the state (`features`).
const FEATURES: &[&str] = &[
    "setup_progress",
    "recovery_commit",
    "disks",
    "events",
    "disk_check",
    "disk_eject",
    "status_levels",
    "prefs",
    "idle_lock_pref",
    "media_files",
    "clock",
    "gallery_session",
];

/// [`FEATURES`], plus `display_modes` when the image has `wlr-randr` and
/// `terminal` when the image carries it.
fn features() -> Vec<&'static str> {
    let mut f = FEATURES.to_vec();
    if display::modes_supported() {
        f.push("display_modes");
    }
    if live_terminal::available() {
        f.push("terminal");
    }
    f
}

fn not_open() -> Fail {
    Fail::new(
        "archive.not_open",
        "Set up the archive first; your photos are kept there.",
    )
}

fn password_field(req: &Request) -> Result<zeroize::Zeroizing<String>, Fail> {
    let b = body(req)?;
    Ok(zeroize::Zeroizing::new(
        field(&b, "password")?
            .as_str()
            .ok_or_else(|| Fail::new("request.json", "\"password\" must be text."))?
            .to_string(),
    ))
}

/// Value of cookie `name` in the request.
fn cookie<'a>(req: &'a Request, name: &str) -> Option<&'a str> {
    req.header("cookie")?.split(';').find_map(|c| {
        let (k, v) = c.trim().split_once('=')?;
        (k == name).then_some(v)
    })
}

/// Compare without stopping at the first difference.
fn ct_eq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

fn random_hex() -> zeroize::Zeroizing<String> {
    use std::io::Read;
    let mut b = zeroize::Zeroizing::new([0u8; 32]);
    // /dev/urandom does not fail on Linux once booted; an empty token would
    // never match a cookie (ct_eq on length 0 vs. 64 hex characters).
    let _ = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b[..]));
    zeroize::Zeroizing::new(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// First character, then `•` (at most 8): names on the lock screen.
fn mask(s: &str) -> String {
    let mut it = s.chars();
    match it.next() {
        Some(c) => std::iter::once(c)
            .chain(std::iter::repeat_n('•', it.count().clamp(1, 8)))
            .collect(),
        None => String::new(),
    }
}

/// What the lock screen may show (contract: "Locked view").
/// One standing problem for `state.alerts` (T54).
fn alert(key: &str, level: &str, message: &str, code: Option<&str>) -> Value {
    json!({"key": key, "level": level, "message": message, "code": code})
}

/// The standing problems in a state (pure, T54): space, the archive's
/// error, Immich paused, the archive disk's own health. Each has a level
/// (ADR 0023); critical only from alpd's own checks (review R7).
fn alerts(st: &Value, archive_health: Option<&str>) -> Vec<Value> {
    let mut v = Vec::new();
    let space = |what: &str, word: &Value, v: &mut Vec<Value>| match word.as_str() {
        Some("low") => v.push(alert(
            &format!("{what}.space_low"),
            "action",
            if what == "index" {
                "The ALPArchive stick is almost full."
            } else {
                "The backup disk is almost full."
            },
            Some("ALP-STO-003"),
        )),
        Some("full") => v.push(alert(
            &format!("{what}.space_full"),
            "now",
            if what == "index" {
                "The ALPArchive stick is full. New photos are not saved."
            } else {
                "The backup disk is full. New photos are not saved."
            },
            Some("ALP-STO-003"),
        )),
        _ => {}
    };
    space("index", &st["index"]["space"], &mut v);
    space("archive", &st["archive"]["space"], &mut v);
    for at in [&st["archive"]["error"], &st["immich"]["error"]] {
        if let (Some(key), Some(level), Some(msg)) = (
            at["key"].as_str(),
            at["level"].as_str(),
            at["message"].as_str(),
        ) {
            v.push(alert(key, level, msg, at["code"].as_str()));
        }
    }
    match archive_health {
        Some("failing") => v.push(alert(
            "archive.disk_failing",
            "critical",
            "The backup disk says it may fail soon. Stop using the archive and copy your photos to a new disk now.",
            Some(archive::CODE_SMART),
        )),
        Some("wear") => v.push(alert(
            "archive.disk_wear",
            "action",
            "The backup disk is wearing out. Plan to move your photos to a new disk soon.",
            Some("ALP-STO-002"),
        )),
        _ => {}
    }
    v
}

/// `state.status`: the most serious alert, or `{"level": "ok"}`.
fn status_of(alerts: &[Value]) -> Value {
    let rank = |a: &Value| {
        alp_live::fail::LEVELS
            .iter()
            .position(|l| a["level"] == *l)
            .unwrap_or(0)
    };
    alerts
        .iter()
        .max_by_key(|a| rank(a))
        .cloned()
        .unwrap_or_else(|| json!({"level": "ok", "key": null, "message": null, "code": null}))
}

/// Pause (or allow again) phone uploads through nginx: a map file it reads
/// (`$alp_critical`), then a reload. True when done.
fn uploads_pause(on: bool) -> bool {
    const MAP: &str = "/run/alp/critical-map.conf";
    let written = if on {
        std::fs::write(MAP, "\"~.\" 1;\n")
    } else {
        match std::fs::remove_file(MAP) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    };
    if let Err(e) = written {
        eprintln!("alpd: upload pause file: {}", e.kind());
        return false;
    }
    systemctl(&["reload", "nginx.service"]);
    true
}

fn locked_view(v: &mut Value) {
    v["console"]["state"] = "locked".into();
    v["hardware"]["report_file"] = Value::Null;
    if let Some(place) = v["archive"]["place"].as_object_mut() {
        for k in ["name", "label"] {
            if let Some(s) = place.get(k).and_then(Value::as_str).map(mask) {
                place.insert(k.into(), s.into());
            }
        }
    }
    if let Some(ports) = v["network"]["ports"].as_array_mut() {
        for p in ports {
            if let Some(s) = p["ssid"].as_str().map(mask) {
                p["ssid"] = s.into();
            }
        }
    }
    v["phone"]["server_url"] = Value::Null;
    v["phone"]["account"] = Value::Null;
    v["phone"]["connected_now"] = json!([]);
    v["pairing"]["pending"] = json!([]);
    v["pairing"]["devices"] = json!([]);
    if let Some(devs) = v["phone"]["devices"].as_array_mut() {
        for d in devs {
            for k in ["id", "app_version"] {
                d[k] = Value::Null;
            }
            if let Some(s) = d["name"].as_str().map(mask) {
                d["name"] = s.into();
            }
        }
    }
    v["chain"]["chain_id"] = Value::Null;
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// `GET phone/connection`: addresses, the certificate file, what nginx
/// really sends on each address, the pairing mode and nginx's last refused
/// requests (it logs only 4xx/5xx, paths without query strings: no secrets).
/// Read by nginx's unit (journald's own field), not by the syslog tag that
/// any local process could forge (security review N1); the lines are shown
/// as plain text only.
fn connection_check() -> Value {
    let ports = live::ports();
    let addrs = live::addresses(&ports);
    let mut v = alp_live::tls::check(&addrs);
    v["addresses"] = json!(addrs);
    v["ports"] = json!(ports
        .iter()
        .map(|p| json!({"name": p.name, "wifi": p.wifi, "carrier": p.carrier, "address": p.address}))
        .collect::<Vec<_>>());
    v["index_open"] = json!(index::is_open());
    v["only_paired_phones"] = json!(index::is_open() && pairing::strict());
    let refused = Command::new("journalctl")
        .args([
            "-b",
            "_SYSTEMD_UNIT=nginx.service",
            "--no-pager",
            "-o",
            "short-iso",
            "-n",
            "20",
        ])
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    v["refused_requests"] = json!(refused);
    v["clock"] = clock_json();
    v
}

/// `state.clock` (feature `clock`): the computer's time in seconds since
/// 1970 (UTC) and whether it was set from the network. systemd-timesyncd
/// touches this file on each successful synchronisation
/// (systemd-timesyncd.service(8), FILES); /run is empty at every start.
/// A clock that was not set this way can be hours off (a hardware clock
/// kept in local time is read as UTC): the 0.2.5-test1 field report.
fn clock_json() -> Value {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    json!({"now": now, "synced": Path::new("/run/systemd/timesync/synchronized").exists()})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live(name: &str) -> Arc<Live> {
        let d = std::env::temp_dir().join(format!("alpd-chain-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let l = Live::new(chain::Store::new(d));
        assert_eq!(l.need_open().unwrap_err().key, "locked");
        Arc::new(Live {
            require_index: false,
            password_ok: |pw| {
                if pw == "right password" {
                    Ok(())
                } else {
                    Err(Fail::new("password.wrong", "That is not the password."))
                }
            },
            wipe: |_| panic!("wipe must not run in this test"),
            format: |_, _| panic!("format must not run in this test"),
            serious: || false,
            prefs: Mutex::new(prefs::Prefs::default()),
            prefs_file: std::env::temp_dir()
                .join(format!("alpd-prefs-{name}-{}.json", std::process::id())),
            randr: fake_randr,
            revert_after: Duration::from_millis(300),
            ..Arc::try_unwrap(l).ok().unwrap()
        })
    }

    /// The `wlr-randr` calls the display tests made (no screen here).
    static RANDR_CALLS: Mutex<Vec<Vec<String>>> = Mutex::new(Vec::new());

    fn fake_randr(args: &[String]) -> Result<String, String> {
        lock(&RANDR_CALLS).push(args.to_vec());
        Ok("eDP-1 \"Screen\"\n  Enabled: yes\n  Modes:\n    1920x1080 px, 60.000000 Hz (preferred, current)\n    1280x720 px, 60.000000 Hz\n"
            .into())
    }

    #[test]
    fn setup_progress_takes_known_steps_only() {
        let ok = |v: Value| setup_request(&v).ok();
        assert_eq!(ok(json!({})), Some((None, false)));
        assert_eq!(ok(json!({"complete": true})), Some((None, true)));
        // Kept once each, in the order of the steps.
        assert_eq!(
            ok(json!({"skipped": ["phone", "network", "phone"]})),
            Some((Some(vec!["network", "phone"]), false))
        );
        assert_eq!(ok(json!({"skipped": []})), Some((Some(vec![]), false)));
        for bad in [
            json!({"skipped": ["network", "nonsense"]}),
            json!({"skipped": ["../x"]}),
            json!({"skipped": [1]}),
            json!({"skipped": "network"}),
            json!({"complete": false}),
            json!({"complete": 1}),
            json!({"other": true}),
        ] {
            let e = setup_request(&bad)
                .err()
                .unwrap_or_else(|| panic!("{bad} was taken"));
            assert_eq!(e.key, "setup.invalid");
        }
    }
    #[test]
    fn lock_screen_hides_names() {
        let mut v = json!({"console": {"state": "open"}, "hardware": {"report_file": "hw.txt"},
            "archive": {"place": {"name": "sdb1", "label": "My Passport"}},
            "network": {"ports": [{"ssid": "Home"}, {"ssid": null}]},
            "phone": {"server_url": "https://x", "account": "family@x", "photos": 3},
            "chain": {"chain_id": "ab"}});
        locked_view(&mut v);
        assert_eq!(v["console"]["state"], "locked");
        assert_eq!(v["archive"]["place"]["label"], "M••••••••");
        assert_eq!(v["archive"]["place"]["name"], "s•••");
        assert_eq!(v["network"]["ports"][0]["ssid"], "H•••");
        assert!(v["network"]["ports"][1]["ssid"].is_null());
        assert!(v["phone"]["account"].is_null() && v["chain"]["chain_id"].is_null());
        assert_eq!(v["phone"]["photos"], 3);
        assert!(ct_eq("abc", "abc") && !ct_eq("abc", "abd") && !ct_eq("abc", "ab"));
        let r = Request {
            headers: vec![("cookie".into(), "a=1; alp_console=xyz".into())],
            ..Request::default()
        };
        assert_eq!(cookie(&r, COOKIE), Some("xyz"));
    }

    fn call(l: &Arc<Live>, method: &str, path: &str, body: &str) -> (u16, Value) {
        let r = l.handle(&Request {
            method: method.into(),
            path: format!("{PREFIX}{path}"),
            body: body.as_bytes().to_vec(),
            ..Request::default()
        });
        (r.status, serde_json::from_slice(&r.body).unwrap())
    }

    /// The gallery proxy's question: never answered to a browser (it sends
    /// `Sec-Fetch-Site` or `Origin`), only to GET, and without a family
    /// key there is nothing to give (the test machine has none).
    #[test]
    fn the_gallery_gate_refuses_browsers_and_gives_no_key_without_one() {
        let l = live("gate");
        let ask = |method: &str, headers: Vec<(String, String)>| {
            l.handle(&Request {
                method: method.into(),
                path: format!("{PREFIX}gallery/gate"),
                headers,
                ..Request::default()
            })
        };
        for r in [
            ask("GET", vec![]),
            ask("POST", vec![]),
            ask("GET", vec![("sec-fetch-site".into(), "same-origin".into())]),
            ask(
                "GET",
                vec![("origin".into(), "http://127.0.0.1:8411".into())],
            ),
        ] {
            assert_eq!(r.status, 401);
            assert!(r.headers.iter().all(|(k, _)| *k != "X-Alp-Gallery-Key"));
        }
        let f = l.state(true)["features"].clone();
        assert!(f.as_array().unwrap().contains(&json!("gallery_session")));
        assert_eq!(l.state(true)["immich"]["gallery_url"], Value::Null);
    }

    #[test]
    fn chain_round_trip_and_errors() {
        let l = live("a");
        assert_eq!(call(&l, "GET", "chain/code", "").0, 404);
        let (s, c) = call(&l, "POST", "chain/create", "{}");
        assert_eq!(s, 200);
        assert_eq!(c["words"].as_array().unwrap().len(), 8);
        assert!(c["qr_svg"].as_str().unwrap().starts_with("<svg"));
        let (s, e) = call(&l, "POST", "chain/create", "{}");
        assert_eq!((s, e["error"]["key"].as_str()), (409, Some("chain.exists")));
        let (s, again) = call(&l, "GET", "chain/code", "");
        assert_eq!((s, &again["words"]), (200, &c["words"]));
        assert_eq!(call(&l, "POST", "chain/leave", "{}").0, 200);
        let join = json!({"qr_text": c["qr_text"]}).to_string();
        let (s, j) = call(&l, "POST", "chain/join", &join);
        assert_eq!((s, &j["chain_id"]), (200, &c["chain_id"]));
        call(&l, "POST", "chain/leave", "{}");
        let (s, e) = call(&l, "POST", "chain/join", r#"{"words": ["abandon"]}"#);
        assert_eq!(
            (s, e["error"]["key"].as_str()),
            (400, Some("chain.word_count"))
        );
    }

    const RECOVERY15: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon address";
    const CHAIN8: &str = "abandon abandon abandon abandon abandon abandon abandon ability";

    fn key(v: &(u16, Value)) -> (u16, Option<&str>) {
        (v.0, v.1["error"]["key"].as_str())
    }

    #[test]
    fn codes_are_told_apart_by_word_count() {
        let l = live("codes");
        // A recovery code in the chain field.
        let j = json!({"words": [RECOVERY15]}).to_string();
        assert_eq!(
            key(&call(&l, "POST", "chain/join", &j)),
            (400, Some("chain.recovery_code"))
        );
        // A chain code in the recovery field.
        let rec = |code: &str, pp: &str| {
            json!({"partuuid": "x", "recovery_key": code, "passphrase": pp}).to_string()
        };
        assert_eq!(
            key(&call(&l, "POST", "archive/recover", &rec(CHAIN8, ""))),
            (400, Some("recovery.chain_code"))
        );
        assert_eq!(
            key(&call(
                &l,
                "POST",
                "archive/recover",
                &rec("abandon abandon", "")
            )),
            (400, Some("recovery.word_count"))
        );
        assert_eq!(
            key(&call(&l, "POST", "archive/recover", &rec("hello", ""))),
            (400, Some("recovery.format"))
        );
        let typo = RECOVERY15.replacen("abandon", "abandn", 1);
        let (s, e) = call(&l, "POST", "archive/recover", &rec(&typo, ""));
        assert_eq!((s, e["error"]["index"].as_u64()), (400, Some(1)));
        let bad = RECOVERY15.replace("address", "abandon");
        assert_eq!(
            key(&call(&l, "POST", "archive/recover", &rec(&bad, ""))),
            (400, Some("recovery.checksum"))
        );
        assert_eq!(
            key(&call(
                &l,
                "POST",
                "archive/recover",
                &rec(RECOVERY15, "short")
            )),
            (400, Some("recovery.passphrase"))
        );
    }

    use std::sync::atomic::AtomicU8;
    /// The test store: 0 none, 1 a code, 2 a code and a pending one.
    static STORE: AtomicU8 = AtomicU8::new(0);
    const TEST_STORE: RecoveryStore = RecoveryStore {
        has: || STORE.load(Ordering::SeqCst) > 0,
        bind: |_| {
            STORE.fetch_max(1, Ordering::SeqCst);
            STORE.store(STORE.load(Ordering::SeqCst) | 2, Ordering::SeqCst);
            Ok(())
        },
        commit: || {
            STORE.store(1, Ordering::SeqCst);
            Ok(())
        },
        cancel: || {
            STORE.fetch_and(1, Ordering::SeqCst);
        },
    };

    #[test]
    fn recovery_create_needs_the_password_and_two_steps() {
        let l = Arc::new(Live {
            rec: TEST_STORE,
            ..Arc::try_unwrap(live("rcreate")).ok().unwrap()
        });
        assert_eq!(
            key(&call(&l, "POST", "recovery/create", "{}")),
            (400, Some("request.json")),
            "no password"
        );
        assert_eq!(
            key(&call(
                &l,
                "POST",
                "recovery/create",
                r#"{"password": "wrong"}"#
            )),
            (403, Some("password.wrong"))
        );
        let pw = r#"{"password": "right password"}"#;
        assert_eq!(
            key(&call(
                &l,
                "POST",
                "recovery/create",
                r#"{"password": "right password", "passphrase": "long enough"}"#
            )),
            (400, Some("recovery.passphrase")),
            "a passphrase only with serious encryption"
        );
        let (s, c) = call(&l, "POST", "recovery/create", pw);
        assert_eq!(s, 200);
        assert_eq!(STORE.load(Ordering::SeqCst), 3);
        // Cancel keeps the old code; a second create needs replace.
        assert_eq!(call(&l, "POST", "recovery/cancel", "{}").0, 200);
        assert_eq!(STORE.load(Ordering::SeqCst), 1);
        assert_eq!(
            key(&call(&l, "POST", "recovery/create", pw)),
            (409, Some("recovery.exists"))
        );
        let (s, c2) = call(
            &l,
            "POST",
            "recovery/create",
            r#"{"password": "right password", "replace": true}"#,
        );
        assert_eq!(s, 200);
        assert_ne!(c["words"], c2["words"]);
        let words: Vec<String> = c2["words"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w.as_str().unwrap().to_string())
            .collect();
        let ask: Vec<usize> = c2["check"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_u64().unwrap() as usize)
            .collect();
        assert_eq!(ask.len(), 3);
        let answer = |good: bool| {
            let m: serde_json::Map<String, Value> = ask
                .iter()
                .map(|&i| {
                    let w = if good {
                        words[i - 1].clone()
                    } else {
                        "zzz".into()
                    };
                    (i.to_string(), Value::from(w))
                })
                .collect();
            json!({"words": m}).to_string()
        };
        assert_eq!(
            key(&call(&l, "POST", "recovery/commit", &answer(false))),
            (422, Some("recovery.commit_wrong"))
        );
        assert_eq!(STORE.load(Ordering::SeqCst), 3, "still pending");
        assert_eq!(call(&l, "POST", "recovery/commit", &answer(true)).0, 200);
        assert_eq!(STORE.load(Ordering::SeqCst), 1);
        assert_eq!(
            key(&call(&l, "POST", "recovery/commit", &answer(true))),
            (409, Some("recovery.no_pending"))
        );
    }

    #[test]
    fn serious_archive_needs_the_password() {
        let l = live("serious");
        let b = |extra: &str| {
            format!(r#"{{"partuuid": "x", "size_bytes": 1, "serious": true{extra}}}"#)
        };
        assert_eq!(
            key(&call(&l, "POST", "archive/create", &b(""))),
            (400, Some("request.json"))
        );
        assert_eq!(
            key(&call(
                &l,
                "POST",
                "archive/create",
                &b(r#", "password": "wrong""#)
            )),
            (403, Some("password.wrong"))
        );
    }

    #[test]
    fn chain_joins_are_rate_limited() {
        let l = live("joinlimit");
        let bad = json!({"words": [CHAIN8.replace("ability", "abandon")]}).to_string();
        for _ in 0..5 {
            assert_eq!(
                key(&call(&l, "POST", "chain/join", &bad)),
                (400, Some("chain.checksum"))
            );
        }
        let (s, e) = call(&l, "POST", "chain/join", &bad);
        assert_eq!(s, 429);
        assert_eq!(e["error"]["key"], "chain.wait");
        assert!(e["error"]["retry_after_seconds"].as_u64().unwrap() > 0);
        // Even the right code waits.
        let good = json!({"words": [CHAIN8]}).to_string();
        assert_eq!(call(&l, "POST", "chain/join", &good).0, 429);
    }

    #[test]
    fn phone_reset_is_gone() {
        let l = live("reset-gone");
        let r = call(&l, "POST", "phone/reset", "{}");
        assert_eq!(key(&r), (410, Some("phone.reset_gone")));
    }

    #[test]
    fn addresses_wired_first_with_urls() {
        let p = |name: &str, wifi: bool, a: Option<&str>| live::Port {
            name: name.into(),
            wifi,
            carrier: a.is_some(),
            address: a.map(str::to_string),
        };
        let v = addresses_json(&[
            p("wlan0", true, Some("192.168.1.40")),
            p("enp1s0", false, None),
            p("enp2s0", false, Some("10.0.0.5")),
        ]);
        assert_eq!(
            v,
            json!([
                {"address": "10.0.0.5", "url": "https://10.0.0.5", "port": "enp2s0", "kind": "wired"},
                {"address": "192.168.1.40", "url": "https://192.168.1.40", "port": "wlan0", "kind": "wifi"},
            ])
        );
        assert_eq!(addresses_json(&[]), json!([]));
    }

    #[test]
    fn power_jobs_never_say_the_disk_is_stuck() {
        let mut j = Job::start(
            "power.off",
            "immich",
            "Stopping the photo service…",
            Some(10),
        );
        j.moved = Instant::now() - Duration::from_secs(STILL_WORKING + 5);
        assert_eq!(j.json()["message"], "Stopping the photo service…");
        assert_eq!(j.json()["can_cancel"], false);
    }

    #[test]
    fn check_ids_are_stable() {
        assert_eq!(check_id("Security chip (TPM)"), "tpm");
        assert_eq!(check_id("Wired network"), "wired_network");
        assert_eq!(
            check_id("AES-NI (fast encryption)"),
            "aes_ni_fast_encryption"
        );
    }

    #[test]
    fn routing_and_bodies() {
        let l = live("b");
        assert_eq!(call(&l, "GET", "nothing", "").0, 404);
        assert_eq!(call(&l, "GET", "chain/create", "").0, 405);
        let (s, e) = call(&l, "POST", "chain/create", "not json");
        assert_eq!((s, e["error"]["key"].as_str()), (400, Some("request.json")));
        let (s, e) = call(&l, "POST", "archive/create", "{}");
        assert_eq!((s, e["error"]["key"].as_str()), (400, Some("request.json")));
        let (s, st) = call(&l, "GET", "state", "");
        assert_eq!((s, st["api"].as_i64()), (200, Some(1)));
        assert_eq!(st["chain"]["state"], "none");
    }

    fn start_job(l: &Arc<Live>) -> Arc<AtomicBool> {
        let j = Job::start(
            "archive.create",
            "preparing",
            "Preparing the disk…",
            Some(0),
        );
        let c = Arc::clone(&j.cancel);
        *lock(&l.archive_job) = Some(j);
        c
    }

    #[test]
    fn job_progress_is_live_and_cancel_stops_it() {
        let l = live("job-cancel");
        let (s, e) = call(&l, "POST", "archive/cancel", "{}");
        assert_eq!(
            (s, e["error"]["key"].as_str()),
            (409, Some("archive.not_creating"))
        );
        let cancel = start_job(&l);
        let me = Arc::clone(&l);
        let worker = std::thread::spawn(move || {
            let res = me.drive("archive.create", |say| {
                let mut done = 0;
                // Writes until cancelled, one update per "block".
                while !cancel.load(Ordering::Relaxed) {
                    done += 1 << 20;
                    say(&archive::checking_update(done, 8 << 30, Some(4_000_000)));
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(job::End::Cancelled)
            });
            me.finish(&res);
            res
        });
        // The API answers at once while the job works, with bytes and speed.
        let mut seen = Vec::new();
        for _ in 0..3 {
            let t = Instant::now();
            let (s, st) = call(&l, "GET", "state", "");
            assert!(t.elapsed() < Duration::from_secs(2));
            assert_eq!(s, 200);
            assert_eq!(st["archive"]["state"], "creating");
            seen.push(st["archive"]["job"]["bytes_done"].as_u64().unwrap_or(0));
            std::thread::sleep(Duration::from_millis(150));
        }
        assert!(seen.windows(2).all(|w| w[0] < w[1]), "{seen:?}");
        let (_, st) = call(&l, "GET", "state", "");
        let j = &st["archive"]["job"];
        assert_eq!(j["step"], "checking");
        assert_eq!(j["bytes_total"].as_u64(), Some(8 << 30));
        assert_eq!(j["bytes_per_second"].as_u64(), Some(4_000_000));
        assert!(j["seconds_left"].as_u64().is_some());
        assert!(j["message"].as_str().unwrap().contains("MB/s"));
        assert_eq!(j["can_cancel"], true);
        // Looking at disks is refused while one is written.
        let (s, e) = call(&l, "GET", "archive/candidates", "");
        assert_eq!((s, e["error"]["key"].as_str()), (409, Some("archive.busy")));
        let (s, c) = call(&l, "POST", "archive/cancel", "{}");
        assert_eq!((s, c["job"]["state"].as_str()), (202, Some("cancelling")));
        // Asking twice is fine.
        assert_eq!(call(&l, "POST", "archive/cancel", "{}").0, 202);
        assert_eq!(worker.join().unwrap(), Err(job::End::Cancelled));
        let (_, st) = call(&l, "GET", "state", "");
        assert_eq!(st["archive"]["job"]["state"], "cancelled");
        assert_eq!(st["archive"]["state"], "none");
        assert!(st["archive"]["error"].is_null());
    }

    #[test]
    fn a_failed_step_ends_the_job_with_a_code() {
        let l = live("job-fail");
        start_job(&l);
        let res = l.drive("archive.create", |say| {
            say(&job::Update::step("encrypting", "Encrypting…", 60));
            Err(job::End::failed(
                archive::CODE_ENCRYPT,
                "encrypting",
                "cryptsetup failed (exit status: 1): Device is busy",
            ))
        });
        l.finish(&res);
        let (_, st) = call(&l, "GET", "state", "");
        let j = &st["archive"]["job"];
        assert_eq!(j["state"], "failed");
        assert_eq!(j["can_cancel"], false);
        assert_eq!(j["error"]["key"], "archive.failed");
        assert_eq!(j["error"]["code"], archive::CODE_ENCRYPT);
        assert!(
            j["error"]["message"]
                .as_str()
                .unwrap()
                .contains("Device is busy")
        );
        let (s, e) = call(&l, "POST", "archive/cancel", "{}");
        assert_eq!(
            (s, e["error"]["key"].as_str()),
            (409, Some("archive.not_creating"))
        );
    }

    #[test]
    fn a_waiting_job_still_shows_time_passing() {
        let l = live("job-wait");
        start_job(&l);
        lock(&l.archive_job).as_mut().unwrap().moved = Instant::now() - Duration::from_secs(90);
        let (_, st) = call(&l, "GET", "state", "");
        assert!(
            st["archive"]["job"]["seconds_since_progress"]
                .as_u64()
                .unwrap()
                >= 90
        );
        // A recovery job cannot be stopped.
        lock(&l.archive_job).as_mut().unwrap().id = "archive.recover";
        let (s, e) = call(&l, "POST", "archive/cancel", "{}");
        assert_eq!((s, e["error"]["key"].as_str()), (409, Some("archive.busy")));
    }

    #[test]
    fn storage_seam_errors_map_to_api_keys() {
        assert_eq!(recovery_fail(recovery::NOT_OPEN).key, "archive.not_open");
        let old = recovery_fail(recovery::OLD_ARCHIVE);
        assert_eq!(
            (status_for(old.key), old.key, old.message.as_str()),
            (409, "recovery.old_archive", recovery::OLD_ARCHIVE)
        );
        assert_eq!(recovery_fail("gocryptfs failed").key, "recovery.failed");
    }

    #[test]
    fn disk_format_checks_password_mode_and_name_and_reports_the_method() {
        static RAN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let l = Arc::new(Live {
            format: |r, say| {
                assert_eq!(r.label, "Photos");
                assert_eq!(r.fs, format::Fs::Exfat);
                say(&job::Update::step(
                    "erasing",
                    "Overwriting with zeros… 50%",
                    45,
                ));
                // Stopped on request while overwriting.
                let t = Instant::now();
                while !r.cancel.load(Ordering::Relaxed) && t.elapsed() < Duration::from_secs(5) {
                    std::thread::sleep(Duration::from_millis(10));
                }
                RAN.fetch_add(1, Ordering::SeqCst);
                if r.cancel.load(Ordering::Relaxed) {
                    return Err(job::End::Cancelled);
                }
                Ok("secure: overwritten with zeros".into())
            },
            ..Arc::try_unwrap(live("format")).ok().unwrap()
        });
        // Each call brings a fresh confirmation, as after an estimate (R4).
        let body = |pw: &str, mode: &str, label: &str| {
            let c = lock(&l.confirms).issue("sdz-1", "sdz-1".into()).unwrap();
            format!(
                r#"{{"disk_id": "sdz-1", "partuuids": ["a"], "mode": "{mode}", "label": "{label}", "password": "{pw}", "confirm": "{c}"}}"#
            )
        };
        assert_eq!(
            key(&call(
                &l,
                "POST",
                "disks/format",
                &body("wrong", "quick", "")
            )),
            (403, Some("password.wrong"))
        );
        // No confirmation, a used one, one from before a lock: refused.
        let no = r#"{"disk_id": "sdz-1", "partuuids": ["a"], "mode": "quick", "password": "right password"}"#;
        assert_eq!(
            key(&call(&l, "POST", "disks/format", no)),
            (409, Some("format.confirm_stale"))
        );
        let c = lock(&l.confirms).issue("sdz-1", "sdz-1".into()).unwrap();
        let with = |c: &str| no.replace("}", &format!(r#", "confirm": "{c}"}}"#));
        lock(&l.confirms).clear();
        assert_eq!(
            key(&call(&l, "POST", "disks/format", &with(&c))),
            (409, Some("format.confirm_stale"))
        );
        // fs: only exfat or ext4; an ext4 name may be 16 long, exFAT 11.
        let fs = |f: &str, label: &str| {
            body("right password", "quick", label)
                .replace(r#""mode""#, &format!(r#""fs": "{f}", "mode""#))
        };
        assert_eq!(
            key(&call(&l, "POST", "disks/format", &fs("ntfs", ""))),
            (400, Some("request.json"))
        );
        assert_eq!(
            key(&call(
                &l,
                "POST",
                "disks/format",
                &fs("exfat", "Twelve chars")
            )),
            (400, Some("request.json"))
        );
        assert_eq!(
            key(&call(
                &l,
                "POST",
                "disks/format",
                &body("right password", "fast", "")
            )),
            (400, Some("request.json"))
        );
        assert_eq!(
            key(&call(
                &l,
                "POST",
                "disks/format",
                &body("right password", "secure", "-o x")
            )),
            (400, Some("request.json"))
        );
        assert_eq!(call(&l, "GET", "disks/format", "").0, 405);
        start_job(&l);
        assert_eq!(
            key(&call(
                &l,
                "POST",
                "disks/format",
                &body("right password", "secure", "Photos")
            )),
            (409, Some("archive.busy"))
        );
        assert_eq!(RAN.load(Ordering::SeqCst), 0);
        *lock(&l.archive_job) = None;
        // Until the step is `want`, or the job has ended.
        let wait = |want: &str| {
            let t = Instant::now();
            loop {
                let (_, st) = call(&l, "GET", "state", "");
                let j = st["archive"]["job"].clone();
                let ended = !matches!(j["state"].as_str(), Some("running" | "cancelling"));
                if j["step"] == want || ended || t.elapsed() > Duration::from_secs(10) {
                    return j;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        // 1: cancelled while overwriting: "partly erased", never Done.
        let (s, r) = call(
            &l,
            "POST",
            "disks/format",
            &body("right password", "secure", "Photos"),
        );
        assert_eq!((s, r["job"]["id"].as_str()), (202, Some("disk.format")));
        assert_eq!(wait("erasing")["can_cancel"], true);
        assert_eq!(call(&l, "POST", "archive/cancel", "{}").0, 202);
        let j = wait("never");
        assert_eq!(j["state"], "cancelled");
        assert!(j["message"].as_str().unwrap().contains("partly erased"));
        // 2: to the end: the method is in the result.
        let (s, _) = call(
            &l,
            "POST",
            "disks/format",
            &body("right password", "secure", "Photos"),
        );
        assert_eq!(s, 202);
        let j = wait("never");
        assert_eq!(j["state"], "done");
        assert_eq!(
            j["message"],
            "Done: the disk is formatted (secure: overwritten with zeros)."
        );
        assert_eq!(RAN.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn alerts_and_the_worst_status() {
        let ok = json!({"index": {"space": "ok"}, "archive": {"space": "ok", "error": null}, "immich": {}});
        assert!(alerts(&ok, None).is_empty());
        assert_eq!(status_of(&alerts(&ok, Some("healthy")))["level"], "ok");
        let st = json!({"index": {"space": "low"}, "archive": {"space": "full",
            "error": {"key": "archive.missing", "level": "now", "message": "m", "code": "ALP-STO-001"}},
            "immich": {"error": fail_json(&Fail::new("space.full", "full"))}});
        let a = alerts(&st, Some("wear"));
        let keys: Vec<&str> = a.iter().map(|x| x["key"].as_str().unwrap()).collect();
        assert_eq!(
            keys,
            [
                "index.space_low",
                "archive.space_full",
                "archive.missing",
                "space.full",
                "archive.disk_wear"
            ]
        );
        assert_eq!(status_of(&a)["level"], "now");
        // Wear alone is "action needed" (spec block 10 disk mapping).
        let a = alerts(&json!({}), Some("wear"));
        assert_eq!(status_of(&a)["level"], "action");
        // A failing archive disk (alpd's own SMART check) is critical.
        let a = alerts(&st, Some("failing"));
        let s = status_of(&a);
        assert_eq!(
            (s["level"].as_str(), s["code"].as_str()),
            (Some("critical"), Some("ALP-STO-117"))
        );
        // Every error answer carries its level.
        assert_eq!(
            fail_json(&Fail::new("password.wrong", "x"))["level"],
            "note"
        );
    }

    #[test]
    fn disk_check_runs_as_a_job_and_is_logged() {
        let l = Arc::new(Live {
            check: |id, _, say| {
                assert_eq!(id, "sdy-3");
                say(&job::Update::step("checking", "Checking sdy1…", 50));
                Ok("sdy1 (EXFAT): no problems found.".into())
            },
            ..Arc::try_unwrap(live("check")).ok().unwrap()
        });
        assert_eq!(
            key(&call(&l, "POST", "disks/check", "{}")),
            (400, Some("request.json"))
        );
        assert_eq!(call(&l, "GET", "disks/check", "").0, 405);
        let (s, r) = call(&l, "POST", "disks/check", r#"{"disk_id": "sdy-3"}"#);
        assert_eq!((s, r["job"]["id"].as_str()), (202, Some("disk.check")));
        let t = Instant::now();
        let j = loop {
            let (_, st) = call(&l, "GET", "state", "");
            let j = st["archive"]["job"].clone();
            if j["state"] != "running" || t.elapsed() > Duration::from_secs(10) {
                break j;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(j["state"], "done");
        assert_eq!(j["message"], "sdy1 (EXFAT): no problems found.");
        let (_, ev) = call(&l, "GET", "events", "");
        let mine: Vec<&Value> = ev["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["kind"] == "disk.check")
            .collect();
        assert_eq!(
            mine.iter()
                .map(|e| e["result"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["started", "done"]
        );
        // Eject: a request without a disk is refused; in tests nothing is switched off.
        assert_eq!(
            key(&call(&l, "POST", "disks/eject", "{}")),
            (400, Some("request.json"))
        );
        assert_eq!(
            call(&l, "POST", "disks/eject", r#"{"disk_id": "sdy-3"}"#).0,
            200
        );
        assert_eq!(status_for("disk.open"), 409);
    }

    #[test]
    fn disk_prepare_checks_password_and_disk() {
        let l = live("prepare");
        let body = |pw: &str, id: &str| {
            format!(r#"{{"disk_id": "{id}", "partuuids": ["a"], "password": "{pw}"}}"#)
        };
        assert_eq!(
            key(&call(&l, "POST", "disks/prepare", &body("wrong", "sdz-1"))),
            (403, Some("password.wrong"))
        );
        // Not a disk id at all: never looked up, never erased.
        if archive::record().is_none() {
            assert_eq!(
                key(&call(
                    &l,
                    "POST",
                    "disks/prepare",
                    &body("right password", "../sda")
                )),
                (404, Some("disk.not_found"))
            );
        }
        assert_eq!(
            key(&call(
                &l,
                "POST",
                "disks/prepare",
                r#"{"disk_id": "sdz-1"}"#
            )),
            (400, Some("request.json"))
        );
        assert_eq!(call(&l, "GET", "disks/prepare", "").0, 405);
        assert_eq!(status_for("disk.changed"), 409);
        assert_eq!(status_for("disk.refused"), 422);
        // While a disk is prepared the archive is still "none".
        let mut j = Job::start("disk.prepare", "erasing", "Erasing the disk…", Some(30));
        j.state = "running";
        *lock(&l.archive_job) = Some(j);
        let (_, st) = call(&l, "GET", "state", "");
        assert_eq!(st["archive"]["job"]["id"], "disk.prepare");
        assert_eq!(st["archive"]["state"], "none");
        assert_eq!(st["archive"]["job"]["can_cancel"], false);
        let (s, e) = call(&l, "POST", "archive/cancel", "{}");
        assert_eq!((s, e["error"]["key"].as_str()), (409, Some("archive.busy")));
    }

    #[test]
    fn reset_needs_the_password_and_backs_off() {
        static WIPED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let l = live("reset");
        // No password, a wrong one, or a busy job: nothing is deleted
        // (`wipe` panics in this test unless replaced below).
        let (s, e) = call(&l, "POST", "reset", "{}");
        assert_eq!((s, e["error"]["key"].as_str()), (400, Some("request.json")));
        let (s, e) = call(&l, "POST", "reset", r#"{"password": "guess"}"#);
        assert_eq!(
            (s, e["error"]["key"].as_str()),
            (403, Some("password.wrong"))
        );
        let l = Arc::new(Live {
            wipe: |say| {
                say(&job::Update::step("erasing", "Erasing the stick… 50%", 55));
                WIPED.fetch_add(1, Ordering::SeqCst);
                Ok("the archive file was deleted".into())
            },
            ..Arc::try_unwrap(live("reset2")).ok().unwrap()
        });
        start_job(&l);
        let (s, e) = call(&l, "POST", "reset", r#"{"password": "right password"}"#);
        assert_eq!((s, e["error"]["key"].as_str()), (409, Some("archive.busy")));
        assert_eq!(WIPED.load(Ordering::SeqCst), 0);
        *lock(&l.archive_job) = None;
        let (s, r) = call(&l, "POST", "reset", r#"{"password": "right password"}"#);
        assert_eq!((s, r["job"]["id"].as_str()), (202, Some("reset")));
        // A job now (the wipe of an unprotected stick takes minutes).
        let t = Instant::now();
        while lock(&l.archive_job)
            .as_ref()
            .is_some_and(|j| j.state == "running")
            && t.elapsed() < Duration::from_secs(10)
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        {
            let g = lock(&l.archive_job);
            let j = g.as_ref().unwrap();
            assert_eq!(j.state, "done");
            assert_eq!(j.message, "Done: the archive file was deleted.");
        }
        assert_eq!(WIPED.load(Ordering::SeqCst), 1);
        // Wrong passwords: after five, wait (shared with sign-in and check).
        for _ in 0..5 {
            assert_eq!(
                call(&l, "POST", "password/check", r#"{"password": "nope"}"#).0,
                403
            );
        }
        let (s, e) = call(&l, "POST", "reset", r#"{"password": "right password"}"#);
        assert_eq!(
            (s, e["error"]["key"].as_str()),
            (429, Some("password.wait"))
        );
        assert!(e["error"]["retry_after_seconds"].as_u64().unwrap() > 0);
        assert_eq!(WIPED.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn password_check_does_not_sign_in() {
        let l = live("pwcheck");
        let r = l.handle(&Request {
            method: "POST".into(),
            path: format!("{PREFIX}password/check"),
            body: br#"{"password": "right password"}"#.to_vec(),
            ..Request::default()
        });
        assert_eq!(r.status, 200);
        assert!(
            r.headers
                .iter()
                .all(|(k, _)| !k.eq_ignore_ascii_case("set-cookie"))
        );
        assert!(lock(&l.console).token.is_none());
        assert_eq!(
            call(&l, "POST", "password/check", r#"{"password": "x"}"#).0,
            403
        );
        assert_eq!(call(&l, "GET", "password/check", "").0, 405);
    }

    #[test]
    fn prefs_are_checked_saved_and_shown() {
        let l = live("prefs");
        let _ = std::fs::remove_file(&l.prefs_file);
        let st = l.state(true);
        assert_eq!(st["prefs"]["ui_scale"], Value::Null);
        assert_eq!(st["prefs"]["home_background"], "blue");
        assert!(st["display"]["auto_scale"].is_number());
        let f = st["features"].as_array().unwrap();
        assert!(f.contains(&json!("prefs")) && f.contains(&json!("status_levels")));
        // The clock: a plausible time and whether the network set it.
        assert!(f.contains(&json!("clock")));
        assert!(st["clock"]["now"].as_u64().unwrap() > 1_700_000_000);
        assert!(st["clock"]["synced"].is_boolean());
        assert_eq!(st["prefs"]["time_zone"], Value::Null);
        let (s, r) = call(&l, "POST", "prefs", r#"{"time_zone": "Etc/GMT-2"}"#);
        assert_eq!((s, &r["prefs"]["time_zone"]), (200, &json!("Etc/GMT-2")));
        assert_eq!(
            key(&call(&l, "POST", "prefs", r#"{"time_zone": "../x"}"#)),
            (422, Some("prefs.invalid"))
        );
        let (s, r) = call(&l, "POST", "prefs", r#"{"ui_scale": 1.25}"#);
        assert_eq!((s, &r["prefs"]["ui_scale"]), (200, &json!(1.25)));
        assert_eq!(prefs::Prefs::load_from(&l.prefs_file).ui_scale, Some(125));
        assert_eq!(
            key(&call(&l, "POST", "prefs", r#"{"ui_scale": 1.3}"#)),
            (422, Some("prefs.invalid"))
        );
        assert_eq!(
            key(&call(&l, "POST", "prefs", r#"{"wallpaper": "https://x"}"#)),
            (422, Some("prefs.invalid"))
        );
        assert_eq!(l.state(true)["prefs"]["ui_scale"], 1.25);
        assert_eq!(call(&l, "GET", "prefs", "").0, 405);
        let _ = std::fs::remove_file(&l.prefs_file);
    }

    #[test]
    fn a_screen_mode_goes_back_unless_kept() {
        let l = live("display");
        lock(&RANDR_CALLS).clear();
        let (s, r) = call(&l, "GET", "display/modes", "");
        assert_eq!(s, 200);
        assert_eq!(r["modes"][0]["native"], true);
        assert_eq!(r["modes"][1]["width"], 1280);
        assert_eq!(
            key(&call(&l, "POST", "display/mode", r#"{"id": 9}"#)),
            (404, Some("display.not_found"))
        );
        assert_eq!(
            key(&call(&l, "POST", "display/mode/keep", "{}")),
            (409, Some("display.no_pending"))
        );
        // Not kept: back after the wait.
        let (s, r) = call(&l, "POST", "display/mode", r#"{"id": 1}"#);
        assert_eq!(s, 200);
        assert!(r["revert_in"].is_number());
        assert_eq!(
            key(&call(&l, "POST", "display/mode", r#"{"id": 0}"#)),
            (409, Some("display.busy"))
        );
        std::thread::sleep(Duration::from_millis(700));
        let calls: Vec<String> = lock(&RANDR_CALLS)
            .iter()
            .filter(|a| a.first().is_some_and(|x| x == "--output"))
            .map(|a| a[3].clone())
            .collect();
        assert_eq!(calls, ["1280x720@60.000Hz", "1920x1080@60.000Hz"]);
        assert!(lock(&l.display_pending).is_none());
        // Revert at once: back now, and the timer does nothing more.
        assert_eq!(call(&l, "POST", "display/mode", r#"{"id": 1}"#).0, 200);
        assert_eq!(call(&l, "POST", "display/mode/revert", "{}").0, 200);
        assert_eq!(
            key(&call(&l, "POST", "display/mode/revert", "{}")),
            (409, Some("display.no_pending"))
        );
        std::thread::sleep(Duration::from_millis(700));
        let sets: Vec<String> = lock(&RANDR_CALLS)
            .iter()
            .filter(|a| a.first().is_some_and(|x| x == "--output"))
            .map(|a| a[3].clone())
            .collect();
        assert_eq!(sets.len(), 4);
        assert_eq!(sets[3], "1920x1080@60.000Hz");
        // Kept: no going back.
        assert_eq!(call(&l, "POST", "display/mode", r#"{"id": 1}"#).0, 200);
        assert_eq!(call(&l, "POST", "display/mode/keep", "{}").0, 200);
        std::thread::sleep(Duration::from_millis(700));
        let sets = lock(&RANDR_CALLS)
            .iter()
            .filter(|a| a.first().is_some_and(|x| x == "--output"))
            .count();
        assert_eq!(sets, 5);
        let _ = std::fs::remove_file(&l.prefs_file);
    }

    #[test]
    fn the_terminal_locks_with_the_console_lock_after() {
        let l = live("term-idle");
        {
            let mut c = lock(&l.console);
            c.token = Some(zeroize::Zeroizing::new("t".into()));
            c.last = Instant::now().checked_sub(Duration::from_secs(400));
        }
        // Default 300 s: 400 s idle is locked.
        assert!(!l.console_fresh());
        // Lock after 15 minutes: still signed in.
        lock(&l.prefs).idle_lock_seconds = 900;
        assert!(l.console_fresh());
        // Lock after 1 minute.
        lock(&l.prefs).idle_lock_seconds = 60;
        assert!(!l.console_fresh());
        assert_eq!(l.idle_lock(), Duration::from_secs(60));
    }
}
