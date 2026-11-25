// SPDX-License-Identifier: AGPL-3.0-or-later
//! First start: check the first-boot seed against the disks found now
//! (contract `docs/contracts/firstboot-seed.md`, rules 2 and 3) and turn it
//! into the confirmed pool plan. Pure: the caller passes the disks
//! (`alp_hw::inspect`) and how each `/dev/disk/by-id` name resolves.
//!
//! Any mismatch refuses with `ALP-INST-005` and nothing is written.

use alp_hw::firstboot::FirstbootSeed;
use alp_hw::{
    ConfirmationEvidence, Confirmed, DiskId, DiskInfo, HwError, PoolMember, PoolPlan, Result,
    safety,
};

pub const CODE_CHANGED: &str = "ALP-INST-005";
const DATA_LABEL: &str = "alp-data";
const LINUX_GENERIC: &str = "0FC63DAF-8483-4772-8E79-3D69D8477DE4";

fn changed(detail: String) -> HwError {
    HwError {
        code: CODE_CHANGED,
        detail,
    }
}

/// One member as found now: the disk's kernel name and the partition's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundMember {
    pub disk: String,
    pub partition: String,
}

/// The checked seed: the plan to create and where its members are now.
#[derive(Debug)]
pub struct Checked {
    pub plan: Confirmed<PoolPlan>,
    /// Same order as the seed's members.
    pub found: Vec<FoundMember>,
}

