// SPDX-License-Identifier: AGPL-3.0-or-later
//! The archive of the installed system: the encrypted Btrfs pool on the
//! `alp-data` partitions the user confirmed in the installer
//! (`docs/plans/0.2.8-first-start.md`). Live's archive on a backup disk
//! ([`crate::archive`]) stays as it is; this is a second kind beside it.
//!
//! First start: once the index is open (password chosen), [`create`] checks
//! the first-boot seed against the disks found now
//! (`alp_storage::seed_check`), makes the pool (`alp_storage::pool`) with a
//! random pool key kept in the index, writes the pool record and only then
//! deletes the seed. Every later start: [`open`]. The decrypted `@archive`
//! subvolume is bind-mounted at [`crate::archive::FILES_MNT`], so the rest
//! of the system sees the same place as on Live.

use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

use alp_hw::firstboot::FirstbootSeed;
use alp_hw::{HwError, inspect};
use alp_storage::pool::{self, Opened, System};
use alp_storage::seed_check;

use crate::archive::{ARCHIVE_UID, FILES_MNT, POOL_MNT, STATE_DIR};

/// The first-boot seed (`alp_core::state::paths().firstboot_seed`).
pub const SEED: &str = "/var/lib/alp/firstboot-seed.toml";
/// What the pool is made of; no secret (readable before unlocking).
pub const RECORD: &str = "/var/lib/alp/pool.toml";
/// Btrfs top level (all subvolumes), not shown to anyone.
pub const TOP_MNT: &str = "/run/alp/pool";
/// Pool key file inside the unlocked index.
pub const KEY_REL: &str = "keys/pool.key";

/// The pool as recorded at creation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// Btrfs filesystem UUID (`alp_hw::PoolId`).
    pub fs_uuid: String,
    /// Member partition UUIDs, lower case, in member order.
    pub members: Vec<String>,
    /// Data profile name (`dup`, `raid1`, ...).
    pub data_profile: String,
}

impl Record {
    pub fn to_toml(&self) -> String {
        let mut t = toml::Table::new();
        t.insert("format_version".into(), 1.into());
        t.insert("fs_uuid".into(), self.fs_uuid.clone().into());
        t.insert("data_profile".into(), self.data_profile.clone().into());
        t.insert(
            "members".into(),
            toml::Value::Array(self.members.iter().map(|m| m.clone().into()).collect()),
        );
        t.to_string()
    }

    pub fn from_toml(s: &str) -> Option<Self> {
        let t: toml::Table = s.parse().ok()?;
        if t.get("format_version")?.as_integer()? != 1 {
            return None;
        }
        let members: Vec<String> = t
            .get("members")?
            .as_array()?
            .iter()
            .map(|v| v.as_str().map(str::to_string))
            .collect::<Option<_>>()?;
        if members.is_empty() {
            return None;
        }
        Some(Self {
            fs_uuid: t.get("fs_uuid")?.as_str()?.to_string(),
            data_profile: t.get("data_profile")?.as_str()?.to_string(),
            members,
        })
    }

    /// How many member disks may be missing with the data still whole:
    /// one for the mirrored profiles, none otherwise.
    pub fn can_lose(&self) -> usize {
        match self.data_profile.as_str() {
            "raid1" | "raid10" => 1,
            "raid1c3" => 2,
            _ => 0,
        }
    }
}

/// The installer left a seed: the first start still has to make the pool.
pub fn seed_waiting() -> bool {
    Path::new(SEED).is_file() && record().is_none()
}

pub fn record() -> Option<Record> {
    Record::from_toml(&fs::read_to_string(RECORD).ok()?)
}

fn write_atomic(path: &Path, body: &[u8], mode: u32) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(mode)
        .open(&tmp)
        .map_err(|e| e.to_string())?;
    f.write_all(body)
        .and_then(|()| f.sync_all())
        .map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        fs::File::open(dir)
            .and_then(|d| d.sync_all())
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// The pool key from the unlocked index; made on the first call.
fn pool_key() -> Result<alp_crypto::keys::PoolKey, String> {
    if !crate::index::is_open() {
        return Err("the archive is locked; type your password first".into());
    }
    let path = crate::index::path(KEY_REL);
    if !path.is_file() {
        let key = alp_crypto::keys::PoolKey::generate().map_err(|e| e.to_string())?;
        let mut bytes = zeroize::Zeroizing::new(Vec::with_capacity(64));
        key.write_key_file(&mut *bytes).map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        write_atomic(&path, &bytes, 0o600)?;
    }
    let mut f = fs::File::open(&path).map_err(|e| e.to_string())?;
    alp_crypto::keys::PoolKey::read_from(&mut f).map_err(|e| e.to_string())
}

fn by_id_disk(by_id: &str) -> Option<String> {
    let target = fs::canonicalize(Path::new("/dev/disk/by-id").join(by_id)).ok()?;
    Some(target.file_name()?.to_string_lossy().into_owned())
}

fn by_partuuid(uuid: &str) -> Option<String> {
    let target = fs::canonicalize(Path::new("/dev/disk/by-partuuid").join(uuid)).ok()?;
    Some(target.to_string_lossy().into_owned())
}

