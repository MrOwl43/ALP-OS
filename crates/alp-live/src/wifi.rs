// SPDX-License-Identifier: AGPL-3.0-or-later
//! Wi-Fi setup on the ALP-OS Live text screen, through iwd (`iwctl`).
//! The passphrase is written to iwd's state directory (`/var/lib/iwd`,
//! bind-mounted from the stick's state partition so Live remembers it;
//! owner decision 2026-10-18) and never appears on a command line or in a
//! log.

use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

const IWD_DIR: &str = "/var/lib/iwd";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Network {
    pub ssid: String,
    /// iwd security type: `open`, `psk`, `8021x` or `wep`.
    pub security: String,
    /// Signal bars 0–4 (from iwd's dBm value; 0 when unknown).
    pub signal: u8,
    /// This computer is connected to it now.
    pub connected: bool,
}

/// Signal bars from iwd's `rssi-dbms` value (hundredths of a dBm).
pub fn bars(raw: i32) -> u8 {
    let dbm = if raw.abs() > 200 { raw / 100 } else { raw };
    match dbm {
        d if d >= -60 => 4,
        d if d >= -67 => 3,
        d if d >= -75 => 2,
        d if d >= -85 => 1,
        _ => 0,
    }
}

/// Remove ANSI escape sequences (iwctl colours its tables).
fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for c2 in chars.by_ref() {
                if c2.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Parse `iwctl station <dev> get-networks`: after the header rows, each
/// row is `[>] <ssid> <security> <signal stars>`; the SSID may contain
/// spaces, the security word is the last word before the signal column.
pub fn parse_networks(output: &str) -> Vec<Network> {
    let mut nets = Vec::new();
    let mut in_table = false;
    for line in strip_ansi(output).lines() {
        let t = line.trim();
        if t.starts_with("---") {
            in_table = true;
            continue;
        }
        if !in_table || t.is_empty() || t.starts_with("Network name") {
            continue;
        }
        let connected = t.starts_with('>');
        let t = t.strip_prefix('>').unwrap_or(t).trim();
        let words: Vec<&str> = t.split_whitespace().collect();
        let Some(pos) = words
            .iter()
            .rposition(|w| matches!(*w, "open" | "psk" | "8021x" | "wep"))
        else {
            continue;
        };
        if pos == 0 {
            continue;
        }
        // Rebuild the SSID from the original text to keep inner spacing.
        let sec_at = t.rfind(&format!(" {}", words[pos])).unwrap_or(t.len());
        let ssid = t[..sec_at].trim_end().to_string();
        let signal = words
            .get(pos + 1)
            .and_then(|w| w.parse::<i32>().ok())
            .map_or(0, bars);
        if !nets.iter().any(|n: &Network| n.ssid == ssid) {
            nets.push(Network {
                ssid,
                security: words[pos].to_string(),
                signal,
                connected,
            });
        }
    }
    nets
}

/// iwd's file name for a network (iwd.network(5)): the SSID itself when it
/// is only letters, digits, space, `-` and `_`; otherwise `=` and the SSID
/// in hexadecimal.
pub fn iwd_file_name(ssid: &str, security: &str) -> String {
    let plain = !ssid.is_empty()
        && ssid
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || " -_".contains(c));
    let stem = if plain {
        ssid.to_string()
    } else {
        format!(
            "={}",
            ssid.bytes().map(|b| format!("{b:02x}")).collect::<String>()
        )
    };
    format!("{stem}.{security}")
}

/// The first wireless interface, if any.
pub fn device() -> Option<String> {
    let mut v: Vec<String> = fs::read_dir("/sys/class/net")
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| {
            let d = Path::new("/sys/class/net").join(n);
            d.join("wireless").exists() || d.join("phy80211").exists()
        })
        .collect();
    v.sort();
    v.into_iter().next()
}

