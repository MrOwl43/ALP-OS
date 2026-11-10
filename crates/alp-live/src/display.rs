// SPDX-License-Identifier: AGPL-3.0-or-later
//! The computer's own screen (T56): its physical size from the EDID, a
//! suggested UI scale, and its modes through `wlr-randr` against cage.
//!
//! The EDID comes from the monitor and is untrusted: only the size bytes
//! and the identity bytes are read, after a length, header and checksum
//! check; anything odd gives "unknown" (scale 1). No EDID text (monitor
//! name, serial) is ever shown or logged: the identity is a SHA-256 hash.
//!
//! `wlr-randr` runs as the kiosk user with privileges dropped (`setpriv`),
//! an environment of only `XDG_RUNTIME_DIR` and `WAYLAND_DISPLAY`, a 5 s
//! limit, and arguments built from its own mode list (review T56).
//!
//! VERIFY in a VM: the plain listing and `--mode WxH@R.RRRHz` with trixie's
//! wlr-randr 0.4.1 (it matches the refresh exactly, in mHz).

use std::fs;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::process::Command;

use bitcoin_hashes::{Hash, sha256};
use serde_json::{Value, json};

use crate::prefs::{Mode, SCALES};

/// The kiosk user (image/units/alp-kiosk.service).
pub const KIOSK_USER: &str = "alp-kiosk";
pub const RANDR: &str = "/usr/bin/wlr-randr";
/// An unconfirmed mode change (review N-T56-1): holds the mode to go back
/// to. In /run, so it survives an alpd restart but not a reboot.
pub const PENDING: &str = "/run/alp/display-pending";

/// Facts read from an EDID block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edid {
    pub width_mm: Option<u32>,
    pub height_mm: Option<u32>,
    /// SHA-256 (hex) of manufacturer, product code and serial number.
    pub identity: String,
}

/// Parse the base EDID block (128 bytes): header, checksum, then only the
/// physical size and the identity bytes.
pub fn parse_edid(b: &[u8]) -> Option<Edid> {
    let base = b.get(..128)?;
    if base[..8] != [0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0] {
        return None;
    }
    if base.iter().fold(0u8, |s, x| s.wrapping_add(*x)) != 0 {
        return None;
    }
    let identity = sha256::Hash::hash(&base[8..16]).to_string();
    // The first detailed timing gives the size in mm (bytes 66-68) when
    // its pixel clock is set; else bytes 21-22 give it in cm.
    let (w, h) = if base[54] != 0 || base[55] != 0 {
        (
            u32::from(base[66]) | (u32::from(base[68] >> 4) << 8),
            u32::from(base[67]) | (u32::from(base[68] & 0x0f) << 8),
        )
    } else {
        (0, 0)
    };
    let (w, h) = if w == 0 || h == 0 {
        (u32::from(base[21]) * 10, u32::from(base[22]) * 10)
    } else {
        (w, h)
    };
    // 0 means unknown (projectors); some EDIDs give an aspect ratio or junk.
    let sane = (50..=3000).contains(&w) && (30..=3000).contains(&h);
    Some(Edid {
        width_mm: sane.then_some(w),
        height_mm: sane.then_some(h),
        identity,
    })
}

/// The suggested UI scale (percent): about 96 effective dots per inch,
/// rounded to the nearest offered step.
pub fn auto_scale(width_mm: Option<u32>, width_px: Option<u32>) -> u16 {
    let (Some(mm), Some(px)) = (width_mm, width_px) else {
        return 100;
    };
    if mm == 0 || px == 0 {
        return 100;
    }
    let dpi = f64::from(px) * 25.4 / f64::from(mm);
    let want = dpi / 96.0 * 100.0;
    *SCALES
        .iter()
        .min_by(|a, b| {
            (f64::from(**a) - want)
                .abs()
                .total_cmp(&(f64::from(**b) - want).abs())
        })
        .unwrap_or(&100)
}

/// The connected screen, from sysfs (the first connected connector).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screen {
    pub edid: Option<Edid>,
    /// Width in pixels of its first (preferred) mode.
    pub width_px: Option<u32>,
}

