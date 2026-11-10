// SPDX-License-Identifier: AGPL-3.0-or-later
//! ALP-OS Live encrypted archive on a backup disk the user picks.
//!
//! Storage v2 (ADR 0014, ALPHA 0.2): a new archive is a gocryptfs folder
//! `ALPArchive/data` that grows as photos arrive (nothing preallocated),
//! mounted decrypted at [`FILES_MNT`]; its password is random and lives in
//! the encrypted index on the stick ([`vault`](crate::vault) runs gocryptfs).
//! Before an archive is made the disk is repaired when it was not removed
//! safely, its SMART health is asked, and a 256 MiB write/read test checks
//! it. Archives of ALPHA 0.1 (one file `ALPArchive/archive.alp`, LUKS2 with
//! Btrfs inside, ADR 0009) still open, read and write as before; their
//! Btrfs subvolume `@archive` is bind-mounted at [`FILES_MNT`] so the rest of
//! the system sees one place. Live adds the `ALPArchive` folder and nothing
//! else; only "Prepare this disk" ([`prepare_disk`]) erases a whole disk,
//! behind the password and a list of what is erased. No own cryptography:
//! gocryptfs, cryptsetup, losetup and mkfs do the work.

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use alp_hw::inspect::{self, gb};

use crate::job::{End, RunError, Update};

/// Stick partition with Live's own state (image/repart.d/40-live-state.conf).
pub const STATE_LABEL: &str = "alp-state";
pub const STATE_DIR: &str = "/var/lib/alp";
const RECORD: &str = "/var/lib/alp/live/archive.toml";
/// Setup was finished or left on purpose (T09): kept on the stick's Live
/// state, so a restart opens the everyday screens (and asks for the
/// password there) instead of setup. Holds no secret.
const SETUP_DONE: &str = "/var/lib/alp/live/setup-done";

/// Has setup been finished (or left) on this stick?
pub fn setup_done() -> bool {
    Path::new(SETUP_DONE).exists()
}

/// Remember that setup is finished (written atomically, synced).
pub fn set_setup_done() -> Result<(), String> {
    write_atomic(SETUP_DONE, "done\n")
}

/// Forget it (reset everything: the next start is a first start).
pub fn clear_setup_done() {
    let _ = fs::remove_file(SETUP_DONE);
}
const HOST_MNT: &str = "/run/alp/host";
const PROBE_MNT: &str = "/run/alp/probe";
/// Why the archive did not open at start-up (empty: it opened).
pub const OPEN_ERROR: &str = "/run/alp/archive-open-error";
/// Root-only parent (0700) of the decrypted archive; an old archive's
/// Btrfs is mounted here (subvolumes below it).
pub const POOL_MNT: &str = "/var/lib/alp-pool";
/// The decrypted archive (ADR 0014): the gocryptfs mount of a new archive,
/// or a bind mount of an old archive's `@archive` subvolume.
pub const FILES_MNT: &str = "/var/lib/alp-pool/files";
/// Owner of the archive's files on exFAT/NTFS (no per-file owners there):
/// Immich's container user (T03).
pub const ARCHIVE_UID: u32 = 50000;
const MAPPER: &str = "alp-archive";
/// The partition the archive is on, made writable; read by protect-disks.
/// One line: `<partition or disk name> <disk id>` ([`disk_key`]: name and
/// `diskseq`), so a disk plugged in later under the same name never
/// matches (review G2). Written by [`allow`].
pub const ALLOWED: &str = "/run/alp/allowed-partition";

/// The allowance text for partition (or whole disk) `name` of `disk`.
pub fn allowance(name: &str, disk: &str) -> String {
    format!("{name} {}\n", disk_key(disk))
}

/// `(name, disk id)` from an allowance text; `None` when it is not
/// exactly that (an old name-only file counts as no allowance).
pub fn parse_allowance(s: &str) -> Option<(&str, &str)> {
    let mut it = s.split_whitespace();
    let (name, id) = (it.next()?, it.next()?);
    // A kernel name only: never ".", ".." or anything with a path in it.
    let plain = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    (plain && it.next().is_none() && disk_of_id(id).is_some()).then_some((name, id))
}

/// The archive's allowance now: the allowed name, if its disk id still
/// matches the disk plugged in under that disk name (`disk`).
pub fn allowed_on(text: &str, disk: &str, disk_id_now: &str) -> Option<String> {
    let (name, id) = parse_allowance(text)?;
    (disk_of_id(id) == Some(disk) && id == disk_id_now).then(|| name.to_string())
}

/// The name in the allowance file (for clean-up only; never to grant).
fn allowed_name() -> Option<String> {
    let s = fs::read_to_string(ALLOWED).ok()?;
    parse_allowance(&s).map(|(n, _)| n.to_string())
}

fn allow(name: &str, disk: &str) -> Result<(), String> {
    fs::create_dir_all("/run/alp").map_err(|e| e.to_string())?;
    fs::write(ALLOWED, allowance(name, disk)).map_err(|e| e.to_string())
}
const DIR: &str = "ALPArchive";
/// An old (ALPHA 0.1) archive file.
const FILE: &str = "archive.alp";
/// The encrypted folder of a new archive, and its name while it is made.
const DATA: &str = "data";
const DATA_PARTIAL: &str = "data.partial";
/// An unprotected archive (ADR 0016): the photos as plain files in
/// `ALPArchive/files`, and its id in `ALPArchive/plain-id.txt`.
const PLAIN: &str = "files";
const PLAIN_ID: &str = "plain-id.txt";
/// The master key wrapped by the recovery secret (ADR 0014).
const RECOVERY_CONF: &str = "recovery.conf";
/// A new recovery code waiting for its words to be checked (T42): it
/// becomes `recovery.conf` only on commit, so the old code keeps working
/// until then. alpd holds the matching words in memory only, so a pending
/// file found without them (restart, power cut) is stale and deleted.
const RECOVERY_PENDING: &str = "recovery.conf.pending";
/// ADR 0016 level 3: the same master key wrapped by the ALP password (via
/// gocryptfs' scrypt), so the password alone reopens the disk from another
/// stick. Never made with strong protection.
const PASSWORD_CONF: &str = "password.conf";
/// Marks an archive made with strong protection (ADR 0013 #6): it never
/// gets a `password.conf`, on whatever stick it is opened later.
const STRONG_MARK: &str = "strong";
/// scrypt cost (log2) for `password.conf`: 2^18 needs 256 MiB once per
/// try, four times gocryptfs' default; a family password guards it.
const PASSWORD_SCRYPTN: u8 = 18;

/// One config check or re-wrap at a time (they share /run/alp/conf-check).
static CONF_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
/// `password.conf` was made or checked with the current password since
/// alpd started: no second scrypt at every sign-in.
static PASSWORD_BOUND: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// The disk test's file (deleted after the test).
const TEST_FILE: &str = ".alp-disk-test";
/// Smallest room for an archive (owner, 2026-10-20): a disk needs this
/// much free space after the margin.
pub const MIN_BYTES: u64 = 1 << 30;
/// Safety margin left free on an external backup disk: 1 GiB or 5 % of
/// the free space, whichever is smaller (owner field test 2026-10-20: the
/// former 20 GB "for Windows" refused every USB stick; OS disks are never
/// used, so nothing else needs that room).
pub const MARGIN_BYTES: u64 = 1 << 30;
const MIB: u64 = 1 << 20;
/// Where the kernel's own read-only flag of a disk is remembered when it was
/// already set before `protect-disks` touched it (write-protect switch).
pub const HW_RO_DIR: &str = "/run/alp/hw-readonly";
/// Disks `protect-disks` made read-only (marker written before `--setro`).
pub const PROTECTED_DIR: &str = "/run/alp/protected";

/// GPT types that are never archive targets (Discoverable Partitions
/// and Microsoft's reserved / recovery types), upper case.
const SPECIAL_TYPES: [&str; 3] = [
    "C12A7328-F81F-11D2-BA4B-00A0C93EC93B", // EFI System
    "E3C9E316-0B5C-4DB8-817D-F92DF00215AE", // Microsoft reserved
    "DE94BBA4-06D1-4D40-A16A-BFD50179D6AC", // Windows recovery
];

/// One partition on a disk of this computer, as seen by Live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    /// On an external (USB) disk that is not the ALP-OS stick.
    pub external: bool,
    /// Kernel name, e.g. `nvme0n1p3`.
    pub name: String,
    pub disk: String,
    pub partuuid: String,
    pub type_uuid: String,
    pub fs_type: String,
    pub fs_uuid: String,
    pub label: String,
    pub size_bytes: u64,
    /// The disk itself is write-protected (switch or device), found before
    /// ALP-OS made it read-only.
    pub read_only: bool,
}

/// Facts found by mounting a partition read-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Probe {
    pub free_bytes: u64,
    /// Windows on this volume is hibernated or shut down with Fast Startup
    /// (its own `hiberfil.sys`).
    pub hibernated: bool,
    /// The volume was not unmounted cleanly (NTFS or exFAT dirty flag).
    pub dirty: bool,
    /// An archive folder from an earlier run is already there.
    pub has_archive: bool,
    /// An operating system is installed there (Windows or Linux).
    pub os: bool,
    /// Size of an unfinished `archive.alp.partial` of ours (counted in
    /// `free_bytes`: a new try reuses or replaces it).
    pub partial_bytes: u64,
}

/// GPT types that mean an operating system is on the disk (owner decision
/// 2026-10-19, ADR 0011): such a disk is never written. EFI System,
/// Microsoft reserved/recovery, Windows dynamic disks (LDM metadata and
/// data) and Storage Spaces, Linux root and /usr (Discoverable Partitions:
/// x86-64, x86, arm64).
const OS_TYPES: [&str; 12] = [
    "C12A7328-F81F-11D2-BA4B-00A0C93EC93B",
    "E3C9E316-0B5C-4DB8-817D-F92DF00215AE",
    "DE94BBA4-06D1-4D40-A16A-BFD50179D6AC",
    "5808C8AA-7E8F-42E0-85D2-E1E90434CFB3",
    "AF9B60A0-1431-4F62-BC68-3311714A69AD",
    "E75CAF8F-F680-4CEE-AFA3-B001E56EFC2D",
    "4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709",
    "8484680C-9521-48C6-9C11-B0720656F69E",
    "44479540-F297-41B2-9AF7-D131D5F0458A",
    "75250D76-8CC6-458E-BD66-BD47CC81A812",
    "B921B045-1DF0-41C3-AF44-4C6F280D3FAE",
    "B0E01050-EE5F-4390-949A-9101B17104E9",
];

/// A mounted filesystem holds an operating system: `\Windows\System32`
/// (any case) or `/etc/os-release` / `/usr/lib/os-release`.
pub fn has_os(root: &Path) -> bool {
    let child = |dir: &Path, name: &str| {
        fs::read_dir(dir).ok().and_then(|it| {
            it.flatten()
                .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name))
                .map(|e| e.path())
        })
    };
    let windows = child(root, "Windows").and_then(|w| child(&w, "System32"));
    windows.is_some_and(|p| p.is_dir())
        || root.join("etc/os-release").exists()
        || root.join("usr/lib/os-release").exists()
}

/// Why a disk or partition cannot hold the archive: a catalog code
/// (`errors/catalog.toml`, ALP-STO-100 to ALP-STO-107) and one plain sentence with the
/// action, ready to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub code: &'static str,
    pub reason: String,
}

impl Refusal {
    fn new(code: &'static str, reason: impl Into<String>) -> Self {
        Self {
            code,
            reason: reason.into(),
        }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason)
    }
}

pub const CODE_SPACE: &str = "ALP-STO-100";
pub const CODE_FAT32: &str = "ALP-STO-101";
pub const CODE_OS: &str = "ALP-STO-102";
pub const CODE_FORMAT: &str = "ALP-STO-103";
pub const CODE_READ_ONLY: &str = "ALP-STO-104";
pub const CODE_BITLOCKER: &str = "ALP-STO-105";
// ALP-STO-106 (Windows of this computer asleep) is no longer used: since
// 2026-10-20 only the external disk itself is checked (owner).
pub const CODE_DIRTY: &str = "ALP-STO-108";
pub const CODE_HIBERNATED: &str = "ALP-STO-109";
pub const CODE_UNREADABLE: &str = "ALP-STO-107";
/// Failed archive-creation jobs, by step (2026-10-21).
pub const CODE_PREPARE: &str = "ALP-STO-110";
pub const CODE_WRITE: &str = "ALP-STO-111";
pub const CODE_STALLED: &str = "ALP-STO-112";
pub const CODE_ENCRYPT: &str = "ALP-STO-113";
/// A failed "Start over" (reset job, including the index wipe).
pub const CODE_RESET: &str = "ALP-STO-121";
pub const CODE_FORMAT_ARCHIVE: &str = "ALP-STO-114";
/// Storage v2 (ADR 0014): disk checks, repair, prepare.
pub const CODE_SLOW: &str = "ALP-STO-115";
pub const CODE_TEST: &str = "ALP-STO-116";
pub const CODE_SMART: &str = "ALP-STO-117";
pub const CODE_REPAIR: &str = "ALP-STO-118";
pub const CODE_PREPARE_DISK: &str = "ALP-STO-119";
pub const CODE_NO_PREPARE: &str = "ALP-STO-120";

/// The one line shown for a disk that was not removed safely (candidate
/// `note`), and the job message while it is repaired.
pub const REPAIR_NOTE: &str =
    "This disk was not removed safely last time. ALPArchive will repair it first.";
pub const REPAIR_MESSAGE: &str = "This disk was not removed safely last time. Repairing it first…";

const OS_DISK: &str = "This disk has an operating system on it (for example Windows), so ALPArchive does not use it. Use a disk that only stores files.";
const USE_OTHER: &str = "Use a disk formatted as exFAT, NTFS or ext4, or another disk.";

fn os_disk() -> Refusal {
    Refusal::new(CODE_OS, OS_DISK)
}

/// Why a partition (or a whole disk without partitions) cannot hold the
/// archive; `None` = usable. Without a probe only what udev shows is
/// checked.
pub fn refusal(p: &Part, probe: Option<&Probe>) -> Option<Refusal> {
    if SPECIAL_TYPES.contains(&p.type_uuid.as_str()) {
        return Some(Refusal::new(
            CODE_OS,
            "This is a system partition of an operating system, so ALPArchive does not use it.",
        ));
    }
    match p.fs_type.as_str() {
        "ext4" | "ntfs" | "exfat" => {}
        "BitLocker" => {
            return Some(Refusal::new(
                CODE_BITLOCKER,
                "This disk is locked with BitLocker. Turn BitLocker off for it in Windows, or use another disk.",
            ));
        }
        "vfat" => {
            return Some(Refusal::new(
                CODE_FAT32,
                format!("FAT32 can't hold the archive. {USE_OTHER}"),
            ));
        }
        "" => {
            return Some(Refusal::new(
                CODE_FORMAT,
                format!("This disk is empty or its format is unknown. {USE_OTHER}"),
            ));
        }
        other => {
            return Some(Refusal::new(
                CODE_FORMAT,
                format!(
                    "This disk uses {}, which ALPArchive can't use yet. {USE_OTHER}",
                    crate::fail::display_safe(other, 20)
                ),
            ));
        }
    }
    if p.read_only {
        return Some(Refusal::new(
            CODE_READ_ONLY,
            "This disk is write-protected. If it has a lock switch, slide it to unlocked, then plug it in again.",
        ));
    }
    let probe = probe?;
    if let Some(why) = volume_refusal(probe) {
        return Some(why);
    }
    if max_size(probe.free_bytes) < MIN_BYTES {
        return Some(Refusal::new(
            CODE_SPACE,
            format!(
                "Not enough free space: {} free, at least {} needed. Free up space on the disk, or use a bigger one.",
                gb(probe.free_bytes),
                gb(min_free())
            ),
        ));
    }
    None
}

/// Why the volume itself may not be written now (hibernated Windows on
/// it), or `None`. A dirty flag is no refusal any more: it is repaired
/// first ([`ensure_clean`], ADR 0014); `CODE_DIRTY` is not returned.
pub fn volume_refusal(probe: &Probe) -> Option<Refusal> {
    if probe.hibernated {
        return Some(Refusal::new(
            CODE_HIBERNATED,
            "Windows on this disk is asleep or used Fast Startup. Start that Windows, hold Shift and click Shut down, then plug the disk in here again.",
        ));
    }
    None
}

/// The space kept free on the disk: 1 GiB or 5 % of the free space,
/// whichever is smaller.
pub fn margin(free: u64) -> u64 {
    MARGIN_BYTES.min(free / 20)
}

/// The room an archive has on a disk with `free` bytes free (whole MiB):
/// free space minus the margin. A new archive grows into it (ADR 0014);
/// the API's `default_size_bytes`/`max_size_bytes` show it to old clients.
pub fn max_size(free: u64) -> u64 {
    (free - margin(free)) / MIB * MIB
}

/// Free space a disk needs for the smallest archive (shown in the reason).
pub fn min_free() -> u64 {
    (MIN_BYTES * 20).div_ceil(19)
}

/// `hiberfil.sys` starting with "hibr" (any case) means Windows will resume
/// from it: the volume must not be written (the check ntfs-3g makes).
pub fn hiberfil_says_hibernated(head: &[u8]) -> bool {
    head.len() >= 4 && head[..4].eq_ignore_ascii_case(b"hibr")
}

