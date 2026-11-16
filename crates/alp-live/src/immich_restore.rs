// SPDX-License-Identifier: AGPL-3.0-or-later
//! The gallery comes back when a backup disk is opened with a new stick
//! (alpha 0.2.7, ADR 0025).
//!
//! The photos are on the backup disk; the gallery's database (which photo
//! is which, albums, the phone's "already saved" list) is in the index on
//! the stick. A stick that was written again has an empty index: the
//! photos were all there, but the gallery was empty and the phone sent
//! everything again. Immich keeps backups of its database beside the
//! photos (`backups/immich-db-backup-….sql.gz`) and can put one back
//! itself; this module only asks it to, with Immich's own calls:
//!
//! 1. [`mark_if_backup`] (at the photo service's start, new index only):
//!    a backup exists on the archive, so a restore is pending;
//! 2. [`tick`] (alpd, once the photo service answers): start Immich's
//!    restore flow, restore the newest backup, wait for it;
//! 3. the restored gallery has an owner whose key was on the old stick:
//!    Immich's own `immich-admin reset-admin-password` gives it a new
//!    random password, with which alpd makes its key as on a first start.
//!
//! While a restore is pending the photo service counts as starting, and
//! [`crate::immich::admin_key`] refuses: nobody may set up the empty
//! gallery in between. A restore that fails is tried again (three times);
//! then the gallery starts empty, as before this module. The backup it
//! started from is kept aside (`backups/alp-kept/`), where Immich's own
//! rotation of old backups cannot remove it. The photos are never touched.
//!
//! [`backup_tick`] and [`backup_now`] ask Immich for a new backup after
//! photos came in and before the computer turns off (Immich's own timer
//! runs at night, when a Live computer is usually off).

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::fail::Fail;
use crate::immich::{self, SECRETS, UID};

/// The cookie Immich's restore flow signs its caller in with.
const COOKIE: &str = "immich_maintenance_token";
/// Longest wait for the photo service to change mode (it restarts itself).
const MODE_WAIT: Duration = Duration::from_secs(300);
/// Longest wait for the restore itself.
const RESTORE_WAIT: Duration = Duration::from_secs(45 * 60);
/// A new backup is asked for at most this often.
const BACKUP_EVERY: Duration = Duration::from_secs(10 * 60);

fn mark_path() -> PathBuf {
    Path::new(SECRETS).join("restore-pending")
}

/// The number of photos and videos at the last backup alpd asked for.
fn count_path() -> PathBuf {
    Path::new(SECRETS).join("backup-count")
}

fn backups_dir() -> PathBuf {
    Path::new(immich::ORIGINALS).join("backups")
}

/// The backup a restore starts from is copied here first. Immich keeps
/// only its newest backups in `backups/`; a gallery that came up empty
/// after a failed restore would push the real one out within hours. This
/// folder is not Immich's, so nothing rotates it.
fn kept_dir() -> PathBuf {
    backups_dir().join("alp-kept")
}

/// A real folder (not a link someone put on the disk), made if missing.
fn real_dir(dir: &Path) -> bool {
    match fs::symlink_metadata(dir) {
        Ok(m) => m.file_type().is_dir(),
        Err(_) => fs::create_dir(dir).is_ok(),
    }
}

/// A real, non-empty file (not a link).
fn real_file(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file() && m.len() > 0)
}

/// Copy `name` from one folder to the other, through a temporary name.
fn copy_backup(from: &Path, to: &Path, name: &str) -> bool {
    if !backup_name_ok(name) || !real_file(&from.join(name)) || !real_dir(to) {
        return false;
    }
    let tmp = to.join(format!(".{name}.alp-tmp"));
    let _ = fs::remove_file(&tmp);
    let ok = fs::copy(from.join(name), &tmp).is_ok() && fs::rename(&tmp, to.join(name)).is_ok();
    if ok {
        // The photo service reads its backups as its own user. A disk
        // without owners (exFAT) refuses this and does not need it.
        let _ = std::os::unix::fs::chown(to.join(name), Some(UID), Some(UID));
    } else {
        let _ = fs::remove_file(&tmp);
    }
    ok
}

/// Keep `name` outside Immich's rotation; older kept copies go.
fn keep_copy(name: &str) {
    let kept = kept_dir();
    if real_file(&kept.join(name)) {
        return;
    }
    if !copy_backup(&backups_dir(), &kept, name) {
        eprintln!("ALP-OS Live: gallery: the backup could not be kept aside");
        return;
    }
    for e in fs::read_dir(&kept).into_iter().flatten().flatten() {
        let other = e.file_name();
        if other
            .to_str()
            .is_some_and(|o| o != name && backup_name_ok(o))
        {
            let _ = fs::remove_file(e.path());
        }
    }
}

