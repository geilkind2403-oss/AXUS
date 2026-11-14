//! Matroska / WebM demuxer: tracks, clusters (Simple- and BlockGroups with
//! lacing), cues for seeking, text subtitles (SRT / ASS) shown by the player.
use super::*;
use std::io::{Read, SeekFrom};

const SEGMENT: u32 = 0x18538067;
const SEEKHEAD: u32 = 0x114D9B74;
const INFO: u32 = 0x1549A966;
const TRACKS: u32 = 0x1654AE6B;
const CLUSTER: u32 = 0x1F43B675;
const CUES: u32 = 0x1C53BB6B;
const TOP_LEVEL: [u32; 9] = [
    SEEKHEAD, INFO, TRACKS, CLUSTER, CUES, 0x1043A770, 0x1254C367, 0x1941A469, 0xEC,
];

struct Track {
    number: u64,
    kind: Kind,
    /// subtitles: ASS events need their text field cut out
    ass: bool,
    nal_fix: bool,
}

pub struct Mkv {
    src: Box<dyn Source>,
    info: StreamInfo,
    tracks: Vec<Track>,
    scale: f64,
    seg_start: u64,
    seg_end: u64,
    pos: u64,
    cluster_time: i64,
    cluster_end: u64,
    cues: Vec<(f64, u64)>,
    queue: std::collections::VecDeque<Packet>,
    seekable: bool,
}

fn read_vint(src: &mut dyn Read, keep_marker: bool) -> std::io::Result<(u64, usize)> {
    let mut first = [0u8; 1];
    src.read_exact(&mut first)?;
    let len = first[0].leading_zeros() as usize + 1;
    if len > 8 {
        return Err(std::io::Error::other("vint"));
    }
    let mut v = if keep_marker {
        first[0] as u64
    } else {
        (first[0] as u64) & (0xff >> len)
    };
    let mut rest = [0u8; 7];
    src.read_exact(&mut rest[..len - 1])?;
    for b in &rest[..len - 1] {
        v = (v << 8) | *b as u64;
    }
    Ok((v, len))
}

fn vint_slice(b: &[u8], p: &mut usize) -> Option<u64> {
    let first = *b.get(*p)?;
    let len = first.leading_zeros() as usize + 1;
    if len > 8 || *p + len > b.len() {
        return None;
    }
    let mut v = (first as u64) & (0xff >> len);
    for i in 1..len {
        v = (v << 8) | b[*p + i] as u64;
    }
    *p += len;
    Some(v)
}

fn id_slice(b: &[u8], p: &mut usize) -> Option<u32> {
    let first = *b.get(*p)?;
    let len = first.leading_zeros() as usize + 1;
    if len > 4 || *p + len > b.len() {
        return None;
    }
    let mut v = 0u32;
    for i in 0..len {
        v = (v << 8) | b[*p + i] as u32;
    }
    *p += len;
    Some(v)
}

/// Child elements of a buffered master element: (id, body).
fn elements(b: &[u8]) -> Vec<(u32, &[u8])> {
    let mut out = Vec::new();
    let mut p = 0;
    while p < b.len() {
        let Some(id) = id_slice(b, &mut p) else { break };
        let Some(size) = vint_slice(b, &mut p) else {
            break;
        };
        let end = (p as u64).saturating_add(size).min(b.len() as u64) as usize;
        out.push((id, &b[p..end]));
        p = end;
    }
    out
}

fn uint(b: &[u8]) -> u64 {
    b.iter().fold(0u64, |a, x| (a << 8) | *x as u64)
}
fn float(b: &[u8]) -> f64 {
    match b.len() {
        4 => f32::from_be_bytes(b.try_into().unwrap()) as f64,
        8 => f64::from_be_bytes(b.try_into().unwrap()),
        _ => 0.0,
    }
}
fn string(b: &[u8]) -> String {
    String::from_utf8_lossy(b)
        .trim_end_matches('\0')
        .to_string()
}

const UNKNOWN: u64 = u64::MAX;

