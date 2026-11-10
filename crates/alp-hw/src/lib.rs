// SPDX-License-Identifier: AGPL-3.0-or-later
//! Hardware profile and the backend traits of spec Section 5.
//!
//! Contract (ADR 0004). Code above this layer must not assume UEFI, TPM,
//! Btrfs or any specific hardware (spec Section 2 rule 5). v1 ships one
//! implementation per trait (two for key sealing); future hardware adds
//! implementations, not changes to callers. Changing a signature or type
//! here is a contract change: one PR that updates every user.
//!
//! Deviations from the literal Section 5 signatures (ADR 0004, approved by
//! the owner in ADR 0005): traits are `Send + Sync`; destructive storage
//! operations take a [`Confirmed`] value (spec Section 2 rule 8);
//! `replace_disk` takes one confirmed [`DiskReplacement`].

use std::fmt;
use std::path::PathBuf;

pub use alp_crypto::{KeySealBackend, PoolKey, SealCapability, SealedBlob, Secret};

pub mod inspect;
pub mod profile;
pub mod safety;

/// Error from a backend. `code` is a user-facing code from
/// `errors/catalog.toml` (e.g. `"ALP-STO-001"`); `detail` is technician
/// detail and must never contain secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HwError {
    pub code: &'static str,
    pub detail: String,
}

impl fmt::Display for HwError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.detail)
    }
}

impl std::error::Error for HwError {}

pub type Result<T> = std::result::Result<T, HwError>;

// -------------------------------------------------------- confirmation

/// A canonical SHA3-256 digest of a plan, as shown to the user. Two plans
/// with the same digest do exactly the same thing. There is exactly one
/// implementation per plan type, here in `alp-hw` (block 4, stream B); the
/// installer and `alpd` both call it and never re-implement it. It hashes a
/// versioned canonical encoding of every field of the plan (members by
/// partition UUID, profiles, usable bytes, protection). Disk serials and
/// sizes are not part of the plan; they are checked separately when a plan
/// is applied (first-boot seed rule 2, `safety::check_member`).
pub trait PlanDigest {
    fn plan_digest(&self) -> [u8; 32];
}

/// Evidence that the user explicitly confirmed an operation (spec Section 2
/// rule 8): the ledger entry `alpd` wrote at confirmation time
/// (`LedgerEvent::OperationConfirmed`) and the digest of the plan it covers.
/// For pool creation during installation, the evidence comes from the
/// first-boot seed (docs/contracts/firstboot-seed.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfirmationEvidence {
    pub ledger_seq: u64,
    pub plan_digest: [u8; 32],
}

/// A value the user confirmed. It cannot be changed after confirmation:
/// there is no mutable access, and it can only be created for a value whose
/// digest equals the confirmed digest, so a backend applies exactly what the
/// user saw. (This guards against mistakes in our own code; it is not a
/// security boundary inside one process.)
#[derive(Debug)]
pub struct Confirmed<T> {
    value: T,
    evidence: ConfirmationEvidence,
}

impl<T: PlanDigest> Confirmed<T> {
    /// Refuses (`ALP-STO-007`) if `value` is not the plan the user confirmed.
    pub fn by_user(value: T, evidence: ConfirmationEvidence) -> Result<Self> {
        if value.plan_digest() != evidence.plan_digest {
            return Err(HwError {
                code: "ALP-STO-007",
                detail: "plan differs from the confirmed plan".into(),
            });
        }
        Ok(Self { value, evidence })
    }
}

impl<T> Confirmed<T> {
    pub fn get(&self) -> &T {
        &self.value
    }

    pub fn evidence(&self) -> ConfirmationEvidence {
        self.evidence
    }
}

// ---------------------------------------------------------------- boot

/// An installed system slot.
///
/// The stable identity is `partition_number` (GPT entry of the slot's root
/// partition). `root_partition_uuid` changes with every update: systemd
/// derives it from the image's dm-verity root hash, so it identifies the
/// image in the slot, not the slot. The update tool labels a slot with the
/// version it holds and marks a free slot `_empty`
/// (docs/contracts/disk-layout.md).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    pub partition_number: u32,
    pub root_partition_uuid: String,
    /// Version held by the slot (from its partition label), if any.
    pub version: Option<String>,
}

/// An update that systemd-sysupdate reported as available. sysupdate itself
/// verifies the signature (`SHA256SUMS.gpg`) and hashes while it downloads
/// and writes (block 2); `stage_update` asks it to install this version into
/// the free slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedImage {
    version: String,
}

impl VerifiedImage {
    /// Only from the list of versions `systemd-sysupdate` reports.
    pub fn from_sysupdate(version: String) -> Self {
        Self { version }
    }

