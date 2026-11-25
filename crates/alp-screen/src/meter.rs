// SPDX-License-Identifier: AGPL-3.0-or-later
//! Live network and disk activity for the lock screen (owner, 2026-11-25:
//! shows whether a backup is running and which disk is being written, with
//! graphs like a task manager). Read straight from the kernel's counters
//! (`/proc/net/dev`, `/proc/diskstats`) once a second; no secrets, no alpd
//! call. The parsers here are pure and unit-tested.

/// The kernel counters at one moment.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sample {
    /// Bytes received and sent on the network (physical ports only).
    pub rx: u64,
    pub tx: u64,
    pub disks: Vec<DiskCount>,
}

/// One whole disk's counters (`/proc/diskstats`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DiskCount {
    pub name: String,
    pub read: u64,
    pub written: u64,
    /// Milliseconds spent doing I/O.
    pub busy_ms: u64,
}

/// Bytes received and sent, summed over the ports `keep` accepts (the
/// caller passes "has a hardware device", so the loopback and the photo
/// service's internal links are not counted).
pub fn net(proc_net_dev: &str, keep: impl Fn(&str) -> bool) -> (u64, u64) {
    let (mut rx, mut tx) = (0u64, 0u64);
    for line in proc_net_dev.lines().skip(2) {
        let Some((name, rest)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if name == "lo" || !keep(name) {
            continue;
        }
        let f: Vec<u64> = rest
            .split_whitespace()
            .filter_map(|x| x.parse().ok())
            .collect();
        if f.len() >= 9 {
            rx = rx.saturating_add(f[0]);
            tx = tx.saturating_add(f[8]);
        }
    }
    (rx, tx)
}

/// A whole disk (not a partition, loop, RAM or mapped device).
pub fn whole_disk(name: &str) -> bool {
    let digits_after = |p: &str| {
        name.strip_prefix(p)
            .is_some_and(|r| !r.is_empty() && r.chars().all(|c| c.is_ascii_digit()))
    };
    let letters_after = |p: &str| {
        name.strip_prefix(p)
            .is_some_and(|r| !r.is_empty() && r.chars().all(|c| c.is_ascii_lowercase()))
    };
    if let Some(r) = name.strip_prefix("nvme") {
        // nvme0n1, not nvme0n1p2.
        return match r.split_once('n') {
            Some((a, b)) => {
                !a.is_empty()
                    && a.chars().all(|c| c.is_ascii_digit())
                    && !b.is_empty()
                    && b.chars().all(|c| c.is_ascii_digit())
            }
            None => false,
        };
    }
    letters_after("sd") || letters_after("vd") || letters_after("hd") || digits_after("mmcblk")
}

/// The whole disks' counters, in the kernel's order.
pub fn disks(diskstats: &str) -> Vec<DiskCount> {
    diskstats
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.len() < 13 || !whole_disk(f[2]) {
                return None;
            }
            let n = |i: usize| f[i].parse::<u64>().unwrap_or(0);
            Some(DiskCount {
                name: f[2].to_string(),
                // Sectors are always 512 bytes in this file.
                read: n(5).saturating_mul(512),
                written: n(9).saturating_mul(512),
                busy_ms: n(12),
            })
        })
        .collect()
}

/// Speeds between two samples `secs` apart.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rate {
    pub rx_bps: u64,
    pub tx_bps: u64,
    pub disks: Vec<DiskRate>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DiskRate {
    pub name: String,
    pub read_bps: u64,
    pub write_bps: u64,
    /// Share of the time the disk was busy, 0-100 (a task manager's
    /// "active time").
    pub busy: u8,
}

