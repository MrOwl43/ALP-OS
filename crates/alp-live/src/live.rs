// SPDX-License-Identifier: AGPL-3.0-or-later
//! ALP-OS Live (ADR 0006): the hardware report screen and the guard that
//! keeps the computer's own disks read-only while Live runs.

use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use alp_hw::inspect::{self, Check, HwReport, Minimums, NicKind, Status};
use alp_hw::{DiskInfo, DiskTransport};

/// GPT label of the small FAT partition on the stick that holds reports
/// (image/repart.d/30-live-report.conf). Windows shows it as a drive.
pub const REPORT_LABEL: &str = "ALP-REPORT";
const REPORT_MOUNT: &str = "/run/alp/report";
const SCREEN_FILE: &str = "/run/alp/screen.txt";
const PROFILE: &str = "/usr/share/alp/profiles/generic-x86_64.toml";
/// Name of the report file saved on the stick (empty: not saved).
pub const REPORT_NAME_FILE: &str = "/run/alp/report-file";

/// `capacity_low_percent` from the hardware profile (default 10).
pub fn capacity_low_percent() -> u64 {
    fs::read_to_string(PROFILE)
        .ok()
        .and_then(|s| s.parse::<toml::Table>().ok())
        .and_then(|t| t.get("capacity_low_percent")?.as_integer())
        .and_then(|v| u64::try_from(v).ok())
        .unwrap_or(10)
}

/// The hardware check for the API: report and checks against the profile.
pub fn hardware() -> Result<(HwReport, Vec<Check>), String> {
    let min = fs::read_to_string(PROFILE)
        .map_err(|e| format!("cannot read {PROFILE}: {e}"))
        .and_then(|s| minimums(&s))?;
    let r = inspect::inspect(&inspect::Roots::system());
    let checks = inspect::assess(&r, min);
    Ok((r, checks))
}

/// Read the minimums from the hardware profile (profiles/*.toml). The
/// profile is the only place hardware limits are defined.
pub fn minimums(profile_toml: &str) -> Result<Minimums, String> {
    let t: toml::Table = profile_toml.parse().map_err(|e| format!("profile: {e}"))?;
    let get = |k: &str| {
        t.get(k)
            .and_then(toml::Value::as_integer)
            .and_then(|v| u64::try_from(v).ok())
            .ok_or(format!("profile: missing {k}"))
    };
    Ok(Minimums {
        ram_bytes: get("min_ram_bytes")?,
        single_disk_bytes: get("min_single_disk_bytes")?,
    })
}

/// `alpctl live protect-disks`: make every disk except the one ALP-OS runs
/// from read-only in the kernel, so nothing on this system can write to it.
/// If the system disk cannot be found, every disk is protected.
/// Safe to run again for each newly added disk (udev rule).
pub fn protect_disks() -> i32 {
    let report = inspect::inspect(&inspect::Roots::system());
    let known = report.disks.iter().any(|d| d.is_system_disk);
    // The one partition the archive is on (archive::allow_write): its disk
    // stays writable, every other partition of that disk read-only.
    // By name and disk id (review G2): a stale file never allows a disk
    // plugged in later under the same name.
    let allowed_text = fs::read_to_string(crate::archive::ALLOWED).unwrap_or_default();
    // A disk a T49 format job is writing (format::ALLOWED_FORMAT, by disk
    // id = name and diskseq): left alone while the job runs, only if it is
    // an external (USB) disk.
    let mut status = 0;
    // Every disk but the stick (ADR 0022); without the stick found, every
    // disk, and none counts as external.
    for (d, class) in crate::disks::classes(&report.disks) {
        if known && class == crate::disks::Class::Stick {
            continue;
        }
        if class == crate::disks::Class::External && crate::format::formatting(&d.id.0) {
            continue;
        }
        // A disk without partitions holds the archive itself: `allowed` is
        // the disk's own name then.
        let allowed =
            crate::archive::allowed_on(&allowed_text, &d.id.0, &crate::archive::disk_key(&d.id.0))
                .unwrap_or_default();
        let allowed = allowed.as_str();
        if !allowed.is_empty()
            && (allowed == d.id.0
                || (!allowed.contains('/')
                    && Path::new("/sys/block").join(&d.id.0).join(allowed).is_dir()))
        {
            for p in crate::archive::partitions()
                .iter()
                .filter(|p| p.disk == d.id.0 && p.name != allowed)
            {
                let dev = format!("/dev/{}", p.name);
                match Command::new("blockdev").args(["--setro", &dev]).output() {
                    Ok(o) if o.status.success() => {}
                    other => {
                        eprintln!(
                            "ALP-OS Live: could not make {dev} read-only: {}",
                            why(&other)
                        );
                        status = 1;
                    }
                }
            }
            continue;
        }
        note_hw_read_only(&d.id.0);
        let dev = format!("/dev/{}", d.id.0);
        match Command::new("blockdev").args(["--setro", &dev]).output() {
            Ok(o) if o.status.success() => println!("ALP-OS Live: {dev} is read-only"),
            other => {
                eprintln!(
                    "ALP-OS Live: could not make {dev} read-only: {}",
                    why(&other)
                );
                status = 1;
            }
        }
    }
    status
}

