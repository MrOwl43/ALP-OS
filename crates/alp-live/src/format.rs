// SPDX-License-Identifier: AGPL-3.0-or-later
//! Format an external disk (T49, Advanced > Disks), quick or secure
//! (owner, 2026-11-03), under the Security and Network Expert's conditions
//! (ux-0.2.4-security-review.md section 4; review of patch 0005):
//!
//! - Only an external (USB) disk. Never the stick, the computer's own
//!   disks, or the disk the archive is on: the server checks this again at
//!   the call, by disk id (`diskseq`) and PARTUUID, whatever the screen
//!   sent. A disk with anything mounted (an open Import or Files view) is
//!   refused until it is ejected.
//! - **Quick:** a new partition table and one exFAT partition. Fast; the
//!   old files can often still be recovered with special tools.
//! - **Secure:** `blkdiscard` where the disk supports it, then every byte
//!   overwritten with zeros ([`crate::wipe`]), then the same new partition
//!   table and exFAT. On USB flash and SSDs, wear-levelling can keep old
//!   copies in spare blocks: very hard to recover, not impossible. If only
//!   the edges could be overwritten, the job fails with "only partly
//!   erased" and never says Done (review W2). Can be stopped; the disk is
//!   then partly erased and has no file system.
//! - The disk is writable only while the job runs, through its own
//!   allowance file ([`ALLOWED_FORMAT`]), so the archive's allowance
//!   ([`archive::ALLOWED`]) is never touched while an archive is open.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use crate::archive::{self, Part, PrepareRefusal};
use crate::job::{End, Progress, Update};
use crate::wipe::{self, Method, Mode};

/// The disk being formatted, as its disk id ([`archive::disk_key`]: kernel
/// name and `diskseq`, so a disk plugged in later under the same name never
/// matches; review G1); `protect-disks` leaves it writable while this file
/// names it. Removed when the job ends (also on a panic, [`Allowance`]) and
/// when alpd starts ([`clear_stale`]).
pub const ALLOWED_FORMAT: &str = "/run/alp/allowed-format";

/// Does the allowance text `stored` name the disk whose id is now
/// `disk_id`? Exact match of the whole id only.
pub fn allowance_matches(stored: &str, disk_id: &str) -> bool {
    let s = stored.trim();
    !s.is_empty() && archive::disk_of_id(s).is_some() && s == disk_id
}

/// Is `disk` (kernel name) the disk a format job is writing now?
/// (`protect-disks` leaves it alone; Import and Files refuse it.)
pub fn formatting(disk: &str) -> bool {
    fs::read_to_string(ALLOWED_FORMAT)
        .is_ok_and(|s| allowance_matches(&s, &archive::disk_key(disk)))
}

/// At alpd start no format job runs: an allowance left by a crash goes.
pub fn clear_stale() {
    if fs::remove_file(ALLOWED_FORMAT).is_ok() {
        eprintln!("ALP-OS Live: removed a format allowance left from an earlier run");
    }
}

/// The allowance while a format job runs. Dropped (also on a panic): the
/// disk is made read-only again and the file removed.
struct Allowance {
    disk: String,
}

impl Allowance {
    fn take(disk: &str, disk_id: &str) -> Result<Self, String> {
        fs::create_dir_all("/run/alp").map_err(|e| e.to_string())?;
        fs::write(ALLOWED_FORMAT, disk_id).map_err(|e| e.to_string())?;
        Ok(Self { disk: disk.into() })
    }
}

impl Drop for Allowance {
    fn drop(&mut self) {
        let _ = Command::new("blockdev")
            .args(["--setro", &format!("/dev/{}", self.disk)])
            .output();
        let _ = fs::remove_file(ALLOWED_FORMAT);
    }
}

/// Formatting failed or was only partly done.
pub const CODE_FORMAT_FAILED: &str = "ALP-STO-122";
/// The disk may not be formatted (refused at the check).
pub const CODE_NO_FORMAT: &str = "ALP-STO-123";

/// Shown before the user chooses (API text; the screen may shorten it but
/// must not promise more).
pub const QUICK_TEXT: &str = "Quick format makes a new, empty disk in a few seconds. The old files are not overwritten: they can often be brought back with special tools.";
pub const SECURE_TEXT: &str = "Secure format first overwrites the whole disk with zeros, then makes a new, empty disk. Old files are then very hard to recover. On USB sticks and SSDs a small part of the old data can stay in spare blocks the disk keeps for itself, so this is not a guarantee.";