/// `iwctl` waits for iwd without a limit; the console state asks it on
/// every poll, so a busy iwd must not hold the state up (T28).
fn iwctl(args: &[&str]) -> String {
    Command::new("timeout")
        .args(["10", "iwctl"])
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

fn set_echo(on: bool) {
    let _ = Command::new("stty")
        .arg(if on { "echo" } else { "-echo" })
        .stdin(Stdio::inherit())
        .status();
}

/// Scan and list networks, strongest first (takes about 5 seconds).
pub fn scan(dev: &str) -> Vec<Network> {
    iwctl(&["station", dev, "scan"]);
    sleep(Duration::from_secs(4));
    let mut nets = parse_networks(&iwctl(&["station", dev, "get-networks", "rssi-dbms"]));
    nets.sort_by_key(|n| std::cmp::Reverse(n.signal));
    nets
}

/// The network `dev` is connected to (`iwctl station <dev> show`).
pub fn connected_ssid(dev: &str) -> Option<String> {
    let out = strip_ansi(&iwctl(&["station", dev, "show"]));
    let mut connected = false;
    let mut ssid = None;
    for l in out.lines() {
        let t = l.trim();
        if let Some(v) = t.strip_prefix("State") {
            connected = v.trim() == "connected";
        }
        if let Some(v) = t.strip_prefix("Connected network") {
            ssid = Some(v.trim().to_string());
        }
    }
    ssid.filter(|_| connected)
}

/// Check a connect request before anything is stored.
pub fn check_request(security: &str, password: &str) -> Result<(), crate::fail::Fail> {
    use crate::fail::Fail;
    match security {
        "psk" if !(8..=63).contains(&password.chars().count()) => Err(Fail::new(
            "wifi.password_length",
            "A Wi-Fi password has 8 to 63 characters.",
        )),
        "psk" if password.contains(['\n', '\r']) => Err(Fail::new(
            "wifi.password_length",
            "This password has characters a Wi-Fi password cannot have.",
        )),
        "psk" | "open" => Ok(()),
        "8021x" => Err(Fail::new(
            "wifi.enterprise",
            "This network needs a work or school login, which ALPArchive cannot use yet. Please pick another network.",
        )),
        _ => Err(Fail::new(
            "wifi.wep",
            "This network uses an old kind of security (WEP) that is not safe. Please pick another network.",
        )),
    }
}

/// Store the password (psk) and ask iwd to connect; waits up to 20 s for
/// iwd to report "connected". `progress` gets step names.
pub fn connect(dev: &str, net: &Network, password: &str) -> Result<(), crate::fail::Fail> {
    use crate::fail::Fail;
    check_request(&net.security, password)?;
    if net.security == "psk" {
        write_psk(&net.ssid, password).map_err(|e| {
            Fail::new(
                "wifi.failed",
                format!("The password could not be stored ({e})."),
            )
        })?;
    }
    iwctl(&["station", dev, "connect", &net.ssid]);
    for _ in 0..20 {
        if connected_ssid(dev).as_deref() == Some(net.ssid.as_str()) {
            return Ok(());
        }
        sleep(Duration::from_secs(1));
    }
    Err(if net.security == "psk" {
        Fail::new(
            "wifi.password_refused",
            "The network did not accept this password. Please try again.",
        )
    } else {
        Fail::new(
            "wifi.failed",
            "ALPArchive could not join this network. Please try again.",
        )
    })
}

/// Interactive setup: scan, choose a network by number, type the password.
/// `next` returns the next line the user typed.
pub fn setup(next: &mut dyn FnMut() -> String) {
    let say = |s: &str| {
        print!("{s}");
        let _ = std::io::stdout().flush();
    };
    say("\x1b[H\x1b[2J\x1b[?25h ALPArchive  ·  Wi-Fi\n\n");
    let Some(dev) = device() else {
        say(" No working Wi-Fi was found on this computer. Press Enter to go back.\n");
        next();
        return;
    };
    say(" Looking for networks…\n");
    let nets: Vec<Network> = scan(&dev).into_iter().take(9).collect();
    if nets.is_empty() {
        say(" No networks found. Press Enter to go back.\n");
        next();
        return;
    }
    say("\n");
    for (i, n) in nets.iter().enumerate() {
        let lock = if n.security == "open" {
            ""
        } else {
            "  (password)"
        };
        say(&format!(
            "   {}  {}{lock}\n",
            i + 1,
            crate::fail::display_safe(&n.ssid, 40)
        ));
    }
    say("\n Type the number of your network and press Enter (Enter alone: back): ");
    let Some(net) = next()
        .trim()
        .parse::<usize>()
        .ok()
        .and_then(|i| nets.get(i.wrapping_sub(1)))
    else {
        return;
    };
    if net.security == "psk" {
        say(&format!(
            " Password for {}: ",
            crate::fail::display_safe(&net.ssid, 40)
        ));
        set_echo(false);
        let pass = next();
        set_echo(true);
        say("\n");
        if !(8..=63).contains(&pass.len()) {
            say(" A Wi-Fi password has 8 to 63 characters. Press Enter to go back.\n");
            next();
            return;
        }
        if let Err(e) = write_psk(&net.ssid, &pass) {
            say(&format!(
                " Could not store the password ({e}). Press Enter to go back.\n"
            ));
            next();
            return;
        }
    } else if net.security != "open" {
        say(
            " This network needs a company login, which ALP-OS does not support. Press Enter to go back.\n",
        );
        next();
        return;
    }
    say(&format!(
        " Connecting to {}…\n",
        crate::fail::display_safe(&net.ssid, 40)
    ));
    iwctl(&["station", &dev, "connect", &net.ssid]);
}

/// Escape a value for an iwd settings file (iwd.network(5)): backslash,
/// tab, CR and LF are backslash-escaped; a leading space is `\s`.
pub fn iwd_escape(v: &str) -> String {
    let mut out = String::new();
    for (i, c) in v.chars().enumerate() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            ' ' if i == 0 => out.push_str("\\s"),
            c => out.push(c),
        }
    }
    out
}

