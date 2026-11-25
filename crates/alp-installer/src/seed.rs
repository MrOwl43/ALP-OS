// SPDX-License-Identifier: AGPL-3.0-or-later
//! The first-boot seed file (contract `docs/contracts/firstboot-seed.md`).
//! The TOML format lives with the type, `alp_core::firstboot::FirstbootSeed`,
//! so the installer and the first start share one writer and reader.

use alp_core::firstboot::FirstbootSeed;

/// The seed as the TOML text written to `paths().firstboot_seed`
/// (`FirstbootSeed::to_toml`, the one writer; its reader sits beside it).
pub fn to_toml(seed: &FirstbootSeed) -> String {
    seed.to_toml()
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

    #[test]
    fn dates_are_rfc3339_utc() {
        assert_eq!(rfc3339_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339_utc(1_795_512_600), "2026-11-24T09:30:00Z");
        assert_eq!(rfc3339_utc(4_102_444_799), "2099-12-31T23:59:59Z");
    }
}
