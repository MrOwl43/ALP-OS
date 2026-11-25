// SPDX-License-Identifier: AGPL-3.0-or-later
//! First-boot seed contract (ADR 0004, docs/contracts/firstboot-seed.md).
//!
//! Written by `alp-installer` to `paths().firstboot_seed` on the installed
//! `state` partition; read and then deleted by the first start when the
//! pool exists. TOML, written and read only here so both sides agree.
//! Contains no secret (disk names, sizes, a digest).

use alp_hw::{Protection, RaidProfile};
use toml::{Table, Value};

/// One pool member as the user confirmed it in the installer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedMember {
    /// `/dev/disk/by-id` name at install time (display, first lookup).
    pub by_id: String,
    pub serial: Option<String>,
    pub size_bytes: u64,
    /// GPT partition UUID of the member's `alp-data` partition (created by
    /// the installer on dedicated data disks too, so it is known here).
    pub partition_uuid: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirstbootSeed {
    /// Schema version of this file; first boot refuses unknown versions.
    pub format_version: u32,
    pub installer_version: String,
    /// Hardware profile name, e.g. `generic-x86_64`.
    pub profile: String,
    /// `standard` in the beta.
    pub protection_tier: String,
    /// The pool the user confirmed on the installer's Confirm screen.
    pub members: Vec<SeedMember>,
    /// The rest of the confirmed `PoolPlan`, so first boot rebuilds exactly
    /// the plan whose digest is below.
    pub data_profile: RaidProfile,
    pub metadata_profile: RaidProfile,
    pub usable_bytes: u64,
    pub protection: Protection,
    /// When the user confirmed (RFC 3339, UTC).
    pub confirmed_at: String,
    /// `alp_hw::PlanDigest` of the confirmed pool plan.
    pub plan_digest: [u8; 32],
}

/// Current seed schema version.
pub const SEED_FORMAT_VERSION: u32 = 1;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

impl FirstbootSeed {
    /// The seed as TOML (the contract's example shape).
    pub fn to_toml(&self) -> String {
        let mut t = Table::new();
        let int = |v: u64| Value::Integer(i64::try_from(v).unwrap_or(i64::MAX));
        let s = |v: &str| Value::String(v.to_string());
        t.insert("format_version".into(), int(u64::from(self.format_version)));
        t.insert("installer_version".into(), s(&self.installer_version));
        t.insert("profile".into(), s(&self.profile));
        t.insert("protection_tier".into(), s(&self.protection_tier));
        t.insert("data_profile".into(), s(self.data_profile.name()));
        t.insert("metadata_profile".into(), s(self.metadata_profile.name()));
        t.insert("usable_bytes".into(), int(self.usable_bytes));
        t.insert("protection".into(), s(self.protection.name()));
        t.insert("confirmed_at".into(), s(&self.confirmed_at));
        t.insert("plan_digest".into(), s(&hex(&self.plan_digest)));
        let members = self
            .members
            .iter()
            .map(|m| {
                let mut e = Table::new();
                e.insert("by_id".into(), s(&m.by_id));
                if let Some(serial) = &m.serial {
                    e.insert("serial".into(), s(serial));
                }
                e.insert("size_bytes".into(), int(m.size_bytes));
                e.insert("partition_uuid".into(), s(&m.partition_uuid));
                Value::Table(e)
            })
            .collect();
        t.insert("members".into(), Value::Array(members));
        t.to_string()
    }