fn read(path: &Path) -> String {
    fs::read_to_string(path)
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn udev(dev_numbers: &str, key: &str) -> String {
    fs::read_to_string(format!("/run/udev/data/b{dev_numbers}"))
        .unwrap_or_default()
        .lines()
        .find_map(|l| {
            l.strip_prefix("E:")?
                .strip_prefix(key)?
                .strip_prefix('=')
                .map(str::to_string)
        })
        .unwrap_or_default()
}

/// MBR extended-partition containers: they hold other partitions and no
/// filesystem, so they are not listed.
fn is_extended(scheme: &str, part_type: &str) -> bool {
    scheme == "dos" && matches!(part_type, "0x5" | "0xf" | "0x85")
}

/// Id of a disk without partitions in the place of a PARTUUID: from its
/// filesystem's UUID, so the archive is found again after a restart.
pub fn whole_disk_id(disk: &str, fs_uuid: &str) -> String {
    if fs_uuid.is_empty() {
        format!("disk-{disk}")
    } else {
        format!("disk-{fs_uuid}")
    }
}

/// A disk as plugged in this time: its name and the kernel's `diskseq`
/// (new for every plug-in; VERIFY kernel >= 5.15), so markers in /run never
/// carry over to another disk that gets the same name.
pub fn disk_key(disk: &str) -> String {
    let seq = read(&Path::new("/sys/block").join(disk).join("diskseq"));
    format!("{disk}-{seq}")
}

/// The disk was read-only before ALP-OS made it so (`protect-disks`): a
/// write-protect switch or a read-only device.
pub fn hw_read_only(disk: &str) -> bool {
    Path::new(HW_RO_DIR).join(disk_key(disk)).exists()
}

/// Partitions of the computer's disks (never the stick). A disk without
/// partitions is listed as one entry with `name == disk`: a filesystem
/// written straight onto the device, or nothing (empty).
pub fn partitions() -> Vec<Part> {
    let r = inspect::inspect(&inspect::Roots::system());
    let mut out = Vec::new();
    // Fail closed: without the stick found, it would be listed as an
    // external USB disk and could be formatted or opened (QA review 0.2.5).
    if !r.disks.iter().any(|d| d.is_system_disk) {
        eprintln!("alpd: the system stick was not found; no disk is offered");
        return out;
    }
    // One rule for every caller (ADR 0022): only a USB disk that is not
    // the stick is external; virtual devices are never listed.
    for (d, class) in crate::disks::classes(&r.disks) {
        if class == crate::disks::Class::Stick {
            continue;
        }
        let dir = Path::new("/sys/block").join(&d.id.0);
        let external = class == crate::disks::Class::External;
        let read_only = hw_read_only(&d.id.0);
        let mut names: Vec<String> = fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| dir.join(n).join("partition").is_file())
            .collect();
        names.sort();
        let whole = names.is_empty();
        if whole {
            names.push(d.id.0.clone());
        }
        for name in names {
            let node = if whole { dir.clone() } else { dir.join(&name) };
            let devno = read(&node.join("dev"));
            let fs_uuid = udev(&devno, "ID_FS_UUID");
            if !whole
                && is_extended(
                    &udev(&devno, "ID_PART_ENTRY_SCHEME"),
                    &udev(&devno, "ID_PART_ENTRY_TYPE"),
                )
            {
                continue;
            }
            out.push(Part {
                external,
                disk: d.id.0.clone(),
                partuuid: if whole {
                    whole_disk_id(&d.id.0, &fs_uuid)
                } else {
                    udev(&devno, "ID_PART_ENTRY_UUID")
                },
                type_uuid: if whole {
                    String::new()
                } else {
                    udev(&devno, "ID_PART_ENTRY_TYPE").to_ascii_uppercase()
                },
                fs_type: udev(&devno, "ID_FS_TYPE"),
                fs_uuid,
                label: udev(&devno, "ID_FS_LABEL"),
                size_bytes: read(&node.join("size")).parse::<u64>().unwrap_or(0) * 512,
                read_only,
                name,
            });
        }
    }
    out
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

fn fs_driver(fs_type: &str) -> &str {
    if fs_type == "ntfs" { "ntfs3" } else { fs_type }
}

/// Mount options of the backup disk. ro probes of ext4 never replay a
/// foreign journal (review S7). Written to, exFAT and NTFS (no per-file
/// owners) show every file as [`ARCHIVE_UID`]'s, readable only by it and
/// root (ADR 0014; exfat and ntfs3 options).
pub fn mount_options(fs_type: &str, rw: bool) -> String {
    match (rw, fs_type) {
        (true, "exfat" | "ntfs") => format!(
            "rw,nosuid,nodev,noexec,uid={ARCHIVE_UID},gid={ARCHIVE_UID},fmask=0077,dmask=0077"
        ),
        (true, _) => "rw,nosuid,nodev,noexec".into(),
        (false, "ext4") => "ro,noload,nosuid,nodev,noexec".into(),
        (false, _) => "ro,nosuid,nodev,noexec".into(),
    }
}

fn mount(p: &Part, at: &str, rw: bool) -> Result<(), String> {
    fs::create_dir_all(at).map_err(|e| e.to_string())?;
    let opts = mount_options(&p.fs_type, rw);
    run(
        "mount",
        &[
            "-t",
            fs_driver(&p.fs_type),
            "-o",
            &opts,
            &format!("/dev/{}", p.name),
            at,
        ],
    )
    .map(drop)
    .map_err(|e| {
        if rw && p.fs_type == "ntfs" {
            // ntfs3 refuses a rw mount of a volume Windows marked dirty
            // (never mounted with `force`; review S11, VERIFY on hardware).
            format!("Windows did not check this disk after it was last used. Start Windows once, then shut it down fully ({e})")
        } else {
            e
        }
    })
}

/// Device numbers `major:minor` of a path's filesystem.
fn dev_of(path: &str) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let d = fs::metadata(path).ok()?.dev();
    let major = ((d >> 8) & 0xfff) | ((d >> 32) & !0xfff);
    let minor = (d & 0xff) | ((d >> 12) & !0xff);
    Some(format!("{major}:{minor}"))
}

/// The stick's `alp-state` partition (found on the system disk only):
/// its kernel name.
fn state_partition() -> Result<String, String> {
    let r = inspect::inspect(&inspect::Roots::system());
    r.disks
        .iter()
        .filter(|d| d.is_system_disk)
        .flat_map(|d| &d.partitions)
        .find(|p| p.label == STATE_LABEL && !p.name.is_empty())
        .map(|p| p.name.clone())
        .ok_or_else(|| "state partition not found on the stick".into())
}

/// `/var/lib/alp` really is the stick's `alp-state` partition, so keys and
/// records written there survive a restart (review B2). Without it nothing
/// that must last may be written.
pub fn state_ready() -> Result<(), String> {
    let name = state_partition()?;
    let want = read(&Path::new("/sys/class/block").join(&name).join("dev"));
    if !want.is_empty() && dev_of(STATE_DIR).as_deref() == Some(want.as_str()) {
        Ok(())
    } else {
        Err("the stick's own storage is not ready, so nothing can be kept on it. Restart the computer with the stick in; if this stays, write ALP-OS to the stick again".into())
    }
}

fn free_bytes(at: &str) -> u64 {
    run("df", &["-B1", "--output=avail", at])
        .ok()
        .and_then(|s| s.lines().last().and_then(|l| l.trim().parse().ok()))
        .unwrap_or(0)
}

fn hibernated(at: &str) -> bool {
    let mut head = [0u8; 4];
    fs::File::open(Path::new(at).join("hiberfil.sys"))
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok()
        && hiberfil_says_hibernated(&head)
}

/// exFAT boot sector: `VolumeFlags` bit 1 (VolumeDirty) set? Offsets from
/// An ext4 superblock (the 1024 bytes at offset 1024) with the journal
/// still to be replayed: `s_magic` 0xEF53 at 0x38, `s_feature_incompat` at
/// 0x60 with `INCOMPAT_RECOVER` (0x4). `None`: not an ext2/3/4 superblock.
pub fn ext4_needs_recovery(sb: &[u8]) -> Option<bool> {
    let magic = u16::from_le_bytes([*sb.get(0x38)?, *sb.get(0x39)?]);
    if magic != 0xEF53 {
        return None;
    }
    let incompat = u32::from_le_bytes([
        *sb.get(0x60)?,
        *sb.get(0x61)?,
        *sb.get(0x62)?,
        *sb.get(0x63)?,
    ]);
    Some(incompat & 0x4 != 0)
}

/// Does the ext4 volume on `p` need its journal replayed (power cut)?
fn ext4_journal_pending(p: &Part) -> bool {
    let mut sb = [0u8; 1024];
    fs::File::open(format!("/dev/{}", p.name))
        .and_then(|mut f| {
            std::io::Seek::seek(&mut f, std::io::SeekFrom::Start(1024))?;
            f.read_exact(&mut sb)
        })
        .ok()
        .and_then(|()| ext4_needs_recovery(&sb))
        .unwrap_or(false)
}

/// Replay a pending ext4 journal: a read-write mount does it (the image has
/// no e2fsprogs), then unmount. Without this, the read-only probes
/// (`ro,noload`, needed because the disk is kept read-only) do not see what
/// was written last before a power cut, and the archive looked missing
/// (T-LIVE-02, 2026-10-29).
fn replay_journal(p: &Part) -> Result<(), String> {
    allow_write(p)?;
    let at = format!("{PROBE_MNT}/{}", p.name);
    mount(p, &at, true)?;
    unmount(&at)
}

/// Microsoft's exFAT specification, Main Boot Sector: FileSystemName at 3
/// ("EXFAT   "), VolumeFlags at 106 (2 bytes, little endian). `None`: not
/// an exFAT boot sector.
pub fn exfat_dirty(boot: &[u8]) -> Option<bool> {
    if boot.len() < 108 || &boot[3..11] != b"EXFAT   " {
        return None;
    }
    let flags = u16::from_le_bytes([boot[106], boot[107]]);
    Some(flags & 0b10 != 0)
}

/// ntfs3's `/proc/fs/ntfs3/<dev>/volinfo` (Linux 6.12 fs/ntfs3/super.c,
/// `ntfs3_volinfo`): line 6 is the dirty state read from `$Volume` at mount
/// (`real_dirty`), "dirty" or "clean". `None`: not in that form.
pub fn ntfs_volinfo_dirty(volinfo: &str) -> Option<bool> {
    match volinfo.lines().nth(5)?.trim() {
        "dirty" => Some(true),
        "clean" => Some(false),
        _ => None,
    }
}

/// The volume's own dirty flag, read without writing. An exFAT boot sector
/// that cannot be read, or ntfs3 without `volinfo`, counts as clean: the
/// rw mount still refuses a dirty NTFS volume.
fn dirty(p: &Part) -> bool {
    match p.fs_type.as_str() {
        "ntfs" => {
            let info = fs::read_to_string(format!("/proc/fs/ntfs3/{}/volinfo", p.name));
            info.ok()
                .and_then(|i| ntfs_volinfo_dirty(&i))
                .unwrap_or(false)
        }
        "exfat" => {
            let mut boot = [0u8; 512];
            fs::File::open(format!("/dev/{}", p.name))
                .and_then(|mut f| f.read_exact(&mut boot))
                .ok()
                .and_then(|()| exfat_dirty(&boot))
                .unwrap_or(false)
        }
        _ => false,
    }
}

/// Mount read-only, look, unmount. Writes nothing.
pub fn probe(p: &Part) -> Result<Probe, String> {
    let at = format!("{PROBE_MNT}/{}", p.name);
    mount(p, &at, false)?;
    let partial_bytes = fs::metadata(Path::new(&at).join(DIR).join(format!("{FILE}.partial")))
        .map_or(0, |m| m.len());
    let pr = Probe {
        free_bytes: free_bytes(&at).saturating_add(partial_bytes),
        hibernated: p.fs_type == "ntfs" && hibernated(&at),
        dirty: dirty(p),
        has_archive: kind_at(Path::new(&at)).is_some(),
        os: has_os(Path::new(&at)),
        partial_bytes,
    };
    unmount(&at)?;
    Ok(pr)
}

/// The two kinds of archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// ALPHA 0.1: `ALPArchive/archive.alp`, LUKS2 + Btrfs (ADR 0009).
    Luks,
    /// Storage v2: the gocryptfs folder `ALPArchive/data` (ADR 0014).
    Gocryptfs,
    /// Unprotected (ADR 0016): the plain folder `ALPArchive/files`.
    Plain,
}

/// The archive in the backup disk mounted at `root`, if any: a finished
/// gocryptfs folder (its config is there) or an old archive file.
pub fn kind_at(root: &Path) -> Option<Format> {
    let dir = root.join(DIR);
    if dir.join(DATA).join("gocryptfs.conf").is_file() {
        Some(Format::Gocryptfs)
    } else if dir.join(PLAIN_ID).is_file() && dir.join(PLAIN).is_dir() {
        Some(Format::Plain)
    } else if dir.join(FILE).is_file() {
        Some(Format::Luks)
    } else {
        None
    }
}

/// The repair program for a dirty volume (ADR 0014), run on the unmounted
/// partition: exfatprogs' `fsck.exfat -p` (repairs what is safe without
/// asking) or ntfs-3g's `ntfsfix -d` (clears the dirty flag when the volume
/// can be fixed). `None`: not a format that has a dirty flag.
pub fn repair_command(fs_type: &str, dev: &str) -> Option<(&'static str, Vec<String>)> {
    match fs_type {
        "exfat" => Some(("fsck.exfat", vec!["-p".into(), dev.into()])),
        "ntfs" => Some(("ntfsfix", vec!["-d".into(), dev.into()])),
        _ => None,
    }
}

/// Did the repair program succeed? fsck.exfat: 0 no errors, 1 errors
/// fixed (fsck(8) convention); ntfsfix: 0.
pub fn repair_ok(fs_type: &str, code: Option<i32>) -> bool {
    match fs_type {
        "exfat" => matches!(code, Some(0 | 1)),
        _ => code == Some(0),
    }
}

/// The volume may be written now (checked before every rw mount: create,
/// recover, open). Hibernated Windows on it is refused; a dirty volume ("not
/// removed safely") is repaired first, after `say` showed one line, and the
/// flag is read again; still dirty or a failed repair → [`CODE_REPAIR`].
/// The partition is made writable for the repair ([`allow_write`]).
pub fn ensure_clean(p: &Part, say: &mut dyn FnMut(&str)) -> Result<Probe, Refusal> {
    let unreadable = |e: String| {
        Refusal::new(
            CODE_UNREADABLE,
            format!(
                "ALPArchive could not read this disk ({e}). Unplug it, plug it in again and retry."
            ),
        )
    };
    if p.fs_type == "ext4" && ext4_journal_pending(p) {
        say(REPAIR_MESSAGE);
        replay_journal(p).map_err(|e| {
            Refusal::new(
                CODE_REPAIR,
                format!("This disk was not removed safely last time and ALPArchive could not finish its last writes ({e}). Unplug it, plug it in again and retry."),
            )
        })?;
        eprintln!(
            "{} repair: ext4 journal replayed on {}",
            crate::job::LOG_PREFIX,
            p.name
        );
    }
    let pr = probe(p).map_err(unreadable)?;
    if let Some(why) = volume_refusal(&pr) {
        return Err(why);
    }
    if !pr.dirty {
        return Ok(pr);
    }
    let failed = |e: String| {
        Refusal::new(
            CODE_REPAIR,
            format!(
                "This disk was not removed safely last time and the automatic repair did not work ({e}). Plug it into a Windows or Mac computer, let it check the disk, eject it properly, then plug it in here again, or use another disk."
            ),
        )
    };
    say(REPAIR_MESSAGE);
    let dev = format!("/dev/{}", p.name);
    let (cmd, args) = repair_command(&p.fs_type, &dev)
        .ok_or_else(|| failed(format!("no repair for {}", p.fs_type)))?;
    allow_write(p).map_err(failed)?;
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let code = crate::job::run_code(
        cmd,
        &args,
        &AtomicBool::new(false),
        Some(Duration::from_secs(10 * MINUTE)),
    )
    .map_err(|e| failed(e.to_string()))?
    .0;
    eprintln!(
        "{} repair: {cmd} on {} ended with {code:?}",
        crate::job::LOG_PREFIX,
        p.name
    );
    let res = if repair_ok(&p.fs_type, code) {
        probe(p).map_err(unreadable).and_then(|again| {
            if again.dirty {
                Err(failed(
                    "the disk still says it was not removed safely".into(),
                ))
            } else {
                Ok(again)
            }
        })
    } else {
        Err(failed(format!("{cmd} ended with {code:?}")))
    };
    if res.as_ref().is_err_and(|r| r.code == CODE_REPAIR) {
        let _ =
            fs::create_dir_all(REPAIR_FAILED_DIR).and_then(|()| fs::write(repair_marker(p), ""));
    } else {
        let _ = fs::remove_file(repair_marker(p));
    }
    res
}

/// [`ensure_clean`] for the API's plain-string paths (open, recover).
fn volume_ok(p: &Part) -> Result<(), String> {
    ensure_clean(p, &mut |m| eprintln!("{} {m}", crate::job::LOG_PREFIX))
        .map(drop)
        .map_err(|r| r.reason)
}

/// Make exactly one partition writable: every other partition of its disk
/// read-only first, then the disk and the partition writable; check sysfs.
pub fn allow_write(p: &Part) -> Result<(), String> {
    // Internal disks stay read-only, always (owner decision 2026-10-19).
    if !p.external {
        return Err("internal disks are never written".into());
    }
    let others: Vec<Part> = partitions()
        .into_iter()
        .filter(|o| o.disk == p.disk && o.name != p.name)
        .collect();
    for o in &others {
        run("blockdev", &["--setro", &format!("/dev/{}", o.name)])?;
    }
    allow(&p.name, &p.disk)?;
    let back = |e: String| {
        let _ = run("blockdev", &["--setro", &format!("/dev/{}", p.disk)]);
        e
    };
    run("blockdev", &["--setrw", &format!("/dev/{}", p.disk)]).map_err(back)?;
    run("blockdev", &["--setrw", &format!("/dev/{}", p.name)]).map_err(back)?;
    let ro = |n: &str| read(&Path::new("/sys/class/block").join(n).join("ro"));
    if ro(&p.name) != "0" {
        return Err(back(
            "the disk stays read-only (is it write-protected? If it has a lock switch, slide it to unlocked)".into(),
        ));
    }
    if others.iter().any(|o| ro(&o.name) != "1") {
        return Err(back("read-only flags are not as expected".into()));
    }
    Ok(())
}

/// The disk's own health (SMART) as `smartctl -H` answers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Ok,
    /// Exit bit 3 (disk failing) or bit 4 (a pre-fail attribute at or
    /// below its threshold).
    Failing,
    /// No answer from the disk (bits 0-2: command line, open, SMART command
    /// failed; a stick, a bridge without SAT, a disk in standby) or the
    /// program was not there.
    Unknown,
}

/// smartctl(8) "EXIT STATUS" is a bit mask.
pub fn smart_health(code: Option<i32>) -> Health {
    match code {
        Some(c) if c & 0b1_1000 != 0 => Health::Failing,
        Some(c) if c & 0b111 != 0 => Health::Unknown,
        Some(_) => Health::Ok,
        None => Health::Unknown,
    }
}

/// `smartctl -H -n standby [-d sat] /dev/<disk>`: health only, and never
/// wake a disk that sleeps.
pub fn smart_args(dev: &str, sat: bool) -> Vec<String> {
    let mut a: Vec<String> = ["-H", "-n", "standby"].map(String::from).to_vec();
    if sat {
        a.extend(["-d".to_string(), "sat".to_string()]);
    }
    a.push(dev.into());
    a
}

/// The SMART check of the checking step (ADR 0014): refused only when the
/// disk itself says it is failing; any other answer (no SMART, a stick, a
/// bridge without SAT, no answer within 30 s) is skipped silently.
fn smart_check(disk: &str, cancel: &AtomicBool) -> Result<(), End> {
    let dev = format!("/dev/{disk}");
    let ask = |sat: bool| -> Result<Health, End> {
        let args = smart_args(&dev, sat);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        match crate::job::run_code("smartctl", &args, cancel, Some(Duration::from_secs(30))) {
            Ok((code, _)) => Ok(smart_health(code)),
            Err(RunError::Cancelled) => Err(End::Cancelled),
            Err(_) => Ok(Health::Unknown),
        }
    };
    let mut h = ask(false)?;
    if h == Health::Unknown {
        h = ask(true)?;
    }
    eprintln!(
        "{} checking: SMART of {disk}: {h:?}",
        crate::job::LOG_PREFIX
    );
    if h == Health::Failing {
        return Err(End::failed(
            CODE_SMART,
            "checking",
            "the disk's own health check says it may fail soon",
        ));
    }
    Ok(())
}

/// Size of the disk test (ADR 0014), its blocks and how often it syncs.
pub const TEST_BYTES: u64 = 256 << 20;
const TEST_BLOCK: usize = 4 << 20;
const TEST_SYNC: u64 = 32 << 20;
/// Slowest average write speed accepted over the whole test (bytes per
/// second; owner decision 2026-10-25: 3 MB/s, low enough for slow but
/// healthy sticks, still catching bad ones), and the longest the test may
/// write: 256 MiB at 3 MB/s is 89.5 s, so 100 s only stops a test that has
/// already failed.
pub const MIN_SPEED: u64 = 3_000_000;
pub const MAX_WRITE_SECS: u64 = 100;

/// Block `n` of the test data seeded with `seed`: xorshift64* (not
/// cryptographic; only so a disk cannot answer from a pattern, compress or
/// deduplicate it, and every block differs).
pub fn pattern(seed: u64, n: u64, buf: &mut [u8]) {
    let mut x = (seed ^ n.wrapping_mul(0x9E37_79B9_7F4A_7C15)) | 1;
    for chunk in buf.chunks_mut(8) {
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        let v = x.wrapping_mul(0x2545_F491_4F6C_DD1D).to_le_bytes();
        chunk.copy_from_slice(&v[..chunk.len()]);
    }
}

