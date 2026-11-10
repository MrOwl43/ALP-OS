// SPDX-License-Identifier: AGPL-3.0-or-later
//! Live UI 0.2.5 stage F, "Files" (feature `media_files`): read-only
//! browsing of external disks over the console API, on top of
//! `alp_live::media` (docs/plans/live-ui-0.2.5-stage-f.md; Security and
//! Network Expert, ux-0.2.4-security-review.md section 4, and the stage F
//! review notes).
//!
//! - The screen names a disk only by the opaque id from `state.media`, and a
//!   path only as a list of names the server listed. Each name is checked
//!   here (no `/`, no `.` or `..`, no NUL) and again by `media::components`;
//!   every lookup is `O_NOFOLLOW`, one name at a time.
//! - Every call checks the disk's role again (`media::mount_ro` resolves the
//!   id and re-runs the checks), because disks can be swapped between calls.
//! - Files are served only for the kinds the screen previews, by an
//!   allow-list of extensions, never by what the file says it is, with
//!   `nosniff`, `Content-Disposition: inline`, a sandbox CSP and no caching.
//!   Large files are served in byte ranges (video needs `Range`), never
//!   read whole into memory.
//! - Paths and file names are never logged: only the disk id and the kind of
//!   failure.

use std::io::{Read, Seek, SeekFrom};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use alp_live::fail::Fail;
use alp_live::media;
use serde_json::{Value, json};

use crate::console::{Request, Response};

/// Most bytes one answer carries. A larger file is read in ranges.
pub const CHUNK: u64 = 4 << 20;
/// A file asked for without `Range` is sent whole only up to this size
/// (photos); anything larger needs `Range` (video does that by itself).
pub const WHOLE_MAX: u64 = 32 << 20;
/// Text is shown only as a preview: the UI reads the first 256 KiB
/// (`TEXT_MAX` in ui/live/app.js), so a text answer never carries more,
/// with or without `Range`.
pub const TEXT_MAX: u64 = 256 << 10;
/// How long `state.media` reuses its last look (the list runs `blkid` on
/// every external partition; the screen polls the state every second).
const LIST_TTL: Duration = Duration::from_secs(5);

/// The content type for a file name, from its extension only, or `None`
/// for every kind of file the screen does not preview (svg and html never).
pub fn content_type(name: &str) -> Option<&'static str> {
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        // Text the screen previews as text (review N1): always plain text,
        // never what the file says it is (nosniff and the sandbox CSP).
        "txt" | "log" | "md" | "csv" | "json" | "xml" | "ini" | "cfg" | "conf" | "nfo" => {
            "text/plain; charset=utf-8"
        }
        _ => return None,
    })
}

fn bad_path() -> Fail {
    Fail::new("media.path", "That is not a file or folder on this disk.")
}

/// A path from the screen: a JSON list of names, each one a single name.
pub fn path_names(v: &Value) -> Result<Vec<String>, Fail> {
    let a = v.as_array().ok_or_else(bad_path)?;
    if a.len() > 64 {
        return Err(bad_path());
    }
    a.iter()
        .map(|n| {
            let n = n.as_str().ok_or_else(bad_path)?;
            if n.is_empty() || n == "." || n == ".." || n.contains(['/', '\0']) || n.len() > 255 {
                return Err(bad_path());
            }
            Ok(n.to_string())
        })
        .collect()
}

fn media_id(b: &Value) -> Result<String, Fail> {
    b.get("media")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| Fail::new("request.json", "\"media\" must be text."))
}

/// The last look of [`state`].
static LAST: Mutex<Option<(Instant, Value)>> = Mutex::new(None);

fn last() -> std::sync::MutexGuard<'static, Option<(Instant, Value)>> {
    LAST.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// `state.media`: the external partitions with their id and verdict
/// (`refused` is the reason a disk is not opened, shown as text only).
pub fn state() -> Value {
    let mut g = last();
    if let Some((at, v)) = g.as_ref()
        && at.elapsed() < LIST_TTL
    {
        return v.clone();
    }
    let v: Value = match media::list() {
        Ok(list) => list
            .iter()
            .map(|m| {
                json!({"id": m.id, "label": m.label, "fs_type": m.fs_type,
                       "size_bytes": m.size_bytes, "mounted": m.mounted, "refused": m.refused})
            })
            .collect::<Vec<_>>()
            .into(),
        Err(f) => {
            eprintln!("alpd: media list: {}", f.key);
            json!([])
        }
    };
    *g = Some((Instant::now(), v.clone()));
    v
}

/// The next `state` looks again (after open and eject, so "mounted" and
/// the ids are current).
fn forget_state() {
    *last() = None;
}

/// `POST media/open {media}`: mount the disk read-only.
pub fn open(b: &Value) -> Result<Response, Fail> {
    let id = media_id(b)?;
    media::mount_ro(&id)?;
    forget_state();
    Ok(Response::json(200, &json!({})))
}

