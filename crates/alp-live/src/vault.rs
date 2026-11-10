// SPDX-License-Identifier: AGPL-3.0-or-later
//! The archive's encrypted folder (storage v2, ADR 0014): gocryptfs on the
//! backup disk. No own cryptography: gocryptfs encrypts, wraps the master
//! key with scrypt and changes passwords; this module only builds its
//! command lines, hands it passwords on stdin or in a file inside the
//! encrypted index, and mounts it in its own systemd scope.
//!
//! gocryptfs 2.5.1 (Debian trixie snapshot 20261016T023245Z), behaviour
//! read from its source (ADR 0014 "Evidence"): passwords come one line
//! each from stdin when stdin is not a terminal; `-passwd` reads the old
//! password, then the new one; exit code 12 means a wrong password; as root
//! with `-allow_other` new files belong to the calling user.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// gocryptfs' exit code for a wrong password (`internal/exitcodes`).
pub const EXIT_WRONG_PASSWORD: i32 = 12;
/// The transient scope the FUSE daemon lives in, so it survives a restart
/// of alpd or of the unit that mounted it.
pub const SCOPE: &str = "alp-archive-files.scope";
const SCOPE_UNIT: &str = "alp-archive-files";
/// Units that use the mounted folder (stopped before the scope).
const ORDERED_BEFORE: &str = "alp-immich.service alp-live-archive.service";
/// Where the mount's error output goes (the daemon keeps a copy of its
/// stderr until it moves to syslog, so it cannot be a pipe we wait on).
const MOUNT_LOG: &str = "/run/alp/gocryptfs-mount.log";

/// Why a gocryptfs call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Exit code 12: the password (archive password or recovery secret) does
    /// not open this config.
    WrongPassword,
    /// Did not finish in time (stopped).
    Timeout(String),
    Failed(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WrongPassword => f.write_str("the password does not open this archive"),
            Self::Timeout(m) | Self::Failed(m) => f.write_str(m),
        }
    }
}

impl From<Error> for String {
    fn from(e: Error) -> Self {
        e.to_string()
    }
}

/// `gocryptfs -init`: a new encrypted folder in the empty `cipherdir`, its
/// config in it (`gocryptfs.conf`). The password is read from stdin.
pub fn init_args(cipherdir: &Path) -> Vec<String> {
    vec![
        "-init".into(),
        "-q".into(),
        "--".into(),
        cipherdir.to_string_lossy().into_owned(),
    ]
}

/// `gocryptfs -passwd -config <config>`: re-wrap the master key in
/// `config` with a new password (stdin: old password line, new password
/// line). `cipherdir` is the folder it belongs to.
pub fn passwd_args(config: &Path, cipherdir: &Path) -> Vec<String> {
    vec![
        "-passwd".into(),
        "-q".into(),
        "-config".into(),
        config.to_string_lossy().into_owned(),
        "--".into(),
        cipherdir.to_string_lossy().into_owned(),
    ]
}

/// Mount `cipherdir` decrypted at `mountpoint`, password from `passfile`,
/// inside the scope [`SCOPE`]. `-allow_other`: Immich's containers (another
/// user) use it; as root this also makes gocryptfs create files as the
/// calling user and the kernel check permissions. `-noexec`: nothing on
/// the archive is ever run. (FUSE mounts are nodev,nosuid by default.)
/// The scope is ordered before Immich and the archive unit, so at shutdown
/// systemd stops it (SIGTERM to the daemon) only after Immich stopped and
/// `alpctl live archive close` unmounted the folder, never in parallel.
pub fn mount_args(passfile: &Path, cipherdir: &Path, mountpoint: &Path) -> Vec<String> {
    vec![
        "--scope".into(),
        format!("--unit={SCOPE_UNIT}"),
        "--quiet".into(),
        "--collect".into(),
        format!("--property=Before={ORDERED_BEFORE}"),
        "--".into(),
        "gocryptfs".into(),
        "-q".into(),
        "-allow_other".into(),
        "-noexec".into(),
        "-passfile".into(),
        passfile.to_string_lossy().into_owned(),
        "--".into(),
        cipherdir.to_string_lossy().into_owned(),
        mountpoint.to_string_lossy().into_owned(),
    ]
}