/// The mode named by the API (`"quick"` or `"secure"`).
pub fn mode(name: &str) -> Option<Mode> {
    match name {
        "quick" => Some(Mode::Quick),
        "secure" => Some(Mode::Secure),
        _ => None,
    }
}

/// A volume label from the screen: 1 to 11 characters of A-Z, a-z, 0-9,
/// space, `-` and `_` (fits exFAT and FAT; nothing a shell or `mkfs` could
/// read as an option). Empty gives the default.
pub fn label(given: &str) -> Option<String> {
    let t = given.trim();
    if t.is_empty() {
        return Some("USB".into());
    }
    let ok = t.len() <= 11
        && !t.starts_with('-')
        && t.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_'));
    ok.then(|| t.to_string())
}

/// The decision of [`check`] (pure, for the tests): `now` = the disk's
/// current id (None: gone), `parts` = its partitions now, `archive` = the
/// PARTUUID the archive record points to, `in_use` = something of the
/// disk is mounted, `shown` = the PARTUUIDs the user saw.
pub fn decision(
    disk_id: &str,
    now: Option<&str>,
    parts: &[&Part],
    archive: Option<&str>,
    in_use: bool,
    shown: &[String],
) -> Result<(), PrepareRefusal> {
    if now != Some(disk_id) || parts.is_empty() {
        return Err(PrepareRefusal::NotFound);
    }
    let mut have: Vec<&str> = parts.iter().map(|p| p.partuuid.as_str()).collect();
    let mut want: Vec<&str> = shown.iter().map(String::as_str).collect();
    have.sort_unstable();
    want.sort_unstable();
    if have != want {
        return Err(PrepareRefusal::Changed);
    }
    let no = |m: &str| Err(PrepareRefusal::Refused(m.into()));
    if parts.iter().any(|p| !p.external) {
        return no("The computer's own disks are never formatted here.");
    }
    if let Some(a) = archive
        && parts.iter().any(|p| p.partuuid.eq_ignore_ascii_case(a))
    {
        return no("This is your backup disk. It is never formatted here.");
    }
    if parts.iter().any(|p| p.read_only) {
        return no("This disk is write-protected. If it has a lock switch, slide it to unlocked.");
    }
    if in_use {
        return no("This disk is open. Eject it first, then format it.");
    }
    Ok(())
}

/// Look at disk `disk_id` now and decide whether it may be formatted with
/// the partitions `shown`. Returns the disk's kernel name.
pub fn check(disk_id: &str, shown: &[String]) -> Result<String, PrepareRefusal> {
    let name = archive::disk_of_id(disk_id).ok_or(PrepareRefusal::NotFound)?;
    let now = Path::new("/sys/block")
        .join(name)
        .is_dir()
        .then(|| archive::disk_key(name));
    let all = archive::partitions();
    let parts: Vec<&Part> = all.iter().filter(|p| p.disk == name).collect();
    let rec = archive::record().map(|r| r.partuuid);
    decision(
        disk_id,
        now.as_deref(),
        &parts,
        rec.as_deref(),
        archive::disk_in_use(name),
        shown,
    )?;
    Ok(name.to_string())
}

/// What the screen shows before the user chooses: size, and the minutes a
/// secure format will take ("about": from a read of the disk, nothing is
/// written; the running job switches to the measured speed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Estimate {
    pub size_bytes: u64,
    pub bytes_per_second: u64,
    pub secure_secs: u64,
    /// What the last screen must name before the slide (review G3):
    /// [`WARN_BITLOCKER`], [`WARN_WINDOWS`], [`WARN_ALPARCHIVE`].
    pub warnings: Vec<&'static str>,
    /// Bytes in use on the partitions that could be looked at (read-only),
    /// None when none could.
    pub bytes_used: Option<u64>,
}

/// A BitLocker-encrypted part: its data is lost for good.
pub const WARN_BITLOCKER: &str = "bitlocker";
/// Windows or another system's partitions (EFI, Microsoft reserved or
/// recovery, Windows dynamic disk / LDM): the disk may start a computer.
pub const WARN_WINDOWS: &str = "windows";
/// An ALPArchive folder (an archive of another stick, or an old one).
pub const WARN_ALPARCHIVE: &str = "alparchive_folder";
/// A Linux system (root, /usr or swap partition) or another installed
/// operating system: the disk may start a computer (review R3).
pub const WARN_SYSTEM: &str = "system";

