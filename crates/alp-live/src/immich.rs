// SPDX-License-Identifier: AGPL-3.0-or-later
//! Immich on ALP-OS Live (ADR 0015, docs/plans/live-immich.md, ADR 0011,
//! ADR 0014): preparing its folders, its state, the admin account (API key
//! only), the family account for phones, the device certificate.
//!
//! Immich runs rootless under the `immich` user (uid 50000, Quadlet pod in
//! the image) and only while the index and the archive are open: database
//! and thumbnails in the index on the stick, originals on the archive.
//! `alp-immich.service` runs `alpctl live immich prepare` and then starts
//! `user@50000.service`. alpd talks to Immich on 127.0.0.1:2283.

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::fail::Fail;
use crate::http;

pub const API: &str = "127.0.0.1:2283";
/// The `immich` user (image/extra/usr/lib/sysusers.d/alp-immich.conf); the
/// archive's exFAT/NTFS disk is mounted with this uid (ADR 0014).
pub const UID: u32 = 50000;
/// Immich's index data (database, thumbnails, videos, the user's podman
/// store) in the encrypted index on the stick (ADR 0011), owned by `UID`.
pub const DATA: &str = "/var/lib/alp-index/immich";
/// The `immich` user's home: `DATA` bind-mounted while Immich runs (the
/// Quadlet units use this path; `/var` itself is RAM on Live).
pub const HOME: &str = "/var/lib/immich";
/// The decrypted archive folder (gocryptfs, ADR 0014).
pub const ARCHIVE_FILES: &str = "/var/lib/alp-pool/files";
/// Immich's originals (upload, library, database dumps) on the archive
/// (`archive::immich_originals_dir()` in ADR 0014).
pub const ORIGINALS: &str = "/var/lib/alp-pool/files/immich";
/// Runtime folder: the database password file and the originals bind.
pub const RUN: &str = "/run/alp/immich";
/// `ORIGINALS` bind-mounted here (`/var/lib/alp-pool` is root-only).
pub const ORIGINALS_BIND: &str = "/run/alp/immich/originals";
/// The folders on the archive the server sees under `/data`.
pub const ORIGINAL_DIRS: [&str; 3] = ["upload", "library", "backups"];
/// alpd's own Immich secrets in the index (root, 0700).
pub const SECRETS: &str = "/var/lib/alp-index/alp/immich";
pub const ADMIN_EMAIL: &str = "admin@alparchive.local";
pub const FAMILY_EMAIL: &str = "family@alparchive.local";
/// System unit that starts Immich's user services (image/units).
pub const UNIT: &str = "alp-immich.service";
/// Progress of `alp-immich-load` (image/extra/usr/lib/alp), written by
/// the immich user into a file `prepare` makes for it (T21).
pub const LOAD_STATUS: &str = "/run/alp/immich/load-status";
/// The Immich web gallery for the kiosk only (loopback; T16).
pub const CONSOLE_URL: &str = "http://127.0.0.1:2283/";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Off,
    Starting,
    Ready,
    Failed(String),
}

pub fn ping() -> bool {
    http::request(API, "GET", "/api/server/ping", &[], None)
        .ok()
        .and_then(|r| r.json())
        .is_some_and(|v| v["res"] == "pong")
}

pub fn state() -> State {
    if !crate::index::is_open() || !crate::archive::is_open() {
        return State::Off;
    }
    if ping() {
        return State::Ready;
    }
    let is_failed = |args: &[&str]| {
        Command::new("systemctl")
            .args(args)
            .status()
            .is_ok_and(|s| s.success())
    };
    // The system unit, or one of the pod's units in the immich user's
    // manager after its restarts ran out (a failed image load leaves the
    // pod's units only inactive: look at the loader itself too).
    let failed = is_failed(&["is-failed", "--quiet", UNIT])
        || user_units_failed(&[
            "immich-images.service",
            "immich-server.service",
            "immich-postgres.service",
            "immich-valkey.service",
        ]);
    if failed {
        return State::Failed(
            "The photo service did not start. Restart the computer; if this stays, save a diagnosis for help."
                .into(),
        );
    }
    // A skipped start (its folders were not mounted) never fails: it
    // would look like "starting" for ever (T14).
    if unit_show().is_some_and(|o| condition_failed(&o)) {
        return State::Failed(
            "The photo service could not start because the archive was not ready. Restart the computer."
                .into(),
        );
    }
    State::Starting
}

/// Is one of `units` failed in the immich user's own manager? Asked as that
/// user (`setpriv`, no PAM; a timeout counts as not failed): `systemctl --machine=immich@` opens a PAM
/// "login" session, which the console's pam_access refuses, so the check
/// never worked and filled the journal (owner's laptop boot log, 0.2.2).
fn user_units_failed(units: &[&str]) -> bool {
    let uid = UID.to_string();
    // At most 3 s: this runs inside the console's state answer, and the
    // user manager can be slow to answer while it starts the pod (CI run
    // 40853549298: the state did not answer in 10 s).
    Command::new("timeout")
        .args([
            "3",
            "setpriv",
            "--reuid",
            &uid,
            "--regid",
            &uid,
            "--clear-groups",
            "--",
        ])
        .args(["env", &format!("XDG_RUNTIME_DIR=/run/user/{uid}")])
        .args(["systemctl", "--user", "is-failed", "--quiet"])
        .args(units)
        .status()
        .is_ok_and(|s| s.success())
}

