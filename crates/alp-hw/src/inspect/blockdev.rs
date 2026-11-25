// SPDX-License-Identifier: AGPL-3.0-or-later
//! Disk discovery for the installer and `alpd` (ADR 0028, plan
//! `docs/plans/0.2.8-installer.md`): every connected disk, with the roles it
//! can take or the one plain reason it cannot be used.
//!
//! Pure functions over the output of
//! `lsblk --json --bytes --tree --output NAME,PATH,TYPE,SIZE,MODEL,TRAN,RM,HOTPLUG,ROTA,RO,PTTYPE,PARTTYPE,PARTLABEL,PARTFLAGS,FSTYPE,LABEL,MOUNTPOINTS`
//! and, for a partition lsblk could not name, `blkid -p -o export`. Nothing
//! here touches a disk; the caller runs the commands.
//!
//! A disk that holds an operating system is never usable in 0.2.8.
//! Partition type GUIDs are from the util-linux 2.39.3 libfdisk table and
//! `systemd-id128 show` (systemd 255), not typed from memory (plan,
//! "Verification status").

use serde_json::Value;

use crate::{HwError, Result};

/// The disk list could not be read (stream D range, `errors/catalog.toml`).
pub const CODE_DISK_LIST: &str = "ALP-INST-400";
/// ESP label of a system disk while the installer writes it; the last
/// install step sets it back to `esp` (crates/alp-installer exec.rs).
pub const UNFINISHED_LABEL: &str = "alp-installing";

// ------------------------------------------------------------ GPT types
// Upper case, as normalised by the parser.

const ESP: &str = "C12A7328-F81F-11D2-BA4B-00A0C93EC93B";
const BIOS_BOOT: &str = "21686148-6449-6E6F-744E-656564454649";
const MS_RESERVED: &str = "E3C9E316-0B5C-4DB8-817D-F92DF00215AE";
const WIN_RECOVERY: &str = "DE94BBA4-06D1-4D40-A16A-BFD50179D6AC";

/// Linux system partitions: root, `/usr` and their verity partitions for
/// the architectures a PC disk realistically comes from (x86, x86-64, ARM,
/// ARM-64, RISC-V-64), plus extended boot, swap, home, server data,
/// variable data, temporary data and user home.
const LINUX_SYSTEM: &[&str] = &[
    // root
    "44479540-F297-41B2-9AF7-D131D5F0458A",
    "4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709",
    "69DAD710-2CE4-4E3C-B16C-21A1D49ABED3",
    "B921B045-1DF0-41C3-AF44-4C6F280D3FAE",
    "72EC70A6-CF74-40E6-BD49-4BDA08E8F224",
    // /usr
    "75250D76-8CC6-458E-BD66-BD47CC81A812",
    "8484680C-9521-48C6-9C11-B0720656F69E",
    "7D0359A3-02B3-4F0A-865C-654403E70625",
    "B0E01050-EE5F-4390-949A-9101B17104E9",
    "BEAEC34B-8442-439B-A40B-984381ED097D",
    // root verity
    "D13C5D3B-B5D1-422A-B29F-9454FDC89D76",
    "2C7357ED-EBD2-46D9-AEC1-23D437EC2BF5",
    "7386CDF2-203C-47A9-A498-F2ECCE45A2D6",
    "DF3300CE-D69F-4C92-978C-9BFB0F38D820",
    "B6ED5582-440B-4209-B8DA-5FF7C419EA3D",
    // /usr verity
    "8F461B0D-14EE-4E81-9AA9-049B6FB97ABD",
    "77FF5F63-E7B6-4633-ACF4-1565B864C0E6",
    "C215D751-7BCD-4649-BE90-6627490A4C05",
    "6E11A4E7-FBCA-4DED-B9E9-E1A512BB664E",
    "8F1056BE-9B05-47C4-81D6-BE53128E5B54",
    // extended boot, swap, home, server data, variable data, temporary data, user home
    "BC13C2FF-59E6-4262-A352-B275FD6F7172",
    "0657FD6D-A4AB-43C4-84E5-0933C84B4F4F",
    "933AC7E1-2EB4-4F13-B844-0E14E2AEF915",
    "3B8F8425-20E0-4F3B-907F-1A25A76F98E8",
    "4D21B016-B534-45C2-A9FB-5C16E091FD2D",
    "7EC6F557-3BC5-4ACA-B293-16EF5DF639D1",
    "773F91EF-66D4-49B5-BD83-D683BF40AD16",
];

const APPLE_SYSTEM: &[&str] = &[
    "48465300-0000-11AA-AA11-00306543ECAC", // HFS/HFS+
    "7C3457EF-0000-11AA-AA11-00306543ECAC", // APFS
    "55465300-0000-11AA-AA11-00306543ECAC", // Apple UFS
    "426F6F74-0000-11AA-AA11-00306543ECAC", // Apple boot
    "4C616265-6C00-11AA-AA11-00306543ECAC", // Apple label
    "5265636F-7665-11AA-AA11-00306543ECAC", // Apple TV recovery
    "53746F72-6167-11AA-AA11-00306543ECAC", // Apple Core storage
    "69646961-6700-11AA-AA11-00306543ECAC", // Apple Silicon boot
    "52637672-7900-11AA-AA11-00306543ECAC", // Apple Silicon recovery
];

const CHROMEOS_SYSTEM: &[&str] = &[
    "FE3A2A5D-4F32-41A7-B725-ACCC3285A309", // kernel
    "3CB8E202-3B7E-47DD-8A3C-7FF2A13CFCEC", // root fs
];

