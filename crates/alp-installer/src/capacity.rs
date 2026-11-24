// SPDX-License-Identifier: AGPL-3.0-or-later
//! Usable capacity of a planned pool, shown before the user confirms
//! (spec block 4: "show the effective capacity"; plan (d)).
//!
//! An estimate from the disk sizes, before anything is written: Btrfs
//! metadata and the LUKS2 header are subtracted as fixed amounts, and the
//! real figure comes from `btrfs filesystem usage` after first boot
//! (**VERIFY** the numbers against trixie's btrfs-progs in the VM test).
//! When stream B's `BtrfsBackend::plan_pool` arrives it must give the same
//! numbers; the function should then move there and be called from here.

use alp_hw::RaidProfile;

/// System partitions on a single-disk install (contract `disk-layout.md`):
/// ESP 1 GiB, two 8 GiB root slots, two 128 MiB verity partitions, 2 GiB
/// state.
pub const SYSTEM_AREA_BYTES: u64 = (1 + 8 + 8 + 2) * GIB + 2 * 128 * MIB;

/// GPT headers at both ends and partition alignment (1 MiB each side is
/// what systemd-repart leaves; rounded up).
pub const GPT_OVERHEAD_BYTES: u64 = 4 * MIB;

/// LUKS2 header with the default 16 MiB size (**VERIFY** against the
/// `cryptsetup luksFormat` defaults stream B uses).
pub const LUKS2_HEADER_BYTES: u64 = 16 * MIB;

const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;

/// Bytes the pool can use on a disk whose data partition fills it:
/// a dedicated data disk, or the rest of a single disk after the system.
pub fn data_partition_bytes(disk_bytes: u64, holds_system: bool) -> u64 {
    let system = if holds_system { SYSTEM_AREA_BYTES } else { 0 };
    disk_bytes
        .saturating_sub(system)
        .saturating_sub(GPT_OVERHEAD_BYTES)
        .saturating_sub(LUKS2_HEADER_BYTES)
}

/// How many copies of every data block a profile keeps.
pub fn copies(profile: RaidProfile) -> u64 {
    match profile {
        RaidProfile::Single => 1,
        RaidProfile::Dup | RaidProfile::Raid1 | RaidProfile::Raid10 => 2,
        RaidProfile::Raid1c3 => 3,
    }
}

/// Usable bytes for `profile` over members of the given sizes.
///
/// `dup` keeps both copies on one disk: half of it. For `c` copies on
/// different disks, Btrfs can fill every disk when no disk is larger than
/// `total / c`; otherwise the largest disk holds one copy of everything and
/// the rest must hold the other `c - 1`, so it is left out and the rule
/// repeats with `c - 1` copies. For RAID1 this is the known
/// `min(total / 2, total - largest)`. RAID10 uses the RAID1 figure
/// (**VERIFY**: Btrfs RAID10 stripes over an even number of disks and can
/// leave some space on odd or mixed sets unused).
pub fn usable_bytes(profile: RaidProfile, sizes: &[u64]) -> u64 {
    match profile {
        RaidProfile::Dup => sizes.iter().sum::<u64>() / 2,
        RaidProfile::Single => sizes.iter().sum(),
        _ => spread(sizes, copies(profile)),
    }
}

fn spread(sizes: &[u64], copies: u64) -> u64 {
    let mut sorted = sizes.to_vec();
    sorted.sort_unstable();
    let mut c = copies;
    while c > 1 {
        if (sorted.len() as u64) < c {
            return 0;
        }
        let total: u64 = sorted.iter().sum();
        let largest = *sorted.last().unwrap_or(&0);
        if largest.saturating_mul(c) <= total {
            return total / c;
        }
        sorted.pop();
        c -= 1;
    }
    sorted.iter().sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TB: u64 = 1_000_000_000_000;

    #[test]
    fn one_disk_dup_is_half() {
        assert_eq!(usable_bytes(RaidProfile::Dup, &[20 * TB]), 10 * TB);
        assert_eq!(usable_bytes(RaidProfile::Single, &[20 * TB]), 20 * TB);
    }

    #[test]
    fn raid1_with_equal_and_mixed_sizes() {
        assert_eq!(
            usable_bytes(RaidProfile::Raid1, &[20 * TB, 20 * TB]),
            20 * TB
        );
        // The largest disk cannot be mirrored beyond what the others hold.
        assert_eq!(usable_bytes(RaidProfile::Raid1, &[20 * TB, 4 * TB]), 4 * TB);
        assert_eq!(
            usable_bytes(RaidProfile::Raid1, &[20 * TB, 10 * TB, 4 * TB]),
            14 * TB
        );
        // Three mixed disks that balance: total / 2.
        assert_eq!(
            usable_bytes(RaidProfile::Raid1, &[8 * TB, 6 * TB, 4 * TB]),
            9 * TB
        );
        // Six 20 TB disks: tens of terabytes.
        assert_eq!(usable_bytes(RaidProfile::Raid1, &[20 * TB; 6]), 60 * TB);
        // A mirror needs two disks.
        assert_eq!(usable_bytes(RaidProfile::Raid1, &[20 * TB]), 0);
    }

    #[test]
    fn raid1c3_keeps_three_copies() {
        assert_eq!(usable_bytes(RaidProfile::Raid1c3, &[20 * TB; 3]), 20 * TB);
        // 20 TB is more than a third of 44 TB: it holds one copy, the rest
        // (10, 10, 4) holds two: min(24 / 2, 24 - 10) = 12.
        assert_eq!(
            usable_bytes(RaidProfile::Raid1c3, &[20 * TB, 10 * TB, 10 * TB, 4 * TB]),
            12 * TB
        );
        assert_eq!(usable_bytes(RaidProfile::Raid1c3, &[20 * TB, 20 * TB]), 0);
    }

    #[test]
    fn raid10_uses_the_mirror_figure() {
        assert_eq!(usable_bytes(RaidProfile::Raid10, &[20 * TB; 4]), 40 * TB);
    }

    #[test]
    fn data_partition_leaves_room_for_the_system() {
        // Contract: the system partitions need about 19.2 GiB.
        assert_eq!(SYSTEM_AREA_BYTES, 20_669_530_112);
        let d = data_partition_bytes(500_107_862_016, true);
        assert_eq!(d, 500_107_862_016 - SYSTEM_AREA_BYTES - 20 * MIB);
        assert_eq!(data_partition_bytes(20 * TB, false), 20 * TB - 20 * MIB);
        assert_eq!(data_partition_bytes(1, true), 0);
    }
}
