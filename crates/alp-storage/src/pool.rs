// SPDX-License-Identifier: AGPL-3.0-or-later
//! The data pool: one LUKS2 container per member, Btrfs across them
//! (spec block 4; plan `docs/plans/0.2.8-first-start.md`).
//!
//! `create` is resumable: every step first looks whether it already
//! happened (LUKS header with our label, open mapping, Btrfs with our
//! label, subvolume), so a restart in the middle continues instead of
//! starting over, and nothing that exists is ever formatted again.
//! `open` never mounts a pool with a missing member read-write.
//!
//! The pool key reaches `cryptsetup` only on stdin (`--key-file=-`), from a
//! buffer that is wiped after use; never in argv, a file or a log.

use alp_crypto::keys::PoolKey;
use alp_hw::{HwError, PoolId, PoolPlan, Result};
use zeroize::Zeroizing;

/// Mapper names: `/dev/mapper/alp-pool-<n>`, n = member index.
pub const MAPPER_PREFIX: &str = "alp-pool-";
/// LUKS2 label and Btrfs label of every pool.
pub const LABEL: &str = "alp-pool";
/// Subvolumes made at creation (`@immich`: Immich's database and
/// thumbnails, which outgrow any fixed partition on tens of TB).
pub const SUBVOLUMES: [&str; 3] = ["@archive", "@immich", "@snapshots"];

pub const CODE_CREATE: &str = "ALP-STO-008";
pub const CODE_OPEN: &str = "ALP-STO-005";

/// What a command returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Out {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Runs one program. The real one is [`System`]; tests use a mock.
pub trait Run {
    fn run(&mut self, program: &str, args: &[String], stdin: Option<&[u8]>) -> Out;
}

fn owned(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| a.to_string()).collect()
}

fn err(code: &'static str, what: &str, out: &Out) -> HwError {
    HwError {
        code,
        detail: format!("{what}: {}", out.stderr.trim()),
    }
}

fn key_bytes(key: &PoolKey) -> Zeroizing<Vec<u8>> {
    let mut buf = Zeroizing::new(Vec::with_capacity(64));
    // Writing to a Vec cannot fail.
    let _ = key.write_key_file(&mut *buf);
    buf
}

fn mapper(i: usize) -> String {
    format!("{MAPPER_PREFIX}{i}")
}

/// One value of `blkid -p` (probes the device itself, not the cache).
fn probe(r: &mut dyn Run, dev: &str, tag: &str) -> Option<String> {
    let out = r.run(
        "blkid",
        &owned(&["-p", "-o", "value", "-s", tag, dev]),
        None,
    );
    let v = out.stdout.trim();
    (out.ok && !v.is_empty()).then(|| v.to_string())
}

/// Like [`probe`], but only "nothing found" counts as empty: `blkid -p`
/// says so with exit 2 and no output. Any other failure (two signatures,
/// exit 8; an I/O error on a failing disk) is an error, never "empty", so
/// it can never lead to formatting.
fn probe_sure(r: &mut dyn Run, dev: &str, tag: &str, code: &'static str) -> Result<Option<String>> {
    let out = r.run(
        "blkid",
        &owned(&["-p", "-o", "value", "-s", tag, dev]),
        None,
    );
    let v = out.stdout.trim();
    if out.ok && !v.is_empty() {
        return Ok(Some(v.to_string()));
    }
    // On a partition, blkid -p succeeds with only PART_ENTRY_* values when
    // it holds nothing: the TYPE filter then prints nothing (exit 0). With
    // nothing found at all it exits 2, silently.
    if v.is_empty() && (out.ok || out.stderr.trim().is_empty()) {
        return Ok(None);
    }
    Err(err(code, &format!("blkid {dev}"), &out))
}