/// Boot, root, swap and file system partitions of other systems (BSDs,
/// Solaris, Haiku, QNX).
const OTHER_SYSTEM: &[&str] = &[
    "83BD6B9D-7F41-11DC-BE0B-001560B84F0F", // FreeBSD boot
    "516E7CB4-6ECF-11D6-8FF8-00022D09712B", // FreeBSD data
    "516E7CB5-6ECF-11D6-8FF8-00022D09712B", // FreeBSD swap
    "516E7CB6-6ECF-11D6-8FF8-00022D09712B", // FreeBSD UFS
    "516E7CBA-6ECF-11D6-8FF8-00022D09712B", // FreeBSD ZFS
    "49F48D32-B10E-11DC-B99B-0019D1879648", // NetBSD swap
    "49F48D5A-B10E-11DC-B99B-0019D1879648", // NetBSD FFS
    "49F48D82-B10E-11DC-B99B-0019D1879648", // NetBSD LFS
    "824CC7A0-36A8-11E3-890A-952519AD3F61", // OpenBSD data
    "85D5E45E-237C-11E1-B4B3-E89A8F7FC3A7", // MidnightBSD boot
    "85D5E45A-237C-11E1-B4B3-E89A8F7FC3A7", // MidnightBSD data
    "0394EF8B-237E-11E1-B4B3-E89A8F7FC3A7", // MidnightBSD UFS
    "9D94CE7C-1CA5-11DC-8817-01301BB8A9F5", // DragonFlyBSD UFS1
    "6A82CB45-1DD2-11B2-99A6-080020736631", // Solaris boot
    "6A85CF4D-1DD2-11B2-99A6-080020736631", // Solaris root
    "6A87C46F-1DD2-11B2-99A6-080020736631", // Solaris swap
    "42465331-3BA3-10F1-802A-4861696B7521", // Haiku BFS
    "CEF5A9AD-73BC-4601-89F3-CDEEEEE321A1", // QNX6
];

/// Members of disk groups that another system assembles.
const GROUP_TYPES: &[&str] = &[
    "A19D880F-05FC-4D3B-A006-743F0F84911E", // Linux RAID
    "E6D6D379-F507-44C2-A23C-238F2A3DF928", // Linux LVM
    "5808C8AA-7E8F-42E0-85D2-E1E90434CFB3", // Microsoft LDM metadata
    "AF9B60A0-1431-4F62-BC68-3311714A69AD", // Microsoft LDM data
    "E75CAF8F-F680-4CEE-AFA3-B001E56EFC2D", // Microsoft Storage Spaces
    "52414944-0000-11AA-AA11-00306543ECAC", // Apple RAID
    "52414944-5F4F-11AA-AA11-00306543ECAC", // Apple RAID offline
    "516E7CB8-6ECF-11D6-8FF8-00022D09712B", // FreeBSD Vinum
    "4FBD7E29-9D25-41B8-AFD0-062C0CEFF05D", // Ceph OSD
    "4FBD7E29-9D25-41B8-AFD0-5EC00CEFF05D", // Ceph crypt OSD
    "AA31E02A-400F-11DB-9590-000C2911D1B8", // VMware VMFS
];

/// MBR flag byte of a bootable (active) partition.
const MBR_BOOT_FLAG: &str = "0x80";

// --------------------------------------------------------------- facts

/// How a disk is connected, for display and for the system-disk rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Sata,
    Nvme,
    Usb,
    /// SD card in a built-in reader, or built-in eMMC memory.
    Mmc,
    Virtio,
    Other,
}

