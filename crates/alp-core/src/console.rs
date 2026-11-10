// SPDX-License-Identifier: AGPL-3.0-or-later
//! The console listener (`docs/contracts/api-access.md`): plain HTTP/1.1 on
//! `127.0.0.1:8411` only, for the kiosk browser on the computer's own
//! screen. Serves the static UI and hands `/api/` requests to a handler.
//!
//! std only: one thread per connection, one request per connection
//! (`Connection: close`). Rules enforced here for every request: the Host
//! header (DNS rebinding), `Sec-Fetch-Site` for the API, and for
//! state-changing requests the Origin and JSON Content-Type (CSRF).

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub const ORIGIN: &str = "http://127.0.0.1:8411";
const HOSTS: [&str; 2] = ["127.0.0.1:8411", "localhost:8411"];
const MAX_HEAD: usize = 16 << 10;
pub const MAX_BODY: usize = 64 << 10;
/// `frame-src`: only the Immich web gallery and the Terminal (ttyd, feature
/// `terminal`) on loopback, shown under the
/// console's own top bar in the kiosk (T16, `immich.console_url`).
const CSP: &str = "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; frame-src http://127.0.0.1:2283 http://127.0.0.1:8413";

#[derive(Debug, Default)]
pub struct Request {
    pub method: String,
    /// Path without the query string.
    pub path: String,
    pub query: String,
    /// Lower-case names.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn json(status: u16, v: &serde_json::Value) -> Self {
        Self {
            status,
            content_type: "application/json",
            headers: vec![],
            body: v.to_string().into_bytes(),
        }
    }

    /// Error body of docs/plans/live-alpha-api.md.
    pub fn error(status: u16, key: &str, message: &str) -> Self {
        Self::json(
            status,
            &serde_json::json!({"error": {"key": key, "message": message, "code": null}}),
        )
    }
}

pub type Handler = dyn Fn(&Request) -> Response + Send + Sync;

pub struct Server {
    pub ui_dir: PathBuf,
    pub api: Box<Handler>,
}

impl Server {
    /// Accept connections forever.
    pub fn run(self: Arc<Self>, listener: TcpListener) {
        for stream in listener.incoming().flatten() {
            let me = Arc::clone(&self);
            std::thread::spawn(move || me.connection(stream));
        }
    }

    fn connection(&self, mut s: TcpStream) {
        let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
        let _ = s.set_write_timeout(Some(Duration::from_secs(30)));
        let resp = match read_request(&mut s) {
            Ok(req) => self.handle(&req),
            Err(r) => r,
        };
        let _ = write_response(&mut s, &resp);
    }

    /// Everything after reading: checks, then the API or a file.
    pub fn handle(&self, req: &Request) -> Response {
        if !req.header("host").is_some_and(|h| HOSTS.contains(&h)) {
            return Response::error(421, "request.forbidden", "Wrong host.");
        }
        let changes = !matches!(req.method.as_str(), "GET" | "HEAD");
        if req.path.starts_with("/api/") {
            // The Immich gallery framed in the console (127.0.0.1:2283) is
            // the same site: the browser would send the console's cookie
            // with its requests, also GETs (archive/candidates mounts disks).
            // A phone pairing through nginx (ADR 0017) uses no cookie: its
            // single-use token is the credential, so CSRF does not apply.
            let phone = matches!(
                req.path.as_str(),
                "/api/v1/live/pair/v1/request" | "/api/v1/live/pair/v1/status"
            );
            if !phone && (!fetch_site_ok(req) || changes && !csrf_ok(req)) {
                return Response::error(
                    403,
                    "request.forbidden",
                    "This request is only allowed from the ALPArchive screen.",
                );
            }
            return (self.api)(req);
        }
        if changes {
            return Response::error(405, "request.method", "Not allowed here.");
        }
        self.file(&req.path)
    }

    fn file(&self, path: &str) -> Response {
        if path == "/" {
            let mut r = Response::error(302, "request.redirect", "/live/");
            r.headers.push(("Location", "/live/".into()));
            return r;
        }
        let not_found = || Response::error(404, "request.not_found", "Not found.");
        let Some(rel) = safe_relative(path) else {
            return not_found();
        };
        let mut p = self.ui_dir.join(rel);
        if path.ends_with('/') {
            p = p.join("index.html");
        }
        // Symlinks out of the UI folder are refused.
        let (Ok(root), Ok(real)) = (self.ui_dir.canonicalize(), p.canonicalize()) else {
            return not_found();
        };
        if !real.starts_with(&root) || !real.is_file() {
            return not_found();
        }
        match fs::read(&real) {
            Ok(body) => Response {
                status: 200,
                content_type: content_type(&real),
                headers: vec![("Content-Security-Policy", CSP.into())],
                body,
            },
            Err(_) => not_found(),
        }
    }
}