/// The backup the pending restore is for (noted in the marker), in
/// `backups/` again if Immich rotated it away since.
fn marked_backup() -> Option<String> {
    let name = fs::read_to_string(mark_path())
        .ok()?
        .lines()
        .next()?
        .trim()
        .to_string();
    if !backup_name_ok(&name) {
        return None;
    }
    if !real_file(&backups_dir().join(&name)) && !copy_backup(&kept_dir(), &backups_dir(), &name) {
        return None;
    }
    Some(name)
}

/// Is a restore waiting or running?
pub fn pending() -> bool {
    mark_path().exists()
}

/// Is the restore waiting for the person's answer? An archive that is not
/// encrypted could carry a backup file someone else put there, and a
/// backup is read as the database's administrator: the person is asked
/// first (owner's decision, 2026-11-16).
pub fn asking() -> bool {
    fs::read_to_string(mark_path()).is_ok_and(|m| marker_asks(&m))
}

/// The marker: the backup's name, and a second line `ask` while the
/// person has not answered.
fn marker_asks(marker: &str) -> bool {
    marker.lines().nth(1).is_some_and(|l| l.trim() == "ask")
}

/// The person's answer. `true`: put the gallery back from the backup.
/// `false`: start with an empty gallery (the backup stays on the disk, in
/// `backups/alp-kept/`).
pub fn answer(restore: bool) -> Result<(), Fail> {
    let marker = fs::read_to_string(mark_path()).unwrap_or_default();
    if !marker_asks(&marker) {
        return Err(Fail::new(
            "restore.not_asked",
            "There is no gallery waiting to be brought back.",
        ));
    }
    if restore {
        let name = marker.lines().next().unwrap_or("").trim().to_string();
        immich::write_secret(&mark_path(), &format!("{name}\n"))
    } else {
        eprintln!("alpd: gallery: not brought back (the person's choice); starting empty");
        fs::remove_file(mark_path())
            .map_err(|e| immich::failed(format!("the choice could not be kept: {e}")))
    }
}

/// A database backup's name as Immich writes it
/// (`immich-db-backup-20261115T020000-v3.3.3-pg14.19.sql.gz`,
/// server/src/utils/database-backups.ts), and nothing that could leave the
/// folder or break a JSON string.
pub fn backup_name_ok(name: &str) -> bool {
    let Some(rest) = name
        .strip_prefix("immich-db-backup-")
        .and_then(|r| r.strip_suffix(".sql.gz"))
    else {
        return false;
    };
    let b = rest.as_bytes();
    name.len() <= 128
        && b.len() > 16
        && b[..8].iter().all(u8::is_ascii_digit)
        && b[8] == b'T'
        && b[9..15].iter().all(u8::is_ascii_digit)
        && rest[15..].starts_with("-v")
        && rest.contains("-pg")
        && b.iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-'))
}

/// The newest backup in `dir`: the name carries its time. Empty files and
/// other names (unfinished `.tmp`, uploads) do not count.
pub fn newest_backup(dir: &Path) -> Option<String> {
    fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.metadata().is_ok_and(|m| m.is_file() && m.len() > 0))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| backup_name_ok(n))
        .max()
}

/// At the photo service's start with a new index: a backup on the archive
/// means there is a gallery to bring back.
pub fn mark_if_backup() {
    let Some(name) = newest_backup(&backups_dir()) else {
        return;
    };
    keep_copy(&name);
    let ask = if crate::archive::open_is_plain() {
        "ask\n"
    } else {
        ""
    };
    match immich::write_secret(&mark_path(), &format!("{name}\n{ask}")) {
        Ok(()) => eprintln!("ALP-OS Live: gallery: a backup is on the disk; it will be put back"),
        Err(e) => eprintln!("ALP-OS Live: gallery: restore not noted: {}", e.message),
    }
}

static RUNNING: AtomicBool = AtomicBool::new(false);
/// Tries that failed since alpd started, and when the last one ended.
static TRIES: Mutex<(u8, Option<Instant>)> = Mutex::new((0, None));
/// A restore that failed is tried again, this often at most.
const MAX_TRIES: u8 = 3;
const TRY_AGAIN_AFTER: Duration = Duration::from_secs(60);

