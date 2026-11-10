// SPDX-License-Identifier: AGPL-3.0-or-later
//! Hardware detection shared by the live hardware report (stream A), the
//! installer's hardware check (stream D) and `alpd` (stream rule 12).
//!
//! Read-only: everything comes from `/sys`, `/proc` and the udev database in
//! `/run/udev/data`. The roots are parameters so tests use a fake tree.
//! Disk serial numbers and network addresses are not collected: the report
//! is meant to be shared, and the checks do not need them.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use crate::{DiskId, DiskInfo, DiskTransport, PartitionInfo};

/// `SecureBoot` variable in the EFI global variable namespace (UEFI spec,
/// `EFI_GLOBAL_VARIABLE`), as named by efivarfs.
const SECURE_BOOT_VAR: &str = "SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c";

/// Where to read from. [`Roots::system`] is the running machine.
#[derive(Debug, Clone)]
pub struct Roots {
    pub sys: PathBuf,
    pub proc: PathBuf,
    pub udev_data: PathBuf,
    /// Device number (`st_dev`) of the root filesystem, to find the disk
    /// the system runs from.
    pub root_dev: Option<u64>,
}

impl Roots {
    pub fn system() -> Self {
        Self {
            sys: "/sys".into(),
            proc: "/proc".into(),
            udev_data: "/run/udev/data".into(),
            root_dev: fs::metadata("/").ok().map(|m| m.dev()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cpu {
    pub model: String,
    pub threads: u32,
    pub is_64bit: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NicKind {
    Wired,
    Wireless,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nic {
    /// Interface name, or `chip <vendor>:<device>` for a PCI network chip
    /// that has no interface (no driver, or its firmware is missing).
    pub name: String,
    pub kind: NicKind,
    pub driver: String,
    pub usb: bool,
    /// False when the chip is present but no interface came up.
    pub working: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gpu {
    pub vendor: String,
    /// PCI `vendor:device`, e.g. `8086:5917`.
    pub pci_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HwReport {
    /// Maker and model from the firmware (DMI), for display.
    pub machine: String,
    pub cpu: Cpu,
    pub ram_bytes: u64,
    pub uefi: bool,
    /// `None` when the firmware does not say.
    pub secure_boot: Option<bool>,
    /// TPM major version (1 or 2), `None` when no TPM is visible.
    pub tpm_version: Option<u8>,
    pub disks: Vec<DiskInfo>,
    pub nics: Vec<Nic>,
    pub gpus: Vec<Gpu>,
}

/// Inspect the machine. Missing information never fails the inspection; it
/// shows up as empty or `None` values.
pub fn inspect(roots: &Roots) -> HwReport {
    let system_disk = roots.root_dev.and_then(|d| system_disk(&roots.sys, d));
    HwReport {
        machine: machine(&roots.sys),
        cpu: cpu(&roots.proc),
        ram_bytes: ram_from_dmi(&roots.sys).unwrap_or_else(|| ram_from_meminfo(&roots.proc)),
        uefi: roots.sys.join("firmware/efi").is_dir(),
        secure_boot: secure_boot(&roots.sys),
        tpm_version: read(&roots.sys.join("class/tpm/tpm0/tpm_version_major"))
            .and_then(|v| v.parse().ok()),
        disks: disks(roots, system_disk.as_deref()),
        nics: nics(&roots.sys),
        gpus: gpus(&roots.sys),
    }
}

fn read(path: &Path) -> Option<String> {
    let s = fs::read_to_string(path).ok()?;
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

fn link_name(path: &Path) -> Option<String> {
    Some(
        fs::read_link(path)
            .ok()?
            .file_name()?
            .to_string_lossy()
            .into_owned(),
    )
}

fn sorted_entries(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

/// Firmware placeholders that mean "not filled in by the maker".
fn is_placeholder(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    [
        "to be filled",
        "default string",
        "system product name",
        "not applicable",
        "none",
    ]
    .iter()
    .any(|p| l.contains(p))
}

fn machine(sys: &Path) -> String {
    let dmi = sys.join("class/dmi/id");
    let mut parts: Vec<String> = Vec::new();
    for f in ["sys_vendor", "product_name", "product_version"] {
        if let Some(v) = read(&dmi.join(f)).filter(|v| !is_placeholder(v)) {
            // Lenovo puts the marketing name in product_version.
            if !parts
                .iter()
                .any(|p| p.contains(&v) || v.contains(p.as_str()))
            {
                parts.push(v);
            }
        }
    }
    if parts.is_empty() {
        "Unknown computer".into()
    } else {
        parts.join(" ")
    }
}

fn cpu(proc: &Path) -> Cpu {
    let info = fs::read_to_string(proc.join("cpuinfo")).unwrap_or_default();
    let field = |line: &str, key: &str| -> Option<String> {
        let (k, v) = line.split_once(':')?;
        (k.trim() == key).then(|| v.trim().to_string())
    };
    let model = info
        .lines()
        .find_map(|l| field(l, "model name"))
        .unwrap_or_else(|| "Unknown".into());
    let threads = info
        .lines()
        .filter(|l| field(l, "processor").is_some())
        .count() as u32;
    let is_64bit = info
        .lines()
        .find_map(|l| field(l, "flags"))
        .is_some_and(|f| f.split_whitespace().any(|x| x == "lm"));
    Cpu {
        model: model.split_whitespace().collect::<Vec<_>>().join(" "),
        threads,
        is_64bit,
    }
}

/// Installed memory from SMBIOS type 17 (memory device) records, which
/// counts what is fitted rather than what the kernel can use. Needs root.
fn ram_from_dmi(sys: &Path) -> Option<u64> {
    let dir = sys.join("firmware/dmi/entries");
    let mut total = 0u64;
    for name in sorted_entries(&dir).iter().filter(|n| n.starts_with("17-")) {
        let Ok(raw) = fs::read(dir.join(name).join("raw")) else {
            continue;
        };
        total += dmi17_size_bytes(&raw).unwrap_or(0);
    }
    (total > 0).then_some(total)
}

/// Size of one SMBIOS type 17 record (SMBIOS spec 7.18: size word at
/// offset 0x0C, extended size dword at 0x1C).
fn dmi17_size_bytes(raw: &[u8]) -> Option<u64> {
    let size = u16::from_le_bytes([*raw.get(0x0C)?, *raw.get(0x0D)?]);
    match size {
        0 | 0xFFFF => None,
        0x7FFF => {
            let ext = u32::from_le_bytes(raw.get(0x1C..0x20)?.try_into().ok()?) & 0x7FFF_FFFF;
            Some(u64::from(ext) << 20)
        }
        s if s & 0x8000 != 0 => Some(u64::from(s & 0x7FFF) << 10),
        s => Some(u64::from(s) << 20),
    }
}

fn ram_from_meminfo(proc: &Path) -> u64 {
    let info = fs::read_to_string(proc.join("meminfo")).unwrap_or_default();
    info.lines()
        .find_map(|l| l.strip_prefix("MemTotal:"))
        .and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<u64>().ok())
        .map_or(0, |kib| kib * 1024)
}

fn secure_boot(sys: &Path) -> Option<bool> {
    // efivarfs: 4 attribute bytes, then the value.
    let raw = fs::read(sys.join("firmware/efi/efivars").join(SECURE_BOOT_VAR)).ok()?;
    raw.get(4).map(|v| *v == 1)
}

/// The whole disk behind the root filesystem: follow device-mapper slaves
/// (dm-verity) down to a partition, then take its parent disk.
fn system_disk(sys: &Path, dev: u64) -> Option<String> {
    let major = ((dev >> 8) & 0xfff) | ((dev >> 32) & !0xfff);
    let minor = (dev & 0xff) | ((dev >> 12) & !0xff);
    let mut node = fs::canonicalize(sys.join(format!("dev/block/{major}:{minor}"))).ok()?;
    for _ in 0..8 {
        if let Some(first) = sorted_entries(&node.join("slaves")).first() {
            node = fs::canonicalize(node.join("slaves").join(first)).ok()?;
        } else if node.join("partition").is_file() {
            return Some(node.parent()?.file_name()?.to_string_lossy().into_owned());
        } else {
            return Some(node.file_name()?.to_string_lossy().into_owned());
        }
    }
    None
}

/// Properties of a block device from the udev database (`E:KEY=value`).
fn udev_props(udev_data: &Path, dev_numbers: &str) -> Vec<(String, String)> {
    fs::read_to_string(udev_data.join(format!("b{dev_numbers}")))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.strip_prefix("E:")?.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn disks(roots: &Roots, system_disk: Option<&str>) -> Vec<DiskInfo> {
    let block = roots.sys.join("block");
    let mut out = Vec::new();
    for name in sorted_entries(&block) {
        let dir = block.join(&name);
        // Real disks have a device; loop, ram, zram and dm devices do not.
        if !dir.join("device").exists() || name.starts_with("sr") {
            continue;
        }
        let path = fs::canonicalize(&dir)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let transport = if name.starts_with("nvme") {
            DiskTransport::Nvme
        } else if on_usb(&path) {
            DiskTransport::Usb
        } else if name.starts_with("vd") || path.contains("/virtio") {
            DiskTransport::Virtio
        } else if path.contains("/ata") {
            DiskTransport::Sata
        } else {
            DiskTransport::Other
        };
        let model = read(&dir.join("device/model"))
            .or_else(|| read(&dir.join("device/device/model")))
            .unwrap_or_else(|| "Unknown disk".into());
        let partitions = sorted_entries(&dir)
            .into_iter()
            .filter(|p| dir.join(p).join("partition").is_file())
            .map(|p| {
                let pdir = dir.join(&p);
                let props = udev_props(
                    &roots.udev_data,
                    &read(&pdir.join("dev")).unwrap_or_default(),
                );
                let get = |k: &str| {
                    props
                        .iter()
                        .find(|(key, _)| key == k)
                        .map(|(_, v)| v.clone())
                        .unwrap_or_default()
                };
                PartitionInfo {
                    name: p.clone(),
                    partition_uuid: get("ID_PART_ENTRY_UUID"),
                    type_uuid: get("ID_PART_ENTRY_TYPE").to_ascii_uppercase(),
                    label: get("ID_PART_ENTRY_NAME"),
                    size_bytes: sectors(&pdir),
                }
            })
            .collect();
        out.push(DiskInfo {
            id: DiskId(name.clone()),
            model: model.split_whitespace().collect::<Vec<_>>().join(" "),
            serial: None,
            size_bytes: sectors(&dir),
            transport,
            rotational: read(&dir.join("queue/rotational")).as_deref() == Some("1"),
            removable: read(&dir.join("removable")).as_deref() == Some("1"),
            is_system_disk: system_disk == Some(name.as_str()),
            partitions,
        });
    }
    out
}

/// Size from sysfs, which counts 512-byte sectors whatever the disk uses.
fn sectors(dir: &Path) -> u64 {
    read(&dir.join("size"))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
        * 512
}

fn nics(sys: &Path) -> Vec<Nic> {
    let net = sys.join("class/net");
    let mut backing = Vec::new();
    let mut out: Vec<Nic> = sorted_entries(&net)
        .into_iter()
        .filter(|n| net.join(n).join("device").exists())
        .map(|name| {
            let dir = net.join(&name);
            let wireless = dir.join("wireless").exists() || dir.join("phy80211").exists();
            let path = fs::canonicalize(&dir)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            if let Ok(dev) = fs::canonicalize(dir.join("device")) {
                backing.push(dev);
            }
            Nic {
                kind: if wireless {
                    NicKind::Wireless
                } else {
                    NicKind::Wired
                },
                driver: link_name(&dir.join("device/driver")).unwrap_or_else(|| "no driver".into()),
                usb: on_usb(&path),
                working: true,
                name,
            }
        })
        .collect();
    // PCI network chips (class 0x02) without an interface: show them, so a
    // report names the chip whose driver or firmware is missing.
    let pci = sys.join("bus/pci/devices");
    for d in sorted_entries(&pci) {
        let dir = pci.join(&d);
        let Some(class) = read(&dir.join("class")).filter(|c| c.starts_with("0x02")) else {
            continue;
        };
        if fs::canonicalize(&dir).is_ok_and(|p| backing.contains(&p)) {
            continue;
        }
        let id = |f: &str| {
            read(&dir.join(f))
                .unwrap_or_default()
                .trim_start_matches("0x")
                .to_string()
        };
        out.push(Nic {
            name: format!("chip {}:{}", id("vendor"), id("device")),
            // 0x0280: "other network controller", which is how Wi-Fi chips report.
            kind: if class.starts_with("0x0280") {
                NicKind::Wireless
            } else {
                NicKind::Wired
            },
            driver: link_name(&dir.join("driver")).unwrap_or_else(|| "no driver".into()),
            usb: false,
            working: false,
        });
    }
    out
}

fn gpus(sys: &Path) -> Vec<Gpu> {
    let pci = sys.join("bus/pci/devices");
    sorted_entries(&pci)
        .into_iter()
        .filter(|d| read(&pci.join(d).join("class")).is_some_and(|c| c.starts_with("0x03")))
        .map(|d| {
            let id = |f: &str| {
                read(&pci.join(&d).join(f))
                    .unwrap_or_default()
                    .trim_start_matches("0x")
                    .to_string()
            };
            let (vendor, device) = (id("vendor"), id("device"));
            let name = match vendor.as_str() {
                "8086" => "Intel",
                "1002" => "AMD",
                "10de" => "NVIDIA",
                "1234" | "1af4" | "1b36" => "Virtual (QEMU)",
                "15ad" => "Virtual (VMware)",
                "80ee" => "Virtual (VirtualBox)",
                _ => "Other",
            };
            Gpu {
                vendor: name.into(),
                pci_id: format!("{vendor}:{device}"),
            }
        })
        .collect()
}

/// Outcome of one check, as shown to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    /// Works, with a limitation or something to know.
    Note,
    /// ALPArchive cannot run on this computer as it is.
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub item: &'static str,
    pub status: Status,
    pub value: String,
    /// Plain-language advice; empty when there is nothing to do.
    pub advice: String,
}

/// Minimums from the hardware profile (`profiles/*.toml`).
#[derive(Debug, Clone, Copy)]
pub struct Minimums {
    pub ram_bytes: u64,
    pub single_disk_bytes: u64,
}

/// Decimal gigabytes, as disk makers count.
pub fn gb(bytes: u64) -> String {
    let g = bytes as f64 / 1e9;
    if g >= 10.0 || (g - g.round()).abs() < 0.05 {
        format!("{g:.0} GB")
    } else {
        format!("{g:.1} GB")
    }
}

/// Memory size in binary gigabytes, as memory is sold.
pub fn ram_gb(bytes: u64) -> String {
    let g = bytes as f64 / f64::from(1u32 << 30);
    if (g - g.round()).abs() < 0.05 {
        format!("{g:.0} GB")
    } else {
        format!("{g:.1} GB")
    }
}

/// A disk that stays in the computer: not the stick ALP-OS runs from, not
/// removable, not on USB.
pub fn is_internal(d: &DiskInfo) -> bool {
    !d.is_system_disk && !d.removable && d.transport != DiskTransport::Usb
}

/// Turn the facts into the checks the user sees, in display order.
pub fn assess(r: &HwReport, min: Minimums) -> Vec<Check> {
    let check = |item, status, value: String, advice: &str| Check {
        item,
        status,
        value,
        advice: advice.into(),
    };
    let mut v = Vec::new();

    v.push(if r.cpu.is_64bit {
        check(
            "Processor",
            Status::Ok,
            format!("{} ({} threads)", r.cpu.model, r.cpu.threads),
            "",
        )
    } else {
        check(
            "Processor",
            Status::Fail,
            r.cpu.model.clone(),
            "ALPArchive needs a 64-bit processor.",
        )
    });

    v.push(if r.ram_bytes >= min.ram_bytes {
        check("Memory", Status::Ok, ram_gb(r.ram_bytes), "")
    } else {
        check(
            "Memory",
            Status::Fail,
            ram_gb(r.ram_bytes),
            &format!("ALPArchive needs at least {} of memory.", gb(min.ram_bytes)),
        )
    });

    v.push(if r.uefi {
        check("Startup (UEFI)", Status::Ok, "UEFI".into(), "")
    } else {
        check(
            "Startup (UEFI)",
            Status::Fail,
            "Legacy BIOS".into(),
            "Turn on UEFI in the BIOS settings.",
        )
    });

    v.push(match r.secure_boot {
        Some(false) => check("Secure Boot", Status::Ok, "Off".into(), ""),
        Some(true) => check(
            "Secure Boot",
            Status::Note,
            "On".into(),
            "Beta images are not signed yet; turn Secure Boot off for the beta.",
        ),
        // UEFI defines SecureBoot only on firmware that supports it.
        None if r.uefi => check("Secure Boot", Status::Ok, "Not available".into(), ""),
        None => check("Secure Boot", Status::Note, "Unknown".into(), ""),
    });

    v.push(match r.tpm_version {
        Some(2) => check("Security chip (TPM)", Status::Ok, "TPM 2.0".into(), ""),
        Some(n) => check("Security chip (TPM)", Status::Note, format!("TPM {n}.x"), "Not used. ALPArchive will keep its key in a key file instead."),
        None => check(
            "Security chip (TPM)",
            Status::Note,
            "Not found".into(),
            "ALPArchive will keep its key in a key file instead. The BIOS may call the chip TPM, PTT or fTPM.",
        ),
    });

    let internal: Vec<&DiskInfo> = r.disks.iter().filter(|d| is_internal(d)).collect();
    let largest = internal.iter().map(|d| d.size_bytes).max();
    v.push(match largest {
        Some(b) if b >= min.single_disk_bytes => check(
            "Storage",
            Status::Ok,
            format!(
                "{} internal disk{}, largest {}",
                internal.len(),
                if internal.len() == 1 { "" } else { "s" },
                gb(b)
            ),
            "",
        ),
        Some(b) => check(
            "Storage",
            Status::Fail,
            format!("Largest internal disk {}", gb(b)),
            &format!(
                "ALPArchive needs a disk of at least {}.",
                gb(min.single_disk_bytes)
            ),
        ),
        None => check(
            "Storage",
            Status::Fail,
            "No internal disk found".into(),
            "ALPArchive needs an internal disk for the archive.",
        ),
    });

    let wired: Vec<&Nic> = r
        .nics
        .iter()
        .filter(|n| n.kind == NicKind::Wired && n.working)
        .collect();
    let wifi = r.nics.iter().any(|n| n.kind == NicKind::Wireless);
    let names = |ns: &[&Nic]| {
        ns.iter()
            .map(|n| format!("{} ({})", n.name, n.driver))
            .collect::<Vec<_>>()
            .join(", ")
    };
    v.push(if wired.is_empty() {
        check(
            "Wired network",
            Status::Note,
            if wifi {
                "Wi-Fi only".into()
            } else {
                "None found".into()
            },
            "Use a USB-Ethernet adapter and a network cable.",
        )
    } else {
        check("Wired network", Status::Ok, names(&wired), "")
    });

    let g = r
        .gpus
        .iter()
        .map(|g| format!("{} {}", g.vendor, g.pci_id))
        .collect::<Vec<_>>()
        .join(", ");
    v.push(check(
        "Graphics",
        Status::Ok,
        if g.is_empty() { "None found".into() } else { g },
        "",
    ));
    v
}

/// The computer's own battery (laptops), from `class/power_supply`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Battery {
    /// Charge now, 0-100.
    pub percent: u8,
    /// Plugged in: charging, full, or held below full by the firmware.
    pub charging: bool,
    pub model: String,
    /// Full charge now against full charge when new, if the firmware says.
    pub health_percent: Option<u8>,
}

/// The first battery of the computer itself (type `Battery`, not a mouse's
/// or a phone's: scope `Device` is skipped), or `None` on a desktop.
pub fn battery(sys: &Path) -> Option<Battery> {
    let dir = sys.join("class/power_supply");
    sorted_entries(&dir).into_iter().find_map(|name| {
        let d = dir.join(&name);
        let kind = read(&d.join("type"));
        if kind.as_deref() != Some("Battery") && !name.starts_with("BAT") {
            return None;
        }
        if read(&d.join("scope")).as_deref() == Some("Device")
            || read(&d.join("present")).as_deref() == Some("0")
        {
            return None;
        }
        let num = |f: &str| read(&d.join(f)).and_then(|v| v.parse::<u64>().ok());
        let ratio = |a: Option<u64>, b: Option<u64>| match (a, b) {
            (Some(a), Some(b)) if b > 0 => u8::try_from((a * 100 / b).min(100)).ok(),
            _ => None,
        };
        let percent = num("capacity")
            .and_then(|c| u8::try_from(c.min(100)).ok())
            .or_else(|| ratio(num("energy_now"), num("energy_full")))
            .or_else(|| ratio(num("charge_now"), num("charge_full")))?;
        let status = read(&d.join("status")).unwrap_or_default();
        let health = ratio(num("energy_full"), num("energy_full_design"))
            .or_else(|| ratio(num("charge_full"), num("charge_full_design")));
        Some(Battery {
            percent,
            charging: matches!(status.as_str(), "Charging" | "Full" | "Not charging"),
            model: read(&d.join("model_name")).unwrap_or_default(),
            health_percent: health,
        })
    })
}

/// Is a sysfs device path below a USB bus? A whole `usb<N>` path segment
/// (the bus), not any text with "/usb" in it (review N2).
fn on_usb(path: &str) -> bool {
    path.split('/').any(|seg| {
        seg.strip_prefix("usb")
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    })
}

#[cfg(test)]
mod tests {

    #[test]
    fn usb_needs_a_whole_bus_segment() {
        assert!(super::on_usb(
            "/sys/devices/pci0000:00/0000:00:14.0/usb2/2-1/2-1:1.0/host0/target0:0:0/0:0:0:0/block/sda"
        ));
        assert!(!super::on_usb(
            "/sys/devices/platform/usbfake-ctrl/ata1/host0/block/sda"
        ));
        assert!(!super::on_usb("/sys/devices/pci0000:00/usb/block/sdb"));
    }
    use super::*;

    struct Tree(PathBuf);
    impl Tree {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!("alp-hw-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Tree(p)
        }
        fn file(&self, rel: &str, body: impl AsRef<[u8]>) {
            let p = self.0.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, body).unwrap();
        }
        fn link(&self, rel: &str, target: &str) {
            let p = self.0.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(target, p).unwrap();
        }
        fn roots(&self, root_dev: Option<u64>) -> Roots {
            Roots {
                sys: self.0.join("sys"),
                proc: self.0.join("proc"),
                udev_data: self.0.join("udev"),
                root_dev,
            }
        }
    }
    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn dmi17(size: u16, ext: u32) -> Vec<u8> {
        let mut raw = vec![0u8; 0x22];
        raw[0] = 17;
        raw[0x0C..0x0E].copy_from_slice(&size.to_le_bytes());
        raw[0x1C..0x20].copy_from_slice(&ext.to_le_bytes());
        raw
    }

    /// A laptop: NVMe disk, the USB stick ALP-OS runs from (behind
    /// dm-verity), wired and Wi-Fi network, TPM 2.0, Secure Boot off.
    fn laptop() -> Tree {
        let t = Tree::new("laptop");
        t.file("sys/class/dmi/id/sys_vendor", "LENOVO\n");
        t.file("sys/class/dmi/id/product_name", "20L5CTO1WW\n");
        t.file("sys/class/dmi/id/product_version", "ThinkPad T480\n");
        t.file("proc/cpuinfo", "processor\t: 0\nmodel name\t: Intel(R) Core(TM)  i5-8250U\nflags\t\t: fpu lm sse\n\nprocessor\t: 1\nmodel name\t: Intel(R) Core(TM)  i5-8250U\n");
        t.file("proc/meminfo", "MemTotal:        7990000 kB\n");
        t.file("sys/firmware/dmi/entries/17-0/raw", dmi17(8192, 0));
        t.file("sys/firmware/dmi/entries/17-1/raw", dmi17(0x7FFF, 8192));
        t.file("sys/firmware/dmi/entries/17-2/raw", dmi17(0, 0)); // empty slot
        t.file(
            &format!("sys/firmware/efi/efivars/{SECURE_BOOT_VAR}"),
            [6, 0, 0, 0, 0],
        );
        t.file("sys/class/tpm/tpm0/tpm_version_major", "2\n");
        // NVMe disk
        t.file("sys/devices/pci0/nvme/nvme0/nvme0n1/size", "1000215216\n");
        t.file("sys/devices/pci0/nvme/nvme0/nvme0n1/removable", "0\n");
        t.file(
            "sys/devices/pci0/nvme/nvme0/nvme0n1/queue/rotational",
            "0\n",
        );
        t.file(
            "sys/devices/pci0/nvme/nvme0/model",
            "Samsung SSD 970 EVO  \n",
        );
        t.link("sys/devices/pci0/nvme/nvme0/nvme0n1/device", "../../nvme0");
        t.link("sys/block/nvme0n1", "../devices/pci0/nvme/nvme0/nvme0n1");
        // USB stick sda with partitions sda1 (report) and sda2 (root data)
        let sda = "sys/devices/pci0/usb1/1-1/host0/target0/0:0:0:0/block/sda";
        t.file(&format!("{sda}/size"), "62521344\n");
        t.file(&format!("{sda}/removable"), "1\n");
        t.file(&format!("{sda}/device/model"), "Cruzer Blade\n");
        t.file(&format!("{sda}/sda1/partition"), "1\n");
        t.file(&format!("{sda}/sda1/size"), "131072\n");
        t.file(&format!("{sda}/sda1/dev"), "8:1\n");
        t.file(&format!("{sda}/sda2/partition"), "2\n");
        t.file(&format!("{sda}/sda2/dev"), "8:2\n");
        t.link(
            "sys/block/sda",
            &format!("../{}", sda.trim_start_matches("sys/")),
        );
        t.file("udev/b8:1", "E:ID_PART_ENTRY_UUID=aaaa\nE:ID_PART_ENTRY_TYPE=ebd0a0a2-b9e5-4433-87c0-68b6b72699c7\nE:ID_PART_ENTRY_NAME=ALP-REPORT\n");
        // dm-0 (verity root) with slave sda2; root_dev = 253:0
        t.file("sys/devices/virtual/block/dm-0/size", "1000\n");
        t.link(
            "sys/devices/virtual/block/dm-0/slaves/sda2",
            &format!("../../../../../../{sda}/sda2"),
        );
        t.link("sys/dev/block/253:0", "../../devices/virtual/block/dm-0");
        t.link("sys/block/dm-0", "../devices/virtual/block/dm-0");
        // network
        t.link("sys/class/net/lo", "../../devices/virtual/net/lo");
        t.file("sys/devices/virtual/net/lo/mtu", "65536\n");
        t.file("sys/devices/pci0/eth/net/enp0s31f6/mtu", "1500\n");
        t.link("sys/devices/pci0/eth/net/enp0s31f6/device", "../..");
        t.link(
            "sys/devices/pci0/eth/driver",
            "../../bus/pci/drivers/e1000e",
        );
        t.link(
            "sys/class/net/enp0s31f6",
            "../../devices/pci0/eth/net/enp0s31f6",
        );
        t.file("sys/devices/pci0/wifi/net/wlp3s0/wireless/x", "");
        t.link("sys/devices/pci0/wifi/net/wlp3s0/device", "../..");
        t.link("sys/class/net/wlp3s0", "../../devices/pci0/wifi/net/wlp3s0");
        // GPU
        t.file("sys/bus/pci/devices/0000:00:02.0/class", "0x030000\n");
        t.file("sys/bus/pci/devices/0000:00:02.0/vendor", "0x8086\n");
        t.file("sys/bus/pci/devices/0000:00:02.0/device", "0x5917\n");
        t.file("sys/bus/pci/devices/0000:00:1f.0/class", "0x060100\n");
        // Wi-Fi chip whose firmware is missing: no interface.
        t.file("sys/bus/pci/devices/0000:00:14.3/class", "0x028000\n");
        t.file("sys/bus/pci/devices/0000:00:14.3/vendor", "0x8086\n");
        t.file("sys/bus/pci/devices/0000:00:14.3/device", "0x51f0\n");
        t
    }

    const MIN: Minimums = Minimums {
        ram_bytes: 4_000_000_000,
        single_disk_bytes: 64_000_000_000,
    };

    #[test]
    fn inspects_a_laptop() {
        let t = laptop();
        let r = inspect(&t.roots(Some(253 << 8)));
        assert_eq!(r.machine, "LENOVO 20L5CTO1WW ThinkPad T480");
        assert_eq!(
            r.cpu,
            Cpu {
                model: "Intel(R) Core(TM) i5-8250U".into(),
                threads: 2,
                is_64bit: true
            }
        );
        assert_eq!(
            r.ram_bytes,
            16 << 30,
            "DMI: 8 GiB + 8 GiB (extended size), empty slot ignored"
        );
        assert!(r.uefi);
        assert_eq!(r.secure_boot, Some(false));
        assert_eq!(r.tpm_version, Some(2));
        let names: Vec<_> = r.disks.iter().map(|d| d.id.0.as_str()).collect();
        assert_eq!(names, ["nvme0n1", "sda"], "dm-0 is not a disk");
        let nvme = &r.disks[0];
        assert_eq!(
            (nvme.transport, nvme.removable, nvme.is_system_disk),
            (DiskTransport::Nvme, false, false)
        );
        assert_eq!(nvme.model, "Samsung SSD 970 EVO");
        assert_eq!(nvme.size_bytes, 1000215216 * 512);
        let stick = &r.disks[1];
        assert_eq!(
            (stick.transport, stick.is_system_disk),
            (DiskTransport::Usb, true)
        );
        assert_eq!(stick.partitions[0].label, "ALP-REPORT");
        assert_eq!(
            stick.partitions[0].type_uuid,
            "EBD0A0A2-B9E5-4433-87C0-68B6B72699C7"
        );
        assert_eq!(stick.partitions[0].size_bytes, 64 << 20);
        assert_eq!(
            r.nics,
            [
                Nic {
                    name: "enp0s31f6".into(),
                    kind: NicKind::Wired,
                    driver: "e1000e".into(),
                    usb: false,
                    working: true
                },
                Nic {
                    name: "wlp3s0".into(),
                    kind: NicKind::Wireless,
                    driver: "no driver".into(),
                    usb: false,
                    working: true
                },
                Nic {
                    name: "chip 8086:51f0".into(),
                    kind: NicKind::Wireless,
                    driver: "no driver".into(),
                    usb: false,
                    working: false
                },
            ]
        );
        assert_eq!(
            r.gpus,
            [Gpu {
                vendor: "Intel".into(),
                pci_id: "8086:5917".into()
            }]
        );

        let checks = assess(&r, MIN);
        assert!(checks.iter().all(|c| c.status == Status::Ok), "{checks:#?}");
        assert_eq!(checks[5].value, "1 internal disk, largest 512 GB");
    }

    #[test]
    fn weak_machine_fails_with_advice() {
        let t = Tree::new("weak");
        t.file(
            "proc/cpuinfo",
            "processor : 0\nmodel name : Old CPU\nflags : fpu\n",
        );
        t.file("proc/meminfo", "MemTotal: 2000000 kB\n");
        let r = inspect(&t.roots(None));
        assert_eq!(r.machine, "Unknown computer");
        assert_eq!(r.ram_bytes, 2_048_000_000, "falls back to MemTotal");
        let checks = assess(&r, MIN);
        let status = |item: &str| checks.iter().find(|c| c.item == item).unwrap().status;
        for item in ["Processor", "Memory", "Startup (UEFI)", "Storage"] {
            assert_eq!(status(item), Status::Fail, "{item}");
        }
        for item in ["Secure Boot", "Security chip (TPM)", "Wired network"] {
            assert_eq!(status(item), Status::Note, "{item}");
        }
        assert!(
            checks
                .iter()
                .filter(|c| c.status == Status::Fail)
                .all(|c| !c.advice.is_empty())
        );
        let memory = checks.iter().find(|c| c.item == "Memory").unwrap();
        assert_eq!(memory.advice, "ALPArchive needs at least 4 GB of memory.");
    }

    #[test]
    fn dmi17_sizes() {
        assert_eq!(dmi17_size_bytes(&dmi17(4096, 0)), Some(4 << 30));
        assert_eq!(dmi17_size_bytes(&dmi17(0x8000 | 512, 0)), Some(512 << 10));
        assert_eq!(dmi17_size_bytes(&dmi17(0x7FFF, 65536)), Some(64 << 30));
        assert_eq!(dmi17_size_bytes(&dmi17(0xFFFF, 0)), None);
        assert_eq!(dmi17_size_bytes(&[17, 0]), None, "short record");
    }

    #[test]
    fn sizes_read_like_the_label() {
        assert_eq!(gb(512_110_190_592), "512 GB");
        assert_eq!(gb(8_000_000_000), "8 GB");
        assert_eq!(gb(2_500_000_000), "2.5 GB");
        assert_eq!(ram_gb(16 << 30), "16 GB");
        assert_eq!(ram_gb(3_900_000_000), "3.6 GB");
    }

    #[test]
    fn battery_of_a_laptop_and_none_on_a_desktop() {
        let t = Tree::new("bat");
        assert_eq!(battery(&t.roots(None).sys), None);
        t.file("sys/class/power_supply/AC/type", "Mains\n");
        t.file("sys/class/power_supply/AC/online", "1\n");
        // A wireless mouse's battery is not the computer's.
        t.file("sys/class/power_supply/hidpp_battery_0/type", "Battery\n");
        t.file("sys/class/power_supply/hidpp_battery_0/scope", "Device\n");
        t.file("sys/class/power_supply/hidpp_battery_0/capacity", "40\n");
        assert_eq!(battery(&t.roots(None).sys), None);
        t.file("sys/class/power_supply/BAT0/type", "Battery\n");
        t.file("sys/class/power_supply/BAT0/status", "Discharging\n");
        t.file("sys/class/power_supply/BAT0/capacity", "87\n");
        t.file("sys/class/power_supply/BAT0/model_name", "L20M4PC1\n");
        t.file("sys/class/power_supply/BAT0/energy_full", "45600000\n");
        t.file(
            "sys/class/power_supply/BAT0/energy_full_design",
            "50000000\n",
        );
        assert_eq!(
            battery(&t.roots(None).sys),
            Some(Battery {
                percent: 87,
                charging: false,
                model: "L20M4PC1".into(),
                health_percent: Some(91),
            })
        );
        // No `capacity`: computed from charge_now/charge_full.
        t.file("sys/class/power_supply/BAT0/capacity", "");
        t.file("sys/class/power_supply/BAT0/status", "Charging\n");
        t.file("sys/class/power_supply/BAT0/charge_now", "1500\n");
        t.file("sys/class/power_supply/BAT0/charge_full", "3000\n");
        let b = battery(&t.roots(None).sys).unwrap();
        assert_eq!((b.percent, b.charging), (50, true));
    }
}