fn run(cmd: &str, args: &[&str]) -> Result<(), String> {
    let out = std::process::Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("{cmd}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{cmd}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

fn is_mounted(at: &str) -> bool {
    run("mountpoint", &["-q", at]).is_ok()
}

/// `@archive` at [`FILES_MNT`] (owned by the archive user, as on Live).
fn show_archive(chown: bool) -> Result<(), String> {
    if is_mounted(FILES_MNT) {
        return Ok(());
    }
    let src = format!("{TOP_MNT}/@archive");
    if chown {
        std::os::unix::fs::chown(&src, Some(ARCHIVE_UID), Some(ARCHIVE_UID))
            .map_err(|e| e.to_string())?;
        fs::set_permissions(&src, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    fs::create_dir_all(POOL_MNT).map_err(|e| e.to_string())?;
    fs::create_dir_all(FILES_MNT).map_err(|e| e.to_string())?;
    run("mount", &["--bind", &src, FILES_MNT])
}

/// `@immich` at Immich's data folder in the index (database, thumbnails):
/// on tens of TB the thumbnails alone outgrow the 1 GiB index. Immich
/// starts only once the index and the archive are open, so the folder is
/// bound before it is first used.
fn show_immich() -> Result<(), String> {
    let at = crate::immich::DATA;
    if is_mounted(at) {
        return Ok(());
    }
    fs::create_dir_all(at).map_err(|e| e.to_string())?;
    run("mount", &["--bind", &format!("{TOP_MNT}/@immich"), at])
}

fn fail(detail: String) -> HwError {
    HwError {
        code: pool::CODE_CREATE,
        detail,
    }
}

/// First start: make the pool from the seed. `say` gets progress text.
/// The error carries its catalog code: `ALP-INST-005` when the disks are
/// not the ones confirmed, `ALP-STO-008` for anything else.
pub fn create(say: &mut dyn FnMut(&str)) -> Result<(), HwError> {
    if record().is_some() {
        return open().map(drop).map_err(fail);
    }
    let text = fs::read_to_string(SEED).map_err(|e| fail(format!("no first-boot seed: {e}")))?;
    let seed = FirstbootSeed::from_toml(&text).map_err(fail)?;
    say("Checking your disks…");
    let _ = run("udevadm", &["settle", "--timeout=30"]);
    let disks = inspect::inspect(&inspect::Roots::system()).disks;
    // No ledger on this system yet: the seed's digest is the evidence the
    // user confirmed (contract firstboot-seed.md, rule 3); seq 0.
    let checked = seed_check::check(&seed, &disks, &by_id_disk, 0)?;
    let key = pool_key().map_err(fail)?;
    say("Locking your disks with a new key…");
    let devs: Vec<String> = checked
        .found
        .iter()
        .map(|f| format!("/dev/{}", f.partition))
        .collect();
    let id = pool::create(checked.plan.get(), &devs, &key, TOP_MNT, &mut System)?;
    say("Making your archive ready…");
    show_archive(true).map_err(fail)?;
    show_immich().map_err(fail)?;
    let rec = Record {
        fs_uuid: id.0,
        members: seed
            .members
            .iter()
            .map(|m| m.partition_uuid.to_ascii_lowercase())
            .collect(),
        data_profile: seed.data_profile.name().into(),
    };
    fs::create_dir_all(STATE_DIR).map_err(|e| fail(e.to_string()))?;
    write_atomic(Path::new(RECORD), rec.to_toml().as_bytes(), 0o644).map_err(fail)?;
    // Contract rule 4: the seed goes only once the pool exists.
    fs::remove_file(SEED).map_err(|e| fail(e.to_string()))?;
    Ok(())
}

/// Every start: open the pool (read-only if a mirrored disk is missing).
pub fn open() -> Result<Opened, String> {
    let rec = record().ok_or("no pool set up yet")?;
    let key = pool_key()?;
    let members: Vec<(String, bool)> = rec
        .members
        .iter()
        .map(|u| match by_partuuid(u) {
            Some(dev) => (dev, true),
            None => (format!("/dev/disk/by-partuuid/{u}"), false),
        })
        .collect();
    let opened = pool::open(&members, rec.can_lose(), &key, TOP_MNT, &mut System)
        .map_err(|e| e.to_string())?;
    show_archive(false)?;
    show_immich()?;
    Ok(opened)
}

/// Lock: the archive view first, then the pool.
pub fn close() -> Result<(), String> {
    let Some(rec) = record() else {
        return Ok(());
    };
    for at in [crate::immich::DATA, FILES_MNT] {
        if is_mounted(at) {
            run("umount", &[at])?;
        }
    }
    pool::close(rec.members.len(), TOP_MNT, &mut System).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_round_trip_and_tolerance() {
        let r = Record {
            fs_uuid: "f00d".into(),
            members: vec!["u1".into(), "u2".into()],
            data_profile: "raid1".into(),
        };
        assert_eq!(Record::from_toml(&r.to_toml()), Some(r.clone()));
        assert_eq!(r.can_lose(), 1);
        let one = Record {
            data_profile: "dup".into(),
            members: vec!["u1".into()],
            ..r.clone()
        };
        assert_eq!(one.can_lose(), 0);
        assert_eq!(
            Record::from_toml(
                &r.to_toml()
                    .replace("format_version = 1", "format_version = 2")
            ),
            None
        );
        assert_eq!(
            Record::from_toml(
                "format_version = 1\nfs_uuid = \"x\"\ndata_profile = \"dup\"\nmembers = []"
            ),
            None
        );
    }
}