/// Is a write of `bytes` in `secs` (until the last fsync) fast enough?
/// Under [`MIN_SPEED`], or longer than [`MAX_WRITE_SECS`], is too slow. A
/// write that took under a second is fast enough (256 MiB in a second is
/// far above the limit; tiny writes cannot be timed reliably).
pub fn speed_ok(bytes: u64, secs: f64) -> bool {
    secs <= MAX_WRITE_SECS as f64 && (secs < 1.0 || bytes as f64 >= MIN_SPEED as f64 * secs)
}

/// The progress of the disk test: 10 % to 34 % while writing.
pub fn checking_update(done: u64, total: u64, speed: Option<u64>) -> Update {
    Update {
        done_bytes: Some(done),
        total_bytes: Some(total),
        bytes_per_second: speed,
        ..Update::step(
            "checking",
            crate::job::checking_message(done, total, speed),
            10 + (done.min(total).saturating_mul(24) / total.max(1)) as u8,
        )
    }
}

fn test_failed(e: &std::io::Error) -> End {
    End::failed(
        CODE_TEST,
        "checking",
        format!("the read and write test failed, the disk may be faulty: {e}"),
    )
}

/// The read/write test (ADR 0014) in `dir`: `size` bytes of [`pattern`]
/// written to [`TEST_FILE`] in `block`-sized writes with an fsync every
/// `sync_every` bytes, then `remount` (so the read comes from the disk,
/// not the page cache), read back, compared, deleted. Too slow →
/// [`CODE_SLOW`]; an I/O error or a difference → [`CODE_TEST`]. `cancel`
/// stops it; the test file is deleted in every case. `progress` gets the
/// bytes on the disk and the speed after each fsync.
#[allow(clippy::too_many_arguments)]
pub fn disk_test(
    dir: &Path,
    size: u64,
    block: usize,
    sync_every: u64,
    seed: u64,
    cancel: &AtomicBool,
    remount: &mut dyn FnMut() -> Result<(), String>,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<(), End> {
    let file = dir.join(TEST_FILE);
    let res = disk_test_in(
        &file, size, block, sync_every, seed, cancel, remount, progress,
    );
    if let Err(e) = fs::remove_file(&file)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        eprintln!(
            "{} checking: the test file was not removed: {e}",
            crate::job::LOG_PREFIX
        );
    }
    let _ = sync_dir(dir);
    res
}

#[allow(clippy::too_many_arguments)]
fn disk_test_in(
    file: &Path,
    size: u64,
    block: usize,
    sync_every: u64,
    seed: u64,
    cancel: &AtomicBool,
    remount: &mut dyn FnMut() -> Result<(), String>,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<(), End> {
    let io = |e: std::io::Error| test_failed(&e);
    let slow = |done: u64, secs: f64| {
        End::failed(
            CODE_SLOW,
            "checking",
            format!(
                "the disk writes too slowly: {} MB in {secs:.0} s, at least {} MB/s are needed",
                done >> 20,
                MIN_SPEED / 1_000_000
            ),
        )
    };
    let mut buf = vec![0u8; block];
    let started = Instant::now();
    {
        // A file left from an earlier try (or a link) is replaced, never
        // followed.
        let mut f = write_new(file, b"").map_err(io)?;
        let (mut done, mut since_sync, mut n) = (0u64, 0u64, 0u64);
        progress(0, None);
        while done < size {
            check(cancel)?;
            let len = (size - done).min(block as u64) as usize;
            pattern(seed, n, &mut buf[..len]);
            f.write_all(&buf[..len]).map_err(io)?;
            done += len as u64;
            since_sync += len as u64;
            n += 1;
            if since_sync >= sync_every || done == size {
                f.sync_data().map_err(io)?;
                since_sync = 0;
                let secs = started.elapsed().as_secs_f64();
                progress(done, Some((done as f64 / secs.max(0.001)) as u64));
                if secs > MAX_WRITE_SECS as f64 {
                    return Err(slow(done, secs));
                }
            }
        }
        let secs = started.elapsed().as_secs_f64();
        if !speed_ok(done, secs) {
            return Err(slow(done, secs));
        }
    }
    check(cancel)?;
    remount().map_err(|e| {
        End::failed(
            CODE_TEST,
            "checking",
            format!("the disk did not come back after the test: {e}"),
        )
    })?;
    let mut f = fs::File::open(file).map_err(io)?;
    let mut want = vec![0u8; block];
    let (mut done, mut n) = (0u64, 0u64);
    while done < size {
        check(cancel)?;
        let len = (size - done).min(block as u64) as usize;
        f.read_exact(&mut buf[..len]).map_err(io)?;
        pattern(seed, n, &mut want[..len]);
        if buf[..len] != want[..len] {
            return Err(End::failed(
                CODE_TEST,
                "checking",
                format!(
                    "the data read back differs from what was written (at {} MB); the disk may be faulty",
                    done >> 20
                ),
            ));
        }
        done += len as u64;
        n += 1;
    }
    let mut extra = [0u8; 1];
    if f.read(&mut extra).map_err(io)? != 0 {
        return Err(End::failed(
            CODE_TEST,
            "checking",
            "the test file came back longer than written; the disk may be faulty",
        ));
    }
    Ok(())
}

/// A seed for the test data that differs every time.
fn test_seed() -> u64 {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    t ^ u64::from(std::process::id()).rotate_left(32)
}

/// The archive password (ADR 0014): 64 hex characters in the index.
pub fn pass_path() -> std::path::PathBuf {
    crate::index::path("keys/archive.pass")
}

/// The copy of the archive's gocryptfs config inside the index (like the
/// LUKS header backup, review S9).
fn conf_copy_path() -> std::path::PathBuf {
    crate::index::path("keys/archive-gocryptfs.conf")
}

/// 32 random bytes from the kernel as 64 lowercase hex characters.
fn new_password() -> Result<zeroize::Zeroizing<String>, String> {
    let mut bytes = zeroize::Zeroizing::new([0u8; 32]);
    fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes[..]))
        .map_err(|e| format!("no random numbers: {e}"))?;
    Ok(zeroize::Zeroizing::new(
        bytes.iter().map(|b| format!("{b:02x}")).collect(),
    ))
}

/// Write a secret file inside the index: mode 0600 from the start,
/// temporary name, fsync, rename, fsync of the folder.
fn write_secret(path: &Path, body: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    let _ = fs::remove_file(&tmp);
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|e| format!("{}: {e}", tmp.display()))?;
    f.write_all(body)
        .and_then(|()| f.sync_all())
        .map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    sync_dir(path.parent().unwrap_or(Path::new("/")))
}

/// The archive password from the index.
fn read_password() -> Result<zeroize::Zeroizing<String>, String> {
    if !crate::index::is_open() {
        return Err("the archive is locked; type your password first".into());
    }
    let s = zeroize::Zeroizing::new(
        fs::read_to_string(pass_path())
            .map_err(|_| "the archive's password is missing from the stick".to_string())?,
    );
    Ok(zeroize::Zeroizing::new(s.trim().to_string()))
}

/// Copy the archive's gocryptfs config into the index (after every change).
fn backup_conf(cipherdir: &Path) -> Result<(), String> {
    let conf = fs::read(cipherdir.join("gocryptfs.conf"))
        .map_err(|e| format!("the archive's config cannot be read: {e}"))?;
    write_secret(&conf_copy_path(), &conf)
}

/// Where Immich keeps its originals inside the decrypted archive (T03
/// bind-mounts it at `/run/alp/immich/originals`; T03 creates and chowns
/// it). The same path for old archives (their `@archive` subvolume is
/// mounted at [`FILES_MNT`]).
pub fn immich_originals_dir() -> std::path::PathBuf {
    Path::new(FILES_MNT).join("immich")
}

/// `/var/lib/alp-pool`, root-only (0700), unless an old archive's Btrfs is
/// mounted there.
fn pool_dir() -> Result<(), String> {
    fs::create_dir_all(POOL_MNT).map_err(|e| e.to_string())?;
    if !is_mounted(POOL_MNT) {
        fs::set_permissions(POOL_MNT, fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Mount a new archive's folder decrypted at [`FILES_MNT`].
fn mount_files(cipherdir: &Path) -> Result<(), String> {
    pool_dir()?;
    if !pass_path().is_file() {
        return Err("the archive's password is missing from the stick".into());
    }
    crate::vault::mount(&pass_path(), cipherdir, Path::new(FILES_MNT)).map_err(String::from)
}

/// An old archive: its `@archive` subvolume (mounted at [`POOL_MNT`]) is
/// bind-mounted at [`FILES_MNT`].
fn bind_files() -> Result<(), String> {
    if is_mounted(FILES_MNT) {
        return Ok(());
    }
    fs::create_dir_all(FILES_MNT).map_err(|e| e.to_string())?;
    run(
        "mount",
        &["--bind", &format!("{POOL_MNT}/@archive"), FILES_MNT],
    )
    .map(drop)
}

/// The archive's key file: inside the unlocked index on the stick
/// (ADR 0011), so the stick alone does not open the archive.
pub fn key_path() -> String {
    crate::index::path("keys/archive.key")
        .to_string_lossy()
        .into_owned()
}

/// Key file: 64 random bytes, mode 0600, created once.
fn key_file() -> Result<String, String> {
    state_ready()?;
    if !crate::index::is_open() {
        return Err("the archive is locked; type your password first".into());
    }
    let key = key_path();
    if Path::new(&key).exists() {
        return Ok(key);
    }
    let mut bytes = zeroize::Zeroizing::new([0u8; 64]);
    fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes[..]))
        .map_err(|e| e.to_string())?;
    let tmp = format!("{key}.tmp");
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|e| e.to_string())?;
    f.write_all(&bytes[..])
        .and_then(|()| f.sync_all())
        .map_err(|e| e.to_string())?;
    fs::rename(&tmp, &key).map_err(|e| e.to_string())?;
    sync_dir(Path::new(&key).parent().unwrap_or(Path::new("/")))?;
    Ok(key)
}

/// The record on the stick that finds the archive again on the next start.
/// Old records (ALPHA 0.1) have no `format` and a `luks_uuid`; new ones
/// `format = "gocryptfs"` and the folder's `archive_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub partuuid: String,
    pub fs_uuid: String,
    pub format: Format,
    /// The LUKS UUID (old) or the gocryptfs folder's id ([`crate::vault::archive_id`]).
    pub id: String,
}

impl Record {
    pub fn to_toml(&self) -> String {
        let mut t = toml::Table::new();
        t.insert("partuuid".into(), self.partuuid.clone().into());
        t.insert("fs_uuid".into(), self.fs_uuid.clone().into());
        match self.format {
            Format::Luks => {
                t.insert("luks_uuid".into(), self.id.clone().into());
                t.insert("path".into(), format!("{DIR}/{FILE}").into());
            }
            Format::Gocryptfs => {
                t.insert("format".into(), "gocryptfs".into());
                t.insert("archive_id".into(), self.id.clone().into());
                t.insert("path".into(), format!("{DIR}/{DATA}").into());
            }
            Format::Plain => {
                t.insert("format".into(), "plain".into());
                t.insert("archive_id".into(), self.id.clone().into());
                t.insert("path".into(), format!("{DIR}/{PLAIN}").into());
            }
        }
        t.to_string()
    }
    pub fn from_toml(s: &str) -> Option<Self> {
        let t: toml::Table = s.parse().ok()?;
        let g = |k: &str| t.get(k)?.as_str().map(str::to_string);
        let (format, id) = match g("format").as_deref() {
            None => (Format::Luks, g("luks_uuid")?),
            Some("gocryptfs") => (Format::Gocryptfs, g("archive_id")?),
            Some("plain") => (Format::Plain, g("archive_id")?),
            Some(_) => return None,
        };
        Some(Self {
            partuuid: g("partuuid")?,
            fs_uuid: g("fs_uuid")?,
            format,
            id,
        })
    }
}

/// Write a file atomically: temporary file, fsync, rename, fsync the
/// directory (without the last step a power cut can lose the rename).
fn write_atomic(path: &str, body: &str) -> Result<(), String> {
    let tmp = format!("{path}.tmp");
    let mut f = fs::File::create(&tmp).map_err(|e| e.to_string())?;
    f.write_all(body.as_bytes())
        .and_then(|()| f.sync_all())
        .map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    sync_dir(Path::new(path).parent().unwrap_or(Path::new("/")))
}

fn sync_dir(dir: &Path) -> Result<(), String> {
    fs::File::open(dir)
        .and_then(|d| d.sync_all())
        .map_err(|e| format!("{}: {e}", dir.display()))
}

fn attach(file: &Path) -> Result<String, String> {
    run(
        "losetup",
        &[
            "--find",
            "--show",
            "--sector-size",
            "4096",
            &file.to_string_lossy(),
        ],
    )
}

/// Progress callback of [`create`] and [`recover`].
pub type Progress<'a> = crate::job::Progress<'a>;

fn tell(say: &mut dyn FnMut(&Update), step: &'static str, message: &str, percent: u8) {
    say(&Update::step(step, message, percent));
}

/// Time limits of external programs (generous: slow USB 2 sticks).
const MINUTE: u64 = 60;

fn check(cancel: &AtomicBool) -> Result<(), End> {
    if cancel.load(Ordering::Relaxed) {
        Err(End::Cancelled)
    } else {
        Ok(())
    }
}

/// Create the archive on partition `partuuid` (ADR 0014): repair when
/// needed, check the disk (SMART, read/write test), make the encrypted
/// folder, mount it. Nothing is preallocated. Every failure ends with a
/// catalog code, the step and a plain cause; `cancel` stops it in every
/// step before the folder gets its final name. Cancelled or failed, only
/// what this job made is removed (the test file and the unfinished
/// `ALPArchive/data.partial`); the folder and its README stay.
pub fn create(partuuid: &str, cancel: &AtomicBool, say: Progress) -> Result<(), End> {
    let prep = |e: String| End::failed(CODE_PREPARE, "preparing", e);
    state_ready().map_err(prep)?;
    if !crate::index::is_open() {
        return Err(prep(
            "the archive is locked; type your password first".into(),
        ));
    }
    let parts = partitions();
    let p = only_one(&parts, partuuid).map_err(prep)?;
    if !p.external {
        return Err(prep(
            "only an external disk or USB stick can hold the archive".into(),
        ));
    }
    let os = parts.iter().filter(|o| o.disk == p.disk).any(|o| {
        OS_TYPES.contains(&o.type_uuid.as_str())
            || (refusal(o, None).is_none() && probe(o).is_ok_and(|pr| pr.os))
    });
    if os {
        return Err(End::failed(CODE_OS, "preparing", OS_DISK));
    }
    let res = create_on(&p, cancel, say);
    if res.is_err() {
        if let Err(e) = discard_unfinished() {
            eprintln!(
                "{} archive.create: the unfinished files were not removed: {e}",
                crate::job::LOG_PREFIX
            );
        }
        // Undo what can be undone (review S2): close, read-only again.
        let _ = close();
    }
    res
}

/// Exactly one partition with this PARTUUID (a cloned disk has two).
fn only_one(parts: &[Part], partuuid: &str) -> Result<Part, String> {
    let mut it = parts.iter().filter(|p| p.partuuid == partuuid);
    match (it.next(), it.next()) {
        (Some(p), None) => Ok(p.clone()),
        (None, _) => Err("the disk with the archive was not found".into()),
        (Some(_), Some(_)) => {
            Err("two disks look the same (a copied disk?); remove one of them".into())
        }
    }
}

/// Unmount the backup disk and mount it again (rw), so what is read next
/// comes from the disk and not from the page cache.
fn remount(p: &Part) -> Result<(), String> {
    run("umount", &[HOST_MNT])?;
    if is_mounted(HOST_MNT) {
        return Err("the disk is still busy".into());
    }
    mount(p, HOST_MNT, true)
}

/// The mount at `at` is writable.
fn mounted_rw(at: &str) -> bool {
    mount_opts(at).is_some_and(|o| o.split(',').any(|x| x == "rw"))
}

fn create_on(p: &Part, cancel: &AtomicBool, say: Progress) -> Result<(), End> {
    let prep = |e: String| End::failed(CODE_PREPARE, "preparing", e);
    let enc = |e: String| End::failed(CODE_ENCRYPT, "encrypting", e);
    let fmt = |e: String| End::failed(CODE_FORMAT_ARCHIVE, "formatting", e);
    tell(say, "preparing", "Preparing the disk…", 5);
    let pr = ensure_clean(p, &mut |m| tell(say, "preparing", m, 5))
        .map_err(|r| End::failed(r.code, "preparing", r.reason))?;
    if let Some(why) = refusal(p, Some(&pr)) {
        return Err(End::failed(why.code, "preparing", why.reason));
    }
    if pr.has_archive {
        return Err(prep(
            "an archive is already on this partition; Live opens it instead".into(),
        ));
    }
    check(cancel)?;
    allow_write(p).map_err(prep)?;
    mount(p, HOST_MNT, true).map_err(prep)?;
    if !mounted_rw(HOST_MNT) {
        return Err(prep("the disk could not be opened for writing".into()));
    }
    let dir = Path::new(HOST_MNT).join(DIR);
    real_dir(&dir).map_err(prep)?;
    fs::create_dir_all(&dir).map_err(|e| prep(e.to_string()))?;
    let readme: &[u8] = if crate::index::plain() {
        b"This folder holds an ALPArchive that is NOT encrypted.\r\nThe folder files holds your photos as normal files: anyone with this disk can open them.\r\n"
    } else {
        b"This folder holds an encrypted ALPArchive.\r\nThe folder data holds your photos, encrypted. Deleting it deletes the archive.\r\n"
    };
    write_new(&dir.join("README.txt"), readme).map_err(|e| prep(e.to_string()))?;
    check(cancel)?;
    tell(say, "checking", "Checking the disk's health…", 8);
    smart_check(&p.disk, cancel)?;
    let started = Instant::now();
    let cell = std::cell::RefCell::new(&mut *say);
    disk_test(
        &dir,
        TEST_BYTES,
        TEST_BLOCK,
        TEST_SYNC,
        test_seed(),
        cancel,
        &mut || {
            (*cell.borrow_mut())(&Update::step(
                "checking",
                "Checking the disk: reading it back…",
                35,
            ));
            remount(p)
        },
        &mut |done, speed| (*cell.borrow_mut())(&checking_update(done, TEST_BYTES, speed)),
    )?;
    eprintln!(
        "{} archive.create: disk test passed in {} s",
        crate::job::LOG_PREFIX,
        started.elapsed().as_secs()
    );
    check(cancel)?;
    if crate::index::plain() {
        return create_plain(p, &dir, say);
    }
    tell(say, "encrypting", "Encrypting…", 60);
    let partial = dir.join(DATA_PARTIAL);
    if fs::symlink_metadata(&partial).is_ok_and(|m| m.is_dir()) {
        // Ours, from an unfinished earlier try.
        fs::remove_dir_all(&partial).map_err(|e| enc(e.to_string()))?;
    }
    fs::create_dir(&partial).map_err(|e| enc(e.to_string()))?;
    let pw = new_password().map_err(enc)?;
    write_secret(&pass_path(), pw.as_bytes()).map_err(enc)?;
    crate::vault::init(&partial, &pw).map_err(|e| match e {
        crate::vault::Error::Timeout(m) => End::failed(CODE_STALLED, "encrypting", m),
        e => enc(e.into()),
    })?;
    check(cancel)?;
    tell(say, "formatting", "Creating the archive…", 80);
    let id = crate::vault::archive_id(&partial).map_err(fmt)?;
    let data = dir.join(DATA);
    // From here on the archive exists: no more cancelling.
    fs::rename(&partial, &data).map_err(|e| fmt(e.to_string()))?;
    if crate::index::serious() {
        let mut f = write_new(&dir.join(STRONG_MARK), b"strong protection\r\n")
            .map_err(|e| fmt(e.to_string()))?;
        std::io::Write::flush(&mut f).map_err(|e| fmt(e.to_string()))?;
    }
    sync_dir(&dir).map_err(fmt)?;
    backup_conf(&data).map_err(fmt)?;
    let rec = Record {
        partuuid: p.partuuid.clone(),
        fs_uuid: p.fs_uuid.clone(),
        format: Format::Gocryptfs,
        id,
    };
    write_atomic(RECORD, &rec.to_toml()).map_err(fmt)?;
    // The new archive is on the disk before "ready" (journal committed).
    run("sync", &["-f", HOST_MNT]).map_err(fmt)?;
    mount_files(&data).map_err(fmt)?;
    tell(say, "done", "The archive is ready.", 100);
    Ok(())
}