fn write_psk(ssid: &str, pass: &str) -> std::io::Result<()> {
    if pass.contains(['\n', '\r']) {
        return Err(std::io::Error::other("invalid character"));
    }
    // Only onto the stick (review B2), atomically (review S14).
    crate::archive::state_ready().map_err(std::io::Error::other)?;
    fs::create_dir_all(IWD_DIR)?;
    let path = Path::new(IWD_DIR).join(iwd_file_name(ssid, "psk"));
    let tmp = Path::new(IWD_DIR).join(".alp-psk.tmp");
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(format!("[Security]\nPassphrase={}\n", iwd_escape(pass)).as_bytes())?;
    f.sync_all()?;
    fs::rename(&tmp, &path)?;
    fs::File::open(IWD_DIR)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_iwctl_networks() {
        let out = "\x1b[0m                               Available networks\n\
--------------------------------------------------------------------------------\n\
      Network name                      Security            Signal\n\
--------------------------------------------------------------------------------\n\
  \x1b[1;90m> \x1b[0m Home WiFi 5G                     psk                 -5200\n\
      Cafe                              open                -8000\n\
      Office                            8021x               ****\n\
      Home WiFi 5G                      psk                 -9000\n";
        let n = parse_networks(out);
        let got: Vec<(&str, &str)> = n
            .iter()
            .map(|n| (n.ssid.as_str(), n.security.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("Home WiFi 5G", "psk"),
                ("Cafe", "open"),
                ("Office", "8021x")
            ]
        );
        assert_eq!(
            n.iter()
                .map(|n| (n.signal, n.connected))
                .collect::<Vec<_>>(),
            [(4, true), (1, false), (0, false)]
        );
        assert!(parse_networks("No networks available\n").is_empty());
    }

    #[test]
    fn checks_requests() {
        assert!(check_request("psk", "12345678").is_ok());
        assert_eq!(
            check_request("psk", "short").unwrap_err().key,
            "wifi.password_length"
        );
        assert!(check_request("open", "").is_ok());
        assert_eq!(
            check_request("8021x", "").unwrap_err().key,
            "wifi.enterprise"
        );
        assert_eq!(check_request("wep", "").unwrap_err().key, "wifi.wep");
        assert_eq!(bars(-5500), 4);
        assert_eq!(bars(-70), 2);
        assert_eq!(bars(-9500), 0);
    }

    #[test]
    fn escapes_values_like_iwd() {
        assert_eq!(iwd_escape("plain pass"), "plain pass");
        assert_eq!(iwd_escape(" a\\b\tc"), "\\sa\\\\b\\tc");
    }

    #[test]
    fn iwd_names_files_like_iwd() {
        assert_eq!(iwd_file_name("Home WiFi_5G-2", "psk"), "Home WiFi_5G-2.psk");
        assert_eq!(iwd_file_name("Café", "psk"), "=436166c3a9.psk");
        assert_eq!(iwd_file_name("a/b", "open"), "=612f62.open");
    }
}