impl Mkv {
    pub fn open(mut src: Box<dyn Source>, seekable: bool) -> Result<Mkv, String> {
        let e = |x: std::io::Error| x.to_string();
        // EBML header
        let (id, _) = read_vint(&mut src, true).map_err(e)?;
        if id != 0x1A45DFA3 {
            return Err("keine MKV-Datei".into());
        }
        let (size, _) = read_vint(&mut src, false).map_err(e)?;
        let mut skip = vec![0u8; size as usize];
        src.read_exact(&mut skip).map_err(e)?;
        let (id, l1) = read_vint(&mut src, true).map_err(e)?;
        if id != SEGMENT as u64 {
            return Err("MKV ohne Segment".into());
        }
        let (size, l2) = read_vint(&mut src, false).map_err(e)?;
        let seg_start = (4 + 1 + skip.len() + l1 + l2) as u64; // approximation fixed below
        let seg_start = src.stream_position().unwrap_or(seg_start);
        let all_ones = |s: u64, len: usize| s == (1u64 << (7 * len)) - 1;
        let seg_end = if all_ones(size, l2) {
            UNKNOWN
        } else {
            seg_start + size
        };
        let mut m = Mkv {
            src,
            info: StreamInfo::default(),
            tracks: Vec::new(),
            scale: 1e-6,
            seg_start,
            seg_end,
            pos: seg_start,
            cluster_time: 0,
            cluster_end: 0,
            cues: Vec::new(),
            queue: Default::default(),
            seekable,
        };
        let mut cues_pos = None;
        // header elements up to the first cluster
        loop {
            m.src.seek(SeekFrom::Start(m.pos)).map_err(e)?;
            let (id, il) = read_vint(&mut m.src, true).map_err(e)?;
            let (size, sl) = read_vint(&mut m.src, false).map_err(e)?;
            let body_at = m.pos + (il + sl) as u64;
            if id as u32 == CLUSTER {
                break;
            }
            if size > 64 << 20 {
                return Err("MKV-Kopf zu groß".into());
            }
            let mut body = vec![0u8; size as usize];
            m.src.read_exact(&mut body).map_err(e)?;
            match id as u32 {
                INFO => {
                    for (cid, b) in elements(&body) {
                        match cid {
                            0x2AD7B1 => m.scale = uint(b) as f64 / 1e9,
                            0x4489 => m.info.duration = float(b),
                            _ => {}
                        }
                    }
                }
                TRACKS => m.parse_tracks(&body),
                SEEKHEAD => {
                    for (sid, s) in elements(&body) {
                        if sid != 0x4DBB {
                            continue;
                        }
                        let mut which = 0;
                        let mut at = 0;
                        for (k, v) in elements(s) {
                            match k {
                                0x53AB => which = uint(v),
                                0x53AC => at = uint(v),
                                _ => {}
                            }
                        }
                        if which == CUES as u64 {
                            cues_pos = Some(seg_start + at);
                        }
                    }
                }
                CUES => m.parse_cues(&body),
                _ => {}
            }
            m.pos = body_at + size;
        }
        m.info.duration *= m.scale;
        if m.cues.is_empty() && seekable {
            if let Some(cp) = cues_pos {
                let here = m.pos;
                if m.src.seek(SeekFrom::Start(cp)).is_ok() {
                    if let (Ok((id, _)), Ok((size, _))) =
                        (read_vint(&mut m.src, true), read_vint(&mut m.src, false))
                    {
                        if id as u32 == CUES && size < 64 << 20 {
                            let mut body = vec![0u8; size as usize];
                            if m.src.read_exact(&mut body).is_ok() {
                                m.parse_cues(&body);
                            }
                        }
                    }
                }
                m.pos = here;
            }
        }
        if m.tracks.is_empty() {
            return Err("MKV ohne Bild und Ton".into());
        }
        Ok(m)
    }