/// Partition types of a Linux system disk (Discoverable Partitions
/// Specification; x86-64 root and /usr, and swap).
const LINUX_SYSTEM_TYPES: [&str; 3] = [
    "4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709", // root (x86-64)
    "8484680C-9521-48C6-9C11-B0720656F69E", // /usr (x86-64)
    "0657FD6D-A4AB-43C4-84E5-0933C84B4F4F", // swap
];

/// Warnings for a disk from its candidates (read-only looks, as in
/// `archive/candidates`). Formatting such a disk is allowed (it is
/// external); the user must only see what is on it.
pub fn warnings(
    entries: &[&(Part, Result<archive::Probe, archive::Refusal>)],
) -> Vec<&'static str> {
    let mut w = Vec::new();
    let mut add = |x| {
        if !w.contains(&x) {
            w.push(x);
        }
    };
    for (p, v) in entries {
        match v {
            Err(r) if r.code == archive::CODE_BITLOCKER => add(WARN_BITLOCKER),
            Err(r) if r.code == archive::CODE_OS => add(WARN_WINDOWS),
            Ok(pr) if pr.has_archive => add(WARN_ALPARCHIVE),
            Ok(pr) if pr.os => add(WARN_SYSTEM),
            _ => {}
        }
        if LINUX_SYSTEM_TYPES.contains(&p.type_uuid.to_ascii_uppercase().as_str()) {
            add(WARN_SYSTEM);
        }
        if p.fs_type.eq_ignore_ascii_case("BitLocker") {
            add(WARN_BITLOCKER);
        }
    }
    w
}

/// Bytes in use, from the read-only looks of the candidates (pure).
pub fn bytes_used(entries: &[&(Part, Result<archive::Probe, archive::Refusal>)]) -> Option<u64> {
    let seen: Vec<u64> = entries
        .iter()
        .filter_map(|(p, v)| {
            v.as_ref()
                .ok()
                .map(|pr| p.size_bytes.saturating_sub(pr.free_bytes))
        })
        .collect();
    (!seen.is_empty()).then(|| seen.iter().sum())
}

/// Write speed assumed when the read sample failed (a slow USB 2 stick).
pub const FALLBACK_BPS: u64 = 10_000_000;

pub fn estimate(disk_id: &str, shown: &[String]) -> Result<Estimate, PrepareRefusal> {
    let name = check(disk_id, shown)?;
    let dev = Path::new("/dev").join(&name);
    let size_bytes = wipe::size_of(&dev).map_err(|_| PrepareRefusal::NotFound)?;
    let bytes_per_second = wipe::guess_write_speed(&dev).unwrap_or(FALLBACK_BPS);
    let all = archive::candidates();
    let mine: Vec<_> = all.iter().filter(|(p, _)| p.disk == name).collect();
    Ok(Estimate {
        size_bytes,
        bytes_per_second,
        secure_secs: wipe::estimate_secs(size_bytes, bytes_per_second),
        warnings: warnings(&mine),
        bytes_used: bytes_used(&mine),
    })
}

/// Progress text during the overwrite, e.g. "Overwriting with zeros… 42%
/// (about 7 min left)".
pub fn overwrite_text(done: u64, total: u64, bps: u64) -> (String, u8) {
    let pct = done.saturating_mul(100) / total.max(1);
    let left = total.saturating_sub(done) / bps.max(1);
    (
        format!(
            "Overwriting with zeros… {pct}% (about {} min left)",
            left.div_ceil(60).max(1)
        ),
        // 5..85 % of the job.
        5 + u8::try_from(pct * 80 / 100).unwrap_or(80),
    )
}

