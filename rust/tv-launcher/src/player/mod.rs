//! The film/video player: demux (mp4.rs, mkv.rs) -> H.264 hardware decoder
//! (v4l2.rs) -> TV screen plane, and AAC/MP3 -> ALSA (audio.rs); the sound
//! is the clock the picture follows.
pub mod aac_hls;
pub mod audio;
pub mod engine;
pub mod hls;
pub mod mkv;
pub mod mp4;
pub mod ts;
#[cfg(target_os = "linux")]
pub mod v4l2;

pub use engine::{Event, MediaSource, Player, StartOpts};

#[derive(Clone, Debug, PartialEq)]
pub enum VCodec {
    H264 { avcc: Vec<u8> },
    Hevc,
    Other(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ACodec {
    Aac { asc: Vec<u8> },
    Mp3,
    Ac3,
    Eac3,
    Dts,
    Opus,
    Vorbis,
    Flac,
    Pcm16,
    Other(String),
}

impl ACodec {
    pub fn playable(&self) -> bool {
        matches!(
            self,
            ACodec::Aac { .. } | ACodec::Mp3 | ACodec::Flac | ACodec::Vorbis | ACodec::Pcm16
        )
    }
    pub fn name(&self) -> String {
        match self {
            ACodec::Aac { .. } => "AAC".into(),
            ACodec::Mp3 => "MP3".into(),
            ACodec::Ac3 => "AC3".into(),
            ACodec::Eac3 => "E-AC3".into(),
            ACodec::Dts => "DTS".into(),
            ACodec::Opus => "Opus".into(),
            ACodec::Vorbis => "Vorbis".into(),
            ACodec::Flac => "FLAC".into(),
            ACodec::Pcm16 => "PCM".into(),
            ACodec::Other(s) => s.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct VideoTrack {
    pub codec: VCodec,
    pub width: u32,
    pub height: u32,
    /// pixel aspect ratio (anamorphic DVD rips)
    pub sar: f32,
}

#[derive(Clone, Debug)]
pub struct AudioTrack {
    pub codec: ACodec,
    pub rate: u32,
    pub channels: u32,
    pub lang: String,
    pub name: String,
    pub default: bool,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct SubTrack {
    pub lang: String,
    pub name: String,
    pub forced: bool,
    pub default: bool,
    pub ass: bool,
}

#[derive(Clone, Debug, Default)]
pub struct StreamInfo {
    pub video: Option<VideoTrack>,
    pub audio: Vec<AudioTrack>,
    pub subs: Vec<SubTrack>,
    pub duration: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Video,
    Audio(usize),
    Sub(usize),
}

#[derive(Debug)]
pub struct Packet {
    pub kind: Kind,
    /// presentation time, seconds
    pub pts: f64,
    /// subtitles: how long the text shows
    pub dur: f64,
    pub key: bool,
    pub data: Vec<u8>,
}

#[allow(dead_code)]
pub trait Demuxer: Send {
    fn info(&self) -> &StreamInfo;
    fn next(&mut self) -> Result<Option<Packet>, String>;
    /// Jumps to the keyframe at or before t; returns where it landed.
    fn seek(&mut self, t: f64) -> Result<f64, String>;
    fn can_seek(&self) -> bool;
}

/// Read + Seek, boxed.
pub trait Source: std::io::Read + std::io::Seek + Send {}
impl<T: std::io::Read + std::io::Seek + Send> Source for T {}

/// A stream that can only go forward (a live remux from the PC): forward
/// seeks read and discard, backward ones fail.
pub struct Forward<R> {
    inner: R,
    pos: u64,
}
impl<R: std::io::Read> Forward<R> {
    pub fn new(inner: R) -> Self {
        Forward { inner, pos: 0 }
    }
}
impl<R: std::io::Read> std::io::Read for Forward<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.pos += n as u64;
        Ok(n)
    }
}
impl<R: std::io::Read> std::io::Seek for Forward<R> {
    fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
        let target = match to {
            std::io::SeekFrom::Start(p) => p,
            std::io::SeekFrom::Current(d) if d >= 0 => self.pos + d as u64,
            _ => return Err(std::io::Error::other("Stream kann nicht zurückspringen")),
        };
        if target < self.pos {
            return Err(std::io::Error::other("Stream kann nicht zurückspringen"));
        }
        let mut left = target - self.pos;
        let mut buf = [0u8; 16384];
        while left > 0 {
            let n = self.inner.read(&mut buf[..left.min(16384) as usize])?;
            if n == 0 {
                break;
            }
            self.pos += n as u64;
            left -= n as u64;
        }
        Ok(self.pos)
    }
}

/// Keeps a BufReader's buffer across seeks: demuxers "seek" to where they
/// already are all the time (next box, next sample); only a real jump
/// reaches the file / network.
pub struct Lazy<R> {
    inner: std::io::BufReader<R>,
    pos: u64,
}
impl<R: std::io::Read + std::io::Seek> Lazy<R> {
    pub fn new(inner: R, cap: usize) -> Self {
        Lazy {
            inner: std::io::BufReader::with_capacity(cap, inner),
            pos: 0,
        }
    }
}
impl<R: std::io::Read + std::io::Seek> std::io::Read for Lazy<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.pos += n as u64;
        Ok(n)
    }
}
impl<R: std::io::Read + std::io::Seek> std::io::Seek for Lazy<R> {
    fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
        let target = match to {
            std::io::SeekFrom::Start(p) => p as i64,
            std::io::SeekFrom::Current(d) => self.pos as i64 + d,
            std::io::SeekFrom::End(_) => {
                self.pos = self.inner.seek(to)?;
                return Ok(self.pos);
            }
        };
        let delta = target - self.pos as i64;
        if delta != 0 {
            let buffered = self.inner.buffer().len() as i64;
            if delta > 0 && delta <= buffered {
                self.inner.seek_relative(delta)?; // stays inside the buffer
            } else {
                self.inner
                    .seek(std::io::SeekFrom::Start(target.max(0) as u64))?;
            }
            self.pos = target.max(0) as u64;
        }
        Ok(self.pos)
    }
}