/// What a failed `blockdev` said (its exit status and message), for the log.
fn why(r: &std::io::Result<std::process::Output>) -> String {
    match r {
        Ok(o) => format!("{} {}", o.status, String::from_utf8_lossy(&o.stderr).trim()),
        Err(e) => e.to_string(),
    }
}

/// Before `--setro`: a disk that is read-only already and that no run of
/// `protect-disks` marked (the marker is written before `--setro`, and read
/// after the flag, so a run in parallel cannot be mistaken for it) was
/// write-protected before ALP-OS touched it; remember that, so the backup
/// disk list can say so (`archive::hw_read_only`).
fn note_hw_read_only(disk: &str) {
    use crate::archive::{HW_RO_DIR, PROTECTED_DIR, disk_key};
    let key = disk_key(disk);
    let ro = fs::read_to_string(Path::new("/sys/block").join(disk).join("ro"))
        .is_ok_and(|v| v.trim() == "1");
    let ours = Path::new(PROTECTED_DIR).join(&key).exists();
    if ro && !ours {
        let _ = fs::create_dir_all(HW_RO_DIR)
            .and_then(|()| fs::write(Path::new(HW_RO_DIR).join(&key), ""));
    }
    let _ = fs::create_dir_all(PROTECTED_DIR)
        .and_then(|()| fs::write(Path::new(PROTECTED_DIR).join(&key), ""));
}

/// `alpctl live splash`: shown on tty1 early in boot, so the screen is not
/// dark until the kiosk (or the text screen) appears (ported from PR #7).
pub fn splash() -> i32 {
    print!(
        "\x1b[H\x1b[2J\x1b[?25l\n\n {}\n\n ALPArchive is starting. This can take up to a minute.\n Nothing on this computer's disks is deleted or formatted.\n",
        paint("\x1b[1m", "ALPArchive", true)
    );
    let _ = std::io::stdout().flush();
    0
}

/// `alpctl live report`: inspect, save the report to the stick, print it
/// (journal) and prepare the screen for `alpctl live screen`.
pub fn report() -> i32 {
    let min = match fs::read_to_string(PROFILE)
        .map_err(|e| e.to_string())
        .and_then(|s| minimums(&s))
    {
        Ok(m) => m,
        Err(e) => {
            eprintln!("ALP-OS Live: cannot read {PROFILE}: {e}");
            return 1;
        }
    };
    let r = inspect::inspect(&inspect::Roots::system());
    let checks = inspect::assess(&r, min);
    let now = now_utc();
    let text = render(&r, &checks, &now, false);
    let saved = save(&r, &text, &now);
    let _ = fs::create_dir_all("/run/alp");
    let _ = fs::write("/run/alp/hwreport.txt", &text);
    let _ = fs::write(REPORT_NAME_FILE, saved.as_deref().unwrap_or(""));
    let mut screen = render(&r, &checks, &now, true);
    screen += &match &saved {
        Ok(name) => format!(
            "\n {} Report saved on the stick: drive {REPORT_LABEL}, file {name}\n",
            paint("\x1b[1;32m", "■", true)
        ),
        Err(e) => format!(
            "\n {} The report could not be saved on the stick ({e}).\n",
            paint("\x1b[1;33m", "■", true)
        ),
    };
    let _ = fs::write(SCREEN_FILE, &screen);
    println!("{text}");
    println!("SAVED: {}", saved.as_deref().unwrap_or("no"));
    let _ = std::io::stdout().flush();
    0
}

/// `alpctl live screen`: draw the saved report on tty1 and keep the home
/// network line current (cable, address). Runs until the computer stops.
pub fn screen() -> i32 {
    let report = fs::read_to_string(SCREEN_FILE)
        .unwrap_or_else(|_| " Hardware report not available.\n".into());
    // Lines typed on the keyboard arrive through a channel, so the screen
    // keeps updating while it waits for Enter.
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lines() {
            if tx.send(line.unwrap_or_default()).is_err() {
                break;
            }
        }
    });
    let mut last = String::new();
    loop {
        let net = network_status();
        if net != last {
            let _ = fs::write("/run/alp/network.txt", &net);
            let mut wifi_hint = String::new();
            if crate::wifi::device().is_some() {
                wifi_hint += " To connect to Wi-Fi, type w and press Enter.\n";
            }
            wifi_hint += &archive_line();
            print!(
                "\x1b[H\x1b[2J\x1b[?25l{report}\n{net}\n{wifi_hint} Nothing on this computer's disks is deleted or formatted.\n To finish, press the power button briefly; the computer turns off.\n"
            );
            let _ = std::io::stdout().flush();
            last = net;
        }
        if let Ok(line) = rx.recv_timeout(std::time::Duration::from_secs(2)) {
            match line.trim() {
                "w" | "W" if crate::wifi::device().is_some() => {
                    crate::wifi::setup(&mut || rx.recv().unwrap_or_default())
                }
                "a" | "A" => crate::archive::setup(&mut || rx.recv().unwrap_or_default()),
                _ => {}
            }
            last.clear();
        }
    }
}