    fn parse_tracks(&mut self, body: &[u8]) {
        for (id, entry) in elements(body) {
            if id != 0xAE {
                continue;
            }
            let (mut number, mut typ, mut codec, mut private, mut lang, mut name) = (
                0,
                0,
                String::new(),
                Vec::new(),
                String::from("eng"),
                String::new(),
            );
            let (mut default, mut forced) = (true, false);
            let (mut w, mut h, mut dw, mut dh, mut rate, mut ch) =
                (0u64, 0u64, 0u64, 0u64, 0f64, 0u64);
            let mut bcp = String::new();
            for (k, v) in elements(entry) {
                match k {
                    0xD7 => number = uint(v),
                    0x83 => typ = uint(v),
                    0x86 => codec = string(v),
                    0x63A2 => private = v.to_vec(),
                    0x22B59C => lang = string(v),
                    0x22B59D => bcp = string(v),
                    0x536E => name = string(v),
                    0x88 => default = uint(v) != 0,
                    0x55AA => forced = uint(v) != 0,
                    0xE0 => {
                        for (vk, vv) in elements(v) {
                            match vk {
                                0xB0 => w = uint(vv),
                                0xBA => h = uint(vv),
                                0x54B0 => dw = uint(vv),
                                0x54BA => dh = uint(vv),
                                _ => {}
                            }
                        }
                    }
                    0xE1 => {
                        for (ak, av) in elements(v) {
                            match ak {
                                0xB5 => rate = float(av),
                                0x9F => ch = uint(av),
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
            if !bcp.is_empty() && lang == "eng" {
                lang = bcp;
            }
            let kind = match typ {
                1 if self.info.video.is_none() => {
                    let vc = match codec.as_str() {
                        "V_MPEG4/ISO/AVC" => VCodec::H264 { avcc: private },
                        "V_MPEGH/ISO/HEVC" => VCodec::Hevc,
                        o => VCodec::Other(o.into()),
                    };
                    let sar = if dw > 0 && dh > 0 && w > 0 && h > 0 {
                        (dw as f32 / dh as f32) / (w as f32 / h as f32)
                    } else {
                        1.0
                    };
                    self.info.video = Some(VideoTrack {
                        codec: vc,
                        width: w as u32,
                        height: h as u32,
                        sar,
                    });
                    Kind::Video
                }
                2 => {
                    let ac = match codec.as_str() {
                        "A_AAC" => ACodec::Aac { asc: private },
                        c if c.starts_with("A_AAC/") => ACodec::Aac { asc: Vec::new() },
                        "A_MPEG/L3" | "A_MPEG/L2" => ACodec::Mp3,
                        "A_AC3" => ACodec::Ac3,
                        "A_EAC3" => ACodec::Eac3,
                        c if c.starts_with("A_DTS") => ACodec::Dts,
                        "A_OPUS" => ACodec::Opus,
                        "A_VORBIS" => ACodec::Vorbis,
                        "A_FLAC" => ACodec::Flac,
                        "A_PCM/INT/LIT" => ACodec::Pcm16,
                        o => ACodec::Other(o.into()),
                    };
                    self.info.audio.push(AudioTrack {
                        codec: ac,
                        rate: if rate > 0.0 { rate as u32 } else { 48000 },
                        channels: ch.max(1) as u32,
                        lang,
                        name,
                        default,
                    });
                    Kind::Audio(self.info.audio.len() - 1)
                }
                17 if codec == "S_TEXT/UTF8"
                    || codec == "S_TEXT/ASS"
                    || codec == "S_TEXT/SSA"
                    || codec == "S_ASS"
                    || codec == "S_SSA" =>
                {
                    let ass = codec != "S_TEXT/UTF8";
                    let tl = name.to_lowercase();
                    self.info.subs.push(SubTrack {
                        lang,
                        forced: forced || tl.contains("forced") || tl.contains("erzwungen"),
                        default,
                        ass,
                        name,
                    });
                    Kind::Sub(self.info.subs.len() - 1)
                }
                _ => continue,
            };
            let ass = matches!(kind, Kind::Sub(i) if self.info.subs[i].ass);
            self.tracks.push(Track {
                number,
                kind,
                ass,
                nal_fix: false,
            });
        }
        let _ = self.tracks.iter().map(|t| t.nal_fix).count();
    }

    fn parse_cues(&mut self, body: &[u8]) {
        let vt = self
            .tracks
            .iter()
            .find(|t| t.kind == Kind::Video)
            .map(|t| t.number);
        for (id, cp) in elements(body) {
            if id != 0xBB {
                continue;
            }
            let mut time = 0;
            let mut pos = None;
            for (k, v) in elements(cp) {
                match k {
                    0xB3 => time = uint(v),
                    0xB7 => {
                        let mut track = 0;
                        let mut cpos = 0;
                        for (kk, vv) in elements(v) {
                            match kk {
                                0xF7 => track = uint(vv),
                                0xF1 => cpos = uint(vv),
                                _ => {}
                            }
                        }
                        if pos.is_none() || Some(track) == vt {
                            pos = Some(cpos);
                        }
                    }
                    _ => {}
                }
            }
            if let Some(p) = pos {
                self.cues
                    .push((time as f64 * self.scale, self.seg_start + p));
            }
        }
        self.cues.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    }

    fn block(&mut self, b: &[u8], key_hint: Option<bool>, dur: Option<u64>) {
        let mut p = 0;
        let Some(track) = vint_slice(b, &mut p) else {
            return;
        };
        if p + 3 > b.len() {
            return;
        }
        let rel = i16::from_be_bytes([b[p], b[p + 1]]) as i64;
        let flags = b[p + 2];
        p += 3;
        let Some(t) = self.tracks.iter().find(|t| t.number == track) else {
            return;
        };
        let kind = t.kind;
        let ass = t.ass;
        let pts = (self.cluster_time + rel) as f64 * self.scale;
        let key = key_hint.unwrap_or(flags & 0x80 != 0);
        let dur = dur.map(|d| d as f64 * self.scale).unwrap_or(0.0);
        let lacing = (flags >> 1) & 3;
        let frames: Vec<&[u8]> = if lacing == 0 {
            vec![&b[p..]]
        } else {
            let Some(&n) = b.get(p) else { return };
            p += 1;
            let n = n as usize + 1;
            let mut sizes = Vec::with_capacity(n);
            match lacing {
                1 => {
                    for _ in 0..n - 1 {
                        let mut s = 0usize;
                        loop {
                            let Some(&c) = b.get(p) else { return };
                            p += 1;
                            s += c as usize;
                            if c != 255 {
                                break;
                            }
                        }
                        sizes.push(s);
                    }
                }
                3 => {
                    let Some(first) = vint_slice(b, &mut p) else {
                        return;
                    };
                    sizes.push(first as usize);
                    let mut last = first as i64;
                    for _ in 1..n - 1 {
                        let start = p;
                        let Some(raw) = vint_slice(b, &mut p) else {
                            return;
                        };
                        let len = p - start;
                        let bias = (1i64 << (7 * len - 1)) - 1;
                        last += raw as i64 - bias;
                        sizes.push(last.max(0) as usize);
                    }
                }
                _ => {
                    let each = (b.len() - p) / n;
                    sizes = vec![each; n - 1];
                }
            }
            let used: usize = sizes.iter().sum();
            if p + used > b.len() {
                return;
            }
            sizes.push(b.len() - p - used);
            let mut v = Vec::new();
            for s in sizes {
                v.push(&b[p..p + s]);
                p += s;
            }
            v
        };
        let frame_dur = match kind {
            Kind::Audio(i) => {
                let a = &self.info.audio[i];
                match a.codec {
                    ACodec::Aac { .. } => 1024.0 / a.rate.max(1) as f64,
                    ACodec::Mp3 => 1152.0 / a.rate.max(1) as f64,
                    _ => 0.0,
                }
            }
            _ => 0.0,
        };
        for (i, f) in frames.into_iter().enumerate() {
            let data = if ass {
                ass_text(f).into_bytes()
            } else {
                f.to_vec()
            };
            self.queue.push_back(Packet {
                kind,
                pts: pts + i as f64 * frame_dur,
                dur,
                key,
                data,
            });
        }
    }

    /// Reads elements until at least one packet is queued.
    fn fill(&mut self) -> Result<bool, String> {
        let e = |x: std::io::Error| x.to_string();
        while self.queue.is_empty() {
            if self.seg_end != UNKNOWN && self.pos >= self.seg_end {
                return Ok(false);
            }
            if self.src.seek(SeekFrom::Start(self.pos)).is_err() {
                return Ok(false);
            }
            let (id, il) = match read_vint(&mut self.src, true) {
                Ok(v) => v,
                Err(_) => return Ok(false),
            };
            let (size, sl) = read_vint(&mut self.src, false).map_err(e)?;
            let unknown = size == (1u64 << (7 * sl)) - 1;
            let body_at = self.pos + (il + sl) as u64;
            let id = id as u32;
            // a new top-level element ends a cluster of unknown size
            if self.cluster_end != UNKNOWN && body_at > self.cluster_end && id != CLUSTER {
                // past the cluster: fine, top-level element follows
            }
            match id {
                CLUSTER => {
                    self.cluster_end = if unknown { UNKNOWN } else { body_at + size };
                    self.pos = body_at; // descend
                }
                0xE7 => {
                    let mut b = vec![0u8; size as usize];
                    self.src.read_exact(&mut b).map_err(e)?;
                    self.cluster_time = uint(&b) as i64;
                    self.pos = body_at + size;
                }
                0xA3 => {
                    if size > 32 << 20 {
                        return Err("Block zu groß".into());
                    }
                    let mut b = vec![0u8; size as usize];
                    self.src.read_exact(&mut b).map_err(e)?;
                    self.pos = body_at + size;
                    self.block(&b, None, None);
                }
                0xA0 => {
                    if size > 32 << 20 {
                        return Err("Block zu groß".into());
                    }
                    let mut b = vec![0u8; size as usize];
                    self.src.read_exact(&mut b).map_err(e)?;
                    self.pos = body_at + size;
                    let kids = elements(&b);
                    let has_ref = kids.iter().any(|(k, _)| *k == 0xFB);
                    let dur = kids.iter().find(|(k, _)| *k == 0x9B).map(|(_, v)| uint(v));
                    if let Some((_, blk)) = kids.iter().find(|(k, _)| *k == 0xA1) {
                        let blk = blk.to_vec();
                        self.block(&blk, Some(!has_ref), dur);
                    }
                }
                _ if unknown => return Ok(false),
                _ if TOP_LEVEL.contains(&id) || id == SEGMENT => {
                    self.pos = body_at + size; // skip cues, tags, attachments...
                }
                _ => {
                    self.pos = body_at + size;
                }
            }
        }
        Ok(true)
    }
}

/// ASS event "ReadOrder,Layer,Style,Name,MarginL,MarginR,MarginV,Effect,Text" -> Text
fn ass_text(b: &[u8]) -> String {
    let s = String::from_utf8_lossy(b);
    s.splitn(9, ',').nth(8).unwrap_or(&s).to_string()
}

impl Demuxer for Mkv {
    fn info(&self) -> &StreamInfo {
        &self.info
    }
    fn next(&mut self) -> Result<Option<Packet>, String> {
        if self.queue.is_empty() && !self.fill()? {
            return Ok(None);
        }
        Ok(self.queue.pop_front())
    }
    fn seek(&mut self, t: f64) -> Result<f64, String> {
        if !self.seekable {
            return Err("nicht springbar".into());
        }
        if self.cues.is_empty() {
            return Err("MKV ohne Sprungmarken (Cues)".into());
        }
        let i = self.cues.iter().rposition(|c| c.0 <= t + 0.01).unwrap_or(0);
        self.pos = self.cues[i].1;
        self.queue.clear();
        self.cluster_end = 0;
        Ok(self.cues[i].0)
    }
    fn can_seek(&self) -> bool {
        self.seekable && !self.cues.is_empty()
    }
}