    pub fn version(&self) -> &str {
        &self.version
    }
}

/// Boot chain and A/B updates (block 2). v1: [`UefiSystemdBoot`].
pub trait BootBackend: Send + Sync {
    fn current_slot(&self) -> Slot;
    fn stage_update(&self, image: &VerifiedImage) -> Result<()>;
    fn mark_good(&self) -> Result<()>;
    fn rollback_pending(&self) -> bool;
}

// ------------------------------------------------------------- storage

/// A disk's `/dev/disk/by-id` name. For display and lookup only: it can
/// change when a disk moves between USB and SATA. Pool members are
/// identified by partition UUID ([`PoolMember`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DiskId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskTransport {
    Sata,
    Nvme,
    Usb,
    Virtio,
    Other,
}

/// A partition found on a disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartitionInfo {
    /// Kernel name, e.g. `nvme0n1p3` (`/dev/<name>`); empty when unknown.
    pub name: String,
    /// GPT partition UUID (stable identity of a pool member).
    pub partition_uuid: String,
    /// GPT partition type UUID, upper case.
    pub type_uuid: String,
    /// GPT partition label.
    pub label: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskInfo {
    pub id: DiskId,
    /// Model as reported by the disk, for display.
    pub model: String,
    pub serial: Option<String>,
    pub size_bytes: u64,
    pub transport: DiskTransport,
    pub rotational: bool,
    pub removable: bool,
    /// True if ALP-OS boots from this disk (holds ESP, slots and `state`).
    pub is_system_disk: bool,
    pub partitions: Vec<PartitionInfo>,
}

/// What joins the pool. Every member must pass [`safety::check_member`]
/// before anything is written.
///
/// Who partitions: the installer creates the `alp-data` partition on every
/// disk the user confirmed (single disk: next to the system partitions;
/// dedicated data disks: `image/install-layout/data-disk/`). So pool
/// creation at first boot uses only [`PoolMember::ExistingPartition`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PoolMember {
    /// After install only (expansion and replacement): a new dedicated data
    /// disk that `alp-storage` partitions with the data-disk definition.
    /// Never the system disk.
    WholeDisk(DiskId),
    /// An existing `alp-data` partition, created by the installer on any
    /// confirmed disk, or by `alp-storage` on a disk added later.
    ExistingPartition {
        disk: DiskId,
        partition_uuid: String,
    },
}

/// Btrfs allocation profiles allowed by spec block 4. RAID5/6 are forbidden
/// and deliberately not representable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RaidProfile {
    /// Only by an explicit advanced setting.
    Single,
    Dup,
    Raid1,
    Raid1c3,
    Raid10,
}

/// How well the planned pool protects data, as shown to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protection {
    /// One disk, `single`: no protection against bit rot or disk failure.
    None,
    /// One disk, `dup`: self-healing against bit rot, not disk failure.
    BitRotOnly,
    /// Two or more disks: survives the failure of one disk.
    DiskFailure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolPlan {
    pub members: Vec<PoolMember>,
    pub data_profile: RaidProfile,
    pub metadata_profile: RaidProfile,
    /// Capacity the user can store, after redundancy.
    pub usable_bytes: u64,
    pub protection: Protection,
}

/// Identifier of a pool: its Btrfs filesystem UUID, recorded in the pool
/// metadata on `state` when the pool is created (readable without unlocking).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PoolId(pub String);

