use sha1::{Digest, Sha1};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// 12 hex chars of SHA-1, the id scheme of the old Node servers (kept, so
/// resume positions and favourites saved by them stay valid).
pub fn id_for(s: &str) -> String {
    let mut h = Sha1::new();
    h.update(s.as_bytes());
    hex(&h.finalize()[..6])
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn sha1_hex(s: &str, chars: usize) -> String {
    let mut h = Sha1::new();
    h.update(s.as_bytes());
    let mut out = hex(&h.finalize());
    out.truncate(chars);
    out
}

/// Path with forward slashes (ids and URLs never depend on the OS).
pub fn slash(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

pub fn ext_lower(p: &Path) -> String {
    p.extension().map(|e| format!(".{}", e.to_string_lossy().to_lowercase())).unwrap_or_default()
}

pub fn read_at(f: &mut File, pos: u64, len: usize) -> Vec<u8> {
    let mut buf = vec![0u8; len];
    if f.seek(SeekFrom::Start(pos)).is_err() {
        return Vec::new();
    }
    let mut n = 0;
    while n < len {
        match f.read(&mut buf[n..]) {
            Ok(0) | Err(_) => break,
            Ok(k) => n += k,
        }
    }
    buf.truncate(n);
    buf
}

pub fn be32(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
pub fn be64(b: &[u8], o: usize) -> u64 {
    u64::from_be_bytes(b[o..o + 8].try_into().unwrap())
}

/// Collapses whitespace, drops NULs.
pub fn clean(s: &str) -> String {
    s.replace('\0', "").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// MP4/MOV/M4A duration from the "mvhd" box inside "moov" (which may sit at
/// the end of the file). Reads only a few KB.
pub fn mp4_duration(path: &Path) -> Option<u64> {
    let mut f = File::open(path).ok()?;
    let size = f.metadata().ok()?.len();
    let mut pos = 0u64;
    while pos + 8 <= size {
        let h = read_at(&mut f, pos, 16);
        if h.len() < 8 {
            return None;
        }
        let mut box_size = be32(&h, 0) as u64;
        let typ = &h[4..8];
        let mut hl = 8u64;
        if box_size == 1 {
            if h.len() < 16 {
                return None;
            }
            box_size = be64(&h, 8);
            hl = 16;
        } else if box_size == 0 {
            box_size = size - pos;
        }
        if box_size < hl {
            return None;
        }
        if typ == b"moov" {
            let len = (box_size - hl).min(4 * 1024 * 1024) as usize;
            let moov = read_at(&mut f, pos + hl, len);
            let at = find(&moov, b"mvhd")?;
            if at < 4 {
                return None;
            }
            let v = at + 4;
            if moov.len() < v + 32 {
                return None;
            }
            let (ts, dur) = if moov[v] == 1 {
                (be32(&moov, v + 20) as u64, be64(&moov, v + 24))
            } else {
                (be32(&moov, v + 12) as u64, be32(&moov, v + 16) as u64)
            };
            return if ts > 0 { Some(((dur as f64) / ts as f64).round() as u64) } else { None };
        }
        pos += box_size;
    }
    None
}

pub fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// .lrc/.txt as Windows editors write them: UTF-8 (BOM or not), UTF-16 with
/// BOM ("Unicode"), or ANSI (Windows-1252).
pub fn decode_text_file(buf: &[u8]) -> String {
    if buf.len() >= 2 && ((buf[0] == 0xff && buf[1] == 0xfe) || (buf[0] == 0xfe && buf[1] == 0xff)) {
        return decode_utf16(buf, true);
    }
    match std::str::from_utf8(buf) {
        Ok(s) => s.trim_start_matches('\u{feff}').to_string(),
        Err(_) => buf.iter().map(|&b| cp1252(b)).collect(),
    }
}

/// UTF-16 with optional BOM; `default_le` when there is none.
pub fn decode_utf16(buf: &[u8], default_le: bool) -> String {
    let mut b = buf;
    let mut le = default_le;
    if b.len() >= 2 && b[0] == 0xff && b[1] == 0xfe {
        le = true;
        b = &b[2..];
    } else if b.len() >= 2 && b[0] == 0xfe && b[1] == 0xff {
        le = false;
        b = &b[2..];
    }
    let units: Vec<u16> = b
        .chunks_exact(2)
        .map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) })
        .collect();
    String::from_utf16_lossy(&units)
}

pub fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

fn cp1252(b: u8) -> char {
    const HI: [u16; 32] = [
        0x20AC, 0x81, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x8D, 0x017D, 0x8F,
        0x90, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x9D, 0x017E, 0x0178,
    ];
    if (0x80..0xa0).contains(&b) {
        char::from_u32(HI[(b - 0x80) as usize] as u32).unwrap_or('?')
    } else {
        b as char
    }
}

pub fn mtime_ms(m: &std::fs::Metadata) -> f64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

pub fn image_mime(p: &Path) -> Option<&'static str> {
    match ext_lower(p).as_str() {
        ".jpg" | ".jpeg" => Some("image/jpeg"),
        ".png" => Some("image/png"),
        ".webp" => Some("image/webp"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ids_match_node() {
        // require('crypto').createHash('sha1').update('a/b.mp4').digest('hex').slice(0,12)
        assert_eq!(id_for("a/b.mp4"), sha1_hex("a/b.mp4", 12));
        assert_eq!(id_for("abc"), "a9993e364706");
    }
    #[test]
    fn text_decoding() {
        assert_eq!(decode_text_file(b"\xef\xbb\xbfhi"), "hi");
        assert_eq!(decode_text_file(b"\xff\xfeh\0i\0"), "hi");
        assert_eq!(decode_text_file(b"Gr\xfc\xdfe \x80"), "Grüße €");
        assert_eq!(clean("  a \0 b\n c "), "a b c");
    }
}