/// `Origin` exactly the console's, and a JSON body.
fn csrf_ok(req: &Request) -> bool {
    let json = req
        .header("content-type")
        .and_then(|c| c.split(';').next())
        .is_some_and(|c| c.trim().eq_ignore_ascii_case("application/json"));
    req.header("origin") == Some(ORIGIN) && json
}

/// `Sec-Fetch-Site`, when the browser sends it: only the console's own
/// pages (or the address typed in the kiosk) may call the API.
fn fetch_site_ok(req: &Request) -> bool {
    req.header("sec-fetch-site")
        .is_none_or(|v| v == "same-origin" || v == "none")
}

/// A URL path as a relative file path, or `None` if it could leave the UI
/// folder or is unusual (encoded characters, backslashes, dot segments).
fn safe_relative(path: &str) -> Option<PathBuf> {
    let rel = path.strip_prefix('/')?;
    if rel.contains(['%', '\\', '\0']) {
        return None;
    }
    let mut out = PathBuf::new();
    for seg in rel.split('/') {
        match seg {
            "" => {}
            "." | ".." => return None,
            s if s.starts_with('.') => return None,
            s => out.push(s),
        }
    }
    Some(out)
}

fn content_type(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn read_request(s: &mut impl Read) -> Result<Request, Response> {
    let bad = |m: &str| Response::error(400, "request.bad", m);
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > MAX_HEAD {
            return Err(Response::error(
                431,
                "request.too_large",
                "Request too large.",
            ));
        }
        let n = s
            .read(&mut chunk)
            .map_err(|_| bad("Request not complete."))?;
        if n == 0 {
            return Err(bad("Request not complete."));
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = std::str::from_utf8(&buf[..head_end]).map_err(|_| bad("Bad request."))?;
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or("").split(' ');
    let (method, target) = match (first.next(), first.next(), first.next()) {
        (Some(m), Some(t), Some(v)) if v.starts_with("HTTP/1.") => (m, t),
        _ => return Err(bad("Bad request line.")),
    };
    let mut headers = Vec::new();
    for l in lines {
        let (k, v) = l.split_once(':').ok_or_else(|| bad("Bad header."))?;
        headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let mut req = Request {
        method: method.to_string(),
        path: path.to_string(),
        query: query.to_string(),
        headers,
        body: Vec::new(),
    };
    if req.header("transfer-encoding").is_some() {
        return Err(Response::error(
            411,
            "request.bad",
            "Send a Content-Length.",
        ));
    }
    let len = match req.header("content-length") {
        None => 0,
        Some(v) => v.parse::<usize>().map_err(|_| bad("Bad Content-Length."))?,
    };
    if len > MAX_BODY {
        return Err(Response::error(
            413,
            "request.too_large",
            "Request too large.",
        ));
    }
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < len {
        let n = s
            .read(&mut chunk)
            .map_err(|_| bad("Request not complete."))?;
        if n == 0 {
            return Err(bad("Request not complete."));
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(len);
    req.body = body;
    Ok(req)
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        302 => "Found",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        411 => "Length Required",
        413 => "Content Too Large",
        421 => "Misdirected Request",
        422 => "Unprocessable Content",
        431 => "Request Header Fields Too Large",
        503 => "Service Unavailable",
        _ => "Error",
    }
}

fn write_response(s: &mut impl Write, r: &Response) -> std::io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n",
        r.status,
        reason(r.status),
        r.content_type,
        r.body.len()
    );
    for (k, v) in &r.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    s.write_all(head.as_bytes())?;
    s.write_all(&r.body)?;
    s.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(dir: &Path) -> Server {
        Server {
            ui_dir: dir.to_path_buf(),
            api: Box::new(|r: &Request| {
                Response::json(
                    200,
                    &serde_json::json!({"path": r.path, "len": r.body.len()}),
                )
            }),
        }
    }

    fn ui() -> PathBuf {
        let d = std::env::temp_dir().join(format!("alpd-ui-{}", std::process::id()));
        let _ = fs::create_dir_all(d.join("live"));
        fs::write(d.join("live/index.html"), "<p>hi</p>").unwrap();
        fs::write(d.join("live/app.js"), "x").unwrap();
        fs::write(d.join("secret.txt"), "no").unwrap();
        d
    }

    fn req(raw: &str) -> Request {
        read_request(&mut raw.as_bytes()).unwrap()
    }

    #[test]
    fn parses_requests() {
        let r = req(
            "POST /api/v1/x?a=1 HTTP/1.1\r\nHost: 127.0.0.1:8411\r\nContent-Length: 2\r\n\r\n{}",
        );
        assert_eq!(
            (r.method.as_str(), r.path.as_str(), r.query.as_str()),
            ("POST", "/api/v1/x", "a=1")
        );
        assert_eq!(r.body, b"{}");
        let big = format!(
            "POST / HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            MAX_BODY + 1
        );
        assert_eq!(read_request(&mut big.as_bytes()).unwrap_err().status, 413);
        let chunked = "POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n";
        assert_eq!(
            read_request(&mut chunked.as_bytes()).unwrap_err().status,
            411
        );
        assert_eq!(
            read_request(&mut "nonsense\r\n\r\n".as_bytes())
                .unwrap_err()
                .status,
            400
        );
    }

    #[test]
    fn enforces_host_origin_and_json() {
        let s = server(&ui());
        let get = req("GET /api/v1/live/state HTTP/1.1\r\nHost: 127.0.0.1:8411\r\n\r\n");
        assert_eq!(s.handle(&get).status, 200);
        let rebind = req("GET /api/v1/live/state HTTP/1.1\r\nHost: evil.example:8411\r\n\r\n");
        assert_eq!(s.handle(&rebind).status, 421);
        let good = "POST /api/v1/live/chain/create HTTP/1.1\r\nHost: 127.0.0.1:8411\r\nOrigin: http://127.0.0.1:8411\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: 2\r\n\r\n{}";
        assert_eq!(s.handle(&req(good)).status, 200);
        let no_origin = good.replace("Origin: http://127.0.0.1:8411\r\n", "");
        assert_eq!(s.handle(&req(&no_origin)).status, 403);
        let other_origin = good.replace(
            "http://127.0.0.1:8411\r\nContent",
            "http://localhost:8411\r\nContent",
        );
        assert_eq!(s.handle(&req(&other_origin)).status, 403);
        let form = good.replace("application/json; charset=utf-8", "text/plain");
        assert_eq!(s.handle(&req(&form)).status, 403);
        // The framed gallery (127.0.0.1:2283) is the same site, not the same origin.
        let framed = |site: &str| {
            req(&format!(
                "GET /api/v1/live/state HTTP/1.1\r\nHost: 127.0.0.1:8411\r\nSec-Fetch-Site: {site}\r\n\r\n"
            ))
        };
        assert_eq!(s.handle(&framed("same-site")).status, 403);
        assert_eq!(s.handle(&framed("cross-site")).status, 403);
        assert_eq!(s.handle(&framed("same-origin")).status, 200);
        assert_eq!(s.handle(&framed("none")).status, 200);
    }

    #[test]
    fn serves_only_ui_files() {
        let s = server(&ui());
        let get = |p: &str| {
            s.handle(&req(&format!(
                "GET {p} HTTP/1.1\r\nHost: localhost:8411\r\n\r\n"
            )))
        };
        let r = get("/");
        assert_eq!((r.status, r.headers[0].1.as_str()), (302, "/live/"));
        let r = get("/live/");
        assert_eq!(
            (r.status, r.content_type),
            (200, "text/html; charset=utf-8")
        );
        assert!(
            r.headers
                .iter()
                .any(|(k, _)| *k == "Content-Security-Policy")
        );
        assert_eq!(
            get("/live/app.js").content_type,
            "text/javascript; charset=utf-8"
        );
        assert_eq!(get("/live/../secret.txt").status, 404);
        assert_eq!(get("/live/%2e%2e/secret.txt").status, 404);
        assert_eq!(get("/live/.hidden").status, 404);
        assert_eq!(get("/nothing.html").status, 404);
        let post = req("POST /live/ HTTP/1.1\r\nHost: localhost:8411\r\n\r\n");
        assert_eq!(s.handle(&post).status, 405);
    }
}
