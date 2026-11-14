//! The music library: what a ripper / Windows Media Player writes.
//!   <root>/<Interpret>/artist.jpg|png                 (optional)
//!         /<Interpret>/<Interpret> - <Album>/Folder.jpg  /AlbumArtSmall.jpg
//!                                           /01 - <Interpret> - <Titel>.mp3 (+ .lrc)
//! Also fine: root being one artist folder, albums directly in root, "CD1" /
//! "Disc 2" subfolders, flac/m4a/ogg/opus/wav. ID3 tags win over file names
//! where they have better data; covers embedded in the MP3 are used when the
//! folder has no Folder.jpg.
use crate::util::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::path::{Path, PathBuf};

pub fn audio_mime(p: &Path) -> Option<&'static str> {
    Some(match ext_lower(p).as_str() {
        ".mp3" => "audio/mpeg",
        ".m4a" => "audio/mp4",
        ".aac" => "audio/aac",
        ".flac" => "audio/flac",
        ".ogg" | ".oga" | ".opus" => "audio/ogg",
        ".wav" => "audio/wav",
        ".webm" => "audio/webm",
        _ => return None,
    })
}

const COVER_NAMES: [&str; 4] = ["folder", "cover", "front", "album"];
const COVER_SMALL_NAMES: [&str; 3] = ["albumartsmall", "folder_small", "cover_small"];

/// A byte range inside a file (embedded cover, USLT lyrics frame).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Blob {
    pub pos: u64,
    pub len: u64,
    #[serde(default)]
    pub mime: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Tags {
    pub title: Option<String>,
    pub artist: Option<String>,
    #[serde(rename = "albumArtist")]
    pub album_artist: Option<String>,
    pub album: Option<String>,
    pub track: Option<String>,
    pub disc: Option<String>,
    pub year: Option<String>,
    pub genre: Option<String>,
    pub picture: Option<Blob>,
    pub lyrics: Option<Blob>,
    #[serde(rename = "tagEnd", default)]
    pub tag_end: u64,
    pub duration: Option<u64>,
}

// ------------------------------------------------------------ ID3v2
fn syncsafe(b: &[u8], o: usize) -> u32 {
    ((b[o] as u32 & 0x7f) << 21) | ((b[o + 1] as u32 & 0x7f) << 14) | ((b[o + 2] as u32 & 0x7f) << 7) | (b[o + 3] as u32 & 0x7f)
}

pub fn decode_text(buf: &[u8], enc: u8) -> String {
    match enc {
        0 => latin1(buf),
        3 => String::from_utf8_lossy(buf).into_owned(),
        1 => decode_utf16(buf, true),
        2 => decode_utf16(buf, false),
        _ => latin1(buf),
    }
}

fn term_index(buf: &[u8], from: usize, enc: u8) -> usize {
    if enc == 1 || enc == 2 {
        let mut i = from;
        while i + 1 < buf.len() {
            if buf[i] == 0 && buf[i + 1] == 0 {
                return i;
            }
            i += 2;
        }
        return buf.len();
    }
    buf.iter().skip(from).position(|&b| b == 0).map(|p| p + from).unwrap_or(buf.len())
}

fn text_frame(d: &[u8]) -> String {
    if d.is_empty() {
        return String::new();
    }
    clean(decode_text(&d[1..], d[0]).split('\0').next().unwrap_or(""))
}