/// Make member `dev` a LUKS2 container (unless it already is ours) and
/// open it as `name`.
fn unlock(
    r: &mut dyn Run,
    dev: &str,
    name: &str,
    key: &[u8],
    may_format: bool,
    code: &'static str,
) -> Result<()> {
    if r.run(
        "test",
        &owned(&["-e", &format!("/dev/mapper/{name}")]),
        None,
    )
    .ok
    {
        return Ok(());
    }
    let kind = probe_sure(r, dev, "TYPE", code)?;
    let is_ours =
        kind.as_deref() == Some("crypto_LUKS") && probe(r, dev, "LABEL").as_deref() == Some(LABEL);
    // A power cut during luksFormat leaves our header (with the label)
    // before the key slot exists: no key opens it. The partition is still
    // empty then, so it is formatted again (only on the first start).
    let unfinished = is_ours && may_format && keyslots(r, dev)?.is_empty();
    if !is_ours || unfinished {
        if !may_format {
            return Err(HwError {
                code,
                detail: format!("{dev} is not an ALP-OS pool member"),
            });
        }
        if !unfinished && kind.is_some() {
            // Never format over something: the installer made this
            // partition empty, so anything here was not put there by us.
            return Err(HwError {
                code,
                detail: format!("{dev} holds data that is not ours; not formatted"),
            });
        }
        // cryptsetup defaults (LUKS2, argon2id, aes-xts-plain64) as on Live.
        let out = r.run(
            "cryptsetup",
            &owned(&[
                "luksFormat",
                "--type",
                "luks2",
                "--batch-mode",
                "--label",
                LABEL,
                "--key-file=-",
                dev,
            ]),
            Some(key),
        );
        if !out.ok {
            return Err(err(code, &format!("luksFormat {dev}"), &out));
        }
    }
    let out = r.run(
        "cryptsetup",
        &owned(&["open", "--type", "luks2", "--key-file=-", dev, name]),
        Some(key),
    );
    if !out.ok {
        return Err(err(code, &format!("open {dev}"), &out));
    }
    Ok(())
}

/// Create the pool of a checked first-boot plan on the member partitions
/// `devs` (`/dev/<name>`, the plan's order) and mount it at `mnt`.
/// Returns the Btrfs filesystem UUID.
pub fn create(
    plan: &PoolPlan,
    devs: &[String],
    key: &PoolKey,
    mnt: &str,
    r: &mut dyn Run,
) -> Result<PoolId> {
    if devs.is_empty() || devs.len() != plan.members.len() {
        return Err(HwError {
            code: CODE_CREATE,
            detail: "members and devices differ".into(),
        });
    }
    let key = key_bytes(key);
    for (i, dev) in devs.iter().enumerate() {
        unlock(r, dev, &mapper(i), &key, true, CODE_CREATE)?;
    }
    let maps: Vec<String> = (0..devs.len())
        .map(|i| format!("/dev/mapper/{}", mapper(i)))
        .collect();
    let has_fs = probe(r, &maps[0], "TYPE").as_deref() == Some("btrfs")
        && probe(r, &maps[0], "LABEL").as_deref() == Some(LABEL);
    if !has_fs {
        // Fresh LUKS containers read as noise; without -f, mkfs.btrfs
        // still refuses anything it recognises.
        let mut args = owned(&[
            "-L",
            LABEL,
            "-d",
            plan.data_profile.name(),
            "-m",
            plan.metadata_profile.name(),
        ]);
        args.extend(maps.iter().cloned());
        let out = r.run("mkfs.btrfs", &args, None);
        if !out.ok {
            return Err(err(CODE_CREATE, "mkfs.btrfs", &out));
        }
    }
    mount(r, &maps, mnt, false, CODE_CREATE)?;
    for sv in SUBVOLUMES {
        let path = format!("{mnt}/{sv}");
        if !r.run("test", &owned(&["-d", &path]), None).ok {
            let out = r.run("btrfs", &owned(&["subvolume", "create", &path]), None);
            if !out.ok {
                return Err(err(CODE_CREATE, &format!("subvolume {sv}"), &out));
            }
        }
    }
    probe(r, &maps[0], "UUID")
        .map(PoolId)
        .ok_or_else(|| HwError {
            code: CODE_CREATE,
            detail: "no filesystem UUID".into(),
        })
}

