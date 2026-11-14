// SPDX-License-Identifier: AGPL-3.0-or-later
//! Phone pairing (ADR 0017): a 5-minute window opened on the console shows a
//! QR with the server's identity (K_id), the CA's key hash, the addresses and
//! a single-use token. A phone sends the token, its name and a P-256 CSR; the
//! first request burns the token. The console then shows the device and a
//! short code that the phone shows too; the user approves with a slide, and
//! the device gets a client certificate signed by the device's CA, listed in
//! the K_id-signed, append-only device register and in nginx's allowlist.
//!
//! No own cryptography: openssl signs certificates and register entries
//! (Ed25519 with K_id, made by `alp-live-tls`); bitcoin_hashes hashes the
//! token. Window and requests live in alpd's memory only.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use bitcoin_hashes::{Hash, sha256};
use serde_json::{Value, json};

use crate::fail::Fail;
use crate::immich::TLS_DIR;

/// How long a pairing window (and its token) is valid.
pub const WINDOW: Duration = Duration::from_secs(5 * 60);
/// How long an unanswered request waits for the console.
pub const REQUEST_TTL: Duration = Duration::from_secs(5 * 60);
/// Wrong tokens before an open window closes.
pub const MAX_WRONG: u32 = 10;
/// Days a device certificate is valid. Long until phones renew their own
/// (the ALPArchive app, 0.2.5): access is decided by the allowlist, so a
/// removed phone is refused whatever its certificate says (review of 0.2.4:
/// an expired one would lock a phone out of nginx with a 400).
pub const DEVICE_DAYS: &str = "825";
/// nginx's allowlist of device certificate serials (`map` include).
pub const DEVICES_MAP: &str = "/run/alp/devices-map.conf";
/// Strict mode for nginx (ADR 0017 §3): only paired devices on the API.
pub const STRICT_CONF: &str = "/run/alp/pairing-mode.conf";

/// The register of paired devices, inside the index.
fn register_path() -> PathBuf {
    Path::new(TLS_DIR).join("devices.jsonl")
}

/// What `alp-live-tls` publishes about this server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub id: String,
    pub ca: String,
    pub endorsement: String,
    /// Endorsement v2 (0.2.6, security review S1): the CA's sequence number
    /// on this stick and K_id's signature that covers it. `None` on an
    /// identity file written before 0.2.6 (rewritten by `alp-live-tls`).
    pub seq: Option<u64>,
    pub endorsement2: Option<String>,
    pub ca_cert: String,
}

pub fn identity() -> Option<Identity> {
    let v: Value =
        serde_json::from_str(&fs::read_to_string(Path::new(TLS_DIR).join("identity.json")).ok()?)
            .ok()?;
    let s = |k: &str| v[k].as_str().map(str::to_string);
    Some(Identity {
        id: s("id")?,
        ca: s("ca")?,
        endorsement: s("endorsement")?,
        seq: v["seq"].as_u64(),
        endorsement2: s("endorsement2"),
        ca_cert: s("ca_cert")?,
    })
}

