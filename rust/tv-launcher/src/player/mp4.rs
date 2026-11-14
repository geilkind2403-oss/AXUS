//! MP4 / MOV demuxer: classic files (sample tables, moov at start or end)
//! and fragmented MP4 (the PC server's live remux, YouTube's DASH files with
//! a sidx index for seeking).
use super::*;
use std::io::{Read, SeekFrom};

fn be16(b: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([b[o], b[o + 1]])
}
fn be32(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn be64(b: &[u8], o: usize) -> u64 {
    u64::from_be_bytes(b[o..o + 8].try_into().unwrap())
}

/// Iterates the child boxes of a payload: (type, body).
fn boxes(b: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    let mut p = 0;
    while p + 8 <= b.len() {
        let mut size = be32(b, p) as usize;
        let typ: [u8; 4] = b[p + 4..p + 8].try_into().unwrap();
        let mut hl = 8;
        if size == 1 && p + 16 <= b.len() {
            size = be64(b, p + 8) as usize;
            hl = 16;
        } else if size == 0 {
            size = b.len() - p;
        }
        if size < hl || p + size > b.len() {
            break;
        }
        out.push((typ, &b[p + hl..p + size]));
        p += size;
    }
    out
}

fn child<'a>(b: &'a [u8], t: &[u8; 4]) -> Option<&'a [u8]> {
    boxes(b)
        .into_iter()
        .find(|(ty, _)| ty == t)
        .map(|(_, body)| body)
}

#[derive(Clone, Copy)]
struct Sample {
    offset: u64,
    size: u32,
    dts: i64,
    cts: i64,
    key: bool,
}

struct Trak {
    id: u32,
    kind: Kind,
    timescale: u32,
    /// edit list shift (media time that plays at 0)
    shift: i64,
    samples: Vec<Sample>,
    next: usize,
    // trex defaults for fragments
    def_dur: u32,
    def_size: u32,
    def_flags: u32,
}

struct Seg {
    offset: u64,
    time: f64,
}

pub struct Mp4 {
    src: Box<dyn Source>,
    info: StreamInfo,
    traks: Vec<Trak>,
    fragmented: bool,
    /// where the next top-level box starts (fragmented reading)
    pos: u64,
    segs: Vec<Seg>,
    seekable: bool,
    /// fragment samples waiting to be read: (trak index, sample)
    pending: std::collections::VecDeque<(usize, Sample)>,
    /// bytes of the last mdat already read into memory (forward-only streams)
    mdat: Option<(u64, Vec<u8>)>,
}

impl Mp4 {
    pub fn open(mut src: Box<dyn Source>, seekable: bool) -> Result<Mp4, String> {
        let mut pos = 0u64;
        let mut moov = None;
        let mut segs = Vec::new();
        let mut first_frag = None;
        // walk top-level boxes until moov (+ sidx) is known
        loop {
            let Some((typ, size, hl)) = read_header(&mut src, pos)? else {
                break;
            };
            match &typ {
                b"moov" => {
                    let mut body = vec![0u8; (size - hl) as usize];
                    src.read_exact(&mut body).map_err(|e| e.to_string())?;
                    moov = Some(body);
                }
                b"sidx" => {
                    let mut body = vec![0u8; (size - hl) as usize];
                    src.read_exact(&mut body).map_err(|e| e.to_string())?;
                    segs = parse_sidx(&body, pos + size);
                }
                b"moof" => {
                    first_frag = Some(pos);
                    break;
                }
                b"mdat" if moov.is_none() && !seekable => {
                    return Err("MP4 ohne Index am Anfang -- als Stream nicht abspielbar".into())
                }
                _ => {}
            }
            if size == 0 {
                break;
            }
            pos += size;
            if moov.is_some() && (first_frag.is_some() || !segs.is_empty()) {
                break;
            }
            if moov.is_some() && !seekable {
                // stream: moov first, fragments follow right after
                first_frag = Some(pos);
                break;
            }
        }
        let moov = moov.ok_or("MP4 ohne moov")?;
        let mut m = Mp4 {
            src,
            info: StreamInfo::default(),
            traks: Vec::new(),
            fragmented: false,
            pos: 0,
            segs,
            seekable,
            pending: Default::default(),
            mdat: None,
        };
        m.parse_moov(&moov)?;
        if m.fragmented {
            m.pos = first_frag
                .or(m.segs.first().map(|s| s.offset))
                .unwrap_or(pos);
        }
        Ok(m)
    }

