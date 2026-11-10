// SPDX-License-Identifier: AGPL-3.0-or-later
//! The server certificate phones see, made sure of (0.2.5 field report).
//!
//! `alp-live-tls` writes a new server certificate when an address is missing
//! from it, and alpd reloads nginx. Nothing checked that nginx really sent
//! the new certificate: on the owner's laptop the phone got one without the
//! computer's address ("the server's certificate does not name
//! 192.168.0.12") and the official Immich app failed the same hostname check
//! without saying why. Now alpd connects to each address on 443 itself,
//! compares the certificate it gets with the file, restarts nginx when they
//! differ, and reports what it found. The QR is shown only when this holds.
//!
//! Only the system's `openssl` and `timeout` are used (already in the image):
//! no new dependency, no cryptography of our own. Certificates are public;
//! nothing secret is logged.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{Value, json};

use crate::immich::{TLS_DIR, TLS_TOOL};

/// nginx's HTTPS port for phones.
pub const PORT: u16 = 443;

/// What a phone connecting to one address gets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Served {
    pub address: String,
    /// SHA-256 of the certificate nginx sent, `AB:CD:…`; `None` if no TLS
    /// answer came within 5 s.
    pub sha256: Option<String>,
    /// The names in that certificate (`DNS:…`, `IP Address:…`).
    pub names: Vec<String>,
}

impl Served {
    /// The certificate sent is the one on the stick and names this address.
    pub fn ok(&self, file_sha256: Option<&str>) -> bool {
        self.sha256.is_some()
            && self.sha256.as_deref() == file_sha256
            && self
                .names
                .iter()
                .any(|n| n == &format!("IP Address:{}", self.address))
    }

    pub fn json(&self, file_sha256: Option<&str>) -> Value {
        json!({
            "address": self.address,
            "answers": self.sha256.is_some(),
            "sha256": self.sha256,
            "names": self.names,
            "same_as_file": self.sha256.is_some() && self.sha256.as_deref() == file_sha256,
            "names_address": self.names.iter().any(|n| n == &format!("IP Address:{}", self.address)),
            "ok": self.ok(file_sha256),
        })
    }
}

/// Why phones cannot use the certificate now; the message is for the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotServed {
    pub message: String,
    pub served: Vec<Served>,
}

/// The last (file fingerprint, addresses) that checked out, so the phone
/// screen's frequent calls do not open a TLS connection each time.
static VERIFIED: Mutex<Option<(String, Vec<String>)>> = Mutex::new(None);

/// One `ensure` at a time: alp-live-tls writes fixed temporary file names,
/// and the watcher and the phone screen may call at the same moment.
static ENSURE: Mutex<()> = Mutex::new(());

/// When nginx was last restarted for a wrong certificate: at most once a
/// minute, however often the phone screen asks.
static LAST_RESTART: Mutex<Option<std::time::Instant>> = Mutex::new(None);

fn verified_lock() -> std::sync::MutexGuard<'static, Option<(String, Vec<String>)>> {
    VERIFIED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Only plain IPv4 text goes to a command line (it comes from `ip`, but
/// nothing else is ever passed on).
pub fn is_ipv4(a: &str) -> bool {
    let parts: Vec<&str> = a.split('.').collect();
    parts.len() == 4
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.len() <= 3
                && p.bytes().all(|b| b.is_ascii_digit())
                && p.parse::<u8>().is_ok()
        })
}

/// The first PEM certificate in `text` (as `openssl s_client` prints it).
pub fn first_pem(text: &str) -> Option<String> {
    const BEGIN: &str = "-----BEGIN CERTIFICATE-----";
    const END: &str = "-----END CERTIFICATE-----";
    let start = text.find(BEGIN)?;
    let end = text[start..].find(END)? + start + END.len();
    Some(format!("{}\n", &text[start..end]))
}