/// Archive status line for the screen.
fn archive_line() -> String {
    if crate::archive::is_open() {
        format!(
            " {} Archive: open and encrypted.\n",
            paint("\x1b[1;32m", "■", true)
        )
    } else {
        " To choose where to keep your archive, type a and press Enter.\n".into()
    }
}

/// One network port (wired or Wi-Fi) as the screen and the API show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Port {
    pub name: String,
    pub wifi: bool,
    /// Cable in, or Wi-Fi joined.
    pub carrier: bool,
    /// Home-network IPv4 address.
    pub address: Option<String>,
}

/// Physical network ports and their state (sysfs and `ip`).
pub fn ports() -> Vec<Port> {
    let net = Path::new("/sys/class/net");
    let mut out = Vec::new();
    for name in fs::read_dir(net)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
    {
        let dir = net.join(&name);
        if !dir.join("device").exists() {
            continue;
        }
        let wifi = dir.join("wireless").exists() || dir.join("phy80211").exists();
        let carrier = fs::read_to_string(dir.join("carrier")).is_ok_and(|c| c.trim() == "1");
        let address = Command::new("ip")
            .args(["-4", "-o", "addr", "show", "dev", &name, "scope", "global"])
            .output()
            .ok()
            .and_then(|o| parse_ip_addr(&String::from_utf8_lossy(&o.stdout)));
        out.push(Port {
            name,
            wifi,
            carrier,
            address,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// First address in `ip -4 -o addr show` output.
pub fn parse_ip_addr(out: &str) -> Option<String> {
    let mut it = out.split_whitespace().skip_while(|w| *w != "inet");
    it.nth(1)
        .and_then(|a| a.split('/').next())
        .map(str::to_string)
}

/// The address phones should use: wired first, then Wi-Fi.
/// Every IPv4 address of the home network ports, wired first.
pub fn addresses(ports: &[Port]) -> Vec<String> {
    let mut v: Vec<&Port> = ports.iter().filter(|p| p.address.is_some()).collect();
    v.sort_by_key(|p| p.wifi);
    v.iter().filter_map(|p| p.address.clone()).collect()
}

pub fn main_address(ports: &[Port]) -> Option<String> {
    let mut v: Vec<&Port> = ports.iter().filter(|p| p.address.is_some()).collect();
    v.sort_by_key(|p| p.wifi);
    v.first().and_then(|p| p.address.clone())
}

/// One line per network port, wired or Wi-Fi: link state and IPv4 address.
fn network_status() -> String {
    let mut lines = Vec::new();
    for p in ports() {
        let kind = if p.wifi { "Wi-Fi" } else { "Home network" };
        let name = &p.name;
        lines.push(match (p.carrier, p.address) {
            (_, Some(a)) => format!(
                " {} {kind}: connected on {name}, address {a}",
                paint("\x1b[1;32m", "■", true)
            ),
            (true, None) => format!(
                " {} {kind}: {} on {name}, waiting for an address",
                paint("\x1b[1;33m", "■", true),
                if p.wifi { "joined" } else { "cable connected" }
            ),
            (false, None) => format!(
                " {} {kind}: {} {name}",
                paint("\x1b[2m", "■", true),
                if p.wifi {
                    "not connected on"
                } else {
                    "no cable in"
                }
            ),
        });
    }
    if lines.is_empty() {
        lines.push(format!(
            " {} Home network: no wired or Wi-Fi port works. Plug in a USB-Ethernet adapter and a network cable.",
            paint("\x1b[1;33m", "■", true)
        ));
    }
    lines.join("\n")
}

fn paint(code: &str, s: &str, color: bool) -> String {
    if color {
        format!("{code}{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

fn tag(s: Status, color: bool) -> String {
    match s {
        Status::Ok => paint("\x1b[1;32m", "[ OK ]", color),
        Status::Note => paint("\x1b[1;33m", "[NOTE]", color),
        Status::Fail => paint("\x1b[1;31m", "[FAIL]", color),
    }
}

/// The report, as text (`color` = false, saved to the file) or for the
/// screen (`color` = true: ANSI colours and box lines from the console
/// font's character set).
pub fn render(r: &HwReport, checks: &[Check], now: &str, color: bool) -> String {
    let mut o = String::new();
    let width = 74;
    let title = "ALPArchive  ·  Hardware check";
    if color {
        let _ = writeln!(o, " ┌{}┐", "─".repeat(width));
        let _ = writeln!(o, " │ \x1b[1m{title:<w$}\x1b[0m│", w = width - 1);
        let _ = writeln!(
            o,
            " │ {:<w$}│",
            format!("ALP-OS Live {}  ·  {now}", env!("CARGO_PKG_VERSION")),
            w = width - 1
        );
        let _ = writeln!(o, " └{}┘", "─".repeat(width));
    } else {
        let _ = writeln!(
            o,
            "{title}\nALP-OS Live {}  ·  {now}\n",
            env!("CARGO_PKG_VERSION")
        );
    }
    let worst = checks
        .iter()
        .map(|c| c.status)
        .max_by_key(|s| *s as u8)
        .unwrap_or(Status::Ok);
    let (code, verdict) = match worst {
        Status::Ok => ("\x1b[1;32m", "This computer can run ALPArchive."),
        Status::Note => (
            "\x1b[1;32m",
            "This computer can run ALPArchive. Read the notes below.",
        ),
        Status::Fail => (
            "\x1b[1;31m",
            "This computer cannot run ALPArchive as it is. See below.",
        ),
    };
    let _ = writeln!(
        o,
        " Computer: {}\n",
        crate::fail::display_safe(&r.machine, 60)
    );
    let _ = writeln!(o, " {}\n", paint(code, verdict, color));
    for c in checks {
        let _ = writeln!(o, " {} {:<20} {}", tag(c.status, color), c.item, c.value);
        let code = check_code(c, r)
            .map(|k| format!(" Code: {k}"))
            .unwrap_or_default();
        if !c.advice.is_empty() || !code.is_empty() {
            let _ = writeln!(
                o,
                "        {:<20} {}",
                "",
                paint("\x1b[2m", &format!("→ {}{code}", c.advice), color)
            );
        }
    }
    let rule = if color {
        paint("\x1b[2m", &"─".repeat(width + 2), true)
    } else {
        "-".repeat(width + 2)
    };
    let _ = writeln!(o, "\n {rule}\n Disks");
    for d in &r.disks {
        let _ = writeln!(
            o,
            "   {:<9} {:<6} {:>8}  {:<28} {}",
            d.id.0,
            transport(d),
            inspect::gb(d.size_bytes),
            clip(&crate::fail::display_safe(&d.model, 28), 28),
            role(d)
        );
    }
    let _ = writeln!(o, " Network");
    if r.nics.is_empty() {
        let _ = writeln!(o, "   none found");
    }
    for n in &r.nics {
        let kind = if n.kind == NicKind::Wired {
            "wired"
        } else {
            "Wi-Fi"
        };
        let _ = writeln!(
            o,
            "   {:<16} {:<6} {}{}",
            n.name,
            kind,
            n.driver,
            if n.usb {
                " (USB)"
            } else if !n.working {
                " (not working: driver or firmware missing)"
            } else {
                ""
            }
        );
    }
    o
}

/// Error code (errors/catalog.toml) for a check that is not fine.
pub fn check_code(c: &Check, r: &HwReport) -> Option<&'static str> {
    match (c.item, c.status) {
        (_, Status::Ok) if c.item != "Graphics" => None,
        ("Memory", Status::Fail) => Some("ALP-HW-100"),
        ("Processor", Status::Fail) => Some("ALP-HW-101"),
        ("Startup (UEFI)", Status::Fail) => Some("ALP-HW-102"),
        ("Security chip (TPM)", _) => Some("ALP-HW-103"),
        ("Secure Boot", _) if r.secure_boot == Some(true) => Some("ALP-HW-104"),
        ("Storage", Status::Fail) => Some(if r.disks.iter().any(inspect::is_internal) {
            "ALP-HW-106"
        } else {
            "ALP-HW-105"
        }),
        ("Graphics", _) if r.gpus.is_empty() => Some("ALP-HW-107"),
        ("Wired network", _) => Some(if r.nics.iter().any(|n| n.kind == NicKind::Wired) {
            "ALP-NET-102"
        } else {
            "ALP-NET-100"
        }),
        _ => None,
    }
}

/// Code shown when the report could not be saved on the stick.
pub const REPORT_NOT_SAVED: &str = "ALP-HW-108";

fn transport(d: &DiskInfo) -> &'static str {
    match d.transport {
        DiskTransport::Nvme => "NVMe",
        DiskTransport::Sata => "SATA",
        DiskTransport::Usb => "USB",
        DiskTransport::Virtio => "virtio",
        DiskTransport::Other => "other",
    }
}

fn role(d: &DiskInfo) -> &'static str {
    if d.is_system_disk {
        "this stick (ALP-OS)"
    } else if inspect::is_internal(d) {
        "internal, read-only"
    } else {
        "external, read-only"
    }
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.into()
    } else {
        s.chars().take(n - 1).chain(['…']).collect()
    }
}

/// Write the report to the stick's report partition, found by partition
/// UUID on the system disk only (never by label alone: another disk could
/// carry the same label). Returns the file name.
fn save(r: &HwReport, text: &str, now: &str) -> Result<String, String> {
    let name = file_name(&r.machine, now);
    save_as(r, &name, text).map(|()| name)
}

/// Write `text` as file `name` on the stick's report partition.
fn save_as(r: &HwReport, name: &str, text: &str) -> Result<(), String> {
    on_report_drive(r, |dir| {
        let mut f = fs::File::create(dir.join(name)).map_err(|e| e.to_string())?;
        f.write_all(text.replace('\n', "\r\n").as_bytes())
            .map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())
    })
}

/// `on_report_drive` when the stick has no ALP-REPORT drive.
const NO_REPORT_DRIVE: &str = "report drive not found";

/// `dir` is a mount point (`/proc/self/mountinfo`, field 5).
fn is_mountpoint(dir: &str) -> bool {
    fs::read_to_string("/proc/self/mountinfo")
        .unwrap_or_default()
        .lines()
        .any(|l| l.split(' ').nth(4) == Some(dir))
}

/// Mount the stick's report partition, run `f` in it, unmount.
fn on_report_drive(
    r: &HwReport,
    f: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let part = r
        .disks
        .iter()
        .filter(|d| d.is_system_disk)
        .flat_map(|d| &d.partitions)
        .find(|p| p.label == REPORT_LABEL && !p.partition_uuid.is_empty())
        .ok_or(NO_REPORT_DRIVE)?;
    let dev = format!("/dev/disk/by-partuuid/{}", part.partition_uuid);
    // One user of the drive at a time (the boot log runs every minute,
    // next to reports and diagnoses).
    fs::create_dir_all("/run/alp").map_err(|e| e.to_string())?;
    let guard = fs::File::create("/run/alp/report.lock").map_err(|e| e.to_string())?;
    // Bounded: the shutdown line must not wait behind a stuck boot log.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        match guard.try_lock() {
            Ok(()) => break,
            Err(fs::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(200))
            }
            Err(fs::TryLockError::WouldBlock) => return Err("report drive busy".into()),
            Err(fs::TryLockError::Error(e)) => return Err(e.to_string()),
        }
    }
    // A user that was killed while the drive was mounted (unit timeout)
    // leaves the mount; with the lock held, any mount here is such a one.
    for _ in 0..4 {
        if !is_mountpoint(REPORT_MOUNT) {
            break;
        }
        eprintln!("ALP-OS Live: unmounting a stale {REPORT_MOUNT}");
        let _ = run("umount", &[REPORT_MOUNT]);
    }
    fs::create_dir_all(REPORT_MOUNT).map_err(|e| e.to_string())?;
    run(
        "mount",
        &[
            "-t",
            "vfat",
            "-o",
            "nosuid,nodev,noexec,flush",
            &dev,
            REPORT_MOUNT,
        ],
    )?;
    let result = f(Path::new(REPORT_MOUNT));
    let unmounted = run("umount", &[REPORT_MOUNT]);
    result.and(unmounted)
}

