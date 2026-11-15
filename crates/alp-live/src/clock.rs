// SPDX-License-Identifier: AGPL-3.0-or-later
//! The computer's clock (alpha 0.2.61).
//!
//! The system clock is UTC and is set from the network by
//! systemd-timesyncd. Until that happens it is whatever the hardware clock
//! held at start, read as UTC. A computer that also runs Windows keeps its
//! hardware clock in local time, so the system clock is then off by the
//! time zone's offset (alpha 0.2.5 and 0.2.6 field reports: seven hours
//! ahead). Two helpers for the console:
//!
//! - [`sync_now`]: ask systemd-timesyncd to ask a time server again;
//! - [`set_base`]: the person says whether the hardware clock holds local
//!   time; the system clock is moved by the chosen zone's offset (with
//!   `date`, the zone's rules from tzdata). Refused once the network set
//!   the clock.
//!
//! The hardware clock itself is never written: `/etc/adjtime` in the image
//! says LOCAL, and systemd-timesyncd then leaves it alone
//! (timesyncd.c: "All system time to RTC updates are disabled"), so the
//! computer's other system finds its clock as it left it.

use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Touched by systemd-timesyncd on each successful synchronisation
/// (systemd-timesyncd.service(8), FILES); `/run` is empty at every start.
const SYNCED: &str = "/run/systemd/timesync/synchronized";
/// The seconds [`set_base`] took off the system clock since this start
/// (0: the hardware clock was said to be UTC).
const BASE: &str = "/run/alp/clock-base";
const ZONEINFO: &str = "/usr/share/zoneinfo";
/// No zone is further from UTC than this (tzdata: -12:00 to +14:00).
const MAX_OFFSET: i64 = 14 * 3600;

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

pub fn synced() -> bool {
    Path::new(SYNCED).exists()
}

/// When the network last set the clock (seconds since 1970).
pub fn synced_at() -> Option<u64> {
    fs::metadata(SYNCED)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
}

/// What was said about the hardware clock since this start: `Some(true)`
/// local time, `Some(false)` UTC, `None` nothing yet.
pub fn base() -> Option<bool> {
    applied().map(|s| s != 0)
}

fn applied() -> Option<i64> {
    fs::read_to_string(BASE).ok()?.trim().parse().ok()
}

/// A tz database name by its shape: one to three parts of letters, digits,
/// `_`, `+`, `-`; at most 64 bytes. No dot and no empty part, so it cannot
/// leave the zone folder.
pub fn zone_shape(s: &str) -> bool {
    let part = |p: &str| {
        !p.is_empty()
            && p.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'+' | b'-'))
    };
    s.len() <= 64 && s.split('/').count() <= 3 && s.split('/').all(part)
}

/// `+HHMM` / `-HHMM` as seconds east of UTC.
pub fn parse_offset(text: &str) -> Option<i64> {
    let t = text.trim();
    let sign = match t.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let d = &t[1..];
    if d.len() != 4 || !d.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (h, m) = (d[..2].parse::<i64>().ok()?, d[2..].parse::<i64>().ok()?);
    let secs = h * 3600 + m * 60;
    (m < 60 && secs <= MAX_OFFSET).then_some(sign * secs)
}

/// The zone's offset from UTC at `at`, by tzdata's rules (`date`).
pub fn zone_offset(zone: &str, at: u64) -> Result<i64, String> {
    if !zone_shape(zone) || !Path::new(ZONEINFO).join(zone).is_file() {
        return Err("the time zone is not known".into());
    }
    let out = Command::new("date")
        .env("TZ", zone)
        .args(["-d", &format!("@{at}"), "+%z"])
        .output()
        .map_err(|e| e.to_string())?;
    parse_offset(&String::from_utf8_lossy(&out.stdout))
        .filter(|_| out.status.success())
        .ok_or_else(|| "the time zone's offset could not be read".into())
}

/// The clock after the person's answer: `raw` is the system clock as the
/// hardware clock gave it (nothing taken off), `offset` the zone's offset
/// when the hardware clock holds local time.
pub fn based(raw: u64, offset: i64) -> u64 {
    raw.saturating_add_signed(-offset)
}

