// SPDX-License-Identifier: AGPL-3.0-or-later
//! The install plan (ADR 0028, plan `docs/plans/0.2.8-installer.md` (c)
//! and (d)): from the disk list and the user's choice to an ordered,
//! fully explicit list of steps, plus the exact list of disks that will be
//! erased for the confirm screen.
//!
//! Pure: nothing here touches a disk. The steps name the systemd-repart
//! definition sets of the disk-layout contract (`image/install-layout/`);
//! the executor (later) runs them. Right before writing, the executor
//! discovers the disks again and calls [`InstallPlan::recheck`]; any
//! change refuses the whole plan (spec block 7: never touch a disk the user
//! did not confirm).

use alp_hw::inspect::blockdev::{Assessed, Contents, Disk, Role, Verdict};
use alp_hw::{HwError, Protection, RaidProfile, Result};

use crate::capacity;

/// The user's choice cannot be planned (a disk not usable, a profile that
/// does not fit the number of disks, ...).
pub const CODE_REFUSED: &str = "ALP-INST-402";
/// The disks changed between the confirm screen and writing.
pub const CODE_CHANGED: &str = "ALP-INST-401";

/// Smallest data disk (owner decision 2026-11-24: 32 GB). A constant until
/// the hardware profile gets the field (contract change request in
/// `docs/streams/d-installer.md`).
pub const MIN_DATA_DISK_BYTES: u64 = 32_000_000_000;

/// The two ways to start (ADR 0028 decision 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartPath {
    /// Install the system on this internal disk (kernel name).
    InstallTo { system_disk: String },
    /// Keep running from the USB stick ALP-OS started from; only its
    /// existing `alp-state` partition is used, nothing on it is erased.
    KeepRunning,
}

/// Data protection: automatic (spec block 4) or an explicit choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileChoice {
    /// One disk `dup`, two or more `raid1`.
    Auto,
    /// Advanced: one of `single` (one disk only), `raid1c3` (three disks
    /// or more), `raid10` (four or more). RAID5/6 do not exist in
    /// [`RaidProfile`].
    Explicit(RaidProfile),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub path: StartPath,
    /// Kernel names of the chosen data disks. Empty with `InstallTo` means
    /// a single-disk install (system and archive on the system disk).
    pub data_disks: Vec<String>,
    pub profile: ProfileChoice,
}

/// A systemd-repart definition set (contract `disk-layout.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// ESP, two root slots with verity, `alp-state`, `alp-index`.
    System,
    /// `alp-data` filling the rest of a single disk, after `System`.
    SingleDiskData,
    /// One `alp-data` partition filling a dedicated data disk.
    DataDisk,
}

impl Layout {
    /// Directory under `image/install-layout/`.
    pub fn dir(self) -> &'static str {
        match self {
            Layout::System => "system",
            Layout::SingleDiskData => "single-disk-data",
            Layout::DataDisk => "data-disk",
        }
    }

    /// GPT partition labels the set creates, in order. `@v` stands for
    /// the running version (`IMAGE_VERSION`: digits and dots).
    pub fn labels(self) -> &'static [&'static str] {
        match self {
            Layout::System => &[
                "esp",
                "alp-os_@v",
                "alp-os_@v_verity",
                "_empty",
                "_empty",
                "alp-state",
                "alp-index",
            ],
            Layout::SingleDiskData | Layout::DataDisk => &["alp-data"],
        }
    }
}

/// One step, in the order the executor runs them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Erase the disk and write a new GPT with these definition sets
    /// (`systemd-repart --empty=force`, one run with all sets).
    Partition { disk: String, layouts: Vec<Layout> },
    /// Make the system disk start: systemd-boot and the running start file
    /// (UKI) on its ESP. Root slot 1 was already filled during
    /// partitioning (`CopyBlocks=auto`, disk-layout contract).
    CopySystem { disk: String },
    /// Keep-running path: use the start stick's existing `alp-state`
    /// partition. Nothing on the stick is erased.
    UseExistingState { disk: String, partition: String },
    /// Write the first-boot seed (`firstboot-seed.md`) to `alp-state` on
    /// `state_disk`: the `alp-data` partition of every member disk, by
    /// partition UUID read after partitioning, plus the pool profiles.
    WriteSeed {
        state_disk: String,
        members: Vec<String>,
        data_profile: RaidProfile,
        metadata_profile: RaidProfile,
    },
}

