// SPDX-License-Identifier: AGPL-3.0-or-later
//! Erasing a partition: the "real wipe" of an unprotected index at reset
//! (owner, 2026-11-03) and the quick or secure format of T49.
//!
//! - **Quick:** `blkdiscard -f` where the device supports it, then
//!   `wipefs --all`. Fast, but the old data can often still be recovered
//!   with special tools.
//! - **Secure:** `blkdiscard -f` where supported, then every byte is
//!   overwritten with zeros (discard alone does not promise zeros on USB
//!   sticks). If a full overwrite fails part-way, at least the first and
//!   last 16 MiB are overwritten (where file-system headers and journals
//!   live) and the result says so. The method used is always logged. On
//!   USB flash and SSDs, wear-levelling can keep old copies in spare
//!   blocks: the overwrite makes the data very hard to recover, not
//!   impossible. Screens must not claim more.
//!
//! No own cryptography and nothing clever: zeros, written in 4 MiB blocks,
//! synced every 256 MiB so progress means "on the device".

use std::fs;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

const BLOCK: usize = 4 << 20;
const SYNC_EVERY: u64 = 256 << 20;
/// Overwritten at both ends when a full overwrite is not possible.
pub const EDGE: u64 = 16 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Quick,
    Secure,
}

/// What was actually done, for the log and the result message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// Signatures removed (and discarded, if `discarded`).
    Quick { discarded: bool },
    /// Every byte overwritten with zeros.
    Full { discarded: bool },
    /// Only the first and last [`EDGE`] bytes overwritten.
    Edges { discarded: bool },
}

/// Stopped on request: the device is partly overwritten.
pub const CANCELLED: &str = "cancelled";

impl Method {
    /// True only when the whole device was handled as asked: a quick wipe,
    /// or a full overwrite. A partial (edges-only) overwrite is not
    /// complete and must never be reported as done (review W2).
    pub fn complete(self) -> bool {
        !matches!(self, Method::Edges { .. })
    }

    pub fn describe(self) -> &'static str {
        match self {
            Method::Quick { discarded: true } => "quick: discarded, signatures removed",
            Method::Quick { discarded: false } => "quick: signatures removed",
            Method::Full { discarded: true } => "secure: discarded, then overwritten with zeros",
            Method::Full { discarded: false } => "secure: overwritten with zeros",
            Method::Edges { .. } => {
                "partial: a full overwrite failed; the first and last 16 MiB were overwritten"
            }
        }
    }
}

/// Progress: bytes done, bytes in all, bytes per second so far.
pub type Progress<'a> = &'a mut dyn FnMut(u64, u64, u64);

/// Size of a block device or file in bytes.
pub fn size_of(dev: &Path) -> Result<u64, String> {
    let mut f = fs::File::open(dev).map_err(|e| format!("{}: {e}", dev.display()))?;
    f.seek(SeekFrom::End(0)).map_err(|e| e.to_string())
}

