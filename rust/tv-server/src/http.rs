//! Small HTTP helpers on top of tiny_http: JSON/text replies, files with
//! Range (seeking anywhere without preloading), CORS for the LAN.
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use tiny_http::{Header, Request, Response, StatusCode};

pub fn header(k: &str, v: &str) -> Header {
    Header::from_bytes(k.as_bytes(), v.as_bytes()).unwrap()
}

pub fn cors(mut r: Response<Box<dyn Read + Send>>) -> Response<Box<dyn Read + Send>> {
    r.add_header(header("Access-Control-Allow-Origin", "*"));
    r.add_header(header("Access-Control-Allow-Headers", "Range, Content-Type, X-Api-Key"));
    r.add_header(header("Access-Control-Expose-Headers", "Content-Range, Content-Length, Accept-Ranges"));
    r
}

pub type Resp = Response<Box<dyn Read + Send>>;

pub fn bytes(status: u16, ctype: &str, body: Vec<u8>) -> Resp {
    let len = body.len();
    Response::new(StatusCode(status), vec![header("Content-Type", ctype)], Box::new(std::io::Cursor::new(body)) as Box<dyn Read + Send>, Some(len), None)
}

pub fn json<T: serde::Serialize>(status: u16, v: &T) -> Resp {
    bytes(status, "application/json; charset=utf-8", serde_json::to_vec(v).unwrap_or_default())
}

pub fn text(status: u16, s: &str) -> Resp {
    bytes(status, "text/plain; charset=utf-8", s.as_bytes().to_vec())
}

pub fn stream(ctype: &str, r: Box<dyn Read + Send>) -> Resp {
    Response::new(StatusCode(200), vec![header("Content-Type", ctype), header("Cache-Control", "no-store"), header("Accept-Ranges", "none")], r, None, None)
}

pub fn range_header(req: &Request) -> Option<String> {
    req.headers().iter().find(|h| h.field.equiv("Range")).map(|h| h.value.as_str().to_string())
}

pub fn get_header(req: &Request, name: &str) -> Option<String> {
    req.headers().iter().find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name)).map(|h| h.value.as_str().to_string())
}

/// Parses "bytes=a-b" / "bytes=a-" / "bytes=-n" against `total`.
pub fn parse_range(h: &str, total: u64) -> Option<Result<(u64, u64), ()>> {
    let spec = h.trim().strip_prefix("bytes=")?;
    let (a, b) = spec.split_once('-')?;
    if a.is_empty() && b.is_empty() {
        return None;
    }
    let (a, b) = (a.trim(), b.trim());
    if !a.chars().all(|c| c.is_ascii_digit()) || !b.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let mut start = if a.is_empty() { total.saturating_sub(b.parse().ok()?) } else { a.parse().ok()? };
    let mut end = if !a.is_empty() && !b.is_empty() { b.parse().ok()? } else { total.saturating_sub(1) };
    start = start.min(u64::MAX);
    end = end.min(total.saturating_sub(1));
    if total == 0 || start > end {
        return Some(Err(()));
    }
    Some(Ok((start, end)))
}

/// A file (or a byte slice of one: embedded covers) with Range support.
pub fn file(req: &Request, path: &Path, ctype: &str, slice: Option<(u64, u64)>, cache: &str) -> Resp {
    let Ok(mut f) = File::open(path) else { return text(404, "not found") };
    let (from, total) = match slice {
        Some((pos, len)) => (pos, len),
        None => (0, f.metadata().map(|m| m.len()).unwrap_or(0)),
    };
    let common = vec![header("Content-Type", ctype), header("Accept-Ranges", "bytes"), header("Cache-Control", cache)];
    let (status, start, len, extra) = match range_header(req).and_then(|h| parse_range(&h, total)) {
        Some(Err(())) => {
            let mut r = text(416, "");
            r.add_header(header("Content-Range", &format!("bytes */{total}")));
            return r;
        }
        Some(Ok((s, e))) => (206, s, e - s + 1, Some(header("Content-Range", &format!("bytes {s}-{e}/{total}")))),
        None => (200, 0, total, None),
    };
    if f.seek(SeekFrom::Start(from + start)).is_err() {
        return text(500, "seek");
    }
    let mut headers = common;
    headers.extend(extra);
    Response::new(StatusCode(status), headers, Box::new(f.take(len)) as Box<dyn Read + Send>, Some(len as usize), None)
}

/// Splits "/a/b?x=1&y=2" into path parts and a query lookup.
pub struct Url {
    pub path: String,
    pub parts: Vec<String>,
    pub query: Vec<(String, String)>,
}

impl Url {
    pub fn parse(raw: &str) -> Url {
        let (p, q) = raw.split_once('?').unwrap_or((raw, ""));
        let query = q
            .split('&')
            .filter(|s| !s.is_empty())
            .map(|kv| {
                let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
                (percent_decode(k), percent_decode(v))
            })
            .collect();
        let path = percent_decode(p);
        let parts = path.split('/').filter(|s| !s.is_empty()).map(String::from).collect();
        Url { path, parts, query }
    }
    pub fn q(&self, k: &str) -> Option<&str> {
        self.query.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str())
    }
    pub fn num(&self, k: &str) -> f64 {
        self.q(k).and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite()).unwrap_or(0.0).max(0.0)
    }
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|c| if c.is_ascii_alphanumeric() || b"-_.~".contains(&c) { (c as char).to_string() } else { format!("%{c:02X}") })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-99", 1000), Some(Ok((0, 99))));
        assert_eq!(parse_range("bytes=900-", 1000), Some(Ok((900, 999))));
        assert_eq!(parse_range("bytes=-100", 1000), Some(Ok((900, 999))));
        assert_eq!(parse_range("bytes=2000-", 1000), Some(Err(())));
        assert_eq!(parse_range("bytes=-", 1000), None);
        let u = Url::parse("/play/abc?t=12.5&v=h264&key=a%20b");
        assert_eq!(u.parts, vec!["play", "abc"]);
        assert_eq!(u.num("t"), 12.5);
        assert_eq!(u.q("key"), Some("a b"));
    }
}