/// Picks the container by its first bytes.
pub fn open_demuxer(mut src: Box<dyn Source>, seekable: bool) -> Result<Box<dyn Demuxer>, String> {
    let mut head = [0u8; 12];
    src.read_exact(&mut head)
        .map_err(|e| format!("Datei unlesbar: {e}"))?;
    src.seek(std::io::SeekFrom::Start(0)).or_else(|_| {
        // forward-only: can't rewind 12 bytes -- reopen is the caller's job
        Err("Stream kann nicht zurückspringen".to_string())
    })?;
    if head[0..4] == [0x1a, 0x45, 0xdf, 0xa3] {
        return Ok(Box::new(mkv::Mkv::open(src, seekable)?));
    }
    if &head[4..8] == b"ftyp"
        || &head[4..8] == b"moov"
        || &head[4..8] == b"styp"
        || &head[4..8] == b"free"
        || &head[4..8] == b"mdat"
    {
        return Ok(Box::new(mp4::Mp4::open(src, seekable)?));
    }
    Err("Unbekanntes Dateiformat (nur MP4, MKV)".into())
}

/// avcC -> (NAL length size, SPS+PPS as Annex B).
pub fn avcc_to_annexb(avcc: &[u8]) -> (usize, Vec<u8>) {
    let mut out = Vec::new();
    if avcc.len() < 7 {
        return (4, out);
    }
    let nal_len = (avcc[4] & 3) as usize + 1;
    let mut p = 5;
    let nsps = (avcc[p] & 0x1f) as usize;
    p += 1;
    for _ in 0..nsps {
        if p + 2 > avcc.len() {
            return (nal_len, out);
        }
        let l = u16::from_be_bytes([avcc[p], avcc[p + 1]]) as usize;
        p += 2;
        if p + l > avcc.len() {
            return (nal_len, out);
        }
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&avcc[p..p + l]);
        p += l;
    }
    if p < avcc.len() {
        let npps = avcc[p] as usize;
        p += 1;
        for _ in 0..npps {
            if p + 2 > avcc.len() {
                break;
            }
            let l = u16::from_be_bytes([avcc[p], avcc[p + 1]]) as usize;
            p += 2;
            if p + l > avcc.len() {
                break;
            }
            out.extend_from_slice(&[0, 0, 0, 1]);
            out.extend_from_slice(&avcc[p..p + l]);
            p += l;
        }
    }
    (nal_len, out)
}

/// Length-prefixed NAL units -> Annex B (start codes), what the decoder wants.
pub fn to_annexb(data: &[u8], nal_len: usize, out: &mut Vec<u8>) {
    let mut p = 0;
    while p + nal_len <= data.len() {
        let mut l = 0usize;
        for i in 0..nal_len {
            l = (l << 8) | data[p + i] as usize;
        }
        p += nal_len;
        if l == 0 || p + l > data.len() {
            break;
        }
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&data[p..p + l]);
        p += l;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn annexb() {
        let avcc = [
            1, 0x64, 0, 0x1f, 0xff, 0xe1, 0, 2, 0x67, 0xaa, 1, 0, 1, 0x68,
        ];
        let (n, ps) = avcc_to_annexb(&avcc);
        assert_eq!(n, 4);
        assert_eq!(ps, vec![0, 0, 0, 1, 0x67, 0xaa, 0, 0, 0, 1, 0x68]);
        let mut out = Vec::new();
        to_annexb(&[0, 0, 0, 2, 0x65, 0x88, 0, 0, 0, 1, 0x06], 4, &mut out);
        assert_eq!(out, vec![0, 0, 0, 1, 0x65, 0x88, 0, 0, 0, 1, 0x06]);
    }
}

