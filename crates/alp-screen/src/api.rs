// SPDX-License-Identifier: AGPL-3.0-or-later
//! A small client for alpd's console API on `127.0.0.1:8411`.
//!
//! alpd answers one request per connection (`Connection: close`) with a
//! `Content-Length`, so a plain blocking HTTP/1.1 exchange over a local TCP
//! socket is enough: no HTTP library, nothing that can reach another host.
//! The screen sends what the web console's browser sends: the console's
//! `Origin`, a JSON body and the console session cookie (the same CSRF rules
//! apply, `crates/alp-core/src/console.rs`).

use serde_json::Value;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::sync::Mutex;
use std::time::Duration;

/// The console's own origin; alpd refuses a POST without exactly this.
pub const ORIGIN: &str = "http://127.0.0.1:8411";
const COOKIE: &str = "alp_console";
const PORT: u16 = 8411;
/// Larger answers are refused (the state is a few tens of KB).
const MAX_ANSWER: usize = 8 * 1024 * 1024;
/// Never wait forever for alpd (same limit as the web console).
const TIMEOUT: Duration = Duration::from_secs(20);

/// An error from alpd (its `error` object) or from the connection.
#[derive(Debug, Clone, PartialEq)]
pub struct ApiError {
    /// alpd's error key, or `api.unreachable` when alpd did not answer.
    pub key: String,
    pub message: String,
    pub status: u16,
    pub retry_after: u64,
}

impl ApiError {
    fn unreachable() -> Self {
        ApiError {
            key: "api.unreachable".into(),
            message: String::new(),
            status: 0,
            retry_after: 0,
        }
    }
}

/// One console session: the cookie alpd set at sign-in, kept in memory only.
pub struct Client {
    addr: SocketAddr,
    session: Mutex<Option<zeroize::Zeroizing<String>>>,
}

impl Default for Client {
    fn default() -> Self {
        Self::new(PORT)
    }
}

impl Client {
    /// A client for alpd on this computer (port given for tests).
    pub fn new(port: u16) -> Self {
        Client {
            addr: SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
            session: Mutex::new(None),
        }
    }

    pub fn get(&self, path: &str) -> Result<Value, ApiError> {
        self.call("GET", path, None)
    }

    pub fn post(&self, path: &str, body: &Value) -> Result<Value, ApiError> {
        self.call("POST", path, Some(body))
    }

    /// The session cookie's value, for the gallery view (alpd's gallery
    /// gate checks the same cookie; nginx never passes it to the photo
    /// service).
    pub fn session(&self) -> Option<zeroize::Zeroizing<String>> {
        self.session.lock().ok().and_then(|s| s.clone())
    }

    /// Forgets the session (after locking, or when alpd says it is locked).
    pub fn forget(&self) {
        if let Ok(mut s) = self.session.lock() {
            *s = None;
        }
    }