/// The rest of [`create_on`] for an unprotected archive (ADR 0016): the
/// plain folder, a random id next to it, the record, the bind mount.
fn create_plain(p: &Part, dir: &Path, say: Progress) -> Result<(), End> {
    let fmt = |e: String| End::failed(CODE_FORMAT_ARCHIVE, "formatting", e);
    tell(say, "formatting", "Creating the archive…", 80);
    let files = dir.join(PLAIN);
    real_dir(&files).map_err(fmt)?;
    fs::create_dir_all(&files).map_err(|e| fmt(e.to_string()))?;
    let id = new_password().map_err(fmt)?[..32].to_string();
    let f = write_new(&dir.join(PLAIN_ID), id.as_bytes()).map_err(|e| fmt(e.to_string()))?;
    f.sync_all().map_err(|e| fmt(e.to_string()))?;
    sync_dir(dir).map_err(fmt)?;
    let rec = Record {
        partuuid: p.partuuid.clone(),
        fs_uuid: p.fs_uuid.clone(),
        format: Format::Plain,
        id,
    };
    write_atomic(RECORD, &rec.to_toml()).map_err(fmt)?;
    run("sync", &["-f", HOST_MNT]).map_err(fmt)?;
    bind_plain(&files).map_err(fmt)?;
    tell(say, "done", "The archive is ready.", 100);
    Ok(())
}

/// Bind-mount an unprotected archive's folder at [`FILES_MNT`] (the same
/// place as the decrypted view of an encrypted one, so the rest is alike).
fn bind_plain(files: &Path) -> Result<(), String> {
    pool_dir()?;
    if is_mounted(FILES_MNT) {
        return Ok(());
    }
    fs::create_dir_all(FILES_MNT).map_err(|e| e.to_string())?;
    run("mount", &["--bind", &files.to_string_lossy(), FILES_MNT]).map(drop)
}

/// `ALPArchive` on the backup disk is a real folder or not there yet, never a
/// symbolic link (root would follow it off the disk, ext4 and NTFS have
/// them).
fn real_dir(dir: &Path) -> Result<(), String> {
    match fs::symlink_metadata(dir) {
        Ok(m) if !m.file_type().is_dir() => Err(format!(
            "{} on this disk is not a folder; rename or remove it",
            dir.file_name().unwrap_or_default().to_string_lossy()
        )),
        _ => Ok(()),
    }
}

/// Write a new file at `path` in a folder on the backup disk: an earlier
/// file (or a symbolic link) of that name is removed first, and the new
/// one is created exclusively, so a link is never followed.
fn write_new(path: &Path, body: &[u8]) -> std::io::Result<fs::File> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    f.write_all(body)?;
    Ok(f)
}

/// After a cancelled or failed creation: let go of a half-mounted archive
/// and delete only this job's own files on the mounted disk (the test file
/// and the unfinished `ALPArchive/data.partial`); the folder and its
/// README stay.
fn discard_unfinished() -> Result<(), String> {
    release()?;
    let dir = Path::new(HOST_MNT).join(DIR);
    if !is_mounted(HOST_MNT) {
        return Ok(());
    }
    let test = dir.join(TEST_FILE);
    if fs::symlink_metadata(&test).is_ok_and(|m| m.is_file()) {
        fs::remove_file(&test).map_err(|e| e.to_string())?;
    }
    let partial = dir.join(DATA_PARTIAL);
    if fs::symlink_metadata(&partial).is_ok_and(|m| m.is_dir()) {
        fs::remove_dir_all(&partial).map_err(|e| e.to_string())?;
    }
    if dir.is_dir() {
        sync_dir(&dir)?;
    }
    Ok(())
}

/// Copy of the archive's LUKS header inside the index (review S9): a
/// damaged header on the external disk loses everything otherwise. It holds
/// the key slots wrapped by keys that are not in it (key file, recovery
/// key), so it adds no exposure next to the key file.
fn header_backup(loopdev: &str) -> Result<(), String> {
    let path = crate::index::path("keys/archive.luks-header");
    let tmp = path.with_extension("tmp");
    let _ = fs::remove_file(&tmp);
    run(
        "cryptsetup",
        &[
            "luksHeaderBackup",
            loopdev,
            "--header-backup-file",
            &tmp.to_string_lossy(),
        ],
    )?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    fs::File::open(&tmp)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    sync_dir(path.parent().unwrap_or(Path::new("/")))
}

/// The loop device of the open archive file.
fn open_loop() -> Result<String, String> {
    let file = Path::new(HOST_MNT).join(DIR).join(FILE);
    run(
        "losetup",
        &["-n", "-O", "NAME", "-j", &file.to_string_lossy()],
    )?
    .split_whitespace()
    .next()
    .map(str::to_string)
    .ok_or_else(|| "the archive is not open".into())
}

/// systemd's recovery key alphabet ("modhex"): 64 characters.
const MODHEX: &str = "cbdefghijklnrtuv";

/// A typed recovery key in systemd's form (8 groups of 8, `-` between),
/// whatever spaces, dashes or case were used; `None` if it cannot be one.
pub fn normalize_recovery_key(typed: &str) -> Option<String> {
    let chars: Vec<char> = typed
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| c.to_ascii_lowercase())
        .collect();
    if chars.len() != 64 || !chars.iter().all(|c| MODHEX.contains(*c)) {
        return None;
    }
    Some(
        chars
            .chunks(8)
            .map(|g| g.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join("-"),
    )
}

/// The recovery key in systemd-cryptenroll's output, if any.
fn find_recovery_key(out: &str) -> Option<String> {
    out.split_whitespace()
        .find(|w| w.len() == 71 && w.matches('-').count() == 7)
        .and_then(normalize_recovery_key)
}

/// Has the open archive a recovery code? New archives: `recovery.conf` is
/// there; old ones: the LUKS2 token `systemd-recovery`.
pub fn has_recovery() -> bool {
    match record().map(|r| r.format) {
        Some(Format::Gocryptfs) => Path::new(HOST_MNT).join(DIR).join(RECOVERY_CONF).is_file(),
        Some(Format::Plain) => false,
        Some(Format::Luks) => open_loop()
            .and_then(|d| run("cryptsetup", &["luksDump", &d]))
            .is_ok_and(|d| d.contains("systemd-recovery")),
        None => false,
    }
}

/// Make a new recovery key for an open old (ALPHA 0.1) archive: an extra
/// key slot made by `systemd-cryptenroll --recovery-key` (it generates the
/// key; no own cryptography), unlocked with the key file. The old recovery
/// slot is wiped after the new one is made. The key is returned once and
/// never written anywhere. New archives use 15 words ([`bind_recovery`]).
pub fn recovery_create() -> Result<zeroize::Zeroizing<String>, String> {
    if !is_open() || !crate::index::is_open() {
        return Err("the archive is not open".into());
    }
    let dev = open_loop()?;
    let o = Command::new("systemd-cryptenroll")
        .args([
            "--wipe-slot=recovery",
            "--recovery-key",
            &format!("--unlock-key-file={}", key_path()),
            &dev,
        ])
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("systemd-cryptenroll: {e}"))?;
    let out = zeroize::Zeroizing::new(format!(
        "{}\n{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    ));
    if !o.status.success() {
        // Its error text never contains the key (it failed before one was
        // shown); report only the exit status to be safe.
        return Err(format!("systemd-cryptenroll failed ({})", o.status));
    }
    let key = find_recovery_key(&out)
        .map(zeroize::Zeroizing::new)
        .ok_or("systemd-cryptenroll showed no recovery key")?;
    header_backup(&dev)?;
    Ok(key)
}

/// Storage v2's recovery seam (ADR 0013/0014, `recovery::bind`), first
/// step (T42): make `secret` open the open archive through a pending
/// config, `ALPArchive/recovery.conf.pending` (a copy of the folder's
/// config re-wrapped by gocryptfs with `secret`, written atomically). The
/// current `recovery.conf` is not touched: [`commit_recovery`] puts the new
/// one in place after the words were checked. Old archives cannot hold
/// words ([`crate::recovery::OLD_ARCHIVE`]); unencrypted ones have nothing
/// to reopen ([`crate::recovery::NOT_ENCRYPTED`]).
pub fn bind_recovery(secret: &str) -> Result<(), String> {
    let rec = record().ok_or(crate::recovery::NOT_OPEN)?;
    if !is_open() {
        return Err(crate::recovery::NOT_OPEN.into());
    }
    match rec.format {
        Format::Luks => return Err(crate::recovery::OLD_ARCHIVE.into()),
        Format::Plain => return Err(crate::recovery::NOT_ENCRYPTED.into()),
        Format::Gocryptfs => {}
    }
    let dir = Path::new(HOST_MNT).join(DIR);
    let data = dir.join(DATA);
    let pw = read_password()?;
    let _one = CONF_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    crate::vault::copy_rewrapped(
        &data.join("gocryptfs.conf"),
        &dir.join(RECOVERY_PENDING),
        &data,
        &pw,
        secret,
    )
    .map_err(String::from)
}

/// Second step (T42): the pending recovery config replaces `recovery.conf`
/// (one rename, then fsync of the folder), so the old code stops working
/// only now.
pub fn commit_recovery() -> Result<(), String> {
    if !is_open() {
        return Err(crate::recovery::NOT_OPEN.into());
    }
    let dir = Path::new(HOST_MNT).join(DIR);
    let pending = dir.join(RECOVERY_PENDING);
    let _one = CONF_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !pending.is_file() {
        return Err(crate::recovery::NO_PENDING.into());
    }
    fs::rename(&pending, dir.join(RECOVERY_CONF))
        .map_err(|e| format!("{}: {e}", pending.display()))?;
    sync_dir(&dir)
}

/// Forget a pending recovery config (cancel, timeout, or a stale one after
/// a restart). The current `recovery.conf` stays valid.
pub fn drop_pending_recovery() {
    let dir = Path::new(HOST_MNT).join(DIR);
    let _one = CONF_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let p = dir.join(RECOVERY_PENDING);
    let mut gone = false;
    for f in [
        p.clone(),
        dir.join(format!("{RECOVERY_PENDING}.new")),
        dir.join(format!("{RECOVERY_PENDING}.new.tmp")),
    ] {
        gone |= fs::remove_file(&f).is_ok();
    }
    if gone {
        let _ = sync_dir(&dir);
    }
}

/// Level 3 (ADR 0016): make `password.conf` open the archive with the ALP
/// password `pw` (already checked), unless it does already. Skipped for
/// strong protection and unprotected archives. Called after the password
/// was typed (sign-in, unlock), so it follows a new password too.
pub fn bind_password(pw: &str) -> Result<(), String> {
    let _one = CONF_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if PASSWORD_BOUND.load(std::sync::atomic::Ordering::SeqCst) {
        return Ok(());
    }
    let Some(rec) = record() else { return Ok(()) };
    if rec.format != Format::Gocryptfs
        || !is_open()
        || crate::index::serious()
        || crate::index::plain()
    {
        return Ok(());
    }
    let dir = Path::new(HOST_MNT).join(DIR);
    if dir.join(STRONG_MARK).exists() {
        // A strong archive reopened on another stick stays strong.
        return Ok(());
    }
    let data = dir.join(DATA);
    let conf = dir.join(PASSWORD_CONF);
    if !(conf.is_file() && opens(&conf, &data, pw)?) {
        let archive_pw = read_password()?;
        crate::vault::copy_rewrapped_n(
            &data.join("gocryptfs.conf"),
            &conf,
            &data,
            &archive_pw,
            pw,
            Some(PASSWORD_SCRYPTN),
        )
        .map_err(String::from)?;
    }
    PASSWORD_BOUND.store(true, std::sync::atomic::Ordering::SeqCst);
    Ok(())
}

/// SHA-256 of a file, read in 1 MiB blocks (hex).
fn file_sha256(path: &Path) -> std::io::Result<String> {
    use bitcoin_hashes::{Hash, HashEngine, sha256};
    let mut f = fs::File::open(path)?;
    let mut e = sha256::HashEngine::default();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        e.input(&buf[..n]);
    }
    Ok(sha256::Hash::from_engine(e).to_string())
}

/// Every regular file under `root` (paths relative to it), folders first
/// seen; symbolic links and other special files are listed apart.
fn walk(
    root: &Path,
    rel: &Path,
    files: &mut Vec<PathBuf>,
    dirs: &mut Vec<PathBuf>,
    other: &mut Vec<PathBuf>,
) -> std::io::Result<()> {
    for e in fs::read_dir(root.join(rel))? {
        let e = e?;
        let r = rel.join(e.file_name());
        let t = e.file_type()?;
        if t.is_dir() {
            dirs.push(r.clone());
            walk(root, &r, files, dirs, other)?;
        } else if t.is_file() {
            files.push(r);
        } else {
            other.push(r);
        }
    }
    Ok(())
}

/// The list of files already copied and verified (ADR 0018), in the index.
fn manifest_path() -> PathBuf {
    crate::index::path("alp/encrypt-manifest.txt")
}

/// ADR 0018 step 6: copy the unprotected archive (`ALPArchive/files`) into a
/// new encrypted one next to it, verify every file through the decrypted
/// view, then switch the record to it. The plain folder stays until
/// [`encrypt_cleanup`]. Resumable: verified files are listed in the index.
/// The caller stopped Immich; the index is encrypted already.
pub fn encrypt_archive(alp_password: &str, say: Progress) -> Result<(), String> {
    use std::io::Write;
    let rec = record().ok_or("no archive set up yet")?;
    let dir = Path::new(HOST_MNT).join(DIR);
    let plain = dir.join(PLAIN);
    let partial = dir.join(DATA_PARTIAL);
    let data = dir.join(DATA);
    if rec.format == Format::Gocryptfs {
        // A resume after the switch: open it and make sure the password
        // reopens it (storage review M2).
        open()?;
        return bind_and_check(alp_password);
    }
    if rec.format != Format::Plain || !plain.is_dir() {
        return Err("the unencrypted archive was not found".into());
    }
    if !is_mounted(HOST_MNT) {
        open()?;
    }
    if data.join("gocryptfs.conf").is_file() && !partial.exists() {
        // Cut between the rename and the record (M3): `data` is the new
        // archive and the key in the index is its key; only the record is
        // missing. Check it opens, then write the record.
        crate::vault::unmount(Path::new(FILES_MNT))?;
        mount_files(&data)?;
        switch_record(&rec, &data)?;
        return bind_and_check(alp_password);
    }
    let (mut files, mut dirs, mut other) = (Vec::new(), Vec::new(), Vec::new());
    walk(&plain, Path::new(""), &mut files, &mut dirs, &mut other).map_err(|e| e.to_string())?;
    if !other.is_empty() {
        // Links or special files would be lost with the open copy: stop.
        return Err(format!(
            "{} items in ALPArchive/files are not normal files or folders (for example {}); nothing was changed",
            other.len(),
            other[0].display()
        ));
    }
    if !partial.exists() {
        // Room for a second copy plus 1 GiB (storage review M5).
        let need: u64 = files
            .iter()
            .filter_map(|f| fs::metadata(plain.join(f)).ok())
            .map(|m| m.len() + m.len() / 32 + 4096)
            .sum::<u64>()
            + (1 << 30);
        let free = free_bytes(HOST_MNT);
        if free < need {
            return Err(format!(
                "the backup disk needs {} GB more free room for the encrypted copy; nothing was changed",
                (need - free).div_ceil(1 << 30)
            ));
        }
    }
    // The plain bind goes; the new encrypted folder takes its place.
    crate::vault::unmount(Path::new(FILES_MNT))?;
    if !partial.join("gocryptfs.conf").is_file() {
        tell(say, "encrypting", "Making the encrypted archive…", 30);
        if fs::symlink_metadata(&partial).is_ok_and(|m| m.is_dir()) {
            fs::remove_dir_all(&partial).map_err(|e| e.to_string())?;
        }
        fs::create_dir(&partial).map_err(|e| e.to_string())?;
        let pw = new_password()?;
        write_secret(&pass_path(), pw.as_bytes())?;
        crate::vault::init(&partial, &pw).map_err(String::from)?;
        let _ = fs::remove_file(manifest_path());
    }
    mount_files(&partial)?;
    let dst = Path::new(FILES_MNT);
    for d in &dirs {
        fs::create_dir_all(dst.join(d)).map_err(|e| e.to_string())?;
        let _ = std::os::unix::fs::chown(dst.join(d), Some(ARCHIVE_UID), Some(ARCHIVE_UID));
    }
    let done: std::collections::HashSet<String> = fs::read_to_string(manifest_path())
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    let mut log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(manifest_path())
        .map_err(|e| e.to_string())?;
    let total = files.len().max(1);
    for (i, f) in files.iter().enumerate() {
        let key = f.to_string_lossy().into_owned();
        if done.contains(&key) {
            continue;
        }
        if i % 20 == 0 {
            let pct = 35 + (55 * i / total) as u8;
            tell(
                say,
                "copying",
                &format!(
                    "Copying your photos into the encrypted archive ({i} of {})…",
                    files.len()
                ),
                pct,
            );
        }
        copy_verified(&plain, dst, f, i)?;
        writeln!(log, "{key}")
            .and_then(|()| log.sync_all())
            .map_err(|e| e.to_string())?;
    }
    // Before the switch: every file is there, with the same size; one that
    // a power cut lost after it was listed is copied again (M8).
    tell(say, "checking", "Checking the copy…", 92);
    for (i, f) in files.iter().enumerate() {
        let a = fs::metadata(plain.join(f))
            .map_err(|e| e.to_string())?
            .len();
        let same = fs::metadata(dst.join(f)).is_ok_and(|m| m.len() == a);
        if !same {
            copy_verified(&plain, dst, f, i)?;
        }
    }
    tell(say, "formatting", "Switching to the encrypted archive…", 95);
    crate::vault::unmount(Path::new(FILES_MNT))?;
    fs::rename(&partial, &data).map_err(|e| e.to_string())?;
    sync_dir(&dir)?;
    backup_conf(&data)?;
    mount_files(&data)?;
    switch_record(&rec, &data)?;
    bind_and_check(alp_password)?;
    let _ = fs::remove_file(manifest_path());
    Ok(())
}

