// SPDX-License-Identifier: AGPL-3.0-or-later
//! The disk manager's rules (T58, ADR 0022): how ALP-OS Live classes each
//! disk, and the one place that says which disks Live may ever change.
//!
//! - **Stick:** the disk the running system booted from.
//! - **External:** a whole physical disk on the USB bus that is not the
//!   stick. The only class Live ever writes, formats, checks or opens.
//! - **Internal:** everything else: NVMe, SATA, MMC/eMMC and SD readers on
//!   the board, virtio, Thunderbolt, eSATA, any unknown transport, and any
//!   disk whose facts could not be read. When unsure, a disk is Internal.
//!
//! Virtual block devices (loop, dm, md, zram, nbd, ram) are never disks
//! here at all (review R1: Live itself runs on loop and dm devices).
//! Partitions take the class of their disk. If the stick is not found,
//! every disk is Internal ([`classes`]): nothing is offered for writing.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use alp_hw::{DiskInfo, DiskTransport};
use serde_json::{Value, json};

use crate::archive::{self, Part};
use crate::job::{End, Progress, RunError, Update};

/// The stick or an internal disk was asked to change (`403 disk.locked`).
pub const CODE_LOCKED: &str = "ALP-STO-124";
/// The read-only disk check could not run to the end.
pub const CODE_CHECK: &str = "ALP-STO-125";

/// What Live may do with a disk (ADR 0022).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Stick,
    External,
    Internal,
}

impl Class {
    /// The `kind` the API shows (`GET live/disks`).
    pub fn kind(self) -> &'static str {
        match self {
            Class::Stick => "stick",
            Class::External => "external",
            Class::Internal => "internal",
        }
    }
}

/// Kernel names of virtual block devices: never a disk Live may use, even
/// if a future kernel gives one a `device` link (review R1).
pub fn is_virtual(name: &str) -> bool {
    ["loop", "dm-", "md", "zram", "nbd", "ram"]
        .iter()
        .any(|p| name.starts_with(p))
}

/// The class of one disk, given whether the stick was found at all.
pub fn class_of(d: &DiskInfo, stick_found: bool) -> Class {
    if d.is_system_disk {
        return Class::Stick;
    }
    if stick_found && !is_virtual(&d.id.0) && d.transport == DiskTransport::Usb {
        Class::External
    } else {
        Class::Internal
    }
}

/// Every disk of the report with its class. Without the stick found, no
/// disk is External (fail closed: the stick itself would look like an
/// external USB disk; QA review 0.2.5).
pub fn classes(disks: &[DiskInfo]) -> Vec<(&DiskInfo, Class)> {
    let stick_found = disks.iter().any(|d| d.is_system_disk);
    if !stick_found {
        eprintln!("alpd: the system stick was not found; every disk is treated as internal");
    }
    disks
        .iter()
        .filter(|d| !is_virtual(&d.id.0))
        .map(|d| (d, class_of(d, stick_found)))
        .collect()
}

/// Is the disk with kernel name `disk` External right now? Looked up again
/// from the system each time (the screen's word is never trusted).
pub fn is_external(disk: &str) -> bool {
    let r = alp_hw::inspect::inspect(&alp_hw::inspect::Roots::system());
    classes(&r.disks)
        .iter()
        .any(|(d, c)| d.id.0 == disk && *c == Class::External)
}

/// What a disk's own health check (SMART) says, for the list (T58).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Smart {
    /// `healthy`, `wear` or `failing`; None when the disk does not answer
    /// (a stick, a USB bridge without SMART, a disk asleep).
    pub health: Option<&'static str>,
    pub temperature_c: Option<i64>,
}