fn unit_show() -> Option<String> {
    let o = Command::new("systemctl")
        .args([
            "show",
            "-p",
            "ActiveState",
            "-p",
            "ConditionResult",
            "-p",
            "ConditionTimestampMonotonic",
            UNIT,
        ])
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&o.stdout).into_owned())
}

fn show_value<'a>(show: &'a str, key: &str) -> Option<&'a str> {
    show.lines()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
}

/// `systemctl show` output: the unit was started, but a condition skipped it.
pub fn condition_failed(show: &str) -> bool {
    show_value(show, "ActiveState") == Some("inactive")
        && show_value(show, "ConditionResult") == Some("no")
        && show_value(show, "ConditionTimestampMonotonic")
            .is_some_and(|t| t != "0" && !t.is_empty())
}

/// `alp-immich.service` is not running and not starting (nobody started
/// it, e.g. alpd restarted, or it was stopped while the disk was full).
pub fn unit_inactive() -> bool {
    unit_show()
        .is_some_and(|o| show_value(&o, "ActiveState") == Some("inactive") && !condition_failed(&o))
}

/// What `alp-immich-load` reports (see `LOAD_STATUS`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Load {
    pub state: String,
    pub part: u64,
    pub parts: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

/// `key=value` lines; `None` without a `state`.
pub fn parse_load(text: &str) -> Option<Load> {
    let mut l = Load::default();
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let n = v.trim().parse().unwrap_or(0);
        match k.trim() {
            "state" => l.state = v.trim().to_string(),
            "part" => l.part = n,
            "parts" => l.parts = n,
            "bytes_done" => l.bytes_done = n,
            "bytes_total" => l.bytes_total = n,
            _ => {}
        }
    }
    (!l.state.is_empty()).then_some(l)
}

/// The sentence for a load in progress, `None` otherwise.
pub fn load_message(l: &Load) -> Option<String> {
    if l.state != "loading" {
        return None;
    }
    let gb = alp_hw::inspect::gb;
    let about = if l.bytes_total > 0 {
        format!("about {}, once after setup or an update", gb(l.bytes_total))
    } else {
        "once after setup or an update".into()
    };
    let mut m = format!("Unpacking the photo service ({about})");
    if l.parts > 0 {
        m += &format!(": part {} of {}", l.part.max(1), l.parts);
        if l.bytes_total > 0 {
            m += &format!(", {} of {} done", gb(l.bytes_done), gb(l.bytes_total));
        }
    }
    Some(m + "…")
}

/// Why Immich is still starting, in one plain sentence (T21).
pub fn detail(seconds_starting: u64) -> String {
    if let Some(m) = fs::read_to_string(LOAD_STATUS)
        .ok()
        .and_then(|t| parse_load(&t))
        .and_then(|l| load_message(&l))
    {
        return m;
    }
    if seconds_starting < 180 {
        "Starting the photo service…".into()
    } else {
        "Still starting: the first start sets up the photo database. This can take a few minutes more.".into()
    }
}

/// Phones and photos of the family account.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Family {
    /// Signed-in devices, without the console's own gallery (T33).
    pub devices: Vec<Device>,
    pub photos: u64,
}

/// One signed-in device: an Immich session of the family account.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Device {
    /// Immich's session id (for removing it).
    pub id: String,
    /// What Immich knows of it: the phone model or browser ("Pixel 8").
    pub name: String,
    /// "Android", "iOS", ...
    pub os: String,
    /// `phone`, `tablet` or `browser` (the console UI's icons).
    pub kind: &'static str,
    /// The Immich app's version; `None` for a browser.
    pub app_version: Option<String>,
    /// Last time Immich saw it (ISO 8601; Immich updates it about hourly).
    pub last_seen: String,
}

/// The devices among the family's Immich sessions (`GET
/// /admin/users/{id}/sessions`). Immich names a session from the request:
/// the app sends its model and OS, a browser its user agent. The console's
/// own gallery is Chromium on Linux, the only browser that can sign in
/// (port 443 serves no web UI), so Linux browser sessions are left out:
/// before T33 they made the console say "a phone is connected".
pub fn devices(sessions: &Value) -> Vec<Device> {
    let text = |v: &Value| v.as_str().unwrap_or("").trim().to_string();
    let mut v: Vec<Device> = sessions
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|x| {
            let os = text(&x["deviceOS"]);
            let app_version = Some(text(&x["appVersion"])).filter(|a| !a.is_empty());
            if app_version.is_none() && os.eq_ignore_ascii_case("linux") {
                return None;
            }
            let name = text(&x["deviceType"]);
            let low = format!("{name} {os}").to_lowercase();
            let kind = if app_version.is_none() {
                "browser"
            } else if low.contains("ipad") || low.contains("tablet") {
                "tablet"
            } else {
                "phone"
            };
            Some(Device {
                id: text(&x["id"]),
                name,
                os,
                kind,
                app_version,
                last_seen: text(&x["updatedAt"]),
            })
        })
        .collect();
    // Most recently seen first (ISO 8601 sorts as text).
    v.sort_by(|a, b| b.last_seen.cmp(&a.last_seen));
    v
}

/// Phone sessions not used for this long are removed (T63, security
/// review 2026-11-03, B3): a lost or sold phone does not stay signed in
/// for ever. Its owner signs in again with the ALP password.
pub const IDLE_SESSION_SECS: u64 = 60 * 24 * 3600;