/// Copy one file into the decrypted view: a short unique temporary name,
/// fsync, rename, fsync of the folder, then compare SHA-256 (M8).
fn copy_verified(plain: &Path, dst: &Path, f: &Path, i: usize) -> Result<(), String> {
    let key = f.to_string_lossy().into_owned();
    let from = plain.join(f);
    let to = dst.join(f);
    let parent = to.parent().unwrap_or(dst).to_path_buf();
    let tmp = parent.join(format!(".alp-copy-{i}"));
    {
        let mut src = fs::File::open(&from).map_err(|e| format!("{key}: {e}"))?;
        let mut out = fs::File::create(&tmp).map_err(|e| format!("{key}: {e}"))?;
        std::io::copy(&mut src, &mut out).map_err(|e| format!("{key}: {e}"))?;
        out.sync_all().map_err(|e| format!("{key}: {e}"))?;
    }
    fs::rename(&tmp, &to).map_err(|e| format!("{key}: {e}"))?;
    sync_dir(&parent)?;
    std::os::unix::fs::chown(&to, Some(ARCHIVE_UID), Some(ARCHIVE_UID))
        .map_err(|e| format!("{key}: {e}"))?;
    let a = file_sha256(&from).map_err(|e| format!("{key}: {e}"))?;
    let b = file_sha256(&to).map_err(|e| format!("{key}: {e}"))?;
    if a != b {
        return Err(format!("{key} differs after copying; nothing was deleted"));
    }
    Ok(())
}

/// The record names the encrypted archive `data` from now on.
fn switch_record(old: &Record, data: &Path) -> Result<(), String> {
    let id = crate::vault::archive_id(data)?;
    let new = Record {
        partuuid: old.partuuid.clone(),
        fs_uuid: old.fs_uuid.clone(),
        format: Format::Gocryptfs,
        id,
    };
    write_atomic(RECORD, &new.to_toml())
}

/// Level 3 after encrypting: the password reopens the disk, checked.
fn bind_and_check(alp_password: &str) -> Result<(), String> {
    bind_password(alp_password)?;
    let conf = Path::new(HOST_MNT).join(DIR).join(PASSWORD_CONF);
    let data = Path::new(HOST_MNT).join(DIR).join(DATA);
    let _one = CONF_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !(conf.is_file() && opens(&conf, &data, alp_password)?) {
        return Err("the password could not be tied to the encrypted archive".into());
    }
    let _ = fs::remove_file(manifest_path());
    Ok(())
}

/// After a failed encrypt step (M4): put the unprotected archive back in
/// place so Immich can run on it again; nothing is deleted.
pub fn encrypt_rollback() -> Result<(), String> {
    let Some(rec) = record() else { return Ok(()) };
    if rec.format != Format::Plain {
        return Ok(()); // switched: the encrypted archive is the archive
    }
    crate::vault::unmount(Path::new(FILES_MNT))?;
    open()
}

/// ADR 0018 step 7: after Immich answered on the encrypted archive, the
/// unencrypted copy and its id go (only our own folder and file).
pub fn encrypt_cleanup() -> Result<(), String> {
    let rec = record().ok_or("no archive set up yet")?;
    if rec.format != Format::Gocryptfs {
        return Err("the archive is not encrypted yet; nothing was deleted".into());
    }
    let dir = Path::new(HOST_MNT).join(DIR);
    let plain = dir.join(PLAIN);
    if fs::symlink_metadata(&plain).is_ok_and(|m| m.is_dir()) {
        // Once more, now through the remounted encrypted archive (not the
        // page cache of the copy): every file the same, then delete.
        let (mut files, mut dirs, mut other) = (Vec::new(), Vec::new(), Vec::new());
        walk(&plain, Path::new(""), &mut files, &mut dirs, &mut other)
            .map_err(|e| e.to_string())?;
        for f in &files {
            let a = file_sha256(&plain.join(f)).map_err(|e| e.to_string())?;
            let b = file_sha256(&Path::new(FILES_MNT).join(f))
                .map_err(|e| format!("{}: {e}; the open copy was kept", f.display()))?;
            if a != b {
                return Err(format!(
                    "{} differs in the encrypted archive; the open copy was kept",
                    f.display()
                ));
            }
        }
        fs::remove_dir_all(&plain).map_err(|e| e.to_string())?;
    }
    let _ = fs::remove_file(dir.join(PLAIN_ID));
    sync_dir(&dir)
}

/// Can the ALP password reopen this archive from another stick (level 3)?
pub fn password_reopen() -> bool {
    Path::new(HOST_MNT).join(DIR).join(PASSWORD_CONF).is_file()
}

/// Does `secret` unlock the gocryptfs config `conf`? Tried on a throwaway
/// copy in /run; nothing on the disk changes.
fn opens(conf: &Path, cipherdir: &Path, secret: &str) -> Result<bool, String> {
    let tmpdir = Path::new("/run/alp/conf-check");
    let _ = fs::remove_dir_all(tmpdir);
    fs::create_dir_all(tmpdir).map_err(|e| e.to_string())?;
    fs::set_permissions(tmpdir, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    let throwaway = new_password()?;
    let res = crate::vault::copy_rewrapped(
        conf,
        &tmpdir.join("check.conf"),
        cipherdir,
        secret,
        &throwaway,
    );
    let _ = fs::remove_dir_all(tmpdir);
    match res {
        Ok(()) => Ok(true),
        Err(crate::vault::Error::WrongPassword) => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}

/// T40: do these recovery words (their secret) open this archive? A copy
/// of `recovery.conf` in /run is re-wrapped with them and thrown away;
/// nothing on the disk changes. `Ok(false)`: wrong words.
pub fn check_recovery(secret: &str) -> Result<bool, String> {
    let rec = record().ok_or(crate::recovery::NOT_OPEN)?;
    if !is_open() {
        return Err(crate::recovery::NOT_OPEN.into());
    }
    match rec.format {
        Format::Luks => return Err(crate::recovery::OLD_ARCHIVE.into()),
        Format::Plain => return Err(crate::recovery::NOT_ENCRYPTED.into()),
        Format::Gocryptfs => {}
    }
    let dir = Path::new(HOST_MNT).join(DIR);
    let rconf = dir.join(RECOVERY_CONF);
    if !rconf.is_file() {
        return Err(RECOVERY_NONE.into());
    }
    let _one = CONF_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    opens(&rconf, &dir.join(DATA), secret)
}

/// Open an old archive made with another stick, with its 64-letter
/// recovery key (spec block 5): give it a new key file in this stick's
/// index, then remember it like a new archive. Never formats or deletes
/// anything.
pub fn recover(partuuid: &str, recovery_key: &str, say: Progress) -> Result<(), String> {
    let key = normalize_recovery_key(recovery_key).ok_or("this is not a recovery key")?;
    let key = zeroize::Zeroizing::new(key);
    recover_any(partuuid, say, &mut |p, kind, say| match kind {
        Format::Luks => recover_luks(p, &key, say),
        Format::Gocryptfs => Err(RECOVERY_NEEDS_WORDS.into()),
        Format::Plain => adopt_plain(p, say),
    })
}

/// Level 3 (ADR 0016): reopen an encrypted archive made with another stick
/// with the ALP password it was used with (`password.conf`).
pub fn recover_with_password(partuuid: &str, pw: &str, say: Progress) -> Result<(), String> {
    recover_any(partuuid, say, &mut |p, kind, say| match kind {
        Format::Gocryptfs => recover_vault_with(p, PASSWORD_CONF, pw, say),
        Format::Luks => Err(RECOVERY_NEEDS_KEY.into()),
        Format::Plain => adopt_plain(p, say),
    })
}

/// Open a new archive made with another stick with the secret of its 15
/// recovery words (`recovery::open`, ADR 0014): `recovery.conf` re-wrapped
/// with a new archive password becomes the folder's config (the old one is
/// kept as `ALPArchive/gocryptfs.conf.replaced-<time>`); the recovery code
/// keeps working.
pub fn recover_with_secret(partuuid: &str, secret: &str, say: Progress) -> Result<(), String> {
    recover_any(partuuid, say, &mut |p, kind, say| match kind {
        Format::Gocryptfs => recover_vault(p, secret, say),
        Format::Luks => Err(RECOVERY_NEEDS_KEY.into()),
        Format::Plain => adopt_plain(p, say),
    })
}

/// Reopen an unprotected archive made with another stick: nothing to
/// unlock, only remember it (ADR 0016). Its folder is never changed.
fn adopt_plain(p: &Part, say: Progress) -> Result<(), String> {
    let dir = Path::new(HOST_MNT).join(DIR);
    let id = fs::read_to_string(dir.join(PLAIN_ID)).map_err(|e| e.to_string())?;
    let rec = Record {
        partuuid: p.partuuid.clone(),
        fs_uuid: p.fs_uuid.clone(),
        format: Format::Plain,
        id: id.trim().to_string(),
    };
    write_atomic(RECORD, &rec.to_toml())?;
    bind_plain(&dir.join(PLAIN))?;
    tell(say, "done", "The archive is open again.", 100);
    Ok(())
}

type RecoverWith<'a> = dyn FnMut(&Part, Format, Progress) -> Result<(), String> + 'a;

fn recover_any(partuuid: &str, say: Progress, with: &mut RecoverWith) -> Result<(), String> {
    state_ready()?;
    if !crate::index::is_open() {
        return Err("the archive is locked; type your password first".into());
    }
    if record().is_some() {
        return Err("this stick already has an archive".into());
    }
    let parts = partitions();
    let p = only_one(&parts, partuuid)?;
    if !p.external {
        return Err("only an external disk or USB stick can hold the archive".into());
    }
    tell(say, "checking", "Checking the recovery key…", 10);
    let clean = ensure_clean(&p, &mut |m| tell(say, "checking", m, 10)).map_err(|r| r.reason);
    let res = clean.and_then(|_| {
        allow_write(&p)?;
        mount(&p, HOST_MNT, true)?;
        if !mounted_rw(HOST_MNT) {
            return Err("the disk could not be opened for writing".into());
        }
        let kind = kind_at(Path::new(HOST_MNT)).ok_or("no archive was found on this disk")?;
        // Never mix levels (review of T35): an encrypted stick must not start
        // saving into an unencrypted archive, and an unprotected stick must
        // not keep an encrypted archive's password in its plain index.
        match (crate::index::plain(), kind == Format::Plain) {
            (false, true) => return Err(PLAIN_ON_ENCRYPTED.into()),
            (true, false) => return Err(ENCRYPTED_ON_PLAIN.into()),
            _ => {}
        }
        with(&p, kind, say)
    });
    if res.is_err() {
        let _ = close();
    }
    res
}

fn recover_luks(p: &Part, recovery_key: &str, say: Progress) -> Result<(), String> {
    let loopdev = attach(&Path::new(HOST_MNT).join(DIR).join(FILE))
        .map_err(|_| "no archive was found on this disk".to_string())?;
    let test = crate::index::run_stdin_pub(
        "cryptsetup",
        &[
            "open",
            "--test-passphrase",
            crate::index::NO_TOKENS,
            "--key-file=-",
            &loopdev,
        ],
        recovery_key,
    );
    if let Err(e) = test {
        return Err(if e.contains("(exit status: 2)") {
            RECOVERY_WRONG.into()
        } else {
            e
        });
    }
    tell(say, "opening", "Opening your archive…", 50);
    let key = key_file()?;
    crate::index::run_stdin_pub(
        "cryptsetup",
        &[
            "luksAddKey",
            crate::index::NO_TOKENS,
            "--batch-mode",
            "--pbkdf",
            "pbkdf2",
            "--pbkdf-force-iterations",
            "1000",
            "--key-file=-",
            &loopdev,
            &key,
        ],
        recovery_key,
    )?;
    let luks_uuid = run("cryptsetup", &["luksUUID", &loopdev])?;
    run(
        "cryptsetup",
        &["open", "--key-file", &key, &loopdev, MAPPER],
    )?;
    header_backup(&loopdev)?;
    fs::create_dir_all(POOL_MNT).map_err(|e| e.to_string())?;
    run(
        "mount",
        &["-o", "noatime", &format!("/dev/mapper/{MAPPER}"), POOL_MNT],
    )?;
    bind_files()?;
    let rec = Record {
        partuuid: p.partuuid.clone(),
        fs_uuid: p.fs_uuid.clone(),
        format: Format::Luks,
        id: luks_uuid,
    };
    write_atomic(RECORD, &rec.to_toml())?;
    tell(say, "done", "Your archive is open.", 100);
    Ok(())
}

fn recover_vault(p: &Part, secret: &str, say: Progress) -> Result<(), String> {
    recover_vault_with(p, RECOVERY_CONF, secret, say)
}

/// Reopen with `conf` (the recovery code's or the ALP password's config).
fn recover_vault_with(
    p: &Part,
    conf_name: &str,
    secret: &str,
    say: Progress,
) -> Result<(), String> {
    let dir = Path::new(HOST_MNT).join(DIR);
    let data = dir.join(DATA);
    let rconf = dir.join(conf_name);
    if !rconf.is_file() {
        return Err(if conf_name == PASSWORD_CONF {
            PASSWORD_NONE
        } else {
            RECOVERY_NONE
        }
        .into());
    }
    let pw = new_password()?;
    let recovered = dir.join("gocryptfs.conf.recovered");
    crate::vault::copy_rewrapped(&rconf, &recovered, &data, secret, &pw).map_err(|e| match e {
        crate::vault::Error::WrongPassword => RECOVERY_WRONG.to_string(),
        e => e.to_string(),
    })?;
    tell(say, "opening", "Opening your archive…", 50);
    write_secret(&pass_path(), pw.as_bytes())?;
    let stamp: String = crate::live::now_rfc3339()
        .chars()
        .filter(char::is_ascii_digit)
        .collect();
    let conf = data.join("gocryptfs.conf");
    let kept = dir.join(format!("gocryptfs.conf.replaced-{stamp}"));
    fs::copy(&conf, &kept)
        .and_then(|_| fs::File::open(&kept))
        .and_then(|f| f.sync_all())
        .map_err(|e| format!("the old config could not be kept: {e}"))?;
    fs::rename(&recovered, &conf).map_err(|e| e.to_string())?;
    sync_dir(&data)?;
    sync_dir(&dir)?;
    backup_conf(&data)?;
    let id = crate::vault::archive_id(&data)?;
    mount_files(&data)?;
    let rec = Record {
        partuuid: p.partuuid.clone(),
        fs_uuid: p.fs_uuid.clone(),
        format: Format::Gocryptfs,
        id,
    };
    write_atomic(RECORD, &rec.to_toml())?;
    tell(say, "done", "Your archive is open.", 100);
    Ok(())
}

/// Message of a recovery key that does not open the archive (the API maps
/// it to `recovery.wrong`).
pub const PLAIN_ON_ENCRYPTED: &str = "This backup disk holds an unencrypted archive, and this stick encrypts. Start over without encryption to use it.";
pub const ENCRYPTED_ON_PLAIN: &str = "This backup disk holds an encrypted archive, and this stick was set up without encryption. Start over with encryption to reopen it.";
pub const PASSWORD_NONE: &str = "This archive cannot be opened with a password (it was made with strong protection or before this version). Use its recovery code.";
pub const RECOVERY_WRONG: &str = "This recovery key does not open this archive.";
/// The 64-letter key typed for a new archive, words for an old one, or a
/// new archive without a recovery code (also `recovery.wrong`).
pub const RECOVERY_NEEDS_WORDS: &str =
    "This archive opens with its 15 recovery words, not with a 64-letter key.";
pub const RECOVERY_NEEDS_KEY: &str = "This archive was made with an earlier test version: it opens with its 64-letter recovery key, not with words.";
pub const RECOVERY_NONE: &str =
    "This archive has no recovery code yet, so it opens only with the stick it was made with.";

/// Open the archive recorded on the stick (at start-up). Only after the
/// index is open: the archive's key is inside it (ADR 0011). A disk that
/// was not removed safely is repaired first (ADR 0014).
pub fn open() -> Result<(), String> {
    let rec = record().ok_or("no archive set up yet")?;
    if is_open() {
        return Ok(());
    }
    if !crate::index::is_open() {
        return Err("the archive is locked; type your password first".into());
    }
    if rec.format == Format::Luks && Path::new(&format!("/dev/mapper/{MAPPER}")).exists() {
        // Opened before (an earlier start of this unit): only the mounts.
        if !is_mounted(POOL_MNT) {
            run(
                "mount",
                &["-o", "noatime", &format!("/dev/mapper/{MAPPER}"), POOL_MNT],
            )?;
        }
        return bind_files();
    }
    state_ready()?;
    let parts = partitions();
    let p = only_one(&parts, &rec.partuuid)?;
    if p.fs_uuid != rec.fs_uuid {
        return Err("the partition changed since the archive was made".into());
    }
    // A repair makes the disk writable (`allow_write`): close also when the
    // repair or the check fails, so it is read-only again.
    let res = volume_ok(&p)
        // Check it is our archive before anything is written (review S1).
        .and_then(|()| check_read_only(&p, &rec))
        .and_then(|()| open_on(&p, &rec));
    if res.is_err() {
        let _ = close();
    }
    res
}

/// Mount read-only, look at the archive's id (LUKS UUID or the folder's
/// id), unmount.
fn check_read_only(p: &Part, rec: &Record) -> Result<(), String> {
    let at = format!("{PROBE_MNT}/{}", p.name);
    mount(p, &at, false)?;
    let dir = Path::new(&at).join(DIR);
    let res = match rec.format {
        Format::Luks => {
            let file = dir.join(FILE);
            // cryptsetup reads the header from the file itself. No loop
            // device: `losetup -d` lets go of the file later, the umount
            // below then failed (busy), and the rw mount for the archive
            // reused this ro,noload mount, so ext4's journal was never
            // replayed after a power cut and the archive read zeros where
            // its newest blocks were (CI run 39266605834).
            if file.is_file() {
                run("cryptsetup", &["luksUUID", &file.to_string_lossy()])
            } else {
                Err("no archive file".into())
            }
        }
        Format::Gocryptfs => crate::vault::archive_id(&dir.join(DATA)),
        Format::Plain => fs::read_to_string(dir.join(PLAIN_ID))
            .map(|s| s.trim().to_string())
            .map_err(|e| e.to_string()),
    };
    unmount(&at)?;
    match res {
        Ok(u) if u == rec.id => Ok(()),
        Ok(_) => Err("this is not the archive made with this stick".into()),
        Err(_) => Err("the archive was not found or cannot be read".into()),
    }
}

/// Unmount a read-only probe mount. It must really be gone: a later rw
/// mount of the same partition would otherwise share its superblock (ro,
/// ext4 journal not replayed).
fn unmount(at: &str) -> Result<(), String> {
    let _ = run("umount", &[at]);
    if is_mounted(at) {
        return Err("the disk is still busy; unplug it, plug it in again and retry".into());
    }
    Ok(())
}

/// Options of the mount at `at` (field 6 of /proc/self/mountinfo).
fn mount_opts(at: &str) -> Option<String> {
    fs::read_to_string("/proc/self/mountinfo")
        .ok()?
        .lines()
        .find_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            (f.get(4) == Some(&at)).then(|| f.get(5).unwrap_or(&"").to_string())
        })
}