/// Lower-case unpadded base32 (RFC 4648 alphabet) of `b`.
pub fn base32(b: &[u8]) -> String {
    const A: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut out = String::new();
    let mut acc: u32 = 0;
    let mut bits = 0;
    for &x in b {
        acc = (acc << 8) | u32::from(x);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(A[((acc >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(A[((acc << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn random<const N: usize>() -> Result<[u8; N], Fail> {
    use std::io::Read;
    let mut b = [0u8; N];
    fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .map_err(|e| Fail::new("pair.failed", format!("No random numbers: {e}")))?;
    Ok(b)
}

fn token_hash(token: &str) -> [u8; 32] {
    sha256::Hash::hash(token.trim().to_ascii_lowercase().as_bytes()).to_byte_array()
}

fn eq32(a: &[u8; 32], b: &[u8; 32]) -> bool {
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// `alparchive://pair?…` (ADR 0017 §2).
pub fn pair_uri(id: &Identity, token: &str, addresses: &[String], name: &str, exp: u64) -> String {
    let enc = |s: &str| {
        s.bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                    (b as char).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect::<String>()
    };
    // e= (T45, owner D2 2026-11-03): the family login, so the app fills it
    // in and the user types only the ALP password. Never the admin account
    // (security review P3): the family user is made without admin rights.
    format!(
        "alparchive://pair?v=1&id={}&ca={}&t={token}&a={}&p=443&n={}&e={}&exp={exp}",
        id.id.to_ascii_lowercase(),
        id.ca,
        enc(&addresses.join(",")),
        enc(name),
        enc(crate::immich::FAMILY_EMAIL)
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestState {
    Pending,
    Approved { cert: String },
    Rejected,
}

#[derive(Debug, Clone)]
pub struct Request {
    pub id: String,
    pub name: String,
    pub model: String,
    pub os: String,
    pub code: String,
    csr: String,
    created: Instant,
    pub state: RequestState,
}

struct Window {
    token_hash: [u8; 32],
    expires: Instant,
    used: bool,
    wrong: u32,
}

/// alpd's pairing state (in memory).
#[derive(Default)]
pub struct Pairing {
    window: Option<Window>,
    requests: Vec<Request>,
}

fn device_text(v: &Value, k: &str, max: usize) -> String {
    v[k].as_str()
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_control() && !matches!(*c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}'))
        .take(max)
        .collect::<String>()
        .trim()
        .to_string()
}

impl Pairing {
    /// Open (or reopen) the window: a new token, the old one is gone.
    /// Returns the QR text and when it ends (unix seconds).
    pub fn open(
        &mut self,
        addresses: &[String],
        name: &str,
        now_unix: u64,
    ) -> Result<String, Fail> {
        let id = identity().ok_or_else(|| {
            Fail::new(
                "pair.not_ready",
                "ALPArchive is still preparing its certificate. Please try again in a moment.",
            )
        })?;
        let token = base32(&random::<16>()?);
        self.window = Some(Window {
            token_hash: token_hash(&token),
            expires: Instant::now() + WINDOW,
            used: false,
            wrong: 0,
        });
        Ok(pair_uri(
            &id,
            &token,
            addresses,
            name,
            now_unix + WINDOW.as_secs(),
        ))
    }

    pub fn close(&mut self) {
        self.window = None;
    }

    /// Seconds left in the open window, if one is open and unused.
    pub fn window_left(&self) -> Option<u64> {
        let w = self.window.as_ref()?;
        (!w.used)
            .then(|| w.expires.checked_duration_since(Instant::now()))
            .flatten()
            .map(|d| d.as_secs())
    }

    /// `POST /alp/pair/v1/request` from a phone. The token is burnt by the
    /// first request that carries it, whatever happens next.
    pub fn request(&mut self, body: &Value) -> Result<(String, String), Fail> {
        let unknown = || {
            Fail::new(
                "pair.token",
                "This pairing code is not valid any more. Open \"Connect a phone\" on the ALPArchive screen again.",
            )
        };
        let token = body["token"].as_str().unwrap_or("");
        let w = self.window.as_mut().ok_or_else(unknown)?;
        if w.used || Instant::now() > w.expires {
            return Err(unknown());
        }
        if !eq32(&w.token_hash, &token_hash(token)) {
            w.wrong += 1;
            if w.wrong >= MAX_WRONG {
                self.window = None;
            }
            return Err(unknown());
        }
        w.used = true;
        let csr = body["csr"].as_str().unwrap_or("").to_string();
        check_csr(&csr)?;
        let req = Request {
            id: base32(&random::<10>()?),
            name: device_text(body, "name", 60),
            model: device_text(body, "model", 60),
            os: device_text(body, "os", 40),
            code: format!("{:04}", u16::from_le_bytes(random::<2>()?) % 10000),
            csr,
            created: Instant::now(),
            state: RequestState::Pending,
        };
        let out = (req.id.clone(), req.code.clone());
        self.requests.retain(|r| r.created.elapsed() < REQUEST_TTL);
        self.requests.push(req);
        Ok(out)
    }

    /// Requests waiting for the console (newest first).
    pub fn pending(&mut self) -> Vec<Request> {
        self.requests.retain(|r| r.created.elapsed() < REQUEST_TTL);
        let mut v: Vec<Request> = self
            .requests
            .iter()
            .filter(|r| r.state == RequestState::Pending)
            .cloned()
            .collect();
        v.reverse();
        v
    }

    /// `GET /alp/pair/v1/status?id=`: what the phone polls.
    pub fn status(&mut self, id: &str) -> Value {
        self.requests.retain(|r| r.created.elapsed() < REQUEST_TTL);
        match self.requests.iter().find(|r| r.id == id) {
            None => json!({"state": "unknown"}),
            Some(r) => match &r.state {
                RequestState::Pending => json!({"state": "pending", "code": r.code}),
                RequestState::Rejected => json!({"state": "rejected"}),
                RequestState::Approved { cert } => {
                    let ident = identity();
                    json!({
                        "state": "approved", "certificate": cert,
                        "ca_cert": ident.as_ref().map(|i| i.ca_cert.clone()),
                        "identity": ident.map(|i| json!({"id": i.id, "ca": i.ca, "endorsement": i.endorsement,
                            "seq": i.seq, "endorsement2": i.endorsement2})),
                    })
                }
            },
        }
    }

    /// Approve on the console: sign the device certificate, register it,
    /// allow it in nginx.
    pub fn approve(&mut self, id: &str, now: &str) -> Result<Device, Fail> {
        let r = self
            .requests
            .iter_mut()
            .find(|r| r.id == id && r.state == RequestState::Pending)
            .ok_or_else(|| {
                Fail::new(
                    "pair.unknown",
                    "This phone's request is not waiting any more.",
                )
            })?;
        let serial = serial_hex(random::<16>()?);
        let cert = sign_device(&r.csr, &serial)?;
        let dev = Device {
            serial,
            name: r.name.clone(),
            model: r.model.clone(),
            os: r.os.clone(),
            added: now.to_string(),
            revoked: false,
        };
        register_append(&dev)?;
        write_map()?;
        r.state = RequestState::Approved { cert };
        // The phone has 5 minutes from now to fetch it.
        r.created = Instant::now();
        self.window = None;
        Ok(dev)
    }

    pub fn reject(&mut self, id: &str) -> Result<(), Fail> {
        let r = self
            .requests
            .iter_mut()
            .find(|r| r.id == id && r.state == RequestState::Pending)
            .ok_or_else(|| {
                Fail::new(
                    "pair.unknown",
                    "This phone's request is not waiting any more.",
                )
            })?;
        r.state = RequestState::Rejected;
        Ok(())
    }
}

/// A serial nginx prints the same way (`$ssl_client_serial`: upper-case hex
/// of the integer's bytes): 16 bytes, positive (top bit clear), no leading
/// zero byte.
pub fn serial_hex(mut b: [u8; 16]) -> String {
    b[0] = (b[0] & 0x7f) | 0x10;
    b.iter().map(|x| format!("{x:02X}")).collect()
}

fn run_in(cmd: &str, args: &[&str], input: &str) -> Result<String, String> {
    crate::index::run_stdin_pub(cmd, args, input)
}

/// A P-256 CSR whose signature is valid (proof the phone holds the key).
fn check_csr(csr: &str) -> Result<(), Fail> {
    let bad = |why: &str| {
        Fail::new(
            "pair.csr",
            format!("The phone sent an unusable key request ({why})."),
        )
    };
    if csr.len() > 4096 || !csr.contains("-----BEGIN CERTIFICATE REQUEST-----") {
        return Err(bad("format"));
    }
    run_in(
        "openssl",
        &["req", "-in", "/dev/stdin", "-noout", "-verify"],
        csr,
    )
    .map_err(|_| bad("signature"))?;
    let text = run_in(
        "openssl",
        &["req", "-in", "/dev/stdin", "-noout", "-text"],
        csr,
    )
    .map_err(|_| bad("unreadable"))?;
    if !(text.contains("prime256v1") || text.contains("P-256")) {
        return Err(bad("only P-256 keys"));
    }
    Ok(())
}

/// `openssl pkey -text_pub` of a P-256 (prime256v1) EC public key.
pub fn p256_key_text(text: &str) -> bool {
    text.lines().any(|l| l.trim() == "ASN1 OID: prime256v1")
        && text
            .lines()
            .any(|l| l.trim().starts_with("Public-Key: (256 bit)"))
}

/// Sign a device certificate (clientAuth, 90 days) with the CA; the
/// subject is ours, not the phone's.
fn sign_device(csr: &str, serial: &str) -> Result<String, Fail> {
    let failed = |e: String| {
        Fail::new(
            "pair.failed",
            format!("The phone's certificate could not be made: {e}."),
        )
    };
    let dir = Path::new(TLS_DIR);
    let ext = dir.join("device.ext");
    fs::write(
        &ext,
        "basicConstraints = critical,CA:FALSE\nkeyUsage = critical,digitalSignature\nextendedKeyUsage = clientAuth\nsubjectKeyIdentifier = hash\nauthorityKeyIdentifier = keyid\n",
    )
    .map_err(|e| failed(e.to_string()))?;
    let res = run_in(
        "openssl",
        &[
            "x509",
            "-req",
            "-in",
            "/dev/stdin",
            "-CA",
            &dir.join("ca.crt").to_string_lossy(),
            "-CAkey",
            &dir.join("ca.key").to_string_lossy(),
            "-set_serial",
            &format!("0x{serial}"),
            "-days",
            DEVICE_DAYS,
            "-sha256",
            "-subj",
            &format!("/O=ALPArchive/CN=alp-device-{}", &serial[..8]),
            "-extfile",
            &ext.to_string_lossy(),
        ],
        csr,
    );
    let _ = fs::remove_file(&ext);
    res.map_err(failed)
}

/// A paired device, as kept in the register.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub serial: String,
    pub name: String,
    pub model: String,
    pub os: String,
    pub added: String,
    pub revoked: bool,
}

impl Device {
    pub fn json(&self) -> Value {
        json!({"serial": self.serial, "name": self.name, "model": self.model, "os": self.os,
               "added": self.added, "revoked": self.revoked})
    }
}

/// The message K_id signs for a register line: the previous line's
/// signature (chaining the lines) and this entry.
pub fn register_message(prev_sig: &str, entry: &str) -> String {
    format!("alparchive-device-register-v1\n{prev_sig}\n{entry}\n")
}

fn sign_kid(msg: &str) -> Result<String, String> {
    // pkeyutl needs Ed25519's message as a file (one-shot signing).
    let dir = Path::new(TLS_DIR);
    let m = dir.join("sign.msg");
    let sig = dir.join("sign.sig");
    fs::write(&m, msg).map_err(|e| e.to_string())?;
    let res = crate::index::run_stdin_pub(
        "openssl",
        &[
            "pkeyutl",
            "-sign",
            "-inkey",
            &dir.join("kid.key").to_string_lossy(),
            "-rawin",
            "-in",
            &m.to_string_lossy(),
            "-out",
            &sig.to_string_lossy(),
        ],
        "",
    )
    .and_then(|_| crate::index::run_stdin_pub("base64", &["-w0", &sig.to_string_lossy()], ""));
    let _ = fs::remove_file(&m);
    let _ = fs::remove_file(&sig);
    res.map(|s| s.trim().to_string()).and_then(|s| {
        if s.is_empty() {
            Err("empty signature".into())
        } else {
            Ok(s)
        }
    })
}

/// Register lines: `{"entry": {…}, "sig": "<base64>"}`.
fn register_lines() -> Vec<Value> {
    fs::read_to_string(register_path())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn register_append(d: &Device) -> Result<(), Fail> {
    let _w = write_lock();
    register_append_entry(&d.json())
}

/// One writer at a time for the register, the signing files and the
/// allowlist (approve, revoke, strict and alpd's timer).
static WRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn write_lock() -> std::sync::MutexGuard<'static, ()> {
    WRITE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn register_append_entry(entry: &Value) -> Result<(), Fail> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let failed = |e: String| {
        Fail::new(
            "pair.failed",
            format!("The device list could not be saved: {e}."),
        )
    };
    let prev = register_lines()
        .last()
        .and_then(|l| l["sig"].as_str().map(str::to_string))
        .unwrap_or_default();
    let e = entry.to_string();
    let sig = sign_kid(&register_message(&prev, &e)).map_err(failed)?;
    let line = json!({"entry": entry, "sig": sig}).to_string();
    let mut f = fs::OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .open(register_path())
        .map_err(|e| failed(e.to_string()))?;
    f.write_all(format!("{line}\n").as_bytes())
        .and_then(|()| f.sync_all())
        .map_err(|e| failed(e.to_string()))
}

/// The devices now: later lines override earlier ones (a revocation is a
/// new line with `revoked: true`).
pub fn devices() -> Vec<Device> {
    let mut out: Vec<Device> = Vec::new();
    for l in register_lines() {
        let e = &l["entry"];
        let s = |k: &str| e[k].as_str().unwrap_or("").to_string();
        let d = Device {
            serial: s("serial"),
            name: s("name"),
            model: s("model"),
            os: s("os"),
            added: s("added"),
            revoked: e["revoked"].as_bool().unwrap_or(false),
        };
        if d.serial.is_empty() {
            continue;
        }
        match out.iter_mut().find(|x| x.serial == d.serial) {
            Some(x) if d.revoked => x.revoked = true,
            Some(_) => {}
            None => out.push(d),
        }
    }
    out
}

/// nginx's allowlist from the register (only devices not revoked).
pub fn map_text(devs: &[Device]) -> String {
    devs.iter()
        .filter(|d| !d.revoked && d.serial.chars().all(|c| c.is_ascii_hexdigit()))
        .map(|d| format!("{} 1;\n", d.serial))
        .collect()
}

/// Write the allowlist and the mode for nginx (also at start, from the
/// register).
pub fn write_map() -> Result<(), Fail> {
    let _w = write_lock();
    write_map_locked()
}

fn write_map_locked() -> Result<(), Fail> {
    let failed = |e: String| {
        Fail::new(
            "pair.failed",
            format!("The device allowlist could not be written: {e}."),
        )
    };
    let _ = fs::create_dir_all("/run/alp");
    // Not *.conf: nginx's include pattern must never see a half-written file.
    let tmp = format!("{DEVICES_MAP}.new");
    fs::write(&tmp, map_text(&devices())).map_err(|e| failed(e.to_string()))?;
    fs::rename(&tmp, DEVICES_MAP).map_err(|e| failed(e.to_string()))?;
    let mode = if strict() {
        "set $alp_strict 1;\n"
    } else {
        "set $alp_strict 0;\n"
    };
    let tmp = format!("{STRICT_CONF}.new");
    fs::write(&tmp, mode).map_err(|e| failed(e.to_string()))?;
    fs::rename(&tmp, STRICT_CONF).map_err(|e| failed(e.to_string()))
}

/// Strict mode chosen (kept in the index; compatibility mode otherwise).
pub fn strict() -> bool {
    Path::new(TLS_DIR).join("strict").exists()
}

/// Turn strict mode on or off and reload nginx.
pub fn set_strict(on: bool) -> Result<(), Fail> {
    let failed = |e: String| {
        Fail::new(
            "pair.failed",
            format!("The setting could not be saved: {e}."),
        )
    };
    let flag = Path::new(TLS_DIR).join("strict");
    if on {
        fs::write(&flag, "").map_err(|e| failed(e.to_string()))?;
    } else if flag.exists() {
        fs::remove_file(&flag).map_err(|e| failed(e.to_string()))?;
    }
    // Durable now: without it a power cut soon after (CI: the VM is stopped
    // seconds later) could bring back the old mode at the next start, and
    // phones were refused with 403 after "Only paired phones" was turned off.
    fs::File::open(TLS_DIR)
        .and_then(|d| d.sync_all())
        .map_err(|e| failed(e.to_string()))?;
    write_map()?;
    let _ = Command::new("systemctl")
        .args(["try-reload-or-restart", "nginx.service"])
        .status();
    Ok(())
}

/// Revoke a paired device: a register line, a new allowlist, nginx
/// reloaded so its open connections close within seconds (ADR 0017 §4).
pub fn revoke(serial: &str, now: &str) -> Result<(), Fail> {
    let Some(d) = devices()
        .into_iter()
        .find(|d| d.serial == serial && !d.revoked)
    else {
        return Err(Fail::new("pair.unknown", "This device is not paired."));
    };
    let mut gone = d;
    gone.revoked = true;
    let mut e = gone.json();
    e["revoked_at"] = now.into();
    {
        let _w = write_lock();
        register_append_entry(&e)?;
        write_map_locked()?;
    }
    let _ = Command::new("systemctl")
        .args(["reload", "nginx.service"])
        .status();
    Ok(())
}

/// `alpctl live pair write-map`: the allowlist and mode for nginx, from the
/// register (before nginx starts or reloads). Without the index, the mode
/// file is not written and nginx stays strict with an empty list.
pub fn cli(args: &[&str]) -> i32 {
    match args {
        ["write-map"] => {
            if !crate::index::is_open() {
                eprintln!("alpctl: the index is not open; nginx stays strict");
                return 0;
            }
            match write_map() {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("alpctl: {}", e.message);
                    1
                }
            }
        }
        _ => {
            eprintln!("usage: alpctl live pair write-map");
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_p256_key_passes() {
        let p256 =
            "Public-Key: (256 bit)\npub:\n    04:aa\nASN1 OID: prime256v1\nNIST CURVE: P-256\n";
        assert!(p256_key_text(p256));
        assert!(!p256_key_text(
            "Public-Key: (2048 bit)\nModulus:\n    00:aa\nExponent: 65537\n"
        ));
        assert!(!p256_key_text(
            "Public-Key: (384 bit)\nASN1 OID: secp384r1\n"
        ));
    }

    #[test]
    fn base32_matches_rfc4648() {
        assert_eq!(base32(b""), "");
        assert_eq!(base32(b"f"), "my");
        assert_eq!(base32(b"foobar"), "mzxw6ytboi");
    }

    #[test]
    fn serials_are_positive_without_leading_zero() {
        let s = serial_hex([0xff; 16]);
        assert_eq!(s.len(), 32);
        assert!(s.starts_with("7F"));
        assert!(serial_hex([0; 16]).starts_with("10"));
    }

    #[test]
    fn uri_carries_identity_token_and_addresses() {
        let id = Identity {
            id: "ABC".into(),
            ca: "00ff".into(),
            endorsement: String::new(),
            seq: Some(1),
            endorsement2: None,
            ca_cert: String::new(),
        };
        let u = pair_uri(
            &id,
            "tok",
            &["192.168.1.2".into(), "10.0.0.3".into()],
            "Home ALP",
            99,
        );
        assert_eq!(
            u,
            "alparchive://pair?v=1&id=abc&ca=00ff&t=tok&a=192.168.1.2%2C10.0.0.3&p=443&n=Home%20ALP&e=family%40alparchive.local&exp=99"
        );
    }

    #[test]
    fn map_lists_only_devices_not_revoked() {
        let d = |s: &str, r| Device {
            serial: s.into(),
            name: String::new(),
            model: String::new(),
            os: String::new(),
            added: String::new(),
            revoked: r,
        };
        assert_eq!(
            map_text(&[d("1A", false), d("2B", true), d("x;", false)]),
            "1A 1;\n"
        );
    }

    #[test]
    fn a_token_is_used_once_and_wrong_ones_close_the_window() {
        let fresh = || Window {
            token_hash: token_hash("good"),
            expires: Instant::now() + WINDOW,
            used: false,
            wrong: 0,
        };
        let mut p = Pairing {
            window: Some(fresh()),
            requests: Vec::new(),
        };
        // A wrong token counts; the right one is burnt even when its CSR is bad.
        assert!(p.request(&json!({"token": "bad"})).is_err());
        assert_eq!(
            p.request(&json!({"token": "good", "csr": "x"}))
                .unwrap_err()
                .key,
            "pair.csr"
        );
        assert_eq!(
            p.request(&json!({"token": "good"})).unwrap_err().key,
            "pair.token"
        );
        assert_eq!(p.window_left(), None);
        p.window = Some(fresh());
        for _ in 0..MAX_WRONG {
            let _ = p.request(&json!({"token": "nope"}));
        }
        assert!(p.window.is_none());
    }
}