/// A disk the plan erases, for the confirm screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Erase {
    pub disk: String,
    pub model: String,
    pub size_bytes: u64,
    /// What is on it now. `Files` means the strong warning naming it and
    /// the slide to confirm (owner decision 2026-11-24).
    pub contents: Contents,
}

/// The pool first boot will create (block 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolSketch {
    /// Disks holding an `alp-data` partition.
    pub members: Vec<String>,
    pub data_profile: RaidProfile,
    pub metadata_profile: RaidProfile,
    /// Estimated capacity after redundancy ([`capacity`]).
    pub usable_bytes: u64,
    pub protection: Protection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallPlan {
    pub choice: Choice,
    pub steps: Vec<Step>,
    /// Every disk that will be erased, in step order. Nothing else is
    /// touched.
    pub erases: Vec<Erase>,
    pub pool: PoolSketch,
    /// The discovery the plan was made from; [`InstallPlan::recheck`]
    /// compares a fresh one with it.
    pub discovery: Vec<Assessed>,
}

impl InstallPlan {
    /// Some erased disk holds files: show the strong warning and the slide.
    pub fn needs_strong_warning(&self) -> bool {
        self.erases.iter().any(|e| e.contents.will_erase_files())
    }

    /// Right before writing: refuses (`ALP-INST-401`) if anything in a
    /// fresh discovery differs from the one the user confirmed, then plans
    /// again from it and refuses unless the result is identical.
    pub fn recheck(&self, fresh: &[Assessed]) -> Result<()> {
        same_discovery(&self.discovery, fresh)?;
        let again = plan(fresh, &self.choice)?;
        if again != *self {
            return Err(changed("the plan differs when made again".into()));
        }
        Ok(())
    }
}

fn refuse(detail: String) -> HwError {
    HwError {
        code: CODE_REFUSED,
        detail,
    }
}

fn changed(detail: String) -> HwError {
    HwError {
        code: CODE_CHANGED,
        detail,
    }
}

/// Refuses (`ALP-INST-401`) unless two discoveries are identical: the same
/// disks in the same order, each with the same facts (size, model,
/// connection, partitions, contents, mounts) and the same verdict.
pub fn same_discovery(before: &[Assessed], after: &[Assessed]) -> Result<()> {
    let names = |l: &[Assessed]| l.iter().map(|a| a.disk.name.clone()).collect::<Vec<_>>();
    if names(before) != names(after) {
        return Err(changed(format!(
            "disks were {:?}, now {:?}",
            names(before),
            names(after)
        )));
    }
    for (a, b) in before.iter().zip(after) {
        if a != b {
            return Err(changed(format!("disk {} changed", a.disk.name)));
        }
    }
    Ok(())
}

fn find<'a>(list: &'a [Assessed], name: &str) -> Result<&'a Assessed> {
    list.iter()
        .find(|a| a.disk.name == name)
        .ok_or_else(|| refuse(format!("disk {name} is not connected")))
}

/// The disk if it may take `role`; its current contents for the erase list.
fn usable_for<'a>(list: &'a [Assessed], name: &str, role: Role) -> Result<(&'a Disk, Contents)> {
    let a = find(list, name)?;
    match &a.verdict {
        Verdict::Usable { roles, contents } if roles.contains(&role) => {
            Ok((&a.disk, contents.clone()))
        }
        Verdict::Usable { .. } => Err(refuse(format!("disk {name} cannot be used as {role:?}"))),
        Verdict::NotUsable(r) => Err(refuse(format!("disk {name}: {}", r.key()))),
    }
}