/// Seconds since 1970 of an ISO 8601 UTC time as the photo service writes
/// it (`2026-10-28T10:00:00.000Z`); `None` for anything else.
fn unix_of_iso(t: &str) -> Option<u64> {
    let b = t.as_bytes();
    if b.len() < 20
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    if !t.ends_with('Z') {
        return None;
    }
    let n = |r: std::ops::Range<usize>| t.get(r)?.parse::<u64>().ok();
    let (y, m, d) = (n(0..4)?, n(5..7)?, n(8..10)?);
    let (h, mi, s) = (n(11..13)?, n(14..16)?, n(17..19)?);
    if y < 1970 || !(1..=12).contains(&m) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 60 {
        return None;
    }
    // Days from 1970-01-01 (the civil-date algorithm; March-based years).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2 / 400;
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = (era * 146_097 + doe).checked_sub(719_468)?;
    Some(days * 86_400 + h * 3600 + mi * 60 + s)
}

/// The ids of the sessions last seen more than `max_age` seconds before
/// `now` (unix seconds). A session without a readable time is kept.
pub fn idle_sessions(sessions: &Value, now: u64, max_age: u64) -> Vec<String> {
    sessions
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|x| {
            let seen = unix_of_iso(x["updatedAt"].as_str()?)?;
            let id = x["id"].as_str()?;
            (now.checked_sub(seen)? > max_age).then(|| id.to_string())
        })
        .collect()
}

/// Remove the family account's sessions idle for [`IDLE_SESSION_SECS`];
/// how many went. Each removal is written to the journal (never a token:
/// a session id is not a credential). The caller makes sure the clock can
/// be trusted: a clock that is months ahead would sign every phone out.
pub fn remove_idle_sessions(now: u64) -> Result<usize, Fail> {
    let k = Zeroizing::new(
        fs::read_to_string(key_path())
            .map_err(|_| failed("No phone is set up yet."))?
            .trim()
            .to_string(),
    );
    let Some(id) = family_id(&k)? else {
        return Ok(0);
    };
    let auth = api_key_header(&k);
    let sessions = call("GET", &format!("/admin/users/{id}/sessions"), &auth, None)?
        .json()
        .ok_or_else(|| failed("The photo service did not list the signed-in devices."))?;
    let mut gone = 0;
    for sid in idle_sessions(&sessions, now, IDLE_SESSION_SECS) {
        if remove_device(&sid).is_ok() {
            eprintln!("alpd: phone: a device not seen for 60 days was signed out (session {sid})");
            gone += 1;
        }
    }
    Ok(gone)
}

/// Addresses with an open connection to the phone app's port 443 now
/// (`ss -Htn state established '( sport = :443 )'`), sorted, without the
/// computer itself. Immich does not keep a device's address; while a phone
/// app is open it keeps a connection, so this shows which addresses use
/// ALPArchive at the moment (T33).
pub fn clients_now() -> Vec<String> {
    Command::new("ss")
        .args(["-Htn", "state", "established", "( sport = :443 )"])
        .output()
        .map(|o| clients_from_ss(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

/// The peer addresses in `ss -Htn state …` lines (Recv-Q Send-Q Local Peer).
pub fn clients_from_ss(text: &str) -> Vec<String> {
    let mut v: Vec<String> = text
        .lines()
        .filter_map(|l| l.split_whitespace().nth(3))
        .filter_map(|peer| peer.rsplit_once(':').map(|(a, _port)| a))
        .map(|a| a.trim_start_matches('[').trim_end_matches(']'))
        .map(|a| a.strip_prefix("::ffff:").unwrap_or(a).to_string())
        .filter(|a| a != "127.0.0.1" && a != "::1" && !a.is_empty())
        .collect();
    v.sort();
    v.dedup();
    v
}

/// Sign one device out (T33): it must sign in again with the ALP password.
pub fn remove_device(id: &str) -> Result<(), Fail> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        return Err(Fail::new(
            "phone.device_unknown",
            "This device is not known.",
        ));
    }
    as_family(|auth| {
        let r = call("DELETE", &format!("/sessions/{id}"), auth, None)?;
        match r.status {
            200..=299 => Ok(()),
            400 | 404 => Err(Fail::new(
                "phone.device_unknown",
                "This device is not known.",
            )),
            s => Err(failed(format!(
                "The photo service did not sign the device out (answer {s})."
            ))),
        }
    })
}

fn failed(msg: impl Into<String>) -> Fail {
    Fail::new("immich.failed", msg)
}

fn random_bytes<const N: usize>() -> Result<Zeroizing<[u8; N]>, Fail> {
    let mut b = Zeroizing::new([0u8; N]);
    fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b[..]))
        .map_err(|e| failed(format!("No random numbers: {e}")))?;
    Ok(b)
}

/// A long random password for the admin account nobody types (its
/// password is thrown away; only the API key is kept).
fn admin_password() -> Result<Zeroizing<String>, Fail> {
    let b = random_bytes::<24>()?;
    Ok(Zeroizing::new(
        b.iter().map(|x| format!("{x:02x}")).collect(),
    ))
}

fn key_path() -> PathBuf {
    Path::new(SECRETS).join("admin-api-key")
}

fn write_secret(path: &Path, value: &str) -> Result<(), Fail> {
    let io = |e: std::io::Error| failed(format!("Could not store a secret: {e}"));
    let dir = path.parent().unwrap_or(Path::new("/"));
    fs::create_dir_all(dir).map_err(io)?;
    fs::set_permissions(dir, std::os::unix::fs::PermissionsExt::from_mode(0o700)).map_err(io)?;
    let tmp = path.with_extension("tmp");
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(io)?;
    f.write_all(value.as_bytes())
        .and_then(|()| f.sync_all())
        .map_err(io)?;
    fs::rename(&tmp, path).map_err(io)
}