/// Reads frame headers and the small text frames only; pictures and lyrics
/// are remembered by position and read when someone asks for them.
pub fn read_id3(f: &mut File, size: u64) -> Tags {
    let mut out = Tags::default();
    if size < 10 {
        return out;
    }
    let head = read_at(f, 0, 10);
    if head.len() < 10 || &head[0..3] != b"ID3" {
        return out;
    }
    let ver = head[3];
    let flags = head[5];
    let tag_size = syncsafe(&head, 6) as u64;
    out.tag_end = 10 + tag_size + if flags & 0x10 != 0 { 10 } else { 0 };
    if !(2..=4).contains(&ver) {
        return out;
    }
    let mut pos = 10u64;
    if ver >= 3 && flags & 0x40 != 0 {
        let ext = read_at(f, pos, 4);
        if ext.len() < 4 {
            return out;
        }
        pos += if ver == 4 { syncsafe(&ext, 0) as u64 } else { be32(&ext, 0) as u64 + 4 };
    }
    let end = (10 + tag_size).min(size);
    let id_len = if ver == 2 { 3 } else { 4 };
    let hdr_len = if ver == 2 { 6 } else { 10 };
    while pos + hdr_len as u64 <= end {
        let h = read_at(f, pos, hdr_len);
        if h.len() < hdr_len || h[0] == 0 {
            break;
        }
        let id = latin1(&h[..id_len]);
        let len = if ver == 2 {
            ((h[3] as u64) << 16) | ((h[4] as u64) << 8) | h[5] as u64
        } else if ver == 4 {
            syncsafe(&h, 4) as u64
        } else {
            be32(&h, 4) as u64
        };
        if !id.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) || len == 0 || pos + hdr_len as u64 + len > end {
            break;
        }
        let data_pos = pos + hdr_len as u64;
        let slot: Option<&mut Option<String>> = match id.as_str() {
            "TIT2" | "TT2" => Some(&mut out.title),
            "TPE1" | "TP1" => Some(&mut out.artist),
            "TPE2" | "TP2" => Some(&mut out.album_artist),
            "TALB" | "TAL" => Some(&mut out.album),
            "TRCK" | "TRK" => Some(&mut out.track),
            "TPOS" | "TPA" => Some(&mut out.disc),
            "TYER" | "TYE" | "TDRC" => Some(&mut out.year),
            "TCON" | "TCO" => Some(&mut out.genre),
            _ => None,
        };
        if let Some(slot) = slot {
            if len < 4096 && slot.is_none() {
                let v = text_frame(&read_at(f, data_pos, len as usize));
                if !v.is_empty() {
                    *slot = Some(v);
                }
            }
        } else if (id == "APIC" || id == "PIC") && out.picture.is_none() {
            let d = read_at(f, data_pos, len.min(512) as usize);
            if d.len() > 5 {
                let enc = d[0];
                let (mime, mut i) = if id == "PIC" {
                    (if latin1(&d[1..4]).to_lowercase().contains("png") { "image/png".to_string() } else { "image/jpeg".to_string() }, 5)
                } else {
                    let m = d.iter().skip(1).position(|&b| b == 0).map(|p| p + 1).unwrap_or(d.len());
                    let mime = latin1(&d[1..m]);
                    (if mime.is_empty() { "image/jpeg".into() } else { mime }, m + 2)
                };
                i = term_index(&d, i.min(d.len()), enc) + if enc == 1 || enc == 2 { 2 } else { 1 };
                if i < d.len() {
                    let mime = if mime.contains('/') { mime } else { format!("image/{}", mime.to_lowercase()) };
                    out.picture = Some(Blob { pos: data_pos + i as u64, len: len - i as u64, mime });
                }
            }
        } else if id == "USLT" && out.lyrics.is_none() {
            out.lyrics = Some(Blob { pos: data_pos, len, mime: String::new() });
        }
        pos = data_pos + len;
    }
    out
}

pub fn read_uslt(file: &Path, at: &Blob) -> String {
    let Ok(mut f) = File::open(file) else { return String::new() };
    let d = read_at(&mut f, at.pos, at.len.min(256 * 1024) as usize);
    if d.len() < 5 {
        return String::new();
    }
    let enc = d[0];
    let start = term_index(&d, 4, enc) + if enc == 1 || enc == 2 { 2 } else { 1 };
    if start >= d.len() {
        return String::new();
    }
    decode_text(&d[start..], enc).replace('\0', "").trim().to_string()
}

// ------------------------------------------------------------ durations
const BR_V1: [[u32; 15]; 3] = [
    [0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448],
    [0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384],
    [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320],
];
const BR_V2: [[u32; 15]; 3] = [
    [0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256],
    [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],
    [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],
];