/// Say whether the hardware clock holds local time (`zone`: the chosen
/// time zone) or UTC (`None`), and move the system clock to match. An
/// earlier answer since this start is undone first, so answering twice
/// never moves the clock twice. Refused once the network set the clock.
pub fn set_base(zone: Option<&str>) -> Result<(), String> {
    if synced() {
        return Err("the clock is already set from the network".into());
    }
    let before = applied().unwrap_or(0);
    let raw = now().saturating_add_signed(before);
    let offset = match zone {
        Some(z) => zone_offset(z, raw)?,
        None => 0,
    };
    if offset != before {
        // The network may have set the clock while the offset was read.
        if synced() {
            return Err("the clock is already set from the network".into());
        }
        let ok = Command::new("date")
            .args(["-u", "-s", &format!("@{}", based(raw, offset))])
            .output()
            .map_err(|e| e.to_string())?
            .status
            .success();
        if !ok {
            return Err("the clock could not be set".into());
        }
    }
    if let Some(dir) = Path::new(BASE).parent() {
        let _ = fs::create_dir_all(dir);
    }
    fs::write(BASE, format!("{offset}\n")).map_err(|e| e.to_string())
}

/// At start: the answer kept on the stick ("the hardware clock holds local
/// time", with the chosen zone) is applied again, once, while the network
/// has not set the clock.
pub fn apply_kept(local: bool, zone: Option<&str>) {
    if !local || synced() || applied().is_some() {
        return;
    }
    let Some(z) = zone else { return };
    match set_base(Some(z)) {
        Ok(()) => eprintln!("alpd: clock: hardware clock taken as local time"),
        Err(e) => eprintln!("alpd: clock: kept answer not applied: {e}"),
    }
}

/// Ask systemd-timesyncd to ask a time server now: a restart makes it
/// send a request at once. The answer shows as a new [`synced_at`].
pub fn sync_now() -> Result<(), String> {
    let ok = Command::new("systemctl")
        .args(["--no-block", "restart", "systemd-timesyncd.service"])
        .status()
        .map_err(|e| e.to_string())?
        .success();
    if ok {
        Ok(())
    } else {
        Err("the time service could not be asked".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_are_read_strictly() {
        assert_eq!(parse_offset("+0700\n"), Some(7 * 3600));
        assert_eq!(parse_offset("-0330"), Some(-(3 * 3600 + 1800)));
        assert_eq!(parse_offset("+0000"), Some(0));
        assert_eq!(parse_offset("+1400"), Some(MAX_OFFSET));
        for bad in [
            "", "0700", "+07", "+07:00", "+1500", "+0760", "+07000", "UTC",
        ] {
            assert_eq!(parse_offset(bad), None, "{bad}");
        }
    }

    #[test]
    fn zone_names_cannot_leave_the_folder() {
        for ok in ["Etc/GMT-2", "UTC", "America/Argentina/Ushuaia"] {
            assert!(zone_shape(ok), "{ok}");
        }
        for bad in ["", "../x", "/etc/passwd", "a//b", "a/b/c/d", "a b", "a.b"] {
            assert!(!zone_shape(bad), "{bad}");
        }
        assert!(!zone_shape(&"z".repeat(65)));
        assert!(zone_offset("../../etc/passwd", 0).is_err());
        assert!(zone_offset("Etc/No_Such_Zone", 0).is_err());
    }

    /// A hardware clock in local time reads seven hours ahead in a zone
    /// seven hours east: the offset is taken off.
    #[test]
    fn a_local_hardware_clock_is_moved_back_by_the_offset() {
        assert_eq!(based(1_800_000_000, 7 * 3600), 1_800_000_000 - 7 * 3600);
        assert_eq!(based(1_800_000_000, -5 * 3600), 1_800_000_000 + 5 * 3600);
        assert_eq!(based(1_800_000_000, 0), 1_800_000_000);
    }

    #[test]
    fn the_zone_offset_comes_from_tzdata() {
        // Skipped where the test machine has no tzdata.
        if Path::new(ZONEINFO).join("Etc/GMT-2").is_file() {
            assert_eq!(zone_offset("Etc/GMT-2", 1_800_000_000), Ok(2 * 3600));
        }
    }
}