/// File on the ALP-REPORT drive with one line per shutdown (T15).
pub const SHUTDOWN_LOG: &str = "shutdown-log.txt";
/// Shutdowns kept in it.
const SHUTDOWN_LINES: usize = 200;

/// `old` (CRLF or LF) with `line` added, only the last `keep` lines, LF.
pub fn log_with(old: &str, line: &str, keep: usize) -> String {
    let mut v: Vec<&str> = old.lines().map(|l| l.trim_end_matches('\r')).collect();
    v.push(line.trim());
    v.retain(|l| !l.is_empty());
    let start = v.len().saturating_sub(keep);
    v[start..].join("\n") + "\n"
}

/// Add `line` to `shutdown-log.txt` on the stick's ALP-REPORT drive.
pub fn shutdown_log(line: &str) -> Result<(), String> {
    let r = inspect::inspect(&inspect::Roots::system());
    on_report_drive(&r, |dir| {
        let path = dir.join(SHUTDOWN_LOG);
        let old = fs::read_to_string(&path).unwrap_or_default();
        let text = log_with(&old, line, SHUTDOWN_LINES).replace('\n', "\r\n");
        let mut f = fs::File::create(&path).map_err(|e| e.to_string())?;
        f.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())
    })
}

/// Boot log on the ALP-REPORT drive (T29): this start and the four before
/// it (T44: a start that hung is often followed by several tries).
pub const BOOT_LOG: &str = "boot-log.txt";
pub const BOOT_LOG_PREV: &str = "boot-log-previous.txt";
/// Older starts: `boot-log-2.txt` (two starts ago) to `boot-log-4.txt`.
const BOOT_LOG_OLDER: [&str; 3] = ["boot-log-2.txt", "boot-log-3.txt", "boot-log-4.txt"];

