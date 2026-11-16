// SPDX-License-Identifier: AGPL-3.0-or-later
//! A new version without erasing the stick (ADR 0026,
//! docs/plans/live-update.md).
//!
//! systemd-sysupdate does the work: it downloads the signed update files,
//! checks the signature, writes the new root and its dm-verity partition
//! into a free slot of the stick and puts the new start file (UKI) on the
//! alp-boot partition with a boot counter. systemd-boot then tries the new
//! version up to three times; a healthy start is marked good here
//! ([`bless`]), an unhealthy one falls back to the version before by itself.
//!
//! This module only: mounts alp-boot for those moments, asks sysupdate what
//! exists, starts it, and reads its progress. No own download, no own
//! signature check, no own partition writing.

use std::fs;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

use crate::fail::Fail;

const SYSUPDATE: &str = "/usr/lib/systemd/systemd-sysupdate";
const BLESS: &str = "/usr/lib/systemd/systemd-bless-boot";
/// GPT type of the alp-boot partition (Boot Loader Specification, XBOOTLDR;
/// image/extra/usr/lib/alp/repart-update.d/40-boot.conf).
const XBOOTLDR: &str = "bc13c2ff-59e6-4262-a352-b275fd6f7172";
/// Where sysupdate and bless-boot look for the XBOOTLDR partition.
const BOOT: &str = "/boot";
const UKI_DIR: &str = "/boot/EFI/Linux";
/// One update action at a time, also between alpd and `alpctl`.
const LOCK: &str = "/run/alp/update.lock";

/// The version systemd compares, from a release name: the third number is
/// read as two digits, so that 0.2.7 (0.2.70) is newer than 0.2.61. Same
/// rule as tools/update-version.sh.
pub fn sortable(name: &str) -> Option<String> {
    let mut parts = name.split('.');
    let (a, b) = (parts.next()?, parts.next()?);
    let c = parts.next().unwrap_or("0");
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if parts.next().is_some() || !digits(a) || !digits(b) || !digits(c) || c.len() > 2 {
        return None;
    }
    // 0.2.10 would be the same version as 0.2.1, and 0.2.05 would sort
    // before it: such names are not release names.
    if c.len() == 2 && (c.starts_with('0') || c.ends_with('0')) {
        return None;
    }
    let c = if c.len() == 1 {
        format!("{c}0")
    } else {
        c.to_string()
    };
    Some(format!("{a}.{b}.{c}"))
}

/// The release name people see, from the version systemd compares:
/// 0.2.70 is shown as 0.2.7, 0.2.61 stays 0.2.61.
pub fn display(version: &str) -> String {
    match version.rsplit_once('.') {
        Some((head, third)) if third.len() == 2 && third.ends_with('0') => {
            format!("{head}.{}", &third[..1])
        }
        _ => version.to_string(),
    }
}

/// Number by number; a version that is not only numbers and dots is older
/// than any that is.
pub fn newer(a: &str, b: &str) -> bool {
    let key = |v: &str| {
        v.split('.')
            .map(|p| p.parse::<u64>().ok())
            .collect::<Option<Vec<u64>>>()
    };
    match (key(a), key(b)) {
        (Some(a), Some(b)) => a > b,
        (Some(_), None) => true,
        _ => false,
    }
}

/// `IMAGE_VERSION` of the running system.
pub fn running() -> Option<String> {
    let text = fs::read_to_string("/usr/lib/os-release").ok()?;
    os_release_version(&text)
}

fn os_release_version(text: &str) -> Option<String> {
    text.lines()
        .find_map(|l| l.strip_prefix("IMAGE_VERSION="))
        .map(|v| v.trim().trim_matches('"').to_string())
        .filter(|v| !v.is_empty())
}

/// What a start file's name says: version, and whether its tries are used up.
/// `alp-os_0.2.80+3-0.efi`, `alp-os_0.2.80+0-3.efi` (bad), `alp-os_0.2.80.efi`
/// (marked good).
fn uki_name(name: &str) -> Option<(String, bool)> {
    let rest = name.strip_prefix("alp-os_")?.strip_suffix(".efi")?;
    match rest.split_once('+') {
        Some((version, tries)) => {
            let left = tries.split('-').next()?;
            Some((version.to_string(), left == "0"))
        }
        None => Some((rest.to_string(), false)),
    }
}