fn call(
    method: &str,
    path: &str,
    auth: &str,
    body: Option<&Value>,
) -> Result<http::Response, Fail> {
    let body = body.map(|b| b.to_string().into_bytes());
    let headers = if auth.is_empty() {
        vec![]
    } else {
        vec![auth.to_string()]
    };
    http::request(
        API,
        method,
        &format!("/api{path}"),
        &headers,
        body.as_deref(),
    )
    .map_err(|_| {
        Fail::new(
            "immich.starting",
            "The photo service is still starting. Please wait a moment.",
        )
    })
}

/// The console's gallery (security review 2026-11-01, section 2): the web
/// gallery on its own local origin, served by nginx on this port, which
/// adds the family account's key itself (after asking alpd, see
/// `gallery/gate`). The browser never holds a credential of the photo
/// service.
pub const GALLERY_URL: &str = "http://127.0.0.1:8412/";

/// The family account's key for the gallery proxy; `None` until the family
/// account exists. Only letters, digits and `-_+/=` are passed on (it goes
/// into a request header).
pub fn family_key() -> Option<Zeroizing<String>> {
    let k = Zeroizing::new(
        fs::read_to_string(family_key_path())
            .ok()?
            .trim()
            .to_string(),
    );
    header_safe(&k).then_some(k)
}

/// A key that can go into a request header as it is.
fn header_safe(k: &str) -> bool {
    !k.is_empty()
        && k.len() <= 128
        && k.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'+' | b'/' | b'='))
}

fn api_key_header(key: &str) -> String {
    format!("x-api-key: {key}")
}

/// The admin API key, creating the admin on first use. The admin's password
/// is random and thrown away: only the API key (on the archive) remains,
/// as in docs/contracts/api-access.md.
pub fn admin_key() -> Result<Zeroizing<String>, Fail> {
    if let Ok(k) = fs::read_to_string(key_path()) {
        let k = Zeroizing::new(k.trim().to_string());
        let r = call("GET", "/users/me", &api_key_header(&k), None)?;
        if r.status == 200 {
            return Ok(k);
        }
    }
    let password = admin_password()?;
    let r = call(
        "POST",
        "/auth/admin-sign-up",
        "",
        Some(&json!({"email": ADMIN_EMAIL, "name": "ALPArchive", "password": password.as_str()})),
    )?;
    if r.status != 201 {
        return Err(failed(
            "The photo service already has an owner that this stick does not know. Code for help: immich admin.",
        ));
    }
    let r = call(
        "POST",
        "/auth/login",
        "",
        Some(&json!({"email": ADMIN_EMAIL, "password": password.as_str()})),
    )?;
    let token = r
        .json()
        .and_then(|v| v["accessToken"].as_str().map(str::to_string))
        .map(Zeroizing::new)
        .ok_or_else(|| failed("The photo service did not let ALPArchive sign in."))?;
    let bearer = format!("Authorization: Bearer {}", token.as_str());
    let r = call(
        "POST",
        "/api-keys",
        &bearer,
        Some(&json!({"name": "alpd", "permissions": ["all"]})),
    )?;
    let key = r
        .json()
        .and_then(|v| v["secret"].as_str().map(str::to_string))
        .map(Zeroizing::new)
        .ok_or_else(|| failed("The photo service did not give ALPArchive a key."))?;
    write_secret(&key_path(), &key)?;
    let _ = call("POST", "/auth/logout", &bearer, Some(&json!({})));
    Ok(key)
}

fn family_id(key: &str) -> Result<Option<String>, Fail> {
    let r = call("GET", "/admin/users", &api_key_header(key), None)?;
    let users = r
        .json()
        .ok_or_else(|| failed("The photo service gave an unexpected answer."))?;
    Ok(users.as_array().and_then(|a| {
        a.iter()
            .find(|u| u["email"] == FAMILY_EMAIL)
            .and_then(|u| u["id"].as_str().map(str::to_string))
    }))
}

/// The family account's API key (alpd's own calls as the family: delete,
/// trash). The family password itself is never kept (T23).
fn family_key_path() -> PathBuf {
    Path::new(SECRETS).join("family-api-key")
}

/// Present once the family account's password is the main ALP password.
/// The family account's password must be set again (encrypt later, ADR
/// 0018: the phone password of an unprotected stick gives way to the ALP
/// password).
pub fn forget_family_password() {
    let _ = fs::remove_file(family_main_marker());
}

fn family_main_marker() -> PathBuf {
    Path::new(SECRETS).join("family-uses-main-password")
}

/// Before T23 the family had its own password, kept here; removed when
/// the main password takes over.
fn family_password_path() -> PathBuf {
    Path::new(SECRETS).join("family-password")
}

/// Phones sign in with the main ALP password (T23 done for this index).
pub fn family_uses_main_password() -> bool {
    family_main_marker().exists() && family_key_path().exists()
}

/// Sign in as `email`, run `f` with the bearer header, sign out.
fn signed_in<T>(email: &str, pw: &str, f: impl FnOnce(&str) -> Result<T, Fail>) -> Result<T, Fail> {
    let r = call(
        "POST",
        "/auth/login",
        "",
        Some(&json!({"email": email, "password": pw})),
    )?;
    let token = r
        .json()
        .and_then(|v| v["accessToken"].as_str().map(str::to_string))
        .map(Zeroizing::new)
        .ok_or_else(|| failed("The photo service did not let ALPArchive sign in."))?;
    let bearer = Zeroizing::new(format!("Authorization: Bearer {}", token.as_str()));
    let res = f(&bearer);
    let _ = call("POST", "/auth/logout", &bearer, Some(&json!({})));
    res
}