pub fn rate(a: &Sample, b: &Sample, secs: f64) -> Rate {
    let secs = secs.max(0.001);
    let per = |x: u64, y: u64| (y.saturating_sub(x) as f64 / secs) as u64;
    Rate {
        rx_bps: per(a.rx, b.rx),
        tx_bps: per(a.tx, b.tx),
        disks: b
            .disks
            .iter()
            .map(|d| {
                let old = a.disks.iter().find(|o| o.name == d.name);
                let (r, w, ms) = old.map_or((d.read, d.written, d.busy_ms), |o| {
                    (o.read, o.written, o.busy_ms)
                });
                DiskRate {
                    name: d.name.clone(),
                    read_bps: per(r, d.read),
                    write_bps: per(w, d.written),
                    busy: (d.busy_ms.saturating_sub(ms) as f64 / (secs * 10.0)).clamp(0.0, 100.0)
                        as u8,
                }
            })
            .collect(),
    }
}

/// A speed for people: "0 B/s", "850 KB/s", "12.4 MB/s", "1.1 GB/s".
pub fn speed(bps: u64) -> String {
    let b = bps as f64;
    if b >= 1e9 {
        format!("{:.1} GB/s", b / 1e9)
    } else if b >= 1e7 {
        format!("{:.0} MB/s", b / 1e6)
    } else if b >= 1e6 {
        format!("{:.1} MB/s", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.0} KB/s", b / 1e3)
    } else {
        format!("{bps} B/s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NET: &str = "Inter-|   Receive    |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
    lo: 1000 10 0 0 0 0 0 0 1000 10 0 0 0 0 0 0
  eth0: 5000 50 0 0 0 0 0 0 700 7 0 0 0 0 0 0
podman0: 9999 1 0 0 0 0 0 0 9999 1 0 0 0 0 0 0
 wlan0: 100 1 0 0 0 0 0 0 30 1 0 0 0 0 0 0
";

    #[test]
    fn network_counts_physical_ports_only() {
        assert_eq!(
            net(NET, |n| n.starts_with("eth") || n.starts_with("wlan")),
            (5100, 730)
        );
        assert_eq!(net(NET, |_| true), (15099, 10729), "lo is never counted");
        assert_eq!(net("", |_| true), (0, 0));
    }

    #[test]
    fn whole_disks_only() {
        for d in ["sda", "sdab", "vdb", "nvme0n1", "nvme1n12", "mmcblk0"] {
            assert!(whole_disk(d), "{d}");
        }
        for d in [
            "sda1",
            "nvme0n1p2",
            "mmcblk0p1",
            "loop0",
            "dm-0",
            "zram0",
            "sr0",
            "nvme0",
        ] {
            assert!(!whole_disk(d), "{d}");
        }
    }

    #[test]
    fn disk_speeds_and_busy_share() {
        let a = disks(
            "   8  0 sda 10 0 2000 5 20 0 4000 9 0 100 0 0 0 0\n   8  1 sda1 1 0 1 1 1 0 1 1 0 1 0 0 0 0\n",
        );
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].read, 2000 * 512);
        let b = disks("   8  0 sda 10 0 2000 5 20 0 6000 9 0 600 0 0 0 0\n");
        let r = rate(
            &Sample {
                disks: a,
                ..Sample::default()
            },
            &Sample {
                disks: b,
                ..Sample::default()
            },
            1.0,
        );
        assert_eq!(r.disks[0].write_bps, 2000 * 512);
        assert_eq!(r.disks[0].read_bps, 0);
        assert_eq!(r.disks[0].busy, 50);
        // Counters that went back (a disk replugged) never give a negative speed.
        let back = rate(
            &Sample {
                rx: 10,
                ..Sample::default()
            },
            &Sample::default(),
            1.0,
        );
        assert_eq!(back.rx_bps, 0);
    }

    #[test]
    fn speeds_read_well() {
        assert_eq!(speed(0), "0 B/s");
        assert_eq!(speed(850_000), "850 KB/s");
        assert_eq!(speed(1_240_000), "1.2 MB/s");
        assert_eq!(speed(35_400_000), "35 MB/s");
        assert_eq!(speed(1_100_000_000), "1.1 GB/s");
    }
}