/// Data and metadata profiles for `n` member disks.
fn profiles(n: usize, choice: ProfileChoice) -> Result<(RaidProfile, RaidProfile)> {
    use RaidProfile::*;
    match (choice, n) {
        (_, 0) => Err(refuse("no data disk".into())),
        (ProfileChoice::Auto, 1) => Ok((Dup, Dup)),
        (ProfileChoice::Auto, _) => Ok((Raid1, Raid1)),
        // Spec block 4: switch data to single; metadata stays dup.
        (ProfileChoice::Explicit(Single), 1) => Ok((Single, Dup)),
        (ProfileChoice::Explicit(Dup), 1) => Ok((Dup, Dup)),
        (ProfileChoice::Explicit(Raid1), n) if n >= 2 => Ok((Raid1, Raid1)),
        (ProfileChoice::Explicit(Raid1c3), n) if n >= 3 => Ok((Raid1c3, Raid1c3)),
        // VERIFY: Btrfs RAID10 minimum device count in trixie's kernel;
        // four is the safe historical minimum.
        (ProfileChoice::Explicit(Raid10), n) if n >= 4 => Ok((Raid10, Raid1)),
        (ProfileChoice::Explicit(p), n) => Err(refuse(format!("{p:?} does not fit {n} disk(s)"))),
    }
}

fn protection(data: RaidProfile) -> Protection {
    match data {
        RaidProfile::Single => Protection::None,
        RaidProfile::Dup => Protection::BitRotOnly,
        _ => Protection::DiskFailure,
    }
}