/// The type of a partition, as the partition table names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartType {
    /// GPT type GUID, upper case.
    Gpt(String),
    /// MBR type byte.
    Mbr(u8),
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partition {
    /// Kernel name, e.g. `nvme0n1p3`.
    pub name: String,
    pub size_bytes: u64,
    pub part_type: PartType,
    /// GPT partition label (`PARTLABEL`); empty when none.
    pub label: String,
    /// File system or signature found inside (`FSTYPE`); empty when unknown.
    pub fstype: String,
    /// File system label (`LABEL`); empty when none.
    pub fs_label: String,
    /// MBR active flag.
    pub boot_flag: bool,
    /// The partition or anything opened on top of it is mounted.
    pub mounted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disk {
    /// Kernel name, e.g. `sda` (`/dev/<name>`).
    pub name: String,
    /// Model for display; "Unknown disk" when the disk does not say.
    pub model: String,
    pub size_bytes: u64,
    pub transport: Transport,
    /// Removable medium or hot-pluggable bus.
    pub removable: bool,
    pub rotational: bool,
    pub read_only: bool,
    /// `gpt`, `dos`, ...; empty when the disk has no partition table.
    pub table: String,
    /// Signature on the whole disk (no partition table), e.g. `iso9660`.
    pub fstype: String,
    pub fs_label: String,
    pub partitions: Vec<Partition>,
    /// The disk, a partition or anything opened on top is mounted.
    pub mounted: bool,
}

impl Disk {
    /// Partitions whose contents lsblk could not name: the caller may run
    /// `blkid -p -o export /dev/<name>` on them and pass the result to
    /// [`Partition::fill_from_blkid`]. (That wakes the disk; lsblk does not.)
    pub fn partitions_to_probe(&self) -> Vec<&str> {
        self.partitions
            .iter()
            .filter(|p| p.fstype.is_empty())
            .map(|p| p.name.as_str())
            .collect()
    }

    /// Stays in the computer: not USB, not a memory card, not removable.
    pub fn is_internal(&self) -> bool {
        !self.removable && !matches!(self.transport, Transport::Usb | Transport::Mmc)
    }
}

/// `KEY=value` lines of `blkid -o export`.
pub fn parse_blkid_export(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

impl Partition {
    /// Takes the file system type and label from `blkid -p -o export`
    /// output when lsblk had none. Never overwrites what lsblk found.
    pub fn fill_from_blkid(&mut self, export: &str) {
        let kv = parse_blkid_export(export);
        let get = |k: &str| kv.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
        if self.fstype.is_empty() {
            self.fstype = get("TYPE").unwrap_or_default();
            if self.fs_label.is_empty() {
                self.fs_label = get("LABEL").unwrap_or_default();
            }
        }
    }
}

// --------------------------------------------------------------- parse

/// A text field; numbers are accepted too (old lsblk printed strings).
fn text(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// A size in bytes: a number (lsblk >= 2.33 with `--bytes`) or a string.
fn bytes(v: &Value, key: &str) -> u64 {
    match v.get(key) {
        Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
        Some(Value::String(s)) => s.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

/// A flag: JSON boolean, or `"1"`/`"0"`/`"true"`/`"false"` in old output.
fn flag(v: &Value, key: &str) -> bool {
    match v.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_u64() == Some(1),
        Some(Value::String(s)) => matches!(s.trim(), "1" | "true"),
        _ => false,
    }
}

/// Mounted: any non-null entry in `mountpoints` (or old `mountpoint`).
fn mounted_here(v: &Value) -> bool {
    let set = |m: &Value| matches!(m, Value::String(s) if !s.is_empty());
    match v.get("mountpoints") {
        Some(Value::Array(a)) => a.iter().any(set),
        _ => v.get("mountpoint").is_some_and(set),
    }
}

fn children(v: &Value) -> &[Value] {
    v.get("children")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// This node or anything below it is mounted.
fn mounted_tree(v: &Value) -> bool {
    mounted_here(v) || children(v).iter().any(mounted_tree)
}

/// Kernel names that are not connected disks, even when lsblk calls them
/// `disk` (zram does), and eMMC hardware boot areas.
fn not_a_disk(name: &str) -> bool {
    ["loop", "zram", "ram", "nbd", "md", "dm-", "sr", "fd"]
        .iter()
        .any(|p| name.starts_with(p))
        || (name.starts_with("mmcblk") && (name.contains("boot") || name.ends_with("rpmb")))
}

fn transport(tran: &str, name: &str) -> Transport {
    match tran {
        "sata" | "ata" => Transport::Sata,
        "nvme" => Transport::Nvme,
        "usb" => Transport::Usb,
        "mmc" => Transport::Mmc,
        "virtio" => Transport::Virtio,
        _ if name.starts_with("nvme") => Transport::Nvme,
        _ if name.starts_with("mmcblk") => Transport::Mmc,
        _ if name.starts_with("vd") => Transport::Virtio,
        _ => Transport::Other,
    }
}

fn part_type(raw: &str) -> PartType {
    if let Some(hex) = raw.strip_prefix("0x") {
        return u8::from_str_radix(hex, 16).map_or(PartType::Unknown, PartType::Mbr);
    }
    if raw.len() == 36 && raw.bytes().filter(|b| *b == b'-').count() == 4 {
        return PartType::Gpt(raw.to_ascii_uppercase());
    }
    PartType::Unknown
}

fn partition(v: &Value) -> Partition {
    Partition {
        name: text(v, "name"),
        size_bytes: bytes(v, "size"),
        part_type: part_type(&text(v, "parttype")),
        label: text(v, "partlabel"),
        fstype: text(v, "fstype"),
        fs_label: text(v, "label"),
        boot_flag: text(v, "partflags").eq_ignore_ascii_case(MBR_BOOT_FLAG),
        mounted: mounted_tree(v),
    }
}

fn disk(v: &Value) -> Disk {
    let name = text(v, "name");
    let transport = transport(&text(v, "tran").to_ascii_lowercase(), &name);
    let model = text(v, "model")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let model = match (model.is_empty(), transport) {
        (false, _) => model,
        (true, Transport::Mmc) => "Memory card".into(),
        (true, _) => "Unknown disk".into(),
    };
    Disk {
        model,
        size_bytes: bytes(v, "size"),
        transport,
        removable: flag(v, "rm") || flag(v, "hotplug"),
        rotational: flag(v, "rota"),
        read_only: flag(v, "ro"),
        table: text(v, "pttype"),
        fstype: text(v, "fstype"),
        fs_label: text(v, "label"),
        partitions: children(v)
            .iter()
            .filter(|c| text(c, "type") == "part")
            .map(partition)
            .collect(),
        mounted: mounted_tree(v),
        name,
    }
}

/// Every connected disk in `lsblk --json --bytes --tree` output, in lsblk's
/// order. Refuses (`ALP-INST-400`) output that is not lsblk's JSON.
pub fn parse_lsblk(json: &str) -> Result<Vec<Disk>> {
    let fail = |detail: String| HwError {
        code: CODE_DISK_LIST,
        detail,
    };
    let v: Value = serde_json::from_str(json).map_err(|e| fail(format!("lsblk JSON: {e}")))?;
    let list = v
        .get("blockdevices")
        .and_then(Value::as_array)
        .ok_or_else(|| fail("lsblk JSON has no blockdevices array".into()))?;
    Ok(list
        .iter()
        .filter(|d| text(d, "type") == "disk" && !not_a_disk(&text(d, "name")))
        .map(disk)
        .collect())
}

// ------------------------------------------------------------- systems

/// An operating system found on a disk. Ordered from most to least
/// specific; the most specific finding is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum System {
    AlpOs,
    Windows,
    MacOs,
    Linux,
    ChromeOs,
    /// BSD, Solaris, Haiku, QNX.
    Other,
    /// A start stick or disc image of some system (`iso9660`, `udf`).
    StartMedia,
    /// A boot partition (EFI, or an MBR boot flag) of a system not named.
    Unknown,
}

fn gpt(p: &Partition) -> Option<&str> {
    match &p.part_type {
        PartType::Gpt(g) => Some(g.as_str()),
        _ => None,
    }
}

fn mbr(p: &Partition) -> Option<u8> {
    match p.part_type {
        PartType::Mbr(b) => Some(b),
        _ => None,
    }
}

fn in_list(g: Option<&str>, list: &[&str]) -> bool {
    g.is_some_and(|g| list.contains(&g))
}

fn system_by_fstype(fstype: &str) -> Option<System> {
    match fstype {
        "BitLocker" | "ReFS" => Some(System::Windows),
        "apfs" | "hfsplus" | "hfs" | "cs_fvault2" => Some(System::MacOs),
        "swap" | "swsuspend" | "DM_verity_hash" | "erofs" | "squashfs" => Some(System::Linux),
        "iso9660" | "udf" => Some(System::StartMedia),
        _ => None,
    }
}

/// The systems one partition shows, given facts about its disk.
fn partition_systems(p: &Partition, windows_disk: bool, out: &mut Vec<System>) {
    let g = gpt(p);
    let m = mbr(p);
    if p.label == "alp-state" || (in_list(g, LINUX_SYSTEM) && p.label.starts_with("alp-os_")) {
        out.push(System::AlpOs);
    }
    if in_list(g, &[MS_RESERVED, WIN_RECOVERY]) || m == Some(0x27) {
        out.push(System::Windows);
    }
    if p.fstype == "ntfs" && (windows_disk || p.boot_flag) {
        out.push(System::Windows);
    }
    if in_list(g, APPLE_SYSTEM) || matches!(m, Some(0xab | 0xaf)) {
        out.push(System::MacOs);
    }
    if in_list(g, LINUX_SYSTEM) || g == Some(BIOS_BOOT) || m == Some(0x82) {
        out.push(System::Linux);
    }
    if m == Some(0x83) && p.boot_flag {
        out.push(System::Linux);
    }
    if in_list(g, CHROMEOS_SYSTEM) {
        out.push(System::ChromeOs);
    }
    if in_list(g, OTHER_SYSTEM) || matches!(m, Some(0xa5 | 0xa6 | 0xa9 | 0xbf)) {
        out.push(System::Other);
    }
    if let Some(s) = system_by_fstype(&p.fstype) {
        out.push(s);
    }
    // EFI system partition, a vendor tool partition, any active MBR
    // partition, or a FAT partition named like a boot partition.
    let boot_name = matches!(
        p.fs_label.to_ascii_lowercase().as_str(),
        "boot" | "bootfs" | "system-boot" | "efi"
    );
    if g == Some(ESP)
        || matches!(m, Some(0xef | 0xde | 0x12))
        || p.boot_flag
        || (p.fstype == "vfat" && boot_name)
    {
        out.push(System::Unknown);
    }
}

/// An install of ALP-OS that stopped before its last step: the ESP still
/// carries [`UNFINISHED_LABEL`], every partition is one the installer makes,
/// and nothing is encrypted (a finished system's index and archive are
/// LUKS; an unfinished one never got that far). The next try may erase it
/// (strong confirmation), instead of the disk being greyed out for good.
pub fn unfinished_install(d: &Disk) -> bool {
    let ours = |p: &Partition| {
        let l = p.label.as_str();
        (l == UNFINISHED_LABEL && gpt(p) == Some(ESP))
            || (l.starts_with("alp-os_") && in_list(gpt(p), LINUX_SYSTEM))
            || (l == "_empty" && in_list(gpt(p), LINUX_SYSTEM))
            || matches!(l, "alp-state" | "alp-index" | "alp-data")
    };
    d.table == "gpt"
        && d.fstype.is_empty()
        && d.partitions.iter().any(|p| p.label == UNFINISHED_LABEL)
        && d.partitions
            .iter()
            .all(|p| ours(p) && p.fstype != "crypto_LUKS" && !is_group_member(p) && !p.mounted)
}

/// A disk of an ALP-OS archive that a new install may open again with the
/// recovery words (docs/plans/0.2.8-reopen-archive.md): every partition is
/// an `alp-data` partition holding the pool's LUKS container (label
/// `alp-pool`). It stays "not usable" (never erased); the plan may only name
/// it to reopen.
pub fn reopenable(d: &Disk) -> bool {
    d.table == "gpt"
        && !d.mounted
        && !d.partitions.is_empty()
        && d.partitions.iter().all(|p| {
            p.label == "alp-data"
                && p.fstype == "crypto_LUKS"
                && p.fs_label == "alp-pool"
                && !p.mounted
        })
}

/// The most specific operating system on the disk, if any (plan (b)).
pub fn find_system(d: &Disk) -> Option<System> {
    let windows_disk = d
        .partitions
        .iter()
        .any(|p| in_list(gpt(p), &[ESP, MS_RESERVED, WIN_RECOVERY]));
    let mut found = Vec::new();
    if let Some(s) = system_by_fstype(&d.fstype) {
        found.push(s);
    }
    for p in &d.partitions {
        partition_systems(p, windows_disk, &mut found);
    }
    found.into_iter().min()
}

fn is_group_member(p: &Partition) -> bool {
    in_list(gpt(p), GROUP_TYPES)
        || matches!(mbr(p), Some(0x8e | 0xfd))
        || is_group_fstype(&p.fstype)
}

fn is_group_fstype(f: &str) -> bool {
    f.ends_with("_raid_member")
        || matches!(
            f,
            "LVM2_member"
                | "LVM1_member"
                | "zfs_member"
                | "bcache"
                | "ceph_bluestore"
                | "drbd"
                | "VMFS_volume_member"
                | "stratis"
                | "vdo"
        )
}

// ------------------------------------------------------------ verdicts

/// Sizes from the hardware profile (`min_single_disk_bytes`,
/// `min_system_disk_bytes`) and the data disk minimum (open question in the
/// plan: the profile has none yet).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub system_and_data_bytes: u64,
    pub system_bytes: u64,
    pub data_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Context<'a> {
    /// Kernel name of the disk ALP-OS runs from (from `alp_hw::inspect`),
    /// or `None` when it was not found.
    pub boot_disk: Option<&'a str>,
    pub limits: Limits,
}

/// What a usable disk can be used for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The only disk: system and archive on it.
    SystemAndData,
    /// The system disk next to separate data disks.
    System,
    /// A data disk of the archive.
    Data,
}

/// What is on a usable disk now, so the confirm screen can say exactly
/// what will be erased.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Contents {
    Empty,
    /// One plain line per partition or whole-disk file system, e.g.
    /// `ntfs "Photos" (4000 GB)`.
    Files(Vec<String>),
}

impl Contents {
    /// Using this disk erases something on it: the confirm screen shows
    /// the strong warning naming it and asks for the slide (owner decision
    /// 2026-11-24). Disks with an operating system never get this far.
    pub fn will_erase_files(&self) -> bool {
        matches!(self, Contents::Files(_))
    }
}

/// Why a disk cannot be used. One per disk: the first that applies, in
/// this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    BootStick,
    InUse,
    NoMedium,
    ReadOnly,
    HoldsSystem(System),
    DiskGroup,
    AlpArchive,
    Encrypted,
    TooSmall {
        size_bytes: u64,
        needed_bytes: u64,
    },
    /// A USB or removable disk while the start stick was not found.
    StartStickUnknown,
}

