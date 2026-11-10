// SPDX-License-Identifier: AGPL-3.0-or-later
//! Read-only access to external disks for Import (T47) and the Files and
//! Disks pages (T49), under the Security and Network Expert's conditions
//! (`docs/security` copy of ux-0.2.4-security-review.md, sections 3 and 4;
//! storage-plan-review-2026-11-03.md):
//!
//! - Only external disks. Never the stick, the computer's internal disks,
//!   or any partition of the disk the archive is on.
//! - Only vfat, exfat, ntfs (ntfs3), ext4, hfsplus and iso9660: every other
//!   kernel file-system driver is attack surface for a malicious disk.
//!   A partition where `blkid -p` finds more than one signature is refused.
//! - The partition is made read-only (`blockdev --setro`) before it is
//!   mounted `ro,nosuid,nodev,noexec,nosymfollow` under
//!   `/run/alp/media/<random id>`; the mount is checked to be that device.
//! - Files are opened one path component at a time with `O_NOFOLLOW`, so a
//!   symbolic link on the disk is never followed; `..`, absolute paths and
//!   special files (devices, FIFOs, sockets) are refused.
//! - Every call checks the partition's role again: disks can be swapped
//!   between calls.
//!
//! Callers name a partition only by the opaque id from [`list`]; ids are
//! random per start and never reveal the device.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::archive::{self, Part};
use crate::fail::Fail;

/// Where external disks are mounted (one folder per random id, 0700).
pub const ROOT: &str = "/run/alp/media";
/// The id map (root only): id → partition, kept for this start only.
const IDS: &str = "/run/alp/media/.ids.toml";

/// File systems that may be mounted, with the kernel driver used.
const ALLOWED_FS: [(&str, &str); 6] = [
    ("vfat", "vfat"),
    ("exfat", "exfat"),
    ("ntfs", "ntfs3"),
    ("ext4", "ext4"),
    ("hfsplus", "hfsplus"),
    ("iso9660", "iso9660"),
];

// Linux x86_64 open(2) flags (the image is x86_64 only; no libc crate and
// no unsafe code in this workspace).
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const O_NOFOLLOW: i32 = 0o400_000;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const O_DIRECTORY: i32 = 0o200_000;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const O_NONBLOCK: i32 = 0o4_000;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const O_NOCTTY: i32 = 0o400;

/// Longest path accepted below a mount root, and longest single name.
const MAX_PATH: usize = 4096;
const MAX_NAME: usize = 255;

/// One external partition as the API shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Media {
    pub id: String,
    pub label: String,
    pub fs_type: String,
    pub size_bytes: u64,
    pub mounted: bool,
    /// `None`: may be mounted read-only. `Some(reason)`: refused.
    pub refused: Option<String>,
}

/// A mounted partition. Paths are always relative to its root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountedMedia {
    pub id: String,
    pub root: PathBuf,
}

/// One lock for the id map and the mounts in this process: two calls at
/// once must not give one disk two ids, or sweep a folder another call is
/// mounting or reading from (review F1).
static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn locked() -> std::sync::MutexGuard<'static, ()> {
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Most entries one [`MountedMedia::walk`] returns (a crafted disk can
/// hold millions in one folder).
pub const MAX_ENTRIES: usize = 10_000;

/// A page of a folder listing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Listing {
    pub entries: Vec<Entry>,
    /// More entries follow: ask again with `offset + entries.len()`.
    pub truncated: bool,
    /// Names left out because they are not valid UTF-8 (they could not be
    /// shown or asked for again).
    pub skipped: usize,
}

/// One entry of a folder listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
    pub size_bytes: u64,
}

fn refused(msg: impl Into<String>) -> Fail {
    Fail::new("media.refused", msg)
}

fn not_found() -> Fail {
    Fail::new(
        "media.not_found",
        "That disk is not plugged in any more. Plug it in and try again.",
    )
}

/// Kernel driver for a file-system type, or `None` when it is not allowed.
pub fn driver(fs_type: &str) -> Option<&'static str> {
    ALLOWED_FS
        .iter()
        .find(|(t, _)| *t == fs_type)
        .map(|(_, d)| *d)
}