/// Called by alpd every few seconds: run the pending restore as soon as
/// the photo service answers. A restore that failed is tried again, up to
/// [`MAX_TRIES`] times; then the gallery starts empty, and the backup it
/// would have come from stays in [`kept_dir`].
pub fn tick() {
    if !pending()
        || asking()
        || !crate::index::is_open()
        || !crate::archive::is_open()
        || RUNNING.load(Ordering::SeqCst)
        || TRIES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .1
            .is_some_and(|t| t.elapsed() < TRY_AGAIN_AFTER)
        || !immich::ping()
        || RUNNING.swap(true, Ordering::SeqCst)
    {
        return;
    }
    std::thread::spawn(|| {
        eprintln!("alpd: gallery: bringing it back from the backup disk");
        match run() {
            Ok(name) => {
                eprintln!("alpd: gallery: back ({name})");
                let _ = fs::remove_file(mark_path());
            }
            Err(e) => {
                let mut tries = TRIES
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                tries.0 = tries.0.saturating_add(1);
                tries.1 = Some(Instant::now());
                eprintln!(
                    "alpd: gallery: not brought back (try {} of {MAX_TRIES}): {}",
                    tries.0, e.message
                );
                if tries.0 >= MAX_TRIES {
                    // Not for ever: the gallery starts empty. The backup
                    // stays on the disk, outside Immich's rotation.
                    let _ = fs::remove_file(mark_path());
                }
            }
        }
        RUNNING.store(false, Ordering::SeqCst);
    });
}

/// Does the photo service say it has an owner? `None`: no clear answer.
fn has_owner() -> Option<bool> {
    immich::call("GET", "/server/config", "", None)
        .ok()
        .filter(|r| r.status == 200)?
        .json()?["isInitialized"]
        .as_bool()
}

fn status(auth: &str) -> Option<Value> {
    immich::call("GET", "/admin/maintenance/status", auth, None)
        .ok()
        .filter(|r| r.status == 200)?
        .json()
}

fn wait_for(limit: Duration, mut f: impl FnMut() -> bool) -> bool {
    let t = Instant::now();
    while t.elapsed() < limit {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_secs(3));
    }
    false
}

/// What a status answer of the restore means.
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    Running,
    Done,
    Failed,
}

/// `active` false: Immich left its maintenance mode, which it does by
/// itself only after a restore that worked. An `error`, or the task
/// `error`: it stays in maintenance mode and waits to be told to end it.
pub fn step_of(status: &Value) -> Step {
    if status["error"].as_str().is_some_and(|e| !e.is_empty()) || status["task"] == "error" {
        Step::Failed
    } else if status["active"] == json!(false) {
        Step::Done
    } else {
        Step::Running
    }
}

fn run() -> Result<String, Fail> {
    let name =
        marked_backup().ok_or_else(|| immich::failed("no backup of the gallery on the disk"))?;
    let r = immich::call(
        "POST",
        "/admin/database-backups/start-restore",
        "",
        Some(&json!({})),
    )?;
    if !(200..300).contains(&r.status) {
        // Immich allows this only while nobody set it up. Only when it
        // says itself that it has an owner (an earlier restore was cut off
        // after the database was back) is the gallery taken over; any
        // other refusal (still starting, an error) is tried again.
        if has_owner() != Some(true) {
            return Err(immich::failed(format!(
                "the photo service did not start the restore (answer {})",
                r.status
            )));
        }
        eprintln!(
            "alpd: gallery: the photo service is set up already (answer {}); taking it over",
            r.status
        );
        adopt()?;
        return Ok(name);
    }
    let token = r
        .cookie(COOKIE)
        .filter(|t| {
            !t.is_empty()
                && t.len() <= 2048
                && t.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        })
        .map(|t| Zeroizing::new(format!("Cookie: {COOKIE}={t}")))
        .ok_or_else(|| immich::failed("the photo service gave no restore sign-in"))?;
    let auth = token.as_str();
    if !wait_for(MODE_WAIT, || {
        status(auth).is_some_and(|s| s["active"] == json!(true))
    }) {
        return Err(immich::failed(
            "the photo service did not get ready for the restore",
        ));
    }
    let r = immich::call(
        "POST",
        "/admin/maintenance",
        auth,
        Some(&json!({"action": "restore_database", "restoreBackupFilename": name})),
    )?;
    if !(200..300).contains(&r.status) {
        end(auth);
        return Err(immich::failed(format!(
            "the photo service refused the restore (answer {})",
            r.status
        )));
    }
    let mut last = Step::Running;
    // No answer: it is restarting, or busy; ask again.
    wait_for(RESTORE_WAIT, || {
        last = status(auth).map_or(Step::Running, |s| step_of(&s));
        last != Step::Running
    });
    if last != Step::Done {
        end(auth);
        return Err(immich::failed(if last == Step::Failed {
            "the photo service could not read the backup"
        } else {
            "the restore took too long"
        }));
    }
    if !wait_for(MODE_WAIT, immich::ping) {
        return Err(immich::failed(
            "the photo service did not come back after the restore",
        ));
    }
    adopt()?;
    Ok(name)
}