fn open_on(p: &Part, rec: &Record) -> Result<(), String> {
    allow_write(p)?;
    mount(p, HOST_MNT, true)?;
    // Never open the archive on a read-only view of its disk.
    if !mounted_rw(HOST_MNT) {
        return Err("the disk could not be opened for writing".into());
    }
    if !crate::index::is_open() {
        return Err("the archive is locked; type your password first".into());
    }
    match rec.format {
        Format::Gocryptfs => {
            // alpd just started or reopened: no words wait for this file.
            drop_pending_recovery();
            mount_files(&Path::new(HOST_MNT).join(DIR).join(DATA))
        }
        Format::Plain => bind_plain(&Path::new(HOST_MNT).join(DIR).join(PLAIN)),
        Format::Luks => {
            let loopdev = attach(&Path::new(HOST_MNT).join(DIR).join(FILE))?;
            run(
                "cryptsetup",
                &["open", "--key-file", &key_path(), &loopdev, MAPPER],
            )?;
            fs::create_dir_all(POOL_MNT).map_err(|e| e.to_string())?;
            run(
                "mount",
                &["-o", "noatime", &format!("/dev/mapper/{MAPPER}"), POOL_MNT],
            )?;
            bind_files()
        }
    }
}

/// Close in reverse order and make the disk read-only again.
/// Stops at the first step that fails and reports it (review S10): a disk
/// is never made read-only under a filesystem that is still mounted.
pub fn close() -> Result<(), String> {
    release()?;
    if is_mounted(HOST_MNT) {
        run("umount", &[HOST_MNT])?;
    }
    if fs::metadata(ALLOWED).is_ok() {
        if let Some(name) = allowed_name()
            && let Some(p) = partitions().into_iter().find(|p| p.name == name)
        {
            run("blockdev", &["--setro", &format!("/dev/{}", p.disk)])?;
        }
        let _ = fs::remove_file(ALLOWED);
    }
    Ok(())
}

/// Unmount the decrypted archive (gocryptfs, waiting until its daemon let
/// go of the disk; or an old archive's bind mount and Btrfs), close an old
/// archive and detach our loop devices; the disk stays mounted.
fn release() -> Result<(), String> {
    crate::vault::unmount(Path::new(FILES_MNT))?;
    if is_mounted(POOL_MNT) {
        run("umount", &[POOL_MNT])?;
    }
    if Path::new(&format!("/dev/mapper/{MAPPER}")).exists() {
        run("cryptsetup", &["close", MAPPER])?;
    }
    // Only our loop devices (an old archive file, or a half-made one).
    let dir = Path::new(HOST_MNT).join(DIR);
    for f in [FILE.to_string(), format!("{FILE}.partial")] {
        let file = dir.join(&f);
        if !file.exists() {
            continue;
        }
        let devs = run(
            "losetup",
            &["-n", "-O", "NAME", "-j", &file.to_string_lossy()],
        )?;
        for d in devs.split_whitespace() {
            run("losetup", &["-d", d])?;
        }
    }
    Ok(())
}

fn is_mounted(at: &str) -> bool {
    fs::read_to_string("/proc/self/mountinfo")
        .is_ok_and(|m| m.lines().any(|l| l.split_whitespace().nth(4) == Some(at)))
}

/// Mount the stick's state partition (found on the system disk only).
pub fn state_mount() -> Result<(), String> {
    let r = inspect::inspect(&inspect::Roots::system());
    let part = r
        .disks
        .iter()
        .filter(|d| d.is_system_disk)
        .flat_map(|d| &d.partitions)
        .find(|p| p.label == STATE_LABEL && !p.name.is_empty())
        .ok_or("state partition not found on the stick")?;
    fs::create_dir_all(STATE_DIR).map_err(|e| e.to_string())?;
    // By kernel name of the partition found on this stick: by-partuuid
    // could point to another ALP-OS stick (same UUIDs; review S5).
    let dev = format!("/dev/{}", part.name);
    run(
        "mount",
        &["-t", "ext4", "-o", "nosuid,nodev,noexec", &dev, STATE_DIR],
    )?;
    fs::create_dir_all(Path::new(STATE_DIR).join("live")).map_err(|e| e.to_string())?;
    // Remembered Wi-Fi networks (owner: remember the password on the stick).
    let iwd = Path::new(STATE_DIR).join("iwd");
    fs::create_dir_all(&iwd).map_err(|e| e.to_string())?;
    fs::set_permissions(&iwd, std::os::unix::fs::PermissionsExt::from_mode(0o700))
        .map_err(|e| e.to_string())?;
    fs::create_dir_all("/var/lib/iwd").map_err(|e| e.to_string())?;
    run("mount", &["--bind", &iwd.to_string_lossy(), "/var/lib/iwd"]).map(drop)
}

/// The archive as the API shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Not set up with this stick.
    None,
    /// Open; sizes of the archive's filesystem.
    Open {
        place: Option<Part>,
        size_bytes: u64,
        free_bytes: u64,
    },
    /// Set up, but not open (why, in plain words).
    Missing(String),
}

/// Is the decrypted archive mounted ([`FILES_MNT`])?
pub fn is_open() -> bool {
    is_mounted(FILES_MNT)
}

pub fn record() -> Option<Record> {
    fs::read_to_string(RECORD)
        .ok()
        .and_then(|s| Record::from_toml(&s))
}

pub fn status() -> Status {
    let Some(rec) = record() else {
        return Status::None;
    };
    if is_open() {
        // gocryptfs answers with the backing disk's numbers (ADR 0014).
        let (size, free) = run("df", &["-B1", "--output=size,avail", FILES_MNT])
            .ok()
            .and_then(|s| {
                let l = s.lines().last()?.to_string();
                let mut it = l.split_whitespace().map(|v| v.parse::<u64>().unwrap_or(0));
                Some((it.next()?, it.next()?))
            })
            .unwrap_or((0, 0));
        return Status::Open {
            place: partitions()
                .into_iter()
                .find(|p| p.partuuid == rec.partuuid),
            size_bytes: size,
            free_bytes: free,
        };
    }
    let why = read(Path::new(OPEN_ERROR));
    Status::Missing(if why.is_empty() {
        "The archive is not open yet.".into()
    } else {
        format!("The archive could not be opened: {why}.")
    })
}

/// Forget the archive this stick knows (state `missing`), so a new place
/// can be chosen. The old archive file is never touched; its record is
/// kept on the stick under another name.
pub fn forget(now: &str) -> Result<(), String> {
    if is_open() {
        return Err("the archive is open".into());
    }
    if record().is_none() {
        return Err("no archive is set up".into());
    }
    let stamp: String = now.chars().filter(char::is_ascii_digit).collect();
    fs::rename(RECORD, format!("{RECORD}.forgotten-{stamp}")).map_err(|e| e.to_string())?;
    let _ = fs::remove_file(OPEN_ERROR);
    sync_dir(Path::new(RECORD).parent().unwrap_or(Path::new("/")))
}

/// The files ALPArchive puts in its folder; nothing else is ever deleted.
const OWN_FILES: [&str; 15] = [
    FILE,
    "archive.alp.partial",
    "README.txt",
    RECOVERY_CONF,
    "recovery.conf.new",
    RECOVERY_PENDING,
    "recovery.conf.pending.new",
    "recovery.conf.pending.new.tmp",
    PASSWORD_CONF,
    "password.conf.new",
    STRONG_MARK,
    "recovery.conf.new.tmp",
    "gocryptfs.conf.recovered",
    "gocryptfs.conf.recovered.new",
    TEST_FILE,
];
/// Our own folders: the encrypted folder of a new archive and its
/// unfinished form (deleted with everything in them: all of it is ours,
/// encrypted).
const OWN_DIRS: [&str; 2] = [DATA, DATA_PARTIAL];
/// Kept configs of a recovered archive (`gocryptfs.conf.replaced-<time>`).
const REPLACED_PREFIX: &str = "gocryptfs.conf.replaced-";

/// Delete our own files in an `ALPArchive` folder (an old archive file, a
/// new archive's `data` folder, unfinished ones, the recovery config, kept
/// configs, the test file, the README), then the folder if it is empty now.
/// Anything else in it (and anything outside it) stays; symbolic links are
/// never followed. Returns whether an archive was deleted.
pub fn remove_own_files(dir: &Path) -> Result<bool, String> {
    if !fs::symlink_metadata(dir).is_ok_and(|m| m.file_type().is_dir()) {
        // Not there, or a link (never followed): nothing of ours to delete.
        return Ok(false);
    }
    let mut archive = false;
    let is_file = |f: &Path| fs::symlink_metadata(f).is_ok_and(|m| m.file_type().is_file());
    let mut names: Vec<String> = OWN_FILES.iter().map(|s| s.to_string()).collect();
    if let Ok(it) = fs::read_dir(dir) {
        names.extend(
            it.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.starts_with(REPLACED_PREFIX)),
        );
    }
    for name in &names {
        let f = dir.join(name);
        if is_file(&f) {
            fs::remove_file(&f).map_err(|e| format!("{}: {e}", f.display()))?;
            archive |= name == FILE || name == "archive.alp.partial";
        }
    }
    for name in OWN_DIRS {
        let d = dir.join(name);
        if fs::symlink_metadata(&d).is_ok_and(|m| m.file_type().is_dir()) {
            // Never follows symbolic links (std's remove_dir_all).
            fs::remove_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
            archive |= name == DATA;
        }
    }
    // Fails (and is left) when other files are in it.
    let _ = fs::remove_dir(dir);
    if let Some(parent) = dir.parent() {
        sync_dir(parent)?;
    }
    Ok(archive)
}

/// "Delete everything and start over" (owner 2026-10-24), the archive part:
/// delete the archive this stick knows from its disk (our own files in the
/// `ALPArchive` folder only, see [`remove_own_files`]) and forget it. It is
/// deleted only after its id (LUKS UUID or the folder's id) showed it is
/// the archive made with this stick. When the disk is not here, not safe to
/// write, or holds another archive, nothing is deleted on it. Returns what
/// happened, in plain words. Immich must be stopped first.
pub fn reset() -> Result<String, String> {
    state_ready()?;
    let Some(rec) = record() else {
        return Ok("no archive was set up".into());
    };
    let res = delete_archive(&rec);
    let closed = close();
    let what = res?;
    closed?;
    fs::remove_file(RECORD).map_err(|e| e.to_string())?;
    let _ = fs::remove_file(OPEN_ERROR);
    sync_dir(Path::new(RECORD).parent().unwrap_or(Path::new("/")))?;
    Ok(what)
}

fn delete_archive(rec: &Record) -> Result<String, String> {
    release()?;
    if !is_mounted(HOST_MNT) {
        let parts = partitions();
        let Ok(p) = only_one(&parts, &rec.partuuid) else {
            return Ok("the backup disk is not plugged in; its archive was left".into());
        };
        if p.fs_uuid != rec.fs_uuid {
            return Ok("the backup disk changed; nothing was deleted on it".into());
        }
        if let Err(e) = volume_ok(&p) {
            return Ok(format!(
                "the backup disk cannot be written now ({e}); its archive was left"
            ));
        }
        if check_read_only(&p, rec).is_err() {
            return Ok("the archive on the backup disk is not the one made with this stick; nothing was deleted on it".into());
        }
        allow_write(&p)?;
        mount(&p, HOST_MNT, true)?;
    }
    if rec.format == Format::Plain {
        // Unprotected: the photos are normal files of the user now; they
        // stay (ADR 0016). Only our id file goes, so it is not reopened.
        let _ = fs::remove_file(Path::new(HOST_MNT).join(DIR).join(PLAIN_ID));
        return Ok(
            "the archive was forgotten; its photos stay on the backup disk in ALPArchive/files"
                .into(),
        );
    }
    if remove_own_files(&Path::new(HOST_MNT).join(DIR))? {
        Ok("the archive was deleted".into())
    } else {
        Ok("no archive was found on the backup disk".into())
    }
}

/// Model name of a disk of this computer (for the candidate list).
pub fn disk_model(disk: &str) -> String {
    read(&Path::new("/sys/block").join(disk).join("device/model"))
}

/// Every partition of every external disk (and every external disk
/// without partitions), each with its verdict: usable (`Ok`) or why not.
pub fn candidates() -> Vec<(Part, Result<Probe, Refusal>)> {
    let parts = partitions();
    let external: Vec<&Part> = parts.iter().filter(|p| p.external).collect();
    let probes: Vec<Option<Result<Probe, String>>> = external
        .iter()
        .map(|p| refusal(p, None).is_none().then(|| probe(p)))
        .collect();
    let os_disks: Vec<&str> = external
        .iter()
        .zip(&probes)
        .filter(|(p, pr)| {
            OS_TYPES.contains(&p.type_uuid.as_str()) || matches!(pr, Some(Ok(pr)) if pr.os)
        })
        .map(|(p, _)| p.disk.as_str())
        .collect();
    external
        .iter()
        .zip(probes)
        .map(|(p, pr)| {
            let os = os_disks.contains(&p.disk.as_str());
            let v = match verdict(p, os, pr) {
                // Repaired once already this plug-in, and it did not work.
                Ok(pr) if pr.dirty && repair_marker(p).exists() => Err(Refusal::new(
                    CODE_REPAIR,
                    "This disk was not removed safely and the automatic repair did not work. Plug it into a Windows or Mac computer, let it check the disk, eject it properly, then plug it in here again, or prepare it here (this erases it).",
                )),
                v => v,
            };
            ((*p).clone(), v)
        })
        .collect()
}

/// The verdict of one candidate: `os` = its disk holds an operating
/// system, `probe` = the read-only look (only made when udev showed
/// nothing against it). Only the disk itself counts: the computer's own
/// Windows being asleep does not (owner 2026-10-20).
pub fn verdict(p: &Part, os: bool, probe: Option<Result<Probe, String>>) -> Result<Probe, Refusal> {
    if os {
        return Err(os_disk());
    }
    if let Some(why) = refusal(p, None) {
        return Err(why);
    }
    let pr = match probe {
        Some(Ok(pr)) => pr,
        Some(Err(e)) => {
            return Err(Refusal::new(
                CODE_UNREADABLE,
                format!(
                    "ALPArchive could not read this disk ({e}). Unplug it, plug it in again and retry."
                ),
            ));
        }
        None => {
            return Err(Refusal::new(
                CODE_UNREADABLE,
                "ALPArchive could not read this disk. Unplug it, plug it in again and retry.",
            ));
        }
    };
    // An archive already there is shown as such (it is opened with its
    // recovery key), even when the space left is too small for a new one.
    if pr.has_archive && volume_refusal(&pr).is_none() {
        return Ok(pr);
    }
    refusal(p, Some(&pr)).map_or(Ok(pr), Err)
}

/// Marker of a repair that did not work, per disk as plugged in this time
/// and partition: such a volume counts as unrepairable for "Prepare this
/// disk" until the disk is plugged in again.
const REPAIR_FAILED_DIR: &str = "/run/alp/repair-failed";

fn repair_marker(p: &Part) -> std::path::PathBuf {
    Path::new(REPAIR_FAILED_DIR).join(format!("{}-{}", disk_key(&p.disk), p.name))
}

/// Refusal codes of a partition that "Prepare this disk" may erase: FAT32,
/// an unknown or unusable format, an unreadable or unrepairable volume.
const PREPARABLE: [&str; 4] = [CODE_FAT32, CODE_FORMAT, CODE_UNREADABLE, CODE_REPAIR];

/// May this disk be erased and formatted as exFAT ("Prepare this disk",
/// ADR 0014)? `entries`: every candidate of one disk. Only an external disk
/// (never the stick: it is never a candidate), not write-protected, with no
/// operating system, no `ALPArchive` folder and nothing usable on it, whose
/// every partition is FAT32, unknown or unreadable.
pub fn preparable(entries: &[&(Part, Result<Probe, Refusal>)]) -> bool {
    !entries.is_empty()
        && entries.iter().all(|(p, v)| {
            p.external
                && !p.read_only
                && !OS_TYPES.contains(&p.type_uuid.as_str())
                && matches!(v, Err(r) if PREPARABLE.contains(&r.code))
        })
}

/// The disk's name in a `disk_id` ([`disk_key`]: `sdb-17`), if it is one.
pub fn disk_of_id(disk_id: &str) -> Option<&str> {
    let (name, seq) = disk_id.rsplit_once('-')?;
    let ok_name = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    let ok_seq = !seq.is_empty() && seq.chars().all(|c| c.is_ascii_digit());
    (ok_name && ok_seq).then_some(name)
}

/// Why "Prepare this disk" is not done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrepareRefusal {
    /// Unplugged, or another disk has this name now.
    NotFound,
    /// The partitions differ from the list the user saw.
    Changed,
    /// Not allowed for this disk ([`CODE_NO_PREPARE`]).
    Refused(String),
}

/// The decision of [`prepare_check`]: `now` = the disk's current id (None:
/// no such disk), `entries` = its candidates now, `shown` = the PARTUUIDs
/// the user saw (order does not matter).
pub fn prepare_decision(
    disk_id: &str,
    now: Option<&str>,
    entries: &[&(Part, Result<Probe, Refusal>)],
    shown: &[String],
) -> Result<(), PrepareRefusal> {
    if now != Some(disk_id) || entries.is_empty() {
        return Err(PrepareRefusal::NotFound);
    }
    let mut have: Vec<&str> = entries.iter().map(|(p, _)| p.partuuid.as_str()).collect();
    let mut want: Vec<&str> = shown.iter().map(String::as_str).collect();
    have.sort_unstable();
    want.sort_unstable();
    if have != want {
        return Err(PrepareRefusal::Changed);
    }
    if !preparable(entries) {
        return Err(PrepareRefusal::Refused(
            "This disk cannot be prepared here: only an external disk with nothing usable on it (no operating system, no ALPArchive folder, not write-protected) can be erased.".into(),
        ));
    }
    Ok(())
}

/// Look at the disk `disk_id` now (read-only) and decide whether it may be
/// prepared with the partitions `shown`. Returns the disk's name.
pub fn prepare_check(disk_id: &str, shown: &[String]) -> Result<String, PrepareRefusal> {
    let name = disk_of_id(disk_id).ok_or(PrepareRefusal::NotFound)?;
    let now = Path::new("/sys/block")
        .join(name)
        .is_dir()
        .then(|| disk_key(name));
    let all = candidates();
    let entries: Vec<&(Part, Result<Probe, Refusal>)> =
        all.iter().filter(|(p, _)| p.disk == name).collect();
    prepare_decision(disk_id, now.as_deref(), &entries, shown)?;
    Ok(name.to_string())
}

/// Disks above this size get a GPT (an MBR addresses at most 2 TiB with
/// 512-byte sectors).
pub const MBR_MAX: u64 = 2 << 40;

/// `sfdisk` command line and script for one exFAT partition over the whole
/// disk (sfdisk(8)): lock the device, wipe old signatures of the disk and
/// of the partitions, MBR (type 7) or GPT (Microsoft basic data).
pub fn sfdisk_plan(dev: &str, disk_bytes: u64) -> (Vec<String>, &'static str) {
    sfdisk_plan_for(dev, disk_bytes, false)
}

/// [`sfdisk_plan`] with a Linux partition type when `linux` (ext4).
pub fn sfdisk_plan_for(dev: &str, disk_bytes: u64, linux: bool) -> (Vec<String>, &'static str) {
    let gpt = disk_bytes > MBR_MAX;
    let args = [
        "--lock",
        "--wipe",
        "always",
        "--wipe-partitions",
        "always",
        "--label",
        if gpt { "gpt" } else { "dos" },
        dev,
    ]
    .map(String::from)
    .to_vec();
    let script = match (gpt, linux) {
        (true, false) => "type=EBD0A0A2-B9E5-4433-87C0-68B6B72699C7\n",
        (false, false) => "type=7\n",
        (true, true) => "type=0FC63DAF-8483-4772-8E79-3D69D8477DE4\n",
        (false, true) => "type=83\n",
    };
    (args, script)
}

