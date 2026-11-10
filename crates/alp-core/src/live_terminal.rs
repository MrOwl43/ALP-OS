// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Terminal (Live UI stage F, feature `terminal`): one text session on
//! the computer's own screen, for support and advanced users. Plan and
//! security rules: docs/plans/live-terminal.md (revision 2, approved by the
//! Security and Network Expert, review T1-T6).
//!
//! alpd only starts and stops `alp-terminal.service` (ttyd as the
//! unprivileged user `alp-term`, image/units/alp-terminal.service). The
//! protections live in the system, not here: the nftables rule that cuts
//! `alp-term` off from every local service (T1), the unit's sandbox (T3)
//! and the user's lack of rights.
//!
//! The session's address carries a random 128-bit path, the only secret
//! (review round 2, note 3). It is handed to the unit in a root-only file
//! that is removed as soon as the unit has started, returned once by
//! `terminal/start`, and never put in the state or in any log.

use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, Once};
use std::time::{Duration, Instant};

use alp_live::fail::Fail;
use serde_json::{Value, json};

pub const UNIT: &str = "alp-terminal.service";
/// ttyd's port: an origin of its own, separate from the console (8411) and
/// the gallery (8412), review U2.
pub const PORT: u16 = 8413;
/// The unit reads the session's path from here (`EnvironmentFile=`).
const ENV_FILE: &str = "/run/alp/terminal.env";
const UNIT_FILE: &str = "/usr/lib/systemd/system/alp-terminal.service";
const TTYD: &str = "/usr/bin/ttyd";
/// Journal tag of the session's start, stop and command lines (T4, T5).
const TAG: &str = "alp-terminal";
/// How long the unit and ttyd may take to come up.
const START_SECS: u64 = 15;

/// A session is running (set by `start`, cleared by the watcher).
static RUNNING: AtomicBool = AtomicBool::new(false);
/// One start or stop at a time.
static TURN: Mutex<()> = Mutex::new(());
/// A session left by an earlier alpd is ended once (its console sign-in
/// is gone with that alpd).
static CLEAN: Once = Once::new();

/// The feature is offered only when the image carries the unit and ttyd
/// (and with them the user and the firewall rule, all in one drop-in).
pub fn available() -> bool {
    Path::new(UNIT_FILE).exists() && Path::new(TTYD).exists()
}

/// `terminal` in the state. Never the address.
pub fn state_json() -> Value {
    CLEAN.call_once(|| {
        if available() && unit_active() {
            eprintln!("alpd: terminal: ending a session left by an earlier start");
            stop_unit();
        }
    });
    json!({"state": if RUNNING.load(Ordering::SeqCst) { "running" } else { "off" }})
}

/// A new random path: 128 bits from the kernel, as 32 hex characters.
/// Fails closed: no session without a good secret.
pub fn secret_path() -> Result<String, Fail> {
    let mut b = [0u8; 16];
    fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .map_err(|_| Fail::new("terminal.failed", "The Terminal could not start."))?;
    if b.iter().all(|x| *x == 0) {
        return Err(Fail::new(
            "terminal.failed",
            "The Terminal could not start.",
        ));
    }
    let s = b.iter().map(|x| format!("{x:02x}")).collect();
    b.fill(0);
    Ok(s)
}

/// The address the console frames (docs/plans/live-terminal.md, "What the
/// console UI needs", item 2).
pub fn url(path: &str) -> String {
    format!("http://127.0.0.1:{PORT}/{path}/")
}

