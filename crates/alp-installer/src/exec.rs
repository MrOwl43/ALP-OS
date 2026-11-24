// SPDX-License-Identifier: AGPL-3.0-or-later
//! Carries out an [`InstallPlan`] step by step (plan
//! `docs/plans/0.2.8-installer.md`, "Executor").
//!
//! Safety rules, in the order they apply:
//! 1. Nothing is written before the preflight: a fresh discovery must pass
//!    [`InstallPlan::recheck`], every disk the plan uses must have the same
//!    `/dev/disk/by-id` name, serial and size as when the user confirmed,
//!    and the seed must be buildable.
//! 2. Every device path passed to a program belongs to a disk in the plan's
//!    erase list (or, keep-running only, the stick's own `alp-state`
//!    mount); anything else is a safety stop (`ALP-INST-408`).
//! 3. Programs and arguments are fixed lists built here from the plan;
//!    disk names come from the discovery, never from typed text.
//! 4. Every step reports progress; every failure stops at once with an
//!    `ALP-INST-*` code and the step that failed. Disks not in the plan are
//!    never named in any command, so they stay untouched.
//!
//! Pool creation (LUKS2, Btrfs) is not done here: first boot does it from
//! the seed (`firstboot-seed.md`; stream B's `StorageBackend::create_pool`).

use std::collections::BTreeSet;

use alp_core::firstboot::{FirstbootSeed, SEED_FORMAT_VERSION, SeedMember};
use alp_hw::inspect::blockdev::{Context, Limits, discover};
use alp_hw::{DiskId, PoolMember, PoolPlan};
use serde_json::Value;

use crate::plan::{InstallPlan, Step};
use crate::run::Runner;
use crate::seed;

/// Two discoveries differ, or a disk's identity changed.
pub const CODE_CHANGED: &str = crate::plan::CODE_CHANGED;
/// The disk list could not be read.
pub const CODE_DISK_LIST: &str = alp_hw::inspect::blockdev::CODE_DISK_LIST;
/// A disk in the plan has no stable `/dev/disk/by-id` name.
pub const CODE_NO_IDENTITY: &str = "ALP-INST-403";
/// Partitioning a disk failed.
pub const CODE_PARTITION: &str = "ALP-INST-404";
/// Copying the system failed.
pub const CODE_COPY: &str = "ALP-INST-405";
/// Writing the first-boot seed failed.
pub const CODE_SEED: &str = "ALP-INST-406";
/// This installer cannot finish an install yet (the pool plan digest of
/// block 4 is missing).
pub const CODE_NOT_READY: &str = "ALP-INST-407";
/// Internal safety stop: a command would have named a disk outside the plan.
pub const CODE_SAFETY: &str = "ALP-INST-408";

/// `lsblk` columns for the disk list (`blockdev` reads these).
pub const LSBLK_LIST_ARGS: &[&str] = &[
    "--json",
    "--bytes",
    "--tree",
    "--output",
    "NAME,PATH,TYPE,SIZE,MODEL,TRAN,RM,HOTPLUG,ROTA,RO,PTTYPE,PARTTYPE,PARTLABEL,PARTFLAGS,FSTYPE,LABEL,MOUNTPOINTS",
];
/// `lsblk` columns for disk identities (checked on util-linux 2.39.3:
/// the JSON key is `id-link`).
pub const LSBLK_ID_ARGS: &[&str] = &[
    "--json",
    "--bytes",
    "--nodeps",
    "--output",
    "NAME,ID-LINK,SERIAL,SIZE",
];

/// Where things are on the installer image and what goes into the seed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecConfig {
    /// `image/install-layout/` as shipped in the installer image.
    pub layout_dir: String,
    /// Install-time systemd-sysupdate transfer definitions (stream A + D
    /// spike; **VERIFY**).
    pub sysupdate_definitions: String,
    /// Empty directory the new `alp-state` is mounted on to write the seed.
    pub mount_dir: String,
    /// `alp-state` of the running system (keep-running path):
    /// `alp_core::state::STATE_ROOT`.
    pub running_state_root: String,
    /// Kernel name of the start stick (from `alp_hw::inspect`).
    pub boot_disk: Option<String>,
    pub limits: Limits,
    pub installer_version: String,
    pub profile: String,
    /// When the user confirmed (RFC 3339, UTC).
    pub confirmed_at: String,
}