/// Check `seed` against `disks`. `resolve` maps a by-id name to the kernel
/// name of the disk it points to now (`None`: the link is gone). The
/// by-id name carries the disk's serial, so a different disk with the same
/// partition UUID (a cloned disk) is caught here.
pub fn check(
    seed: &FirstbootSeed,
    disks: &[DiskInfo],
    resolve: &dyn Fn(&str) -> Option<String>,
    ledger_seq: u64,
) -> Result<Checked> {
    let mut found = Vec::new();
    let mut members = Vec::new();
    for m in &seed.members {
        let uuid = m.partition_uuid.to_ascii_lowercase();
        let holders: Vec<_> = disks
            .iter()
            .flat_map(|d| d.partitions.iter().map(move |p| (d, p)))
            .filter(|(_, p)| p.partition_uuid.eq_ignore_ascii_case(&uuid))
            .collect();
        let (disk, part) = match holders.as_slice() {
            [one] => *one,
            [] => return Err(changed(format!("disk {} is missing", m.by_id))),
            _ => {
                return Err(changed(format!(
                    "partition {uuid} of {} is on more than one disk",
                    m.by_id
                )));
            }
        };
        if resolve(&m.by_id).as_deref() != Some(disk.id.0.as_str()) {
            return Err(changed(format!(
                "{} is not the disk that holds partition {uuid}",
                m.by_id
            )));
        }
        if disk.size_bytes != m.size_bytes {
            return Err(changed(format!(
                "{} is {} bytes, was {}",
                m.by_id, disk.size_bytes, m.size_bytes
            )));
        }
        if part.label != DATA_LABEL || !part.type_uuid.eq_ignore_ascii_case(LINUX_GENERIC) {
            return Err(changed(format!(
                "partition {uuid} on {} is not an alp-data partition",
                m.by_id
            )));
        }
        let here = PoolMember::ExistingPartition {
            disk: disk.id.clone(),
            partition_uuid: uuid.clone(),
        };
        safety::check_member(&here, disks).map_err(|e| changed(e.to_string()))?;
        if found.iter().any(|f: &FoundMember| f.partition == part.name) {
            return Err(changed(format!("{} is listed twice", m.by_id)));
        }
        found.push(FoundMember {
            disk: disk.id.0.clone(),
            partition: part.name.clone(),
        });
        // The installer names members by their by-id name in the plan.
        members.push(PoolMember::ExistingPartition {
            disk: DiskId(m.by_id.clone()),
            partition_uuid: uuid,
        });
    }
    let plan = PoolPlan {
        members,
        data_profile: seed.data_profile,
        metadata_profile: seed.metadata_profile,
        usable_bytes: seed.usable_bytes,
        protection: seed.protection,
    };
    let evidence = ConfirmationEvidence {
        ledger_seq,
        plan_digest: seed.plan_digest,
    };
    let plan = Confirmed::by_user(plan, evidence)
        .map_err(|_| changed("the disks no longer match the confirmed plan".into()))?;
    Ok(Checked { plan, found })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alp_hw::firstboot::{SEED_FORMAT_VERSION, SeedMember};
    use alp_hw::{DiskTransport, PartitionInfo, PlanDigest, Protection, RaidProfile};

    const U1: &str = "1b4e28ba-2fa1-11d2-883f-0016d3cca427";
    const U2: &str = "2b4e28ba-2fa1-11d2-883f-0016d3cca427";
    const GB64: u64 = 68_719_476_736;

    fn part(name: &str, uuid: &str, label: &str, ty: &str) -> PartitionInfo {
        PartitionInfo {
            name: name.into(),
            partition_uuid: uuid.into(),
            type_uuid: ty.into(),
            label: label.into(),
            size_bytes: 1,
        }
    }

    fn disk(name: &str, system: bool, parts: Vec<PartitionInfo>) -> DiskInfo {
        DiskInfo {
            id: DiskId(name.into()),
            model: "Unknown disk".into(),
            serial: None,
            size_bytes: GB64,
            transport: DiskTransport::Virtio,
            rotational: false,
            removable: false,
            is_system_disk: system,
            partitions: parts,
        }
    }

    fn disks() -> Vec<DiskInfo> {
        vec![
            disk(
                "vda",
                true,
                vec![
                    part("vda1", "e1", "esp", "C12A7328-F81F-11D2-BA4B-00A0C93EC93B"),
                    part("vda6", "e6", "alp-state", LINUX_GENERIC),
                    part("vda8", U1, DATA_LABEL, LINUX_GENERIC),
                ],
            ),
            disk(
                "vdb",
                false,
                vec![part("vdb1", U2, DATA_LABEL, LINUX_GENERIC)],
            ),
        ]
    }

    fn seed() -> FirstbootSeed {
        let mut s = FirstbootSeed {
            format_version: SEED_FORMAT_VERSION,
            installer_version: "0.2.8".into(),
            profile: "generic-x86_64".into(),
            protection_tier: "standard".into(),
            members: vec![
                SeedMember {
                    by_id: "virtio-ALPTARGET01".into(),
                    serial: None,
                    size_bytes: GB64,
                    partition_uuid: U1.into(),
                },
                SeedMember {
                    by_id: "virtio-ALPDATA02".into(),
                    serial: None,
                    size_bytes: GB64,
                    partition_uuid: U2.into(),
                },
            ],
            data_profile: RaidProfile::Raid1,
            metadata_profile: RaidProfile::Raid1,
            usable_bytes: 40_000_000_000,
            protection: Protection::DiskFailure,
            confirmed_at: "2026-11-25T09:30:00Z".into(),
            plan_digest: [0; 32],
            reopen: false,
        };
        // The digest the installer computed over the same plan.
        let plan = PoolPlan {
            members: s
                .members
                .iter()
                .map(|m| PoolMember::ExistingPartition {
                    disk: DiskId(m.by_id.clone()),
                    partition_uuid: m.partition_uuid.clone(),
                })
                .collect(),
            data_profile: s.data_profile,
            metadata_profile: s.metadata_profile,
            usable_bytes: s.usable_bytes,
            protection: s.protection,
        };
        s.plan_digest = plan.plan_digest();
        s
    }

    fn resolve(by_id: &str) -> Option<String> {
        match by_id {
            "virtio-ALPTARGET01" => Some("vda".into()),
            "virtio-ALPDATA02" => Some("vdb".into()),
            _ => None,
        }
    }

    fn code(r: Result<Checked>) -> &'static str {
        r.map(|_| "ok").unwrap_or_else(|e| e.code)
    }

    #[test]
    fn matching_disks_give_the_confirmed_plan() {
        let c = check(&seed(), &disks(), &resolve, 7).unwrap();
        assert_eq!(c.plan.get().data_profile, RaidProfile::Raid1);
        assert_eq!(c.plan.evidence().ledger_seq, 7);
        assert_eq!(
            c.found,
            vec![
                FoundMember {
                    disk: "vda".into(),
                    partition: "vda8".into()
                },
                FoundMember {
                    disk: "vdb".into(),
                    partition: "vdb1".into()
                },
            ]
        );
    }

    #[test]
    fn a_missing_disk_is_refused() {
        let mut d = disks();
        d.pop();
        assert_eq!(code(check(&seed(), &d, &resolve, 1)), CODE_CHANGED);
    }

    #[test]
    fn a_swapped_or_cloned_disk_is_refused() {
        let swapped = |id: &str| match id {
            "virtio-ALPDATA02" => Some("vdc".into()),
            other => resolve(other),
        };
        assert_eq!(code(check(&seed(), &disks(), &swapped, 1)), CODE_CHANGED);
        let mut d = disks();
        d.push(disk(
            "vdc",
            false,
            vec![part("vdc1", U2, DATA_LABEL, LINUX_GENERIC)],
        ));
        assert_eq!(code(check(&seed(), &d, &resolve, 1)), CODE_CHANGED);
    }

    #[test]
    fn a_different_size_or_partition_is_refused() {
        let mut d = disks();
        d[1].size_bytes += 1;
        assert_eq!(code(check(&seed(), &d, &resolve, 1)), CODE_CHANGED);
        let mut d = disks();
        d[1].partitions[0].label = "photos".into();
        assert_eq!(code(check(&seed(), &d, &resolve, 1)), CODE_CHANGED);
    }

    #[test]
    fn a_system_partition_is_refused() {
        let mut s = seed();
        s.members[0].partition_uuid = "e6".into();
        assert_eq!(code(check(&s, &disks(), &resolve, 1)), CODE_CHANGED);
    }

    #[test]
    fn an_edited_plan_is_refused() {
        let mut s = seed();
        s.data_profile = RaidProfile::Single;
        assert_eq!(code(check(&s, &disks(), &resolve, 1)), CODE_CHANGED);
        let mut s = seed();
        s.members.pop();
        assert_eq!(code(check(&s, &disks(), &resolve, 1)), CODE_CHANGED);
    }
}