/// Mount options: read-only, no set-uid, no devices, no programs, no
/// symbolic links followed by any path lookup (`nosymfollow`, Linux 5.10+);
/// ext4 never replays a foreign journal (as `archive::mount_options`).
pub fn mount_options(fs_type: &str) -> &'static str {
    if fs_type == "ext4" {
        "ro,noload,nosuid,nodev,noexec,nosymfollow"
    } else {
        "ro,nosuid,nodev,noexec,nosymfollow"
    }
}

/// Why this partition may not be mounted, or `None`. `archive_disk` is
/// the disk the archive record points to (every partition of it is
/// refused); `probe` is `blkid -p`'s verdict ([`probe_verdict`]), `None`
/// while it was not run yet (the cheap checks come first).
pub fn refusal(
    p: &Part,
    archive_disk: Option<&str>,
    probe: Option<ProbeVerdict>,
) -> Option<String> {
    if !p.external {
        return Some("The computer's own disks are never opened here.".into());
    }
    if archive_disk == Some(p.disk.as_str()) {
        return Some("This is your backup disk; its files are in the gallery.".into());
    }
    if p.fs_type == "crypto_LUKS" {
        return Some("This disk is encrypted with another system.".into());
    }
    if driver(&p.fs_type).is_none() {
        return Some(if p.fs_type.is_empty() {
            "No files were found on this part of the disk.".into()
        } else {
            format!(
                "ALPArchive cannot read this kind of disk ({}).",
                crate::fail::display_safe(&p.fs_type, 20)
            )
        });
    }
    match probe {
        Some(ProbeVerdict::Ambiguous) => {
            Some("This disk looks like two kinds of disk at once, so it is not opened.".into())
        }
        Some(ProbeVerdict::Unchecked) => {
            Some("This disk could not be checked, so it is not opened.".into())
        }
        _ => None,
    }
}

/// How long `blkid -p` may take (slow USB 2 sticks can hang it).
const PROBE_SECS: &str = "15";

/// The low-level probe's verdict. `blkid -p` exits 0 with one clear
/// signature and 8 on an ambivalent result (two signatures); any other
/// exit, a time-out or a failure to run it means "could not be checked".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeVerdict {
    Clear,
    Ambiguous,
    Unchecked,
}

/// Fail closed: only exit 0 is [`ProbeVerdict::Clear`].
pub fn probe_verdict(exit_code: Option<i32>) -> ProbeVerdict {
    match exit_code {
        Some(0) => ProbeVerdict::Clear,
        Some(8) => ProbeVerdict::Ambiguous,
        _ => ProbeVerdict::Unchecked,
    }
}

fn probe(name: &str) -> ProbeVerdict {
    let code = Command::new("timeout")
        .args([
            "--kill-after=5",
            PROBE_SECS,
            "blkid",
            "-p",
            "-o",
            "export",
            &format!("/dev/{name}"),
        ])
        .output()
        .ok()
        .and_then(|o| o.status.code());
    probe_verdict(code)
}

/// All checks, the cheap ones first; `blkid` runs only when they pass.
fn verdict(p: &Part, archive_disk: Option<&str>) -> Option<String> {
    refusal(p, archive_disk, None).or_else(|| refusal(p, None, Some(probe(&p.name))))
}

/// The disk the archive is on, from the record on the stick and the
/// partitions now plugged in.
fn archive_disk(parts: &[Part]) -> Option<String> {
    let rec = archive::record()?;
    parts
        .iter()
        .find(|p| p.partuuid.eq_ignore_ascii_case(&rec.partuuid))
        .map(|p| p.disk.clone())
}

/// The key that ties an id to a partition: a swapped or re-plugged disk
/// gets another key (`diskseq` is new for every plug-in).
fn part_key(p: &Part) -> String {
    format!(
        "{}|{}|{}",
        archive::disk_key(&p.disk),
        p.partuuid,
        p.fs_uuid
    )
}