/// The names in rotation order, newest first.
fn boot_log_names() -> Vec<&'static str> {
    let mut v = vec![BOOT_LOG, BOOT_LOG_PREV];
    v.extend(BOOT_LOG_OLDER);
    v
}

/// A new start: every boot log moves one place older; the oldest goes.
fn rotate_boot_logs(dir: &Path) -> Result<(), String> {
    let names = boot_log_names();
    for w in names.windows(2).rev() {
        let from = dir.join(w[0]);
        if from.exists() {
            fs::rename(&from, dir.join(w[1])).map_err(|e| format!("{}: {e}", w[0]))?;
        }
    }
    Ok(())
}
/// Marks the boot that `boot-log.txt` belongs to (`/proc/sys/kernel/random/boot_id`).
const BOOT_LOG_ID: &str = "boot-log.id";

/// Run `cmd` with at most `secs` (coreutils `timeout`), stdout and stderr.
fn out_bounded(secs: u32, cmd: &str, args: &[&str]) -> String {
    let secs = secs.to_string();
    match Command::new("timeout")
        .arg(&secs)
        .arg(cmd)
        .args(args)
        .output()
    {
        Ok(o) => {
            let mut t = String::from_utf8_lossy(&o.stdout).into_owned();
            t += &String::from_utf8_lossy(&o.stderr);
            if o.status.code() == Some(124) {
                t += &format!("({cmd}: no answer within {secs} s)\n");
            }
            t
        }
        Err(e) => format!("({cmd}: {e})\n"),
    }
}

