// SPDX-License-Identifier: AGPL-3.0-or-later
//! Minimal HTTP/1.1 client for loopback services (Immich on 127.0.0.1).
//! One request per connection (`Connection: close`); never used for the
//! network outside this computer.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Response {
    pub fn json(&self) -> Option<serde_json::Value> {
        serde_json::from_slice(&self.body).ok()
    }
}

/// Send one request to `addr` (`127.0.0.1:port`). `headers` are extra
/// header lines without CRLF.
pub fn request(
    addr: &str,
    method: &str,
    path: &str,
    headers: &[String],
    body: Option<&[u8]>,
) -> std::io::Result<Response> {
    let mut s = TcpStream::connect_timeout(
        &addr
            .parse()
            .map_err(|e| std::io::Error::other(format!("{addr}: {e}")))?,
        Duration::from_secs(3),
    )?;
    s.set_read_timeout(Some(Duration::from_secs(30)))?;
    s.set_write_timeout(Some(Duration::from_secs(30)))?;
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nAccept: application/json\r\n"
    );
    for h in headers {
        req.push_str(h);
        req.push_str("\r\n");
    }
    if let Some(b) = body {
        req.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            b.len()
        ));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes())?;
    if let Some(b) = body {
        s.write_all(b)?;
    }
    let mut raw = Vec::new();
    s.take(16 << 20).read_to_end(&mut raw)?;
    parse(&raw).ok_or_else(|| std::io::Error::other("malformed HTTP response"))
}

/// Parse a complete HTTP/1.1 response (Content-Length, chunked or EOF).
pub fn parse(raw: &[u8]) -> Option<Response> {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&raw[..end]).ok()?;
    let mut lines = head.split("\r\n");
    let status = lines.next()?.split_whitespace().nth(1)?.parse().ok()?;
    let mut chunked = false;
    let mut len = None;
    for l in lines {
        let (k, v) = l.split_once(':')?;
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
        if k == "transfer-encoding" && v.eq_ignore_ascii_case("chunked") {
            chunked = true;
        } else if k == "content-length" {
            len = v.parse::<usize>().ok();
        }
    }
    let rest = &raw[end + 4..];
    let body = if chunked {
        dechunk(rest)?
    } else if let Some(n) = len {
        rest.get(..n)?.to_vec()
    } else {
        rest.to_vec()
    };
    Some(Response { status, body })
}

fn dechunk(mut b: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let eol = b.windows(2).position(|w| w == b"\r\n")?;
        let size_txt = std::str::from_utf8(&b[..eol]).ok()?;
        let size = usize::from_str_radix(size_txt.split(';').next()?.trim(), 16).ok()?;
        b = &b[eol + 2..];
        if size == 0 {
            return Some(out);
        }
        out.extend_from_slice(b.get(..size)?);
        b = b.get(size + 2..)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_length_and_chunked() {
        let r = parse(b"HTTP/1.1 201 Created\r\nContent-Length: 2\r\n\r\n{}junk").unwrap();
        assert_eq!((r.status, r.body.as_slice()), (201, &b"{}"[..]));
        let r = parse(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\n{\"a\r\n4\r\n\":1}\r\n0\r\n\r\n").unwrap();
        assert_eq!(r.json().unwrap()["a"], 1);
        assert!(parse(b"garbage").is_none());
    }
}