/// Something of this disk is mounted (never erase it then).
pub(crate) fn disk_in_use(disk: &str) -> bool {
    let dev = format!("/dev/{disk}");
    fs::read_to_string("/proc/self/mountinfo").is_ok_and(|m| {
        m.lines().any(|l| {
            l.split(" - ")
                .nth(1)
                .and_then(|r| r.split_whitespace().nth(1))
                .is_some_and(|src| src.starts_with(&dev))
        })
    })
}

/// "Prepare this disk" (job `disk.prepare`, ADR 0014): check again, erase
/// the whole disk `disk_id` and make one exFAT partition labelled
/// `ALPArchive`. The disk is writable only while this runs
/// ([`ALLOWED`] = the disk, then the new partition) and read-only again
/// afterwards. Failures: [`CODE_PREPARE_DISK`]; refused at the check:
/// [`CODE_NO_PREPARE`].
pub fn prepare_disk(disk_id: &str, shown: &[String], say: Progress) -> Result<(), End> {
    let fail = |e: String| End::failed(CODE_PREPARE_DISK, "erasing", e);
    tell(say, "checking", "Checking the disk…", 5);
    if record().is_some() {
        return Err(End::failed(
            CODE_NO_PREPARE,
            "checking",
            "this stick already has an archive",
        ));
    }
    let disk = prepare_check(disk_id, shown).map_err(|r| {
        End::failed(
            CODE_NO_PREPARE,
            "checking",
            match r {
                PrepareRefusal::NotFound => "the disk is not plugged in any more".into(),
                PrepareRefusal::Changed => "the disk's partitions changed".into(),
                PrepareRefusal::Refused(m) => m,
            },
        )
    })?;
    if disk_in_use(&disk) {
        return Err(End::failed(
            CODE_NO_PREPARE,
            "checking",
            "a part of this disk is in use",
        ));
    }
    tell(say, "erasing", "Erasing the disk…", 30);
    eprintln!(
        "{} disk.prepare: erasing {disk_id} (password checked, list confirmed)",
        crate::job::LOG_PREFIX
    );
    let res = erase_and_format(
        &disk,
        disk_id,
        ALLOWED,
        "ALPArchive",
        crate::format::Fs::Exfat,
        60,
        say,
    );
    // Read-only again in every case.
    let mut back = Vec::new();
    if let Some(name) = allowed_name()
        && name != disk
    {
        back.push(name);
    }
    back.push(disk.clone());
    for d in &back {
        if let Err(e) = run("blockdev", &["--setro", &format!("/dev/{d}")]) {
            eprintln!("{} disk.prepare: {e}", crate::job::LOG_PREFIX);
        }
    }
    let _ = fs::remove_file(ALLOWED);
    // udev looks at the new filesystem (blkid), so the list shows it.
    let _ = run(
        "udevadm",
        &["trigger", "--action=change", &format!("/dev/{disk}")],
    );
    let _ = run("udevadm", &["settle", "--timeout=30"]);
    res.map_err(fail)?;
    tell(say, "done", "The disk is ready.", 100);
    Ok(())
}

/// New partition table with one exFAT partition labelled `label` on the
/// whole disk `disk` (still `disk_id`). `allow` is the allowance file that
/// keeps `protect-disks` off the disk while this runs ([`ALLOWED`] for
/// "Prepare this disk", `format::ALLOWED_FORMAT` for T49 format).
pub(crate) fn erase_and_format(
    disk: &str,
    disk_id: &str,
    allow_file: &str,
    label: &str,
    fs_kind: crate::format::Fs,
    percent: u8,
    say: Progress,
) -> Result<(), String> {
    let ext4 = fs_kind == crate::format::Fs::Ext4;
    let never = AtomicBool::new(false);
    let limit = Some(Duration::from_secs(10 * MINUTE));
    let dev = format!("/dev/{disk}");
    let sys = Path::new("/sys/block").join(disk);
    // The disk the user confirmed, still: the check above looked at every
    // partition (seconds), and an unplugged disk's name can be reused.
    // ... and still an external disk (ADR 0022; review R2: checked again
    // right before every write).
    let same = || {
        (disk_key(disk) == disk_id && crate::disks::is_external(disk))
            .then_some(())
            .ok_or_else(|| "the disk was unplugged; nothing was erased on another disk".to_string())
    };
    same()?;
    let bytes = read(&sys.join("size")).parse::<u64>().unwrap_or(0) * 512;
    fs::create_dir_all("/run/alp").map_err(|e| e.to_string())?;
    // A T49 format wrote its own allowance (disk id, review G1) already.
    if allow_file == ALLOWED {
        allow(disk, disk)?;
    }
    run("blockdev", &["--setrw", &dev])?;
    if read(&sys.join("ro")) != "0" {
        return Err("the disk stays read-only (is it write-protected?)".into());
    }
    let (args, script) = sfdisk_plan_for(&dev, bytes, ext4);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    same()?;
    crate::job::run_input("sfdisk", &args, script, &never, limit).map_err(|e| e.to_string())?;
    let _ = run("udevadm", &["settle", "--timeout=30"]);
    let mut parts: Vec<String> = fs::read_dir(&sys)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| sys.join(n).join("partition").is_file())
        .collect();
    parts.sort();
    let part = match parts.as_slice() {
        [one] => one.clone(),
        _ => return Err(format!("expected one new partition, found {}", parts.len())),
    };
    // "Prepare" narrows its allowance to the new partition; a T49 format
    // keeps the whole disk (it is not the archive's disk).
    if allow_file == ALLOWED {
        allow(&part, disk)?;
    }
    run("blockdev", &["--setrw", &format!("/dev/{part}")])?;
    let pdev = format!("/dev/{part}");
    // -m 0: no blocks kept back for root on a data disk.
    let (cmd, args, text): (&str, Vec<&str>, &str) = if ext4 {
        (
            "mkfs.ext4",
            vec!["-q", "-F", "-m", "0", "-L", label, &pdev],
            "Formatting the disk as ext4…",
        )
    } else {
        (
            "mkfs.exfat",
            vec!["-L", label, &pdev],
            "Formatting the disk as exFAT…",
        )
    };
    tell(say, "formatting", text, percent);
    same()?;
    crate::job::run(cmd, &args, &never, limit, &mut || ())
        .map(drop)
        .map_err(|e| e.to_string())
}

/// A job end as one line: "<code>: <cause> (step <step>)".
pub fn end_text(e: &End) -> String {
    match e {
        End::Cancelled => "stopped on request".into(),
        End::Failed {
            code,
            step,
            message,
        } => format!("{code}: {message} (step {step})"),
    }
}

/// `alpctl live archive …` entry point.
pub fn cli(args: &[&str]) -> i32 {
    let res = match args {
        ["candidates"] => {
            let all = candidates();
            for (p, v) in &all {
                match v {
                    Ok(pr) => println!(
                        "ok {} {} {} {} free {}{}",
                        p.partuuid,
                        p.name,
                        p.fs_type,
                        gb(p.size_bytes),
                        gb(pr.free_bytes),
                        if pr.dirty { " (will be repaired first)" } else { "" }
                    ),
                    Err(why) => println!(
                        "no {} {} {} {}: {}",
                        p.partuuid, p.name, p.fs_type, why.code, why.reason
                    ),
                }
            }
            let mut disks: Vec<&str> = all.iter().map(|(p, _)| p.disk.as_str()).collect();
            disks.dedup();
            for d in disks {
                let entries: Vec<_> = all.iter().filter(|(p, _)| p.disk == d).collect();
                if preparable(&entries) {
                    println!("can prepare (erase) disk {}", disk_key(d));
                }
            }
            Ok(())
        }
        ["create", partuuid] => create(partuuid, &AtomicBool::new(false), &mut |u| {
            println!("{}", u.message);
        })
        .map_err(|e| end_text(&e)),
        ["open"] => {
            let r = open();
            let _ = fs::create_dir_all("/run/alp");
            let _ = fs::write(OPEN_ERROR, r.as_ref().err().map_or("", String::as_str));
            r
        }
        ["close"] => close(),
        ["state-mount"] => state_mount(),
        _ => Err("usage: alpctl live archive candidates | create <PARTUUID> | open | close | state-mount".into()),
    };
    match res {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("ALP-OS Live: {e}");
            1
        }
    }
}

