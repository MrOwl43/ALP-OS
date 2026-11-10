// SPDX-License-Identifier: AGPL-3.0-or-later
//! `BtrfsBackend` (the v1 `StorageBackend` from `alp-hw`), disk discovery, SMART.
//!
//! Only this crate may invoke `btrfs`, `cryptsetup`, `mkfs` and `smartctl`
//! (spec Section 2 rule 6). Every pool member must pass
//! `alp_hw::safety::check_member` right before anything is written. The pool
//! key is handed to `cryptsetup` only through `PoolKey::write_key_file`.
//! Implementation arrives with block 4.

use alp_hw::{
    Confirmed, DiskInfo, DiskReplacement, ExpansionPlan, MountedPool, PoolHealth, PoolId, PoolKey,
    PoolPlan, Result, ScrubHandle, SnapshotPolicy, StorageBackend, StorageEvent,
};

/// v1 storage backend: Btrfs on LUKS2, one container per pool member. Stub.
#[derive(Debug, Default)]
pub struct BtrfsBackend;

impl StorageBackend for BtrfsBackend {
    fn discover_disks(&self) -> Result<Vec<DiskInfo>> {
        todo!("block 4")
    }
    fn plan_pool(&self, _disks: &[DiskInfo]) -> Result<PoolPlan> {
        todo!("block 4")
    }
    fn create_pool(&self, _plan: &Confirmed<PoolPlan>, _key: &PoolKey) -> Result<PoolId> {
        todo!("block 4")
    }
    fn open_pool(&self, _id: PoolId, _key: &PoolKey) -> Result<MountedPool> {
        todo!("block 4")
    }
    fn propose_expansion(&self, _id: PoolId, _new_disk: &DiskInfo) -> Result<ExpansionPlan> {
        todo!("block 4")
    }
    fn apply_expansion(&self, _plan: &Confirmed<ExpansionPlan>) -> Result<()> {
        todo!("block 4")
    }
    fn replace_disk(&self, _replacement: &Confirmed<DiskReplacement>) -> Result<()> {
        todo!("block 4")
    }
    fn snapshot(&self, _id: PoolId, _policy: &SnapshotPolicy) -> Result<()> {
        todo!("block 4")
    }
    fn scrub(&self, _id: PoolId) -> Result<ScrubHandle> {
        todo!("block 4")
    }
    fn health(&self, _id: PoolId) -> Result<PoolHealth> {
        todo!("block 4")
    }
    fn on_event(&self, _handler: Box<dyn Fn(StorageEvent) + Send + Sync>) {
        todo!("block 4")
    }
}