/// Leave Immich's maintenance mode after a restore that did not work.
fn end(auth: &str) {
    let _ = immich::call(
        "POST",
        "/admin/maintenance",
        auth,
        Some(&json!({"action": "end"})),
    );
    wait_for(MODE_WAIT, || {
        status("").is_some_and(|s| s["active"] == json!(false))
    });
}

/// The restored gallery's owner gets a new random password, alpd its key;
/// then the small pictures (they were in the old index) are made again.
fn adopt() -> Result<(), Fail> {
    let password = immich::admin_password()?;
    reset_admin_password(&password)?;
    let mut key = immich::key_as_admin(&password);
    for _ in 0..20 {
        if key.is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_secs(6));
        key = immich::key_as_admin(&password);
    }
    let auth = immich::api_key_header(&key?);
    for queue in ["thumbnailGeneration", "videoConversion"] {
        let r = immich::call(
            "PUT",
            // `PUT /queues/<name>` only pauses a queue; this one runs it
            // (job.controller.ts, v3.3.3).
            &format!("/jobs/{queue}"),
            &auth,
            // All of them: the database still lists the old files.
            Some(&json!({"command": "start", "force": true})),
        );
        if !r.is_ok_and(|r| (200..300).contains(&r.status)) {
            eprintln!("alpd: gallery: {queue} not started; Immich does it at night");
        }
    }
    Ok(())
}

/// `immich-admin reset-admin-password` in the server's container, as the
/// immich user (its own podman). The command asks two questions on its
/// input: the new password, and whether to sign the owner's sessions out.
/// The password goes through the pipe only, never an argument; the output
/// (which repeats it) is dropped.
fn reset_admin_password(password: &str) -> Result<(), Fail> {
    let uid = UID.to_string();
    let no = |what: &str| immich::failed(format!("the gallery's owner could not be reset: {what}"));
    let mut child = Command::new("timeout")
        .args(["180", "setpriv", "--reuid", &uid, "--regid", &uid])
        .args(["--clear-groups", "--"])
        .args(["env", &format!("XDG_RUNTIME_DIR=/run/user/{uid}")])
        .arg(format!("HOME={}", immich::HOME))
        .args(["podman", "exec", "-i", "immich-server"])
        .args(["immich-admin", "reset-admin-password"])
        .current_dir("/")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| no(&e.to_string()))?;
    let mut stdin = child.stdin.take().ok_or_else(|| no("no input"))?;
    let mut stdout = child.stdout.take().ok_or_else(|| no("no output"))?;
    let seen = Arc::new(Mutex::new(Zeroizing::new(Vec::<u8>::new())));
    let sink = Arc::clone(&seen);
    let reader = std::thread::spawn(move || {
        let mut buf = Zeroizing::new([0u8; 1024]);
        while let Ok(n) = stdout.read(&mut buf[..]) {
            if n == 0 {
                break;
            }
            sink.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .extend_from_slice(&buf[..n]);
        }
    });
    let has = |word: &str| {
        let g = seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        g.windows(word.len()).any(|w| w == word.as_bytes())
    };
    let wait_word = |word: &str, secs: u64| {
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(secs) {
            if has(word) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        false
    };
    // Each answer only after its question: a line sent early is lost.
    let mut asked = wait_word("new password", 120);
    if asked {
        let line = Zeroizing::new(format!("{password}\n"));
        asked = stdin.write_all(line.as_bytes()).is_ok() && stdin.flush().is_ok();
    }
    if asked && wait_word("Invalidate", 60) {
        let _ = stdin.write_all(b"y\n").and_then(|()| stdin.flush());
    }
    let done = wait_word("has been updated", 60);
    drop(stdin);
    let _ = child.wait();
    let _ = reader.join();
    if done {
        Ok(())
    } else if !asked {
        Err(no("the tool did not ask for a password"))
    } else {
        Err(no("the tool did not finish"))
    }
}