/// The alp-boot partition of the stick (kernel name), if the stick has one.
fn boot_partition() -> Option<String> {
    let (_, parts) = crate::index::stick().ok()?;
    parts
        .into_iter()
        .find(|(_, _, t)| t.eq_ignore_ascii_case(XBOOTLDR))
        .map(|(name, _, _)| name)
}

/// Whether this stick can take a new version in place (it was first started
/// with room for it: 28 GiB or more, ALP-OS 0.2.7 or later).
pub fn supported() -> bool {
    boot_partition().is_some()
}

fn run(cmd: &str, args: &[&str]) -> Result<String, String> {
    let o = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("{cmd}: {e}"))?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
    } else {
        Err(format!(
            "{cmd} failed: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        ))
    }
}

/// alp-boot mounted at /boot, for one action; unmounted when dropped. Holds
/// the lock, so two actions never share the mount.
struct Boot {
    _lock: fs::File,
}

impl Boot {
    fn open() -> Result<Self, Fail> {
        Self::open_within(std::time::Duration::ZERO)
    }

    /// As `open`, but waits up to `wait` for another update action to end
    /// (marking a version good must not fail because someone pressed
    /// "Check" at that moment).
    fn open_within(wait: std::time::Duration) -> Result<Self, Fail> {
        let unsupported = || {
            Fail::new(
                "update.unsupported",
                "This stick has no room for a new version. Write the new version to the stick instead.",
            )
        };
        let part = boot_partition().ok_or_else(unsupported)?;
        let internal =
            |e: String| Fail::new("update.failed", format!("The update could not start: {e}."));
        let _ = fs::create_dir_all("/run/alp");
        let lock = fs::File::create(LOCK).map_err(|e| internal(e.to_string()))?;
        let start = std::time::Instant::now();
        while lock.try_lock().is_err() {
            if start.elapsed() >= wait {
                return Err(Fail::new(
                    "update.busy",
                    "An update is already being checked or written. Try again in a moment.",
                ));
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
        // Left mounted by an action that was killed: with the lock held
        // nobody else uses it, so it is taken off and mounted afresh.
        let dev = format!("/dev/{part}");
        let stale = run(
            "findmnt",
            &["-rn", "-t", "vfat", "-o", "SOURCE", "--mountpoint", BOOT],
        );
        if stale.is_ok_and(|src| src.lines().any(|l| l == dev)) {
            let _ = run("umount", &[BOOT]);
        }
        run(
            "mount",
            &[
                "-t",
                "vfat",
                "-o",
                "rw,nodev,nosuid,noexec,umask=0077",
                &dev,
                BOOT,
            ],
        )
        .map_err(internal)?;
        Ok(Self { _lock: lock })
    }

    fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(UKI_DIR)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    /// Start files of updates on alp-boot: version, used up.
    fn ukis(&self) -> Vec<(String, bool)> {
        fs::read_dir(UKI_DIR)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| uki_name(&e.file_name().to_string_lossy()))
            .collect()
    }
}

impl Drop for Boot {
    fn drop(&mut self) {
        let _ = run("sync", &["-f", BOOT]);
        let _ = run("umount", &[BOOT]);
    }
}

/// What the stick and the update server say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// The running version, as systemd compares it.
    pub running: Option<String>,
    /// The newest version that is newer than the running one, if any.
    pub newer: Option<Newer>,
    /// Names of the start files of updates on alp-boot (for diagnosis).
    pub start_files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Newer {
    pub version: String,
    pub state: NewerState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewerState {
    /// On the server, not on the stick.
    Available,
    /// Written to the stick; the next start uses it.
    Installed,
    /// Written, tried three times, never healthy: the stick stays on the
    /// running version.
    Failed,
}

impl NewerState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Installed => "installed",
            Self::Failed => "failed",
        }
    }
}