fn mount(
    r: &mut dyn Run,
    maps: &[String],
    mnt: &str,
    degraded: bool,
    code: &'static str,
) -> Result<()> {
    if r.run("mountpoint", &owned(&["-q", mnt]), None).ok {
        return Ok(());
    }
    let out = r.run("btrfs", &owned(&["device", "scan"]), None);
    if !out.ok {
        return Err(err(code, "btrfs device scan", &out));
    }
    let out = r.run("mkdir", &owned(&["-p", mnt]), None);
    if !out.ok {
        return Err(err(code, &format!("mkdir {mnt}"), &out));
    }
    let opts = if degraded {
        "noatime,degraded,ro"
    } else {
        "noatime"
    };
    let out = r.run(
        "mount",
        &owned(&["-t", "btrfs", "-o", opts, &maps[0], mnt]),
        None,
    );
    if !out.ok {
        return Err(err(code, "mount", &out));
    }
    Ok(())
}

/// How an opened pool is mounted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opened {
    /// Every member is there; read-write.
    Full,
    /// Members missing (their device paths): read-only, `degraded`. Never
    /// read-write by itself: writing to a degraded RAID1 can leave chunks
    /// with one copy, and the user must decide (replace or reconnect).
    Degraded { missing: Vec<String> },
}

/// Open an existing pool: `members` are the member partitions as
/// `(device, present)`, `profile_copies` how many disks may be lost
/// (1 for RAID1, 0 for one disk). Never formats anything.
pub fn open(
    members: &[(String, bool)],
    can_lose: usize,
    key: &PoolKey,
    mnt: &str,
    r: &mut dyn Run,
) -> Result<Opened> {
    let missing: Vec<String> = members
        .iter()
        .filter(|(_, present)| !present)
        .map(|(d, _)| d.clone())
        .collect();
    if members.is_empty() || missing.len() > can_lose || missing.len() == members.len() {
        return Err(HwError {
            code: CODE_OPEN,
            detail: format!("{} of {} disks missing", missing.len(), members.len()),
        });
    }
    let key = key_bytes(key);
    let mut maps = Vec::new();
    for (i, (dev, present)) in members.iter().enumerate() {
        if *present {
            unlock(r, dev, &mapper(i), &key, false, CODE_OPEN)?;
            maps.push(format!("/dev/mapper/{}", mapper(i)));
        }
    }
    mount(r, &maps, mnt, !missing.is_empty(), CODE_OPEN)?;
    Ok(if missing.is_empty() {
        Opened::Full
    } else {
        Opened::Degraded { missing }
    })
}

/// Close the pool mounted at `mnt` with `members` containers: unmount,
/// then close every mapping that exists. Stops at the first failure, so
/// a container is never closed under a mounted filesystem.
pub fn close(members: usize, mnt: &str, r: &mut dyn Run) -> Result<()> {
    if r.run("mountpoint", &owned(&["-q", mnt]), None).ok {
        let out = r.run("umount", &owned(&[mnt]), None);
        if !out.ok {
            return Err(err(CODE_OPEN, "umount", &out));
        }
    }
    for i in 0..members {
        let name = mapper(i);
        if r.run(
            "test",
            &owned(&["-e", &format!("/dev/mapper/{name}")]),
            None,
        )
        .ok
        {
            let out = r.run("cryptsetup", &owned(&["close", &name]), None);
            if !out.ok {
                return Err(err(CODE_OPEN, &format!("close {name}"), &out));
            }
        }
    }
    Ok(())
}

/// LUKS2 key slots that hold a recovery code (ADR 0013 on the pool): two,
/// so a new code is added beside the current one and the old one is
/// removed only after the user typed words back (T42). Slot 0 is the pool
/// key (luksFormat); these functions refuse any other slot.
pub const RECOVERY_SLOTS: [u8; 2] = [1, 2];

fn recovery_slot(slot: u8) -> Result<String> {
    if RECOVERY_SLOTS.contains(&slot) {
        Ok(slot.to_string())
    } else {
        Err(HwError {
            code: CODE_OPEN,
            detail: format!("key slot {slot} is not a recovery slot"),
        })
    }
}

/// The LUKS2 key slots in use on `dev` (`cryptsetup luksDump`, section
/// "Keyslots").
pub fn keyslots(r: &mut dyn Run, dev: &str) -> Result<Vec<u8>> {
    let out = r.run("cryptsetup", &owned(&["luksDump", dev]), None);
    if !out.ok {
        return Err(err(CODE_OPEN, &format!("luksDump {dev}"), &out));
    }
    Ok(parse_keyslots(&out.stdout))
}