    fn parse_moov(&mut self, moov: &[u8]) -> Result<(), String> {
        let mvhd = child(moov, b"mvhd");
        let mut duration = mvhd
            .map(|b| {
                if b[0] == 1 {
                    be64(b, 24) as f64 / be32(b, 20).max(1) as f64
                } else {
                    be32(b, 16) as f64 / be32(b, 12).max(1) as f64
                }
            })
            .unwrap_or(0.0);
        let mut trex = std::collections::HashMap::new();
        if let Some(mvex) = child(moov, b"mvex") {
            self.fragmented = true;
            for (t, b) in boxes(mvex) {
                if &t == b"trex" && b.len() >= 24 {
                    trex.insert(be32(b, 4), (be32(b, 12), be32(b, 16), be32(b, 20)));
                }
                if &t == b"mehd" {
                    let d = if b[0] == 1 {
                        be64(b, 4)
                    } else {
                        be32(b, 4) as u64
                    };
                    if let Some(mv) = mvhd {
                        duration = d as f64
                            / if mv[0] == 1 {
                                be32(mv, 20)
                            } else {
                                be32(mv, 12)
                            }
                            .max(1) as f64;
                    }
                }
            }
        }
        for (t, trak) in boxes(moov) {
            if &t != b"trak" {
                continue;
            }
            let Some(tkhd) = child(trak, b"tkhd") else {
                continue;
            };
            let id = if tkhd[0] == 1 {
                be32(tkhd, 20)
            } else {
                be32(tkhd, 12)
            };
            let Some(mdia) = child(trak, b"mdia") else {
                continue;
            };
            let Some(mdhd) = child(mdia, b"mdhd") else {
                continue;
            };
            let timescale = if mdhd[0] == 1 {
                be32(mdhd, 20)
            } else {
                be32(mdhd, 12)
            }
            .max(1);
            let handler = child(mdia, b"hdlr")
                .map(|h| [h[8], h[9], h[10], h[11]])
                .unwrap_or([0; 4]);
            let lang = {
                let off = if mdhd[0] == 1 { 32 } else { 20 };
                let l = be16(mdhd, off);
                let c = |s: u16| (((l >> s) & 0x1f) as u8 + 0x60) as char;
                if l == 0 || l == 0x7fff {
                    String::new()
                } else {
                    [c(10), c(5), c(0)].iter().collect()
                }
            };
            let Some(stbl) = child(mdia, b"minf").and_then(|m| child(m, b"stbl")) else {
                continue;
            };
            let Some(stsd) = child(stbl, b"stsd") else {
                continue;
            };
            let entries = boxes(&stsd[8..]);
            let Some((fmt, entry)) = entries.first().copied() else {
                continue;
            };
            let shift = child(trak, b"edts")
                .and_then(|e| child(e, b"elst"))
                .and_then(|e| {
                    let n = be32(e, 4);
                    let mut p = 8;
                    for _ in 0..n {
                        let (_dur, mt) = if e[0] == 1 {
                            (be64(e, p), be64(e, p + 8) as i64)
                        } else {
                            (be32(e, p) as u64, be32(e, p + 4) as i32 as i64)
                        };
                        p += if e[0] == 1 { 20 } else { 12 };
                        if mt >= 0 {
                            return Some(mt);
                        }
                    }
                    None
                });
            let kind = match &handler {
                b"vide" if self.info.video.is_none() => {
                    let (w, h) = if entry.len() >= 28 {
                        (be16(entry, 24) as u32, be16(entry, 26) as u32)
                    } else {
                        (0, 0)
                    };
                    let kids = if entry.len() > 78 {
                        &entry[78..]
                    } else {
                        &[][..]
                    };
                    let codec = match &fmt {
                        b"avc1" | b"avc3" => VCodec::H264 {
                            avcc: child(kids, b"avcC").map(|b| b.to_vec()).unwrap_or_default(),
                        },
                        b"hvc1" | b"hev1" => VCodec::Hevc,
                        o => VCodec::Other(String::from_utf8_lossy(o).into()),
                    };
                    let sar = child(kids, b"pasp")
                        .filter(|p| p.len() >= 8)
                        .map(|p| be32(p, 0) as f32 / be32(p, 4).max(1) as f32)
                        .unwrap_or(1.0);
                    self.info.video = Some(VideoTrack {
                        codec,
                        width: w,
                        height: h,
                        sar: if sar > 0.1 { sar } else { 1.0 },
                    });
                    Kind::Video
                }
                b"soun" => {
                    let version = if entry.len() >= 10 { be16(entry, 8) } else { 0 };
                    let channels = if entry.len() >= 18 {
                        be16(entry, 16) as u32
                    } else {
                        2
                    };
                    let rate = if entry.len() >= 28 {
                        be32(entry, 24) >> 16
                    } else {
                        48000
                    };
                    let kids_at = 28
                        + match version {
                            1 => 16,
                            2 => 36,
                            _ => 0,
                        };
                    let kids = if entry.len() > kids_at {
                        &entry[kids_at..]
                    } else {
                        &[][..]
                    };
                    let codec = match &fmt {
                        b"mp4a" => match child(kids, b"esds").and_then(parse_esds) {
                            Some((0x40 | 0x66 | 0x67 | 0x68, asc)) => ACodec::Aac { asc },
                            Some((0x69 | 0x6b, _)) => ACodec::Mp3,
                            Some((oti, _)) => ACodec::Other(format!("mp4a/{oti:x}")),
                            None => ACodec::Aac { asc: Vec::new() },
                        },
                        b".mp3" => ACodec::Mp3,
                        b"ac-3" => ACodec::Ac3,
                        b"ec-3" => ACodec::Eac3,
                        b"Opus" => ACodec::Opus,
                        b"fLaC" => ACodec::Flac,
                        b"dtsc" | b"dtsh" | b"dtsl" => ACodec::Dts,
                        o => ACodec::Other(String::from_utf8_lossy(o).into()),
                    };
                    let n = self.info.audio.len();
                    self.info.audio.push(AudioTrack {
                        codec,
                        rate: if rate > 0 { rate } else { 48000 },
                        channels,
                        lang,
                        name: String::new(),
                        default: n == 0,
                    });
                    Kind::Audio(n)
                }
                _ => continue,
            };
            let samples = if self.fragmented {
                Vec::new()
            } else {
                sample_table(stbl)
            };
            let (dd, ds, df) = trex.get(&id).copied().unwrap_or((0, 0, 0));
            self.traks.push(Trak {
                id,
                kind,
                timescale,
                shift: shift.unwrap_or(0),
                samples,
                next: 0,
                def_dur: dd,
                def_size: ds,
                def_flags: df,
            });
        }
        if duration <= 0.0 {
            duration = self
                .traks
                .iter()
                .filter_map(|t| t.samples.last().map(|s| s.dts as f64 / t.timescale as f64))
                .fold(0.0, f64::max);
        }
        self.info.duration = duration;
        if self.traks.is_empty() {
            return Err("MP4 ohne Bild und Ton".into());
        }
        Ok(())
    }