/// From the versions sysupdate lists (`all`) and the start files on
/// alp-boot: the newest version above the running one, and where it stands.
fn pick(running: Option<&str>, all: &[String], ukis: &[(String, bool)]) -> Option<Newer> {
    let version = all
        .iter()
        .filter(|v| running.is_none_or(|r| newer(v, r)))
        .fold(None::<&String>, |best, v| match best {
            Some(b) if !newer(v, b) => Some(b),
            _ => Some(v),
        })?
        .clone();
    let state = match ukis.iter().find(|(v, _)| *v == version) {
        Some((_, true)) => NewerState::Failed,
        Some((_, false)) => NewerState::Installed,
        None => NewerState::Available,
    };
    Some(Newer { version, state })
}

fn sysupdate_failed(e: String) -> Fail {
    let low = e.to_ascii_lowercase();
    if low.contains("signature")
        || low.contains("no public key")
        || low.contains("download invalid")
    {
        Fail::new(
            "update.bad_signature",
            "The update files are not signed by the ALP-OS project. Nothing was changed.",
        )
    } else if low.contains("resolve")
        || low.contains("connect")
        || low.contains("timed out")
        || low.contains("network")
    {
        Fail::new(
            "update.offline",
            "The update server could not be reached. Check the internet connection and try again.",
        )
    } else {
        // The last line says what went wrong; the lines before are progress.
        let last = e
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim();
        Fail::new("update.failed", format!("The update did not work: {last}"))
    }
}

/// Ask the update server (signature checked by sysupdate) and the stick.
pub fn check() -> Result<Status, Fail> {
    let boot = Boot::open()?;
    let out = run(SYSUPDATE, &["--json=short", "list"]).map_err(sysupdate_failed)?;
    let all = parse_list(&out).ok_or_else(|| {
        Fail::new(
            "update.failed",
            "The update tool gave an answer that could not be read.",
        )
    })?;
    let running = running();
    let newer = pick(running.as_deref(), &all, &boot.ukis());
    Ok(Status {
        running,
        newer,
        start_files: boot.names(),
    })
}

/// `{"current":…,"all":["0.2.80","0.2.70"],…}` (systemd 257).
fn parse_list(out: &str) -> Option<Vec<String>> {
    let line = out
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with('{'))?;
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    Some(
        v.get("all")?
            .as_array()?
            .iter()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect(),
    )
}

/// Percent from one of sysupdate's progress lines ("Got 15% of https://…").
fn percent_of(line: &str) -> Option<u8> {
    let rest = line.trim().strip_prefix("Got ")?;
    rest.split('%').next()?.trim().parse().ok()
}

/// Which of the three files a progress line is about: root, verity, UKI.
fn part_of(line: &str) -> Option<usize> {
    if line.contains(".root.raw") {
        Some(0)
    } else if line.contains(".verity.raw") {
        Some(1)
    } else if line.contains(".efi") {
        Some(2)
    } else {
        None
    }
}

/// Download and write `version` (signature checked by sysupdate). The
/// running system is not touched; the next start tries the new version.
/// `say` gets a percentage over all three files (the root is nearly all).
pub fn apply(version: &str, say: &mut dyn FnMut(u8)) -> Result<(), Fail> {
    if !version.bytes().all(|b| b.is_ascii_digit() || b == b'.') || version.is_empty() {
        return Err(Fail::new("update.failed", "That is not a version."));
    }
    let _boot = Boot::open()?;
    let mut child = Command::new(SYSUPDATE)
        .args(["update", version])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Fail::new("update.failed", format!("The update could not start: {e}.")))?;
    let mut tail: Vec<String> = Vec::new();
    if let Some(err) = child.stderr.take() {
        // Shares of the whole: root 88 %, verity 2 %, UKI 10 %.
        const START: [u32; 3] = [0, 88, 90];
        const SHARE: [u32; 3] = [88, 2, 10];
        for line in BufReader::new(err).lines().map_while(Result::ok) {
            if let (Some(p), Some(i)) = (percent_of(&line), part_of(&line)) {
                say((START[i] + u32::from(p.min(100)) * SHARE[i] / 100).min(99) as u8);
                continue;
            }
            tail.push(line);
            if tail.len() > 40 {
                tail.remove(0);
            }
        }
    }
    let status = child
        .wait()
        .map_err(|e| Fail::new("update.failed", format!("The update did not work: {e}.")))?;
    if !status.success() {
        return Err(sysupdate_failed(tail.join("\n")));
    }
    say(100);
    Ok(())
}