/// The archive's id: gocryptfs' random IV of the top folder
/// (`gocryptfs.diriv`, 16 bytes, never changes), as hex. Not secret: it is
/// stored plain on the disk; it tells this archive apart from another one.
pub fn archive_id(cipherdir: &Path) -> Result<String, String> {
    let b = fs::read(cipherdir.join("gocryptfs.diriv"))
        .map_err(|e| format!("the archive's folder cannot be read: {e}"))?;
    if b.len() != 16 {
        return Err("the archive's folder is damaged (gocryptfs.diriv)".into());
    }
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// Run gocryptfs with `input` on stdin (password lines), within `limit`.
/// Its error output is kept (it never contains a password: gocryptfs does
/// not print them).
fn gocryptfs(args: &[String], input: &str, limit: Duration) -> Result<(), Error> {
    let mut child = Command::new("gocryptfs")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::Failed(format!("gocryptfs: {e}")))?;
    let mut stderr = child.stderr.take();
    let errors = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(p) = stderr.as_mut() {
            let _ = p.read_to_string(&mut s);
        }
        s
    });
    if let Some(mut stdin) = child.stdin.take() {
        // Dropped (closed) at the end of this block.
        let _ = stdin.write_all(input.as_bytes());
    }
    let status = wait(&mut child, limit)?;
    let err = errors.join().unwrap_or_default();
    match status.code() {
        Some(0) => Ok(()),
        Some(EXIT_WRONG_PASSWORD) => Err(Error::WrongPassword),
        _ => Err(Error::Failed(format!(
            "gocryptfs failed ({status}): {}",
            crate::fail::display_safe(err.trim(), 300)
        ))),
    }
}

fn wait(
    child: &mut std::process::Child,
    limit: Duration,
) -> Result<std::process::ExitStatus, Error> {
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(s)) => return Ok(s),
            Ok(None) => {}
            Err(e) => return Err(Error::Failed(e.to_string())),
        }
        if start.elapsed() >= limit {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Timeout(format!(
                "gocryptfs did not finish within {} seconds; the disk may be very slow or not answering",
                limit.as_secs()
            )));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Make a new encrypted folder in the empty folder `cipherdir`, locked with
/// `password`.
pub fn init(cipherdir: &Path, password: &str) -> Result<(), Error> {
    let input = zeroize::Zeroizing::new(format!("{password}\n"));
    gocryptfs(&init_args(cipherdir), &input, Duration::from_secs(120))
}

/// Re-wrap the master key held by `config` (a gocryptfs config of
/// `cipherdir`): it then opens with `new` instead of `old`. gocryptfs
/// writes it through `<config>.tmp` and renames.
pub fn rewrap(config: &Path, cipherdir: &Path, old: &str, new: &str) -> Result<(), Error> {
    rewrap_n(config, cipherdir, old, new, None)
}

/// [`rewrap`] with gocryptfs' `-scryptn` (log2 of scrypt's cost) for the
/// new password: a config a person's password guards (level 3's
/// `password.conf`) gets a higher cost than the default 16.
pub fn rewrap_n(
    config: &Path,
    cipherdir: &Path,
    old: &str,
    new: &str,
    scryptn: Option<u8>,
) -> Result<(), Error> {
    let _ = fs::remove_file(tmp_of(config));
    let input = zeroize::Zeroizing::new(format!("{old}\n{new}\n"));
    let mut args = passwd_args(config, cipherdir);
    if let Some(n) = scryptn {
        args.insert(1, n.to_string());
        args.insert(1, "-scryptn".into());
    }
    gocryptfs(&args, &input, Duration::from_secs(120))
}