    fn pts(&self, ti: usize, s: &Sample) -> f64 {
        let t = &self.traks[ti];
        (s.dts + s.cts - t.shift) as f64 / t.timescale as f64
    }

    fn read_sample(&mut self, ti: usize, s: Sample) -> Result<Option<Packet>, String> {
        let data = if let Some((start, buf)) = &self.mdat {
            let a = s.offset.checked_sub(*start).ok_or("Sample vor mdat")? as usize;
            buf.get(a..a + s.size as usize)
                .ok_or("Sample außerhalb mdat")?
                .to_vec()
        } else {
            self.src
                .seek(SeekFrom::Start(s.offset))
                .map_err(|e| e.to_string())?;
            let mut d = vec![0u8; s.size as usize];
            self.src.read_exact(&mut d).map_err(|e| e.to_string())?;
            d
        };
        let pts = self.pts(ti, &s);
        Ok(Some(Packet {
            kind: self.traks[ti].kind,
            pts,
            dur: 0.0,
            key: s.key,
            data,
        }))
    }

    /// Reads the next moof (+ its mdat) into `pending`.
    fn next_fragment(&mut self) -> Result<bool, String> {
        loop {
            let Some((typ, size, hl)) = read_header(&mut self.src, self.pos)? else {
                return Ok(false);
            };
            let start = self.pos;
            if size == 0 {
                return Ok(false);
            }
            match &typ {
                b"moof" => {
                    let mut body = vec![0u8; (size - hl) as usize];
                    self.src.read_exact(&mut body).map_err(|e| e.to_string())?;
                    self.pos = start + size;
                    let mut list = self.parse_moof(&body, start);
                    list.sort_by_key(|(_, s)| s.offset);
                    if !self.seekable {
                        // stream: pull the following mdat into memory
                        if let Some(first) = list.first() {
                            if let Some((t2, size2, hl2)) = read_header(&mut self.src, self.pos)? {
                                if &t2 == b"mdat" {
                                    let mut d = vec![0u8; (size2 - hl2) as usize];
                                    self.src.read_exact(&mut d).map_err(|e| e.to_string())?;
                                    self.mdat = Some((self.pos + hl2, d));
                                    self.pos += size2;
                                }
                            }
                            let _ = first;
                        }
                    }
                    self.pending.extend(list);
                    return Ok(true);
                }
                _ => {
                    self.pos = start + size;
                }
            }
        }
    }