fn screen_in(drm: &Path) -> Option<Screen> {
    let mut names: Vec<PathBuf> = fs::read_dir(drm)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("card") && n.contains('-'))
        })
        .collect();
    names.sort();
    let c = names
        .into_iter()
        .find(|p| fs::read_to_string(p.join("status")).is_ok_and(|s| s.trim() == "connected"))?;
    let edid = fs::read(c.join("edid"))
        .ok()
        .filter(|b| b.len() <= 32 * 1024)
        .and_then(|b| parse_edid(&b));
    let width_px = fs::read_to_string(c.join("modes")).ok().and_then(|m| {
        m.lines()
            .next()?
            .split('x')
            .next()?
            .trim()
            .parse::<u32>()
            .ok()
    });
    Some(Screen { edid, width_px })
}

pub fn screen() -> Option<Screen> {
    screen_in(Path::new("/sys/class/drm"))
}

/// `state.display`: physical size and the suggested scale.
pub fn state_json() -> Value {
    let s = screen();
    let e = s.as_ref().and_then(|s| s.edid.clone());
    let (w, h) = e.map_or((None, None), |e| (e.width_mm, e.height_mm));
    let px = s.and_then(|s| s.width_px);
    json!({
        "width_mm": w,
        "height_mm": h,
        "auto_scale": f64::from(auto_scale(w, px)) / 100.0,
    })
}

/// One mode as `wlr-randr --json` lists it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Listed {
    pub mode: Mode,
    pub preferred: bool,
    pub current: bool,
}

/// One output (screen) as `wlr-randr --json` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub name: String,
    pub modes: Vec<Listed>,
}

/// An output name from the compositor goes back into a command line only
/// when it is a plain connector name.
fn plain_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// One mode line of `wlr-randr`'s plain listing, e.g.
/// `    1920x1080 px, 60.008000 Hz (preferred, current)`.
fn mode_line(l: &str) -> Option<Listed> {
    let (size, rest) = l.trim().split_once(" px, ")?;
    let (w, h) = size.split_once('x')?;
    let (hz, flags) = rest.split_once(" Hz").unwrap_or((rest, ""));
    let hz: f64 = hz.trim().parse().ok()?;
    if !(1.0..=1000.0).contains(&hz) {
        return None;
    }
    let width: u32 = w
        .trim()
        .parse()
        .ok()
        .filter(|w| (320..=16384).contains(w))?;
    let height: u32 = h
        .trim()
        .parse()
        .ok()
        .filter(|h| (200..=16384).contains(h))?;
    Some(Listed {
        mode: Mode {
            width,
            height,
            // Range checked just above.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            refresh_mhz: (hz * 1000.0).round() as u32,
        },
        preferred: flags.contains("preferred"),
        current: flags.contains("current"),
    })
}

/// Parse `wlr-randr`'s plain listing (the form every version prints;
/// `--json` is not in all of them): the enabled outputs and their modes.
/// An output starts at a line without indent, whose first word is its
/// name (the description after it, monitor text, is never read). Odd
/// lines are left out; no output at all is an error.
pub fn parse_outputs(text: &str) -> Result<Vec<Output>, String> {
    let mut outs: Vec<(Output, bool)> = Vec::new();
    let mut in_modes = false;
    for l in text.lines() {
        if l.trim().is_empty() {
            continue;
        }
        if !l.starts_with(' ') && !l.starts_with('\t') {
            let name = l.split_whitespace().next().unwrap_or_default();
            outs.push((
                Output {
                    name: name.to_string(),
                    modes: Vec::new(),
                },
                plain_name(name),
            ));
            in_modes = false;
            continue;
        }
        let Some((o, ok)) = outs.last_mut() else {
            continue;
        };
        let t = l.trim();
        if t == "Modes:" {
            in_modes = true;
        } else if let Some(v) = t.strip_prefix("Enabled:") {
            if v.trim() == "no" {
                *ok = false;
            }
            in_modes = false;
        } else if in_modes && let Some(m) = mode_line(t) {
            o.modes.push(m);
        } else if t.ends_with(':') || t.contains(": ") {
            in_modes = false;
        }
    }
    if outs.is_empty() {
        return Err("the screen list is not readable".into());
    }
    Ok(outs
        .into_iter()
        .filter(|(_, ok)| *ok)
        .map(|(o, _)| o)
        .collect())
}