/// MP3: Xing/Info or VBRI header when present (VBR), else size / bitrate (CBR).
pub fn mp3_duration(f: &mut File, size: u64, audio_start: u64) -> Option<u64> {
    let buf = read_at(f, audio_start, (size.saturating_sub(audio_start)).min(64 * 1024) as usize);
    let mut i = 0;
    while i + 4 < buf.len() {
        if buf[i] != 0xff || buf[i + 1] & 0xe0 != 0xe0 {
            i += 1;
            continue;
        }
        let v_bits = (buf[i + 1] >> 3) & 3;
        let l_bits = (buf[i + 1] >> 1) & 3;
        let br_idx = (buf[i + 2] >> 4) as usize;
        let sr_idx = ((buf[i + 2] >> 2) & 3) as usize;
        if v_bits == 1 || l_bits == 0 || br_idx == 0 || br_idx == 15 || sr_idx == 3 {
            i += 1;
            continue;
        }
        let version = match v_bits { 3 => 1, 2 => 2, _ => 25 };
        let layer = 4 - l_bits as usize;
        let sample_rate = match version { 1 => [44100, 48000, 32000], 2 => [22050, 24000, 16000], _ => [11025, 12000, 8000] }[sr_idx] as u64;
        let bitrate = if version == 1 { BR_V1 } else { BR_V2 }[layer - 1][br_idx] as u64 * 1000;
        let mono = buf[i + 3] >> 6 == 3;
        let spf: u64 = if layer == 1 { 384 } else if layer == 2 || version == 1 { 1152 } else { 576 };
        let xing = i + 4 + if version == 1 { if mono { 17 } else { 32 } } else if mono { 9 } else { 17 };
        if xing + 12 <= buf.len() {
            let name = &buf[xing..xing + 4];
            if (name == b"Xing" || name == b"Info") && be32(&buf, xing + 4) & 1 != 0 {
                return Some(((be32(&buf, xing + 8) as u64 * spf) as f64 / sample_rate as f64).round() as u64);
            }
        }
        if i + 54 <= buf.len() && &buf[i + 36..i + 40] == b"VBRI" {
            return Some(((be32(&buf, i + 50) as u64 * spf) as f64 / sample_rate as f64).round() as u64);
        }
        let mut bytes = size - (audio_start + i as u64);
        if size > 128 && read_at(f, size - 128, 3) == b"TAG" {
            bytes -= 128;
        }
        return if bitrate > 0 { Some(((bytes * 8) as f64 / bitrate as f64).round() as u64) } else { None };
    }
    None
}

pub fn flac_duration(f: &mut File) -> Option<u64> {
    let b = read_at(f, 0, 42);
    if b.len() < 26 || &b[0..4] != b"fLaC" {
        return None;
    }
    let s = 8;
    let rate = ((b[s + 10] as u64) << 12) | ((b[s + 11] as u64) << 4) | (b[s + 12] as u64 >> 4);
    let total = ((b[s + 13] & 0x0f) as u64) * (1u64 << 32) + be32(&b, s + 14) as u64;
    if rate > 0 { Some((total as f64 / rate as f64).round() as u64) } else { None }
}

pub fn probe(file: &Path, size: u64) -> Tags {
    let Ok(mut f) = File::open(file) else { return Tags::default() };
    let mut tags = read_id3(&mut f, size);
    tags.duration = match ext_lower(file).as_str() {
        ".mp3" => mp3_duration(&mut f, size, tags.tag_end),
        ".flac" => flac_duration(&mut f),
        ".m4a" => mp4_duration(file),
        _ => None,
    };
    tags
}

// ------------------------------------------------------------ names
#[derive(Debug, PartialEq)]
pub struct TrackName {
    pub number: Option<u32>,
    pub artist: Option<String>,
    pub title: String,
    pub disc: Option<u32>,
}

/// "01 - Madison Beer - Good In Goodbye" -> { 1, artist, title }.
/// Also "01. Title", "1-03 Title" (disc-track), "Title".
pub fn parse_track_name(base: &str) -> TrackName {
    let mut base = base.to_string();
    let mut disc = None;
    // disc-track: "1-03 Title"
    if let Some((d, rest)) = split_digits(&base, 1, 2) {
        if let Some(rest) = rest.strip_prefix('-') {
            if let Some((t, rest2)) = split_digits(rest, 1, 3) {
                let rest2t = rest2.trim_start_matches([' ', '.', '_', '-']);
                if rest2t.len() < rest2.len() && !rest2t.is_empty() {
                    disc = Some(d);
                    base = format!("{t} - {rest2t}");
                }
            }
        }
    }
    let Some((num, rest)) = split_digits(&base, 1, 3) else {
        return TrackName { number: None, artist: None, title: clean(&base), disc };
    };
    let mut rest = rest.trim_start_matches([' ', '.', ')', '_']);
    if let Some(r) = rest.strip_prefix('-') {
        rest = r.trim_start();
    }
    if rest.is_empty() {
        return TrackName { number: None, artist: None, title: clean(&base), disc };
    }
    let parts = split_dash(rest);
    if parts.len() >= 2 {
        return TrackName { number: Some(num), artist: Some(clean(&parts[0])), title: clean(&parts[1..].join(" - ")), disc };
    }
    TrackName { number: Some(num), artist: None, title: clean(rest), disc }
}