/// alpd's admin key as kept in the index; never makes one.
fn kept_key() -> Option<Zeroizing<String>> {
    let k = Zeroizing::new(
        fs::read_to_string(Path::new(SECRETS).join("admin-api-key"))
            .ok()?
            .trim()
            .to_string(),
    );
    (!k.is_empty()).then_some(k)
}

/// Photos and videos in the gallery, all accounts.
fn count(auth: &str) -> Option<u64> {
    let v = immich::call("GET", "/server/statistics", auth, None)
        .ok()
        .filter(|r| r.status == 200)?
        .json()?;
    Some(v["photos"].as_u64()? + v["videos"].as_u64().unwrap_or(0))
}

fn kept_count() -> Option<u64> {
    fs::read_to_string(count_path()).ok()?.trim().parse().ok()
}

/// Is a new backup due: the gallery holds something, and not what it held
/// at the last backup alpd asked for?
pub fn backup_due(now: u64, kept: Option<u64>) -> bool {
    now > 0 && kept != Some(now)
}

/// Ask Immich for a database backup now (its own job, as from its
/// administration page). `true`: asked.
fn ask_backup(auth: &str, n: u64) -> bool {
    let ok = immich::call(
        "POST",
        "/jobs",
        auth,
        Some(&json!({"name": "backup-database"})),
    )
    .is_ok_and(|r| (200..300).contains(&r.status));
    if ok {
        let _ = immich::write_secret(&count_path(), &format!("{n}\n"));
    }
    ok
}

struct Asked {
    checked: Option<Instant>,
    asked: Option<Instant>,
    /// The count at the look before this one.
    seen: Option<u64>,
}

static ASKED: Mutex<Asked> = Mutex::new(Asked {
    checked: None,
    asked: None,
    seen: None,
});

/// Called by alpd every few seconds; looks once a minute. After photos or
/// videos came in (or went) and the count stood still for a minute (not in
/// the middle of a phone's backup), ask for a new backup, at most every
/// [`BACKUP_EVERY`].
pub fn backup_tick() {
    {
        let mut a = ASKED
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if a.checked
            .is_some_and(|t| t.elapsed() < Duration::from_secs(60))
        {
            return;
        }
        a.checked = Some(Instant::now());
        if a.asked.is_some_and(|t| t.elapsed() < BACKUP_EVERY) {
            return;
        }
    }
    if pending() || !crate::index::is_open() || !crate::archive::is_open() {
        return;
    }
    let Some(key) = kept_key() else { return };
    let auth = immich::api_key_header(&key);
    let Some(n) = count(&auth) else { return };
    let still = {
        let mut a = ASKED
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        a.seen.replace(n) == Some(n)
    };
    if still && backup_due(n, kept_count()) && ask_backup(&auth, n) {
        eprintln!("alpd: gallery: backup of the gallery's database asked for");
        ASKED
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .asked = Some(Instant::now());
    }
}