    fn parse_moof(&mut self, moof: &[u8], moof_start: u64) -> Vec<(usize, Sample)> {
        let mut out = Vec::new();
        for (t, traf) in boxes(moof) {
            if &t != b"traf" {
                continue;
            }
            let Some(tfhd) = child(traf, b"tfhd") else {
                continue;
            };
            let flags = be32(tfhd, 0) & 0xff_ffff;
            let id = be32(tfhd, 4);
            let Some(ti) = self.traks.iter().position(|t| t.id == id) else {
                continue;
            };
            let tr = &self.traks[ti];
            let mut p = 8;
            let mut base = moof_start;
            if flags & 1 != 0 {
                base = be64(tfhd, p);
                p += 8;
            }
            if flags & 2 != 0 {
                p += 4;
            }
            let mut def_dur = tr.def_dur;
            let mut def_size = tr.def_size;
            let mut def_flags = tr.def_flags;
            if flags & 8 != 0 {
                def_dur = be32(tfhd, p);
                p += 4;
            }
            if flags & 0x10 != 0 {
                def_size = be32(tfhd, p);
                p += 4;
            }
            if flags & 0x20 != 0 {
                def_flags = be32(tfhd, p);
            }
            let mut dts = child(traf, b"tfdt")
                .map(|b| {
                    if b[0] == 1 {
                        be64(b, 4) as i64
                    } else {
                        be32(b, 4) as i64
                    }
                })
                .unwrap_or(0);
            let mut data_end = base;
            for (tt, trun) in boxes(traf) {
                if &tt != b"trun" {
                    continue;
                }
                let v = trun[0];
                let f = be32(trun, 0) & 0xff_ffff;
                let n = be32(trun, 4);
                let mut q = 8;
                let mut off = data_end;
                if f & 1 != 0 {
                    off = (base as i64 + be32(trun, q) as i32 as i64) as u64;
                    q += 4;
                }
                let mut first_flags = None;
                if f & 4 != 0 {
                    first_flags = Some(be32(trun, q));
                    q += 4;
                }
                for i in 0..n {
                    let mut dur = def_dur;
                    let mut size = def_size;
                    let mut sflags = if i == 0 {
                        first_flags.unwrap_or(def_flags)
                    } else {
                        def_flags
                    };
                    let mut cts = 0i64;
                    if f & 0x100 != 0 {
                        dur = be32(trun, q);
                        q += 4;
                    }
                    if f & 0x200 != 0 {
                        size = be32(trun, q);
                        q += 4;
                    }
                    if f & 0x400 != 0 {
                        sflags = be32(trun, q);
                        q += 4;
                    }
                    if f & 0x800 != 0 {
                        cts = if v == 0 {
                            be32(trun, q) as i64
                        } else {
                            be32(trun, q) as i32 as i64
                        };
                        q += 4;
                    }
                    if q > trun.len() {
                        break;
                    }
                    let key = matches!(tr.kind, Kind::Audio(_)) || sflags & 0x0001_0000 == 0;
                    out.push((
                        ti,
                        Sample {
                            offset: off,
                            size,
                            dts,
                            cts,
                            key,
                        },
                    ));
                    off += size as u64;
                    dts += dur as i64;
                }
                data_end = off;
            }
        }
        out
    }
}

fn read_header(src: &mut Box<dyn Source>, pos: u64) -> Result<Option<([u8; 4], u64, u64)>, String> {
    src.seek(SeekFrom::Start(pos)).map_err(|e| e.to_string())?;
    let mut h = [0u8; 8];
    match src.read_exact(&mut h) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.to_string()),
    }
    let mut size = be32(&h, 0) as u64;
    let typ: [u8; 4] = h[4..8].try_into().unwrap();
    let mut hl = 8;
    if size == 1 {
        let mut e = [0u8; 8];
        src.read_exact(&mut e).map_err(|e| e.to_string())?;
        size = u64::from_be_bytes(e);
        hl = 16;
    }
    if size != 0 && size < hl {
        return Err("kaputte MP4-Box".into());
    }
    Ok(Some((typ, size, hl)))
}