/// `smartctl -j -H -A -n standby` output and exit code, read (pure).
/// Failing: the disk says so (exit bits 3-4, or `passed: false`). Wear:
/// an NVMe disk 80 % used or with a critical warning, or an ATA disk with
/// reallocated, pending or uncorrectable sectors.
pub fn parse_smart(out: &str, code: Option<i32>) -> Smart {
    let Some(code) = code else {
        return Smart::default();
    };
    // Bits 0-1: bad command line, or the device could not be opened (also
    // "in standby" with -n standby): no answer.
    if code & 0b11 != 0 {
        return Smart::default();
    }
    let v: Value = serde_json::from_str(out).unwrap_or(Value::Null);
    let temperature_c = v["temperature"]["current"]
        .as_i64()
        .filter(|t| (-40..=150).contains(t));
    let passed = v["smart_status"]["passed"].as_bool();
    let health = if code & 0b1_1000 != 0 || passed == Some(false) {
        Some("failing")
    } else if passed.is_none() {
        None
    } else {
        let nvme = &v["nvme_smart_health_information_log"];
        let nvme_wear = nvme["percentage_used"].as_u64().is_some_and(|p| p >= 80)
            || nvme["critical_warning"].as_u64().is_some_and(|w| w != 0);
        let ata_wear = v["ata_smart_attributes"]["table"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|a| {
                matches!(a["id"].as_u64(), Some(5 | 197 | 198))
                    && a["raw"]["value"].as_u64().is_some_and(|r| r > 0)
            });
        Some(if nvme_wear || ata_wear {
            "wear"
        } else {
            "healthy"
        })
    };
    Smart {
        health,
        temperature_c,
    }
}

/// SMART answers, kept about a minute per disk id (review: not on every
/// list call).
static SMART_CACHE: Mutex<Option<HashMap<String, (Instant, Smart)>>> = Mutex::new(None);
const SMART_TTL: Duration = Duration::from_secs(60);

/// Ask the disk (never waking one that sleeps; 5 s at most; a USB disk is
/// asked again through SAT).
fn smart(name: &str, key: &str, usb: bool) -> Smart {
    if let Some((at, s)) = SMART_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_or_insert_with(HashMap::new)
        .get(key)
        && at.elapsed() < SMART_TTL
    {
        return *s;
    }
    let dev = format!("/dev/{name}");
    let no = AtomicBool::new(false);
    let ask = |sat: bool| {
        let mut a = vec!["-j", "-H", "-A", "-n", "standby"];
        if sat {
            a.extend(["-d", "sat"]);
        }
        a.push(&dev);
        match crate::job::run_code("smartctl", &a, &no, Some(Duration::from_secs(5))) {
            Ok((code, out)) => parse_smart(&out, code),
            Err(_) => Smart::default(),
        }
    };
    let mut s = ask(false);
    if s.health.is_none() && usb {
        s = ask(true);
    }
    SMART_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_or_insert_with(HashMap::new)
        .insert(key.to_string(), (Instant::now(), s));
    s
}

/// The archive disk's own health (SMART), or None when there is no
/// archive, its disk is not here, or the disk does not answer.
pub fn archive_disk_health() -> Option<&'static str> {
    let rec = archive::record()?;
    let part = archive::partitions()
        .into_iter()
        .find(|p| p.partuuid.eq_ignore_ascii_case(&rec.partuuid))?;
    let r = alp_hw::inspect::inspect(&alp_hw::inspect::Roots::system());
    let d = r.disks.iter().find(|d| d.id.0 == part.disk)?;
    smart(
        &part.disk,
        &archive::disk_key(&part.disk),
        d.transport == DiskTransport::Usb,
    )
    .health
}

/// Why a disk cannot be changed now, most important first: the stick, an
/// internal disk (Live), the archive's disk, write-protected, open.
pub fn lock_reason(
    class: Class,
    backup: bool,
    write_protected: bool,
    open: bool,
) -> Option<&'static str> {
    match class {
        Class::Stick => Some("stick"),
        Class::Internal => Some("internal_live"),
        Class::External if backup => Some("backup"),
        Class::External if write_protected => Some("write_protected"),
        Class::External if open => Some("open"),
        Class::External => None,
    }
}

/// One partition as `live/disks` shows it (no UUIDs but the PARTUUID the
/// format call needs; labels made safe for the screen).
fn part_json(p: &Part) -> Value {
    json!({
        "partuuid": p.partuuid,
        "name": p.name,
        "label": crate::fail::display_safe(&p.label, 64),
        "fs": p.fs_type,
        "size_bytes": p.size_bytes,
    })
}