/// The names in `openssl x509 -noout -ext subjectAltName` output.
pub fn parse_names(out: &str) -> Vec<String> {
    out.lines()
        .skip_while(|l| !l.contains("Subject Alternative Name"))
        .skip(1)
        .flat_map(|l| l.split(','))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// `openssl x509 -fingerprint -sha256` output to `AB:CD:…` (`None` otherwise).
pub fn parse_fingerprint(out: &str) -> Option<String> {
    crate::immich::fingerprint(out)
}

fn x509(pem: &str, args: &[&str]) -> Option<String> {
    let mut child = Command::new("openssl")
        .arg("x509")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(pem.as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn describe(pem: &str) -> (Option<String>, Vec<String>) {
    let sha = x509(pem, &["-noout", "-fingerprint", "-sha256"]).and_then(|o| parse_fingerprint(&o));
    let names = x509(pem, &["-noout", "-ext", "subjectAltName"])
        .map(|o| parse_names(&o))
        .unwrap_or_default();
    (sha, names)
}

/// SHA-256 and names of the certificate file nginx is configured with.
pub fn file_certificate() -> (Option<String>, Vec<String>) {
    match std::fs::read_to_string(Path::new(TLS_DIR).join("server.crt")) {
        Ok(pem) => describe(&pem),
        Err(_) => (None, Vec::new()),
    }
}

/// What nginx sends a client connecting to `address`:443 right now.
pub fn served(address: &str) -> Served {
    let mut s = Served {
        address: address.to_string(),
        sha256: None,
        names: Vec::new(),
    };
    if !is_ipv4(address) {
        return s;
    }
    // No client certificate, no SNI (phones connect to the bare address).
    let out = Command::new("timeout")
        .args([
            "5",
            "openssl",
            "s_client",
            "-connect",
            &format!("{address}:{PORT}"),
            "-noservername",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    if let Some(pem) = out
        .ok()
        .and_then(|o| first_pem(&String::from_utf8_lossy(&o.stdout)))
    {
        let (sha, names) = describe(&pem);
        s.sha256 = sha;
        s.names = names;
    }
    s
}

fn served_all(addresses: &[String]) -> Vec<Served> {
    addresses.iter().map(|a| served(a)).collect()
}

fn systemctl(args: &[&str]) -> bool {
    Command::new("systemctl")
        .args(args)
        .status()
        .is_ok_and(|s| s.success())
}

/// Make sure phones connecting to every one of `addresses` get a
/// certificate that names it: run `alp-live-tls`, reload nginx when the
/// certificate changed, then check what nginx really sends. When it is not
/// the file's certificate, restart nginx and check again (up to ~10 s).
pub fn ensure(addresses: &[String]) -> Result<(), NotServed> {
    let _one = ENSURE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let addresses: Vec<String> = addresses.iter().filter(|a| is_ipv4(a)).cloned().collect();
    let out = Command::new(TLS_TOOL).args(&addresses).output();
    let changed = match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).contains("changed"),
        Ok(o) => {
            let e = String::from_utf8_lossy(&o.stderr).trim().to_string();
            eprintln!("alpd: tls: certificate not made: {e}");
            return Err(NotServed {
                message: "The secure connection for phones could not be prepared. Restart the computer and try again.".into(),
                served: Vec::new(),
            });
        }
        Err(e) => {
            eprintln!("alpd: tls: certificate not made: {e}");
            return Err(NotServed {
                message: "The secure connection for phones could not be prepared. Restart the computer and try again.".into(),
                served: Vec::new(),
            });
        }
    };
    let (file_sha, file_names) = file_certificate();
    let Some(file_sha) = file_sha else {
        eprintln!("alpd: tls: the server certificate file cannot be read");
        return Err(NotServed {
            message:
                "The secure connection for phones is not ready. Restart the computer and try again."
                    .into(),
            served: Vec::new(),
        });
    };
    if changed {
        eprintln!(
            "alpd: tls: new server certificate {file_sha} naming {}",
            file_names.join(", ")
        );
        *verified_lock() = None;
        if !systemctl(&["try-reload-or-restart", "nginx.service"]) {
            eprintln!("alpd: tls: nginx reload failed");
        }
    }
    if verified_lock().as_ref() == Some(&(file_sha.clone(), addresses.clone())) {
        return Ok(());
    }
    if addresses.is_empty() {
        return Ok(()); // nothing a phone could connect to yet
    }
    if !systemctl(&["is-active", "--quiet", "nginx.service"]) {
        // nginx runs only with Immich; nothing to check before it starts.
        return Err(NotServed {
            message: "The connection for phones is not running yet. Wait until the photo service has started.".into(),
            served: Vec::new(),
        });
    }
    let mut served = served_all(&addresses);
    // A reload takes a moment to reach new connections.
    for _ in 0..3 {
        if served.iter().all(|s| s.ok(Some(&file_sha))) {
            break;
        }
        std::thread::sleep(Duration::from_secs(1));
        served = served_all(&addresses);
    }
    let may_restart = {
        let mut last = LAST_RESTART
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let ok = last.is_none_or(|t| t.elapsed() >= Duration::from_secs(60));
        if ok && !served.iter().all(|s| s.ok(Some(&file_sha))) {
            *last = Some(std::time::Instant::now());
        }
        ok
    };
    if may_restart && !served.iter().all(|s| s.ok(Some(&file_sha))) {
        for s in &served {
            eprintln!(
                "alpd: tls: {} serves {} naming {} (file {file_sha}); restarting nginx",
                s.address,
                s.sha256.as_deref().unwrap_or("nothing"),
                s.names.join(", ")
            );
        }
        if !systemctl(&["restart", "nginx.service"]) {
            eprintln!("alpd: tls: nginx restart failed");
        }
        for _ in 0..5 {
            std::thread::sleep(Duration::from_secs(1));
            served = served_all(&addresses);
            if served.iter().all(|s| s.ok(Some(&file_sha))) {
                break;
            }
        }
    }
    if served.iter().all(|s| s.ok(Some(&file_sha))) {
        eprintln!(
            "alpd: tls: phones get certificate {file_sha} on {}",
            addresses.join(", ")
        );
        *verified_lock() = Some((file_sha, addresses));
        return Ok(());
    }
    for s in &served {
        eprintln!(
            "alpd: tls: still wrong: {} serves {} naming {}",
            s.address,
            s.sha256.as_deref().unwrap_or("nothing"),
            s.names.join(", ")
        );
    }
    Err(NotServed {
        message: "Phones cannot connect safely yet: the server's certificate does not match this network address. Restart the computer with Wi-Fi or the cable connected, then try again.".into(),
        served,
    })
}

/// For the connection check screen: the file, what each address serves,
/// nginx's state and the pairing mode. Opens one TLS connection per address.
pub fn check(addresses: &[String]) -> Value {
    let (file_sha, file_names) = file_certificate();
    let served: Vec<Value> = addresses
        .iter()
        .filter(|a| is_ipv4(a))
        .map(|a| served(a).json(file_sha.as_deref()))
        .collect();
    json!({
        "certificate": {"sha256": file_sha, "names": file_names},
        "ca_sha256": crate::immich::certificate_sha256(),
        "nginx_active": systemctl(&["is-active", "--quiet", "nginx.service"]),
        "served": served,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv4_only() {
        assert!(is_ipv4("192.168.0.12"));
        assert!(is_ipv4("10.0.0.1"));
        assert!(!is_ipv4("192.168.0.256"));
        assert!(!is_ipv4("192.168.0"));
        assert!(!is_ipv4("1.2.3.4;rm"));
        assert!(!is_ipv4("-connect"));
        assert!(!is_ipv4("fe80::1"));
    }

    #[test]
    fn names_from_openssl() {
        let out = "X509v3 Subject Alternative Name: \n    DNS:alparchive.local, IP Address:192.168.0.12\n";
        assert_eq!(
            parse_names(out),
            ["DNS:alparchive.local", "IP Address:192.168.0.12"]
        );
        assert!(parse_names("").is_empty());
    }

    #[test]
    fn pem_from_s_client() {
        let out = "CONNECTED(00000003)\n---\nServer certificate\n-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\nsubject=CN=x\n";
        assert_eq!(
            first_pem(out).unwrap(),
            "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n"
        );
        assert!(first_pem("CONNECTED\nno peer certificate available\n").is_none());
    }

    #[test]
    fn served_ok_needs_same_cert_and_address() {
        let s = Served {
            address: "192.168.0.12".into(),
            sha256: Some("AA".into()),
            names: vec![
                "DNS:alparchive.local".into(),
                "IP Address:192.168.0.12".into(),
            ],
        };
        assert!(s.ok(Some("AA")));
        assert!(!s.ok(Some("BB")));
        let old = Served {
            names: vec!["DNS:alparchive.local".into()],
            ..s.clone()
        };
        assert!(!old.ok(Some("AA")));
        let none = Served { sha256: None, ..s };
        assert!(!none.ok(None));
    }
}