/// After a healthy start of a version with a boot counter: remove the
/// counter (systemd-bless-boot good), so this version keeps starting.
pub fn bless() -> Result<(), String> {
    let _boot = Boot::open_within(std::time::Duration::from_secs(240)).map_err(|f| f.message)?;
    run(BLESS, &["good"]).map(|_| ())
}

/// Whether alpd answers on its API (`GET live/state` gives 200), asked for
/// up to three minutes. A new version that cannot do this is not marked
/// good.
pub fn healthy() -> Result<(), String> {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::{Duration, Instant};
    let ask = || -> Result<(), String> {
        let mut s = TcpStream::connect("127.0.0.1:8411").map_err(|e| e.to_string())?;
        let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
        s.write_all(b"GET /api/v1/live/state HTTP/1.0\r\nHost: 127.0.0.1:8411\r\n\r\n")
            .map_err(|e| e.to_string())?;
        let mut head = [0u8; 12];
        s.read_exact(&mut head).map_err(|e| e.to_string())?;
        if head.ends_with(b" 200") {
            Ok(())
        } else {
            Err(format!("alpd answered {}", String::from_utf8_lossy(&head)))
        }
    };
    let start = Instant::now();
    loop {
        match ask() {
            Ok(()) => return Ok(()),
            Err(e) if start.elapsed() > Duration::from_secs(180) => {
                return Err(format!("alpd does not answer: {e}"));
            }
            Err(_) => std::thread::sleep(Duration::from_secs(2)),
        }
    }
}