/// Before the photo service is stopped for turning off: if the gallery
/// changed since the last backup, ask for one and wait up to `wait` for
/// its file. `true`: nothing to do, or the new backup is there.
pub fn backup_now(wait: Duration) -> bool {
    if pending() || !crate::index::is_open() || !crate::archive::is_open() {
        return true;
    }
    let Some(key) = kept_key() else { return true };
    let auth = immich::api_key_header(&key);
    let Some(n) = count(&auth) else { return true };
    if !backup_due(n, kept_count()) {
        return true;
    }
    let before = newest_backup(&backups_dir());
    if !ask_backup(&auth, n) {
        return false;
    }
    let t = Instant::now();
    while t.elapsed() < wait {
        if newest_backup(&backups_dir()) != before {
            return true;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_names_are_immichs_and_nothing_else() {
        for ok in [
            "immich-db-backup-20261115T020000-v3.3.3-pg14.19.sql.gz",
            "immich-db-backup-20250729T114018-v1.136.0-pg14.17.sql.gz",
        ] {
            assert!(backup_name_ok(ok), "{ok}");
        }
        for bad in [
            "",
            "immich-db-backup-1700000000000.sql.gz",
            "immich-db-backup-20261115T020000-v3.3.3-pg14.19.sql.gz.tmp",
            "uploaded-immich-db-backup-20261115T020000-v3.3.3-pg14.19.sql.gz",
            "immich-db-backup-20261115T020000-v3/../x-pg14.sql.gz",
            "immich-db-backup-20261115T020000-v3\"-pg14.sql.gz",
            "immich-db-backup-2026111XT020000-v3.3.3-pg14.19.sql.gz",
            "immich-db-backup-20261115T020000-v3.3.3.sql.gz",
        ] {
            assert!(!backup_name_ok(bad), "{bad}");
        }
        let long = format!(
            "immich-db-backup-20261115T020000-v{}-pg14.sql.gz",
            "1".repeat(120)
        );
        assert!(!backup_name_ok(&long));
    }

    #[test]
    fn the_newest_backup_is_picked_by_its_name() {
        let dir = std::env::temp_dir().join(format!("alp-restore-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(newest_backup(&dir), None);
        let old = "immich-db-backup-20261114T020000-v3.3.3-pg14.19.sql.gz";
        let new = "immich-db-backup-20261115T101500-v3.3.3-pg14.19.sql.gz";
        fs::write(dir.join(old), b"x").unwrap();
        fs::write(dir.join(new), b"x").unwrap();
        // Not finished, empty, or not Immich's: never picked.
        fs::write(
            dir.join("immich-db-backup-20261116T020000-v3.3.3-pg14.19.sql.gz.tmp"),
            b"x",
        )
        .unwrap();
        fs::write(
            dir.join("immich-db-backup-20261117T020000-v3.3.3-pg14.19.sql.gz"),
            b"",
        )
        .unwrap();
        fs::write(dir.join("zzz.sql.gz"), b"x").unwrap();
        fs::create_dir(dir.join("immich-db-backup-20261118T020000-v3.3.3-pg14.19.sql.gz")).unwrap();
        assert_eq!(newest_backup(&dir).as_deref(), Some(new));
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(newest_backup(&dir), None);
    }

    #[test]
    fn the_marker_says_when_the_person_is_asked_first() {
        let name = "immich-db-backup-20261115T101500-v3.3.3-pg14.19.sql.gz";
        assert!(marker_asks(&format!("{name}\nask\n")));
        assert!(!marker_asks(&format!("{name}\n")));
        assert!(!marker_asks("ask\n"));
        assert!(!marker_asks(""));
    }

    #[test]
    fn a_backup_is_kept_aside_and_links_are_refused() {
        let dir = std::env::temp_dir().join(format!("alp-restore-keep-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let name = "immich-db-backup-20261115T101500-v3.3.3-pg14.19.sql.gz";
        fs::write(dir.join(name), b"backup").unwrap();
        let kept = dir.join("alp-kept");
        assert!(copy_backup(&dir, &kept, name));
        assert_eq!(fs::read(kept.join(name)).unwrap(), b"backup");
        // And back, after Immich rotated it away.
        fs::remove_file(dir.join(name)).unwrap();
        assert!(copy_backup(&kept, &dir, name));
        assert_eq!(fs::read(dir.join(name)).unwrap(), b"backup");
        // A folder that is a link, a file that is a link, another name: no.
        let elsewhere = dir.join("elsewhere");
        fs::create_dir(&elsewhere).unwrap();
        let link = dir.join("link");
        std::os::unix::fs::symlink(&elsewhere, &link).unwrap();
        assert!(!copy_backup(&dir, &link, name));
        assert!(fs::read_dir(&elsewhere).unwrap().next().is_none());
        let linked = "immich-db-backup-20261116T101500-v3.3.3-pg14.19.sql.gz";
        std::os::unix::fs::symlink(dir.join(name), dir.join(linked)).unwrap();
        assert!(!copy_backup(&dir, &kept, linked));
        assert!(!copy_backup(&dir, &kept, "../x"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_restore_ends_when_immich_leaves_maintenance_or_reports_an_error() {
        assert_eq!(
            step_of(
                &json!({"active": true, "action": "restore_database", "progress": 40, "task": "restore"})
            ),
            Step::Running
        );
        assert_eq!(
            step_of(&json!({"active": false, "action": "end"})),
            Step::Done
        );
        assert_eq!(
            step_of(
                &json!({"active": true, "action": "restore_database", "task": "error", "error": "Error: x"})
            ),
            Step::Failed
        );
        // Without the sign-in Immich hides the text but not the task.
        assert_eq!(
            step_of(&json!({"active": true, "action": "restore_database", "task": "error"})),
            Step::Failed
        );
    }

    #[test]
    fn a_backup_is_due_only_after_the_gallery_changed() {
        assert!(!backup_due(0, None));
        assert!(backup_due(5, None));
        assert!(backup_due(5, Some(4)));
        assert!(!backup_due(5, Some(5)));
    }
}