/// Plans the user's choice over a discovery. Refuses (`ALP-INST-402`) any
/// disk that is not usable for its role, the start stick (except its
/// existing `alp-state` on the keep-running path), a disk named twice, and
/// a profile that does not fit the number of disks.
pub fn plan(discovery: &[Assessed], choice: &Choice) -> Result<InstallPlan> {
    let mut seen = std::collections::BTreeSet::new();
    for d in &choice.data_disks {
        if !seen.insert(d.as_str()) {
            return Err(refuse(format!("disk {d} chosen twice")));
        }
    }
    let mut steps = Vec::new();
    let mut erases = Vec::new();
    let mut members: Vec<(String, u64)> = Vec::new();
    let erase = |d: &Disk, contents: Contents| Erase {
        disk: d.name.clone(),
        model: d.model.clone(),
        size_bytes: d.size_bytes,
        contents,
    };

    let state_disk = match &choice.path {
        StartPath::InstallTo { system_disk } => {
            if seen.contains(system_disk.as_str()) {
                return Err(refuse(format!(
                    "disk {system_disk} is the system disk and a data disk"
                )));
            }
            let single = choice.data_disks.is_empty();
            let role = if single {
                Role::SystemAndData
            } else {
                Role::System
            };
            let (d, contents) = usable_for(discovery, system_disk, role)?;
            let layouts = if single {
                members.push((
                    d.name.clone(),
                    capacity::data_partition_bytes(d.size_bytes, true),
                ));
                vec![Layout::System, Layout::SingleDiskData]
            } else {
                vec![Layout::System]
            };
            steps.push(Step::Partition {
                disk: d.name.clone(),
                layouts,
            });
            erases.push(erase(d, contents));
            d.name.clone()
        }
        StartPath::KeepRunning => {
            let stick = discovery
                .iter()
                .find(|a| {
                    a.verdict == Verdict::NotUsable(alp_hw::inspect::blockdev::Reason::BootStick)
                })
                .ok_or_else(|| refuse("the start stick was not found".into()))?;
            let state = stick
                .disk
                .partitions
                .iter()
                .find(|p| p.label == "alp-state")
                .ok_or_else(|| refuse("the start stick has no alp-state partition".into()))?;
            if choice.data_disks.is_empty() {
                return Err(refuse("keep running needs at least one data disk".into()));
            }
            steps.push(Step::UseExistingState {
                disk: stick.disk.name.clone(),
                partition: state.name.clone(),
            });
            stick.disk.name.clone()
        }
    };

    for name in &choice.data_disks {
        let (d, contents) = usable_for(discovery, name, Role::Data)?;
        steps.push(Step::Partition {
            disk: d.name.clone(),
            layouts: vec![Layout::DataDisk],
        });
        erases.push(erase(d, contents));
        members.push((
            d.name.clone(),
            capacity::data_partition_bytes(d.size_bytes, false),
        ));
    }

    let (data_profile, metadata_profile) = profiles(members.len(), choice.profile)?;
    let sizes: Vec<u64> = members.iter().map(|(_, s)| *s).collect();
    let pool = PoolSketch {
        members: members.into_iter().map(|(n, _)| n).collect(),
        data_profile,
        metadata_profile,
        usable_bytes: capacity::usable_bytes(data_profile, &sizes),
        protection: protection(data_profile),
    };
    if let StartPath::InstallTo { system_disk } = &choice.path {
        steps.push(Step::CopySystem {
            disk: system_disk.clone(),
        });
    }
    steps.push(Step::WriteSeed {
        state_disk,
        members: pool.members.clone(),
        data_profile,
        metadata_profile,
    });
    Ok(InstallPlan {
        choice: choice.clone(),
        steps,
        erases,
        pool,
        discovery: discovery.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alp_hw::inspect::blockdev::{Context, Limits, discover};

    const GB: u64 = 1_000_000_000;

    fn limits() -> Limits {
        Limits {
            system_and_data_bytes: 64 * GB,
            system_bytes: 32 * GB,
            data_bytes: MIN_DATA_DISK_BYTES,
        }
    }

    /// Start stick `sdc` (with `alp-state`); `sda` internal 500 GB blank;
    /// `sdb` internal 1 TB with NTFS files; `nvme0n1` Windows; `sdd` USB
    /// 20 TB blank; `sde` internal 20 TB blank.
    const HOME: &str = include_str!("../tests/fixtures/lsblk/home.json");

    fn disks(json: &str, boot: Option<&str>) -> Vec<Assessed> {
        discover(
            json,
            &Context {
                boot_disk: boot,
                limits: limits(),
            },
        )
        .expect("fixture parses")
    }

    fn home() -> Vec<Assessed> {
        disks(HOME, Some("sdc"))
    }

    fn install(system: &str, data: &[&str], profile: ProfileChoice) -> Choice {
        Choice {
            path: StartPath::InstallTo {
                system_disk: system.into(),
            },
            data_disks: data.iter().map(|s| s.to_string()).collect(),
            profile,
        }
    }

    fn keep(data: &[&str]) -> Choice {
        Choice {
            path: StartPath::KeepRunning,
            data_disks: data.iter().map(|s| s.to_string()).collect(),
            profile: ProfileChoice::Auto,
        }
    }

    fn code(r: Result<InstallPlan>) -> &'static str {
        r.expect_err("must be refused").code
    }

    #[test]
    fn single_disk_install_is_system_plus_data_with_dup() {
        let p = plan(&home(), &install("sda", &[], ProfileChoice::Auto)).unwrap();
        assert_eq!(
            p.steps,
            vec![
                Step::Partition {
                    disk: "sda".into(),
                    layouts: vec![Layout::System, Layout::SingleDiskData]
                },
                Step::CopySystem { disk: "sda".into() },
                Step::WriteSeed {
                    state_disk: "sda".into(),
                    members: vec!["sda".into()],
                    data_profile: RaidProfile::Dup,
                    metadata_profile: RaidProfile::Dup,
                },
            ]
        );
        assert_eq!(p.erases.len(), 1);
        assert_eq!(p.erases[0].contents, Contents::Empty);
        assert!(!p.needs_strong_warning());
        assert_eq!(p.pool.protection, Protection::BitRotOnly);
        let data = capacity::data_partition_bytes(500_107_862_016, true);
        assert_eq!(p.pool.usable_bytes, data / 2);
    }

    #[test]
    fn two_data_disks_are_a_mirror_and_the_system_disk_stays_out() {
        let p = plan(
            &home(),
            &install("sda", &["sde", "sdd"], ProfileChoice::Auto),
        )
        .unwrap();
        assert_eq!(
            p.steps,
            vec![
                Step::Partition {
                    disk: "sda".into(),
                    layouts: vec![Layout::System]
                },
                Step::Partition {
                    disk: "sde".into(),
                    layouts: vec![Layout::DataDisk]
                },
                Step::Partition {
                    disk: "sdd".into(),
                    layouts: vec![Layout::DataDisk]
                },
                Step::CopySystem { disk: "sda".into() },
                Step::WriteSeed {
                    state_disk: "sda".into(),
                    members: vec!["sde".into(), "sdd".into()],
                    data_profile: RaidProfile::Raid1,
                    metadata_profile: RaidProfile::Raid1,
                },
            ]
        );
        let erased: Vec<_> = p.erases.iter().map(|e| e.disk.as_str()).collect();
        assert_eq!(erased, vec!["sda", "sde", "sdd"]);
        assert_eq!(p.pool.protection, Protection::DiskFailure);
        // Two 20 TB disks: 20 TB minus the headers.
        assert_eq!(
            p.pool.usable_bytes,
            capacity::data_partition_bytes(20_000_000_000_000, false)
        );
        assert_eq!(Layout::System.labels().last(), Some(&"alp-index"));
        assert_eq!(Layout::DataDisk.dir(), "data-disk");
    }

    #[test]
    fn a_disk_with_files_is_offered_with_the_strong_warning() {
        let p = plan(
            &home(),
            &install("sda", &["sdb", "sde"], ProfileChoice::Auto),
        )
        .unwrap();
        let sdb = &p.erases[1];
        assert_eq!(sdb.disk, "sdb");
        assert_eq!(
            sdb.contents,
            Contents::Files(vec!["ntfs \"Old photos\" (1 TB)".into()])
        );
        assert!(p.needs_strong_warning());
        // Mixed sizes: the mirror is as big as the smaller side.
        assert_eq!(
            p.pool.usable_bytes,
            capacity::data_partition_bytes(1_000_204_886_016, false)
        );
    }

    #[test]
    fn keep_running_uses_only_the_sticks_state_partition() {
        let p = plan(&home(), &keep(&["sdd"])).unwrap();
        assert_eq!(
            p.steps,
            vec![
                Step::UseExistingState {
                    disk: "sdc".into(),
                    partition: "sdc6".into()
                },
                Step::Partition {
                    disk: "sdd".into(),
                    layouts: vec![Layout::DataDisk]
                },
                Step::WriteSeed {
                    state_disk: "sdc".into(),
                    members: vec!["sdd".into()],
                    data_profile: RaidProfile::Dup,
                    metadata_profile: RaidProfile::Dup,
                },
            ]
        );
        // The stick is never in the erase list.
        assert!(p.erases.iter().all(|e| e.disk != "sdc"));
        // Without data disks there is nothing to keep running for.
        assert_eq!(code(plan(&home(), &keep(&[]))), CODE_REFUSED);
        // The stick itself is never a data disk.
        assert_eq!(code(plan(&home(), &keep(&["sdc"]))), CODE_REFUSED);
    }

    #[test]
    fn keep_running_refuses_without_a_known_stick_or_its_state() {
        // Start stick not found: no keep-running path, and no USB disk.
        assert_eq!(
            code(plan(&disks(HOME, None), &keep(&["sde"]))),
            CODE_REFUSED
        );
        // A stick without alp-state (from the shared fixtures).
        let stick = disks(
            include_str!("../../alp-hw/tests/fixtures/lsblk/boot-stick.json"),
            Some("sdb"),
        );
        assert_eq!(code(plan(&stick, &keep(&["sdc"]))), CODE_REFUSED);
    }

    #[test]
    fn unusable_and_wrong_disks_are_refused() {
        let h = home();
        let auto = ProfileChoice::Auto;
        // Windows disk, as system or data disk.
        assert_eq!(code(plan(&h, &install("nvme0n1", &[], auto))), CODE_REFUSED);
        assert_eq!(
            code(plan(&h, &install("sda", &["nvme0n1"], auto))),
            CODE_REFUSED
        );
        // The start stick.
        assert_eq!(code(plan(&h, &install("sdc", &[], auto))), CODE_REFUSED);
        assert_eq!(
            code(plan(&h, &install("sda", &["sdc"], auto))),
            CODE_REFUSED
        );
        // A USB disk cannot hold the system.
        assert_eq!(code(plan(&h, &install("sdd", &[], auto))), CODE_REFUSED);
        // Unknown disk, a disk twice, system disk also a data disk.
        assert_eq!(code(plan(&h, &install("sdq", &[], auto))), CODE_REFUSED);
        assert_eq!(
            code(plan(&h, &install("sda", &["sde", "sde"], auto))),
            CODE_REFUSED
        );
        assert_eq!(
            code(plan(&h, &install("sda", &["sda"], auto))),
            CODE_REFUSED
        );
    }

    #[test]
    fn explicit_profiles_must_fit_the_number_of_disks() {
        let h = home();
        let ex = ProfileChoice::Explicit;
        let p = plan(&h, &install("sda", &[], ex(RaidProfile::Single))).unwrap();
        assert_eq!(
            (p.pool.data_profile, p.pool.metadata_profile),
            (RaidProfile::Single, RaidProfile::Dup)
        );
        assert_eq!(p.pool.protection, Protection::None);
        assert_eq!(
            code(plan(&h, &install("sda", &[], ex(RaidProfile::Raid1)))),
            CODE_REFUSED
        );
        assert_eq!(
            code(plan(
                &h,
                &install("sda", &["sde", "sdd"], ex(RaidProfile::Raid1c3))
            )),
            CODE_REFUSED
        );
        assert_eq!(
            code(plan(
                &h,
                &install("sda", &["sde", "sdd"], ex(RaidProfile::Single))
            )),
            CODE_REFUSED
        );
        let p = plan(
            &h,
            &install("sda", &["sdb", "sdd", "sde"], ex(RaidProfile::Raid1c3)),
        )
        .unwrap();
        assert_eq!(p.pool.data_profile, RaidProfile::Raid1c3);
        assert_eq!(
            code(plan(
                &h,
                &install("sda", &["sdb", "sdd", "sde"], ex(RaidProfile::Raid10))
            )),
            CODE_REFUSED
        );
    }

    #[test]
    fn recheck_refuses_any_change() {
        let p = plan(
            &home(),
            &install("sda", &["sde", "sdd"], ProfileChoice::Auto),
        )
        .unwrap();
        assert!(p.recheck(&home()).is_ok());

        // A disk unplugged.
        let mut fresh = home();
        fresh.retain(|a| a.disk.name != "sdd");
        assert_eq!(p.recheck(&fresh).unwrap_err().code, CODE_CHANGED);

        // Same names, but a disk of another size answers as sde.
        let mut fresh = home();
        let sde = fresh.iter_mut().find(|a| a.disk.name == "sde").unwrap();
        sde.disk.size_bytes -= 1;
        assert_eq!(p.recheck(&fresh).unwrap_err().code, CODE_CHANGED);

        // Something got written to a disk the user saw as empty.
        let mut fresh = home();
        let sda = fresh.iter_mut().find(|a| a.disk.name == "sda").unwrap();
        sda.disk.table = "gpt".into();
        assert_eq!(p.recheck(&fresh).unwrap_err().code, CODE_CHANGED);

        // A new disk plugged in.
        let mut fresh = home();
        fresh.push(fresh[0].clone());
        fresh.last_mut().unwrap().disk.name = "sdf".into();
        assert_eq!(p.recheck(&fresh).unwrap_err().code, CODE_CHANGED);
    }
}