/// `alpctl live update healthy | bless | check | apply <version>`.
pub fn cli(args: &[&str]) -> i32 {
    let r = match args {
        ["healthy"] => healthy(),
        ["bless"] => bless(),
        ["check"] => check().map_err(|f| f.message).map(|s| {
            println!("running {}", s.running.as_deref().unwrap_or("unknown"));
            match s.newer {
                Some(n) => println!("newer {} {}", n.version, n.state.as_str()),
                None => println!("newer none"),
            }
            for f in s.start_files {
                println!("start-file {f}");
            }
        }),
        ["apply", version] => {
            let mut last = 255u8;
            apply(version, &mut |p| {
                if p != last && p % 10 == 0 {
                    println!("{p}%");
                }
                last = p;
            })
            .map_err(|f| f.message)
            .map(|()| println!("written {version}; restart to use it"))
        }
        _ => Err("usage: alpctl live update healthy | bless | check | apply <version>".into()),
    };
    match r {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("ALP-OS Live: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_names_sort_in_the_order_they_were_made() {
        assert_eq!(sortable("0.2.7").as_deref(), Some("0.2.70"));
        assert_eq!(sortable("0.2.61").as_deref(), Some("0.2.61"));
        assert_eq!(sortable("0.3").as_deref(), Some("0.3.00"));
        assert_eq!(sortable("0.2.100"), None);
        assert_eq!(sortable("0.2.10"), None);
        assert_eq!(sortable("0.2.05"), None);
        assert_eq!(sortable("0.2.x"), None);
        assert_eq!(sortable("0.2.7.1"), None);
        let order = [
            "0.2.5", "0.2.51", "0.2.6", "0.2.61", "0.2.7", "0.2.71", "0.2.8", "0.3",
        ];
        for pair in order.windows(2) {
            let (a, b) = (sortable(pair[0]).unwrap(), sortable(pair[1]).unwrap());
            assert!(
                newer(&b, &a),
                "{} should be newer than {}",
                pair[1],
                pair[0]
            );
            assert!(!newer(&a, &b));
        }
        assert!(!newer("0.2.70", "0.2.70"));
    }

    #[test]
    fn people_see_the_short_release_name() {
        assert_eq!(display("0.2.70"), "0.2.7");
        assert_eq!(display("0.2.61"), "0.2.61");
        assert_eq!(display("0.3.00"), "0.3.0");
        for name in ["0.2.7", "0.2.61", "0.2.8"] {
            assert_eq!(display(&sortable(name).unwrap()), name);
        }
    }

    #[test]
    fn the_running_version_is_read_from_os_release() {
        assert_eq!(
            os_release_version("ID=debian\nIMAGE_ID=\"alp-os\"\nIMAGE_VERSION=\"0.2.70\"\n")
                .as_deref(),
            Some("0.2.70")
        );
        assert_eq!(os_release_version("ID=debian\n"), None);
    }

    #[test]
    fn start_file_names_say_version_and_used_up_tries() {
        assert_eq!(
            uki_name("alp-os_0.2.80+3-0.efi"),
            Some(("0.2.80".into(), false))
        );
        assert_eq!(
            uki_name("alp-os_0.2.80+1-2.efi"),
            Some(("0.2.80".into(), false))
        );
        assert_eq!(
            uki_name("alp-os_0.2.80+0-3.efi"),
            Some(("0.2.80".into(), true))
        );
        assert_eq!(
            uki_name("alp-os_0.2.80+0.efi"),
            Some(("0.2.80".into(), true))
        );
        assert_eq!(
            uki_name("alp-os_0.2.80.efi"),
            Some(("0.2.80".into(), false))
        );
        assert_eq!(uki_name("other.efi"), None);
    }

    #[test]
    fn the_newest_version_above_the_running_one_is_offered() {
        let all = [
            "0.2.70".to_string(),
            "0.2.80".to_string(),
            "0.2.71".to_string(),
        ];
        assert_eq!(
            pick(Some("0.2.70"), &all, &[]),
            Some(Newer {
                version: "0.2.80".into(),
                state: NewerState::Available
            })
        );
        assert_eq!(
            pick(Some("0.2.70"), &all, &[("0.2.80".into(), false)])
                .unwrap()
                .state,
            NewerState::Installed
        );
        assert_eq!(
            pick(Some("0.2.70"), &all, &[("0.2.80".into(), true)])
                .unwrap()
                .state,
            NewerState::Failed
        );
        // Nothing newer: nothing offered, also not the running version itself.
        assert_eq!(pick(Some("0.2.80"), &all, &[]), None);
        assert_eq!(pick(Some("0.2.80"), &[], &[]), None);
    }

    #[test]
    fn sysupdate_output_is_read() {
        let out =
            "Discovering…\n{\"current\":null,\"all\":[\"0.2.80\",\"0.2.70\"],\"appstreamUrls\":[]}";
        assert_eq!(
            parse_list(out),
            Some(vec!["0.2.80".to_string(), "0.2.70".to_string()])
        );
        assert_eq!(parse_list("nothing"), None);
        assert_eq!(
            percent_of("Got 15% of http://x/alp-os_0.2.70_1111.root.raw.xz."),
            Some(15)
        );
        assert_eq!(percent_of("Exiting."), None);
        assert_eq!(
            part_of("Got 1% of http://x/alp-os_0.2.70_1.root.raw.xz."),
            Some(0)
        );
        assert_eq!(
            part_of("Got 1% of http://x/alp-os_0.2.70_2.verity.raw.xz."),
            Some(1)
        );
        assert_eq!(part_of("Got 1% of http://x/alp-os_0.2.70.efi."), Some(2));
    }

    #[test]
    fn failures_are_said_plainly() {
        assert_eq!(
            sysupdate_failed(
                "gpg: BAD signature\nDOWNLOAD INVALID: Signature verification failed.".into()
            )
            .key,
            "update.bad_signature"
        );
        assert_eq!(
            sysupdate_failed("Could not resolve host: github.com".into()).key,
            "update.offline"
        );
        assert_eq!(
            sysupdate_failed("something else".into()).key,
            "update.failed"
        );
    }
}
