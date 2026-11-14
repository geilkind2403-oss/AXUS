//! MPEG-TS demuxer (what TV streams over HLS are made of): PAT/PMT, PES
//! assembly, H.264 (Annex B, keyframes found by their IDR slices) and AAC in
//! ADTS frames. 33-bit timestamps are unwrapped and made to start at 0.
use super::*;
use std::collections::{HashMap, VecDeque};
use std::io::Read;

const PKT: usize = 188;

#[derive(Default)]
struct Pes {
    buf: Vec<u8>,
    pts: Option<i64>,
}

#[derive(Clone, Copy, PartialEq)]
enum Es {
    Video,
    Audio(usize),
}

pub struct Ts {
    src: Box<dyn Read + Send>,
    info: StreamInfo,
    pmt_pid: Option<u16>,
    streams: HashMap<u16, Es>,
    pes: HashMap<u16, Pes>,
    queue: VecDeque<Packet>,
    first_pts: Option<i64>,
    last_pts: i64,
    wraps: i64,
    eof: bool,
    /// AAC frames already split from a PES whose pts we've used
    audio_next: HashMap<usize, f64>,
    got_video_size: bool,
}

/// ADTS header -> (header len, frame len, rate, channels, asc)
fn adts(b: &[u8]) -> Option<(usize, usize, u32, u32, Vec<u8>)> {
    if b.len() < 7 || b[0] != 0xff || b[1] & 0xf6 != 0xf0 {
        return None;
    }
    let protection_absent = b[1] & 1;
    let profile = (b[2] >> 6) + 1; // object type
    let sfi = (b[2] >> 2) & 0x0f;
    let ch = (((b[2] & 1) << 2) | (b[3] >> 6)) as u32;
    let len = (((b[3] & 3) as usize) << 11) | ((b[4] as usize) << 3) | ((b[5] as usize) >> 5);
    const RATES: [u32; 13] = [
        96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
    ];
    let rate = *RATES.get(sfi as usize)?;
    let hl = if protection_absent == 1 { 7 } else { 9 };
    if len < hl {
        return None;
    }
    let asc = vec![
        (profile << 3) | (sfi >> 1),
        ((sfi & 1) << 7) | ((ch as u8) << 3),
    ];
    Some((hl, len, rate, ch.max(1), asc))
}

/// NAL units of an Annex B buffer.
pub fn nal_units(b: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut i = 0;
    let mut start = None;
    while i + 3 <= b.len() {
        if b[i] == 0 && b[i + 1] == 0 && b[i + 2] == 1 {
            if let Some(s) = start {
                let mut e = i;
                while e > s && b[e - 1] == 0 {
                    e -= 1;
                }
                out.push(&b[s..e]);
            }
            i += 3;
            start = Some(i);
            continue;
        }
        i += 1;
    }
    if let Some(s) = start {
        if s < b.len() {
            out.push(&b[s..]);
        }
    }
    out
}

impl Ts {
    /// Reads until the programme is known (video size, sound format) so the
    /// player can be set up; those first packets stay queued.
    pub fn open(src: Box<dyn Read + Send>) -> Result<Ts, String> {
        let mut t = Ts {
            src,
            info: StreamInfo::default(),
            pmt_pid: None,
            streams: HashMap::new(),
            pes: HashMap::new(),
            queue: VecDeque::new(),
            first_pts: None,
            last_pts: 0,
            wraps: 0,
            eof: false,
            audio_next: HashMap::new(),
            got_video_size: false,
        };
        let mut read = 0usize;
        loop {
            let audio_known = t.info.audio.iter().all(|a| a.rate > 0) && !t.info.audio.is_empty();
            let video_known = t.info.video.is_none() || t.got_video_size;
            let audio_ready = t.info.audio.is_empty() || audio_known;
            if t.pmt_pid.is_some()
                && !t.streams.is_empty()
                && audio_ready
                && video_known
                && !t.queue.is_empty()
            {
                break;
            }
            if t.pmt_pid.is_some() && !t.streams.is_empty() && read > 6 << 20 {
                break; // something never showed up (radio without picture, ...): go with what we have
            }
            if !t.step()? {
                if t.streams.is_empty() {
                    return Err("Kein Programm im Stream".into());
                }
                break;
            }
            read += PKT;
        }
        // tracks whose format never appeared: drop the placeholder rate
        for a in &mut t.info.audio {
            if a.rate == 0 {
                a.rate = 48000;
            }
        }
        Ok(t)
    }