fn split_digits(s: &str, min: usize, max: usize) -> Option<(u32, &str)> {
    let n = s.chars().take_while(|c| c.is_ascii_digit()).count();
    if n < min || n > max {
        return None;
    }
    Some((s[..n].parse().ok()?, &s[n..]))
}

/// Splits on " - " with any whitespace around the dash.
fn split_dash(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '-' && i > 0 && chars[i - 1].is_whitespace() && i + 1 < chars.len() && chars[i + 1].is_whitespace() {
            out.push(cur.trim_end().to_string());
            cur = String::new();
            i += 1;
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            continue;
        }
        cur.push(chars[i]);
        i += 1;
    }
    out.push(cur);
    out
}

/// "Madison Beer - Life Support" -> (artist, album)
pub fn parse_album_name(name: &str) -> (Option<String>, String) {
    let parts = split_dash(name);
    if parts.len() >= 2 {
        (Some(clean(&parts[0])), clean(&parts[1..].join(" - ")))
    } else {
        (None, clean(name))
    }
}

fn first_num(s: &Option<String>) -> Option<u32> {
    let s = s.as_deref()?.trim();
    let n: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    n.parse().ok()
}

fn clean_genre(g: &Option<String>) -> Option<String> {
    let g = g.as_deref()?;
    let g = if g.starts_with('(') { g.split_once(')').map(|x| x.1).unwrap_or(g) } else { g };
    Some(clean(g)).filter(|s| !s.is_empty())
}

fn disc_dir(name: &str) -> Option<u32> {
    let l = name.to_lowercase();
    for p in ["cd", "disc", "disk", "seite", "side"] {
        if let Some(rest) = l.strip_prefix(p) {
            let rest = rest.trim_start_matches([' ', '_', '-']);
            if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
                return rest.parse().ok();
            }
        }
    }
    None
}

fn find_image(dir: &Path, names: &[&str], files: &[String]) -> Option<PathBuf> {
    for want in names {
        if let Some(hit) = files.iter().find(|n| {
            let p = Path::new(n);
            image_mime(p).is_some() && p.file_stem().map(|s| s.to_string_lossy().to_lowercase() == *want).unwrap_or(false)
        }) {
            return Some(dir.join(hit));
        }
    }
    None
}

fn list_names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir).map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    v.sort();
    v
}