/// `POST media/list {media, path, offset}`: one page of a folder.
pub fn list(b: &Value) -> Result<Response, Fail> {
    let id = media_id(b)?;
    let names = path_names(b.get("path").unwrap_or(&json!([])))?;
    let offset = b
        .get("offset")
        .map(|o| {
            o.as_u64()
                .ok_or_else(|| Fail::new("request.json", "\"offset\" must be a number."))
        })
        .transpose()?
        .unwrap_or(0);
    let m = media::opened(&id)?;
    let l = m.walk(
        &names.join("/"),
        usize::try_from(offset).unwrap_or(usize::MAX),
    )?;
    Ok(Response::json(
        200,
        &json!({
            "entries": l.entries.iter().map(|e| json!({"name": e.name, "dir": e.dir, "size_bytes": e.size_bytes})).collect::<Vec<_>>(),
            "truncated": l.truncated,
            "skipped": l.skipped,
        }),
    ))
}

/// `POST media/eject {media}`.
pub fn eject(b: &Value) -> Result<Response, Fail> {
    media::eject(&media_id(b)?)?;
    forget_state();
    Ok(Response::json(200, &json!({})))
}

/// `%XX` decoding of one query value (`encodeURIComponent`); `None` on a
/// broken escape or text that is not UTF-8.
pub fn url_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(h, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn query_param(q: &str, key: &str) -> Option<String> {
    q.split('&')
        .find_map(|kv| kv.split_once('=').filter(|(k, _)| *k == key))
        .and_then(|(_, v)| url_decode(v))
}

/// A single byte range of a file of `size` bytes: `(start, end)`, end
/// inclusive, at most `cap` bytes long. `Ok(None)`: no `Range` header.
/// `Err(())`: not satisfiable (416).
#[allow(clippy::result_unit_err)]
pub fn byte_range(header: Option<&str>, size: u64, cap: u64) -> Result<Option<(u64, u64)>, ()> {
    let Some(h) = header else { return Ok(None) };
    let spec = h.trim().strip_prefix("bytes=").ok_or(())?;
    if spec.contains(',') || size == 0 {
        return Err(());
    }
    let (a, z) = spec.split_once('-').ok_or(())?;
    let (start, end) = match (a.trim(), z.trim()) {
        ("", n) => {
            let n: u64 = n.parse().map_err(|_| ())?;
            if n == 0 {
                return Err(());
            }
            (size.saturating_sub(n), size - 1)
        }
        (s, "") => (s.parse().map_err(|_| ())?, size - 1),
        (s, e) => (
            s.parse().map_err(|_| ())?,
            e.parse::<u64>().map_err(|_| ())?.min(size - 1),
        ),
    };
    if start > end || start >= size {
        return Err(());
    }
    Ok(Some((start, end.min(start + cap.max(1) - 1))))
}

/// Headers of a file answer; the console adds `Cache-Control: no-store`
/// and `X-Content-Type-Options: nosniff` to every answer (console.rs).
fn file_headers(size: u64, range: Option<(u64, u64)>) -> Vec<(&'static str, String)> {
    let mut h = vec![
        ("Content-Disposition", "inline".to_string()),
        (
            "Content-Security-Policy",
            "sandbox; default-src 'none'".to_string(),
        ),
        ("Accept-Ranges", "bytes".to_string()),
    ];
    if let Some((s, e)) = range {
        h.push(("Content-Range", format!("bytes {s}-{e}/{size}")));
    }
    h
}

/// `Sec-Fetch-Dest`, when the browser sends it, is an image, a video or a
/// fetch: the file is never a page of its own (stage F review F3).
pub fn dest_ok(req: &Request) -> bool {
    req.header("sec-fetch-dest")
        .is_none_or(|d| matches!(d, "image" | "video" | "empty"))
}

/// `GET media/file?media=<id>&path=<JSON list>`: a file for the preview,
/// only from a disk that is open now (review F2).
pub fn file(req: &Request) -> Result<Response, Fail> {
    if !dest_ok(req) {
        return Err(Fail::new(
            "request.forbidden",
            "This request is only allowed from the ALPArchive screen.",
        ));
    }
    let id = query_param(&req.query, "media")
        .ok_or_else(|| Fail::new("request.json", "\"media\" is missing."))?;
    let path: Value = query_param(&req.query, "path")
        .and_then(|p| serde_json::from_str(&p).ok())
        .ok_or_else(bad_path)?;
    let names = path_names(&path)?;
    let last = names.last().ok_or_else(bad_path)?;
    let ct = content_type(last)
        .ok_or_else(|| Fail::new("media.type", "This kind of file cannot be opened here."))?;
    let m = media::opened(&id)?;
    let mut f = m.open(&names.join("/"))?;
    let unreadable = |e: std::io::Error| {
        eprintln!("alpd: media file on {id}: {}", e.kind());
        Fail::new("media.unreadable", "This file could not be read.")
    };
    let size = f.metadata().map_err(unreadable)?.len();
    let (cap, whole) = if ct.starts_with("text/") {
        (TEXT_MAX, TEXT_MAX)
    } else {
        (CHUNK, WHOLE_MAX)
    };
    let range = match byte_range(req.header("range"), size, cap) {
        Ok(r) => r,
        Err(()) => {
            let mut r =
                Response::error(416, "media.range", "That part of the file does not exist.");
            r.headers.push(("Content-Range", format!("bytes */{size}")));
            return Ok(r);
        }
    };
    let (start, len) = match range {
        Some((s, e)) => (s, e - s + 1),
        None if size <= whole => (0, size),
        None => {
            return Err(Fail::new(
                "media.too_large",
                "This file is too large to show here.",
            ));
        }
    };
    f.seek(SeekFrom::Start(start)).map_err(unreadable)?;
    let mut body = Vec::with_capacity(usize::try_from(len).unwrap_or(0));
    f.take(len).read_to_end(&mut body).map_err(unreadable)?;
    Ok(Response {
        status: if range.is_some() { 206 } else { 200 },
        content_type: ct,
        headers: file_headers(size, range),
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_previewed_kinds_by_extension() {
        assert_eq!(content_type("IMG_1001.JPG"), Some("image/jpeg"));
        assert_eq!(content_type("clip.MOV"), Some("video/quicktime"));
        assert_eq!(
            content_type("readme.txt"),
            Some("text/plain; charset=utf-8")
        );
        for n in [
            "a.svg",
            "a.html",
            "a.htm",
            "a.exe",
            "a.pdf",
            "noext",
            "a.txt.html",
            ".jpg.",
        ] {
            assert_eq!(content_type(n), None, "{n}");
        }
    }

    #[test]
    fn path_names_are_single_names() {
        assert_eq!(
            path_names(&json!(["DCIM", "100CANON"])).unwrap(),
            vec!["DCIM", "100CANON"]
        );
        assert_eq!(path_names(&json!([])).unwrap(), Vec::<String>::new());
        for bad in [
            json!([".."]),
            json!(["."]),
            json!([""]),
            json!(["a/b"]),
            json!(["a\u{0}"]),
            json!("DCIM"),
            json!([1]),
            json!(vec!["a"; 65]),
        ] {
            assert!(path_names(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn files_are_never_a_page_of_their_own() {
        let req = |d: Option<&str>| Request {
            method: "GET".into(),
            path: "/api/v1/live/media/file".into(),
            query: String::new(),
            headers: d
                .map(|d| vec![("sec-fetch-dest".to_string(), d.to_string())])
                .unwrap_or_default(),
            body: vec![],
        };
        for ok in [None, Some("image"), Some("video"), Some("empty")] {
            assert!(dest_ok(&req(ok)), "{ok:?}");
        }
        for no in ["document", "iframe", "frame", "object", "embed", "script"] {
            assert!(!dest_ok(&req(Some(no))), "{no}");
        }
    }

    #[test]
    fn query_values_decode_strictly() {
        assert_eq!(
            url_decode("%5B%22DCIM%22%5D").as_deref(),
            Some("[\"DCIM\"]")
        );
        assert_eq!(url_decode("%zz"), None);
        assert_eq!(url_decode("%4"), None);
        assert_eq!(url_decode("%ff"), None, "not UTF-8");
        assert_eq!(
            query_param("media=ab12&path=%5B%22a%22%5D", "path").as_deref(),
            Some("[\"a\"]")
        );
        assert_eq!(query_param("xmedia=1", "media"), None);
    }

    #[test]
    fn ranges_are_single_bounded_and_checked() {
        let s = 10 * CHUNK;
        assert_eq!(byte_range(None, s, CHUNK), Ok(None));
        assert_eq!(byte_range(Some("bytes=0-99"), s, CHUNK), Ok(Some((0, 99))));
        assert_eq!(
            byte_range(Some("bytes=0-"), s, CHUNK),
            Ok(Some((0, CHUNK - 1))),
            "capped"
        );
        assert_eq!(
            byte_range(Some("bytes=-100"), s, CHUNK),
            Ok(Some((s - 100, s - 1)))
        );
        assert_eq!(
            byte_range(Some("bytes=5-1000000000000"), 50, CHUNK),
            Ok(Some((5, 49)))
        );
        for bad in [
            "bytes=0-1,5-6",
            "bytes=60-",
            "bytes=9-3",
            "items=0-1",
            "bytes=-0",
            "bytes=x-",
        ] {
            assert_eq!(byte_range(Some(bad), 50, CHUNK), Err(()), "{bad}");
        }
        assert_eq!(byte_range(Some("bytes=0-"), 0, CHUNK), Err(()));
        // Text: the preview cap, also for a request that asks for more.
        assert_eq!(
            byte_range(Some("bytes=0-"), s, TEXT_MAX),
            Ok(Some((0, TEXT_MAX - 1)))
        );
        assert_eq!(
            byte_range(Some("bytes=0-262143"), 20 << 30, TEXT_MAX),
            Ok(Some((0, 262_143))),
            "the UI's own request"
        );
    }

    #[test]
    fn file_answers_carry_the_safety_headers() {
        let h = file_headers(100, Some((0, 9)));
        let get = |k: &str| h.iter().find(|(n, _)| *n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("Content-Disposition"), Some("inline"));
        assert_eq!(
            get("Content-Security-Policy"),
            Some("sandbox; default-src 'none'")
        );
        assert_eq!(get("Content-Range"), Some("bytes 0-9/100"));
    }
}