/// An open pool and where its subvolumes are mounted
/// (paths from `alp_core::state::paths()`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountedPool {
    pub id: PoolId,
    pub mount_point: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpansionPlan {
    pub pool: PoolId,
    pub new_disk: DiskInfo,
    pub usable_bytes_before: u64,
    pub usable_bytes_after: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskReplacement {
    pub pool: PoolId,
    pub old: DiskId,
    pub new_disk: DiskInfo,
}

/// Snapshot retention (spec block 4 defaults: 24 hourly, 14 daily, 8 weekly;
/// tunable in the hardware profile).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotPolicy {
    pub hourly: u32,
    pub daily: u32,
    pub weekly: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrubHandle {
    pub pool: PoolId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolState {
    Healthy,
    /// Redundancy lost (e.g. a disk missing); data still readable.
    Degraded,
    /// Pool forced read-only (e.g. USB disk disconnected).
    ReadOnly,
    /// Pool could not be opened; the system stays up (spec Section 6).
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolHealth {
    pub state: PoolState,
    pub used_bytes: u64,
    pub usable_bytes: u64,
    pub missing_disks: Vec<DiskId>,
    /// Error codes from `errors/catalog.toml` currently raised for the pool.
    pub alerts: Vec<&'static str>,
}

/// Events from the storage backend, consumed by `alpd`'s health
/// aggregation (spec block 4 Interfaces).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageEvent {
    PoolDegraded {
        pool: PoolId,
    },
    DiskMissing {
        pool: PoolId,
        disk: DiskId,
    },
    ScrubFinished {
        pool: PoolId,
        errors_found: u64,
        errors_corrected: u64,
    },
    CapacityLow {
        pool: PoolId,
        free_bytes: u64,
    },
}

/// Encrypted Btrfs data pool (block 4). v1: `BtrfsBackend` in `alp-storage`.
/// Destructive operations take [`Confirmed`] values (spec Section 2 rule 8).
pub trait StorageBackend: Send + Sync {
    fn discover_disks(&self) -> Result<Vec<DiskInfo>>;
    fn plan_pool(&self, disks: &[DiskInfo]) -> Result<PoolPlan>;
    fn create_pool(&self, plan: &Confirmed<PoolPlan>, key: &PoolKey) -> Result<PoolId>;
    fn open_pool(&self, id: PoolId, key: &PoolKey) -> Result<MountedPool>;
    fn propose_expansion(&self, id: PoolId, new_disk: &DiskInfo) -> Result<ExpansionPlan>;
    fn apply_expansion(&self, plan: &Confirmed<ExpansionPlan>) -> Result<()>;
    fn replace_disk(&self, replacement: &Confirmed<DiskReplacement>) -> Result<()>;
    fn snapshot(&self, id: PoolId, policy: &SnapshotPolicy) -> Result<()>;
    fn scrub(&self, id: PoolId) -> Result<ScrubHandle>;
    fn health(&self, id: PoolId) -> Result<PoolHealth>;
    /// Register the receiver of [`StorageEvent`]s.
    fn on_event(&self, handler: Box<dyn Fn(StorageEvent) + Send + Sync>);
}

// --------------------------------------------------------------- power

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PowerStatus {
    pub ups_present: bool,
    pub on_battery: bool,
    pub battery_percent: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerEvent {
    OnBattery,
    BatteryLow,
    PowerRestored,
}

/// Power and UPS (block 10). v1: [`GenericPcPower`] (NUT-aware).
pub trait PowerBackend: Send + Sync {
    fn status(&self) -> PowerStatus;
    fn on_event(&self, handler: Box<dyn Fn(PowerEvent) + Send + Sync>);
}

// ------------------------------------------------- v1 implementations

/// v1 boot backend: UEFI + systemd-boot + systemd-sysupdate (block 2). Stub.
#[derive(Debug, Default)]
pub struct UefiSystemdBoot;

impl BootBackend for UefiSystemdBoot {
    fn current_slot(&self) -> Slot {
        todo!("block 2")
    }
    fn stage_update(&self, _image: &VerifiedImage) -> Result<()> {
        todo!("block 2")
    }
    fn mark_good(&self) -> Result<()> {
        todo!("block 2")
    }
    fn rollback_pending(&self) -> bool {
        todo!("block 2")
    }
}

/// v1 power backend: generic PC, NUT when a UPS is detected (block 10). Stub.
#[derive(Debug, Default)]
pub struct GenericPcPower;

impl PowerBackend for GenericPcPower {
    fn status(&self) -> PowerStatus {
        todo!("block 10")
    }
    fn on_event(&self, _handler: Box<dyn Fn(PowerEvent) + Send + Sync>) {
        todo!("block 10")
    }
}

// ------------------------------------------------- plan digests (block 4)

impl PlanDigest for PoolPlan {
    fn plan_digest(&self) -> [u8; 32] {
        todo!("block 4: canonical SHA3-256 of the plan")
    }
}

impl PlanDigest for ExpansionPlan {
    fn plan_digest(&self) -> [u8; 32] {
        todo!("block 4: canonical SHA3-256 of the plan")
    }
}

impl PlanDigest for DiskReplacement {
    fn plan_digest(&self) -> [u8; 32] {
        todo!("block 4: canonical SHA3-256 of the plan")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Plan(u8);
    impl PlanDigest for Plan {
        fn plan_digest(&self) -> [u8; 32] {
            [self.0; 32]
        }
    }

    #[test]
    fn confirmed_only_for_the_plan_the_user_saw() {
        let evidence = ConfirmationEvidence {
            ledger_seq: 7,
            plan_digest: [1; 32],
        };
        assert!(Confirmed::by_user(Plan(1), evidence).is_ok());
        let err = Confirmed::by_user(Plan(2), evidence).unwrap_err();
        assert_eq!(err.code, "ALP-STO-007");
    }
}