/// The job `disk.format`: check again, (secure: overwrite), new partition
/// table and exFAT, read-only again. Returns the method used.
pub fn format(
    disk_id: &str,
    shown: &[String],
    mode: Mode,
    fs_kind: Fs,
    label: &str,
    cancel: &AtomicBool,
    say: Progress,
) -> Result<Method, End> {
    let fail = |step, e: String| End::failed(CODE_FORMAT_FAILED, step, e);
    say(&Update::step("checking", "Checking the disk…", 2));
    let disk = check(disk_id, shown).map_err(|r| {
        End::failed(
            CODE_NO_FORMAT,
            "checking",
            match r {
                PrepareRefusal::NotFound => "the disk is not plugged in any more".into(),
                PrepareRefusal::Changed => "the disk's partitions changed".into(),
                PrepareRefusal::Refused(m) => m,
            },
        )
    })?;
    eprintln!(
        "{} disk.format: {disk_id} ({}) (password checked, list confirmed)",
        crate::job::LOG_PREFIX,
        if mode == Mode::Secure {
            "secure"
        } else {
            "quick"
        }
    );
    let allowance = Allowance::take(&disk, disk_id).map_err(|e| fail("checking", e))?;
    let res = run(&disk, disk_id, mode, fs_kind, label, cancel, say);
    // Read-only again in every case, partitions first, then the disk.
    let mut back: Vec<String> = archive::partitions()
        .into_iter()
        .filter(|p| p.disk == disk && p.name != disk)
        .map(|p| p.name)
        .collect();
    back.push(disk.clone());
    for d in &back {
        if let Err(e) = Command::new("blockdev")
            .args(["--setro", &format!("/dev/{d}")])
            .output()
            .map_err(|e| e.to_string())
            .and_then(|o| {
                o.status
                    .success()
                    .then_some(())
                    .ok_or_else(|| String::from_utf8_lossy(&o.stderr).trim().to_string())
            })
        {
            eprintln!(
                "{} disk.format: {d} stays writable: {e}",
                crate::job::LOG_PREFIX
            );
        }
    }
    drop(allowance);
    let _ = Command::new("udevadm")
        .args(["trigger", "--action=change", &format!("/dev/{disk}")])
        .output();
    let _ = Command::new("udevadm")
        .args(["settle", "--timeout=30"])
        .output();
    let m = match res {
        Ok(m) => m,
        Err(e) if e == wipe::CANCELLED => return Err(End::Cancelled),
        Err(e) => return Err(fail("formatting", e)),
    };
    eprintln!(
        "{} disk.format: {disk_id}: {}",
        crate::job::LOG_PREFIX,
        m.describe()
    );
    if !m.complete() {
        // Review W2: never "Done" after an edges-only overwrite.
        return Err(fail(
            "erasing",
            format!(
                "the disk was only partly erased ({}); it has a new, empty file system, but old files may still be found",
                m.describe()
            ),
        ));
    }
    say(&Update::step("done", "The disk is formatted.", 100));
    Ok(m)
}

fn run(
    disk: &str,
    disk_id: &str,
    mode: Mode,
    fs_kind: Fs,
    label: &str,
    cancel: &AtomicBool,
    say: Progress,
) -> Result<Method, String> {
    let dev = Path::new("/dev").join(disk);
    // The same disk, still external (review R2), before each write.
    let same = || {
        (archive::disk_key(disk) == disk_id && crate::disks::is_external(disk))
            .then_some(())
            .ok_or_else(|| "the disk was unplugged; nothing was erased on another disk".to_string())
    };
    let m = match mode {
        // Quick: no discard (that would already destroy data the user may
        // want back from a "quick" format), no overwrite.
        Mode::Quick => Method::Quick { discarded: false },
        Mode::Secure => {
            same()?;
            let o = Command::new("blockdev")
                .args(["--setrw", &dev.to_string_lossy()])
                .output()
                .map_err(|e| e.to_string())?;
            if !o.status.success() {
                return Err(format!(
                    "the disk stays read-only: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                ));
            }
            say(&Update::step("erasing", "Overwriting with zeros…", 5));
            wipe::wipe(&dev, Mode::Secure, cancel, &mut |done, total, bps| {
                let (text, pct) = overwrite_text(done, total, bps);
                let mut u = Update::step("erasing", text, pct);
                u.done_bytes = Some(done);
                u.total_bytes = Some(total);
                u.bytes_per_second = Some(bps);
                say(&u);
            })?
        }
    };
    same()?;
    say(&Update::step("formatting", "Making a new, empty disk…", 88));
    archive::erase_and_format(disk, disk_id, ALLOWED_FORMAT, label, fs_kind, 92, say)?;
    Ok(m)
}

/// The file system a format makes: exFAT (the default; every computer and
/// camera reads it) or ext4 (Linux).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fs {
    Exfat,
    Ext4,
}

impl Fs {
    pub fn name(self) -> &'static str {
        match self {
            Fs::Exfat => "exfat",
            Fs::Ext4 => "ext4",
        }
    }
}