    /// Reads one 188-byte packet. false at the end of the stream.
    fn step(&mut self) -> Result<bool, String> {
        let mut p = [0u8; PKT];
        if self.read_exact(&mut p[..1]).is_err() {
            return Ok(self.finish_all());
        }
        if p[0] != 0x47 {
            return Ok(true); // resync byte by byte
        }
        if self.read_exact(&mut p[1..]).is_err() {
            return Ok(self.finish_all());
        }
        let pusi = p[1] & 0x40 != 0;
        let pid = (((p[1] & 0x1f) as u16) << 8) | p[2] as u16;
        let afc = (p[3] >> 4) & 3;
        let mut off = 4;
        if afc & 2 != 0 {
            off += 1 + p[4] as usize;
        }
        if afc & 1 == 0 || off >= PKT {
            return Ok(true);
        }
        let payload = &p[off..];
        if pid == 0 {
            self.parse_pat(payload, pusi);
        } else if Some(pid) == self.pmt_pid {
            self.parse_pmt(payload, pusi);
        } else if self.streams.contains_key(&pid) {
            if pusi {
                self.flush(pid);
                let e = self.pes.entry(pid).or_default();
                e.buf.clear();
                e.pts = None;
                // PES header
                if payload.len() >= 9 && payload[0] == 0 && payload[1] == 0 && payload[2] == 1 {
                    let flags = payload[7];
                    let hl = payload[8] as usize;
                    if flags & 0x80 != 0 && payload.len() >= 14 {
                        let b = &payload[9..14];
                        let pts = (((b[0] >> 1) & 7) as i64) << 30
                            | (b[1] as i64) << 22
                            | ((b[2] >> 1) as i64) << 15
                            | (b[3] as i64) << 7
                            | (b[4] >> 1) as i64;
                        e.pts = Some(pts);
                    }
                    if 9 + hl <= payload.len() {
                        e.buf.extend_from_slice(&payload[9 + hl..]);
                    }
                }
            } else if let Some(e) = self.pes.get_mut(&pid) {
                if e.buf.len() < 8 << 20 {
                    e.buf.extend_from_slice(payload);
                }
            }
        }
        Ok(true)
    }

    fn read_exact(&mut self, b: &mut [u8]) -> std::io::Result<()> {
        self.src.read_exact(b)
    }