fn discard(dev: &Path) -> bool {
    Command::new("blkdiscard")
        .arg("-f")
        .arg(dev)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Erase `dev`. Quick never fails on a missing discard; secure fails only
/// when not even the edges could be overwritten.
/// `cancel` stops a secure wipe at its next block: the error is then
/// [`CANCELLED`] and the device is partly overwritten.
pub fn wipe(
    dev: &Path,
    mode: Mode,
    cancel: &AtomicBool,
    progress: Progress,
) -> Result<Method, String> {
    let discarded = discard(dev);
    let m = match mode {
        Mode::Quick => {
            let o = Command::new("wipefs")
                .args(["--all", "--quiet"])
                .arg(dev)
                .output()
                .map_err(|e| format!("wipefs: {e}"))?;
            if !o.status.success() {
                return Err(format!(
                    "wipefs failed: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                ));
            }
            Method::Quick { discarded }
        }
        Mode::Secure => match zero_fill(dev, cancel, progress) {
            Ok(()) => Method::Full { discarded },
            Err(e) if e == CANCELLED => return Err(e),
            Err(full) => {
                eprintln!(
                    "ALP-OS Live: wipe: full overwrite of {} failed: {full}",
                    dev.display()
                );
                zero_edges(dev).map_err(|e| {
                    format!("the stick could not be overwritten ({full}; edges: {e})")
                })?;
                Method::Edges { discarded }
            }
        },
    };
    eprintln!("ALP-OS Live: wipe: {}: {}", dev.display(), m.describe());
    Ok(m)
}

/// Overwrite every byte with zeros, with progress.
pub fn zero_fill(dev: &Path, cancel: &AtomicBool, progress: Progress) -> Result<(), String> {
    let total = size_of(dev)?;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .open(dev)
        .map_err(|e| e.to_string())?;
    let zeros = vec![0u8; BLOCK];
    let started = Instant::now();
    let (mut done, mut since_sync) = (0u64, 0u64);
    while done < total {
        if cancel.load(Ordering::Relaxed) {
            let _ = f.sync_data();
            return Err(CANCELLED.into());
        }
        let n = BLOCK.min(usize::try_from(total - done).unwrap_or(BLOCK));
        f.write_all(&zeros[..n])
            .map_err(|e| format!("at byte {done}: {e}"))?;
        done += n as u64;
        since_sync += n as u64;
        if since_sync >= SYNC_EVERY || done == total {
            f.sync_data()
                .map_err(|e| format!("sync at byte {done}: {e}"))?;
            since_sync = 0;
            let secs = started.elapsed().as_secs().max(1);
            progress(done, total, done / secs);
        }
    }
    Ok(())
}

/// Overwrite the first and last [`EDGE`] bytes (all of a smaller device).
pub fn zero_edges(dev: &Path) -> Result<(), String> {
    let total = size_of(dev)?;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .open(dev)
        .map_err(|e| e.to_string())?;
    let zeros = vec![0u8; BLOCK];
    for (start, len) in edges(total) {
        f.seek(SeekFrom::Start(start)).map_err(|e| e.to_string())?;
        let mut left = len;
        while left > 0 {
            let n = BLOCK.min(usize::try_from(left).unwrap_or(BLOCK));
            f.write_all(&zeros[..n]).map_err(|e| e.to_string())?;
            left -= n as u64;
        }
    }
    f.sync_data().map_err(|e| e.to_string())
}

/// The two ranges [`zero_edges`] writes: (start, length), not overlapping.
pub fn edges(total: u64) -> Vec<(u64, u64)> {
    if total <= 2 * EDGE {
        return vec![(0, total)];
    }
    vec![(0, EDGE), (total - EDGE, EDGE)]
}

/// Seconds a secure wipe of `size` bytes takes at `bytes_per_second`
/// (rounded up to a whole minute; at least one minute).
pub fn estimate_secs(size: u64, bytes_per_second: u64) -> u64 {
    let secs = size.div_ceil(bytes_per_second.max(1));
    secs.div_ceil(60).max(1) * 60
}

/// Write speed assumed before a secure wipe starts, from a 64 MiB
/// sequential read of the device (nothing is written): USB sticks write at
/// about half their read speed, so half of it, and never more than
/// 200 MB/s. `None` if the read failed.
pub fn guess_write_speed(dev: &Path) -> Option<u64> {
    let t = Instant::now();
    let o = Command::new("dd")
        .arg(format!("if={}", dev.display()))
        .args([
            "of=/dev/null",
            "bs=4M",
            "count=16",
            "iflag=direct",
            "status=none",
        ])
        .output()
        .ok()?;
    if !o.status.success() {
        return None;
    }
    let secs = t.elapsed().as_secs_f64().max(0.01);
    let read = (64u64 << 20) as f64 / secs;
    Some(((read / 2.0) as u64).clamp(1 << 20, 200_000_000))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn temp(name: &str, len: u64) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("alp-wipe-{name}-{}", std::process::id()));
        let mut f = fs::File::create(&p).unwrap();
        let pattern = b"ALP-SECRET-MARKER-".repeat(1000);
        let mut left = len;
        while left > 0 {
            let n = (pattern.len() as u64).min(left) as usize;
            f.write_all(&pattern[..n]).unwrap();
            left -= n as u64;
        }
        p
    }

    fn all_zero(p: &Path) -> bool {
        let mut b = Vec::new();
        fs::File::open(p).unwrap().read_to_end(&mut b).unwrap();
        b.iter().all(|&x| x == 0)
    }

    #[test]
    fn zero_fill_overwrites_everything_with_progress() {
        let p = temp("full", 9 * (1 << 20) + 123);
        let mut last = (0, 0);
        zero_fill(&p, &AtomicBool::new(false), &mut |d, t, _| last = (d, t)).unwrap();
        assert!(all_zero(&p));
        assert_eq!(last.0, last.1, "progress reaches the end");
        assert_eq!(size_of(&p).unwrap(), 9 * (1 << 20) + 123, "size unchanged");
        let _ = fs::remove_file(p);
    }

    #[test]
    fn cancel_stops_a_secure_wipe() {
        let p = temp("cancel", 3 * (1 << 20));
        let stop = AtomicBool::new(true);
        assert_eq!(
            zero_fill(&p, &stop, &mut |_, _, _| {}),
            Err(CANCELLED.into())
        );
        assert!(!all_zero(&p), "nothing written after an early cancel");
        let _ = fs::remove_file(p);
    }

    #[test]
    fn edges_cover_both_ends_without_overlap() {
        assert_eq!(edges(10), vec![(0, 10)]);
        assert_eq!(edges(2 * EDGE), vec![(0, 2 * EDGE)]);
        let t = 100 * EDGE;
        assert_eq!(edges(t), vec![(0, EDGE), (t - EDGE, EDGE)]);
        let p = temp("edges", 3 * EDGE);
        zero_edges(&p).unwrap();
        let mut b = Vec::new();
        fs::File::open(&p).unwrap().read_to_end(&mut b).unwrap();
        let e = EDGE as usize;
        assert!(b[..e].iter().all(|&x| x == 0));
        assert!(b[2 * e..].iter().all(|&x| x == 0));
        assert!(b[e..2 * e].iter().any(|&x| x != 0), "middle untouched");
        let _ = fs::remove_file(p);
    }

    #[test]
    fn estimates_are_whole_minutes() {
        assert_eq!(estimate_secs(0, 10), 60);
        assert_eq!(estimate_secs(30_000_000_000, 25_000_000), 1200);
        assert_eq!(estimate_secs(1, 0), 60);
    }

    #[test]
    fn methods_say_what_was_done() {
        assert!(
            Method::Edges { discarded: true }
                .describe()
                .contains("16 MiB")
        );
        assert!(
            Method::Full { discarded: false }
                .describe()
                .contains("zeros")
        );
        assert!(!Method::Edges { discarded: false }.complete());
        assert!(Method::Full { discarded: true }.complete());
        assert!(Method::Quick { discarded: false }.complete());
    }
}