/// The file system named by the API (`"exfat"`, `"ext4"`; absent = exFAT).
pub fn fs_named(name: &str) -> Option<Fs> {
    match name {
        "exfat" => Some(Fs::Exfat),
        "ext4" => Some(Fs::Ext4),
        _ => None,
    }
}

/// A label for `fs`: [`label`]'s characters, up to 11 (exFAT) or 16 (ext4).
pub fn label_for(given: &str, fs: Fs) -> Option<String> {
    let t = given.trim();
    let max = match fs {
        Fs::Exfat => 11,
        Fs::Ext4 => 16,
    };
    if t.is_empty() {
        return Some("USB".into());
    }
    let ok = t.len() <= max
        && !t.starts_with('-')
        && t.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_'));
    ok.then(|| t.to_string())
}

/// How long a confirmation from `disks/format/estimate` stays good.
pub const CONFIRM_TTL: Duration = Duration::from_secs(5 * 60);

/// One-time confirmations (review R4): `disks/format/estimate` issues one
/// per disk, bound to what the person saw (disk id with diskseq, size,
/// PARTUUIDs, a whole-disk file system); `disks/format` must bring it back
/// within [`CONFIRM_TTL`], and any attempt uses it up. This guards against
/// races (the disk changed between the screens), not against an attacker:
/// the password is the access control. Kept in memory only, never logged,
/// all dropped when the console locks or a new sign-in starts.
#[derive(Default)]
pub struct Confirms {
    by_disk: HashMap<String, ([u8; 16], String, Instant)>,
}

impl Confirms {
    /// A new confirmation for `disk_id` with `binding` (replaces an older
    /// one for the same disk). None if the kernel gave no random bytes.
    pub fn issue(&mut self, disk_id: &str, binding: String) -> Option<String> {
        let mut b = [0u8; 16];
        fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut b))
            .ok()?;
        if b.iter().all(|x| *x == 0) {
            return None;
        }
        let hex = b.iter().map(|x| format!("{x:02x}")).collect();
        self.by_disk
            .insert(disk_id.to_string(), (b, binding, Instant::now()));
        Some(hex)
    }

    /// Use up the confirmation for `disk_id`: true only if `given` is the
    /// one issued, for the same `binding`, and not older than the limit.
    pub fn take(&mut self, disk_id: &str, binding: &str, given: &str) -> bool {
        self.take_at(disk_id, binding, given, Instant::now())
    }

    fn take_at(&mut self, disk_id: &str, binding: &str, given: &str, now: Instant) -> bool {
        let Some((want, bound, at)) = self.by_disk.remove(disk_id) else {
            return false;
        };
        let Some(got) = unhex16(given) else {
            return false;
        };
        // Constant time over the 16 bytes (review R4).
        let same = want
            .iter()
            .zip(got.iter())
            .fold(0u8, |a, (x, y)| a | (x ^ y))
            == 0;
        same && bound == binding && now.duration_since(at) <= CONFIRM_TTL
    }

    /// Forget every confirmation (console lock, new sign-in).
    pub fn clear(&mut self) {
        self.by_disk.clear();
    }
}

