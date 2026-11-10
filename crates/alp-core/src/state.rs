// SPDX-License-Identifier: AGPL-3.0-or-later
//! State paths contract (spec block 3, ADR 0004).
//!
//! [`paths()`] is the only source of filesystem paths for persistent state,
//! runtime state and the data pool. No other code may hard-code these
//! paths. The `state` partition (label `alp-state`,
//! docs/contracts/disk-layout.md) is mounted at [`StatePaths::root`].

use std::path::{Path, PathBuf};

/// Canonical mount point of the `state` partition (spec block 3).
pub const STATE_ROOT: &str = "/var/lib/alp";
/// Runtime directory (tmpfs) for sockets and transient files.
pub const RUNTIME_ROOT: &str = "/run/alp";
/// Mount point of the data pool's top level (block 4). Subvolumes are
/// mounted below it. `/var` is a tmpfs, so systemd creates the directory.
pub const POOL_ROOT: &str = "/var/lib/alp-pool";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatePaths {
    /// Mount point of the `state` partition.
    pub root: PathBuf,
    /// `alpd` SQLite database (WAL mode), including the audit ledger table.
    pub database: PathBuf,
    /// Wrapped keys and sealed blobs (block 5). Mode 0700, owner alpd.
    pub keys: PathBuf,
    /// Node configuration (TOML).
    pub config: PathBuf,
    /// Pool metadata: member partition UUIDs, pool id, chosen profile (block 4).
    pub pool_metadata: PathBuf,
    /// Local CA and server certificate (block 9).
    pub tls: PathBuf,
    /// Upper and work directories of the `/etc` overlay (block 3).
    pub etc_overlay_upper: PathBuf,
    pub etc_overlay_work: PathBuf,
    /// Service secrets (Immich admin API key, PostgreSQL password), each
    /// stored as a `SealedBlob` (block 5), never in plain text.
    /// Mode 0700, owner alpd.
    pub service_secrets: PathBuf,
    /// Written by the installer for first boot (profile choice, disk ids,
    /// protection tier); deleted by `alpd` when first boot completes.
    pub firstboot_seed: PathBuf,
    /// Runtime directory (not persistent).
    pub runtime: PathBuf,
    /// `alpd` API unix socket: the same HTTP API as `api/alpd.openapi.yaml`
    /// (`/api/v1/...`), mode 0600, owner root. Used by `alpctl` and by the
    /// health check ("alpd responding on its socket", block 2).
    pub api_socket: PathBuf,
    /// The data pool (spec block 4).
    pub pool: PoolPaths,
}

/// Data pool top level and its subvolumes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolPaths {
    pub root: PathBuf,
    /// `@archive`: user data (the Immich library).
    pub archive: PathBuf,
    /// `@snapshots`: read-only snapshots.
    pub snapshots: PathBuf,
    /// `@system-backup`: encrypted `state` backups and the ledger mirror.
    pub system_backup: PathBuf,
    /// `@apps`: application databases and container storage (Immich
    /// PostgreSQL and Redis), snapshotted before any migration (spec 2.1).
    pub apps: PathBuf,
}

/// The production state paths.
pub fn paths() -> StatePaths {
    paths_under(
        Path::new(STATE_ROOT),
        Path::new(RUNTIME_ROOT),
        Path::new(POOL_ROOT),
    )
}

/// State paths rooted elsewhere, for tests.
pub fn paths_under(state_root: &Path, runtime_root: &Path, pool_root: &Path) -> StatePaths {
    StatePaths {
        root: state_root.to_path_buf(),
        database: state_root.join("alpd.sqlite"),
        keys: state_root.join("keys"),
        config: state_root.join("config.toml"),
        pool_metadata: state_root.join("pool"),
        tls: state_root.join("tls"),
        etc_overlay_upper: state_root.join("etc-overlay/upper"),
        etc_overlay_work: state_root.join("etc-overlay/work"),
        service_secrets: state_root.join("secrets"),
        firstboot_seed: state_root.join("firstboot-seed.toml"),
        runtime: runtime_root.to_path_buf(),
        api_socket: runtime_root.join("alpd.sock"),
        pool: PoolPaths {
            root: pool_root.to_path_buf(),
            archive: pool_root.join("archive"),
            snapshots: pool_root.join("snapshots"),
            system_backup: pool_root.join("system-backup"),
            apps: pool_root.join("apps"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn persistent(p: &StatePaths) -> Vec<&PathBuf> {
        vec![
            &p.database,
            &p.keys,
            &p.config,
            &p.pool_metadata,
            &p.tls,
            &p.etc_overlay_upper,
            &p.etc_overlay_work,
            &p.service_secrets,
            &p.firstboot_seed,
        ]
    }

    fn on_pool(p: &StatePaths) -> Vec<&PathBuf> {
        vec![
            &p.pool.archive,
            &p.pool.snapshots,
            &p.pool.system_backup,
            &p.pool.apps,
        ]
    }

    #[test]
    fn each_path_lives_where_the_contract_says() {
        let p = paths();
        for path in persistent(&p) {
            assert!(
                path.starts_with(STATE_ROOT),
                "{} not on state",
                path.display()
            );
        }
        for path in on_pool(&p) {
            assert!(
                path.starts_with(POOL_ROOT),
                "{} not on the pool",
                path.display()
            );
        }
        assert!(p.api_socket.starts_with(RUNTIME_ROOT));
        assert!(!Path::new(POOL_ROOT).starts_with(STATE_ROOT));
    }

    #[test]
    fn paths_are_distinct() {
        let p = paths();
        let mut all = persistent(&p);
        all.extend(on_pool(&p));
        all.extend([&p.api_socket, &p.pool.root, &p.root, &p.runtime]);
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }
}