    fn call(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value, ApiError> {
        let cookie = self
            .session
            .lock()
            .ok()
            .and_then(|s| s.as_ref().map(|c| c.to_string()));
        let req = request(method, path, body, cookie.as_deref());
        let mut s =
            TcpStream::connect_timeout(&self.addr, TIMEOUT).map_err(|_| ApiError::unreachable())?;
        let _ = s.set_read_timeout(Some(TIMEOUT));
        let _ = s.set_write_timeout(Some(TIMEOUT));
        s.write_all(req.as_bytes())
            .map_err(|_| ApiError::unreachable())?;
        let mut raw = Vec::new();
        s.take(MAX_ANSWER as u64 + 1)
            .read_to_end(&mut raw)
            .map_err(|_| ApiError::unreachable())?;
        if raw.len() > MAX_ANSWER {
            return Err(ApiError::unreachable());
        }
        let answer = parse(&raw).ok_or_else(ApiError::unreachable)?;
        if let Some(c) = answer.session
            && let Ok(mut s) = self.session.lock()
        {
            *s = Some(zeroize::Zeroizing::new(c));
        }
        if (200..300).contains(&answer.status) {
            return Ok(answer.body);
        }
        let e = &answer.body["error"];
        let err = ApiError {
            key: e["key"].as_str().unwrap_or("internal").to_string(),
            message: e["message"].as_str().unwrap_or_default().to_string(),
            status: answer.status,
            retry_after: e["retry_after_seconds"].as_u64().unwrap_or(0),
        };
        if err.key == "console.locked" {
            self.forget();
        }
        Err(err)
    }
}

/// The request text. `path` is below `/api/v1/` and comes from this crate's
/// own code, never from typed input.
fn request(method: &str, path: &str, body: Option<&Value>, cookie: Option<&str>) -> String {
    let mut r = format!(
        "{method} /api/v1/{path} HTTP/1.1\r\nHost: 127.0.0.1:{PORT}\r\nConnection: close\r\n"
    );
    if let Some(c) = cookie {
        r += &format!("Cookie: {COOKIE}={c}\r\n");
    }
    match body {
        Some(b) => {
            let text = b.to_string();
            r += &format!(
                "Origin: {ORIGIN}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{text}",
                text.len()
            );
        }
        None => r += "\r\n",
    }
    r
}

struct Answer {
    status: u16,
    session: Option<String>,
    body: Value,
}

fn parse(raw: &[u8]) -> Option<Answer> {
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&raw[..split]).ok()?;
    let rest = &raw[split + 4..];
    let mut lines = head.split("\r\n");
    let status = lines.next()?.split(' ').nth(1)?.parse().ok()?;
    let mut len = None;
    let mut session = None;
    for l in lines {
        let (k, v) = l.split_once(':')?;
        let v = v.trim();
        if k.eq_ignore_ascii_case("content-length") {
            len = v.parse::<usize>().ok();
        } else if k.eq_ignore_ascii_case("set-cookie") {
            let first = v.split(';').next().unwrap_or_default();
            if let Some(c) = first.strip_prefix(&format!("{COOKIE}=")) {
                session = Some(c.to_string());
            }
        }
    }
    let body = &rest[..len.unwrap_or(rest.len()).min(rest.len())];
    let body = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(body).ok()?
    };
    Some(Answer {
        status,
        session,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::net::TcpListener;

    #[test]
    fn post_carries_origin_json_and_cookie() {
        let r = request("POST", "live/console/lock", Some(&json!({})), Some("abc"));
        assert!(r.starts_with("POST /api/v1/live/console/lock HTTP/1.1\r\n"));
        assert!(r.contains("\r\nOrigin: http://127.0.0.1:8411\r\n"));
        assert!(r.contains("\r\nContent-Type: application/json\r\n"));
        assert!(r.contains("\r\nCookie: alp_console=abc\r\n"));
        assert!(r.ends_with("\r\n\r\n{}"));
    }

    #[test]
    fn get_has_no_body_and_no_origin() {
        let r = request("GET", "live/state", None, None);
        assert!(!r.contains("Origin"));
        assert!(r.ends_with("Connection: close\r\n\r\n"));
    }

    #[test]
    fn parses_status_cookie_and_error() {
        let a = parse(b"HTTP/1.1 200 OK\r\nSet-Cookie: alp_console=xyz; Path=/api/; HttpOnly\r\nContent-Length: 2\r\n\r\n{}").unwrap();
        assert_eq!(a.status, 200);
        assert_eq!(a.session.as_deref(), Some("xyz"));
        let b = parse(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 47\r\n\r\n{\"error\":{\"key\":\"console.locked\",\"message\":\"\"}}").unwrap();
        assert_eq!(b.status, 403);
        assert_eq!(b.body["error"]["key"], "console.locked");
        assert!(parse(b"garbage").is_none());
    }

    #[test]
    fn keeps_the_session_and_forgets_it_when_locked() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let answers = [
                "HTTP/1.1 200 OK\r\nSet-Cookie: alp_console=s1; Path=/api/\r\nContent-Length: 2\r\n\r\n{}",
                "HTTP/1.1 403 Forbidden\r\nContent-Length: 47\r\n\r\n{\"error\":{\"key\":\"console.locked\",\"message\":\"\"}}",
            ];
            let mut seen = Vec::new();
            for a in answers {
                let (mut s, _) = l.accept().unwrap();
                let mut buf = [0u8; 4096];
                let n = s.read(&mut buf).unwrap();
                seen.push(String::from_utf8_lossy(&buf[..n]).to_string());
                s.write_all(a.as_bytes()).unwrap();
            }
            seen
        });
        let c = Client::new(port);
        c.post("live/console/signin", &json!({"password": "x"}))
            .unwrap();
        let e = c.get("live/wifi/networks").unwrap_err();
        assert_eq!(e.key, "console.locked");
        assert!(c.session.lock().unwrap().is_none());
        let seen = server.join().unwrap();
        assert!(seen[1].contains("Cookie: alp_console=s1"));
    }

    #[test]
    fn no_alpd_is_unreachable() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        assert_eq!(
            Client::new(port).get("live/state").unwrap_err().key,
            "api.unreachable"
        );
    }
}