/// Phones sign in with the main ALP password (owner, T23): make the
/// family account, or give it `main` as its password. Then keep only a
/// key of the family account for alpd's own calls (older keys of alpd are
/// deleted), never the password. Phones signed in stay signed in.
/// `main` must already be checked against the index; it is never logged.
pub fn set_family_password(main: &str) -> Result<(), Fail> {
    let key = admin_key()?;
    let auth = api_key_header(&key);
    let r = match family_id(&key)? {
        Some(id) => call(
            "PUT",
            &format!("/admin/users/{id}"),
            &auth,
            Some(&json!({"password": main, "shouldChangePassword": false})),
        )?,
        None => call(
            "POST",
            "/admin/users",
            &auth,
            Some(&json!({
                "email": FAMILY_EMAIL, "name": "Family", "password": main,
                "shouldChangePassword": false, "notify": false
            })),
        )?,
    };
    if !(200..300).contains(&r.status) {
        return Err(failed(format!(
            "The photo service did not accept the family account (answer {}).",
            r.status
        )));
    }
    let family_key = signed_in(FAMILY_EMAIL, main, |bearer| {
        let old: Vec<String> = call("GET", "/api-keys", bearer, None)?
            .json()
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter(|k| k["name"] == FAMILY_KEY_NAME)
            .filter_map(|k| k["id"].as_str().map(str::to_string))
            .collect();
        for id in old {
            let _ = call("DELETE", &format!("/api-keys/{id}"), bearer, None);
        }
        call(
            "POST",
            "/api-keys",
            bearer,
            Some(&json!({"name": FAMILY_KEY_NAME, "permissions": ["all"]})),
        )?
        .json()
        .and_then(|v| v["secret"].as_str().map(str::to_string))
        .map(Zeroizing::new)
        .ok_or_else(|| failed("The photo service did not give ALPArchive a key."))
    })?;
    write_secret(&family_key_path(), &family_key)?;
    write_secret(&family_main_marker(), "1\n")?;
    let _ = fs::remove_file(family_password_path());
    Ok(())
}

const FAMILY_KEY_NAME: &str = "alpd-family";

/// Where the device's CA and nginx's certificate live: in the encrypted
/// index (ADR 0015), made by `/usr/lib/alp/alp-live-tls`.
pub const TLS_DIR: &str = "/var/lib/alp-index/alp/tls";
/// The certificate maker (image/extra/usr/lib/alp/alp-live-tls).
pub const TLS_TOOL: &str = "/usr/lib/alp/alp-live-tls";

/// SHA-256 fingerprint of the device's CA certificate (the one phones
/// install), `AB:CD:…`, if made.
pub fn certificate_sha256() -> Option<String> {
    fs::read_to_string(Path::new(TLS_DIR).join("ca.sha256"))
        .ok()
        .and_then(|s| fingerprint(&s))
}

/// A SHA-256 fingerprint as `openssl x509 -fingerprint` writes it
/// (32 colon-separated hex pairs), upper case; `None` for anything else.
pub fn fingerprint(s: &str) -> Option<String> {
    let s = s.trim();
    let s = s
        .rsplit_once('=')
        .map_or(s, |(_, v)| v)
        .to_ascii_uppercase();
    let pairs: Vec<&str> = s.split(':').collect();
    (pairs.len() == 32
        && pairs
            .iter()
            .all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit())))
    .then_some(s)
}

/// The LAN certificate must name every one of `addresses` (the phone
/// connects to them), and nginx must really send it: see [`crate::tls`].
pub fn certificate_for(addresses: &[String]) -> Result<(), crate::tls::NotServed> {
    crate::tls::ensure(addresses)
}

/// Run `f` as the family account (its photos are the gallery's): with
/// its API key, or (an index from before T23) by signing in with the old
/// family password.
fn as_family<T>(f: impl FnOnce(&str) -> Result<T, Fail>) -> Result<T, Fail> {
    if let Ok(k) = fs::read_to_string(family_key_path()) {
        let k = Zeroizing::new(k.trim().to_string());
        return f(&api_key_header(&k));
    }
    let pw = Zeroizing::new(
        fs::read_to_string(family_password_path())
            .map_err(|_| failed("No phone is set up yet."))?
            .trim()
            .to_string(),
    );
    signed_in(FAMILY_EMAIL, &pw, f)
}

/// Move the family's photos `ids` to Immich's trash (kept 30 days).
pub fn trash_assets(ids: &[String]) -> Result<usize, Fail> {
    as_family(|auth| {
        let r = call(
            "DELETE",
            "/assets",
            auth,
            Some(&json!({"ids": ids, "force": false})),
        )?;
        if (200..300).contains(&r.status) {
            Ok(ids.len())
        } else {
            Err(failed(format!(
                "The photo service did not delete them (answer {}).",
                r.status
            )))
        }
    })
}

/// Empty the family's trash for good.
pub fn empty_trash() -> Result<(), Fail> {
    as_family(|auth| {
        let r = call("POST", "/trash/empty", auth, Some(&json!({})))?;
        if (200..300).contains(&r.status) {
            Ok(())
        } else {
            Err(failed(format!(
                "The photo service did not empty the trash (answer {}).",
                r.status
            )))
        }
    })
}