fn parse_sidx(b: &[u8], end_of_box: u64) -> Vec<Seg> {
    let v = b[0];
    let timescale = be32(b, 8).max(1) as f64;
    let (mut t, first, mut p) = if v == 0 {
        (be32(b, 12) as u64, be32(b, 16) as u64, 20)
    } else {
        (be64(b, 12), be64(b, 20), 28)
    };
    let count = be16(b, p + 2) as usize;
    p += 4;
    let mut off = end_of_box + first;
    let mut segs = Vec::new();
    for _ in 0..count {
        if p + 12 > b.len() {
            break;
        }
        let size = (be32(b, p) & 0x7fff_ffff) as u64;
        let dur = be32(b, p + 4) as u64;
        segs.push(Seg {
            offset: off,
            time: t as f64 / timescale,
        });
        off += size;
        t += dur;
        p += 12;
    }
    segs
}

/// ES descriptor -> (objectTypeIndication, AudioSpecificConfig)
fn parse_esds(b: &[u8]) -> Option<(u8, Vec<u8>)> {
    let mut p = 4;
    let desc = |p: &mut usize| -> Option<(u8, usize)> {
        let tag = *b.get(*p)?;
        *p += 1;
        let mut len = 0usize;
        for _ in 0..4 {
            let c = *b.get(*p)?;
            *p += 1;
            len = (len << 7) | (c & 0x7f) as usize;
            if c & 0x80 == 0 {
                break;
            }
        }
        Some((tag, len))
    };
    let (tag, _) = desc(&mut p)?;
    if tag != 3 {
        return None;
    }
    p += 2;
    let flags = *b.get(p)?;
    p += 1;
    if flags & 0x80 != 0 {
        p += 2;
    }
    if flags & 0x40 != 0 {
        let l = *b.get(p)? as usize;
        p += 1 + l;
    }
    if flags & 0x20 != 0 {
        p += 2;
    }
    let (tag, _) = desc(&mut p)?;
    if tag != 4 {
        return None;
    }
    let oti = *b.get(p)?;
    p += 13;
    let mut asc = Vec::new();
    if let Some((5, len)) = desc(&mut p) {
        asc = b.get(p..p + len)?.to_vec();
    }
    Some((oti, asc))
}

fn sample_table(stbl: &[u8]) -> Vec<Sample> {
    let get = |t: &[u8; 4]| child(stbl, t);
    // sizes
    let sizes: Vec<u32> = match get(b"stsz") {
        Some(b) => {
            let fixed = be32(b, 4);
            let n = be32(b, 8) as usize;
            if fixed != 0 {
                vec![fixed; n]
            } else {
                (0..n)
                    .filter(|i| 12 + i * 4 + 4 <= b.len())
                    .map(|i| be32(b, 12 + i * 4))
                    .collect()
            }
        }
        None => match get(b"stz2") {
            Some(b) => {
                let fs = b[7];
                let n = be32(b, 8) as usize;
                (0..n)
                    .map(|i| match fs {
                        4 => ((b[12 + i / 2] >> if i % 2 == 0 { 4 } else { 0 }) & 15) as u32,
                        8 => b[12 + i] as u32,
                        _ => be16(b, 12 + i * 2) as u32,
                    })
                    .collect()
            }
            None => return Vec::new(),
        },
    };
    let n = sizes.len();
    // chunk offsets
    let chunks: Vec<u64> = if let Some(b) = get(b"stco") {
        (0..be32(b, 4) as usize)
            .map(|i| be32(b, 8 + i * 4) as u64)
            .collect()
    } else if let Some(b) = get(b"co64") {
        (0..be32(b, 4) as usize)
            .map(|i| be64(b, 8 + i * 8))
            .collect()
    } else {
        return Vec::new();
    };
    // sample-to-chunk
    let stsc: Vec<(u32, u32)> = get(b"stsc")
        .map(|b| {
            (0..be32(b, 4) as usize)
                .map(|i| (be32(b, 8 + i * 12), be32(b, 12 + i * 12)))
                .collect()
        })
        .unwrap_or_default();
    let mut offsets = Vec::with_capacity(n);
    let mut si = 0usize;
    for (ci, &co) in chunks.iter().enumerate() {
        let chunk_no = ci as u32 + 1;
        let per = stsc
            .iter()
            .rev()
            .find(|(first, _)| *first <= chunk_no)
            .map(|x| x.1)
            .unwrap_or(1);
        let mut off = co;
        for _ in 0..per {
            if si >= n {
                break;
            }
            offsets.push(off);
            off += sizes[si] as u64;
            si += 1;
        }
    }
    // decode times
    let mut dts = Vec::with_capacity(n);
    if let Some(b) = get(b"stts") {
        let mut t = 0i64;
        for i in 0..be32(b, 4) as usize {
            let (count, delta) = (be32(b, 8 + i * 8), be32(b, 12 + i * 8));
            for _ in 0..count {
                dts.push(t);
                t += delta as i64;
            }
        }
    }
    let mut cts = vec![0i64; n];
    if let Some(b) = get(b"ctts") {
        let mut k = 0;
        for i in 0..be32(b, 4) as usize {
            let (count, off) = (be32(b, 8 + i * 8), be32(b, 12 + i * 8) as i32 as i64);
            for _ in 0..count {
                if k < n {
                    cts[k] = off;
                }
                k += 1;
            }
        }
    }
    let sync: Option<std::collections::HashSet<u32>> = get(b"stss").map(|b| {
        (0..be32(b, 4) as usize)
            .map(|i| be32(b, 8 + i * 4))
            .collect()
    });
    (0..n.min(offsets.len()).min(dts.len()))
        .map(|i| Sample {
            offset: offsets[i],
            size: sizes[i],
            dts: dts[i],
            cts: cts[i],
            key: sync
                .as_ref()
                .map(|s| s.contains(&(i as u32 + 1)))
                .unwrap_or(true),
        })
        .collect()
}