    fn section<'a>(&self, payload: &'a [u8], pusi: bool) -> Option<&'a [u8]> {
        if !pusi || payload.is_empty() {
            return None;
        }
        let ptr = payload[0] as usize;
        let s = payload.get(1 + ptr..)?;
        if s.len() < 3 {
            return None;
        }
        let len = (((s[1] & 0x0f) as usize) << 8) | s[2] as usize;
        s.get(..(3 + len).min(s.len()))
    }

    fn parse_pat(&mut self, payload: &[u8], pusi: bool) {
        let Some(s) = self.section(payload, pusi) else {
            return;
        };
        let mut i = 8;
        while i + 4 <= s.len().saturating_sub(4) {
            let prog = u16::from_be_bytes([s[i], s[i + 1]]);
            let pid = (((s[i + 2] & 0x1f) as u16) << 8) | s[i + 3] as u16;
            if prog != 0 {
                self.pmt_pid.get_or_insert(pid);
                return;
            }
            i += 4;
        }
    }

    fn parse_pmt(&mut self, payload: &[u8], pusi: bool) {
        if !self.streams.is_empty() {
            return;
        }
        let Some(s) = self.section(payload, pusi) else {
            return;
        };
        if s.len() < 12 {
            return;
        }
        let pil = (((s[10] & 0x0f) as usize) << 8) | s[11] as usize;
        let mut i = 12 + pil;
        let end = s.len().saturating_sub(4);
        while i + 5 <= end {
            let st = s[i];
            let pid = (((s[i + 1] & 0x1f) as u16) << 8) | s[i + 2] as u16;
            let il = (((s[i + 3] & 0x0f) as usize) << 8) | s[i + 4] as usize;
            let desc = s.get(i + 5..(i + 5 + il).min(end)).unwrap_or(&[]);
            // descriptors: 0x0a language, 0x6a AC-3, 0x7a E-AC-3
            let mut lang = String::new();
            let mut ac3 = false;
            let mut d = 0;
            while d + 2 <= desc.len() {
                let (tag, len) = (desc[d], desc[d + 1] as usize);
                if tag == 0x0a && len >= 3 && d + 5 <= desc.len() {
                    lang = String::from_utf8_lossy(&desc[d + 2..d + 5]).to_string();
                }
                if tag == 0x6a || tag == 0x7a {
                    ac3 = true;
                }
                d += 2 + len;
            }
            match st {
                0x1b if self.info.video.is_none() => {
                    self.info.video = Some(VideoTrack {
                        codec: VCodec::H264 { avcc: Vec::new() },
                        width: 0,
                        height: 0,
                        sar: 1.0,
                    });
                    self.streams.insert(pid, Es::Video);
                }
                0x24 if self.info.video.is_none() => {
                    self.info.video = Some(VideoTrack {
                        codec: VCodec::Hevc,
                        width: 0,
                        height: 0,
                        sar: 1.0,
                    });
                    self.streams.insert(pid, Es::Video);
                }
                0x0f | 0x11 | 0x03 | 0x04 | 0x81 | 0x87 | 0x06 => {
                    let codec = match st {
                        0x0f | 0x11 => ACodec::Aac { asc: Vec::new() },
                        0x03 | 0x04 => ACodec::Mp3,
                        0x81 => ACodec::Ac3,
                        0x87 => ACodec::Eac3,
                        _ if ac3 => ACodec::Ac3,
                        _ => {
                            i += 5 + il;
                            continue;
                        }
                    };
                    let n = self.info.audio.len();
                    let known = !matches!(codec, ACodec::Aac { .. } | ACodec::Mp3);
                    self.info.audio.push(AudioTrack {
                        codec,
                        rate: if known { 48000 } else { 0 },
                        channels: 2,
                        lang,
                        name: String::new(),
                        default: n == 0,
                    });
                    self.streams.insert(pid, Es::Audio(n));
                }
                _ => {}
            }
            i += 5 + il;
        }
    }

    /// 90 kHz timestamp -> seconds from the first one, across 33-bit wraps.
    fn time(&mut self, pts: i64) -> f64 {
        const WRAP: i64 = 1 << 33;
        let mut full = pts + self.wraps * WRAP;
        if full < self.last_pts - WRAP / 2 {
            self.wraps += 1;
            full += WRAP;
        } else if full > self.last_pts + WRAP / 2 && self.last_pts != 0 {
            full -= WRAP; // a slightly older pts right after a wrap
        }
        self.last_pts = self.last_pts.max(full);
        let first = *self.first_pts.get_or_insert(full);
        (full - first) as f64 / 90000.0
    }

    fn flush(&mut self, pid: u16) {
        let Some(es) = self.streams.get(&pid).copied() else {
            return;
        };
        let Some(pes) = self.pes.get_mut(&pid) else {
            return;
        };
        if pes.buf.is_empty() {
            return;
        }
        let data = std::mem::take(&mut pes.buf);
        let pts = pes.pts.take();
        match es {
            Es::Video => {
                let mut key = false;
                for n in nal_units(&data) {
                    match n.first().map(|b| b & 0x1f) {
                        Some(5) => key = true,
                        Some(7) => {
                            key = true;
                            if !self.got_video_size {
                                if let Some((w, h)) = sps_size(n) {
                                    if let Some(v) = self.info.video.as_mut() {
                                        v.width = w;
                                        v.height = h;
                                    }
                                    self.got_video_size = true;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                if matches!(
                    self.info.video.as_ref().map(|v| &v.codec),
                    Some(VCodec::Hevc)
                ) {
                    self.got_video_size = true;
                }
                let t = pts
                    .map(|p| self.time(p))
                    .unwrap_or_else(|| self.queue.back().map(|p| p.pts + 0.04).unwrap_or(0.0));
                self.queue.push_back(Packet {
                    kind: Kind::Video,
                    pts: t,
                    dur: 0.0,
                    key,
                    data,
                });
            }
            Es::Audio(i) => {
                let base = pts.map(|p| self.time(p));
                let mut t = base
                    .or_else(|| self.audio_next.get(&i).copied())
                    .unwrap_or(0.0);
                if matches!(self.info.audio[i].codec, ACodec::Aac { .. }) {
                    let mut p = 0;
                    while p < data.len() {
                        let Some((hl, len, rate, ch, asc)) = adts(&data[p..]) else {
                            break;
                        };
                        if p + len > data.len() {
                            break;
                        }
                        let a = &mut self.info.audio[i];
                        if a.rate == 0 {
                            a.rate = rate;
                            a.channels = ch;
                            a.codec = ACodec::Aac { asc };
                        }
                        self.queue.push_back(Packet {
                            kind: Kind::Audio(i),
                            pts: t,
                            dur: 0.0,
                            key: true,
                            data: data[p + hl..p + len].to_vec(),
                        });
                        t += 1024.0 / rate as f64;
                        p += len;
                    }
                } else {
                    if self.info.audio[i].rate == 0 {
                        self.info.audio[i].rate = mp3_rate(&data).unwrap_or(48000);
                    }
                    self.queue.push_back(Packet {
                        kind: Kind::Audio(i),
                        pts: t,
                        dur: 0.0,
                        key: true,
                        data,
                    });
                    t += 0.024;
                }
                self.audio_next.insert(i, t);
            }
        }
    }

    fn finish_all(&mut self) -> bool {
        let pids: Vec<u16> = self.pes.keys().copied().collect();
        for pid in pids {
            self.flush(pid);
        }
        self.eof = true;
        false
    }
}

fn mp3_rate(b: &[u8]) -> Option<u32> {
    let i = b
        .windows(2)
        .position(|w| w[0] == 0xff && w[1] & 0xe0 == 0xe0)?;
    let h = b.get(i + 2)?;
    let v = (b[i + 1] >> 3) & 3;
    let sr = ((h >> 2) & 3) as usize;
    let base = [44100, 48000, 32000].get(sr)?;
    Some(match v {
        3 => *base,
        2 => base / 2,
        _ => base / 4,
    })
}

/// Width and height from an H.264 SPS (with cropping), e.g. 1920x1080
/// although the coded picture is 1920x1088.
pub fn sps_size(nal: &[u8]) -> Option<(u32, u32)> {
    // drop emulation prevention bytes
    let mut rbsp = Vec::with_capacity(nal.len());
    let mut zeros = 0;
    for &b in nal.get(1..)? {
        if zeros >= 2 && b == 3 {
            zeros = 0;
            continue;
        }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        rbsp.push(b);
    }
    let mut r = Bits { b: &rbsp, pos: 0 };
    let profile = r.bits(8)?;
    r.bits(16)?; // constraints + level
    r.ue()?; // sps id
    let mut chroma = 1;
    if [100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135].contains(&profile) {
        chroma = r.ue()?;
        if chroma == 3 {
            r.bits(1)?;
        }
        r.ue()?;
        r.ue()?;
        r.bits(1)?;
        if r.bits(1)? == 1 {
            for i in 0..if chroma != 3 { 8 } else { 12 } {
                if r.bits(1)? == 1 {
                    let size = if i < 6 { 16 } else { 64 };
                    let (mut last, mut next) = (8i64, 8i64);
                    for _ in 0..size {
                        if next != 0 {
                            next = (last + r.se()? + 256) % 256;
                        }
                        if next != 0 {
                            last = next;
                        }
                    }
                }
            }
        }
    }
    r.ue()?; // log2_max_frame_num
    match r.ue()? {
        0 => {
            r.ue()?;
        }
        1 => {
            r.bits(1)?;
            r.se()?;
            r.se()?;
            for _ in 0..r.ue()? {
                r.se()?;
            }
        }
        _ => {}
    }
    r.ue()?; // max refs
    r.bits(1)?;
    let wm = r.ue()? + 1;
    let hm = r.ue()? + 1;
    let frame_mbs_only = r.bits(1)?;
    if frame_mbs_only == 0 {
        r.bits(1)?;
    }
    r.bits(1)?;
    let (mut cl, mut cr, mut ct, mut cb) = (0, 0, 0, 0);
    if r.bits(1)? == 1 {
        cl = r.ue()?;
        cr = r.ue()?;
        ct = r.ue()?;
        cb = r.ue()?;
    }
    let (cx, cy) = match chroma {
        0 | 3 => (1, 2 - frame_mbs_only),
        2 => (2, 2 - frame_mbs_only),
        _ => (2, 2 * (2 - frame_mbs_only)),
    };
    let w = wm * 16 - cx * (cl + cr);
    let h = (2 - frame_mbs_only) * hm * 16 - cy * (ct + cb);
    Some((w, h))
}

struct Bits<'a> {
    b: &'a [u8],
    pos: usize,
}
impl Bits<'_> {
    fn bits(&mut self, n: u32) -> Option<u32> {
        let mut v = 0u32;
        for _ in 0..n {
            let byte = *self.b.get(self.pos / 8)?;
            v = (v << 1) | ((byte >> (7 - self.pos % 8)) & 1) as u32;
            self.pos += 1;
        }
        Some(v)
    }
    fn ue(&mut self) -> Option<u32> {
        let mut z = 0;
        while self.bits(1)? == 0 {
            z += 1;
            if z > 31 {
                return None;
            }
        }
        Some((1u32 << z) - 1 + self.bits(z)?)
    }
    fn se(&mut self) -> Option<i64> {
        let k = self.ue()? as i64;
        Some(if k % 2 == 1 { (k + 1) / 2 } else { -(k / 2) })
    }
}

impl Demuxer for Ts {
    fn info(&self) -> &StreamInfo {
        &self.info
    }
    fn next(&mut self) -> Result<Option<Packet>, String> {
        while self.queue.is_empty() {
            if self.eof || !self.step()? {
                return Ok(self.queue.pop_front());
            }
        }
        Ok(self.queue.pop_front())
    }
    fn seek(&mut self, _t: f64) -> Result<f64, String> {
        Err("Live-Sender: kein Springen".into())
    }
    fn can_seek(&self) -> bool {
        false
    }
}