fn load_ids() -> BTreeMap<String, String> {
    fs::read_to_string(IDS)
        .ok()
        .and_then(|s| s.parse::<toml::Table>().ok())
        .map(|t| {
            t.into_iter()
                .filter_map(|(k, v)| Some((k, v.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

fn save_ids(ids: &BTreeMap<String, String>) -> Result<(), String> {
    ensure_root()?;
    let t: toml::Table = ids
        .iter()
        .map(|(k, v)| (k.clone(), toml::Value::String(v.clone())))
        .collect();
    let tmp = format!("{IDS}.tmp");
    let _ = fs::remove_file(&tmp);
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(O_NOFOLLOW)
        .open(&tmp)
        .and_then(|mut f| std::io::Write::write_all(&mut f, t.to_string().as_bytes()))
        .map_err(|e| e.to_string())?;
    fs::rename(&tmp, IDS).map_err(|e| e.to_string())
}

/// `/run/alp/media` exists, is a real folder (not a link), root's, 0700.
fn ensure_root() -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(ROOT)
        .map_err(|e| e.to_string())?;
    let m = fs::symlink_metadata(ROOT).map_err(|e| e.to_string())?;
    if root_ok(m.file_type().is_dir(), m.uid(), m.mode()) {
        Ok(())
    } else {
        Err(format!("{ROOT} is not a folder of root's with mode 0700"))
    }
}

/// The checks of [`ensure_root`], apart for the tests.
pub fn root_ok(is_dir: bool, uid: u32, mode: u32) -> bool {
    is_dir && uid == 0 && mode & 0o7777 == 0o700
}

/// A new opaque id: 12 random bytes as hex.
fn new_id() -> Result<String, String> {
    let mut b = [0u8; 12];
    fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .map_err(|e| e.to_string())?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// Is `id` a well-formed id (so it can be a folder name)?
pub fn valid_id(id: &str) -> bool {
    id.len() == 24
        && id
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

fn mount_point(id: &str) -> PathBuf {
    Path::new(ROOT).join(id)
}

/// `major:minor` of the file system mounted at `at`, from mountinfo text
/// (field 3; field 5 is the mount point; the last mount there wins).
pub fn mount_dev(mountinfo: &str, at: &str) -> Option<String> {
    mountinfo
        .lines()
        .rev()
        .map(|l| l.split_whitespace().collect::<Vec<_>>())
        .find(|f| f.get(4) == Some(&at))
        .and_then(|f| f.get(2).map(|s| s.to_string()))
}

fn is_mounted(at: &Path) -> bool {
    let at = at.to_string_lossy();
    fs::read_to_string("/proc/self/mountinfo")
        .is_ok_and(|m| m.lines().any(|l| l.split_whitespace().nth(4) == Some(&*at)))
}

/// Every partition of every external disk, with an id and its verdict.
pub fn list() -> Result<Vec<Media>, Fail> {
    let _g = locked();
    let parts = archive::partitions();
    let adisk = archive_disk(&parts);
    let mut ids = load_ids();
    let mut out = Vec::new();
    for p in parts.iter().filter(|p| p.external) {
        let key = part_key(p);
        let id = match ids.iter().find(|(_, k)| **k == key) {
            Some((id, _)) => id.clone(),
            None => {
                let id = new_id().map_err(|e| Fail::new("media.failed", e))?;
                ids.insert(id.clone(), key);
                id
            }
        };
        let no = verdict(p, adisk.as_deref());
        out.push(Media {
            mounted: is_mounted(&mount_point(&id)),
            id,
            label: crate::fail::display_safe(&p.label, 64),
            fs_type: p.fs_type.clone(),
            size_bytes: p.size_bytes,
            refused: no,
        });
    }
    // Forget ids of disks that are gone, and let go of their folders: a
    // disk pulled out while mounted leaves a dead mount behind.
    let present: Vec<String> = parts.iter().filter(|p| p.external).map(part_key).collect();
    ids.retain(|_, k| present.contains(k));
    save_ids(&ids).map_err(|e| Fail::new("media.failed", e))?;
    let folders: Vec<String> = fs::read_dir(ROOT)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    for id in orphans(&folders, &ids) {
        let at = mount_point(&id);
        if is_mounted(&at)
            && let Err(e) = run("umount", &["-l", &at.to_string_lossy()])
        {
            eprintln!("ALP-OS Live: media: could not let go of {id}: {e}");
            continue;
        }
        let _ = fs::remove_dir(&at);
    }
    Ok(out)
}

/// Folders under [`ROOT`] that no current id owns (the id map's own files
/// are not ids, so they never count).
pub fn orphans(folders: &[String], ids: &BTreeMap<String, String>) -> Vec<String> {
    folders
        .iter()
        .filter(|f| valid_id(f) && !ids.contains_key(*f))
        .cloned()
        .collect()
}

/// The partition behind `id`, checked again now (role, file system).
fn resolve(id: &str) -> Result<Part, Fail> {
    if !valid_id(id) {
        return Err(not_found());
    }
    let key = load_ids().get(id).cloned().ok_or_else(not_found)?;
    let parts = archive::partitions();
    let p = parts
        .iter()
        .find(|p| part_key(p) == key)
        .cloned()
        .ok_or_else(not_found)?;
    if let Some(why) = verdict(&p, archive_disk(&parts).as_deref()) {
        return Err(refused(why));
    }
    if crate::format::formatting(&p.disk) {
        return Err(refused("This disk is being formatted."));
    }
    Ok(p)
}

/// Mount `id` read-only (or return the existing mount).
pub fn mount_ro(id: &str) -> Result<MountedMedia, Fail> {
    let _g = locked();
    let p = resolve(id)?;
    let at = mount_point(id);
    // The raw cause goes to the log (the (i) window); the screen gets a
    // plain sentence (review S2).
    let failed = |e: String| {
        eprintln!("ALP-OS Live: media: {id}: {e}");
        Fail::new(
            "media.failed",
            "ALPArchive could not open this disk. Unplug it, plug it in again and try once more.",
        )
    };
    if is_mounted(&at) {
        return Ok(MountedMedia {
            id: id.into(),
            root: at,
        });
    }
    let dev = format!("/dev/{}", p.name);
    run("blockdev", &["--setro", &dev]).map_err(failed)?;
    if fs::read_to_string(Path::new("/sys/class/block").join(&p.name).join("ro"))
        .map(|s| s.trim() != "1")
        .unwrap_or(true)
    {
        return Err(failed("it could not be made read-only".into()));
    }
    ensure_root().map_err(failed)?;
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&at)
        .or_else(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                Ok(())
            } else {
                Err(e)
            }
        })
        .map_err(|e| failed(e.to_string()))?;
    let drv = driver(&p.fs_type).ok_or_else(|| refused("This kind of disk cannot be read."))?;
    if let Err(e) = run(
        "mount",
        &[
            "-t",
            drv,
            "-o",
            mount_options(&p.fs_type),
            &dev,
            &at.to_string_lossy(),
        ],
    ) {
        let _ = fs::remove_dir(&at);
        return Err(failed(e));
    }
    // The mount really is the device that was checked (review S1): the
    // device name could have moved to another disk in between.
    let want = fs::read_to_string(Path::new("/sys/class/block").join(&p.name).join("dev"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let got = fs::read_to_string("/proc/self/mountinfo")
        .ok()
        .and_then(|m| mount_dev(&m, &at.to_string_lossy()));
    if want.is_empty() || got.as_deref() != Some(want.as_str()) {
        let _ = run("umount", &[&at.to_string_lossy()]);
        let _ = fs::remove_dir(&at);
        return Err(failed(format!(
            "mounted device {got:?} is not the checked one {want:?}"
        )));
    }
    Ok(MountedMedia {
        id: id.into(),
        root: at,
    })
}

/// Unmount `id` and remove its folder.
pub fn eject(id: &str) -> Result<(), Fail> {
    let _g = locked();
    if !valid_id(id) {
        return Err(not_found());
    }
    let at = mount_point(id);
    if is_mounted(&at) {
        run("umount", &[&at.to_string_lossy()])
            .map_err(|e| Fail::new("media.busy", format!("The disk is still in use: {e}.")))?;
    }
    let _ = fs::remove_dir(&at);
    Ok(())
}

/// Split a path given by the screen into names, refusing anything that
/// could leave the mount root.
pub fn components(rel: &str) -> Result<Vec<&str>, Fail> {
    let bad = || Fail::new("media.path", "That is not a file or folder on this disk.");
    if rel.len() > MAX_PATH || rel.contains('\0') || rel.starts_with('/') {
        return Err(bad());
    }
    let mut out = Vec::new();
    for c in rel.split('/') {
        match c {
            "" | "." => continue,
            ".." => return Err(bad()),
            c if c.len() > MAX_NAME => return Err(bad()),
            c => out.push(c),
        }
    }
    Ok(out)
}

impl MountedMedia {
    /// Open each folder on the way with `O_NOFOLLOW | O_DIRECTORY`, through
    /// the previous folder's own descriptor, so no symbolic link anywhere
    /// on the path is followed.
    fn walk_dirs(&self, names: &[&str]) -> Result<fs::File, Fail> {
        let bad = || Fail::new("media.path", "That is not a file or folder on this disk.");
        let mut dir = open_nofollow(&self.root, true).map_err(|_| bad())?;
        for n in names {
            let next = fd_path(&dir).join(n);
            dir = open_nofollow(&next, true).map_err(|_| bad())?;
        }
        Ok(dir)
    }

    /// Open a regular file for reading. Links and special files are refused.
    pub fn open(&self, rel: &str) -> Result<fs::File, Fail> {
        let bad = || Fail::new("media.path", "That is not a file on this disk.");
        let names = components(rel)?;
        let (last, dirs) = names.split_last().ok_or_else(bad)?;
        let dir = self.walk_dirs(dirs)?;
        let f = open_nofollow(&fd_path(&dir).join(last), false).map_err(|_| bad())?;
        let m = f.metadata().map_err(|_| bad())?;
        if !m.file_type().is_file() {
            return Err(bad());
        }
        Ok(f)
    }

    /// List a folder, at most [`MAX_ENTRIES`] entries from `offset` on, in
    /// the order the disk gives them: regular files and folders only; links,
    /// special files and names that are not valid UTF-8 are left out.
    pub fn walk(&self, rel: &str, offset: usize) -> Result<Listing, Fail> {
        let dir = self.walk_dirs(&components(rel)?)?;
        let mut out = Listing::default();
        let it = fs::read_dir(fd_path(&dir)).map_err(|e| {
            eprintln!("ALP-OS Live: media: {}: {e}", self.id);
            Fail::new("media.failed", "This folder cannot be read.")
        })?;
        let mut seen = 0usize;
        for e in it.flatten() {
            let Ok(t) = e.file_type() else { continue };
            if t.is_symlink()
                || t.is_block_device()
                || t.is_char_device()
                || t.is_fifo()
                || t.is_socket()
            {
                continue;
            }
            let Some(name) = e.file_name().to_str().map(str::to_string) else {
                out.skipped += 1;
                continue;
            };
            seen += 1;
            if seen <= offset {
                continue;
            }
            if out.entries.len() == MAX_ENTRIES {
                out.truncated = true;
                break;
            }
            let size = if t.is_file() {
                e.metadata().map(|m| m.len()).unwrap_or(0)
            } else {
                0
            };
            out.entries.push(Entry {
                name,
                dir: t.is_dir(),
                size_bytes: size,
            });
        }
        Ok(out)
    }
}

/// `/proc/self/fd/<n>` of an open folder: opening a name below it resolves
/// from that exact folder (the descriptor), not from a path that could
/// have changed.
fn fd_path(f: &fs::File) -> PathBuf {
    use std::os::fd::AsRawFd;
    PathBuf::from(format!("/proc/self/fd/{}", f.as_raw_fd()))
}

fn open_nofollow(p: &Path, dir: bool) -> std::io::Result<fs::File> {
    let mut flags = O_NOFOLLOW | O_NOCTTY | O_NONBLOCK;
    if dir {
        flags |= O_DIRECTORY;
    }
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(flags)
        .open(p)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn part(external: bool, disk: &str, fs: &str) -> Part {
        Part {
            external,
            name: format!("{disk}1"),
            disk: disk.into(),
            partuuid: "p".into(),
            type_uuid: String::new(),
            fs_type: fs.into(),
            fs_uuid: "u".into(),
            label: "L".into(),
            size_bytes: 1,
            read_only: false,
        }
    }

    #[test]
    fn only_external_allowed_file_systems() {
        for fs in ["vfat", "exfat", "ntfs", "ext4", "hfsplus", "iso9660"] {
            assert_eq!(
                refusal(&part(true, "sdb", fs), None, Some(ProbeVerdict::Clear)),
                None,
                "{fs}"
            );
        }
        for fs in ["btrfs", "xfs", "squashfs", "crypto_LUKS", "", "zfs_member"] {
            assert!(
                refusal(&part(true, "sdb", fs), None, Some(ProbeVerdict::Clear)).is_some(),
                "{fs}"
            );
        }
        assert!(
            refusal(
                &part(false, "nvme0n1", "ext4"),
                None,
                Some(ProbeVerdict::Clear)
            )
            .is_some()
        );
        assert_eq!(driver("ntfs"), Some("ntfs3"));
    }

    #[test]
    fn archive_disk_and_ambiguous_refused() {
        assert!(
            refusal(
                &part(true, "sdb", "exfat"),
                Some("sdb"),
                Some(ProbeVerdict::Clear)
            )
            .is_some()
        );
        assert_eq!(
            refusal(
                &part(true, "sdc", "exfat"),
                Some("sdb"),
                Some(ProbeVerdict::Clear)
            ),
            None
        );
        for v in [ProbeVerdict::Ambiguous, ProbeVerdict::Unchecked] {
            assert!(
                refusal(&part(true, "sdc", "exfat"), None, Some(v)).is_some(),
                "{v:?}"
            );
        }
    }

    #[test]
    fn probe_fails_closed() {
        assert_eq!(probe_verdict(Some(0)), ProbeVerdict::Clear);
        assert_eq!(probe_verdict(Some(8)), ProbeVerdict::Ambiguous);
        // 2 = nothing found, 124/137 = time-out, None = killed or not run.
        for c in [Some(1), Some(2), Some(4), Some(124), Some(137), None] {
            assert_eq!(probe_verdict(c), ProbeVerdict::Unchecked, "{c:?}");
        }
    }

    #[test]
    fn orphan_folders_are_found() {
        let a = "0123456789abcdef01234567".to_string();
        let b = "abcdefabcdefabcdefabcdef".to_string();
        let ids: BTreeMap<String, String> = [(a.clone(), "k".to_string())].into();
        let folders = vec![a, b.clone(), ".ids.toml".into(), ".ids.toml.tmp".into()];
        assert_eq!(orphans(&folders, &ids), [b]);
    }

    #[test]
    fn mount_device_is_read_from_mountinfo() {
        let mi = "36 25 8:17 / /run/alp/media/aaa ro,nosuid shared:1 - exfat /dev/sdb1 ro\n\
                  37 25 8:33 / /run/alp/media/bbb ro shared:2 - vfat /dev/sdc1 ro\n";
        assert_eq!(mount_dev(mi, "/run/alp/media/aaa").as_deref(), Some("8:17"));
        assert_eq!(mount_dev(mi, "/run/alp/media/bbb").as_deref(), Some("8:33"));
        assert_eq!(mount_dev(mi, "/run/alp/media/ccc"), None);
    }

    #[test]
    fn root_folder_must_be_roots_0700() {
        assert!(root_ok(true, 0, 0o40700));
        assert!(!root_ok(true, 1000, 0o40700));
        assert!(!root_ok(true, 0, 0o40755));
        assert!(!root_ok(false, 0, 0o120700));
    }

    #[test]
    fn mount_options_never_writable() {
        for fs in ["vfat", "exfat", "ntfs", "ext4", "hfsplus", "iso9660"] {
            let o = mount_options(fs);
            for need in ["ro", "nosuid", "nodev", "noexec", "nosymfollow"] {
                assert!(o.split(',').any(|x| x == need), "{fs}: {o}");
            }
            assert!(!o.split(',').any(|x| x == "rw"));
        }
        assert!(mount_options("ext4").contains("noload"));
    }

    #[test]
    fn paths_cannot_leave_the_root() {
        assert_eq!(components("a/b/c.jpg").unwrap(), ["a", "b", "c.jpg"]);
        assert_eq!(components("./a//b/").unwrap(), ["a", "b"]);
        assert_eq!(components("").unwrap(), Vec::<&str>::new());
        for bad in ["/etc/passwd", "../x", "a/../../x", "a/..", "a\0b"] {
            assert!(components(bad).is_err(), "{bad:?}");
        }
        assert!(components(&"x".repeat(256)).is_err());
        assert!(components(&"a/".repeat(2049)).is_err());
    }

    #[test]
    fn ids_are_opaque_hex() {
        let id = new_id().unwrap();
        assert!(valid_id(&id), "{id}");
        for bad in [
            "",
            "../x",
            "ABCDEFABCDEFABCDEFABCDEF",
            "abc",
            &"g".repeat(24),
        ] {
            assert!(!valid_id(bad), "{bad}");
        }
    }

    #[test]
    fn walk_and_open_never_follow_links() {
        let root = std::env::temp_dir().join(format!("alp-media-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("photos/2026")).unwrap();
        fs::write(root.join("photos/2026/a.jpg"), b"jpg").unwrap();
        fs::write(root.join("secret.txt"), b"outside").unwrap();
        let disk = root.join("disk");
        fs::create_dir_all(&disk).unwrap();
        fs::rename(root.join("photos"), disk.join("photos")).unwrap();
        std::os::unix::fs::symlink("../secret.txt", disk.join("link.txt")).unwrap();
        std::os::unix::fs::symlink("..", disk.join("up")).unwrap();
        std::os::unix::fs::symlink("photos", disk.join("photos-link")).unwrap();
        let m = MountedMedia {
            id: "t".into(),
            root: disk.clone(),
        };
        let mut s = String::new();
        m.open("photos/2026/a.jpg")
            .unwrap()
            .read_to_string(&mut s)
            .unwrap();
        assert_eq!(s, "jpg");
        assert!(m.open("link.txt").is_err(), "file link followed");
        assert!(m.open("up/secret.txt").is_err(), "folder link followed");
        assert!(
            m.open("photos-link/2026/a.jpg").is_err(),
            "folder link followed"
        );
        assert!(m.open("photos").is_err(), "a folder opened as a file");
        assert!(m.open("../secret.txt").is_err());
        let l = m.walk("", 0).unwrap();
        let names: Vec<String> = l.entries.into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["photos"], "links are left out of listings");
        assert!(!l.truncated);
        assert!(m.walk("up", 0).is_err());
        // Names that are not UTF-8 are counted, not shown.
        use std::os::unix::ffi::OsStrExt;
        fs::write(disk.join(std::ffi::OsStr::from_bytes(b"bad\xff.jpg")), b"x").unwrap();
        let l = m.walk("", 0).unwrap();
        assert_eq!((l.entries.len(), l.skipped), (1, 1));
        // Paging: more than MAX_ENTRIES files come in two pages.
        let many = disk.join("many");
        fs::create_dir(&many).unwrap();
        for i in 0..MAX_ENTRIES + 3 {
            fs::write(many.join(format!("{i}.jpg")), b"").unwrap();
        }
        let p1 = m.walk("many", 0).unwrap();
        assert_eq!((p1.entries.len(), p1.truncated), (MAX_ENTRIES, true));
        let p2 = m.walk("many", MAX_ENTRIES).unwrap();
        assert_eq!((p2.entries.len(), p2.truncated), (3, false));
        let _ = fs::remove_dir_all(&root);
    }
}