impl Reason {
    /// Stable key for the translation files (spec 2.2).
    pub fn key(&self) -> &'static str {
        match self {
            Reason::BootStick => "boot_stick",
            Reason::InUse => "in_use",
            Reason::NoMedium => "no_medium",
            Reason::ReadOnly => "read_only",
            Reason::HoldsSystem(s) => match s {
                System::AlpOs => "holds_alp_os",
                System::Windows => "holds_windows",
                System::MacOs => "holds_macos",
                System::Linux => "holds_linux",
                System::ChromeOs => "holds_chromeos",
                System::Other => "holds_other_system",
                System::StartMedia => "holds_start_media",
                System::Unknown => "holds_unknown_system",
            },
            Reason::DiskGroup => "disk_group",
            Reason::AlpArchive => "alp_archive",
            Reason::Encrypted => "encrypted",
            Reason::TooSmall { .. } => "too_small",
            Reason::StartStickUnknown => "start_stick_unknown",
        }
    }

    /// English fallback text (the screen uses the translation of `key`).
    pub fn text(&self) -> String {
        let system = |what: &str| {
            format!("Holds {what}. ALP-OS does not erase a disk with an operating system.")
        };
        match self {
            Reason::BootStick => "This is the USB stick ALP-OS started from.".into(),
            Reason::InUse => "In use right now.".into(),
            Reason::NoMedium => "No card or disk inserted.".into(),
            Reason::ReadOnly => "Write-protected (check the lock switch on the card).".into(),
            Reason::HoldsSystem(s) => system(match s {
                System::AlpOs => "an ALP-OS system",
                System::Windows => "Windows",
                System::MacOs => "macOS",
                System::Linux => "Linux",
                System::ChromeOs => "ChromeOS",
                System::Other => "another operating system",
                System::StartMedia => "a start-up stick or disc of another system",
                System::Unknown => "a start-up area of another system",
            }),
            Reason::DiskGroup => "Part of a disk group another system uses.".into(),
            Reason::AlpArchive => "Already holds an ALP-OS archive.".into(),
            Reason::Encrypted => "Holds encrypted data ALP-OS cannot look inside.".into(),
            Reason::TooSmall {
                size_bytes,
                needed_bytes,
            } => format!(
                "Too small: {}, at least {} needed.",
                size_text(*size_bytes),
                size_text(*needed_bytes)
            ),
            Reason::StartStickUnknown => {
                "ALP-OS could not tell which USB stick it started from, so it uses no USB disk."
                    .into()
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Usable {
        roles: Vec<Role>,
        contents: Contents,
    },
    NotUsable(Reason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assessed {
    pub disk: Disk,
    pub verdict: Verdict,
}

/// Decimal size, as disk makers count: GB below 1 TB, TB above.
pub fn size_text(bytes: u64) -> String {
    if bytes >= 1_000_000_000_000 {
        let t = bytes as f64 / 1e12;
        if (t - t.round()).abs() < 0.05 {
            format!("{t:.0} TB")
        } else {
            format!("{t:.1} TB")
        }
    } else {
        super::gb(bytes)
    }
}

fn contents(d: &Disk) -> Contents {
    let describe = |fstype: &str, label: &str, size: u64| {
        let what = if fstype.is_empty() {
            "unknown contents"
        } else {
            fstype
        };
        if label.is_empty() {
            format!("{what} ({})", size_text(size))
        } else {
            format!("{what} \"{label}\" ({})", size_text(size))
        }
    };
    let mut v: Vec<String> = d
        .partitions
        .iter()
        .map(|p| describe(&p.fstype, &p.fs_label, p.size_bytes))
        .collect();
    if v.is_empty() && !d.fstype.is_empty() {
        v.push(describe(&d.fstype, &d.fs_label, d.size_bytes));
    }
    if v.is_empty() {
        Contents::Empty
    } else {
        Contents::Files(v)
    }
}

fn verdict(d: &Disk, ctx: &Context, stick_found: bool) -> Verdict {
    let no = Verdict::NotUsable;
    if stick_found && ctx.boot_disk == Some(d.name.as_str()) {
        return no(Reason::BootStick);
    }
    if d.mounted {
        return no(Reason::InUse);
    }
    if d.size_bytes == 0 {
        return no(Reason::NoMedium);
    }
    if d.read_only {
        return no(Reason::ReadOnly);
    }
    let unfinished = unfinished_install(d);
    if !unfinished && let Some(s) = find_system(d) {
        return no(Reason::HoldsSystem(s));
    }
    if is_group_fstype(&d.fstype) || d.partitions.iter().any(is_group_member) {
        return no(Reason::DiskGroup);
    }
    if !unfinished && d.partitions.iter().any(|p| p.label == "alp-data") {
        return no(Reason::AlpArchive);
    }
    if d.fstype == "crypto_LUKS" || d.partitions.iter().any(|p| p.fstype == "crypto_LUKS") {
        return no(Reason::Encrypted);
    }
    let lim = ctx.limits;
    let internal = d.is_internal();
    let mut roles = Vec::new();
    if internal && d.size_bytes >= lim.system_and_data_bytes {
        roles.push(Role::SystemAndData);
    }
    if internal && d.size_bytes >= lim.system_bytes {
        roles.push(Role::System);
    }
    if d.size_bytes >= lim.data_bytes {
        roles.push(Role::Data);
    }
    if roles.is_empty() {
        let needed = if internal {
            lim.system_bytes.min(lim.data_bytes)
        } else {
            lim.data_bytes
        };
        return no(Reason::TooSmall {
            size_bytes: d.size_bytes,
            needed_bytes: needed,
        });
    }
    if !stick_found && !internal {
        return no(Reason::StartStickUnknown);
    }
    Verdict::Usable {
        roles,
        contents: if unfinished {
            Contents::Files(vec!["an unfinished ALP-OS install".into()])
        } else {
            contents(d)
        },
    }
}

/// A verdict for every disk (plan (a)), in the order given.
pub fn assess(disks: Vec<Disk>, ctx: &Context) -> Vec<Assessed> {
    let stick_found = ctx
        .boot_disk
        .is_some_and(|b| disks.iter().any(|d| d.name == b));
    disks
        .into_iter()
        .map(|disk| Assessed {
            verdict: verdict(&disk, ctx, stick_found),
            disk,
        })
        .collect()
}

/// Parse lsblk's JSON and assess every disk.
pub fn discover(lsblk_json: &str, ctx: &Context) -> Result<Vec<Assessed>> {
    Ok(assess(parse_lsblk(lsblk_json)?, ctx))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1_000_000_000;
    const TB: u64 = 1_000 * GB;

    fn ctx(boot: Option<&str>) -> Context<'_> {
        Context {
            boot_disk: boot,
            limits: Limits {
                system_and_data_bytes: 64 * GB,
                system_bytes: 32 * GB,
                data_bytes: 32 * GB,
            },
        }
    }

    fn fixture(name: &str) -> &'static str {
        match name {
            "blank" => include_str!("../../tests/fixtures/lsblk/blank.json"),
            // util-linux 2.41 (trixie): an unmounted disk has `mountpoints: []`.
            "blank-trixie" => include_str!("../../tests/fixtures/lsblk/blank-trixie.json"),
            "windows" => include_str!("../../tests/fixtures/lsblk/windows.json"),
            "linux" => include_str!("../../tests/fixtures/lsblk/linux.json"),
            "boot-stick" => include_str!("../../tests/fixtures/lsblk/boot-stick.json"),
            "sd-card" => include_str!("../../tests/fixtures/lsblk/sd-card.json"),
            "data-only" => include_str!("../../tests/fixtures/lsblk/data-only.json"),
            "alp" => include_str!("../../tests/fixtures/lsblk/alp.json"),
            "others" => include_str!("../../tests/fixtures/lsblk/others.json"),
            "old-strings" => include_str!("../../tests/fixtures/lsblk/old-strings.json"),
            _ => unreachable!("no fixture {name}"),
        }
    }

    fn run(name: &str, boot: Option<&str>) -> Vec<Assessed> {
        discover(fixture(name), &ctx(boot)).expect("fixture parses")
    }

    fn verdict_of<'a>(list: &'a [Assessed], disk: &str) -> &'a Verdict {
        &list
            .iter()
            .find(|a| a.disk.name == disk)
            .unwrap_or_else(|| panic!("{disk} not listed"))
            .verdict
    }

    fn reason_of(list: &[Assessed], disk: &str) -> Reason {
        match verdict_of(list, disk) {
            Verdict::NotUsable(r) => *r,
            v => panic!("{disk} should not be usable: {v:?}"),
        }
    }

    fn usable(list: &[Assessed], disk: &str) -> (Vec<Role>, Contents) {
        match verdict_of(list, disk) {
            Verdict::Usable { roles, contents } => (roles.clone(), contents.clone()),
            v => panic!("{disk} should be usable: {v:?}"),
        }
    }

    #[test]
    fn trixie_empty_mountpoints_is_unmounted() {
        let l = run("blank-trixie", Some("sdz"));
        let (roles, _) = usable(&l, "sda");
        assert_eq!(roles, vec![Role::SystemAndData, Role::System, Role::Data]);
    }

    #[test]
    fn blank_internal_disk_takes_every_role() {
        let l = run("blank", Some("sdz"));
        assert_eq!(l.len(), 1);
        let d = &l[0].disk;
        assert_eq!(d.model, "Samsung SSD 870 EVO 500GB");
        assert_eq!(d.transport, Transport::Sata);
        assert_eq!(d.size_bytes, 500_107_862_016);
        // The start stick "sdz" is not in this list: internal disks stay usable.
        let (roles, contents) = usable(&l, "sda");
        assert_eq!(roles, vec![Role::SystemAndData, Role::System, Role::Data]);
        assert_eq!(contents, Contents::Empty);
    }

    #[test]
    fn windows_disks_are_never_offered() {
        let l = run("windows", Some("sdb"));
        // ESP + Microsoft reserved + NTFS + recovery.
        assert_eq!(
            reason_of(&l, "nvme0n1"),
            Reason::HoldsSystem(System::Windows)
        );
        // BitLocker volume only (GPT basic data), no ESP on that disk.
        assert_eq!(reason_of(&l, "sda"), Reason::HoldsSystem(System::Windows));
        // Legacy MBR install: NTFS with the boot flag.
        assert_eq!(reason_of(&l, "sdc"), Reason::HoldsSystem(System::Windows));
        assert_eq!(reason_of(&l, "sda").key(), "holds_windows");
        assert!(reason_of(&l, "sda").text().contains("Windows"));
    }

    #[test]
    fn linux_disks_are_never_offered() {
        let l = run("linux", Some("sdx"));
        // ESP + ext4 on "Linux filesystem" + swap.
        assert_eq!(reason_of(&l, "sda"), Reason::HoldsSystem(System::Linux));
        // Discoverable x86-64 root partition, ESP on another disk.
        assert_eq!(reason_of(&l, "sdb"), Reason::HoldsSystem(System::Linux));
        // ESP alone on an otherwise blank disk: an unknown system (fail safe).
        assert_eq!(reason_of(&l, "sdc"), Reason::HoldsSystem(System::Unknown));
    }

    #[test]
    fn the_boot_stick_is_never_offered() {
        let l = run("boot-stick", Some("sdb"));
        assert_eq!(l.len(), 3);
        assert_eq!(reason_of(&l, "sdb"), Reason::BootStick);
        assert_eq!(usable(&l, "sda").0[0], Role::SystemAndData);
        // A second USB disk: data only, never the system disk.
        assert_eq!(usable(&l, "sdc").0, vec![Role::Data]);
    }

    #[test]
    fn without_the_boot_stick_known_usb_disks_are_refused() {
        let l = run("boot-stick", None);
        // The stick is still caught: its root is mounted.
        assert_eq!(reason_of(&l, "sdb"), Reason::InUse);
        assert_eq!(reason_of(&l, "sdc"), Reason::StartStickUnknown);
        assert!(matches!(verdict_of(&l, "sda"), Verdict::Usable { .. }));
        // A boot disk name that is not in the list counts as not found.
        let l = run("boot-stick", Some("sdq"));
        assert_eq!(reason_of(&l, "sdc"), Reason::StartStickUnknown);
    }

    #[test]
    fn sd_cards_are_listed_with_a_role_or_a_reason() {
        let l = run("sd-card", Some("sdb"));
        let names: Vec<_> = l.iter().map(|a| a.disk.name.as_str()).collect();
        // eMMC boot areas are not disks.
        assert_eq!(names, vec!["mmcblk0", "sdb", "sdc", "mmcblk1"]);
        let card = &l[0].disk;
        assert_eq!(card.transport, Transport::Mmc);
        assert_eq!(card.model, "Memory card");
        let (roles, contents) = usable(&l, "mmcblk0");
        assert_eq!(roles, vec![Role::Data]);
        assert_eq!(
            contents,
            Contents::Files(vec!["vfat \"CAMERA\" (64 GB)".into()])
        );
        // USB card reader without a card.
        assert_eq!(reason_of(&l, "sdc"), Reason::NoMedium);
        // Write-protected card.
        assert_eq!(reason_of(&l, "mmcblk1"), Reason::ReadOnly);
    }

    #[test]
    fn a_disk_with_only_a_data_partition_is_usable_and_says_what_is_on_it() {
        let l = run("data-only", Some("sda"));
        let (roles, contents) = usable(&l, "sdb");
        assert_eq!(roles, vec![Role::Data]);
        assert_eq!(
            contents,
            Contents::Files(vec!["ntfs \"Photos\" (4 TB)".into()])
        );
        // 20 TB internal disk with an old ext4 backup: tens of TB fit in u64.
        let (roles, contents) = usable(&l, "sdc");
        assert_eq!(roles, vec![Role::SystemAndData, Role::System, Role::Data]);
        assert_eq!(
            contents,
            Contents::Files(vec!["ext4 \"backup\" (20 TB)".into()])
        );
        assert_eq!(l[2].disk.size_bytes, 20 * TB);
        // A partition lsblk could not name is listed for a blkid probe.
        let probe: Vec<_> = l[3].disk.partitions_to_probe();
        assert_eq!(probe, vec!["sdd1"]);
        let mut d = l[3].disk.clone();
        d.partitions[0].fill_from_blkid("DEVNAME=/dev/sdd1\nTYPE=BitLocker\nPART_ENTRY_NAME=x\n");
        assert_eq!(find_system(&d), Some(System::Windows));
    }

    #[test]
    fn alp_disks_are_never_erased() {
        let l = run("alp", Some("sdc"));
        assert_eq!(reason_of(&l, "sda"), Reason::HoldsSystem(System::AlpOs));
        // alp-data holds LUKS: the archive reason wins over "encrypted".
        assert_eq!(reason_of(&l, "sdb"), Reason::AlpArchive);
    }

    #[test]
    fn an_archive_disk_can_be_reopened_never_erased() {
        let with_label = fixture("alp").replacen(
            "\"partlabel\": \"alp-data\", \"partflags\": null, \"fstype\": \"crypto_LUKS\", \"label\": null",
            "\"partlabel\": \"alp-data\", \"partflags\": null, \"fstype\": \"crypto_LUKS\", \"label\": \"alp-pool\"",
            1,
        );
        let l = discover(&with_label, &ctx(Some("sdc"))).unwrap();
        let sdb = l.iter().find(|a| a.disk.name == "sdb").unwrap();
        assert_eq!(sdb.verdict, Verdict::NotUsable(Reason::AlpArchive));
        assert!(reopenable(&sdb.disk));
        // Without the pool's LUKS label: some other encrypted data.
        let l = run("alp", Some("sdc"));
        assert!(!reopenable(
            &l.iter().find(|a| a.disk.name == "sdb").unwrap().disk
        ));
        // The system disk is never an archive to reopen.
        assert!(!reopenable(
            &l.iter().find(|a| a.disk.name == "sda").unwrap().disk
        ));
    }

    #[test]
    fn an_unfinished_install_can_be_erased_again() {
        let unfinished = fixture("alp")
            .replacen(
                "\"partlabel\": \"esp\"",
                "\"partlabel\": \"alp-installing\"",
                1,
            )
            .replacen(
                "\"partlabel\": \"alp-state\", \"partflags\": null, \"fstype\": \"crypto_LUKS\"",
                "\"partlabel\": \"alp-state\", \"partflags\": null, \"fstype\": \"ext4\"",
                1,
            );
        let l = discover(&unfinished, &ctx(Some("sdc"))).unwrap();
        let (_, contents) = usable(&l, "sda");
        assert!(contents.will_erase_files(), "strong confirmation");
        // The mark alone is not enough: something encrypted stays greyed.
        let luks = fixture("alp").replacen(
            "\"partlabel\": \"esp\"",
            "\"partlabel\": \"alp-installing\"",
            1,
        );
        let l = discover(&luks, &ctx(Some("sdc"))).unwrap();
        assert_eq!(reason_of(&l, "sda"), Reason::HoldsSystem(System::AlpOs));
        // A finished system (ESP labelled esp) stays greyed.
        let l = run("alp", Some("sdc"));
        assert_eq!(reason_of(&l, "sda"), Reason::HoldsSystem(System::AlpOs));
    }

    #[test]
    fn other_systems_groups_and_encrypted_disks_are_refused() {
        let l = run("others", Some("sdz"));
        let names: Vec<_> = l.iter().map(|a| a.disk.name.as_str()).collect();
        // loop, zram and the CD drive are not connected disks.
        assert_eq!(names, vec!["sda", "sdb", "sdc", "sdd", "sde", "sdf", "sdg"]);
        assert_eq!(reason_of(&l, "sda"), Reason::HoldsSystem(System::MacOs));
        assert_eq!(reason_of(&l, "sdb"), Reason::DiskGroup); // LVM
        assert_eq!(reason_of(&l, "sdc"), Reason::DiskGroup); // Linux RAID
        assert_eq!(reason_of(&l, "sdd"), Reason::Encrypted); // LUKS
        assert_eq!(
            reason_of(&l, "sde"),
            Reason::HoldsSystem(System::StartMedia)
        ); // another installer stick (hybrid ISO)
        assert_eq!(
            reason_of(&l, "sdf"),
            Reason::TooSmall {
                size_bytes: 16 * GB,
                needed_bytes: 32 * GB
            }
        );
        assert_eq!(
            reason_of(&l, "sdf").text(),
            "Too small: 16 GB, at least 32 GB needed."
        );
        // Internal 40 GB: a system disk next to data disks, not a single disk.
        assert_eq!(usable(&l, "sdg").0, vec![Role::System, Role::Data]);
    }

    #[test]
    fn old_lsblk_output_with_strings_parses_the_same() {
        let l = run("old-strings", Some("sdb"));
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].disk.size_bytes, 500_107_862_016);
        assert!(!l[0].disk.removable);
        assert!(l[1].disk.removable);
        assert_eq!(reason_of(&l, "sda"), Reason::HoldsSystem(System::Windows));
        assert_eq!(reason_of(&l, "sdb"), Reason::BootStick);
    }

    #[test]
    fn broken_output_is_refused_with_the_installer_code() {
        for bad in ["", "not json", "{}", "{\"blockdevices\": 3}"] {
            assert_eq!(parse_lsblk(bad).unwrap_err().code, "ALP-INST-400");
        }
    }

    #[test]
    fn type_lists_hold_well_formed_upper_case_guids() {
        let all = [
            LINUX_SYSTEM,
            APPLE_SYSTEM,
            CHROMEOS_SYSTEM,
            OTHER_SYSTEM,
            GROUP_TYPES,
            &[ESP, BIOS_BOOT, MS_RESERVED, WIN_RECOVERY],
        ];
        for g in all.iter().flat_map(|l| l.iter()) {
            assert_eq!(part_type(g), PartType::Gpt((*g).to_string()), "{g}");
            assert_eq!(*g, g.to_ascii_uppercase());
        }
    }

    #[test]
    fn sizes_read_like_disk_labels() {
        assert_eq!(size_text(500_107_862_016), "500 GB");
        assert_eq!(size_text(4 * TB), "4 TB");
        assert_eq!(size_text(18_500 * GB), "18.5 TB");
    }
}