/// Screen flow: choose a partition, confirm, create. `next` returns the
/// next line typed.
pub fn setup(next: &mut dyn FnMut() -> String) {
    let say = |s: &str| {
        print!("{s}");
        let _ = std::io::stdout().flush();
    };
    say(
        "\x1b[H\x1b[2J\x1b[?25h ALPArchive  ·  Where to keep your archive\n\n Looking at this computer's disks (nothing is changed)…\n\n",
    );
    if Path::new(RECORD).exists() {
        say(" An archive is already set up with this stick. Press Enter to go back.\n");
        next();
        return;
    }
    if let Err(e) = state_ready() {
        say(&format!(
            " {}: {e}.\n Press Enter to go back.\n",
            crate::fail::display_safe("Not possible now", 40)
        ));
        next();
        return;
    }
    let all = candidates();
    let ok: Vec<(Part, Probe)> = all
        .iter()
        .filter_map(|(p, v)| v.clone().ok().map(|pr| (p.clone(), pr)))
        .collect();
    let mut sorted = ok.clone();
    sorted.sort_by_key(|(_, pr)| std::cmp::Reverse(pr.free_bytes));
    for (i, (p, pr)) in sorted.iter().enumerate() {
        let name = if p.label.is_empty() {
            p.name.clone()
        } else {
            format!("{} ({})", crate::fail::display_safe(&p.label, 20), p.name)
        };
        let rec = if i == 0 { "   ← recommended" } else { "" };
        say(&format!(
            "   {}  {name:<30} {} free{rec}\n",
            i + 1,
            gb(pr.free_bytes)
        ));
    }
    for (p, v) in &all {
        if let Err(why) = v {
            say(&format!(
                "   -  {:<30} not usable ({}): {why}\n",
                p.name, why.code
            ));
        }
    }
    if sorted.is_empty() {
        say("\n No partition can hold the archive. Press Enter to go back.\n");
        next();
        return;
    }
    say(
        "\n Live adds one folder, ALPArchive, with your encrypted photos. It never\n deletes or formats anything.\n\n Type a number and press Enter (Enter alone: 1): ",
    );
    let choice = next();
    let idx: usize = if choice.trim().is_empty() {
        1
    } else {
        choice.trim().parse().unwrap_or(0)
    };
    let Some((p, pr)) = sorted.get(idx.wrapping_sub(1)) else {
        return;
    };
    say(&format!(
        "\n The archive will grow into the {} free on {} as photos arrive. The disk is\n checked first (about a minute).\n\n Type yes and press Enter to create it (anything else: back): ",
        gb(max_size(pr.free_bytes)),
        p.name
    ));
    if next().trim() != "yes" {
        return;
    }
    say("\n");
    let mut last = String::new();
    let res = create(&p.partuuid, &AtomicBool::new(false), &mut |u| {
        if u.message != last {
            last.clone_from(&u.message);
            say(&format!(" {}\n", u.message));
        }
    });
    match res {
        Ok(()) => say("\n Press Enter to continue.\n"),
        Err(e) => say(&format!(
            "\n The archive could not be created: {}.\n Nothing was deleted; an ALPArchive folder may be left on that disk. Press Enter to go back.\n",
            end_text(&e)
        )),
    }
    next();
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_archive_allowance_names_one_plug_in_of_one_disk() {
        assert_eq!(parse_allowance("sdb1 sdb-17\n"), Some(("sdb1", "sdb-17")));
        assert_eq!(parse_allowance("sdb1"), None, "old name-only file");
        assert_eq!(parse_allowance("sdb1 sdb"), None);
        assert_eq!(parse_allowance("sdb1 sdb-17 x"), None);
        assert_eq!(parse_allowance(".. sdb-17"), None);
        assert_eq!(parse_allowance(". sdb-17"), None);
        assert_eq!(parse_allowance("../sda sdb-17"), None);
        assert_eq!(
            parse_allowance("nvme0n1p2 nvme0n1-3"),
            Some(("nvme0n1p2", "nvme0n1-3"))
        );
        assert_eq!(
            allowed_on("sdb1 sdb-17", "sdb", "sdb-17").as_deref(),
            Some("sdb1")
        );
        // Same name, plugged in later (new diskseq): nothing allowed.
        assert_eq!(allowed_on("sdb1 sdb-17", "sdb", "sdb-18"), None);
        // Another disk's allowance.
        assert_eq!(allowed_on("sdb1 sdb-17", "sdc", "sdc-17"), None);
        // A whole disk holding the archive.
        assert_eq!(
            allowed_on("sdc sdc-4", "sdc", "sdc-4").as_deref(),
            Some("sdc")
        );
    }

    #[test]
    fn plain_record_round_trip() {
        let r = Record {
            partuuid: "p".into(),
            fs_uuid: "f".into(),
            format: Format::Plain,
            id: "abc".into(),
        };
        assert_eq!(Record::from_toml(&r.to_toml()), Some(r));
    }

    use super::*;

    fn part(fs_type: &str) -> Part {
        Part {
            external: true,
            name: "sdb1".into(),
            disk: "sdb".into(),
            partuuid: "u".into(),
            type_uuid: "EBD0A0A2-B9E5-4433-87C0-68B6B72699C7".into(),
            fs_type: fs_type.into(),
            fs_uuid: "f".into(),
            label: "Backup".into(),
            size_bytes: 500_000_000_000,
            read_only: false,
        }
    }

    fn free(bytes: u64) -> Probe {
        Probe {
            free_bytes: bytes,
            ..Probe::default()
        }
    }

    fn code(p: &Part, pr: Option<&Probe>) -> Option<&'static str> {
        refusal(p, pr).map(|r| r.code)
    }

    #[test]
    fn refuses_unsafe_partitions() {
        let roomy = free(200_000_000_000);
        assert_eq!(refusal(&part("ntfs"), Some(&roomy)), None);
        assert_eq!(refusal(&part("ext4"), Some(&roomy)), None);
        assert_eq!(refusal(&part("exfat"), Some(&roomy)), None);
        let bl = refusal(&part("BitLocker"), None).unwrap();
        assert_eq!(bl.code, CODE_BITLOCKER);
        assert!(bl.reason.contains("BitLocker"));
        let fat = refusal(&part("vfat"), None).unwrap();
        assert_eq!(fat.code, CODE_FAT32);
        assert!(fat.reason.starts_with("FAT32 can't hold the archive."));
        assert_eq!(code(&part("btrfs"), None), Some(CODE_FORMAT));
        assert!(
            refusal(&part("btrfs"), None)
                .unwrap()
                .reason
                .contains("btrfs")
        );
        assert_eq!(code(&part(""), None), Some(CODE_FORMAT));
        assert!(refusal(&part(""), None).unwrap().reason.contains("empty"));
        let mut esp = part("vfat");
        esp.type_uuid = SPECIAL_TYPES[0].into();
        assert_eq!(code(&esp, None), Some(CODE_OS));
        let asleep = Probe {
            hibernated: true,
            ..roomy
        };
        let r = refusal(&part("ntfs"), Some(&asleep)).unwrap();
        assert_eq!(r.code, CODE_HIBERNATED);
        assert!(r.reason.contains("Fast Startup"));
        let mut ro = part("exfat");
        ro.read_only = true;
        assert_eq!(code(&ro, None), Some(CODE_READ_ONLY));
        assert!(
            refusal(&ro, None)
                .unwrap()
                .reason
                .contains("write-protected")
        );
    }

    #[test]
    fn small_external_disks_are_usable() {
        // Field test 2026-10-20: a 16 GB stick and an 8 GB card are fine now.
        assert_eq!(refusal(&part("exfat"), Some(&free(15_000_000_000))), None);
        assert_eq!(refusal(&part("ntfs"), Some(&free(7_500_000_000))), None);
        assert_eq!(refusal(&part("exfat"), Some(&free(min_free()))), None);
        let tight = refusal(&part("ext4"), Some(&free(MIN_BYTES))).unwrap();
        assert_eq!(tight.code, CODE_SPACE);
        // The numbers are in the sentence: free now, needed.
        assert!(tight.reason.contains("1.1 GB free"), "{}", tight.reason);
        assert!(
            tight.reason.contains("at least 1.1 GB needed"),
            "{}",
            tight.reason
        );
        assert_eq!(code(&part("ext4"), Some(&free(0))), Some(CODE_SPACE));
    }

    #[test]
    fn size_keeps_only_a_small_margin() {
        const GIB: u64 = 1 << 30;
        // 1 GiB margin once 5 % is more than that (free > 20 GiB).
        assert_eq!(max_size(500 * GIB), 499 * GIB);
        // 5 % below that: 10 GiB free keeps 512 MiB.
        assert_eq!(max_size(10 * GIB), 10 * GIB - 512 * MIB);
        // Whole MiB, never more than free minus margin.
        let odd = 3_333_333_333;
        assert_eq!(max_size(odd) % MIB, 0);
        assert!(max_size(odd) <= odd - margin(odd));
        assert!(max_size(min_free()) >= MIN_BYTES);
        assert!(max_size(MIN_BYTES) < MIN_BYTES);
        assert_eq!(max_size(0), 0);
        // The 24 GiB disk of T-LIVE-02 (about 21 GiB free on ext4).
        assert!(max_size(21 * GIB) >= MIN_BYTES);
    }

    #[test]
    fn verdicts_name_the_reason() {
        let ok = Some(Ok(free(100_000_000_000)));
        assert!(verdict(&part("exfat"), false, ok.clone()).is_ok());
        assert_eq!(
            verdict(&part("exfat"), true, ok.clone()).unwrap_err().code,
            CODE_OS
        );
        // Only the disk itself counts. A dirty one is usable now: it is
        // repaired first (ADR 0014); ALP-STO-108 is not returned.
        let dirty = Some(Ok(Probe {
            dirty: true,
            ..free(100_000_000_000)
        }));
        assert!(verdict(&part("exfat"), false, dirty).unwrap().dirty);
        assert_eq!(
            verdict(&part("ext4"), false, Some(Err("mount failed".into())))
                .unwrap_err()
                .code,
            CODE_UNREADABLE
        );
        // A format refusal wins over the (missing) probe.
        assert_eq!(
            verdict(&part("vfat"), false, None).unwrap_err().code,
            CODE_FAT32
        );
        // An old archive is offered even on a full disk.
        let old = Probe {
            has_archive: true,
            ..free(0)
        };
        assert!(verdict(&part("ext4"), false, Some(Ok(old))).is_ok());
    }

    #[test]
    fn whole_disk_and_extended_entries() {
        assert_eq!(whole_disk_id("sdb", "1234-ABCD"), "disk-1234-ABCD");
        assert_eq!(whole_disk_id("sdc", ""), "disk-sdc");
        assert!(is_extended("dos", "0x5"));
        assert!(is_extended("dos", "0xf"));
        assert!(!is_extended("dos", "0x7"));
        assert!(!is_extended("gpt", "0x5"));
        // A filesystem straight on the device is judged like a partition.
        let mut whole = part("exfat");
        whole.name = "sdb".into();
        whole.type_uuid = String::new();
        whole.partuuid = whole_disk_id("sdb", "1234-ABCD");
        assert_eq!(refusal(&whole, Some(&free(30_000_000_000))), None);
    }

    #[test]
    fn every_refusal_code_is_in_the_catalog() {
        let cat = include_str!("../../../errors/catalog.toml");
        for c in [
            CODE_SPACE,
            CODE_FAT32,
            CODE_OS,
            CODE_FORMAT,
            CODE_READ_ONLY,
            CODE_BITLOCKER,
            CODE_DIRTY,
            CODE_HIBERNATED,
            CODE_UNREADABLE,
            CODE_PREPARE,
            CODE_WRITE,
            CODE_STALLED,
            CODE_ENCRYPT,
            CODE_FORMAT_ARCHIVE,
            CODE_SLOW,
            CODE_TEST,
            CODE_SMART,
            CODE_REPAIR,
            CODE_PREPARE_DISK,
            CODE_NO_PREPARE,
        ] {
            assert!(cat.contains(&format!("[{c}]")), "{c} missing");
        }
    }

    #[test]
    fn finds_operating_systems() {
        let d = std::env::temp_dir().join(format!("alp-os-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("WINDOWS/system32")).unwrap();
        assert!(has_os(&d));
        fs::remove_dir_all(d.join("WINDOWS")).unwrap();
        assert!(!has_os(&d));
        fs::create_dir_all(d.join("etc")).unwrap();
        fs::write(d.join("etc/os-release"), "ID=debian\n").unwrap();
        assert!(has_os(&d));
    }

    #[test]
    fn dirty_flags() {
        let mut boot = [0u8; 512];
        boot[3..11].copy_from_slice(b"EXFAT   ");
        assert_eq!(exfat_dirty(&boot), Some(false));
        boot[106] = 0b10;
        assert_eq!(exfat_dirty(&boot), Some(true));
        boot[106] = 0b01; // ActiveFat only
        assert_eq!(exfat_dirty(&boot), Some(false));
        boot[3..11].copy_from_slice(b"NTFS    ");
        assert_eq!(exfat_dirty(&boot), None);
        assert_eq!(exfat_dirty(&boot[..100]), None);
        let mut sb = vec![0u8; 1024];
        assert_eq!(ext4_needs_recovery(&sb), None);
        sb[0x38] = 0x53;
        sb[0x39] = 0xEF;
        sb[0x60] = 0x2c2_u32.to_le_bytes()[0];
        sb[0x61] = 0x2c2_u32.to_le_bytes()[1];
        assert_eq!(ext4_needs_recovery(&sb), Some(false));
        sb[0x60] |= 0x4;
        assert_eq!(ext4_needs_recovery(&sb), Some(true));
        assert_eq!(ext4_needs_recovery(&sb[..0x40]), None);
        let info = "ntfs3.1\n4096\n1000\n64\n60\ndirty\nclean\n";
        assert_eq!(ntfs_volinfo_dirty(info), Some(true));
        assert_eq!(
            ntfs_volinfo_dirty(&info.replace("dirty", "clean")),
            Some(false)
        );
        assert_eq!(ntfs_volinfo_dirty("ntfs3.1\n4096\n"), None);
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("alp-archive-{name}-{}", std::process::id()))
    }

    #[test]
    fn disk_test_writes_reads_back_and_cleans_up() {
        let d = tmp("disktest");
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let no = AtomicBool::new(false);
        let mut seen = Vec::new();
        let mut remounts = 0;
        let r = disk_test(
            &d,
            10 * 4096 + 100,
            4096,
            3 * 4096,
            7,
            &no,
            &mut || {
                remounts += 1;
                Ok(())
            },
            &mut |done, _| seen.push(done),
        );
        assert_eq!(r, Ok(()));
        assert_eq!(remounts, 1);
        // Progress after every fsync (every 3 blocks and at the end).
        assert_eq!(seen, [0, 3 * 4096, 6 * 4096, 9 * 4096, 10 * 4096 + 100]);
        assert!(!d.join(TEST_FILE).exists(), "the test file is deleted");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn disk_test_finds_differences_and_stops_when_cancelled() {
        let d = tmp("disktest-bad");
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let no = AtomicBool::new(false);
        let file = d.join(TEST_FILE);
        // A "disk" that changes one byte between writing and reading.
        let r = disk_test(
            &d,
            8 * 4096,
            4096,
            4096,
            1,
            &no,
            &mut || {
                let mut b = fs::read(&file).unwrap();
                b[5000] ^= 1;
                fs::write(&file, b).map_err(|e| e.to_string())
            },
            &mut |_, _| (),
        );
        match r {
            Err(End::Failed {
                code,
                step,
                message,
            }) => {
                assert_eq!((code, step), (CODE_TEST, "checking"));
                assert!(message.contains("differs"), "{message}");
            }
            other => panic!("{other:?}"),
        }
        assert!(!file.exists());
        // A remount that fails is a faulty disk too.
        let r = disk_test(
            &d,
            4096,
            4096,
            4096,
            1,
            &no,
            &mut || Err("busy".into()),
            &mut |_, _| (),
        );
        assert!(
            matches!(
                r,
                Err(End::Failed {
                    code: CODE_TEST,
                    ..
                })
            ),
            "{r:?}"
        );
        // Cancel while writing: stopped, the test file deleted.
        let stop = AtomicBool::new(false);
        let r = disk_test(
            &d,
            64 * 4096,
            4096,
            4096,
            1,
            &stop,
            &mut || Ok(()),
            &mut |done, _| {
                if done >= 2 * 4096 {
                    stop.store(true, Ordering::Relaxed);
                }
            },
        );
        assert_eq!(r, Err(End::Cancelled));
        assert!(!file.exists());
        // Writing where nothing can be written: an I/O error, code 116.
        let r = disk_test(
            &d.join("missing"),
            4096,
            4096,
            4096,
            1,
            &no,
            &mut || Ok(()),
            &mut |_, _| (),
        );
        assert!(
            matches!(
                r,
                Err(End::Failed {
                    code: CODE_TEST,
                    ..
                })
            ),
            "{r:?}"
        );
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn test_pattern_and_speed_rules() {
        let (mut a, mut b, mut c) = ([0u8; 64], [0u8; 64], [0u8; 61]);
        pattern(9, 0, &mut a);
        pattern(9, 1, &mut b);
        assert_ne!(a, b, "every block differs");
        pattern(9, 0, &mut c);
        assert_eq!(&a[..61], &c[..], "the same block reads the same");
        assert!(a.iter().any(|x| *x != a[0]));
        // 256 MiB: 3 MB/s at most 89.5 s; and never more than 100 s.
        assert!(speed_ok(TEST_BYTES, 89.0));
        assert!(!speed_ok(TEST_BYTES, 90.0));
        assert!(!speed_ok(1 << 40, 101.0));
        assert!(speed_ok(TEST_BYTES, 0.0));
        assert!(speed_ok(4096, 0.5));
        assert!(!speed_ok(4096, 1.5));
        let u = checking_update(128 << 20, TEST_BYTES, Some(18_000_000));
        assert_eq!((u.step, u.percent), ("checking", 22));
        assert_eq!(u.message, "Checking the disk: 128 MB of 256 MB at 18 MB/s…");
        assert_eq!(checking_update(TEST_BYTES, TEST_BYTES, None).percent, 34);
    }

    #[test]
    fn smart_answers() {
        assert_eq!(smart_health(Some(0)), Health::Ok);
        // Errors logged in the past (bit 6) do not refuse a disk.
        assert_eq!(smart_health(Some(64)), Health::Ok);
        assert_eq!(smart_health(Some(8)), Health::Failing);
        assert_eq!(smart_health(Some(16)), Health::Failing);
        assert_eq!(smart_health(Some(8 | 4)), Health::Failing);
        // Could not open or talk to it (a stick, a bridge without SAT,
        // standby): skipped.
        assert_eq!(smart_health(Some(2)), Health::Unknown);
        assert_eq!(smart_health(Some(4)), Health::Unknown);
        assert_eq!(smart_health(Some(1)), Health::Unknown);
        assert_eq!(smart_health(None), Health::Unknown);
        assert_eq!(
            smart_args("/dev/sdb", false),
            ["-H", "-n", "standby", "/dev/sdb"]
        );
        assert_eq!(
            smart_args("/dev/sdb", true),
            ["-H", "-n", "standby", "-d", "sat", "/dev/sdb"]
        );
    }

    #[test]
    fn repairs_and_mount_options() {
        let (cmd, args) = repair_command("exfat", "/dev/sdb1").unwrap();
        assert_eq!(
            (cmd, args.as_slice()),
            (
                "fsck.exfat",
                ["-p", "/dev/sdb1"].map(String::from).as_slice()
            )
        );
        let (cmd, args) = repair_command("ntfs", "/dev/sdb1").unwrap();
        assert_eq!(
            (cmd, args.as_slice()),
            ("ntfsfix", ["-d", "/dev/sdb1"].map(String::from).as_slice())
        );
        assert!(repair_command("ext4", "/dev/sdb1").is_none());
        assert!(repair_ok("exfat", Some(0)) && repair_ok("exfat", Some(1)));
        assert!(!repair_ok("exfat", Some(4)) && !repair_ok("exfat", None));
        assert!(repair_ok("ntfs", Some(0)) && !repair_ok("ntfs", Some(1)));
        assert_eq!(
            mount_options("exfat", true),
            "rw,nosuid,nodev,noexec,uid=50000,gid=50000,fmask=0077,dmask=0077"
        );
        assert_eq!(mount_options("ntfs", true), mount_options("exfat", true));
        assert_eq!(mount_options("ext4", true), "rw,nosuid,nodev,noexec");
        assert_eq!(
            mount_options("ext4", false),
            "ro,noload,nosuid,nodev,noexec"
        );
        assert_eq!(mount_options("exfat", false), "ro,nosuid,nodev,noexec");
        assert_eq!(immich_originals_dir(), Path::new(crate::immich::ORIGINALS));
        assert_eq!(
            immich_originals_dir(),
            Path::new("/var/lib/alp-pool/files/immich")
        );
    }

    #[test]
    fn archive_kinds_on_a_disk() {
        let root = tmp("kinds");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(DIR).join(DATA_PARTIAL)).unwrap();
        // An unfinished folder is no archive.
        assert_eq!(kind_at(&root), None);
        fs::write(root.join(DIR).join(FILE), "x").unwrap();
        assert_eq!(kind_at(&root), Some(Format::Luks));
        fs::create_dir_all(root.join(DIR).join(DATA)).unwrap();
        fs::write(root.join(DIR).join(DATA).join("gocryptfs.conf"), "{}").unwrap();
        assert_eq!(kind_at(&root), Some(Format::Gocryptfs));
        fs::remove_dir_all(&root).unwrap();
    }

    fn entry(
        name: &str,
        fs_type: &str,
        v: Result<Probe, Refusal>,
    ) -> (Part, Result<Probe, Refusal>) {
        let mut p = part(fs_type);
        p.name = name.into();
        p.partuuid = format!("uuid-{name}");
        (p, v)
    }

    fn no(code: &'static str) -> Result<Probe, Refusal> {
        Err(Refusal::new(code, "x"))
    }

    #[test]
    fn which_disks_may_be_prepared() {
        let fat = entry("sdb1", "vfat", no(CODE_FAT32));
        let unknown = entry("sdb2", "", no(CODE_FORMAT));
        let broken = entry("sdb3", "ntfs", no(CODE_UNREADABLE));
        let ok = entry("sdb4", "exfat", Ok(free(10_000_000_000)));
        assert!(preparable(&[&fat, &unknown, &broken]));
        // Anything usable, an archive, an OS, BitLocker, a full disk with
        // files, Windows asleep: never.
        assert!(!preparable(&[&fat, &ok]));
        let archive = entry(
            "sdb5",
            "exfat",
            Ok(Probe {
                has_archive: true,
                ..free(0)
            }),
        );
        assert!(!preparable(&[&fat, &archive]));
        for code in [
            CODE_OS,
            CODE_BITLOCKER,
            CODE_SPACE,
            CODE_HIBERNATED,
            CODE_READ_ONLY,
        ] {
            assert!(
                !preparable(&[&fat, &entry("sdb6", "ntfs", no(code))]),
                "{code}"
            );
        }
        let mut ro = fat.clone();
        ro.0.read_only = true;
        assert!(!preparable(&[&ro]), "a write-protected FAT32 stick");
        let mut inside = fat.clone();
        inside.0.external = false;
        assert!(!preparable(&[&inside]));
        let mut esp_like = unknown.clone();
        esp_like.0.type_uuid = OS_TYPES[3].into();
        assert!(!preparable(&[&esp_like]));
        assert!(!preparable(&[]));
        // The repair that did not work makes a volume preparable.
        assert!(preparable(&[&entry("sdb1", "exfat", no(CODE_REPAIR))]));
    }

    #[test]
    fn prepare_needs_the_same_disk_and_list() {
        let fat = entry("sdb1", "vfat", no(CODE_FAT32));
        let unknown = entry("sdb2", "", no(CODE_FORMAT));
        let list = [&fat, &unknown];
        let shown = vec!["uuid-sdb2".to_string(), "uuid-sdb1".to_string()];
        assert_eq!(
            prepare_decision("sdb-17", Some("sdb-17"), &list, &shown),
            Ok(())
        );
        // Plugged in again (new diskseq) or gone.
        assert_eq!(
            prepare_decision("sdb-17", Some("sdb-18"), &list, &shown),
            Err(PrepareRefusal::NotFound)
        );
        assert_eq!(
            prepare_decision("sdb-17", None, &list, &shown),
            Err(PrepareRefusal::NotFound)
        );
        assert_eq!(
            prepare_decision("sdb-17", Some("sdb-17"), &list, &shown[..1]),
            Err(PrepareRefusal::Changed)
        );
        let ok = entry("sdb3", "exfat", Ok(free(10_000_000_000)));
        let mut more = shown.clone();
        more.push("uuid-sdb3".into());
        assert!(matches!(
            prepare_decision("sdb-17", Some("sdb-17"), &[&fat, &unknown, &ok], &more),
            Err(PrepareRefusal::Refused(_))
        ));
        assert_eq!(disk_of_id("sdb-17"), Some("sdb"));
        assert_eq!(disk_of_id("nvme0n1-3"), Some("nvme0n1"));
        for bad in [
            "sdb", "sdb-", "-17", "../sda-1", "sdb-1x", "SDB-1", "sd b-1",
        ] {
            assert_eq!(disk_of_id(bad), None, "{bad}");
        }
    }

    #[test]
    fn prepare_partitions_the_whole_disk_once() {
        let (args, script) = sfdisk_plan("/dev/sdb", 1_000_000_000_000);
        assert_eq!(
            args,
            [
                "--lock",
                "--wipe",
                "always",
                "--wipe-partitions",
                "always",
                "--label",
                "dos",
                "/dev/sdb"
            ]
        );
        assert_eq!(script, "type=7\n");
        let (args, script) = sfdisk_plan("/dev/sdb", MBR_MAX + 512);
        assert_eq!(args[6], "gpt");
        assert_eq!(script, "type=EBD0A0A2-B9E5-4433-87C0-68B6B72699C7\n");
        assert_eq!(sfdisk_plan("/dev/sdb", MBR_MAX).0[6], "dos");
    }

    #[test]
    fn reset_deletes_only_our_own_files() {
        let root = tmp("reset");
        let _ = fs::remove_dir_all(&root);
        let dir = root.join(DIR);
        fs::create_dir_all(&dir).unwrap();
        for f in [FILE, "archive.alp.partial", "README.txt", "holiday.jpg"] {
            fs::write(dir.join(f), "x").unwrap();
        }
        fs::write(root.join("letter.docx"), "mine").unwrap();
        let outside = root.join("outside.txt");
        fs::write(&outside, "keep").unwrap();
        // A link named like ours is not followed (nor deleted).
        let _ = fs::remove_file(dir.join("README.txt"));
        std::os::unix::fs::symlink(&outside, dir.join("README.txt")).unwrap();
        assert_eq!(remove_own_files(&dir), Ok(true));
        let mut left: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["README.txt", "holiday.jpg"]);
        assert_eq!(fs::read_to_string(&outside).unwrap(), "keep");
        assert_eq!(
            fs::read_to_string(root.join("letter.docx")).unwrap(),
            "mine"
        );
        // A new archive: its folder, recovery config, kept configs and test
        // file are ours; a link to a folder is not followed.
        let data = dir.join(DATA);
        fs::create_dir_all(data.join("ab")).unwrap();
        fs::write(data.join("ab").join("cd"), "enc").unwrap();
        for f in [RECOVERY_CONF, "gocryptfs.conf.replaced-20261025", TEST_FILE] {
            fs::write(dir.join(f), "x").unwrap();
        }
        let elsewhere = root.join("photos");
        fs::create_dir_all(&elsewhere).unwrap();
        fs::write(elsewhere.join("a.jpg"), "mine").unwrap();
        std::os::unix::fs::symlink(&elsewhere, dir.join(DATA_PARTIAL)).unwrap();
        assert_eq!(remove_own_files(&dir), Ok(true));
        assert!(!data.exists());
        assert!(elsewhere.join("a.jpg").exists());
        let mut left: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["README.txt", "data.partial", "holiday.jpg"]);
        fs::remove_file(dir.join(DATA_PARTIAL)).unwrap();
        // Only our files there: the folder goes too, the disk's files stay.
        fs::remove_file(dir.join("holiday.jpg")).unwrap();
        fs::remove_file(dir.join("README.txt")).unwrap();
        fs::write(dir.join("README.txt"), "x").unwrap();
        assert_eq!(remove_own_files(&dir), Ok(false));
        assert!(!dir.exists());
        assert!(root.join("letter.docx").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn hibernation_header() {
        assert!(hiberfil_says_hibernated(b"HIBRxxxx"));
        assert!(hiberfil_says_hibernated(b"hibr"));
        assert!(!hiberfil_says_hibernated(b"\0\0\0\0"));
        assert!(!hiberfil_says_hibernated(b"wake"));
        assert!(!hiberfil_says_hibernated(b"hi"));
    }

    #[test]
    fn recovery_keys() {
        let k = "fkblrkiu-ghdbbctr-vudvhlkc-tcrlvgbb-jcvnhfhl-tdflktnl-jhhcgrdk-uldjueee";
        assert_eq!(normalize_recovery_key(k).as_deref(), Some(k));
        let typed = "FKBL RKIU ghdb-bctr vudvhlkc tcrlvgbb jcvnhfhl tdflktnl jhhcgrdk uldjueee";
        assert_eq!(normalize_recovery_key(typed).as_deref(), Some(k));
        assert!(normalize_recovery_key(&k.replace('f', "a")).is_none());
        assert!(normalize_recovery_key(&k[..60]).is_none());
        let out = format!(
            "A secret recovery key has been generated for this volume:\n\n    \u{1f510} {k}\n\nPlease save it."
        );
        assert_eq!(find_recovery_key(&out).as_deref(), Some(k));
        assert!(find_recovery_key("no key here").is_none());
    }

    #[test]
    fn record_round_trip() {
        let r = Record {
            partuuid: "p".into(),
            fs_uuid: "f".into(),
            format: Format::Luks,
            id: "l".into(),
        };
        assert_eq!(Record::from_toml(&r.to_toml()), Some(r));
        // ALPHA 0.1 records (no format) still read as LUKS archives.
        let old = "partuuid = \"p\"\nfs_uuid = \"f\"\nluks_uuid = \"l\"\npath = \"ALPArchive/archive.alp\"\n";
        assert_eq!(Record::from_toml(old).map(|r| r.format), Some(Format::Luks));
        let n = Record {
            partuuid: "p".into(),
            fs_uuid: "f".into(),
            format: Format::Gocryptfs,
            id: "ab".repeat(16),
        };
        assert!(n.to_toml().contains("format = \"gocryptfs\""));
        assert!(n.to_toml().contains("path = \"ALPArchive/data\""));
        assert_eq!(Record::from_toml(&n.to_toml()), Some(n));
        assert_eq!(Record::from_toml("partuuid = 'p'"), None);
        assert_eq!(
            Record::from_toml("partuuid = 'p'\nfs_uuid = 'f'\nformat = 'other'\narchive_id = 'x'"),
            None
        );
    }
}