impl ExecConfig {
    /// The installer image's paths (stream A ships the trees there).
    pub fn image_defaults(boot_disk: Option<String>, limits: Limits) -> Self {
        Self {
            layout_dir: "/usr/lib/alp/install-layout".into(),
            sysupdate_definitions: "/usr/lib/alp/install-sysupdate.d".into(),
            mount_dir: "/run/alp-installer/state".into(),
            running_state_root: alp_core::state::STATE_ROOT.into(),
            boot_disk,
            limits,
            installer_version: env!("CARGO_PKG_VERSION").into(),
            profile: "generic-x86_64".into(),
            confirmed_at: String::new(),
        }
    }
}

/// A disk's stable identity, read with [`LSBLK_ID_ARGS`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub name: String,
    /// `/dev/disk/by-id` name (`ID-LINK`); empty when the disk has none.
    pub by_id: String,
    pub serial: Option<String>,
    pub size_bytes: u64,
}

/// Identities from [`LSBLK_ID_ARGS`] output.
pub fn parse_identities(json: &str) -> Result<Vec<Identity>, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("lsblk JSON: {e}"))?;
    let list = v
        .get("blockdevices")
        .and_then(Value::as_array)
        .ok_or("lsblk JSON has no blockdevices array")?;
    let text = |d: &Value, k: &str| {
        d.get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    };
    Ok(list
        .iter()
        .map(|d| Identity {
            name: text(d, "name"),
            by_id: text(d, "id-link"),
            serial: Some(text(d, "serial")).filter(|s| !s.is_empty()),
            size_bytes: d.get("size").and_then(Value::as_u64).unwrap_or(0),
        })
        .collect())
}

/// A partition read back after partitioning.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NewPartition {
    name: String,
    partuuid: String,
    label: String,
}

fn parse_partitions(json: &str) -> Result<Vec<NewPartition>, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("lsblk JSON: {e}"))?;
    let disk = v
        .get("blockdevices")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .ok_or("lsblk JSON has no disk")?;
    let text = |d: &Value, k: &str| {
        d.get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    };
    Ok(disk
        .get("children")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .map(|p| NewPartition {
            name: text(p, "name"),
            partuuid: text(p, "partuuid").to_ascii_lowercase(),
            label: text(p, "partlabel"),
        })
        .collect())
}

/// Computes the pool plan digest (`alp_hw::PlanDigest`, block 4).
pub trait Digester {
    /// False until block 4 implements the digest; then the installer
    /// refuses before writing anything (`ALP-INST-407`).
    fn ready(&self) -> bool;
    fn digest(&self, plan: &PoolPlan) -> [u8; 32];
}

/// The contract implementation. `PlanDigest for PoolPlan` is still
/// `todo!("block 4")` in `alp-hw`; calling it would panic, so `ready()`
/// says no until stream B lands it (then flip it and the VM test runs to
/// the end).
#[derive(Debug, Default)]
pub struct ContractDigest;

impl Digester for ContractDigest {
    fn ready(&self) -> bool {
        false
    }
    fn digest(&self, plan: &PoolPlan) -> [u8; 32] {
        alp_hw::PlanDigest::plan_digest(plan)
    }
}

/// Progress for the Installing screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    /// 0 for the preflight, then the 1-based plan step.
    pub step: usize,
    pub steps: usize,
    pub percent: u8,
    /// Translation key of `message`.
    pub key: &'static str,
    /// Plain English fallback.
    pub message: String,
}

/// Why the install stopped. Nothing after `step` ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecError {
    pub code: &'static str,
    /// 0: preflight (nothing written); otherwise the 1-based plan step.
    pub step: usize,
    /// Technician detail (program, exit status, last lines of its errors).
    pub detail: String,
}

/// Last lines of a program's error output, for technician detail.
fn tail(s: &str) -> String {
    let lines: Vec<&str> = s.lines().rev().take(5).collect();
    lines.into_iter().rev().collect::<Vec<_>>().join(" | ")
}

struct Exec<'a> {
    plan: &'a InstallPlan,
    cfg: &'a ExecConfig,
    runner: &'a mut dyn Runner,
    progress: &'a mut dyn FnMut(Progress),
    step: usize,
}