/// ALPArchive's own units, in start order.
const BOOT_UNITS: [&str; 13] = [
    "alp-live-protect.service",
    "alp-live-state.service",
    "alp-live-report.service",
    "alpd.service",
    "alp-kiosk.service",
    "alp-live-screen.service",
    "alp-live-index.service",
    "alp-live-archive.service",
    "alp-immich.service",
    "user@50000.service",
    "nginx.service",
    "iwd.service",
    // The maintenance login (Ctrl+Alt+F2; T38: it failed on a laptop).
    "getty@tty2.service",
];

/// The boot log: what still waits (jobs), what failed, the state of
/// ALPArchive's units and their messages, warnings of the whole system.
/// No secrets: ALPArchive never logs passwords or keys, and nginx logs
/// paths without query strings.
pub fn boot_log_text(now: &str) -> String {
    let mut t = format!("ALPArchive boot log, {now} (updated every minute)\n\n== Version ==\n");
    t += &fs::read_to_string("/usr/lib/alp/build").unwrap_or_else(|_| "(unknown)\n".into());
    t += "\n== Up for ==\n";
    t += &fs::read_to_string("/proc/uptime").unwrap_or_default();
    t += "\n== Waiting jobs (a stuck start shows here) ==\n";
    t += &out_bounded(10, "systemctl", &["list-jobs", "--no-pager"]);
    t += "\n== Failed units ==\n";
    t += &out_bounded(10, "systemctl", &["--failed", "--no-pager"]);
    t += "\n== ALPArchive units ==\n";
    let mut args = vec!["status", "--no-pager", "-n", "0"];
    args.extend(BOOT_UNITS);
    t += &out_bounded(15, "systemctl", &args);
    t += "\n== Screen seat (kiosk on tty1, maintenance login on tty2) ==\n";
    t += &out_bounded(5, "loginctl", &["seat-status", "seat0", "--no-pager"]);
    t += &out_bounded(5, "loginctl", &["list-sessions", "--no-pager"]);
    t += "\n== Console (alpd) answers ==\n";
    t += &out_bounded(
        8,
        "curl",
        &[
            "-s",
            "-o",
            "/dev/null",
            "-m",
            "5",
            "-w",
            "%{http_code} in %{time_total} s\n",
            "http://127.0.0.1:8411/",
        ],
    );
    t += "\n== Messages of ALPArchive units ==\n";
    let mut args = vec!["-b", "--no-pager", "-o", "short-iso", "-n", "400"];
    for u in BOOT_UNITS {
        args.extend(["-u", u]);
    }
    t += &out_bounded(20, "journalctl", &args);
    t += "\n== Warnings and errors (whole system) ==\n";
    t += &out_bounded(
        20,
        "journalctl",
        &[
            "-b",
            "--no-pager",
            "-o",
            "short-iso",
            "-p",
            "warning",
            "-n",
            "300",
        ],
    );
    t + "\n"
}

/// `alpctl live bootlog`: write the boot log to the ALP-REPORT drive. The
/// first write of a new boot moves the earlier logs one place older
/// (`boot-log-previous.txt`, then `boot-log-2.txt` to `-4.txt`), so a start
/// that hung can be read after the next ones (or from Windows: the drive
/// opens there). The first write comes early (alp-live-bootlog-early, T44),
/// so even a start that hangs before the timer leaves its own log.
pub fn boot_log() -> i32 {
    let r = inspect::inspect(&inspect::Roots::system());
    let now = now_utc();
    let text = boot_log_text(&now).replace('\n', "\r\n");
    let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap_or_default();
    let res = on_report_drive(&r, |dir| {
        let id = dir.join(BOOT_LOG_ID);
        let log = dir.join(BOOT_LOG);
        if fs::read_to_string(&id).unwrap_or_default().trim() != boot.trim() {
            rotate_boot_logs(dir)?;
            fs::write(&id, boot.trim()).map_err(|e| e.to_string())?;
        }
        let tmp = dir.join("boot-log.tmp");
        let mut f = fs::File::create(&tmp).map_err(|e| e.to_string())?;
        f.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
        fs::rename(&tmp, &log).map_err(|e| e.to_string())
    });
    match res {
        Ok(()) => 0,
        // An installed system or another stick: nothing to do, not a failure.
        Err(e) if e == NO_REPORT_DRIVE => {
            eprintln!("ALP-OS Live: no report drive, boot log not saved");
            0
        }
        Err(e) => {
            eprintln!("ALP-OS Live: the boot log could not be saved on the stick: {e}");
            1
        }
    }
}

