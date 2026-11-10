<!-- Generated from docs/spec.md (Version 0.5) by tools/split-spec.py. Do not edit; edit docs/spec.md and rerun the tool. -->

## 5. Abstraction layer: traits and hardware profile

Four traits isolate hardware and platform differences. v1 ships exactly one implementation of each (two for key sealing). Future hardware adds implementations, not changes to callers.

```rust
pub trait BootBackend {            // v1: UefiSystemdBoot
    fn current_slot(&self) -> Slot;
    fn stage_update(&self, image: &VerifiedImage) -> Result<()>;
    fn mark_good(&self) -> Result<()>;
    fn rollback_pending(&self) -> bool;
}
pub trait KeySealBackend {         // v1: Tpm2Seal, PassphraseOrKeyfileSeal
    fn available(&self) -> SealCapability;
    fn seal(&self, secret: &Secret) -> Result<SealedBlob>;
    fn unseal(&self, blob: &SealedBlob) -> Result<Secret>;
}
pub trait StorageBackend {         // v1: BtrfsBackend
    fn discover_disks(&self) -> Result<Vec<DiskInfo>>;
    fn plan_pool(&self, disks: &[DiskInfo]) -> Result<PoolPlan>;
    fn create_pool(&self, plan: &PoolPlan, key: &PoolKey) -> Result<PoolId>;
    fn open_pool(&self, id: PoolId, key: &PoolKey) -> Result<MountedPool>;
    fn propose_expansion(&self, id: PoolId, new_disk: &DiskInfo) -> Result<ExpansionPlan>;
    fn apply_expansion(&self, plan: &ExpansionPlan) -> Result<()>;   // needs user confirmation token
    fn replace_disk(&self, id: PoolId, old: DiskId, new: &DiskInfo) -> Result<()>;
    fn snapshot(&self, id: PoolId, policy: &SnapshotPolicy) -> Result<()>;
    fn scrub(&self, id: PoolId) -> Result<ScrubHandle>;
    fn health(&self, id: PoolId) -> Result<PoolHealth>;
}
pub trait PowerBackend {           // v1: GenericPcPower (NUT-aware if a UPS is detected)
    fn status(&self) -> PowerStatus;
    fn on_event(&self, handler: Box<dyn Fn(PowerEvent) + Send>);
}
```

A hardware profile is a declarative TOML file selecting the implementation for each trait and setting numeric defaults (ARC-style limits do not apply to Btrfs, but scrub throttling, snapshot retention and container quotas do). v1 has only `generic-x86_64.toml`. Never scatter `if hardware == ...` checks in code.