impl Exec<'_> {
    fn fail(&self, code: &'static str, detail: String) -> ExecError {
        ExecError {
            code,
            step: self.step,
            detail,
        }
    }

    fn say(&mut self, key: &'static str, message: String) {
        let steps = self.plan.steps.len();
        let percent = if key == "install.done" {
            100
        } else {
            u8::try_from(self.step * 100 / (steps + 1)).unwrap_or(100)
        };
        (self.progress)(Progress {
            step: self.step,
            steps,
            percent,
            key,
            message,
        });
    }

    /// `/dev/<disk>` for a disk the plan erases; anything else stops.
    fn dev(&self, disk: &str) -> Result<String, ExecError> {
        if self.plan.erases.iter().any(|e| e.disk == disk) {
            Ok(format!("/dev/{disk}"))
        } else {
            Err(self.fail(
                CODE_SAFETY,
                format!("refused to name /dev/{disk}: not in the erase list"),
            ))
        }
    }

    /// Runs a fixed command; a failure to start or a non-zero exit is
    /// `code`.
    fn run(
        &mut self,
        code: &'static str,
        program: &str,
        args: &[String],
        stdin: Option<&[u8]>,
    ) -> Result<String, ExecError> {
        match self.runner.run(program, args, stdin) {
            Ok(o) if o.ok() => Ok(o.stdout),
            Ok(o) => Err(self.fail(
                code,
                format!("{program} exited with {:?}: {}", o.status, tail(&o.stderr)),
            )),
            Err(e) => Err(self.fail(code, format!("{program} could not start: {e}"))),
        }
    }
}

fn owned(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| (*s).to_string()).collect()
}