/// `terminal/start`, after alpd checked the console sign-in and the
/// password: start the unit with a new path; returns the address.
/// `signed_in` tells the watcher whether the console is still signed in.
pub fn start(signed_in: impl Fn() -> bool + Send + 'static) -> Result<String, Fail> {
    let _turn = TURN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if RUNNING.load(Ordering::SeqCst) || unit_active() {
        return Err(Fail::new(
            "terminal.running",
            "A Terminal session is already open.",
        ));
    }
    let path = secret_path()?;
    // T6: the log page, written here as root (the session only reads it).
    // Without it the session still works; `logs` then says so.
    let _ = alp_live::live::term_logs();
    let failed = || Fail::new("terminal.failed", "The Terminal could not start.");
    write_env(&path).map_err(|e| {
        eprintln!("alpd: terminal: session file: {}", e.kind());
        failed()
    })?;
    let started = run_bounded("systemctl", &["start", UNIT], START_SECS);
    // systemd has read the file by now (or will never): it goes at once.
    let _ = fs::remove_file(ENV_FILE);
    if !started || !port_open(Duration::from_secs(5)) {
        eprintln!("alpd: terminal: the session did not start");
        stop_unit();
        return Err(failed());
    }
    RUNNING.store(true, Ordering::SeqCst);
    note("Terminal session started");
    std::thread::spawn(move || watch(signed_in));
    Ok(url(&path))
}

/// `terminal/stop` (and the console lock): end the session, if any.
pub fn stop() {
    let _turn = TURN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if RUNNING.load(Ordering::SeqCst) || unit_active() {
        stop_unit();
    }
}

/// While the session runs: end it when the console is no longer signed in
/// (lock, idle lock, a new sign-in elsewhere); notice when it ended by
/// itself (ttyd `--once`: the frame left; bash `TMOUT`: no input for 10
/// minutes; `RuntimeMaxSec=`).
fn watch(signed_in: impl Fn() -> bool) {
    loop {
        std::thread::sleep(Duration::from_secs(2));
        if !unit_active() {
            break;
        }
        if !signed_in() {
            eprintln!("alpd: terminal: the console locked, ending the session");
            stop();
            break;
        }
    }
    RUNNING.store(false, Ordering::SeqCst);
    let _ = fs::remove_file(ENV_FILE);
    alp_live::live::term_logs_clear();
    note("Terminal session ended");
}

fn stop_unit() {
    if !run_bounded("systemctl", &["stop", UNIT], START_SECS) {
        // The stop job goes on in systemd; kill what is left at once.
        let _ = run_bounded("systemctl", &["kill", "--signal=SIGKILL", UNIT], 5);
    }
    let _ = fs::remove_file(ENV_FILE);
    alp_live::live::term_logs_clear();
}

fn unit_active() -> bool {
    run_bounded("systemctl", &["is-active", "--quiet", UNIT], 5)
}

/// The session file: root only, created fresh (a leftover is replaced).
fn write_env(path: &str) -> std::io::Result<()> {
    let _ = fs::remove_file(ENV_FILE);
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(ENV_FILE)?;
    f.write_all(env_text(path).as_bytes())?;
    f.sync_all()
}

fn env_text(path: &str) -> String {
    format!("ALP_TERM_PATH={path}\n")
}

fn port_open(within: Duration) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], PORT));
    let t = Instant::now();
    while t.elapsed() < within {
        if TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

/// A start or stop in the journal under the tag `alp-terminal` (T5).
fn note(msg: &str) {
    let _ = run_bounded("logger", &["-t", TAG, "--", msg], 5);
}

/// Run a command for at most `secs`; true when it ended well in time.
fn run_bounded(cmd: &str, args: &[&str], secs: u64) -> bool {
    let Ok(mut child) = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let t = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(s)) => return s.success(),
            Ok(None) if t.elapsed() < Duration::from_secs(secs) => {
                std::thread::sleep(Duration::from_millis(100));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_128_bit_and_differ() {
        let a = secret_path().unwrap();
        let b = secret_path().unwrap();
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn the_address_is_loopback_on_its_own_port() {
        let u = url("0123456789abcdef0123456789abcdef");
        assert_eq!(u, "http://127.0.0.1:8413/0123456789abcdef0123456789abcdef/");
    }

    #[test]
    fn the_unit_gets_only_the_path() {
        assert_eq!(env_text("ab"), "ALP_TERM_PATH=ab\n");
    }

    #[test]
    fn the_state_never_holds_the_address() {
        let s = state_json().to_string();
        assert!(s == r#"{"state":"off"}"#, "{s}");
    }
}