impl Demuxer for Mp4 {
    fn info(&self) -> &StreamInfo {
        &self.info
    }

    fn next(&mut self) -> Result<Option<Packet>, String> {
        if self.fragmented {
            loop {
                if let Some((ti, s)) = self.pending.pop_front() {
                    return self.read_sample(ti, s);
                }
                self.mdat = None;
                if !self.next_fragment()? {
                    return Ok(None);
                }
            }
        }
        // classic: the sample with the lowest file offset next (keeps reads sequential)
        let mut best: Option<(usize, u64)> = None;
        for (i, t) in self.traks.iter().enumerate() {
            if let Some(s) = t.samples.get(t.next) {
                if best.map(|b| s.offset < b.1).unwrap_or(true) {
                    best = Some((i, s.offset));
                }
            }
        }
        let Some((ti, _)) = best else { return Ok(None) };
        let s = self.traks[ti].samples[self.traks[ti].next];
        self.traks[ti].next += 1;
        self.read_sample(ti, s)
    }

    fn seek(&mut self, t: f64) -> Result<f64, String> {
        if !self.seekable {
            return Err("nicht springbar".into());
        }
        if self.fragmented {
            if self.segs.is_empty() {
                return Err("kein Index".into());
            }
            let i = self
                .segs
                .iter()
                .rposition(|s| s.time <= t + 0.01)
                .unwrap_or(0);
            self.pos = self.segs[i].offset;
            self.pending.clear();
            self.mdat = None;
            return Ok(self.segs[i].time);
        }
        // keyframe of the video track at or before t; other tracks follow its time
        let vi = self.traks.iter().position(|t| t.kind == Kind::Video);
        let target = if let Some(vi) = vi {
            let tr = &self.traks[vi];
            let ts = tr.timescale as f64;
            let idx = tr
                .samples
                .iter()
                .rposition(|s| s.key && ((s.dts + s.cts - tr.shift) as f64 / ts) <= t + 0.01)
                .unwrap_or(0);
            self.traks[vi].next = idx;
            let s = self.traks[vi].samples.get(idx).copied();
            s.map(|s| {
                (s.dts + s.cts - self.traks[vi].shift) as f64 / self.traks[vi].timescale as f64
            })
            .unwrap_or(0.0)
        } else {
            t
        };
        for (i, tr) in self.traks.iter_mut().enumerate() {
            if Some(i) == vi {
                continue;
            }
            let ts = tr.timescale as f64;
            let shift = tr.shift;
            tr.next = tr
                .samples
                .iter()
                .position(|s| ((s.dts + s.cts - shift) as f64 / ts) >= target - 0.05)
                .unwrap_or(tr.samples.len());
        }
        Ok(target.max(0.0))
    }

    fn can_seek(&self) -> bool {
        self.seekable && (!self.fragmented || !self.segs.is_empty())
    }
}
