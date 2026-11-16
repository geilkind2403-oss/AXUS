//! HTTP(S) for everything that talks to the internet or the PC server.
use std::io::{Read, Seek, SeekFrom};
use std::sync::OnceLock;
use std::time::Duration;

pub fn agent() -> &'static ureq::Agent {
    static A: OnceLock<ureq::Agent> = OnceLock::new();
    A.get_or_init(|| {
        ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(6))
            .timeout_read(Duration::from_secs(20))
            .user_agent("Mozilla/5.0 (X11; Linux armv7l) TV-Launcher/2.0")
            .build()
    })
}

pub fn get_bytes(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let r = agent().get(url).call().map_err(|e| e.to_string())?;
    let mut v = Vec::new();
    r.into_reader()
        .take(limit)
        .read_to_end(&mut v)
        .map_err(|e| e.to_string())?;
    Ok(v)
}

/// Downloads `url` in pieces of `chunk` bytes and hands each one on as soon
/// as it has arrived (a live segment starts playing before it is complete).
/// `on_chunk` returns false to stop. Ok(total bytes) or the first error;
/// pieces already handed on stay delivered.
pub fn get_chunks(
    url: &str,
    limit: u64,
    chunk: usize,
    mut on_chunk: impl FnMut(Vec<u8>) -> bool,
) -> Result<u64, String> {
    let r = agent().get(url).call().map_err(|e| e.to_string())?;
    let mut rd = r.into_reader().take(limit);
    let mut total = 0u64;
    loop {
        let mut buf = vec![0u8; chunk];
        let mut got = 0;
        // fill a piece (a read often returns less), but do not wait for
        // more once the stream pauses: hand on what there is
        while got < chunk {
            match rd.read(&mut buf[got..]) {
                Ok(0) => break,
                Ok(n) => {
                    got += n;
                    if got >= chunk / 4 {
                        break;
                    }
                }
                Err(e) => {
                    if got > 0 {
                        buf.truncate(got);
                        total += got as u64;
                        let _ = on_chunk(buf);
                    }
                    return Err(e.to_string());
                }
            }
        }
        if got == 0 {
            return Ok(total);
        }
        buf.truncate(got);
        total += got as u64;
        if !on_chunk(buf) {
            return Ok(total);
        }
    }
}

pub fn get_json(url: &str) -> Result<serde_json::Value, String> {
    let r = agent().get(url).call().map_err(|e| e.to_string())?;
    let mut s = String::new();
    r.into_reader()
        .take(32 << 20)
        .read_to_string(&mut s)
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&s).map_err(|e| e.to_string())
}

pub fn post_json(
    url: &str,
    body: &serde_json::Value,
    headers: &[(&str, &str)],
) -> Result<serde_json::Value, String> {
    let mut req = agent().post(url).set("Content-Type", "application/json");
    for (k, v) in headers {
        req = req.set(k, v);
    }
    let r = req
        .send_string(&body.to_string())
        .map_err(|e| e.to_string())?;
    let mut s = String::new();
    r.into_reader()
        .take(32 << 20)
        .read_to_string(&mut s)
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&s).map_err(|e| e.to_string())
}

pub fn url_encode(s: &str) -> String {
    s.bytes()
        .map(|c| {
            if c.is_ascii_alphanumeric() || b"-_.~".contains(&c) {
                (c as char).to_string()
            } else {
                format!("%{c:02X}")
            }
        })
        .collect()
}

/// A remote file read like a local one: Read + Seek over HTTP Range
/// requests, in chunks (YouTube's servers slow down long open-ended reads).
pub struct HttpFile {
    url: String,
    pos: u64,
    len: Option<u64>,
    chunk: u64,
    cur: Option<(u64, Box<dyn Read + Send>)>, // (end of the current range, body)
    headers: Vec<(String, String)>,
}

impl HttpFile {
    pub fn open(url: &str, chunk: u64, headers: Vec<(String, String)>) -> Result<HttpFile, String> {
        let mut f = HttpFile {
            url: url.to_string(),
            pos: 0,
            len: None,
            chunk,
            cur: None,
            headers,
        };
        f.fill()?;
        Ok(f)
    }
    fn fill(&mut self) -> Result<(), String> {
        let end = self.pos + self.chunk - 1;
        let mut req = agent()
            .get(&self.url)
            .set("Range", &format!("bytes={}-{}", self.pos, end));
        for (k, v) in &self.headers {
            req = req.set(k, v);
        }
        let r = req.call().map_err(|e| e.to_string())?;
        if let Some(cr) = r.header("Content-Range") {
            if let Some(total) = cr.rsplit('/').next().and_then(|t| t.parse().ok()) {
                self.len = Some(total);
            }
        } else if r.status() == 200 {
            // no range support: the whole body, from 0
            self.len = r.header("Content-Length").and_then(|l| l.parse().ok());
            if self.pos != 0 {
                return Err("Server kann nicht springen".into());
            }
            self.cur = Some((u64::MAX, Box::new(r.into_reader())));
            return Ok(());
        }
        self.cur = Some((end + 1, Box::new(r.into_reader())));
        Ok(())
    }
}

impl Read for HttpFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            if let Some(len) = self.len {
                if self.pos >= len {
                    return Ok(0);
                }
            }
            if self.cur.as_ref().map(|c| self.pos >= c.0).unwrap_or(true) {
                self.cur = None;
                self.fill().map_err(std::io::Error::other)?;
            }
            let (_, r) = self.cur.as_mut().unwrap();
            let n = r.read(buf)?;
            if n == 0 {
                if self.len.is_none() {
                    return Ok(0);
                }
                self.cur = None; // range ended early: ask again from here
                continue;
            }
            self.pos += n as u64;
            return Ok(n);
        }
    }
}

impl Seek for HttpFile {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let new = match to {
            SeekFrom::Start(p) => p as i64,
            SeekFrom::Current(d) => self.pos as i64 + d,
            SeekFrom::End(d) => {
                self.len
                    .ok_or_else(|| std::io::Error::other("Länge unbekannt"))? as i64
                    + d
            }
        };
        if new < 0 {
            return Err(std::io::Error::other("seek < 0"));
        }
        let new = new as u64;
        if new != self.pos {
            // short forward skips: read through instead of a new request
            if new > self.pos
                && new - self.pos < 64 * 1024
                && self.cur.as_ref().map(|c| new < c.0).unwrap_or(false)
            {
                let mut skip = vec![0u8; (new - self.pos) as usize];
                self.read_exact(&mut skip)?;
            } else {
                self.pos = new;
                self.cur = None;
            }
        }
        Ok(self.pos)
    }
}