fn unhex16(s: &str) -> Option<[u8; 16]> {
    if s.len() != 32 || !s.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// What a confirmation is bound to (pure): the disk id (name and
/// diskseq), its size, its sorted PARTUUIDs and, for a disk without a
/// partition table, the file system written straight onto it.
pub fn binding_of(disk_id: &str, size_bytes: u64, parts: &[&Part]) -> String {
    let mut ids: Vec<String> = parts
        .iter()
        .map(|p| p.partuuid.to_ascii_lowercase())
        .collect();
    ids.sort();
    let whole = match parts {
        [p] if p.name == p.disk => format!("{}:{}", p.fs_type, p.fs_uuid),
        _ => String::new(),
    };
    format!("{disk_id}|{size_bytes}|{}|{whole}", ids.join(","))
}

/// [`binding_of`] for the disk `name` as it is now.
pub fn binding(disk_id: &str, name: &str) -> String {
    let size = wipe::size_of(&Path::new("/dev").join(name)).unwrap_or(0);
    let all = archive::partitions();
    let parts: Vec<&Part> = all.iter().filter(|p| p.disk == name).collect();
    binding_of(disk_id, size, &parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(name: &str, uuid: &str, external: bool) -> Part {
        Part {
            external,
            name: name.into(),
            disk: "sdb".into(),
            partuuid: uuid.into(),
            type_uuid: String::new(),
            fs_type: "exfat".into(),
            fs_uuid: String::new(),
            label: String::new(),
            size_bytes: 1 << 30,
            read_only: false,
        }
    }

    fn shown(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn only_an_external_unused_non_archive_disk_as_seen() {
        let a = part("sdb1", "aaaa", true);
        let b = part("sdb2", "bbbb", true);
        let ps = [&a, &b];
        let ok =
            |arch, used, s: &[&str]| decision("sdb-7", Some("sdb-7"), &ps, arch, used, &shown(s));
        assert_eq!(ok(None, false, &["bbbb", "aaaa"]), Ok(()));
        // Swapped or re-plugged disk, or partitions changed since the list.
        assert_eq!(
            decision(
                "sdb-7",
                Some("sdb-8"),
                &ps,
                None,
                false,
                &shown(&["aaaa", "bbbb"])
            ),
            Err(PrepareRefusal::NotFound)
        );
        assert_eq!(ok(None, false, &["aaaa"]), Err(PrepareRefusal::Changed));
        // The archive's disk, by PARTUUID of any of its partitions.
        assert!(matches!(
            ok(Some("BBBB"), false, &["aaaa", "bbbb"]),
            Err(PrepareRefusal::Refused(m)) if m.contains("backup disk")
        ));
        // Mounted (Import / Files open).
        assert!(matches!(
            ok(None, true, &["aaaa", "bbbb"]),
            Err(PrepareRefusal::Refused(m)) if m.contains("Eject")
        ));
        // An internal disk, or a write-protected one.
        let i = part("sda1", "iiii", false);
        assert!(matches!(
            decision("sda-1", Some("sda-1"), &[&i], None, false, &shown(&["iiii"])),
            Err(PrepareRefusal::Refused(m)) if m.contains("own disks")
        ));
        let mut r = part("sdb1", "rrrr", true);
        r.read_only = true;
        assert!(matches!(
            decision("sdb-7", Some("sdb-7"), &[&r], None, false, &shown(&["rrrr"])),
            Err(PrepareRefusal::Refused(m)) if m.contains("write-protected")
        ));
        assert_eq!(
            decision("sdb-7", Some("sdb-7"), &[], None, false, &[]),
            Err(PrepareRefusal::NotFound)
        );
    }

    #[test]
    fn warnings_name_what_is_on_the_disk() {
        let refused = |code| {
            Err(archive::Refusal {
                code,
                reason: String::new(),
            })
        };
        let esp = (part("sdb1", "e", true), refused(archive::CODE_OS));
        let bl = (part("sdb2", "b", true), refused(archive::CODE_BITLOCKER));
        let alp = (
            part("sdb3", "a", true),
            Ok(archive::Probe {
                has_archive: true,
                ..Default::default()
            }),
        );
        let plain = (part("sdb4", "p", true), Ok(archive::Probe::default()));
        assert_eq!(
            warnings(&[&esp, &bl, &alp, &plain, &esp]),
            vec![WARN_WINDOWS, WARN_BITLOCKER, WARN_ALPARCHIVE]
        );
        assert!(warnings(&[&plain]).is_empty());
    }

    #[test]
    fn the_allowance_names_one_plug_in_of_one_disk() {
        assert!(allowance_matches("sdb-17\n", "sdb-17"));
        // Same name, plugged in later (new diskseq): not allowed.
        assert!(!allowance_matches("sdb-17", "sdb-18"));
        // A bare kernel name (old format) or anything else never matches.
        assert!(!allowance_matches("sdb", "sdb-"));
        assert!(!allowance_matches("sdb", "sdb"));
        assert!(!allowance_matches("", "sdb-17"));
        assert!(!allowance_matches("sdb-1", "sdb-17"));
    }

    #[test]
    fn modes_and_labels_from_the_screen() {
        assert_eq!(mode("quick"), Some(Mode::Quick));
        assert_eq!(mode("secure"), Some(Mode::Secure));
        assert_eq!(mode("Secure"), None);
        assert_eq!(label("").as_deref(), Some("USB"));
        assert_eq!(label(" Photos 2026 ").as_deref(), Some("Photos 2026"));
        assert_eq!(label("-L x"), None);
        assert_eq!(label("a;rm -rf"), None);
        assert_eq!(label("twelve chars"), None);
        assert_eq!(label("Fotos-ü"), None);
    }

    #[test]
    fn overwrite_progress_text() {
        let (t, p) = overwrite_text(50, 100, 1);
        assert_eq!(p, 45);
        assert!(t.starts_with("Overwriting with zeros… 50% (about 1 min left)"));
        let (t, p) = overwrite_text(0, 32_000_000_000, 20_000_000);
        assert_eq!(p, 5);
        assert!(t.contains("about 27 min left"), "{t}");
        assert_eq!(overwrite_text(100, 100, 0).1, 85);
    }

    #[test]
    fn the_texts_do_not_promise_too_much() {
        assert!(QUICK_TEXT.contains("can often be brought back"));
        assert!(SECURE_TEXT.contains("not a guarantee"));
        assert!(!SECURE_TEXT.to_lowercase().contains("impossible"));
    }

    #[test]
    fn file_systems_and_their_labels() {
        assert_eq!(fs_named("exfat"), Some(Fs::Exfat));
        assert_eq!(fs_named("ext4"), Some(Fs::Ext4));
        assert_eq!(fs_named("ntfs"), None);
        assert_eq!(label_for("", Fs::Ext4).as_deref(), Some("USB"));
        assert_eq!(
            label_for("Sixteen chars ok", Fs::Ext4).as_deref(),
            Some("Sixteen chars ok")
        );
        assert_eq!(label_for("Sixteen chars ok", Fs::Exfat), None);
        assert_eq!(label_for("Seventeen chars!!", Fs::Ext4), None);
        assert_eq!(label_for("-L", Fs::Ext4), None);
        assert_eq!(label_for("a;b", Fs::Ext4), None);
    }

    #[test]
    fn a_confirmation_is_bound_single_use_and_short_lived() {
        let mut c = Confirms::default();
        let t = c.issue("sdb-7", "b1".into()).unwrap();
        assert_eq!(t.len(), 32);
        assert!(c.take("sdb-7", "b1", &t));
        assert!(!c.take("sdb-7", "b1", &t), "used up");
        // Another disk, another binding, a wrong token: refused, and used up.
        let t = c.issue("sdb-7", "b1".into()).unwrap();
        assert!(!c.take("sdc-1", "b1", &t));
        assert!(!c.take("sdb-7", "b2", &t));
        let t = c.issue("sdb-7", "b1".into()).unwrap();
        assert!(!c.take("sdb-7", "b1", "00000000000000000000000000000000"));
        assert!(!c.take("sdb-7", "b1", &t), "a wrong try used it up");
        assert!(!c.take("sdb-7", "b1", "zz"));
        // A newer one replaces the older one for the same disk.
        let old = c.issue("sdb-7", "b1".into()).unwrap();
        let new = c.issue("sdb-7", "b1".into()).unwrap();
        assert_ne!(old, new);
        assert!(!c.take("sdb-7", "b1", &old));
        // Too old.
        let t = c.issue("sdb-7", "b1".into()).unwrap();
        let later = Instant::now() + CONFIRM_TTL + Duration::from_secs(1);
        assert!(!c.take_at("sdb-7", "b1", &t, later));
        // Cleared on lock.
        let t = c.issue("sdb-7", "b1".into()).unwrap();
        c.clear();
        assert!(!c.take("sdb-7", "b1", &t));
    }

    #[test]
    fn the_binding_names_size_partitions_and_a_whole_disk_fs() {
        let a = part("sdb1", "AAAA", true);
        let b = part("sdb2", "bbbb", true);
        assert_eq!(binding_of("sdb-7", 100, &[&b, &a]), "sdb-7|100|aaaa,bbbb|");
        assert_ne!(
            binding_of("sdb-7", 100, &[&a]),
            binding_of("sdb-7", 200, &[&a])
        );
        let mut w = part("sdb", "", true);
        w.fs_uuid = "1234".into();
        assert_eq!(binding_of("sdb-7", 100, &[&w]), "sdb-7|100||exfat:1234");
    }
}