#[cfg(test)]
mod boot_log_tests {
    use super::*;

    #[test]
    fn five_boot_logs_rotate() {
        let d = std::env::temp_dir().join(format!("alp-bootlog-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        for start in 1..=7 {
            rotate_boot_logs(&d).unwrap();
            fs::write(d.join(BOOT_LOG), format!("start {start}")).unwrap();
        }
        let got: Vec<String> = boot_log_names()
            .iter()
            .map(|n| fs::read_to_string(d.join(n)).unwrap())
            .collect();
        assert_eq!(got, ["start 7", "start 6", "start 5", "start 4", "start 3"]);
        let _ = fs::remove_dir_all(&d);
    }
}

/// Kernel lines worth keeping for a disk problem (USB resets, I/O errors,
/// filesystems, loop and dm devices, tasks stuck waiting for a disk).
pub fn kernel_line_relevant(line: &str) -> bool {
    const WORDS: [&str; 16] = [
        "usb",
        "exfat",
        "ntfs",
        "ext4",
        "btrfs",
        "loop",
        "dm-",
        "device-mapper",
        "i/o error",
        "blk_update",
        "reset",
        "scsi",
        " sd ",
        "blocked for more than",
        "hung_task",
        "uas",
    ];
    let l = line.to_ascii_lowercase();
    WORDS.iter().any(|w| l.contains(w))
}

/// alpd lines worth keeping: jobs and the archive.
pub fn alpd_line_relevant(line: &str) -> bool {
    line.contains(crate::job::LOG_PREFIX) || line.to_ascii_lowercase().contains("archive")
}

fn last_lines(text: &str, keep: impl Fn(&str) -> bool, n: usize) -> String {
    let v: Vec<&str> = text.lines().filter(|l| keep(l)).collect();
    v[v.len().saturating_sub(n)..].join("\n")
}

fn out(cmd: &str, args: &[&str]) -> String {
    match Command::new(cmd).args(args).output() {
        Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
        Err(e) => format!("({cmd}: {e})"),
    }
}

/// The diagnosis text: last job status, disks, loop devices, dirty memory,
/// the job and archive lines of alpd and the disk lines of the kernel.
/// No secrets: alpd never logs any, and only these lines are taken.
pub fn diag_text(now: &str) -> String {
    let mut t = format!("ALPArchive diagnosis, {now}\n\n== Last archive job ==\n");
    t += &crate::job::read_last().unwrap_or_else(|| "(none recorded)\n".into());
    t += "\n== Disks ==\n";
    t += &out("lsblk", &["-o", "NAME,SIZE,FSTYPE,TRAN,RO,MOUNTPOINTS"]);
    t += "\n== Loop devices ==\n";
    t += &out("losetup", &["-l"]);
    t += "\n== Memory ==\n";
    t += &fs::read_to_string("/proc/meminfo")
        .unwrap_or_default()
        .lines()
        .filter(|l| {
            ["MemTotal", "MemAvailable", "Dirty", "Writeback:"]
                .iter()
                .any(|k| l.starts_with(k))
        })
        .collect::<Vec<_>>()
        .join("\n");
    t += "\n\n== alpd (jobs and archive) ==\n";
    let alpd = out(
        "journalctl",
        &[
            "-b",
            "--no-pager",
            "-o",
            "short-iso",
            "-u",
            "alpd.service",
            "-n",
            "5000",
        ],
    );
    t += &last_lines(&alpd, alpd_line_relevant, 300);
    t += "\n\n== Kernel (disks) ==\n";
    let kernel = out(
        "journalctl",
        &["-b", "-k", "--no-pager", "-o", "short-iso", "-n", "20000"],
    );
    t += &last_lines(&kernel, kernel_line_relevant, 300);
    t + "\n"
}

/// Save the diagnosis on the stick's ALP-REPORT drive; returns its name.
pub fn diag_save() -> Result<String, String> {
    let r = inspect::inspect(&inspect::Roots::system());
    let now = now_utc();
    let name = diag_file_name(&r.machine, &now);
    save_as(&r, &name, &diag_text(&now).replace('\n', "\r\n")).map(|()| name)
}

/// `diag-<machine>-<YYYYMMDD-HHMM>.txt`.
pub fn diag_file_name(machine: &str, now: &str) -> String {
    file_name(machine, now).replacen("hw-", "diag-", 1)
}

/// `alpctl live diag`: save the diagnosis on the ALP-REPORT drive (the
/// owner sends that file).
pub fn diag() -> i32 {
    match diag_save() {
        Ok(name) => {
            println!("Saved on the stick: drive {REPORT_LABEL}, file {name}");
            0
        }
        Err(e) => {
            eprintln!(
                "ALP-OS Live: the diagnosis could not be saved on the stick ({e}); here it is:"
            );
            println!("{}", diag_text(&now_utc()));
            1
        }
    }
}

fn run(cmd: &str, args: &[&str]) -> Result<(), String> {
    match Command::new(cmd).args(args).status() {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("{cmd} failed ({s})")),
        Err(e) => Err(format!("{cmd}: {e}")),
    }
}