/// `GET display/modes`: the first output's modes; `id` is the position.
pub fn modes_json(o: &Output) -> Value {
    json!({"modes": o.modes.iter().enumerate().map(|(i, m)| json!({
        "id": i, "width": m.mode.width, "height": m.mode.height,
        "refresh": f64::from(m.mode.refresh_mhz) / 1000.0,
        "native": m.preferred, "current": m.current,
    })).collect::<Vec<_>>()})
}

/// The arguments that set `m` on output `name` (`wlr-randr --output NAME
/// --mode WxH@RHz`; both from wlr-randr's own list).
pub fn set_args(name: &str, m: Mode) -> Vec<String> {
    vec![
        "--output".into(),
        name.into(),
        "--mode".into(),
        format!(
            "{}x{}@{}.{:03}Hz",
            m.width,
            m.height,
            m.refresh_mhz / 1000,
            m.refresh_mhz % 1000
        ),
    ]
}

/// The kiosk user's uid and gid (from /etc/passwd; sysusers made it).
fn kiosk_ids() -> Option<(u32, u32)> {
    let p = fs::read_to_string("/etc/passwd").ok()?;
    p.lines().find_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        (f.first() == Some(&KIOSK_USER) && f.len() > 3)
            .then(|| Some((f[2].parse().ok()?, f[3].parse().ok()?)))
            .flatten()
    })
}

/// cage's socket in the kiosk user's runtime folder: (folder, socket
/// name, a stamp that changes when cage restarts).
pub fn socket() -> Option<(String, String, (u64, i64))> {
    let (uid, _) = kiosk_ids()?;
    let dir = format!("/run/user/{uid}");
    let mut found: Vec<(String, (u64, i64))> = fs::read_dir(&dir)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let md = fs::symlink_metadata(e.path()).ok()?;
            (name.starts_with("wayland-") && !name.contains('.') && md.file_type().is_socket())
                .then(|| (name, (md.ino(), md.mtime())))
        })
        .collect();
    found.sort();
    let (name, stamp) = found.into_iter().next()?;
    Some((dir, name, stamp))
}

/// Run `wlr-randr` with `args` as the kiosk user (see the module notes).
pub fn randr(args: &[String]) -> Result<String, String> {
    let (uid, gid) = kiosk_ids().ok_or("the screen user is missing")?;
    let (dir, name, _) = socket().ok_or("the screen is not running")?;
    let out = Command::new("/usr/bin/timeout")
        .args(["-k", "1", "5", "/usr/bin/setpriv"])
        .arg(format!("--reuid={uid}"))
        .arg(format!("--regid={gid}"))
        .args(["--clear-groups", "--no-new-privs", "--", RANDR])
        .args(args)
        .env_clear()
        .env("XDG_RUNTIME_DIR", dir)
        .env("WAYLAND_DISPLAY", name)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else if out.status.code() == Some(124) {
        Err("the screen did not answer within 5 s".into())
    } else {
        // wlr-randr's own words, made safe and short; never EDID text.
        Err(crate::fail::display_safe(
            String::from_utf8_lossy(&out.stderr).trim(),
            200,
        ))
    }
}

/// Can this system change screen modes (the tool is in the image)?
pub fn modes_supported() -> bool {
    Path::new(RANDR).exists()
}

/// The pending marker's text: output and the mode to go back to.
pub fn pending_line(name: &str, back: Mode) -> String {
    format!(
        "{name} {} {} {}\n",
        back.width, back.height, back.refresh_mhz
    )
}

