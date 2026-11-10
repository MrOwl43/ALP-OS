// SPDX-License-Identifier: AGPL-3.0-or-later
//! Safety rule for pool members (contract, ADR 0004): nothing that holds the
//! operating system may ever be formatted for the data pool.
//! `alp-storage` must call [`check_member`] for every member, right before
//! writing, with freshly discovered disks.

use crate::{DiskInfo, HwError, PoolMember, Result};

/// Discoverable Partitions type UUIDs that belong to the system.
const SYSTEM_TYPES: [&str; 4] = [
    "C12A7328-F81F-11D2-BA4B-00A0C93EC93B", // ESP
    "4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709", // root x86-64
    "2C7357ED-EBD2-46D9-AEC1-23D437EC2BF5", // root-verity x86-64
    "41092B05-9FC8-4523-994F-2DEF0408B176", // root-verity-sig x86-64
];
const LINUX_GENERIC: &str = "0FC63DAF-8483-4772-8E79-3D69D8477DE4";
const STATE_LABEL: &str = "alp-state";
const DATA_LABEL: &str = "alp-data";
const REFUSED: &str = "ALP-STO-006";

fn refuse(detail: String) -> HwError {
    HwError {
        code: REFUSED,
        detail,
    }
}

fn holds_system(disk: &DiskInfo) -> bool {
    disk.is_system_disk
        || disk.partitions.iter().any(|p| {
            SYSTEM_TYPES.contains(&p.type_uuid.to_ascii_uppercase().as_str())
                || p.label == STATE_LABEL
        })
}

/// Refuse a member that is not a safe target for the data pool.
pub fn check_member(member: &PoolMember, disks: &[DiskInfo]) -> Result<()> {
    let disk_id = match member {
        PoolMember::WholeDisk(id) => id,
        PoolMember::ExistingPartition { disk, .. } => disk,
    };
    let disk = disks
        .iter()
        .find(|d| &d.id == disk_id)
        .ok_or_else(|| refuse(format!("disk {} not found", disk_id.0)))?;
    match member {
        PoolMember::WholeDisk(_) => {
            if holds_system(disk) {
                return Err(refuse(format!(
                    "disk {} holds the system; it can never be a whole-disk pool member",
                    disk.id.0
                )));
            }
            Ok(())
        }
        PoolMember::ExistingPartition { partition_uuid, .. } => {
            let part = disk
                .partitions
                .iter()
                .find(|p| p.partition_uuid.eq_ignore_ascii_case(partition_uuid))
                .ok_or_else(|| {
                    refuse(format!(
                        "partition {partition_uuid} not found on {}",
                        disk.id.0
                    ))
                })?;
            if !part.type_uuid.eq_ignore_ascii_case(LINUX_GENERIC) || part.label != DATA_LABEL {
                return Err(refuse(format!(
                    "partition {partition_uuid} is not an '{DATA_LABEL}' data partition"
                )));
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DiskId, DiskTransport, PartitionInfo};

    fn part(uuid: &str, type_uuid: &str, label: &str) -> PartitionInfo {
        PartitionInfo {
            name: String::new(),
            partition_uuid: uuid.into(),
            type_uuid: type_uuid.into(),
            label: label.into(),
            size_bytes: 1,
        }
    }

    fn disk(id: &str, system: bool, partitions: Vec<PartitionInfo>) -> DiskInfo {
        DiskInfo {
            id: DiskId(id.into()),
            model: "test".into(),
            serial: None,
            size_bytes: 1 << 40,
            transport: DiskTransport::Virtio,
            rotational: false,
            removable: false,
            is_system_disk: system,
            partitions,
        }
    }

    fn laptop() -> DiskInfo {
        disk(
            "laptop",
            true,
            vec![
                part("p1", SYSTEM_TYPES[0], "esp"),
                part("p2", SYSTEM_TYPES[1], "alp-os_1"),
                part("p6", LINUX_GENERIC, STATE_LABEL),
                part("p7", LINUX_GENERIC, DATA_LABEL),
            ],
        )
    }

    #[test]
    fn system_disk_is_never_a_whole_disk_member() {
        let disks = [laptop()];
        let err =
            check_member(&PoolMember::WholeDisk(DiskId("laptop".into())), &disks).unwrap_err();
        assert_eq!(err.code, REFUSED);
    }

    #[test]
    fn disk_holding_system_partitions_is_refused_even_if_not_flagged() {
        let mut d = laptop();
        d.is_system_disk = false; // e.g. another ALP-OS disk plugged in
        let err = check_member(&PoolMember::WholeDisk(d.id.clone()), &[d]).unwrap_err();
        assert_eq!(err.code, REFUSED);
    }

    #[test]
    fn data_partition_on_single_disk_is_accepted() {
        let m = PoolMember::ExistingPartition {
            disk: DiskId("laptop".into()),
            partition_uuid: "P7".into(),
        };
        assert!(check_member(&m, &[laptop()]).is_ok());
    }

    #[test]
    fn state_and_system_partitions_are_refused_as_partition_members() {
        for uuid in ["p1", "p2", "p6"] {
            let m = PoolMember::ExistingPartition {
                disk: DiskId("laptop".into()),
                partition_uuid: uuid.into(),
            };
            assert_eq!(check_member(&m, &[laptop()]).unwrap_err().code, REFUSED);
        }
    }

    #[test]
    fn empty_data_disk_is_accepted_and_unknown_disk_refused() {
        let blank = disk("blank", false, vec![]);
        assert!(check_member(&PoolMember::WholeDisk(blank.id.clone()), &[blank]).is_ok());
        let err = check_member(&PoolMember::WholeDisk(DiskId("gone".into())), &[]).unwrap_err();
        assert_eq!(err.code, REFUSED);
    }
}