    /// Read a seed. `Err` names what is wrong (contract rule 1: an unknown
    /// `format_version` is refused).
    pub fn from_toml(text: &str) -> Result<Self, String> {
        let t: Table = text.parse().map_err(|e| format!("not TOML: {e}"))?;
        let str_of = |t: &Table, k: &str| -> Result<String, String> {
            t.get(k)
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| format!("missing {k}"))
        };
        let u64_of = |t: &Table, k: &str| -> Result<u64, String> {
            t.get(k)
                .and_then(Value::as_integer)
                .and_then(|v| u64::try_from(v).ok())
                .ok_or_else(|| format!("missing or negative {k}"))
        };
        let version = u64_of(&t, "format_version")?;
        if version != u64::from(SEED_FORMAT_VERSION) {
            return Err(format!("unknown format_version {version}"));
        }
        let profile = |k: &str| {
            str_of(&t, k).and_then(|v| RaidProfile::parse(&v).ok_or(format!("bad {k} {v:?}")))
        };
        let members = t
            .get("members")
            .and_then(Value::as_array)
            .ok_or("missing members")?
            .iter()
            .map(|m| {
                let m = m.as_table().ok_or("a member is not a table")?;
                Ok(SeedMember {
                    by_id: str_of(m, "by_id")?,
                    serial: m.get("serial").and_then(Value::as_str).map(str::to_string),
                    size_bytes: u64_of(m, "size_bytes")?,
                    partition_uuid: str_of(m, "partition_uuid")?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        if members.is_empty() {
            return Err("no members".into());
        }
        let protection = str_of(&t, "protection")?;
        Ok(Self {
            format_version: SEED_FORMAT_VERSION,
            installer_version: str_of(&t, "installer_version")?,
            profile: str_of(&t, "profile")?,
            protection_tier: str_of(&t, "protection_tier")?,
            members,
            data_profile: profile("data_profile")?,
            metadata_profile: profile("metadata_profile")?,
            usable_bytes: u64_of(&t, "usable_bytes")?,
            protection: Protection::parse(&protection)
                .ok_or(format!("bad protection {protection:?}"))?,
            confirmed_at: str_of(&t, "confirmed_at")?,
            plan_digest: unhex32(&str_of(&t, "plan_digest")?).ok_or("bad plan_digest")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn sample() -> FirstbootSeed {
        FirstbootSeed {
            format_version: SEED_FORMAT_VERSION,
            installer_version: "0.2.8".into(),
            profile: "generic-x86_64".into(),
            protection_tier: "standard".into(),
            members: vec![
                SeedMember {
                    by_id: "ata-WDC_WUH722020BLE6L4_X".into(),
                    serial: Some("X".into()),
                    size_bytes: 20_000_000_000_000,
                    partition_uuid: "1b4e28ba-2fa1-11d2-883f-0016d3cca427".into(),
                },
                SeedMember {
                    by_id: "virtio-ALPDATA02".into(),
                    serial: None,
                    size_bytes: 68_719_476_736,
                    partition_uuid: "2b4e28ba-2fa1-11d2-883f-0016d3cca427".into(),
                },
            ],
            data_profile: RaidProfile::Raid1,
            metadata_profile: RaidProfile::Raid1,
            usable_bytes: 68_000_000_000,
            protection: Protection::DiskFailure,
            confirmed_at: "2026-11-24T09:30:00Z".into(),
            plan_digest: [0xab; 32],
        }
    }

    #[test]
    fn round_trip() {
        let s = sample();
        assert_eq!(FirstbootSeed::from_toml(&s.to_toml()), Ok(s));
    }

    #[test]
    fn has_the_contract_fields() {
        let back: Table = sample().to_toml().parse().unwrap();
        assert_eq!(back["format_version"].as_integer(), Some(1));
        assert_eq!(back["data_profile"].as_str(), Some("raid1"));
        assert_eq!(back["protection"].as_str(), Some("disk-failure"));
        assert_eq!(back["plan_digest"].as_str(), Some("ab".repeat(32).as_str()));
        let m = &back["members"].as_array().unwrap()[0];
        assert_eq!(m["size_bytes"].as_integer(), Some(20_000_000_000_000));
        assert_eq!(m["serial"].as_str(), Some("X"));
    }

    #[test]
    fn refuses_bad_seeds() {
        let good = sample().to_toml();
        for (from, to) in [
            ("format_version = 1", "format_version = 2"),
            ("data_profile = \"raid1\"", "data_profile = \"raid5\""),
            ("protection = \"disk-failure\"", "protection = \"lots\""),
            (&*format!("\"{}\"", "ab".repeat(32)), "\"abcd\""),
            ("size_bytes = 68719476736", "size_bytes = -1"),
        ] {
            assert!(good.contains(from), "{from}");
            let bad = good.replacen(from, to, 1);
            assert!(FirstbootSeed::from_toml(&bad).is_err(), "{to}");
        }
        assert!(FirstbootSeed::from_toml("format_version = 1\nmembers = []").is_err());
        assert!(FirstbootSeed::from_toml("not toml [").is_err());
    }
}