fn tmp_of(config: &Path) -> PathBuf {
    let mut s = config.as_os_str().to_owned();
    s.push(".tmp");
    PathBuf::from(s)
}

/// Make a copy of the config `from` at `to` with `new` as its password
/// (unlocked with `old`), then put it in place atomically (temporary name,
/// fsync, rename, fsync of the folder). An earlier unfinished copy is
/// replaced; `from` is never changed.
pub fn copy_rewrapped(
    from: &Path,
    to: &Path,
    cipherdir: &Path,
    old: &str,
    new: &str,
) -> Result<(), Error> {
    copy_rewrapped_n(from, to, cipherdir, old, new, None)
}

/// [`copy_rewrapped`] with a scrypt cost for the new password ([`rewrap_n`]).
pub fn copy_rewrapped_n(
    from: &Path,
    to: &Path,
    cipherdir: &Path,
    old: &str,
    new: &str,
    scryptn: Option<u8>,
) -> Result<(), Error> {
    let fail = |e: std::io::Error| Error::Failed(format!("{}: {e}", to.display()));
    let mut work = to.as_os_str().to_owned();
    work.push(".new");
    let work = PathBuf::from(work);
    let _ = fs::remove_file(&work);
    fs::copy(from, &work).map_err(fail)?;
    // gocryptfs writes configs read-only (0400); the copy must be
    // replaceable by its own `.tmp` rename, which needs only the folder.
    let res = rewrap_n(&work, cipherdir, old, new, scryptn);
    if res.is_err() {
        let _ = fs::remove_file(&work);
        let _ = fs::remove_file(tmp_of(&work));
        return res;
    }
    fs::File::open(&work)
        .and_then(|f| f.sync_all())
        .map_err(fail)?;
    fs::rename(&work, to).map_err(fail)?;
    sync_dir(to.parent().unwrap_or(Path::new("/"))).map_err(Error::Failed)
}

fn sync_dir(dir: &Path) -> Result<(), String> {
    fs::File::open(dir)
        .and_then(|d| d.sync_all())
        .map_err(|e| format!("{}: {e}", dir.display()))
}

fn is_mounted(at: &Path) -> bool {
    let at = at.to_string_lossy();
    fs::read_to_string("/proc/self/mountinfo").is_ok_and(|m| {
        m.lines()
            .any(|l| l.split_whitespace().nth(4) == Some(at.as_ref()))
    })
}

fn scope_active() -> bool {
    Command::new("systemctl")
        .args(["is-active", "--quiet", SCOPE])
        .status()
        .is_ok_and(|s| s.success())
}

/// Mount the folder (see [`mount_args`]). A daemon left from an earlier
/// mount that is gone (its scope still there) is stopped first, otherwise
/// the scope name is taken.
pub fn mount(passfile: &Path, cipherdir: &Path, mountpoint: &Path) -> Result<(), Error> {
    if is_mounted(mountpoint) {
        return Ok(());
    }
    if scope_active() {
        let _ = Command::new("systemctl").args(["stop", SCOPE]).status();
    }
    fs::create_dir_all(mountpoint).map_err(|e| Error::Failed(e.to_string()))?;
    let _ = fs::create_dir_all("/run/alp");
    let log = fs::File::create(MOUNT_LOG).map_err(|e| Error::Failed(e.to_string()))?;
    let mut child = Command::new("systemd-run")
        .args(mount_args(passfile, cipherdir, mountpoint))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log))
        .spawn()
        .map_err(|e| Error::Failed(format!("systemd-run: {e}")))?;
    let status = match wait(&mut child, Duration::from_secs(120)) {
        Ok(s) => s,
        Err(e) => {
            // Killing systemd-run (the gocryptfs parent) leaves a daemon
            // that may still be starting in the scope.
            let _ = Command::new("systemctl").args(["stop", SCOPE]).status();
            return Err(e);
        }
    };
    let err = fs::read_to_string(MOUNT_LOG).unwrap_or_default();
    match status.code() {
        Some(0) if is_mounted(mountpoint) => Ok(()),
        Some(0) => Err(Error::Failed(
            "gocryptfs ended without mounting the archive".into(),
        )),
        Some(EXIT_WRONG_PASSWORD) => Err(Error::WrongPassword),
        _ => Err(Error::Failed(format!(
            "gocryptfs could not open the archive ({status}): {}",
            crate::fail::display_safe(err.trim(), 300)
        ))),
    }
}