/// Carries out `plan`. `confirmed` are the disk identities read when the
/// user confirmed (same [`LSBLK_ID_ARGS`]). Stops at the first failure.
pub fn execute(
    plan: &InstallPlan,
    confirmed: &[Identity],
    cfg: &ExecConfig,
    runner: &mut dyn Runner,
    digester: &dyn Digester,
    progress: &mut dyn FnMut(Progress),
) -> Result<(), ExecError> {
    let mut x = Exec {
        plan,
        cfg,
        runner,
        progress,
        step: 0,
    };

    // ---- preflight: nothing is written before this passes.
    x.say("install.check", "Checking the disks again…".into());
    if !digester.ready() {
        return Err(x.fail(
            CODE_NOT_READY,
            "pool plan digest (alp_hw::PlanDigest for PoolPlan, block 4) not implemented".into(),
        ));
    }
    let list = x.run(CODE_DISK_LIST, "lsblk", &owned(LSBLK_LIST_ARGS), None)?;
    let ctx = Context {
        boot_disk: cfg.boot_disk.as_deref(),
        limits: cfg.limits,
    };
    let fresh = discover(&list, &ctx).map_err(|e| x.fail(e.code, e.detail))?;
    plan.recheck(&fresh).map_err(|e| x.fail(e.code, e.detail))?;
    let ids_json = x.run(CODE_DISK_LIST, "lsblk", &owned(LSBLK_ID_ARGS), None)?;
    let ids = parse_identities(&ids_json).map_err(|e| x.fail(CODE_DISK_LIST, e))?;
    let mut used: Vec<&str> = plan.erases.iter().map(|e| e.disk.as_str()).collect();
    if let Some(Step::UseExistingState { disk, .. }) = plan.steps.first() {
        used.push(disk);
    }
    for name in &used {
        let then = confirmed.iter().find(|i| i.name == *name);
        let now = ids.iter().find(|i| i.name == *name);
        match (then, now) {
            (Some(t), _) if t.by_id.is_empty() => {
                return Err(x.fail(CODE_NO_IDENTITY, format!("disk {name} has no by-id name")));
            }
            (None, _) => {
                return Err(x.fail(
                    CODE_NO_IDENTITY,
                    format!("disk {name} was not identified at confirmation"),
                ));
            }
            (Some(t), Some(n)) if t == n => {}
            (Some(t), n) => {
                return Err(x.fail(
                    CODE_CHANGED,
                    format!(
                        "disk {name} was {:?}, now {:?}",
                        t.by_id,
                        n.map(|n| &n.by_id)
                    ),
                ));
            }
        }
    }

    // ---- the steps.
    let mut parts: Vec<(String, Vec<NewPartition>)> = Vec::new();
    for (i, step) in plan.steps.iter().enumerate() {
        x.step = i + 1;
        match step {
            Step::Partition { disk, layouts } => {
                let dev = x.dev(disk)?;
                let model = plan
                    .erases
                    .iter()
                    .find(|e| e.disk == *disk)
                    .map(|e| e.model.clone())
                    .unwrap_or_default();
                x.say("install.partition", format!("Preparing the disk {model}…"));
                let mut args = owned(&[
                    "--dry-run=no",
                    "--no-pager",
                    "--empty=force",
                    "--seed=random",
                ]);
                for l in layouts {
                    args.push(format!("--definitions={}/{}", cfg.layout_dir, l.dir()));
                }
                args.push(dev.clone());
                x.run(CODE_PARTITION, "systemd-repart", &args, None)?;
                x.run(
                    CODE_PARTITION,
                    "udevadm",
                    &owned(&["settle", "--timeout=30"]),
                    None,
                )?;
                let out = x.run(
                    CODE_PARTITION,
                    "lsblk",
                    &owned(&["--json", "--output", "NAME,PARTUUID,PARTLABEL", &dev]),
                    None,
                )?;
                let got = parse_partitions(&out).map_err(|e| x.fail(CODE_PARTITION, e))?;
                let want: Vec<&str> = layouts
                    .iter()
                    .flat_map(|l| l.labels().iter().copied())
                    .collect();
                let labels: Vec<&str> = got.iter().map(|p| p.label.as_str()).collect();
                if labels != want || got.iter().any(|p| p.partuuid.is_empty()) {
                    return Err(x.fail(
                        CODE_PARTITION,
                        format!("{dev}: partitions {labels:?}, expected {want:?}"),
                    ));
                }
                parts.push((disk.clone(), got));
                // Every partition UUID must be new and unique: pool members
                // are found by it (repart's --seed=random, VERIFY).
                let mut seen = BTreeSet::new();
                if !parts
                    .iter()
                    .flat_map(|(_, p)| p)
                    .all(|p| seen.insert(p.partuuid.clone()))
                {
                    return Err(x.fail(CODE_PARTITION, "two partitions got the same UUID".into()));
                }
            }
            Step::CopySystem { disk } => {
                let dev = x.dev(disk)?;
                x.say("install.copy", "Copying ALP-OS to the disk…".into());
                // VERIFY (stream A + D spike): sysupdate --image and the
                // install-time transfer definitions; bootctl --image.
                let defs = format!("--definitions={}", cfg.sysupdate_definitions);
                let image = format!("--image={dev}");
                x.run(
                    CODE_COPY,
                    "/usr/lib/systemd/systemd-sysupdate",
                    &[defs, image.clone(), "update".into()],
                    None,
                )?;
                x.run(CODE_COPY, "bootctl", &[image, "install".into()], None)?;
            }
            Step::UseExistingState { disk, partition } => {
                x.say("install.state", "Using the USB stick's own storage…".into());
                let ok = fresh.iter().any(|a| {
                    a.disk.name == *disk
                        && a.disk
                            .partitions
                            .iter()
                            .any(|p| p.name == *partition && p.label == "alp-state" && p.mounted)
                });
                if !ok {
                    return Err(x.fail(
                        CODE_SEED,
                        format!("{partition} on {disk} is not the mounted alp-state"),
                    ));
                }
            }
            Step::WriteSeed {
                state_disk,
                members,
                data_profile,
                metadata_profile,
            } => {
                x.say(
                    "install.seed",
                    "Saving your choices for the first start…".into(),
                );
                let part_of = |disk: &str, label: &str| {
                    parts
                        .iter()
                        .find(|(d, _)| d == disk)
                        .and_then(|(_, p)| p.iter().find(|p| p.label == label))
                        .map(|p| p.partuuid.clone())
                };
                let mut seed_members = Vec::new();
                let mut pool_members = Vec::new();
                for m in members {
                    let id = ids.iter().find(|i| i.name == *m);
                    let (Some(id), Some(uuid)) = (id, part_of(m, "alp-data")) else {
                        return Err(x.fail(CODE_SEED, format!("no alp-data partition on {m}")));
                    };
                    seed_members.push(SeedMember {
                        by_id: id.by_id.clone(),
                        serial: id.serial.clone(),
                        size_bytes: id.size_bytes,
                        partition_uuid: uuid.clone(),
                    });
                    pool_members.push(PoolMember::ExistingPartition {
                        disk: DiskId(id.by_id.clone()),
                        partition_uuid: uuid,
                    });
                }
                let pool = PoolPlan {
                    members: pool_members,
                    data_profile: *data_profile,
                    metadata_profile: *metadata_profile,
                    usable_bytes: plan.pool.usable_bytes,
                    protection: plan.pool.protection,
                };
                let text = seed::to_toml(&FirstbootSeed {
                    format_version: SEED_FORMAT_VERSION,
                    installer_version: cfg.installer_version.clone(),
                    profile: cfg.profile.clone(),
                    protection_tier: "standard".into(),
                    members: seed_members,
                    confirmed_at: cfg.confirmed_at.clone(),
                    plan_digest: digester.digest(&pool),
                });
                if plan.erases.iter().any(|e| e.disk == *state_disk) {
                    let Some(uuid) = part_of(state_disk, "alp-state") else {
                        return Err(
                            x.fail(CODE_SEED, format!("no alp-state partition on {state_disk}"))
                        );
                    };
                    write_seed_new_state(&mut x, &uuid, &text)?;
                } else {
                    let root = std::path::Path::new(&cfg.running_state_root);
                    let file = alp_core::state::paths_under(root, root, root).firstboot_seed;
                    let file = file.to_string_lossy().into_owned();
                    x.run(
                        CODE_SEED,
                        "install",
                        &owned(&["-m", "0600", "/dev/stdin", &file]),
                        Some(text.as_bytes()),
                    )?;
                    x.run(CODE_SEED, "sync", &owned(&["-f", &file]), None)?;
                }
            }
        }
    }
    x.step = plan.steps.len();
    x.say("install.done", "ALP-OS is installed.".into());
    Ok(())
}