fn parse_keyslots(dump: &str) -> Vec<u8> {
    let mut inside = false;
    let mut slots = Vec::new();
    for line in dump.lines() {
        if !line.starts_with(char::is_whitespace) {
            inside = line.trim_end() == "Keyslots:";
            continue;
        }
        // "  1: luks2" opens a slot; its details are indented further.
        if inside
            && let Some((n, kind)) = line.trim().split_once(':')
            && kind.trim() == "luks2"
            && let Ok(n) = n.parse::<u8>()
        {
            slots.push(n);
        }
    }
    slots
}

/// Add `secret` (the recovery code's unlock secret, one line) to member
/// `dev` in recovery `slot`, unlocking with the pool key. Both reach
/// cryptsetup on stdin: the 64-byte key first (`--keyfile-size=64`),
/// then the secret up to its newline; never argv or a file.
pub fn add_recovery(
    r: &mut dyn Run,
    dev: &str,
    key: &PoolKey,
    secret: &str,
    slot: u8,
) -> Result<()> {
    let n = recovery_slot(slot)?;
    if secret.is_empty() || secret.contains('\n') {
        return Err(HwError {
            code: CODE_OPEN,
            detail: "the recovery secret must be one non-empty line".into(),
        });
    }
    let mut input = key_bytes(key);
    if input.len() != 64 {
        return Err(HwError {
            code: CODE_OPEN,
            detail: "the pool key file is not 64 bytes".into(),
        });
    }
    input.extend_from_slice(secret.as_bytes());
    input.push(b'\n');
    let out = r.run(
        "cryptsetup",
        &owned(&[
            "luksAddKey",
            "--batch-mode",
            "--force-password",
            "--key-file=-",
            "--keyfile-size=64",
            &format!("--new-key-slot={n}"),
            dev,
        ]),
        Some(&input),
    );
    if !out.ok {
        return Err(err(CODE_OPEN, &format!("luksAddKey {dev}"), &out));
    }
    Ok(())
}

/// Remove recovery `slot` from `dev` if it is in use (batch mode: no
/// passphrase; only slots 1 and 2 can be named here). With stdin empty
/// cryptsetup 2.7 skips the passphrase check in batch mode
/// (action_luksKillSlot ignores "Nothing to read on input").
pub fn remove_recovery(r: &mut dyn Run, dev: &str, slot: u8) -> Result<()> {
    let n = recovery_slot(slot)?;
    if !keyslots(r, dev)?.contains(&slot) {
        return Ok(());
    }
    let out = r.run(
        "cryptsetup",
        &owned(&["luksKillSlot", "--batch-mode", dev, &n]),
        None,
    );
    if !out.ok {
        return Err(err(CODE_OPEN, &format!("luksKillSlot {dev} {n}"), &out));
    }
    Ok(())
}

/// Does `secret` open recovery `slot` of `dev`? Nothing is opened
/// (`--test-passphrase`); the secret goes on stdin up to its newline.
pub fn test_recovery(r: &mut dyn Run, dev: &str, secret: &str, slot: u8) -> Result<bool> {
    let n = recovery_slot(slot)?;
    let mut input = Zeroizing::new(secret.as_bytes().to_vec());
    input.push(b'\n');
    let out = r.run(
        "cryptsetup",
        &owned(&["open", "--test-passphrase", &format!("--key-slot={n}"), dev]),
        Some(&input),
    );
    // Wrong words: cryptsetup says "No key available with this
    // passphrase" (tools_passphrase_msg, -EPERM). Anything else (out of
    // memory for Argon2, an I/O error) is a failure, not "wrong words".
    if out.ok {
        Ok(true)
    } else if out.stderr.contains("No key available") {
        Ok(false)
    } else {
        Err(err(CODE_OPEN, &format!("test {dev}"), &out))
    }
}

/// The real runner: the program with these arguments, stdin piped when
/// given (and then closed), output captured.
#[derive(Debug, Default)]
pub struct System;