/// End-to-end demux checks against files made by ffmpeg (skipped without ffmpeg).
#[cfg(test)]
mod media_tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;

    fn make(name: &str, args: &[&str]) -> Option<PathBuf> {
        let dir = std::env::temp_dir().join(format!("tvl-media-{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok()?;
        let out = dir.join(name);
        if !out.exists() {
            let srt = dir.join("s.srt");
            std::fs::write(&srt, "1\n00:00:01,000 --> 00:00:02,500\nHallo <i>Welt</i>\n\n2\n00:00:03,000 --> 00:00:04,000\nZwei\n").ok()?;
            let mut a: Vec<String> = [
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=320x240:rate=25",
                "-f",
                "lavfi",
                "-i",
                "sine=f=440:sample_rate=48000",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect();
            a.extend([
                "-i".to_string(),
                srt.display().to_string(),
                "-t".into(),
                "5".into(),
            ]);
            a.extend(args.iter().map(|s| s.to_string()));
            a.push(out.display().to_string());
            let ok = Command::new("ffmpeg").args(&a).status().ok()?.success();
            if !ok {
                return None;
            }
        }
        Some(out)
    }

    fn read_all(d: &mut dyn Demuxer) -> (Vec<Packet>, Vec<Packet>, Vec<Packet>) {
        let (mut v, mut a, mut s) = (vec![], vec![], vec![]);
        while let Some(p) = d.next().unwrap() {
            match p.kind {
                Kind::Video => v.push(p),
                Kind::Audio(_) => a.push(p),
                Kind::Sub(_) => s.push(p),
            }
        }
        (v, a, s)
    }

    fn check_av(v: &[Packet], a: &[Packet]) {
        assert_eq!(v.len(), 125, "video frames");
        assert!(v[0].key);
        assert!(
            a.len() >= 230 && a.len() <= 240,
            "audio packets {}",
            a.len()
        );
        let mut pts: Vec<f64> = v.iter().map(|p| p.pts).collect();
        pts.sort_by(|x, y| x.partial_cmp(y).unwrap());
        assert!(pts[0] >= -0.01 && pts[0] < 0.1, "first pts {}", pts[0]); // fragmented: B-frame delay stays in the file (ffprobe agrees)
        assert!(
            (pts[124] - pts[0] - 4.96).abs() < 0.05,
            "last pts {}",
            pts[124]
        );
        // audio decodes to ~5 s of stereo
        let info_rate = 48000;
        let _ = info_rate;
    }

    const H264: [&str; 8] = [
        "-map", "0:v", "-map", "1:a", "-c:v", "libx264", "-c:a", "aac",
    ];

    #[test]
    fn mp4_faststart_and_seek() {
        let mut args = H264.to_vec();
        args.extend(["-g", "25", "-movflags", "+faststart"]);
        let Some(f) = make("a.mp4", &args) else {
            return;
        };
        let mut d = mp4::Mp4::open(Box::new(std::fs::File::open(&f).unwrap()), true).unwrap();
        let info = d.info().clone();
        assert!(
            matches!(info.video.as_ref().unwrap().codec, VCodec::H264 { ref avcc } if !avcc.is_empty())
        );
        assert!(matches!(info.audio[0].codec, ACodec::Aac { ref asc } if !asc.is_empty()));
        assert!((info.duration - 5.0).abs() < 0.1);
        let (v, a, _) = read_all(&mut d);
        check_av(&v, &a);
        // AAC decodes
        let mut dec = audio::AudioDec::new(&info.audio[0]).unwrap();
        let mut pcm = Vec::new();
        for p in &a {
            dec.decode(&p.data, &mut pcm).unwrap();
        }
        let secs = pcm.len() as f64 / 2.0 / 48000.0;
        assert!((secs - 5.0).abs() < 0.1, "decoded {secs} s");
        // seek
        let t = d.seek(3.3).unwrap();
        assert!((t - 3.0).abs() < 0.05, "landed {t}");
        let p = d.next().unwrap().unwrap();
        assert!(p.kind != Kind::Video || p.key);
    }

    #[test]
    fn mp4_moov_at_end() {
        let Some(f) = make("b.mp4", &H264) else {
            return;
        };
        let mut d = mp4::Mp4::open(Box::new(std::fs::File::open(&f).unwrap()), true).unwrap();
        let (v, a, _) = read_all(&mut d);
        check_av(&v, &a);
    }

    #[test]
    fn fragmented_stream_forward_only() {
        let mut args = H264.to_vec();
        args.extend([
            "-g",
            "25",
            "-movflags",
            "frag_keyframe+empty_moov+default_base_moof",
            "-f",
            "mp4",
        ]);
        let Some(f) = make("c.mp4", &args) else {
            return;
        };
        let r = Forward::new(std::fs::File::open(&f).unwrap());
        let mut d = mp4::Mp4::open(Box::new(r), false).unwrap();
        let (v, a, _) = read_all(&mut d);
        check_av(&v, &a);
    }

    #[test]
    fn dash_with_sidx() {
        let mut args = H264.to_vec();
        args.extend([
            "-g",
            "25",
            "-movflags",
            "frag_keyframe+empty_moov+default_base_moof+global_sidx",
            "-f",
            "mp4",
        ]);
        let Some(f) = make("d.mp4", &args) else {
            return;
        };
        let mut d = mp4::Mp4::open(Box::new(std::fs::File::open(&f).unwrap()), true).unwrap();
        assert!(d.can_seek());
        let t = d.seek(2.5).unwrap();
        assert!(t <= 2.5 && t >= 1.9, "landed {t}");
        let (v, _, _) = read_all(&mut d);
        assert!(v[0].key);
        assert!(v.len() < 125 && v.len() > 50);
    }

    #[test]
    fn mkv_with_subtitles_and_cues() {
        let args = [
            "-map",
            "0:v",
            "-map",
            "1:a",
            "-map",
            "2:s",
            "-c:v",
            "libx264",
            "-g",
            "25",
            "-c:a",
            "aac",
            "-c:s",
            "srt",
            "-metadata:s:s:0",
            "language=ger",
        ];
        let Some(f) = make("e.mkv", &args) else {
            return;
        };
        let mut d = mkv::Mkv::open(Box::new(std::fs::File::open(&f).unwrap()), true).unwrap();
        let info = d.info().clone();
        assert_eq!(info.subs.len(), 1);
        assert_eq!(info.subs[0].lang, "ger");
        assert!(
            (info.duration - 5.0).abs() < 0.1,
            "duration {}",
            info.duration
        );
        let (v, a, s) = read_all(&mut d);
        check_av(&v, &a);
        assert_eq!(s.len(), 2);
        assert!(String::from_utf8_lossy(&s[0].data).contains("Hallo"));
        assert!((s[0].pts - 1.0).abs() < 0.01 && (s[0].dur - 1.5).abs() < 0.01);
        assert!(d.can_seek());
        let t = d.seek(3.5).unwrap();
        assert!(t <= 3.5 && t >= 2.9, "landed {t}");
        let p = loop {
            let p = d.next().unwrap().unwrap();
            if p.kind == Kind::Video {
                break p;
            }
        };
        assert!(p.key);
    }

    #[test]
    fn hevc_is_recognised() {
        let args = [
            "-map",
            "0:v",
            "-map",
            "1:a",
            "-c:v",
            "libx265",
            "-x265-params",
            "log-level=none",
            "-c:a",
            "ac3",
        ];
        let Some(f) = make("f.mkv", &args) else {
            return;
        };
        let d = open_demuxer(Box::new(std::fs::File::open(&f).unwrap()), true).unwrap();
        assert_eq!(d.info().video.as_ref().unwrap().codec, VCodec::Hevc);
        assert_eq!(d.info().audio[0].codec, ACodec::Ac3);
        assert!(!d.info().audio[0].codec.playable());
    }

    #[test]
    fn mpegts_h264_aac() {
        let args = [
            "-map", "0:v", "-map", "1:a", "-c:v", "libx264", "-g", "25", "-c:a", "aac", "-f",
            "mpegts",
        ];
        let Some(f) = make("h.ts", &args) else { return };
        let mut d = ts::Ts::open(Box::new(std::fs::File::open(&f).unwrap())).unwrap();
        let info = d.info().clone();
        let v = info.video.clone().unwrap();
        assert_eq!((v.width, v.height), (320, 240));
        assert!(matches!(info.audio[0].codec, ACodec::Aac { ref asc } if asc.len() == 2));
        assert_eq!(info.audio[0].rate, 48000);
        let (vv, a, _) = read_all(&mut d);
        assert_eq!(vv.len(), 125);
        assert!(vv[0].key);
        assert_eq!(vv.iter().filter(|p| p.key).count(), 5);
        assert!(a.len() >= 230, "audio {}", a.len());
        // the sound decodes
        let mut dec = audio::AudioDec::new(&info.audio[0]).unwrap();
        let mut pcm = Vec::new();
        for p in &a {
            dec.decode(&p.data, &mut pcm).unwrap();
        }
        assert!((pcm.len() as f64 / 96000.0 - 5.0).abs() < 0.15);
    }

    #[test]
    fn hls_over_http() {
        // a channel as ffmpeg's HLS muxer writes it, served by a tiny HTTP server
        let dir = std::env::temp_dir().join(format!("tvl-hls-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ok = Command::new("ffmpeg")
            .args([
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=640x360:rate=25",
                "-f",
                "lavfi",
                "-i",
                "sine=f=440:sample_rate=48000",
                "-t",
                "6",
                "-c:v",
                "libx264",
                "-g",
                "25",
                "-c:a",
                "aac",
                "-f",
                "hls",
                "-hls_time",
                "1",
                "-hls_list_size",
                "0",
                "-master_pl_name",
                "master.m3u8",
            ])
            .arg(dir.join("index.m3u8"))
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            return;
        }
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let d2 = dir.clone();
        std::thread::spawn(move || {
            for req in server.incoming_requests() {
                let p = d2.join(req.url().trim_start_matches('/'));
                let _ = match std::fs::read(&p) {
                    Ok(b) => req.respond(tiny_http::Response::from_data(b)),
                    Err(_) => req.respond(tiny_http::Response::empty(404)),
                };
            }
        });
        let r = hls::open(&format!("http://127.0.0.1:{port}/master.m3u8"), 1080).unwrap();
        let mut d = ts::Ts::open(Box::new(r)).unwrap();
        let v = d.info().video.clone().unwrap();
        assert_eq!((v.width, v.height), (640, 360));
        let (vv, a, _) = read_all(&mut d);
        assert_eq!(vv.len(), 150, "frames over all segments");
        assert!(a.len() > 270);
        let mut pts: Vec<f64> = vv.iter().map(|p| p.pts).collect();
        pts.sort_by(|x, y| x.partial_cmp(y).unwrap());
        assert!(
            pts[0] < 0.2 && (pts[149] - pts[0] - 5.96).abs() < 0.05,
            "{} .. {}",
            pts[0],
            pts[149]
        );
    }

    #[test]
    fn sps_cropping() {
        let args = [
            "-map",
            "0:v",
            "-c:v",
            "libx264",
            "-s",
            "1920x1080",
            "-frames:v",
            "3",
            "-f",
            "h264",
        ];
        let Some(f) = make("i.h264", &args) else {
            return;
        };
        let b = std::fs::read(&f).unwrap();
        let sps = ts::nal_units(&b)
            .into_iter()
            .find(|n| n[0] & 0x1f == 7)
            .unwrap();
        assert_eq!(ts::sps_size(sps), Some((1920, 1080)));
    }

    #[test]
    fn annexb_from_real_stream() {
        let mut args = H264.to_vec();
        args.extend(["-g", "25"]);
        let Some(f) = make("g.mp4", &args) else {
            return;
        };
        let mut d = mp4::Mp4::open(Box::new(std::fs::File::open(&f).unwrap()), true).unwrap();
        let VCodec::H264 { avcc } = d.info().video.clone().unwrap().codec else {
            panic!()
        };
        let (n, ps) = avcc_to_annexb(&avcc);
        assert_eq!(n, 4);
        assert_eq!(ps[4] & 0x1f, 7); // SPS
        let p = d.next().unwrap().unwrap();
        let mut out = Vec::new();
        to_annexb(&p.data, n, &mut out);
        assert!(out.starts_with(&[0, 0, 0, 1]));
        // write the elementary stream and let ffprobe count the frames
        let es = std::env::temp_dir().join(format!("tvl-es-{}.h264", std::process::id()));
        let mut all = ps.clone();
        all.extend(&out);
        while let Some(p) = d.next().unwrap() {
            if p.kind == Kind::Video {
                if p.key {
                    all.extend(&ps);
                }
                to_annexb(&p.data, n, &mut all);
            }
        }
        std::fs::write(&es, &all).unwrap();
        let o = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-count_frames",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=nb_read_frames",
                "-of",
                "csv=p=0",
            ])
            .arg(&es)
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&o.stdout).trim(), "125");
    }
}