/// `hw-<machine>-<YYYYMMDD-HHMM>.txt`, safe on FAT and Windows.
pub fn file_name(machine: &str, now: &str) -> String {
    let m: String = machine
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let stamp: String = now
        .chars()
        .filter(|c| c.is_ascii_digit())
        .take(12)
        .collect();
    let (date, time) = stamp.split_at(stamp.len().min(8));
    format!("hw-{}-{date}-{time}.txt", clip(&m, 40).replace('…', ""))
}

/// RFC 3339 UTC time, e.g. `2026-10-19T09:41:00Z`.
pub fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let d = now_utc(); // YYYY-MM-DD HH:MM UTC
    format!("{}T{}:{:02}Z", &d[..10], &d[11..16], secs % 60)
}

/// `YYYY-MM-DD HH:MM UTC` from the system clock (civil-from-days, Howard
/// Hinnant's public-domain algorithm).
fn now_utc() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (days, rem) = ((secs / 86400) as i64, secs % 86400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
        rem / 3600,
        rem % 3600 / 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_minimums_from_the_profile() {
        let p = include_str!("../../../profiles/generic-x86_64.toml");
        let m = minimums(p).unwrap();
        assert_eq!(
            (m.ram_bytes, m.single_disk_bytes),
            (4_000_000_000, 64_000_000_000)
        );
        assert!(minimums("name = 'x'").is_err());
    }

    #[test]
    fn file_names_are_windows_safe() {
        assert_eq!(
            file_name("LENOVO 20L5CTO1WW ThinkPad T480", "2026-10-18 09:41 UTC"),
            "hw-LENOVO-20L5CTO1WW-ThinkPad-T480-20261018-0941.txt"
        );
        assert_eq!(
            file_name("A/B:C*?", "2026-01-02 03:04 UTC"),
            "hw-A-B-C-20260102-0304.txt"
        );
    }

    #[test]
    fn ip_and_main_address() {
        let out = "3: wlan0    inet 192.168.1.23/24 brd 192.168.1.255 scope global dynamic wlan0\n";
        assert_eq!(parse_ip_addr(out).as_deref(), Some("192.168.1.23"));
        assert_eq!(parse_ip_addr(""), None);
        let p = |name: &str, wifi, a: Option<&str>| Port {
            name: name.into(),
            wifi,
            carrier: true,
            address: a.map(String::from),
        };
        let v = [
            p("wlan0", true, Some("10.0.0.5")),
            p("enp1s0", false, Some("10.0.0.4")),
        ];
        assert_eq!(main_address(&v).as_deref(), Some("10.0.0.4"));
        assert_eq!(main_address(&v[..1]).as_deref(), Some("10.0.0.5"));
        assert!(now_rfc3339().ends_with('Z'));
    }

    #[test]
    fn shutdown_log_keeps_the_last_lines() {
        assert_eq!(log_with("", "a 1", 3), "a 1\n");
        assert_eq!(log_with("x\r\ny\r\nz\r\n", "a 1", 3), "y\nz\na 1\n");
        assert_eq!(log_with("x\n\n", " b ", 5), "x\nb\n");
    }

    #[test]
    fn diag_keeps_only_relevant_lines() {
        assert!(kernel_line_relevant(
            "usb 2-1: reset high-speed USB device number 3"
        ));
        assert!(kernel_line_relevant(
            "INFO: task alpd:812 blocked for more than 120 seconds."
        ));
        assert!(kernel_line_relevant("I/O error, dev sdb, sector 123"));
        assert!(!kernel_line_relevant("wlan0: associated"));
        assert!(alpd_line_relevant(
            "alpd[1]: ALP-JOB archive.create: step=reserving"
        ));
        assert!(!alpd_line_relevant(
            "alpd[1]: the security chip opened the index"
        ));
        let text = "a usb 1\nb\nc usb 2\nd usb 3\n";
        assert_eq!(
            last_lines(text, kernel_line_relevant, 2),
            "c usb 2\nd usb 3"
        );
        assert_eq!(
            diag_file_name("ASUS VivoBook", "2026-10-21 09:41 UTC"),
            "diag-ASUS-VivoBook-20261021-0941.txt"
        );
    }

    #[test]
    fn clock_format() {
        let s = now_utc();
        assert_eq!(s.len(), "2026-10-18 09:41 UTC".len(), "{s}");
        assert!(s.starts_with("20"));
    }
}