impl Run for System {
    fn run(&mut self, program: &str, args: &[String], stdin: Option<&[u8]>) -> Out {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let child = Command::new(program)
            .args(args)
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                return Out {
                    ok: false,
                    stdout: String::new(),
                    stderr: format!("{program}: {e}"),
                };
            }
        };
        if let (Some(bytes), Some(mut pipe)) = (stdin, child.stdin.take()) {
            let _ = pipe.write_all(bytes);
        }
        match child.wait_with_output() {
            Ok(o) => Out {
                ok: o.status.success(),
                stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
            },
            Err(e) => Out {
                ok: false,
                stdout: String::new(),
                stderr: format!("{program}: {e}"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alp_hw::{DiskId, PoolMember, Protection, RaidProfile};
    use std::collections::{HashMap, HashSet};

    /// A tiny model of the machine: what `blkid -p` reports per device,
    /// which mappings and directories exist, what is mounted.
    #[derive(Default)]
    struct Mock {
        probes: HashMap<(String, String), String>,
        exists: HashSet<String>,
        mounted: HashSet<String>,
        calls: Vec<(String, Vec<String>, bool)>,
        fail: Option<&'static str>,
        key_seen: Vec<Vec<u8>>,
        /// LUKS headers without a key slot (power cut during luksFormat).
        no_slots: HashSet<String>,
    }

    impl Mock {
        fn set(&mut self, dev: &str, tag: &str, v: &str) {
            self.probes.insert((dev.into(), tag.into()), v.into());
        }
        fn ran(&self, program: &str) -> Vec<&Vec<String>> {
            self.calls
                .iter()
                .filter(|c| c.0 == program)
                .map(|c| &c.1)
                .collect()
        }
    }

    fn ok(stdout: &str) -> Out {
        Out {
            ok: true,
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }
    fn no() -> Out {
        Out {
            ok: false,
            stdout: String::new(),
            stderr: "failed".into(),
        }
    }

    impl Run for Mock {
        fn run(&mut self, program: &str, args: &[String], stdin: Option<&[u8]>) -> Out {
            self.calls
                .push((program.into(), args.to_vec(), stdin.is_some()));
            if let Some(b) = stdin {
                self.key_seen.push(b.to_vec());
            }
            if self.fail == Some(program) {
                return no();
            }
            let a = |i: usize| args[i].as_str();
            match program {
                // Nothing found: exit 2 without output (blkid -p).
                "blkid" => match self.probes.get(&(a(5).into(), a(4).into())) {
                    Some(v) => ok(v),
                    None => Out {
                        ok: false,
                        stdout: String::new(),
                        stderr: String::new(),
                    },
                },
                "cryptsetup" if a(0) == "luksDump" => {
                    if self.no_slots.contains(a(1)) {
                        ok("Keyslots:\nTokens:\n")
                    } else {
                        ok("Keyslots:\n  0: luks2\n")
                    }
                }
                "test" => {
                    if self.exists.contains(a(1)) {
                        ok("")
                    } else {
                        no()
                    }
                }
                "mountpoint" => {
                    if self.mounted.contains(a(1)) {
                        ok("")
                    } else {
                        no()
                    }
                }
                "cryptsetup" if a(0) == "luksFormat" => {
                    let dev = args.last().unwrap().clone();
                    self.no_slots.remove(&dev);
                    self.set(&dev, "TYPE", "crypto_LUKS");
                    self.set(&dev, "LABEL", LABEL);
                    ok("")
                }
                "cryptsetup" if a(0) == "open" => {
                    let name = args.last().unwrap();
                    self.exists.insert(format!("/dev/mapper/{name}"));
                    ok("")
                }
                "mkfs.btrfs" => {
                    let first = args[6].clone();
                    self.set(&first, "TYPE", "btrfs");
                    self.set(&first, "LABEL", LABEL);
                    self.set(&first, "UUID", "f00d-uuid");
                    ok("")
                }
                "mount" => {
                    self.mounted.insert(args.last().unwrap().clone());
                    ok("")
                }
                "btrfs" if a(0) == "subvolume" => {
                    self.exists.insert(a(2).into());
                    ok("")
                }
                _ => ok(""),
            }
        }
    }

    fn plan(n: usize) -> PoolPlan {
        let profile = if n == 1 {
            RaidProfile::Dup
        } else {
            RaidProfile::Raid1
        };
        PoolPlan {
            members: (0..n)
                .map(|i| PoolMember::ExistingPartition {
                    disk: DiskId(format!("d{i}")),
                    partition_uuid: format!("u{i}"),
                })
                .collect(),
            data_profile: profile,
            metadata_profile: profile,
            usable_bytes: 1,
            protection: Protection::DiskFailure,
        }
    }

    fn key() -> PoolKey {
        let mut b: &[u8] = &[5u8; 64];
        PoolKey::read_from(&mut b).unwrap()
    }

    fn devs(n: usize) -> Vec<String> {
        (0..n)
            .map(|i| format!("/dev/vd{}1", (b'b' + i as u8) as char))
            .collect()
    }

    const MNT: &str = "/var/lib/alp-pool";

    #[test]
    fn two_disks_make_a_raid1_pool() {
        let mut m = Mock::default();
        let id = create(&plan(2), &devs(2), &key(), MNT, &mut m).unwrap();
        assert_eq!(id, PoolId("f00d-uuid".into()));
        let formats = m
            .ran("cryptsetup")
            .into_iter()
            .filter(|a| a[0] == "luksFormat")
            .count();
        assert_eq!(formats, 2);
        let mkfs = m.ran("mkfs.btrfs");
        assert_eq!(
            mkfs[0],
            &owned(&[
                "-L",
                "alp-pool",
                "-d",
                "raid1",
                "-m",
                "raid1",
                "/dev/mapper/alp-pool-0",
                "/dev/mapper/alp-pool-1"
            ])
        );
        let subs: Vec<_> = m
            .ran("btrfs")
            .into_iter()
            .filter(|a| a[0] == "subvolume")
            .map(|a| a[2].clone())
            .collect();
        assert_eq!(
            subs,
            ["@archive", "@immich", "@snapshots"].map(|s| format!("{MNT}/{s}"))
        );
    }

    #[test]
    fn one_disk_is_dup() {
        let mut m = Mock::default();
        create(&plan(1), &devs(1), &key(), MNT, &mut m).unwrap();
        let mkfs = m.ran("mkfs.btrfs");
        assert_eq!(&mkfs[0][2..6], &owned(&["-d", "dup", "-m", "dup"])[..]);
    }

    #[test]
    fn the_key_goes_only_to_stdin() {
        let mut m = Mock::default();
        create(&plan(2), &devs(2), &key(), MNT, &mut m).unwrap();
        for (prog, args, stdin) in &m.calls {
            assert!(!args.iter().any(|a| a.contains('\u{5}')), "{prog} argv");
            if prog == "cryptsetup" {
                assert!(*stdin && args.contains(&"--key-file=-".to_string()));
            }
        }
        assert!(m.key_seen.iter().all(|k| k == &[5u8; 64]));
    }

    #[test]
    fn a_second_run_resumes_without_formatting_again() {
        let mut m = Mock {
            fail: Some("mount"),
            ..Mock::default()
        };
        assert_eq!(
            create(&plan(2), &devs(2), &key(), MNT, &mut m)
                .unwrap_err()
                .code,
            CODE_CREATE
        );
        m.fail = None;
        m.calls.clear();
        // The mappings were closed by the restart; the headers stay.
        m.exists.clear();
        create(&plan(2), &devs(2), &key(), MNT, &mut m).unwrap();
        assert!(
            m.ran("cryptsetup")
                .iter()
                .all(|a| a[0] == "open" || a[0] == "luksDump")
        );
        assert!(m.ran("mkfs.btrfs").is_empty());
    }

    #[test]
    fn a_header_without_a_key_slot_is_formatted_again() {
        // Power cut between the LUKS2 header and its first key slot.
        let mut m = Mock::default();
        for d in devs(2) {
            m.set(&d, "TYPE", "crypto_LUKS");
            m.set(&d, "LABEL", LABEL);
        }
        m.no_slots.insert("/dev/vdb1".into());
        create(&plan(2), &devs(2), &key(), MNT, &mut m).unwrap();
        let formatted: Vec<_> = m
            .ran("cryptsetup")
            .into_iter()
            .filter(|a| a[0] == "luksFormat")
            .map(|a| a.last().unwrap().clone())
            .collect();
        assert_eq!(formatted, ["/dev/vdb1"]);
    }

    #[test]
    fn a_failed_probe_never_formats() {
        // blkid -p exit 8 (two signatures) or an I/O error: not "empty".
        struct Bad(Mock);
        impl Run for Bad {
            fn run(&mut self, p: &str, a: &[String], i: Option<&[u8]>) -> Out {
                if p == "blkid" && a.last().map(String::as_str) == Some("/dev/vdc1") {
                    return Out {
                        ok: false,
                        stdout: String::new(),
                        stderr: "ambivalent result".into(),
                    };
                }
                self.0.run(p, a, i)
            }
        }
        let mut b = Bad(Mock::default());
        let e = create(&plan(2), &devs(2), &key(), MNT, &mut b).unwrap_err();
        assert_eq!(e.code, CODE_CREATE);
        assert!(
            !b.0.ran("cryptsetup")
                .iter()
                .any(|a| a[0] == "luksFormat" && a.last().unwrap() == "/dev/vdc1")
        );
    }

    #[test]
    fn an_empty_partition_is_formatted() {
        // blkid -p on an empty GPT partition: exit 0, only PART_ENTRY_*
        // values, so the TYPE filter prints nothing.
        struct PartOnly(Mock);
        impl Run for PartOnly {
            fn run(&mut self, p: &str, a: &[String], i: Option<&[u8]>) -> Out {
                if p == "blkid" && a.contains(&"TYPE".to_string()) {
                    let _ = self.0.run(p, a, i);
                    return Out {
                        ok: true,
                        stdout: String::new(),
                        stderr: String::new(),
                    };
                }
                self.0.run(p, a, i)
            }
        }
        let mut b = PartOnly(Mock::default());
        create(&plan(2), &devs(2), &key(), MNT, &mut b).unwrap();
        let formatted =
            b.0.ran("cryptsetup")
                .iter()
                .filter(|a| a[0] == "luksFormat")
                .count();
        assert_eq!(formatted, 2);
    }

    #[test]
    fn something_foreign_is_never_formatted() {
        let mut m = Mock::default();
        m.set("/dev/vdc1", "TYPE", "ntfs");
        let e = create(&plan(2), &devs(2), &key(), MNT, &mut m).unwrap_err();
        assert_eq!(e.code, CODE_CREATE);
        assert!(m.ran("mkfs.btrfs").is_empty());
        assert!(
            !m.ran("cryptsetup")
                .iter()
                .any(|a| a[0] == "luksFormat" && a.last().unwrap() == "/dev/vdc1")
        );
    }

    #[test]
    fn open_full_and_degraded() {
        let mut m = Mock::default();
        create(&plan(2), &devs(2), &key(), MNT, &mut m).unwrap();
        let d = devs(2);
        let mut m2 = Mock {
            probes: m.probes.clone(),
            ..Mock::default()
        };
        let all = vec![(d[0].clone(), true), (d[1].clone(), true)];
        assert_eq!(open(&all, 1, &key(), MNT, &mut m2).unwrap(), Opened::Full);
        assert!(m2.ran("mount")[0].contains(&"noatime".to_string()));

        let mut m3 = Mock {
            probes: m.probes.clone(),
            ..Mock::default()
        };
        let one = vec![(d[0].clone(), true), (d[1].clone(), false)];
        assert_eq!(
            open(&one, 1, &key(), MNT, &mut m3).unwrap(),
            Opened::Degraded {
                missing: vec![d[1].clone()]
            }
        );
        assert!(m3.ran("mount")[0].contains(&"noatime,degraded,ro".to_string()));
        assert!(m3.ran("cryptsetup").iter().all(|a| a[0] == "open"));
    }

    #[test]
    fn close_unmounts_before_closing() {
        let mut m = Mock::default();
        create(&plan(2), &devs(2), &key(), MNT, &mut m).unwrap();
        m.calls.clear();
        close(2, MNT, &mut m).unwrap();
        let order: Vec<_> = m
            .calls
            .iter()
            .filter(|c| c.0 == "umount" || c.0 == "cryptsetup")
            .map(|c| format!("{} {}", c.0, c.1.join(" ")))
            .collect();
        assert_eq!(
            order,
            [
                format!("umount {MNT}"),
                "cryptsetup close alp-pool-0".into(),
                "cryptsetup close alp-pool-1".into()
            ]
        );
        let mut m = Mock {
            fail: Some("umount"),
            ..Mock::default()
        };
        m.mounted.insert(MNT.into());
        m.exists.insert("/dev/mapper/alp-pool-0".into());
        assert!(close(1, MNT, &mut m).is_err());
        assert!(m.ran("cryptsetup").is_empty());
    }

    #[test]
    fn open_refuses_too_many_missing_and_foreign_disks() {
        let d = devs(2);
        let mut m = Mock::default();
        let none = vec![(d[0].clone(), true), (d[1].clone(), false)];
        assert_eq!(
            open(&none, 0, &key(), MNT, &mut m).unwrap_err().code,
            CODE_OPEN
        );
        // Present but not ours: never formatted on open.
        let both = vec![(d[0].clone(), true), (d[1].clone(), true)];
        assert_eq!(
            open(&both, 1, &key(), MNT, &mut m).unwrap_err().code,
            CODE_OPEN
        );
        assert!(m.ran("cryptsetup").iter().all(|a| a[0] != "luksFormat"));
    }

    const DUMP: &str = "LUKS header information\nVersion:       \t2\n\nData segments:\n  0: crypt\n\toffset: 16777216 [bytes]\nKeyslots:\n  0: luks2\n\tKey:        512 bits\n  2: luks2\n\tKey:        512 bits\nTokens:\n  0: systemd-tpm2\n\tKeyslot:    3\nDigests:\n  0: pbkdf2\n";

    #[test]
    fn keyslots_only_from_their_section() {
        assert_eq!(parse_keyslots(DUMP), [0, 2]);
        assert!(parse_keyslots("").is_empty());
    }

    #[test]
    fn recovery_slots_only_and_secrets_on_stdin() {
        let key = PoolKey::generate().unwrap();
        let mut m = Mock::default();
        // Slot 0 (the pool key) or any other slot is never touched.
        for bad in [0u8, 3, 31] {
            assert!(add_recovery(&mut m, "/dev/vdb1", &key, "abc", bad).is_err());
            assert!(remove_recovery(&mut m, "/dev/vdb1", bad).is_err());
            assert!(test_recovery(&mut m, "/dev/vdb1", "abc", bad).is_err());
        }
        assert!(m.calls.is_empty());
        assert!(add_recovery(&mut m, "/dev/vdb1", &key, "two\nlines", 1).is_err());
        assert!(add_recovery(&mut m, "/dev/vdb1", &key, "", 1).is_err());
        add_recovery(&mut m, "/dev/vdb1", &key, "s3cret-hex", 2).unwrap();
        let args = m.ran("cryptsetup")[0].clone();
        assert!(args.contains(&"--new-key-slot=2".to_string()), "{args:?}");
        assert!(args.contains(&"--keyfile-size=64".to_string()));
        // Neither the key nor the secret is in argv.
        assert!(!args.iter().any(|a| a.contains("s3cret")));
        let sent = m.key_seen.last().unwrap();
        assert_eq!(sent.len(), 64 + "s3cret-hex".len() + 1);
        assert_eq!(&sent[..64], key_bytes(&key).as_slice());
        assert_eq!(&sent[64..], b"s3cret-hex\n");
    }

    #[test]
    fn remove_recovery_only_when_in_use() {
        struct Dump(Vec<String>);
        impl Run for Dump {
            fn run(&mut self, program: &str, args: &[String], _: Option<&[u8]>) -> Out {
                self.0.push(format!("{program} {}", args.join(" ")));
                ok(DUMP)
            }
        }
        let mut d = Dump(vec![]);
        remove_recovery(&mut d, "/dev/vdb1", 1).unwrap();
        assert_eq!(d.0, ["cryptsetup luksDump /dev/vdb1"]);
        remove_recovery(&mut d, "/dev/vdb1", 2).unwrap();
        assert_eq!(d.0[2], "cryptsetup luksKillSlot --batch-mode /dev/vdb1 2");
    }
}