pub fn parse_pending(text: &str) -> Option<(String, Mode)> {
    let f: Vec<&str> = text.split_whitespace().collect();
    let [name, w, h, r] = f.as_slice() else {
        return None;
    };
    if !plain_name(name) {
        return None;
    }
    Some((
        (*name).to_string(),
        Mode {
            width: w.parse().ok()?,
            height: h.parse().ok()?,
            refresh_mhz: r.parse().ok()?,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edid(dtd_mm: (u16, u16), cm: (u8, u8)) -> Vec<u8> {
        let mut b = vec![0u8; 128];
        b[..8].copy_from_slice(&[0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0]);
        b[8..16].copy_from_slice(&[0x10, 0xac, 1, 2, 3, 4, 5, 6]);
        b[21] = cm.0;
        b[22] = cm.1;
        if dtd_mm.0 > 0 {
            b[54] = 1;
            b[66] = (dtd_mm.0 & 0xff) as u8;
            b[67] = (dtd_mm.1 & 0xff) as u8;
            b[68] = (((dtd_mm.0 >> 8) as u8) << 4) | ((dtd_mm.1 >> 8) as u8);
        }
        let s = b.iter().fold(0u8, |s, x| s.wrapping_add(*x));
        b[127] = 0u8.wrapping_sub(s);
        b
    }

    #[test]
    fn edid_size_is_read_only_when_it_checks_out() {
        let e = parse_edid(&edid((344, 194), (34, 19))).unwrap();
        assert_eq!((e.width_mm, e.height_mm), (Some(344), Some(194)));
        assert_eq!(e.identity.len(), 64);
        // No detailed timing: the size in cm.
        let e = parse_edid(&edid((0, 0), (53, 30))).unwrap();
        assert_eq!((e.width_mm, e.height_mm), (Some(530), Some(300)));
        // A projector (no size): unknown.
        assert_eq!(parse_edid(&edid((0, 0), (0, 0))).unwrap().width_mm, None);
        // A wrong checksum, a short block or a bad header: nothing.
        let mut bad = edid((344, 194), (34, 19));
        bad[100] ^= 1;
        assert_eq!(parse_edid(&bad), None);
        assert_eq!(parse_edid(&bad[..100]), None);
        assert_eq!(parse_edid(&[0u8; 128]), None);
    }

    #[test]
    fn the_scale_aims_at_96_dpi() {
        assert_eq!(auto_scale(Some(344), Some(1920)), 150); // 15.6" laptop
        assert_eq!(auto_scale(Some(531), Some(1920)), 100); // 24" monitor
        assert_eq!(auto_scale(Some(597), Some(3840)), 175); // 27" 4K
        assert_eq!(auto_scale(Some(290), Some(3840)), 200); // capped
        assert_eq!(auto_scale(None, Some(1920)), 100);
    }

    #[test]
    fn screen_from_sysfs() {
        let d = std::env::temp_dir().join(format!("alp-drm-{}", std::process::id()));
        let c0 = d.join("card0-HDMI-A-1");
        let c1 = d.join("card0-eDP-1");
        fs::create_dir_all(&c0).unwrap();
        fs::create_dir_all(&c1).unwrap();
        fs::write(c0.join("status"), "disconnected\n").unwrap();
        fs::write(c1.join("status"), "connected\n").unwrap();
        fs::write(c1.join("edid"), edid((344, 194), (34, 19))).unwrap();
        fs::write(c1.join("modes"), "1920x1080\n1280x720\n").unwrap();
        let s = screen_in(&d).unwrap();
        assert_eq!(s.width_px, Some(1920));
        assert_eq!(s.edid.unwrap().width_mm, Some(344));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn randr_list_and_arguments() {
        let t = "eDP-1 \"BOE 0x0747 (eDP-1)\"\n  Make: BOE\n  Physical size: 344x194 mm\n  Enabled: yes\n  Modes:\n    1920x1080 px, 60.008000 Hz (preferred)\n    1280x720 px, 59.940000 Hz (current)\n    1x720 px, 60.000000 Hz\n  Position: 0,0\n  Scale: 1.000000\nbad;name \"x\"\n  Enabled: yes\n  Modes:\n    800x600 px, 60.000000 Hz\nHDMI-A-1 \"Dell\"\n  Enabled: no\n  Modes:\n    800x600 px, 60.000000 Hz\n";
        let o = parse_outputs(t).unwrap();
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].modes.len(), 2);
        assert_eq!(o[0].modes[0].mode.refresh_mhz, 60008);
        let j = modes_json(&o[0]);
        assert_eq!(j["modes"][1]["current"], true);
        assert_eq!(j["modes"][0]["native"], true);
        assert_eq!(
            set_args("eDP-1", o[0].modes[1].mode),
            ["--output", "eDP-1", "--mode", "1280x720@59.940Hz"]
        );
        assert!(parse_outputs("").is_err());
        let p = pending_line("eDP-1", o[0].modes[0].mode);
        assert_eq!(
            parse_pending(&p),
            Some(("eDP-1".into(), o[0].modes[0].mode))
        );
        assert_eq!(parse_pending("x;y 1 2 3"), None);
    }
}