/// Phones logged in and photos kept, for the status screen. `None` until
/// the family account exists.
pub fn family() -> Option<Family> {
    let k = Zeroizing::new(fs::read_to_string(key_path()).ok()?.trim().to_string());
    let id = family_id(&k).ok()??;
    let auth = api_key_header(&k);
    let sessions = call("GET", &format!("/admin/users/{id}/sessions"), &auth, None)
        .ok()?
        .json()?;
    let devices = devices(&sessions);
    let photos = call("GET", &format!("/admin/users/{id}/statistics"), &auth, None)
        .ok()
        .and_then(|r| r.json())
        .and_then(|v| v["total"].as_u64())
        .unwrap_or(0);
    Some(Family { devices, photos })
}

/// Mount points listed in `/proc/self/mountinfo` text (field 5, with the
/// kernel's octal escapes such as `\040` for a space undone).
pub fn mount_points(mountinfo: &str) -> Vec<String> {
    mountinfo
        .lines()
        .filter_map(|l| l.split_whitespace().nth(4))
        .map(unescape_mount)
        .collect()
}

fn unescape_mount(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\'
            && i + 3 < b.len()
            && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c))
        {
            out.push((b[i + 1] - b'0') * 64 + (b[i + 2] - b'0') * 8 + (b[i + 3] - b'0'));
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn is_mount_point(path: &str) -> bool {
    fs::read_to_string("/proc/self/mountinfo")
        .is_ok_and(|m| mount_points(&m).iter().any(|p| p == path))
}

/// The database password file the Quadlet units read (`EnvironmentFile=`):
/// Postgres takes `POSTGRES_PASSWORD` at its first start, the server
/// `DB_PASSWORD` every time; both are the same random hex string.
pub fn db_env(password: &str) -> String {
    format!("DB_PASSWORD={password}\nPOSTGRES_PASSWORD={password}\n")
}

/// A folder with this mode, owned by `uid:gid` when it is new (or always,
/// with `force_owner`). Existing folders keep the owner a container gave
/// them (Postgres and valkey change their own data folder's owner).
fn ensure_dir(path: &Path, mode: u32, uid: u32, gid: u32, force_owner: bool) -> Result<(), String> {
    let io = |e: std::io::Error| format!("{}: {e}", path.display());
    let new = !path.exists();
    if new {
        fs::create_dir_all(path).map_err(io)?;
    }
    if new || force_owner {
        let m = fs::metadata(path).map_err(io)?;
        if (m.uid(), m.gid()) != (uid, gid) {
            // exFAT and NTFS have no per-file owners: the archive disk is
            // mounted with uid=50000 (ADR 0014), so a refused chown there
            // is fine as long as the folder already shows that owner.
            if let Err(e) = std::os::unix::fs::chown(path, Some(uid), Some(gid))
                && m.uid() != uid
            {
                return Err(io(e));
            }
        }
        if fs::metadata(path).map_err(io)?.mode() & 0o7777 != mode {
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
        }
    }
    Ok(())
}

fn bind(src: &str, dst: &str) -> Result<(), String> {
    if is_mount_point(dst) {
        return Ok(());
    }
    let out = Command::new("mount")
        .args(["--bind", src, dst])
        .output()
        .map_err(|e| format!("mount: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "mount --bind {src} {dst}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

fn unbind(dst: &str) -> Result<(), String> {
    if !is_mount_point(dst) {
        return Ok(());
    }
    let ok = |args: &[&str]| {
        Command::new("umount")
            .args(args)
            .status()
            .is_ok_and(|s| s.success())
    };
    if ok(&[dst]) {
        return Ok(());
    }
    // Something still holds a file (Immich was killed a moment ago):
    // detach the bind; the archive's own close reports a real hold.
    if ok(&["--lazy", dst]) {
        eprintln!("ALP-OS Live: {dst} was busy; detached");
        return Ok(());
    }
    Err(format!("{dst} could not be unmounted"))
}

fn db_password() -> Result<Zeroizing<String>, String> {
    let path = Path::new(SECRETS).join("db-password");
    if let Ok(p) = fs::read_to_string(&path) {
        let p = Zeroizing::new(p.trim().to_string());
        if !p.is_empty() {
            return Ok(p);
        }
    }
    let pw = admin_password().map_err(|e| e.message)?;
    write_secret(&path, &pw).map_err(|e| e.message)?;
    Ok(pw)
}

/// Everything Immich needs before `user@50000.service` starts (run by
/// `alp-immich.service` as root): the index folders bound at the user's
/// home, the originals folders on the archive bound at `ORIGINALS_BIND`,
/// the database password file and the device certificate.
pub fn prepare() -> Result<(), String> {
    if !is_mount_point(crate::index::MNT) {
        return Err("the index is not open".into());
    }
    if !is_mount_point(ARCHIVE_FILES) {
        return Err("the archive is not open".into());
    }
    // Index: home, database, cache, thumbnails (btrfs, real owners).
    let data = Path::new(DATA);
    ensure_dir(data, 0o750, UID, UID, true)?;
    let pg = data.join("postgres");
    let pg_new = !pg.exists();
    ensure_dir(&pg, 0o700, UID, UID, false)?;
    if pg_new {
        // No copy-on-write for the database on btrfs (fewer writes on the
        // stick; Postgres has its own checksums). Best effort.
        let _ = Command::new("chattr").arg("+C").arg(&pg).output();
    }
    ensure_dir(&data.join("valkey"), 0o700, UID, UID, false)?;
    ensure_dir(&data.join("data"), 0o750, UID, UID, true)?;
    for d in ORIGINAL_DIRS {
        // Mount points for the archive's folders inside /data.
        ensure_dir(&data.join("data").join(d), 0o750, UID, UID, false)?;
    }
    ensure_dir(Path::new(HOME), 0o755, 0, 0, false)?;
    bind(DATA, HOME)?;
    // Archive: the originals, never anywhere else.
    let orig = Path::new(ORIGINALS);
    ensure_dir(orig, 0o700, UID, UID, true)?;
    for d in ORIGINAL_DIRS {
        ensure_dir(&orig.join(d), 0o700, UID, UID, true)?;
    }
    if !Path::new("/run/alp").exists() {
        fs::create_dir("/run/alp").map_err(|e| format!("/run/alp: {e}"))?;
        let _ = fs::set_permissions("/run/alp", fs::Permissions::from_mode(0o755));
    }
    ensure_dir(Path::new(RUN), 0o750, 0, UID, true)?;
    ensure_dir(Path::new(ORIGINALS_BIND), 0o700, UID, UID, false)?;
    // The image loader (immich user) may write its progress here (T21);
    // it cannot create files in RUN itself.
    let status = Path::new(LOAD_STATUS);
    if fs::write(status, "state=waiting\n").is_ok() {
        let _ = std::os::unix::fs::chown(status, Some(UID), Some(UID));
        let _ = fs::set_permissions(status, fs::Permissions::from_mode(0o644));
    }
    bind(ORIGINALS, ORIGINALS_BIND)?;
    // Database password: random once, kept in the index; the file the
    // units read is in RAM, readable only by the immich user.
    let pw = db_password()?;
    let env = Path::new(RUN).join("db.env");
    let tmp = env.with_extension("tmp");
    let io = |e: std::io::Error| format!("{}: {e}", env.display());
    let _ = fs::remove_file(&tmp);
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(io)?;
    f.write_all(db_env(&pw).as_bytes()).map_err(io)?;
    std::os::unix::fs::chown(&tmp, Some(UID), Some(UID)).map_err(io)?;
    fs::rename(&tmp, &env).map_err(io)?;
    // Device certificate for nginx (443) and the certificate page (80).
    let address = crate::live::main_address(&crate::live::ports());
    let mut tls = Command::new(TLS_TOOL);
    if let Some(a) = &address {
        tls.arg(a);
    }
    match tls.output() {
        Ok(o) if o.status.success() => {}
        // Immich still starts; pairing tries again (`certificate_for`).
        Ok(o) => eprintln!(
            "ALP-OS Live: certificate not made: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        ),
        Err(e) => eprintln!("ALP-OS Live: certificate not made: {e}"),
    }
    Ok(())
}

/// After `user@50000.service` stopped: undo the binds, forget the
/// database password file.
pub fn cleanup() -> Result<(), String> {
    let _ = fs::remove_file(Path::new(RUN).join("db.env"));
    let a = unbind(ORIGINALS_BIND);
    let b = unbind(HOME);
    a.and(b)
}

/// `alpctl live immich prepare | cleanup`.
pub fn cli(args: &[&str]) -> i32 {
    let r = match args {
        ["prepare"] => prepare(),
        ["cleanup"] => cleanup(),
        _ => Err("usage: alpctl live immich prepare | cleanup".into()),
    };
    match r {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("ALP-OS Live: Immich: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_a_plain_key_goes_into_a_header() {
        assert!(super::header_safe("AbC-12_3+/="));
        for bad in ["", "a b", "a\r\nx-api-key: z", "a;b", &"k".repeat(129)] {
            assert!(!super::header_safe(bad), "{bad:?}");
        }
    }

    #[test]
    fn sessions_idle_for_60_days_are_found() {
        assert_eq!(super::unix_of_iso("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(
            super::unix_of_iso("2026-11-14T08:15:55.000Z"),
            Some(1_794_644_155)
        );
        assert_eq!(
            super::unix_of_iso("2024-02-29T12:00:00Z"),
            Some(1_709_208_000)
        );
        for bad in [
            "",
            "yesterday",
            "2026-11-14 08:15:55",
            "2026-13-01T00:00:00Z",
            "2026-11-14T08:15:55+02:00",
        ] {
            assert_eq!(super::unix_of_iso(bad), None, "{bad}");
        }
        let now = 1_794_644_155;
        let day = 86_400;
        let iso = |secs_ago: u64| {
            // Built from the same function's inverse by search: whole days only.
            let t = now - secs_ago;
            let (d, r) = (t / day, t % day);
            let (mut y, mut left) = (1970, d);
            loop {
                let len = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
                    366
                } else {
                    365
                };
                if left < len {
                    break;
                }
                left -= len;
                y += 1;
            }
            let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
            let months = [
                31,
                if leap { 29 } else { 28 },
                31,
                30,
                31,
                30,
                31,
                31,
                30,
                31,
                30,
                31,
            ];
            let mut m = 0;
            while left >= months[m] {
                left -= months[m];
                m += 1;
            }
            format!(
                "{y:04}-{:02}-{:02}T{:02}:{:02}:{:02}.000Z",
                m + 1,
                left + 1,
                r / 3600,
                r % 3600 / 60,
                r % 60
            )
        };
        assert_eq!(super::unix_of_iso(&iso(61 * day)), Some(now - 61 * day));
        let s = serde_json::json!([
            {"id": "old", "updatedAt": iso(61 * day)},
            {"id": "edge", "updatedAt": iso(60 * day)},
            {"id": "new", "updatedAt": iso(3600)},
            {"id": "future", "updatedAt": "2099-01-01T00:00:00.000Z"},
            {"id": "odd", "updatedAt": "soon"},
            {"updatedAt": iso(90 * day)},
        ]);
        assert_eq!(
            super::idle_sessions(&s, now, super::IDLE_SESSION_SECS),
            vec!["old".to_string()]
        );
        // A clock that is behind (a session "from the future") removes nothing.
        assert!(super::idle_sessions(&s, 0, super::IDLE_SESSION_SECS).is_empty());
    }

    #[test]
    fn devices_leave_out_the_console() {
        let s = serde_json::json!([
            {"id": "a1", "deviceType": "Chrome", "deviceOS": "Linux", "appVersion": null, "updatedAt": "2026-10-28T10:00:00.000Z"},
            {"id": "b2", "deviceType": "Pixel 8", "deviceOS": "Android", "appVersion": "3.3.3", "updatedAt": "2026-10-28T09:00:00.000Z"},
            {"id": "c3", "deviceType": "iPad", "deviceOS": "iOS", "appVersion": "3.3.3", "updatedAt": "2026-10-28T11:00:00.000Z"},
            {"id": "d4", "deviceType": "Safari", "deviceOS": "iOS", "appVersion": "", "updatedAt": "2026-10-27T11:00:00.000Z"}
        ]);
        let d = super::devices(&s);
        let ids: Vec<&str> = d.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, ["c3", "b2", "d4"]);
        assert_eq!(
            d.iter().map(|x| x.kind).collect::<Vec<_>>(),
            ["tablet", "phone", "browser"]
        );
        assert_eq!(d[1].name, "Pixel 8");
        assert!(super::devices(&serde_json::json!(null)).is_empty());
    }

    #[test]
    fn clients_from_ss_lines() {
        let t = "0      0      192.168.1.196:443   192.168.1.50:51234\n\
                 0      0      192.168.1.196:443   192.168.1.50:51240\n\
                 0      0      [::ffff:192.168.1.196]:443 [::ffff:192.168.1.7]:40000\n\
                 0      0      127.0.0.1:443       127.0.0.1:5555\n";
        assert_eq!(super::clients_from_ss(t), ["192.168.1.50", "192.168.1.7"]);
    }

    #[test]
    fn admin_passwords_are_long() {
        assert_eq!(super::admin_password().unwrap().len(), 48);
    }

    #[test]
    fn load_progress_is_told_plainly() {
        use super::{load_message, parse_load};
        let l = parse_load(
            "state=loading\npart=2\nparts=3\nbytes_done=1100000000\nbytes_total=2500000000\n",
        )
        .unwrap();
        assert_eq!(
            load_message(&l).unwrap(),
            "Unpacking the photo service (about 2.5 GB, once after setup or an update): part 2 of 3, 1.1 GB of 2.5 GB done…"
        );
        let l = parse_load("state=loading\npart=1\nparts=3\n").unwrap();
        assert_eq!(
            load_message(&l).unwrap(),
            "Unpacking the photo service (once after setup or an update): part 1 of 3…"
        );
        assert_eq!(load_message(&parse_load("state=done\n").unwrap()), None);
        assert!(parse_load("").is_none());
        assert!(parse_load("garbage").is_none());
    }

    #[test]
    fn skipped_unit_is_seen() {
        use super::condition_failed;
        assert!(condition_failed(
            "ActiveState=inactive\nConditionResult=no\nConditionTimestampMonotonic=123\n"
        ));
        // Never started: no condition checked yet.
        assert!(!condition_failed(
            "ActiveState=inactive\nConditionResult=no\nConditionTimestampMonotonic=0\n"
        ));
        assert!(!condition_failed(
            "ActiveState=active\nConditionResult=yes\nConditionTimestampMonotonic=5\n"
        ));
    }

    #[test]
    fn mountinfo_points() {
        let m = "36 35 98:0 / /var/lib/alp-index rw,noatime - btrfs /dev/mapper/alp-index rw\n\
                 40 36 0:55 / /var/lib/alp-pool/files rw - fuse.gocryptfs /run/alp/host rw\n\
                 41 36 0:56 / /mnt/My\\040Disk rw - exfat /dev/sdb1 rw\n";
        let p = super::mount_points(m);
        assert_eq!(
            p,
            [
                "/var/lib/alp-index",
                "/var/lib/alp-pool/files",
                "/mnt/My Disk"
            ]
        );
        assert_eq!(super::unescape_mount("a\\134b\\0"), "a\\b\\0");
    }

    #[test]
    fn db_env_has_both_names() {
        assert_eq!(
            super::db_env("ab12"),
            "DB_PASSWORD=ab12\nPOSTGRES_PASSWORD=ab12\n"
        );
    }

    #[test]
    fn fingerprints() {
        let hex = vec!["ab"; 32].join(":");
        let want = hex.to_ascii_uppercase();
        assert_eq!(super::fingerprint(&hex).as_deref(), Some(want.as_str()));
        let line = format!("sha256 Fingerprint={want}\n");
        assert_eq!(super::fingerprint(&line).as_deref(), Some(want.as_str()));
        assert_eq!(super::fingerprint("AB:CD"), None);
        assert_eq!(super::fingerprint(""), None);
        assert_eq!(super::fingerprint(&vec!["zz"; 32].join(":")), None);
    }

    #[test]
    fn paths_follow_storage_v2() {
        assert_eq!(super::ORIGINALS, "/var/lib/alp-pool/files/immich");
        assert!(super::ORIGINALS.starts_with(super::ARCHIVE_FILES));
        assert!(super::ORIGINALS_BIND.starts_with(super::RUN));
        assert!(super::DATA.starts_with(crate::index::MNT));
        assert!(super::TLS_DIR.starts_with(crate::index::MNT));
    }
}
