//! Playback threads: demux -> (video decoder + display) and (audio decoder +
//! ALSA). The audio output is the master clock; the picture waits for it.
use super::*;
use crate::app::{KmsHandle, Msg};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicUsize, Ordering::*};
use std::sync::mpsc::{sync_channel, Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub enum MediaSource {
    File(std::path::PathBuf),
    /// A seekable file over HTTP (Range requests).
    Http(String),
    /// The PC server's live remux: the URL for (start time, audio track);
    /// seeking or switching the sound restarts it.
    Live {
        url: Arc<dyn Fn(f64, usize) -> String + Send + Sync>,
        keyframe: Option<Arc<dyn Fn(f64) -> f64 + Send + Sync>>,
    },
    /// YouTube: picture and sound are separate files.
    Dash {
        video: String,
        audio: String,
    },
    /// A TV channel (HLS); `max_h`: highest picture height to pick.
    Hls {
        url: String,
        max_h: u32,
    },
    /// A broadcaster HLS master with separate video and packed AAC audio.
    HlsSeparate {
        page_url: String,
        max_h: u32,
    },
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub enum Event {
    Ready(StreamInfo),
    Ended,
    Error(String),
}

pub struct StartOpts {
    pub start: f64,
    pub audio: Option<usize>,
    pub sub: Option<usize>,
    pub audio_out: String,
    pub duration_hint: f64,
}

#[derive(Clone)]
pub struct Cue {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

pub struct Shared {
    /// media time in µs of what is audible / visible now
    clock: AtomicI64,
    /// added to the clock: a live stream restarted at t starts at 0
    offset: Mutex<f64>,
    pub paused: AtomicBool,
    stop: AtomicBool,
    gen: AtomicU32,
    audio_clock: AtomicBool,
    pub buffering: AtomicBool,
    pub info: Mutex<StreamInfo>,
    pub duration: Mutex<f64>,
    pub audio_sel: AtomicUsize,
    /// -1: none
    pub sub_sel: AtomicI64,
    pub subs: Mutex<Vec<Cue>>,
    /// subtitles loaded from elsewhere (the PC server's WebVTT)
    pub external_subs: Mutex<Vec<Cue>>,
    wall: Mutex<(Instant, f64)>,
    pub frames_shown: AtomicU32,
}

impl Shared {
    pub fn position(&self) -> f64 {
        *self.offset.lock().unwrap() + self.clock.load(Relaxed) as f64 / 1e6
    }
    fn set_clock(&self, t: f64) {
        self.clock.store((t * 1e6) as i64, Relaxed);
    }
    /// Without sound the picture follows the wall clock.
    fn wall_now(&self) -> f64 {
        let w = self.wall.lock().unwrap();
        if self.paused.load(Relaxed) {
            w.1
        } else {
            w.1 + w.0.elapsed().as_secs_f64()
        }
    }
    fn wall_set(&self, t: f64) {
        *self.wall.lock().unwrap() = (Instant::now(), t);
    }
    pub fn subtitle_now(&self) -> Option<String> {
        if self.sub_sel.load(Relaxed) < 0 {
            return None;
        }
        let t = self.position();
        let tl = self.clock.load(Relaxed) as f64 / 1e6;
        let ext = self.external_subs.lock().unwrap();
        if !ext.is_empty() {
            return ext
                .iter()
                .find(|c| c.start <= t && t < c.end)
                .map(|c| c.text.clone());
        }
        drop(ext);
        self.subs
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|c| c.start <= tl && tl < c.end)
            .map(|c| c.text.clone())
    }
}

enum Item {
    Pkt(Packet),
    Flush(u32),
    Eof,
}

enum Cmd {
    Seek(f64),
    Audio(usize),
}

pub struct Player {
    pub shared: Arc<Shared>,
    cmd: Sender<Cmd>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl Player {
    pub fn start(
        src: MediaSource,
        opts: StartOpts,
        tx: Sender<Msg>,
        kms: Option<KmsHandle>,
    ) -> Player {
        let shared = Arc::new(Shared {
            clock: AtomicI64::new(0),
            offset: Mutex::new(0.0),
            paused: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            gen: AtomicU32::new(0),
            audio_clock: AtomicBool::new(false),
            buffering: AtomicBool::new(true),
            info: Mutex::new(StreamInfo::default()),
            duration: Mutex::new(opts.duration_hint),
            audio_sel: AtomicUsize::new(opts.audio.unwrap_or(usize::MAX)),
            sub_sel: AtomicI64::new(opts.sub.map(|s| s as i64).unwrap_or(-2)),
            subs: Mutex::new(Vec::new()),
            external_subs: Mutex::new(Vec::new()),
            wall: Mutex::new((Instant::now(), 0.0)),
            frames_shown: AtomicU32::new(0),
        });
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();
        let (vtx, vrx) = sync_channel::<Item>(160);
        let (atx, arx) = sync_channel::<Item>(600);
        let mut threads = Vec::new();
        {
            let (sh, tx) = (shared.clone(), tx.clone());
            let out = opts.audio_out.clone();
            threads.push(std::thread::spawn(move || audio_thread(sh, arx, out, tx)));
        }
        {
            let (sh, tx) = (shared.clone(), tx.clone());
            threads.push(std::thread::spawn(move || video_thread(sh, vrx, kms, tx)));
        }
        {
            let (sh, tx) = (shared.clone(), tx);
            let start = opts.start;
            threads.push(std::thread::spawn(move || {
                if let Err(e) = demux_thread(sh.clone(), src, start, cmd_rx, vtx, atx, tx.clone()) {
                    if !sh.stop.load(Relaxed) {
                        let _ = tx.send(Msg::Player(Event::Error(e)));
                    }
                }
            }));
        }
        Player {
            shared,
            cmd: cmd_tx,
            threads,
        }
    }

    pub fn seek(&self, t: f64) {
        let d = *self.shared.duration.lock().unwrap();
        let t = if d > 0.0 {
            t.clamp(0.0, (d - 2.0).max(0.0))
        } else {
            t.max(0.0)
        };
        let _ = self.cmd.send(Cmd::Seek(t));
    }
    pub fn set_audio(&self, i: usize) {
        let _ = self.cmd.send(Cmd::Audio(i));
    }
    pub fn toggle_pause(&self) {
        let p = !self.shared.paused.load(Relaxed);
        self.set_paused(p);
    }
    pub fn set_paused(&self, p: bool) {
        let pos = self.shared.wall_now();
        self.shared.paused.store(p, Relaxed);
        self.shared.wall_set(pos);
    }
    pub fn position(&self) -> f64 {
        self.shared.position()
    }
    pub fn duration(&self) -> f64 {
        *self.shared.duration.lock().unwrap()
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.shared.stop.store(true, Relaxed);
        self.shared.paused.store(false, Relaxed);
        // audio + video hold the sound device and the decoder: wait for them.
        // The demuxer may sit in a slow network read; it ends on its own.
        let mut ts: Vec<_> = self.threads.drain(..).collect();
        let _demux = ts.pop();
        for t in ts {
            let _ = t.join();
        }
    }
}

fn open(src: &MediaSource, t: f64, audio: usize) -> Result<(Vec<Box<dyn Demuxer>>, f64), String> {
    match src {
        MediaSource::File(p) => {
            let f = std::fs::File::open(p).map_err(|e| format!("{}: {e}", p.display()))?;
            Ok((
                vec![open_demuxer(Box::new(Lazy::new(f, 256 << 10)), true)?],
                0.0,
            ))
        }
        MediaSource::Http(u) => {
            let f = crate::net::HttpFile::open(u, 8 << 20, vec![])?;
            Ok((
                vec![open_demuxer(Box::new(Lazy::new(f, 256 << 10)), true)?],
                0.0,
            ))
        }
        MediaSource::Live { url, .. } => {
            let r = crate::net::agent()
                .get(&url(t, audio))
                .call()
                .map_err(|e| format!("PC-Server: {e}"))?;
            let fwd = Forward::new(std::io::BufReader::with_capacity(
                512 << 10,
                r.into_reader(),
            ));
            Ok((vec![Box::new(mp4::Mp4::open(Box::new(fwd), false)?)], t))
        }
        MediaSource::Hls { url, max_h } => {
            let r = super::hls::open(url, *max_h)?;
            Ok((vec![Box::new(super::ts::Ts::open(Box::new(r))?)], 0.0))
        }
        MediaSource::HlsSeparate { page_url, max_h } => {
            let (video, audio) = super::hls::open_zdf(page_url, *max_h)?;
            let video = super::ts::Ts::open(Box::new(video))?;
            let audio = super::aac_hls::AacHls::open(Box::new(audio))?;
            Ok((vec![Box::new(video), Box::new(audio)], 0.0))
        }
        MediaSource::Dash { video, audio } => {
            let hdr = vec![];
            let v = crate::net::HttpFile::open(video, 4 << 20, hdr.clone())?;
            let a = crate::net::HttpFile::open(audio, 4 << 20, hdr)?;
            let dv = mp4::Mp4::open(Box::new(Lazy::new(v, 256 << 10)), true)?;
            let da = mp4::Mp4::open(Box::new(Lazy::new(a, 64 << 10)), true)?;
            Ok((vec![Box::new(dv), Box::new(da)], 0.0))
        }
    }
}

/// Merges the demuxers' info (DASH: picture from one, sound from the other).
fn merged_info(d: &[Box<dyn Demuxer>]) -> StreamInfo {
    let mut info = d[0].info().clone();
    for x in &d[1..] {
        let i = x.info();
        if info.video.is_none() {
            info.video = i.video.clone();
        }
        info.audio.extend(i.audio.iter().cloned());
        info.duration = info.duration.max(i.duration);
    }
    info
}

fn demux_thread(
    sh: Arc<Shared>,
    src: MediaSource,
    start: f64,
    cmd: Receiver<Cmd>,
    vtx: SyncSender<Item>,
    atx: SyncSender<Item>,
    tx: Sender<Msg>,
) -> Result<(), String> {
    let pick_audio = |info: &StreamInfo, want: usize| -> usize {
        if want < info.audio.len() {
            return want;
        }
        let ok = |t: &AudioTrack| t.codec.playable();
        info.audio
            .iter()
            .position(|t| ok(t) && tvcore::tracks::is_german(&t.lang))
            .or_else(|| info.audio.iter().position(|t| ok(t) && t.default))
            .or_else(|| info.audio.iter().position(ok))
            .unwrap_or(0)
    };
    let mut audio = sh.audio_sel.load(Relaxed);
    let (mut dm, offset) = open(&src, start, if audio == usize::MAX { 0 } else { audio })?;
    *sh.offset.lock().unwrap() = offset;
    let info = merged_info(&dm);
    audio = pick_audio(&info, audio);
    sh.audio_sel.store(audio, Relaxed);
    if sh.sub_sel.load(Relaxed) == -2 {
        // like the PC server: German forced subs to German sound, German subs to foreign sound
        let german = info
            .audio
            .get(audio)
            .map(|a| tvcore::tracks::is_german(&a.lang))
            .unwrap_or(false);
        let g: Vec<usize> = info
            .subs
            .iter()
            .enumerate()
            .filter(|(_, s)| tvcore::tracks::is_german(&s.lang))
            .map(|(i, _)| i)
            .collect();
        let pick = if german {
            g.iter().find(|i| info.subs[**i].forced)
        } else {
            g.iter().find(|i| !info.subs[**i].forced).or(g.first())
        };
        sh.sub_sel
            .store(pick.map(|i| *i as i64).unwrap_or(-1), Relaxed);
    }
    {
        let mut d = sh.duration.lock().unwrap();
        if info.duration > 1.0 && !matches!(src, MediaSource::Live { .. }) {
            *d = info.duration;
        }
    }
    *sh.info.lock().unwrap() = info.clone();
    let _ = tx.send(Msg::Player(Event::Ready(info.clone())));
    if start > 0.0 && !matches!(src, MediaSource::Live { .. }) {
        let mut landed = start;
        for d in dm.iter_mut() {
            if let Ok(t) = d.seek(start) {
                landed = landed.min(t);
            }
        }
        sh.set_clock(landed);
        sh.wall_set(landed);
    }
    // per demuxer: one packet of lookahead, so DASH sources interleave by time
    let mut ahead: Vec<Option<Packet>> = dm.iter().map(|_| None).collect();
    let mut done: Vec<bool> = dm.iter().map(|_| false).collect();
    let n_video_dm = 0usize;
    let mut eof_sent = false;
    loop {
        if sh.stop.load(Relaxed) {
            return Ok(());
        }
        // commands
        while let Ok(c) = cmd.try_recv() {
            let (t, new_audio) = match c {
                Cmd::Seek(t) => (t, audio),
                Cmd::Audio(a) => (sh.position(), a),
            };
            let g = sh.gen.fetch_add(1, SeqCst) + 1;
            sh.buffering.store(true, Relaxed);
            let _ = vtx.send(Item::Flush(g));
            let _ = atx.send(Item::Flush(g));
            if let MediaSource::Live { keyframe, .. } = &src {
                // restart the remux at the keyframe before t
                let k = keyframe.as_ref().map(|f| f(t)).unwrap_or(t);
                let (d, off) = open(&src, k, new_audio)?;
                dm = d;
                *sh.offset.lock().unwrap() = off;
                sh.set_clock(0.0);
                sh.wall_set(0.0);
                audio = 0; // the remux carries only the chosen track
                sh.audio_sel.store(new_audio, Relaxed);
            } else {
                audio = new_audio;
                sh.audio_sel.store(audio, Relaxed);
                let mut landed = t;
                for d in dm.iter_mut() {
                    match d.seek(t) {
                        Ok(x) => landed = landed.min(x),
                        Err(e) => crate::log(&format!("Springen: {e}")),
                    }
                }
                sh.set_clock(landed);
                sh.wall_set(landed);
            }
            ahead = dm.iter().map(|_| None).collect();
            done = dm.iter().map(|_| false).collect();
            sh.subs.lock().unwrap().clear();
            eof_sent = false;
        }
        // fill lookahead
        for i in 0..dm.len() {
            if ahead[i].is_none() && !done[i] {
                match dm[i].next() {
                    Ok(Some(p)) => ahead[i] = Some(p),
                    Ok(None) => done[i] = true,
                    Err(e) => {
                        crate::log(&format!("Lesefehler: {e}"));
                        done[i] = true;
                    }
                }
            }
        }
        let next = (0..dm.len())
            .filter(|i| ahead[*i].is_some())
            .min_by(|a, b| {
                ahead[*a]
                    .as_ref()
                    .unwrap()
                    .pts
                    .partial_cmp(&ahead[*b].as_ref().unwrap().pts)
                    .unwrap()
            });
        let Some(i) = next else {
            if !eof_sent {
                let _ = vtx.send(Item::Eof);
                let _ = atx.send(Item::Eof);
                eof_sent = true;
            }
            std::thread::sleep(Duration::from_millis(50));
            continue;
        };
        let mut p = ahead[i].take().unwrap();
        // DASH: the second demuxer's audio track 0 is track 0 overall
        if i > n_video_dm {
            if let Kind::Audio(a) = p.kind {
                p.kind =
                    Kind::Audio(a + dm[..i].iter().map(|d| d.info().audio.len()).sum::<usize>());
            }
        }
        match p.kind {
            Kind::Video => {
                if vtx.send(Item::Pkt(p)).is_err() {
                    return Ok(());
                }
            }
            Kind::Audio(a) if a == audio => {
                if atx.send(Item::Pkt(p)).is_err() {
                    return Ok(());
                }
            }
            Kind::Sub(s) if s as i64 == sh.sub_sel.load(Relaxed) => {
                let text = String::from_utf8_lossy(&p.data)
                    .replace("\\N", "\n")
                    .replace("\\n", "\n");
                let text = strip_markup(&text);
                let dur = if p.dur > 0.0 { p.dur } else { 4.0 };
                sh.subs.lock().unwrap().push(Cue {
                    start: p.pts,
                    end: p.pts + dur,
                    text,
                });
                let mut s = sh.subs.lock().unwrap();
                if s.len() > 400 {
                    s.drain(..200);
                }
            }
            _ => {}
        }
    }
}

fn strip_markup(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in s.chars() {
        match c {
            '<' | '{' => depth += 1,
            '>' | '}' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.trim().to_string()
}

fn audio_thread(sh: Arc<Shared>, rx: Receiver<Item>, out: String, tx: Sender<Msg>) {
    let mut dec: Option<(usize, super::audio::AudioDec)> = None;
    let mut pcm: Option<crate::platform_audio::Pcm> = None;
    let mut base: Option<f64> = None;
    let mut gen = 0u32;
    let mut samples = Vec::with_capacity(8192);
    let mut warned = false;
    loop {
        if sh.stop.load(Relaxed) {
            return;
        }
        let item = match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(i) => i,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => return,
        };
        match item {
            Item::Flush(g) => {
                gen = g;
                base = None;
                if let Some(p) = pcm.as_mut() {
                    p.flush();
                }
                if let Some((_, d)) = dec.as_mut() {
                    d.reset();
                }
            }
            Item::Eof => {
                sh.audio_clock.store(false, Relaxed);
            }
            Item::Pkt(p) => {
                if gen != sh.gen.load(Relaxed) {
                    continue;
                }
                let Kind::Audio(a) = p.kind else { continue };
                if dec.as_ref().map(|d| d.0 != a).unwrap_or(true) {
                    let info = sh.info.lock().unwrap().clone();
                    let Some(t) = info.audio.get(a).or(info.audio.first()) else {
                        continue;
                    };
                    match super::audio::AudioDec::new(t) {
                        Ok(d) => dec = Some((a, d)),
                        Err(e) => {
                            if !warned {
                                let _ = tx.send(Msg::Run(Box::new(move |cx| cx.toast(e))));
                                warned = true;
                            }
                            sh.audio_clock.store(false, Relaxed);
                            continue;
                        }
                    }
                }
                samples.clear();
                if dec
                    .as_mut()
                    .unwrap()
                    .1
                    .decode(&p.data, &mut samples)
                    .is_err()
                    || samples.is_empty()
                {
                    continue;
                }
                let rate = dec.as_ref().unwrap().1.rate;
                if pcm.as_ref().map(|x| x.rate != rate).unwrap_or(true) {
                    match crate::platform_audio::Pcm::open(&out, rate) {
                        Ok(x) => pcm = Some(x),
                        Err(e) => {
                            if !warned {
                                crate::log(&format!("Ton: {e}"));
                                warned = true;
                            }
                            sh.audio_clock.store(false, Relaxed);
                            continue;
                        }
                    }
                }
                let b = *base.get_or_insert(p.pts);
                while sh.paused.load(Relaxed)
                    && !sh.stop.load(Relaxed)
                    && gen == sh.gen.load(Relaxed)
                {
                    if let Some(x) = pcm.as_ref() {
                        x.pause(true);
                    }
                    std::thread::sleep(Duration::from_millis(30));
                }
                if let Some(x) = pcm.as_ref() {
                    x.pause(false);
                }
                let x = pcm.as_mut().unwrap();
                if x.write(&samples).is_err() {
                    continue;
                }
                sh.audio_clock.store(true, Relaxed);
                sh.set_clock(b + x.clock());
                sh.buffering.store(false, Relaxed);
            }
        }
    }
}

/// Current media time the picture should match.
fn clock(sh: &Shared) -> f64 {
    if sh.audio_clock.load(Relaxed) {
        sh.clock.load(Relaxed) as f64 / 1e6
    } else {
        let t = sh.wall_now();
        sh.set_clock(t);
        t
    }
}

#[cfg(target_os = "linux")]
fn video_thread(sh: Arc<Shared>, rx: Receiver<Item>, kms: Option<KmsHandle>, tx: Sender<Msg>) {
    use super::v4l2::Decoder;
    let mut dec: Option<Decoder> = None;
    let mut nal = (4usize, Vec::new());
    let mut shown: Option<u32> = None;
    let mut waiting: std::collections::VecDeque<super::v4l2::Frame> = Default::default();
    let mut pending: Option<Vec<u8>> = None;
    let mut pending_pts = 0.0;
    let mut gen = 0u32;
    let mut need_key = true;
    let mut eof = false;
    let mut failed = false;
    let mut first = true;
    let mut crop = (0u32, 0u32, 1.0f32);
    loop {
        if sh.stop.load(Relaxed) {
            return;
        }
        // 1. take a packet when there's room
        if pending.is_none() && !eof {
            match rx.recv_timeout(Duration::from_millis(if dec.is_some() { 2 } else { 50 })) {
                Ok(Item::Flush(g)) => {
                    gen = g;
                    need_key = true;
                    waiting.clear();
                    if let Some(d) = dec.as_mut() {
                        d.flush(shown);
                    }
                    eof = false;
                    first = true;
                }
                Ok(Item::Eof) => eof = true,
                Ok(Item::Pkt(p)) => {
                    if gen != sh.gen.load(Relaxed) || failed {
                        continue;
                    }
                    if dec.is_none() {
                        let info = sh.info.lock().unwrap().clone();
                        let Some(v) = info.video.clone() else {
                            continue;
                        };
                        match &v.codec {
                            VCodec::H264 { avcc } => nal = avcc_to_annexb(avcc),
                            VCodec::Hevc => {
                                failed = true;
                                let _ = tx.send(Msg::Player(Event::Error("Dieser Film ist HEVC (H.265) -- das kann der Pi 3 nicht. Über den PC-Server abspielen, der wandelt um.".into())));
                                continue;
                            }
                            VCodec::Other(c) => {
                                failed = true;
                                let _ = tx.send(Msg::Player(Event::Error(format!(
                                    "Bildformat {c} kann der Pi nicht abspielen."
                                ))));
                                continue;
                            }
                        }
                        crop = (v.width, v.height, v.sar);
                        let Some(k) = kms.clone() else {
                            failed = true;
                            continue;
                        };
                        match Decoder::open(v.width, v.height, k) {
                            Ok(d) => dec = Some(d),
                            Err(e) => {
                                failed = true;
                                let _ = tx.send(Msg::Player(Event::Error(e)));
                                continue;
                            }
                        }
                    }
                    if need_key && !p.key {
                        continue;
                    }
                    need_key = false;
                    let mut au = Vec::with_capacity(p.data.len() + 64);
                    if p.key {
                        au.extend_from_slice(&nal.1);
                    }
                    if p.data.starts_with(&[0, 0, 0, 1]) || p.data.starts_with(&[0, 0, 1]) {
                        au.extend_from_slice(&p.data);
                    } else {
                        to_annexb(&p.data, nal.0, &mut au);
                    }
                    pending = Some(au);
                    pending_pts = p.pts;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return,
            }
        }
        if dec.is_none() && eof {
            // nothing to show (sound only, or the picture can't be decoded): end with the sound
            while sh.audio_clock.load(Relaxed) && !sh.stop.load(Relaxed) {
                std::thread::sleep(Duration::from_millis(200));
            }
            if !sh.stop.load(Relaxed) {
                let _ = tx.send(Msg::Player(Event::Ended));
            }
            return;
        }
        let Some(d) = dec.as_mut() else {
            if failed {
                // drain so the demuxer never blocks on us
                while let Ok(i) = rx.try_recv() {
                    if let Item::Flush(g) = i {
                        gen = g;
                    }
                }
            }
            continue;
        };
        // 2. feed the decoder
        if let Some(au) = pending.as_ref() {
            match d.push(au, pending_pts) {
                Ok(true) => pending = None,
                Ok(false) => {}
                Err(e) => {
                    crate::log(&e);
                    pending = None;
                }
            }
        }
        // 3. collect pictures
        let _ = d.wait(if pending.is_some() { 5 } else { 1 });
        while let Some(f) = d.frame() {
            waiting.push_back(f);
        }
        // 4. show the picture that's due
        let now = clock(&sh);
        while let Some(f) = waiting.front() {
            let late = now - f.pts;
            if first || late >= -0.004 {
                let f = waiting.pop_front().unwrap();
                // far behind and more waiting: drop it to catch up
                if late > 0.12 && !waiting.is_empty() && !first {
                    d.recycle(f.index);
                    continue;
                }
                if let Some(k) = kms.as_ref() {
                    let (cw, ch) = if crop.0 > 0 {
                        (crop.0.min(d.coded.0), crop.1.min(d.coded.1))
                    } else {
                        d.coded
                    };
                    k.lock()
                        .unwrap()
                        .show_video(d.display_id(f.index), (cw, ch), crop.2);
                }
                if let Some(old) = shown.replace(f.index) {
                    d.recycle(old);
                }
                if first {
                    first = false;
                    if !sh.audio_clock.load(Relaxed) {
                        sh.wall_set(f.pts);
                    }
                    sh.buffering.store(false, Relaxed);
                    let _ = tx.send(Msg::Redraw);
                }
                sh.frames_shown.fetch_add(1, Relaxed);
            } else {
                break;
            }
            if sh.paused.load(Relaxed) {
                break;
            }
        }
        while sh.paused.load(Relaxed) && !sh.stop.load(Relaxed) {
            std::thread::sleep(Duration::from_millis(30));
        }
        if eof && waiting.is_empty() && pending.is_none() && rx.try_recv().is_err() {
            std::thread::sleep(Duration::from_millis(300));
            if !sh.stop.load(Relaxed) {
                let _ = tx.send(Msg::Player(Event::Ended));
            }
            return;
        }
    }
}

/// Without a hardware decoder (development machines): keep pace, show nothing.
#[cfg(not(target_os = "linux"))]
fn video_thread(sh: Arc<Shared>, rx: Receiver<Item>, _kms: Option<KmsHandle>, tx: Sender<Msg>) {
    null_video(sh, rx, tx)
}

#[allow(dead_code)]
fn null_video(sh: Arc<Shared>, rx: Receiver<Item>, tx: Sender<Msg>) {
    loop {
        if sh.stop.load(Relaxed) {
            return;
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Item::Pkt(p)) => {
                while clock(&sh) < p.pts - 0.01 && !sh.stop.load(Relaxed) {
                    std::thread::sleep(Duration::from_millis(5));
                }
                sh.buffering.store(false, Relaxed);
            }
            Ok(Item::Eof) => {
                let _ = tx.send(Msg::Player(Event::Ended));
                return;
            }
            Ok(Item::Flush(_)) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => return,
        }
    }
}
