// SPDX-License-Identifier: AGPL-3.0-or-later
//! The first-boot seed file (contract `docs/contracts/firstboot-seed.md`,
//! type `alp_core::firstboot::FirstbootSeed`) as TOML. The contract leaves
//! serialization to streams C and D; this writes the contract's example
//! shape. Contains no secret (disk names, sizes, a digest).

use alp_core::firstboot::FirstbootSeed;
use toml::{Table, Value};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The seed as the TOML text written to `paths().firstboot_seed`.
pub fn to_toml(seed: &FirstbootSeed) -> String {
    let mut t = Table::new();
    let int = |v: u64| Value::Integer(i64::try_from(v).unwrap_or(i64::MAX));
    t.insert("format_version".into(), int(u64::from(seed.format_version)));
    t.insert(
        "installer_version".into(),
        Value::String(seed.installer_version.clone()),
    );
    t.insert("profile".into(), Value::String(seed.profile.clone()));
    t.insert(
        "protection_tier".into(),
        Value::String(seed.protection_tier.clone()),
    );
    t.insert(
        "confirmed_at".into(),
        Value::String(seed.confirmed_at.clone()),
    );
    t.insert("plan_digest".into(), Value::String(hex(&seed.plan_digest)));
    let members = seed
        .members
        .iter()
        .map(|m| {
            let mut e = Table::new();
            e.insert("by_id".into(), Value::String(m.by_id.clone()));
            if let Some(s) = &m.serial {
                e.insert("serial".into(), Value::String(s.clone()));
            }
            e.insert("size_bytes".into(), int(m.size_bytes));
            e.insert(
                "partition_uuid".into(),
                Value::String(m.partition_uuid.clone()),
            );
            Value::Table(e)
        })
        .collect();
    t.insert("members".into(), Value::Array(members));
    t.to_string()
}

/// `confirmed_at`: seconds since 1970 as RFC 3339 in UTC
/// (`2026-11-24T09:30:00Z`), civil date from days (proleptic Gregorian).
pub fn rfc3339_utc(unix_secs: u64) -> String {
    let days = i64::try_from(unix_secs / 86_400).unwrap_or(0);
    let rem = unix_secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        rem / 60 % 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use alp_core::firstboot::{SEED_FORMAT_VERSION, SeedMember};

    #[test]
    fn seed_has_the_contract_fields() {
        let seed = FirstbootSeed {
            format_version: SEED_FORMAT_VERSION,
            installer_version: "0.2.8".into(),
            profile: "generic-x86_64".into(),
            protection_tier: "standard".into(),
            members: vec![SeedMember {
                by_id: "ata-WDC_WUH722020BLE6L4_X".into(),
                serial: Some("X".into()),
                size_bytes: 20_000_000_000_000,
                partition_uuid: "1b4e28ba-2fa1-11d2-883f-0016d3cca427".into(),
            }],
            confirmed_at: "2026-11-24T09:30:00Z".into(),
            plan_digest: [0xab; 32],
        };
        let text = to_toml(&seed);
        let back: Table = text.parse().expect("valid TOML");
        assert_eq!(back["format_version"].as_integer(), Some(1));
        assert_eq!(back["plan_digest"].as_str(), Some("ab".repeat(32).as_str()));
        let m = &back["members"].as_array().unwrap()[0];
        assert_eq!(m["size_bytes"].as_integer(), Some(20_000_000_000_000));
        assert_eq!(
            m["partition_uuid"].as_str(),
            Some("1b4e28ba-2fa1-11d2-883f-0016d3cca427")
        );
        assert_eq!(m["serial"].as_str(), Some("X"));
    }

    #[test]
    fn dates_are_rfc3339_utc() {
        assert_eq!(rfc3339_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339_utc(1_795_512_600), "2026-11-24T09:30:00Z");
        assert_eq!(rfc3339_utc(4_102_444_799), "2099-12-31T23:59:59Z");
    }
}