/// Unmount the decrypted view and wait (up to 30 s) until the daemon has
/// let go of the backup disk, so the disk itself can be unmounted next.
pub fn unmount(mountpoint: &Path) -> Result<(), String> {
    if is_mounted(mountpoint) {
        let o = Command::new("umount")
            .arg(mountpoint)
            .output()
            .map_err(|e| format!("umount: {e}"))?;
        if !o.status.success() {
            return Err(format!(
                "the archive is still in use: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            ));
        }
    }
    let wait_gone = |secs: u64| {
        let t = Instant::now();
        while scope_active() && t.elapsed() < Duration::from_secs(secs) {
            std::thread::sleep(Duration::from_millis(200));
        }
    };
    wait_gone(30);
    if scope_active() {
        // `kill`, not `stop`: at shutdown the scope's own stop job waits for
        // the archive unit whose ExecStop runs this (ordering above), and a
        // `systemctl stop` here would wait for that job.
        let _ = Command::new("systemctl")
            .args(["kill", "--signal=SIGTERM", SCOPE])
            .status();
        wait_gone(15);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_lines() {
        let d = Path::new("/run/alp/host/ALPArchive/data");
        assert_eq!(
            init_args(d),
            ["-init", "-q", "--", "/run/alp/host/ALPArchive/data"]
        );
        assert_eq!(
            passwd_args(Path::new("/x/recovery.conf.new"), d),
            [
                "-passwd",
                "-q",
                "-config",
                "/x/recovery.conf.new",
                "--",
                "/run/alp/host/ALPArchive/data"
            ]
        );
        let m = mount_args(
            Path::new("/var/lib/alp-index/keys/archive.pass"),
            d,
            Path::new("/var/lib/alp-pool/files"),
        );
        assert_eq!(
            &m[..5],
            [
                "--scope",
                "--unit=alp-archive-files",
                "--quiet",
                "--collect",
                "--property=Before=alp-immich.service alp-live-archive.service",
            ]
        );
        assert_eq!(m[5], "--");
        assert_eq!(m[6], "gocryptfs");
        assert!(m.contains(&"-allow_other".to_string()));
        assert!(m.contains(&"-noexec".to_string()));
        // The password is never on the command line, only the file's path.
        let pf = m.iter().position(|a| a == "-passfile").unwrap();
        assert_eq!(m[pf + 1], "/var/lib/alp-index/keys/archive.pass");
        assert_eq!(
            m[m.len() - 2..],
            ["/run/alp/host/ALPArchive/data", "/var/lib/alp-pool/files"]
        );
        assert_eq!(
            tmp_of(Path::new("/a/gocryptfs.conf")),
            PathBuf::from("/a/gocryptfs.conf.tmp")
        );
    }

    #[test]
    fn ids_come_from_the_folder_iv() {
        let d = std::env::temp_dir().join(format!("alp-vault-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        assert!(archive_id(&d).is_err());
        fs::write(d.join("gocryptfs.diriv"), [0xab; 16]).unwrap();
        assert_eq!(archive_id(&d).unwrap(), "ab".repeat(16));
        fs::write(d.join("gocryptfs.diriv"), [1; 15]).unwrap();
        assert!(archive_id(&d).unwrap_err().contains("damaged"));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn wrong_password_reads_plainly() {
        assert_eq!(
            Error::WrongPassword.to_string(),
            "the password does not open this archive"
        );
    }
}