// ------------------------------------------------------------ library
#[derive(Debug, Clone, Serialize)]
pub struct Artist {
    pub id: String,
    pub name: String,
    #[serde(skip)]
    pub image_file: Option<PathBuf>,
    #[serde(rename = "albumIds")]
    pub album_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Album {
    pub id: String,
    pub title: String,
    pub artist: String,
    #[serde(rename = "artistId")]
    pub artist_id: String,
    pub year: Option<u32>,
    pub genre: Option<String>,
    pub duration: Option<u64>,
    pub discs: usize,
    #[serde(rename = "trackIds")]
    pub track_ids: Vec<String>,
    #[serde(skip)]
    pub cover_file: Option<PathBuf>,
    #[serde(skip)]
    pub cover_small_file: Option<PathBuf>,
    /// Embedded cover: (file, blob) when the folder has no Folder.jpg.
    #[serde(skip)]
    pub picture: Option<(PathBuf, Blob)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    #[serde(rename = "albumId")]
    pub album_id: String,
    #[serde(rename = "artistId")]
    pub artist_id: String,
    pub number: Option<u32>,
    pub disc: u32,
    pub duration: Option<u64>,
    pub genre: Option<String>,
    pub year: Option<u32>,
    pub size: u64,
    #[serde(skip)]
    pub file: PathBuf,
    #[serde(skip)]
    pub mime: &'static str,
    #[serde(skip)]
    pub lyrics_file: Option<PathBuf>,
    #[serde(skip)]
    pub lyrics_tag: Option<Blob>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Library {
    pub artists: Vec<Artist>,
    pub albums: Vec<Album>,
    pub tracks: Vec<Track>,
}

impl Library {
    pub fn track(&self, id: &str) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }
    pub fn album(&self, id: &str) -> Option<&Album> {
        self.albums.iter().find(|a| a.id == id)
    }
    pub fn artist(&self, id: &str) -> Option<&Artist> {
        self.artists.iter().find(|a| a.id == id)
    }
}

#[derive(Serialize, Deserialize)]
struct CacheEntry {
    key: String,
    t: Tags,
}

struct RawTrack {
    file: PathBuf,
    rel: String,
    mime: &'static str,
    size: u64,
    title: String,
    artist: Option<String>,
    number: Option<u32>,
    disc: u32,
    duration: Option<u64>,
    tag_album: Option<String>,
    tag_album_artist: Option<String>,
    year: Option<u32>,
    genre: Option<String>,
    picture: Option<Blob>,
    lyrics_file: Option<PathBuf>,
    lyrics_tag: Option<Blob>,
}

struct RawAlbum {
    dir: PathBuf,
    rel: String,
    folder_artist: Option<String>,
    folder_album: String,
    parent_name: Option<String>,
    parent_dir: PathBuf,
    cover_file: Option<PathBuf>,
    cover_small_file: Option<PathBuf>,
    tracks: Vec<RawTrack>,
}

/// Tag/duration cache (per file + size + mtime), kept on disk so a restart
/// doesn't have to open every file again.
pub struct Scanner {
    root: PathBuf,
    cache_file: Option<PathBuf>,
    cache: HashMap<String, CacheEntry>,
    dirty: bool,
}

fn most_common<T: Clone + Eq + std::hash::Hash>(list: impl Iterator<Item = Option<T>>) -> Option<T> {
    let mut counts: Vec<(T, usize)> = Vec::new();
    for v in list.flatten() {
        if let Some(c) = counts.iter_mut().find(|c| c.0 == v) {
            c.1 += 1;
        } else {
            counts.push((v, 1));
        }
    }
    // stable: the first one seen wins a tie, like the JS sort
    let mut best: Option<(T, usize)> = None;
    for c in counts {
        if best.as_ref().map(|b| c.1 > b.1).unwrap_or(true) {
            best = Some(c);
        }
    }
    best.map(|b| b.0)
}

impl Scanner {
    pub fn new(root: impl Into<PathBuf>, cache_file: Option<PathBuf>) -> Self {
        let cache = cache_file
            .as_ref()
            .and_then(|f| fs::read(f).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Scanner { root: root.into(), cache_file, cache, dirty: false }
    }

    fn tags_for(&mut self, file: &Path, meta: &fs::Metadata) -> Tags {
        let key = format!("{}|{}", meta.len(), mtime_ms(meta).round());
        let k = file.to_string_lossy().into_owned();
        if let Some(c) = self.cache.get(&k) {
            if c.key == key {
                return c.t.clone();
            }
        }
        let t = probe(file, meta.len());
        self.cache.insert(k, CacheEntry { key, t: t.clone() });
        self.dirty = true;
        t
    }

    pub fn scan(&mut self) -> Library {
        let mut albums: Vec<RawAlbum> = Vec::new();
        let mut seen: HashSet<PathBuf> = HashSet::new();
        let root = self.root.clone();
        self.walk(&root, 0, &mut albums, &mut seen);

        if self.dirty {
            let all: HashSet<String> = albums.iter().flat_map(|a| a.tracks.iter().map(|t| t.file.to_string_lossy().into_owned())).collect();
            self.cache.retain(|k, _| all.contains(k));
            if let Some(cf) = &self.cache_file {
                if let Some(d) = cf.parent() {
                    fs::create_dir_all(d).ok();
                }
                fs::write(cf, serde_json::to_vec(&self.cache).unwrap_or_default()).ok();
            }
            self.dirty = false;
        }
        shape(&root, albums)
    }

    fn walk(&mut self, dir: &Path, depth: u32, albums: &mut Vec<RawAlbum>, seen: &mut HashSet<PathBuf>) {
        if depth > 6 {
            return;
        }
        let mut entries: Vec<fs::DirEntry> = match fs::read_dir(dir) {
            Ok(r) => r.flatten().collect(),
            Err(_) => return,
        };
        entries.sort_by_key(|e| e.file_name());
        let names: Vec<String> = entries.iter().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        let audio: Vec<&fs::DirEntry> = entries.iter().filter(|e| e.path().is_file() && audio_mime(&e.path()).is_some()).collect();
        if !audio.is_empty() {
            let disc_no = if dir != self.root { dir.file_name().and_then(|n| disc_dir(&n.to_string_lossy())) } else { None };
            let album_dir = if disc_no.is_some() { dir.parent().unwrap_or(dir).to_path_buf() } else { dir.to_path_buf() };
            let idx = match albums.iter().position(|a| a.dir == album_dir) {
                Some(i) => i,
                None => {
                    let album_files = if album_dir == dir { names.clone() } else { list_names(&album_dir) };
                    let (folder_artist, folder_album) = parse_album_name(&album_dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                    let parent_dir = if album_dir == self.root { self.root.clone() } else { album_dir.parent().unwrap_or(&album_dir).to_path_buf() };
                    let parent_name = if parent_dir != self.root || album_dir != self.root {
                        parent_dir.file_name().map(|n| n.to_string_lossy().into_owned())
                    } else {
                        None
                    };
                    albums.push(RawAlbum {
                        rel: { let r = slash(album_dir.strip_prefix(&self.root).unwrap_or(&album_dir)); if r.is_empty() { ".".into() } else { r } },
                        folder_artist,
                        folder_album,
                        parent_name,
                        cover_file: find_image(&album_dir, &COVER_NAMES, &album_files).or_else(|| if disc_no.is_some() { find_image(dir, &COVER_NAMES, &names) } else { None }),
                        cover_small_file: find_image(&album_dir, &COVER_SMALL_NAMES, &album_files),
                        parent_dir,
                        dir: album_dir,
                        tracks: Vec::new(),
                    });
                    albums.len() - 1
                }
            };
            for e in audio {
                let file = e.path();
                if !seen.insert(file.clone()) {
                    continue;
                }
                let Ok(meta) = fs::metadata(&file) else { continue };
                let t = self.tags_for(&file, &meta);
                let base = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let by_name = parse_track_name(&base);
                let bl = base.to_lowercase();
                let lrc = names.iter().find(|n| n.to_lowercase() == format!("{bl}.lrc")).or_else(|| names.iter().find(|n| n.to_lowercase() == format!("{bl}.txt")));
                albums[idx].tracks.push(RawTrack {
                    rel: slash(file.strip_prefix(&self.root).unwrap_or(&file)),
                    mime: audio_mime(&file).unwrap_or("application/octet-stream"),
                    size: meta.len(),
                    title: t.title.clone().unwrap_or(by_name.title),
                    artist: t.artist.clone().or(by_name.artist),
                    number: by_name.number.or_else(|| first_num(&t.track)),
                    disc: disc_no.or(by_name.disc).or_else(|| first_num(&t.disc)).unwrap_or(1),
                    duration: t.duration,
                    tag_album: t.album.clone(),
                    tag_album_artist: t.album_artist.clone(),
                    year: first_num(&t.year),
                    genre: clean_genre(&t.genre),
                    picture: t.picture.clone(),
                    lyrics_file: lrc.map(|n| dir.join(n)),
                    lyrics_tag: t.lyrics.clone(),
                    file,
                });
            }
        }
        for e in entries {
            let n = e.file_name().to_string_lossy().into_owned();
            if e.path().is_dir() && !n.starts_with('.') && !n.starts_with('$') {
                self.walk(&e.path(), depth + 1, albums, seen);
            }
        }
    }
}

fn shape(root: &Path, albums: Vec<RawAlbum>) -> Library {
    let mut artists: Vec<Artist> = Vec::new();
    let mut out_albums: Vec<Album> = Vec::new();
    let mut out_tracks: Vec<Track> = Vec::new();
    for mut a in albums {
        if a.tracks.is_empty() {
            continue;
        }
        let artist_name = a
            .folder_artist
            .clone()
            .or_else(|| most_common(a.tracks.iter().map(|t| t.tag_album_artist.clone())))
            .or_else(|| most_common(a.tracks.iter().map(|t| t.artist.clone())))
            .or_else(|| a.parent_name.clone())
            .unwrap_or_else(|| "Unbekannter Interpret".into());
        let title = if a.folder_artist.is_some() { Some(a.folder_album.clone()) } else { None }
            .or_else(|| most_common(a.tracks.iter().map(|t| t.tag_album.clone())))
            .unwrap_or_else(|| a.folder_album.clone());
        let artist_id = id_for(&format!("artist:{artist_name}").to_lowercase());
        let parent_files = list_names(&a.parent_dir);
        match artists.iter_mut().find(|x| x.id == artist_id) {
            None => {
                let names: Vec<&str> = ["artist", "interpret", "folder"].into_iter().filter(|n| a.parent_dir != a.dir || *n != "folder").collect();
                let img = find_image(&a.parent_dir, &names, &parent_files)
                    .or_else(|| if a.parent_dir != root { None } else { find_image(root, &["artist", "interpret"], &list_names(root)) });
                artists.push(Artist { id: artist_id.clone(), name: artist_name.clone(), image_file: img, album_ids: Vec::new() });
            }
            Some(x) if x.image_file.is_none() => x.image_file = find_image(&a.parent_dir, &["artist", "interpret"], &parent_files),
            _ => {}
        }
        let id = id_for(&format!("album:{}", a.rel).to_lowercase());
        a.tracks.sort_by(|x, y| {
            x.disc.cmp(&y.disc).then(x.number.unwrap_or(999).cmp(&y.number.unwrap_or(999))).then_with(|| x.title.to_lowercase().cmp(&y.title.to_lowercase()))
        });
        let year = most_common(a.tracks.iter().map(|t| t.year));
        let genre = most_common(a.tracks.iter().map(|t| t.genre.clone()));
        let picture = if a.cover_file.is_none() { a.tracks.iter().find_map(|t| t.picture.clone().map(|p| (t.file.clone(), p))) } else { None };
        let discs = a.tracks.iter().map(|t| t.disc).collect::<HashSet<_>>().len();
        let mut album = Album {
            id: id.clone(),
            title: title.clone(),
            artist: artist_name.clone(),
            artist_id: artist_id.clone(),
            year,
            genre: genre.clone(),
            duration: None,
            discs,
            track_ids: Vec::new(),
            cover_file: a.cover_file.clone(),
            cover_small_file: a.cover_small_file.clone(),
            picture,
        };
        let total: u64 = a.tracks.iter().map(|t| t.duration.unwrap_or(0)).sum();
        album.duration = if total > 0 { Some(total) } else { None };
        for t in a.tracks {
            let tid = id_for(&format!("track:{}", t.rel).to_lowercase());
            album.track_ids.push(tid.clone());
            out_tracks.push(Track {
                id: tid,
                title: t.title,
                artist: t.artist.unwrap_or_else(|| artist_name.clone()),
                album: title.clone(),
                album_id: id.clone(),
                artist_id: artist_id.clone(),
                number: t.number,
                disc: t.disc,
                duration: t.duration,
                genre: t.genre.or_else(|| genre.clone()),
                year: t.year.or(year),
                size: t.size,
                mime: t.mime,
                lyrics_file: t.lyrics_file,
                lyrics_tag: t.lyrics_tag,
                file: t.file,
            });
        }
        artists.iter_mut().find(|x| x.id == artist_id).unwrap().album_ids.push(id);
        out_albums.push(album);
    }
    // Newest album first per artist.
    let years: HashMap<String, (u32, String)> = out_albums.iter().map(|a| (a.id.clone(), (a.year.unwrap_or(0), a.title.to_lowercase()))).collect();
    for ar in &mut artists {
        ar.album_ids.sort_by(|x, y| {
            let (yx, tx) = &years[x];
            let (yy, ty) = &years[y];
            yy.cmp(yx).then(tx.cmp(ty))
        });
    }
    Library { artists, albums: out_albums, tracks: out_tracks }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lyrics {
    #[serde(rename = "type")]
    pub kind: String,
    pub text: String,
}

fn is_lrc(text: &str) -> bool {
    text.lines().any(|l| {
        let l = l.trim_start();
        let b = l.as_bytes();
        b.first() == Some(&b'[') && {
            let digits = b[1..].iter().take_while(|c| c.is_ascii_digit()).count();
            (1..=2).contains(&digits) && b.get(1 + digits) == Some(&b':') && b.get(2 + digits).map(|c| c.is_ascii_digit()).unwrap_or(false) && b.get(3 + digits).map(|c| c.is_ascii_digit()).unwrap_or(false)
        }
    })
}

/// Lyrics for a track: the .lrc/.txt next to it, else the ID3 USLT frame.
pub fn read_lyrics(t: &Track) -> Option<Lyrics> {
    if let Some(f) = &t.lyrics_file {
        if let Ok(b) = fs::read(f) {
            let text = decode_text_file(&b);
            return Some(Lyrics { kind: if is_lrc(&text) { "lrc" } else { "text" }.into(), text });
        }
    }
    if let Some(tag) = &t.lyrics_tag {
        let text = read_uslt(&t.file, tag);
        if !text.is_empty() {
            return Some(Lyrics { kind: if is_lrc(&text) { "lrc" } else { "text" }.into(), text });
        }
    }
    None
}

/// Parses LRC "[mm:ss.xx] line" into (seconds, line), sorted.
pub fn parse_lrc(text: &str) -> Vec<(f64, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut rest = line.trim();
        let mut times = Vec::new();
        while rest.starts_with('[') {
            let Some(end) = rest.find(']') else { break };
            let tag = &rest[1..end];
            if let Some((m, s)) = tag.split_once(':') {
                if let (Ok(m), Ok(s)) = (m.parse::<f64>(), s.parse::<f64>()) {
                    times.push(m * 60.0 + s);
                }
            }
            rest = rest[end + 1..].trim_start();
        }
        for t in times {
            out.push((t, rest.to_string()));
        }
    }
    out.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn track_names() {
        assert_eq!(
            parse_track_name("01 - Madison Beer - Good In Goodbye"),
            TrackName { number: Some(1), artist: Some("Madison Beer".into()), title: "Good In Goodbye".into(), disc: None }
        );
        assert_eq!(parse_track_name("01. Title").title, "Title");
        assert_eq!(parse_track_name("01. Title").number, Some(1));
        let d = parse_track_name("1-03 Title");
        assert_eq!((d.disc, d.number, d.title.as_str()), (Some(1), Some(3), "Title"));
        assert_eq!(parse_track_name("Just A Song").number, None);
        assert_eq!(parse_album_name("Madison Beer - Life Support"), (Some("Madison Beer".into()), "Life Support".into()));
        assert_eq!(disc_dir("CD 2"), Some(2));
        assert_eq!(disc_dir("Disc_1"), Some(1));
        assert_eq!(disc_dir("Discovery"), None);
    }
    #[test]
    fn lrc() {
        let l = parse_lrc("[00:01.50]a\n[00:00.10][01:00.00]b\nnoise");
        assert_eq!(l, vec![(0.1, "b".into()), (1.5, "a".into()), (60.0, "b".into())]);
        assert!(is_lrc("x\n[01:02.00] y"));
        assert!(!is_lrc("[Chorus]"));
    }
    #[test]
    fn scans_library() {
        let root = std::env::temp_dir().join(format!("tvcore-music-{}", std::process::id()));
        let album = root.join("Madison Beer").join("Madison Beer - Life Support");
        fs::create_dir_all(&album).unwrap();
        // tiny ID3v2.3 tag with TIT2 "Channel Surfing / The End"
        let title = b"\x00Channel Surfing / The End";
        let mut frame = b"TIT2".to_vec();
        frame.extend((title.len() as u32).to_be_bytes());
        frame.extend([0, 0]);
        frame.extend(title);
        let mut tag = b"ID3\x03\x00\x00".to_vec();
        let n = frame.len() as u32;
        tag.extend([(n >> 21 & 0x7f) as u8, (n >> 14 & 0x7f) as u8, (n >> 7 & 0x7f) as u8, (n & 0x7f) as u8]);
        tag.extend(frame);
        fs::write(album.join("02 - Madison Beer - Channel Surfing.mp3"), &tag).unwrap();
        fs::write(album.join("01 - Madison Beer - Good In Goodbye.mp3"), b"").unwrap();
        fs::write(album.join("01 - Madison Beer - Good In Goodbye.lrc"), "[00:01.00]hi").unwrap();
        fs::write(album.join("Folder.jpg"), b"x").unwrap();
        fs::write(root.join("Madison Beer").join("artist.jpg"), b"x").unwrap();
        let lib = Scanner::new(&root, None).scan();
        assert_eq!(lib.artists.len(), 1);
        assert_eq!(lib.artists[0].name, "Madison Beer");
        assert!(lib.artists[0].image_file.is_some());
        assert_eq!(lib.albums[0].title, "Life Support");
        assert!(lib.albums[0].cover_file.is_some());
        assert_eq!(lib.tracks.len(), 2);
        assert_eq!(lib.tracks[0].title, "Good In Goodbye");
        assert_eq!(lib.tracks[1].title, "Channel Surfing / The End");
        assert_eq!(read_lyrics(&lib.tracks[0]).unwrap().kind, "lrc");
        fs::remove_dir_all(root).ok();
    }
}