/// The alert level of a disk's health (spec block 10 disk mapping, ADR
/// 0023): healthy is none, showing wear is `action`, failing is `now`, or
/// `critical` when the failing disk holds the archive.
pub fn health_level(health: Option<&str>, backup: bool) -> Option<&'static str> {
    match health {
        Some("wear") => Some("action"),
        Some("failing") if backup => Some("critical"),
        Some("failing") => Some("now"),
        _ => None,
    }
}

/// `GET live/disks` (T58): every disk with its class, role, lock and
/// health; never serial numbers or WWNs (review). Internal disks show
/// their partitions only so the person can recognise the disk; nothing on
/// them is mounted or read.
pub fn list_json() -> Value {
    let r = alp_hw::inspect::inspect(&alp_hw::inspect::Roots::system());
    let parts = archive::partitions();
    let backup_part = archive::record().map(|r| r.partuuid);
    let disks: Vec<Value> = classes(&r.disks)
        .into_iter()
        .map(|(d, class)| {
            let name = d.id.0.as_str();
            let key = archive::disk_key(name);
            let mine: Vec<&Part> = parts.iter().filter(|p| p.disk == name).collect();
            let backup = backup_part
                .as_deref()
                .is_some_and(|b| mine.iter().any(|p| p.partuuid.eq_ignore_ascii_case(b)));
            let open = archive::disk_in_use(name);
            let write_protected = archive::hw_read_only(name);
            let role = match (class, backup) {
                (Class::Stick, _) => "system",
                (_, true) => "backup",
                _ => "other",
            };
            let lock = lock_reason(class, backup, write_protected, open);
            let s = smart(name, &key, d.transport == DiskTransport::Usb);
            let level = health_level(s.health, backup);
            json!({
                "id": key,
                "name": name,
                "model": crate::fail::display_safe(&d.model, 64),
                "size_bytes": d.size_bytes,
                "kind": class.kind(),
                "role": role,
                "locked": lock.is_some(),
                "lock": lock,
                "safe_to_remove": class == Class::External && !open,
                "health": s.health,
                "temperature_c": s.temperature_c,
                "level": level,
                "parts": mine.iter().map(|p| part_json(p)).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({ "disks": disks })
}

/// The read-only check program for a file system, as argv (`dev` is the
/// partition). None: this kind cannot be checked here.
pub fn check_command(fs: &str, dev: &str) -> Option<Vec<String>> {
    let v: &[&str] = match fs {
        "exfat" => &["fsck.exfat", "-n"],
        "ext4" | "ext3" | "ext2" => &["e2fsck", "-n", "-f"],
        "ntfs" => &["ntfsfix", "-n"],
        _ => return None,
    };
    let mut a: Vec<String> = v.iter().map(|s| s.to_string()).collect();
    a.push(dev.to_string());
    Some(a)
}

/// What a check's exit code means, in words (pure). fsck convention: 0
/// clean, 1 or 4 errors found (nothing is changed with -n); ntfsfix -n: 0
/// fine, any other code: needs repair. 8 and up, a signal or no program:
/// the check itself could not run (None).
pub fn check_verdict(fs: &str, code: Option<i32>) -> Option<bool> {
    match (fs, code) {
        (_, Some(0)) => Some(true),
        ("ntfs", Some(c)) if c > 0 && c < 8 => Some(false),
        (_, Some(1 | 4)) => Some(false),
        _ => None,
    }
}

/// systemd-run arguments for one read-only check (review R8): no network,
/// only this partition, read-only, no capabilities, limits; its output
/// comes back on a pipe and is never logged.
pub fn sandbox_args(dev: &str, argv: &[String]) -> Vec<String> {
    let mut a: Vec<String> = [
        "--wait",
        "--pipe",
        "--quiet",
        "--collect",
        "--unit=alp-disk-check",
        "-p",
        "PrivateNetwork=yes",
        "-p",
        "DevicePolicy=closed",
        "-p",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    a.push(format!("DeviceAllow={dev} r"));
    for p in [
        "CapabilityBoundingSet=",
        "NoNewPrivileges=yes",
        "ProtectSystem=strict",
        "ProtectHome=yes",
        "PrivateTmp=yes",
        "ProtectKernelTunables=yes",
        "ProtectKernelModules=yes",
        "RestrictAddressFamilies=AF_UNIX",
        // Review N3.
        "RestrictNamespaces=yes",
        "SystemCallFilter=@system-service",
        "LockPersonality=yes",
        "ProtectProc=invisible",
        "ProtectControlGroups=yes",
        "ProtectKernelLogs=yes",
        "PrivateIPC=yes",
        "MemoryMax=512M",
        "TasksMax=16",
        "RuntimeMaxSec=1800",
    ] {
        a.push("-p".into());
        a.push(p.into());
    }
    a.push("--".into());
    a.extend(argv.iter().cloned());
    a
}

/// The job `disk.check` (T58): a read-only file system check of every
/// partition of an external disk that is not open. Returns the result in
/// words, e.g. "Photos (exFAT): no problems found."
pub fn check_disk(disk_id: &str, cancel: &AtomicBool, say: Progress) -> Result<String, End> {
    let fail = |m: String| End::failed(CODE_CHECK, "checking", m);
    say(&Update::step(
        "checking",
        "Checking the disk (nothing is changed)…",
        2,
    ));
    let name = archive::disk_of_id(disk_id)
        .filter(|n| archive::disk_key(n) == disk_id)
        .ok_or_else(|| fail("the disk is not plugged in any more".into()))?
        .to_string();
    if !is_external(&name) {
        return Err(fail("this is the computer's own disk or the stick".into()));
    }
    if archive::disk_in_use(&name) {
        return Err(fail("the disk is open; eject it in Files first".into()));
    }
    let parts: Vec<Part> = archive::partitions()
        .into_iter()
        .filter(|p| p.disk == name)
        .collect();
    if parts.is_empty() {
        return Err(fail("there is nothing on this disk to check".into()));
    }
    let mut lines = Vec::new();
    let n = parts.len();
    for (i, p) in parts.iter().enumerate() {
        let shown = if p.label.is_empty() {
            p.name.clone()
        } else {
            crate::fail::display_safe(&p.label, 40)
        };
        let kind = p.fs_type.to_ascii_uppercase();
        let pct = 5 + u8::try_from(i * 90 / n).unwrap_or(90);
        say(&Update::step("checking", format!("Checking {shown}…"), pct));
        let dev = format!("/dev/{}", p.name);
        let Some(argv) = check_command(&p.fs_type, &dev) else {
            lines.push(format!(
                "{shown}: this kind of disk cannot be checked here."
            ));
            continue;
        };
        if archive::disk_key(&name) != disk_id {
            return Err(fail("the disk was unplugged".into()));
        }
        let args = sandbox_args(&dev, &argv);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let res = crate::job::run_code(
            "systemd-run",
            &args,
            cancel,
            Some(Duration::from_secs(1800)),
        );
        let code = match res {
            Ok((code, _out)) => code,
            Err(RunError::Cancelled) => {
                stop_check();
                return Err(End::Cancelled);
            }
            Err(_) => {
                stop_check();
                None
            }
        };
        lines.push(match check_verdict(&p.fs_type, code) {
            Some(true) => format!("{shown} ({kind}): no problems found."),
            Some(false) => format!(
                "{shown} ({kind}): problems found. Nothing was changed; copy what you need, then format the disk or repair it on a computer."
            ),
            None => format!("{shown} ({kind}): the check could not finish."),
        });
    }
    say(&Update::step("done", "The check is finished.", 100));
    Ok(lines.join(" "))
}

fn stop_check() {
    let _ = std::process::Command::new("systemctl")
        .args(["stop", "alp-disk-check.service"])
        .output();
}

/// `disks/eject` (T58): an external disk that is not open is synced and
/// switched off (its SCSI device deleted), so it can be pulled out.
pub fn eject(disk_id: &str) -> Result<(), &'static str> {
    let name = archive::disk_of_id(disk_id)
        .filter(|n| archive::disk_key(n) == disk_id)
        .ok_or("not_found")?
        .to_string();
    if !is_external(&name) {
        return Err("locked");
    }
    if archive::disk_in_use(&name) {
        return Err("open");
    }
    let _ = std::process::Command::new("sync").output();
    let _ = std::process::Command::new("blockdev")
        .args(["--flushbufs", &format!("/dev/{name}")])
        .output();
    std::fs::write(
        std::path::Path::new("/sys/block")
            .join(&name)
            .join("device/delete"),
        "1",
    )
    .map_err(|_| "failed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use alp_hw::DiskId;

    fn disk(name: &str, t: DiskTransport, system: bool) -> DiskInfo {
        DiskInfo {
            id: DiskId(name.into()),
            model: "M".into(),
            serial: None,
            size_bytes: 1 << 30,
            transport: t,
            rotational: false,
            removable: false,
            is_system_disk: system,
            partitions: vec![],
        }
    }

    #[test]
    fn only_usb_disks_that_are_not_the_stick_are_external() {
        use DiskTransport::*;
        let ds = vec![
            disk("sda", Usb, true),
            disk("sdb", Usb, false),
            disk("nvme0n1", Nvme, false),
            disk("sdc", Sata, false),
            disk("vdb", Virtio, false),
            disk("mmcblk0", Other, false),
        ];
        let got: Vec<(&str, Class)> = classes(&ds)
            .into_iter()
            .map(|(d, c)| (d.id.0.as_str(), c))
            .collect();
        assert_eq!(
            got,
            [
                ("sda", Class::Stick),
                ("sdb", Class::External),
                ("nvme0n1", Class::Internal),
                ("sdc", Class::Internal),
                ("vdb", Class::Internal),
                ("mmcblk0", Class::Internal),
            ]
        );
    }

    #[test]
    fn without_the_stick_nothing_is_external() {
        let ds = vec![disk("sda", DiskTransport::Usb, false)];
        assert_eq!(classes(&ds)[0].1, Class::Internal);
    }

    #[test]
    fn a_removable_disk_is_not_external_by_that_alone() {
        let mut sd = disk("mmcblk0", DiskTransport::Other, false);
        sd.removable = true;
        assert_eq!(class_of(&sd, true), Class::Internal);
    }

    #[test]
    fn virtual_devices_are_never_disks() {
        for n in ["loop0", "dm-0", "md127", "zram0", "nbd0", "ram0"] {
            assert!(is_virtual(n), "{n}");
            let ds = vec![
                disk("sda", DiskTransport::Usb, true),
                disk(n, DiskTransport::Usb, false),
            ];
            assert_eq!(classes(&ds).len(), 1, "{n} is left out");
            assert_eq!(class_of(&ds[1], true), Class::Internal, "{n}");
        }
        for n in ["sda", "nvme0n1", "vdb", "mmcblk0"] {
            assert!(!is_virtual(n), "{n}");
        }
    }

    #[test]
    fn kinds_are_the_api_words() {
        assert_eq!(Class::Stick.kind(), "stick");
        assert_eq!(Class::External.kind(), "external");
        assert_eq!(Class::Internal.kind(), "internal");
    }

    #[test]
    fn smart_answers_become_words() {
        let ok = r#"{"smart_status":{"passed":true},"temperature":{"current":38}}"#;
        assert_eq!(
            parse_smart(ok, Some(0)),
            Smart {
                health: Some("healthy"),
                temperature_c: Some(38)
            }
        );
        let fail = r#"{"smart_status":{"passed":false}}"#;
        assert_eq!(parse_smart(fail, Some(8)).health, Some("failing"));
        // Exit bit 3 alone is enough.
        assert_eq!(parse_smart(ok, Some(8)).health, Some("failing"));
        let worn = r#"{"smart_status":{"passed":true},"nvme_smart_health_information_log":{"percentage_used":85}}"#;
        assert_eq!(parse_smart(worn, Some(0)).health, Some("wear"));
        let warn = r#"{"smart_status":{"passed":true},"nvme_smart_health_information_log":{"critical_warning":1}}"#;
        assert_eq!(parse_smart(warn, Some(0)).health, Some("wear"));
        let realloc = r#"{"smart_status":{"passed":true},"ata_smart_attributes":{"table":[{"id":9,"raw":{"value":100}},{"id":5,"raw":{"value":3}}]}}"#;
        assert_eq!(parse_smart(realloc, Some(0)).health, Some("wear"));
        let clean = r#"{"smart_status":{"passed":true},"ata_smart_attributes":{"table":[{"id":5,"raw":{"value":0}}]}}"#;
        assert_eq!(parse_smart(clean, Some(0)).health, Some("healthy"));
        // Asleep, not opened, no program, garbage: unknown.
        assert_eq!(parse_smart(ok, Some(2)), Smart::default());
        assert_eq!(parse_smart("", None), Smart::default());
        assert_eq!(parse_smart("not json", Some(0)), Smart::default());
        // A silly temperature is dropped.
        let hot = r#"{"smart_status":{"passed":true},"temperature":{"current":900}}"#;
        assert_eq!(parse_smart(hot, Some(0)).temperature_c, None);
    }

    #[test]
    fn locks_in_order() {
        assert_eq!(
            lock_reason(Class::Stick, false, false, false),
            Some("stick")
        );
        assert_eq!(
            lock_reason(Class::Internal, false, false, false),
            Some("internal_live")
        );
        assert_eq!(
            lock_reason(Class::Internal, true, true, true),
            Some("internal_live")
        );
        assert_eq!(
            lock_reason(Class::External, true, true, true),
            Some("backup")
        );
        assert_eq!(
            lock_reason(Class::External, false, true, true),
            Some("write_protected")
        );
        assert_eq!(
            lock_reason(Class::External, false, false, true),
            Some("open")
        );
        assert_eq!(lock_reason(Class::External, false, false, false), None);
    }

    #[test]
    fn checks_are_read_only_and_sandboxed() {
        assert_eq!(
            check_command("exfat", "/dev/sdb1").unwrap(),
            ["fsck.exfat", "-n", "/dev/sdb1"]
        );
        assert_eq!(
            check_command("ext4", "/dev/sdb1").unwrap()[..3],
            ["e2fsck", "-n", "-f"]
        );
        assert_eq!(
            check_command("ntfs", "/dev/sdb1").unwrap()[..2],
            ["ntfsfix", "-n"]
        );
        assert_eq!(check_command("BitLocker", "/dev/sdb1"), None);
        assert_eq!(check_command("", "/dev/sdb1"), None);
        let a = sandbox_args("/dev/sdb1", &check_command("exfat", "/dev/sdb1").unwrap());
        for want in [
            "PrivateNetwork=yes",
            "DevicePolicy=closed",
            "DeviceAllow=/dev/sdb1 r",
            "CapabilityBoundingSet=",
            "NoNewPrivileges=yes",
            "--pipe",
        ] {
            assert!(a.iter().any(|x| x == want), "{want}");
        }
        let dd = a.iter().position(|x| x == "--").unwrap();
        assert_eq!(a[dd + 1..], ["fsck.exfat", "-n", "/dev/sdb1"]);
    }

    #[test]
    fn check_codes_in_words() {
        assert_eq!(check_verdict("exfat", Some(0)), Some(true));
        assert_eq!(check_verdict("exfat", Some(4)), Some(false));
        assert_eq!(check_verdict("ext4", Some(1)), Some(false));
        assert_eq!(check_verdict("ntfs", Some(1)), Some(false));
        assert_eq!(check_verdict("exfat", Some(8)), None);
        assert_eq!(check_verdict("exfat", None), None);
    }

    #[test]
    fn health_maps_to_levels() {
        assert_eq!(health_level(Some("healthy"), true), None);
        assert_eq!(health_level(None, true), None);
        assert_eq!(health_level(Some("wear"), true), Some("action"));
        assert_eq!(health_level(Some("failing"), false), Some("now"));
        assert_eq!(health_level(Some("failing"), true), Some("critical"));
    }
}