/// Mount the new `alp-state` (by its partition UUID, read back after
/// partitioning a disk in the erase list), write the seed, unmount.
fn write_seed_new_state(x: &mut Exec<'_>, uuid: &str, text: &str) -> Result<(), ExecError> {
    let dir = x.cfg.mount_dir.clone();
    let dev = format!("/dev/disk/by-partuuid/{uuid}");
    x.run(CODE_SEED, "mkdir", &owned(&["-p", &dir]), None)?;
    x.run(
        CODE_SEED,
        "mount",
        &owned(&["-t", "ext4", &dev, &dir]),
        None,
    )?;
    let root = std::path::Path::new(&dir);
    let file = alp_core::state::paths_under(root, root, root).firstboot_seed;
    let file = file.to_string_lossy().into_owned();
    let written = x
        .run(
            CODE_SEED,
            "install",
            &owned(&["-m", "0600", "/dev/stdin", &file]),
            Some(text.as_bytes()),
        )
        .and_then(|_| x.run(CODE_SEED, "sync", &owned(&["-f", &file]), None));
    // Unmount even when writing failed; report the first failure.
    let unmounted = x.run(CODE_SEED, "umount", &owned(&[&dir]), None);
    written.and(unmounted).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{Choice, Layout, ProfileChoice, StartPath, plan};
    use crate::run::Output;
    use alp_hw::inspect::blockdev::Assessed;

    const GB: u64 = 1_000_000_000;
    const HOME: &str = include_str!("../tests/fixtures/lsblk/home.json");

    fn limits() -> Limits {
        Limits {
            system_and_data_bytes: 64 * GB,
            system_bytes: 32 * GB,
            data_bytes: crate::plan::MIN_DATA_DISK_BYTES,
        }
    }

    fn cfg() -> ExecConfig {
        let mut c = ExecConfig::image_defaults(Some("sdc".into()), limits());
        c.installer_version = "0.2.8".into();
        c.confirmed_at = "2026-11-24T09:30:00Z".into();
        c
    }

    fn home() -> Vec<Assessed> {
        discover(
            HOME,
            &Context {
                boot_disk: Some("sdc"),
                limits: limits(),
            },
        )
        .unwrap()
    }

    fn ids() -> Vec<Identity> {
        ["nvme0n1", "sda", "sdb", "sdc", "sdd", "sde"]
            .iter()
            .map(|n| Identity {
                name: (*n).into(),
                by_id: format!("ata-DISK_{n}"),
                serial: Some(format!("SER{n}")),
                size_bytes: 1,
            })
            .collect()
    }

    fn ids_json(ids: &[Identity]) -> String {
        let list: Vec<Value> = ids
            .iter()
            .map(|i| {
                serde_json::json!({"name": i.name, "id-link": i.by_id, "serial": i.serial, "size": i.size_bytes})
            })
            .collect();
        serde_json::json!({ "blockdevices": list }).to_string()
    }

    /// Partitions repart would make on `disk` for its labels.
    fn parts_json(disk: &str, labels: &[&str]) -> String {
        let children: Vec<Value> = labels
            .iter()
            .enumerate()
            .map(|(i, l)| {
                serde_json::json!({"name": format!("{disk}{}", i + 1),
                    "partuuid": format!("00000000-0000-4000-8000-{:06}{:06}", disk.len() * 1000 + disk.bytes().last().unwrap() as usize, i),
                    "partlabel": l})
            })
            .collect();
        serde_json::json!({"blockdevices": [{"name": disk, "partuuid": null, "partlabel": null, "children": children}]})
            .to_string()
    }

    #[derive(Default)]
    struct Mock {
        calls: Vec<(String, Vec<String>, Option<String>)>,
        list: String,
        ids: String,
        /// Program whose run fails (exit 1).
        fail: Option<&'static str>,
        /// Labels repart produces per disk.
        layout: Vec<(String, Vec<&'static str>)>,
        /// Every new partition reads back with the same UUID.
        same_uuid: bool,
    }

    impl Runner for Mock {
        fn run(
            &mut self,
            program: &str,
            args: &[String],
            stdin: Option<&[u8]>,
        ) -> std::io::Result<Output> {
            self.calls.push((
                program.into(),
                args.to_vec(),
                stdin.map(|s| String::from_utf8_lossy(s).into_owned()),
            ));
            if self.fail == Some(program) {
                return Ok(Output {
                    status: Some(1),
                    stdout: String::new(),
                    stderr: "boom".into(),
                });
            }
            let stdout = match (program, args.get(2).map(String::as_str)) {
                ("lsblk", Some("--tree")) => self.list.clone(),
                ("lsblk", Some("--nodeps")) => self.ids.clone(),
                ("lsblk", _) => {
                    let dev = args.last().unwrap().trim_start_matches("/dev/").to_string();
                    let labels = self
                        .layout
                        .iter()
                        .find(|(d, _)| *d == dev)
                        .map(|(_, l)| l.clone())
                        .unwrap_or_default();
                    let json = parts_json(&dev, &labels);
                    if self.same_uuid {
                        let mut v: Value = serde_json::from_str(&json).unwrap();
                        for c in v["blockdevices"][0]["children"].as_array_mut().unwrap() {
                            c["partuuid"] = "11111111-2222-4333-8444-555555555555".into();
                        }
                        v.to_string()
                    } else {
                        json
                    }
                }
                _ => String::new(),
            };
            Ok(Output {
                status: Some(0),
                stdout,
                stderr: String::new(),
            })
        }
    }

    struct TestDigest(bool);
    impl Digester for TestDigest {
        fn ready(&self) -> bool {
            self.0
        }
        fn digest(&self, plan: &PoolPlan) -> [u8; 32] {
            [plan.members.len() as u8; 32]
        }
    }

    const SYSTEM: &[&str] = &["esp", "_empty", "_empty", "_empty", "_empty", "alp-state"];

    fn mock() -> Mock {
        Mock {
            list: HOME.into(),
            ids: ids_json(&ids()),
            layout: vec![
                ("sda".into(), [SYSTEM, &["alp-data"]].concat()),
                ("sdd".into(), vec!["alp-data"]),
                ("sde".into(), vec!["alp-data"]),
                ("sdb".into(), vec!["alp-data"]),
            ],
            ..Mock::default()
        }
    }

    fn install(system: &str, data: &[&str]) -> InstallPlan {
        plan(
            &home(),
            &Choice {
                path: StartPath::InstallTo {
                    system_disk: system.into(),
                },
                data_disks: data.iter().map(|s| s.to_string()).collect(),
                profile: ProfileChoice::Auto,
            },
        )
        .unwrap()
    }

    fn run_it(
        p: &InstallPlan,
        m: &mut Mock,
        d: &dyn Digester,
    ) -> (Result<(), ExecError>, Vec<Progress>) {
        let mut events = Vec::new();
        let r = execute(p, &ids(), &cfg(), m, d, &mut |e| events.push(e));
        (r, events)
    }

    fn programs(m: &Mock) -> Vec<String> {
        m.calls.iter().map(|(p, _, _)| p.clone()).collect()
    }

    fn device_args(m: &Mock) -> Vec<String> {
        m.calls
            .iter()
            .filter(|(p, _, _)| p != "lsblk")
            .flat_map(|(_, a, _)| a.iter())
            .filter(|a| a.contains("/dev/"))
            .cloned()
            .collect()
    }

    #[test]
    fn two_disk_install_runs_exactly_the_planned_commands() {
        let p = install("sda", &["sde", "sdd"]);
        let mut m = mock();
        m.layout[0].1 = SYSTEM.to_vec();
        let (r, events) = run_it(&p, &mut m, &TestDigest(true));
        r.unwrap();
        assert_eq!(
            programs(&m),
            vec![
                "lsblk",
                "lsblk", // preflight: list, identities
                "systemd-repart",
                "udevadm",
                "lsblk", // sda
                "systemd-repart",
                "udevadm",
                "lsblk", // sde
                "systemd-repart",
                "udevadm",
                "lsblk", // sdd
                "/usr/lib/systemd/systemd-sysupdate",
                "bootctl",
                "mkdir",
                "mount",
                "install",
                "sync",
                "umount",
            ]
        );
        let repart = &m.calls[2].1;
        assert_eq!(
            repart,
            &vec![
                "--dry-run=no".to_string(),
                "--no-pager".into(),
                "--empty=force".into(),
                "--seed=random".into(),
                "--definitions=/usr/lib/alp/install-layout/system".into(),
                "/dev/sda".into(),
            ]
        );
        assert_eq!(
            m.calls[5].1[4],
            "--definitions=/usr/lib/alp/install-layout/data-disk"
        );
        // Only the three planned disks are ever named.
        for a in device_args(&m) {
            assert!(
                ["/dev/sda", "/dev/sde", "/dev/sdd", "--image=/dev/sda"].contains(&a.as_str())
                    || a.starts_with("/dev/disk/by-partuuid/")
                    || a == "/dev/stdin",
                "unexpected device argument {a}"
            );
        }
        // The seed goes to the new alp-state, with both members.
        let (_, args, stdin) = &m.calls[15];
        assert_eq!(args[..3], ["-m", "0600", "/dev/stdin"]);
        assert_eq!(args[3], "/run/alp-installer/state/firstboot-seed.toml");
        let seed: toml::Table = stdin.as_deref().unwrap().parse().unwrap();
        let members = seed["members"].as_array().unwrap();
        assert_eq!(members[0]["by_id"].as_str(), Some("ata-DISK_sde"));
        assert_eq!(members[1]["by_id"].as_str(), Some("ata-DISK_sdd"));
        assert_eq!(seed["plan_digest"].as_str(), Some("02".repeat(32).as_str()));
        assert_eq!(seed["confirmed_at"].as_str(), Some("2026-11-24T09:30:00Z"));
        // Progress: preflight, every step, done; percent never goes back.
        assert_eq!(events.first().unwrap().key, "install.check");
        assert_eq!(events.last().unwrap().key, "install.done");
        assert_eq!(events.last().unwrap().percent, 100);
        assert!(events.windows(2).all(|w| w[0].percent <= w[1].percent));
    }

    #[test]
    fn single_disk_install_checks_system_and_data_partitions() {
        let p = install("sda", &[]);
        let mut m = mock();
        run_it(&p, &mut m, &TestDigest(true)).0.unwrap();
        let repart = &m.calls[2].1;
        assert!(
            repart.contains(
                &"--definitions=/usr/lib/alp/install-layout/single-disk-data".to_string()
            )
        );
        // Wrong partitions after repart: stop before copying anything.
        let mut m = mock();
        m.layout[0].1 = vec!["esp"];
        let (r, _) = run_it(&p, &mut m, &TestDigest(true));
        let e = r.unwrap_err();
        assert_eq!((e.code, e.step), (CODE_PARTITION, 1));
        assert!(!programs(&m).iter().any(|p| p.contains("sysupdate")));
    }

    #[test]
    fn keep_running_writes_only_to_the_running_state() {
        let p = plan(
            &home(),
            &Choice {
                path: StartPath::KeepRunning,
                data_disks: vec!["sdd".into()],
                profile: ProfileChoice::Auto,
            },
        )
        .unwrap();
        let mut m = mock();
        run_it(&p, &mut m, &TestDigest(true)).0.unwrap();
        assert_eq!(
            programs(&m),
            vec![
                "lsblk",
                "lsblk",
                "systemd-repart",
                "udevadm",
                "lsblk",
                "install",
                "sync"
            ]
        );
        assert_eq!(m.calls[2].1.last().unwrap(), "/dev/sdd");
        assert_eq!(m.calls[5].1[3], "/var/lib/alp/firstboot-seed.toml");
        assert!(device_args(&m).iter().all(|a| !a.contains("sdc")));
    }

    #[test]
    fn nothing_is_written_when_the_preflight_fails() {
        let p = install("sda", &["sde", "sdd"]);
        let writes = |m: &Mock| programs(m).iter().filter(|p| *p != "lsblk").count();

        // Block 4 digest not ready.
        let mut m = mock();
        let e = run_it(&p, &mut m, &TestDigest(false)).0.unwrap_err();
        assert_eq!((e.code, e.step), (CODE_NOT_READY, 0));
        assert!(m.calls.is_empty());
        assert!(!ContractDigest.ready());

        // A disk changed since the confirm screen.
        let mut m = mock();
        m.list = HOME.replace("Elements 25A3", "Elements 25A4");
        let e = run_it(&p, &mut m, &TestDigest(true)).0.unwrap_err();
        assert_eq!((e.code, e.step), (CODE_CHANGED, 0));
        assert_eq!(writes(&m), 0);

        // Same names, but another disk answers as sde (by-id differs).
        let mut m = mock();
        let mut now = ids();
        now[5].by_id = "ata-OTHER".into();
        m.ids = ids_json(&now);
        let e = run_it(&p, &mut m, &TestDigest(true)).0.unwrap_err();
        assert_eq!((e.code, e.step), (CODE_CHANGED, 0));
        assert_eq!(writes(&m), 0);

        // A planned disk without a by-id name (virtio without serial).
        let mut m = mock();
        let mut confirmed = ids();
        confirmed[1].by_id.clear();
        let e = execute(
            &p,
            &confirmed,
            &cfg(),
            &mut m,
            &TestDigest(true),
            &mut |_| {},
        )
        .unwrap_err();
        assert_eq!(e.code, CODE_NO_IDENTITY);
        assert_eq!(writes(&m), 0);

        // Unreadable disk list.
        let mut m = mock();
        m.list = "not json".into();
        let e = run_it(&p, &mut m, &TestDigest(true)).0.unwrap_err();
        assert_eq!(e.code, CODE_DISK_LIST);
        assert_eq!(writes(&m), 0);
    }

    #[test]
    fn a_failing_step_stops_everything_after_it() {
        let p = install("sda", &["sde", "sdd"]);
        let mut m = mock();
        m.layout[0].1 = SYSTEM.to_vec();
        m.fail = Some("bootctl");
        let e = run_it(&p, &mut m, &TestDigest(true)).0.unwrap_err();
        assert_eq!((e.code, e.step), (CODE_COPY, 4));
        assert!(e.detail.contains("boom"));
        assert_eq!(programs(&m).last().unwrap(), "bootctl");

        let mut m = mock();
        m.fail = Some("systemd-repart");
        let e = run_it(&p, &mut m, &TestDigest(true)).0.unwrap_err();
        assert_eq!((e.code, e.step), (CODE_PARTITION, 1));
        assert_eq!(
            programs(&m)
                .iter()
                .filter(|p| *p == "systemd-repart")
                .count(),
            1
        );
    }

    #[test]
    fn a_failed_seed_write_still_unmounts() {
        let p = install("sda", &["sde", "sdd"]);
        let mut m = mock();
        m.layout[0].1 = SYSTEM.to_vec();
        m.fail = Some("install");
        let e = run_it(&p, &mut m, &TestDigest(true)).0.unwrap_err();
        assert_eq!((e.code, e.step), (CODE_SEED, 5));
        assert_eq!(programs(&m).last().unwrap(), "umount");
    }

    #[test]
    fn a_disk_outside_the_erase_list_is_a_safety_stop() {
        let mut p = install("sda", &["sde", "sdd"]);
        // A plan altered after confirmation (a bug): sdb is not in the erase list.
        p.steps[1] = Step::Partition {
            disk: "sdb".into(),
            layouts: vec![Layout::DataDisk],
        };
        let mut m = mock();
        m.layout[0].1 = SYSTEM.to_vec();
        // recheck catches the altered plan first.
        let e = run_it(&p, &mut m, &TestDigest(true)).0.unwrap_err();
        assert_eq!(e.code, CODE_CHANGED);
        // The guard itself, when reached.
        let mut events = |_: Progress| {};
        let c = cfg();
        let x = Exec {
            plan: &p,
            cfg: &c,
            runner: &mut m,
            progress: &mut events,
            step: 2,
        };
        assert_eq!(x.dev("sdb").unwrap_err().code, CODE_SAFETY);
        assert_eq!(x.dev("nvme0n1").unwrap_err().code, CODE_SAFETY);
        assert_eq!(x.dev("sde").unwrap(), "/dev/sde");
    }

    #[test]
    fn identical_partition_uuids_are_refused() {
        // Two data disks whose alp-data partitions read back with one UUID
        // (a repart seed that is not random): stop before the second disk
        // is used any further.
        let p = install("sda", &["sde", "sdd"]);
        let mut m = mock();
        m.layout[0].1 = SYSTEM.to_vec();
        m.same_uuid = true;
        let e = run_it(&p, &mut m, &TestDigest(true)).0.unwrap_err();
        assert_eq!((e.code, e.step), (CODE_PARTITION, 1));
        assert!(e.detail.contains("same UUID"));
    }

    #[test]
    fn identities_parse_from_lsblk() {
        let ids = parse_identities(
            r#"{"blockdevices":[{"name":"vda","id-link":null,"serial":null,"size":274877906944},
               {"name":"sda","id-link":"ata-WDC_X_123","serial":"123","size":4000787030016}]}"#,
        )
        .unwrap();
        assert_eq!(ids[0].by_id, "");
        assert_eq!(ids[0].serial, None);
        assert_eq!(ids[1].by_id, "ata-WDC_X_123");
        assert_eq!(ids[1].size_bytes, 4_000_787_030_016);
        assert!(parse_identities("{}").is_err());
    }
}
