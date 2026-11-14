//! HLS (how TV channels stream): picks the best variant the Pi 3 can decode
//! (H.264, up to the chosen height), then keeps fetching the newest segments
//! of the live playlist in the background and hands them on as one
//! continuous MPEG-TS byte stream.
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::mpsc::{sync_channel, Receiver};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

static ARTE_PLAYLIST_CACHE: OnceLock<Mutex<Option<(Instant, String, u32)>>> = OnceLock::new();

pub fn resolve(base: &str, u: &str) -> String {
    if u.starts_with("http://") || u.starts_with("https://") {
        return u.to_string();
    }
    if u.starts_with("//") {
        let scheme = base.split("//").next().unwrap_or("https:");
        return format!("{scheme}{u}");
    }
    let base = base.split('?').next().unwrap_or(base);
    if u.starts_with('/') {
        let origin_end = base
            .find("://")
            .map(|i| {
                base[i + 3..]
                    .find('/')
                    .map(|j| i + 3 + j)
                    .unwrap_or(base.len())
            })
            .unwrap_or(0);
        return format!("{}{u}", &base[..origin_end]);
    }
    let dir = &base[..base.rfind('/').map(|i| i + 1).unwrap_or(base.len())];
    format!("{dir}{u}")
}

fn attr(line: &str, key: &str) -> Option<String> {
    // KEY=value or KEY="value", comma separated (quotes may contain commas)
    let mut rest = line.split_once(':').map(|x| x.1).unwrap_or(line);
    while !rest.is_empty() {
        let (k, v) = rest.split_once('=')?;
        let k = k.trim();
        let (val, after) = if let Some(stripped) = v.strip_prefix('"') {
            let end = stripped.find('"').unwrap_or(stripped.len());
            (
                stripped[..end].to_string(),
                stripped.get(end + 1..).unwrap_or(""),
            )
        } else {
            let end = v.find(',').unwrap_or(v.len());
            (v[..end].to_string(), &v[end..])
        };
        if k.eq_ignore_ascii_case(key) {
            return Some(val);
        }
        rest = after.trim_start_matches(',');
    }
    None
}

#[derive(Debug, Clone)]
pub struct Variant {
    pub url: String,
    pub bandwidth: u64,
    pub height: u32,
    pub codecs: String,
    pub audio_group: Option<String>,
}

/// Variants of a master playlist (empty: it's already a media playlist).
pub fn variants(base: &str, text: &str) -> Vec<Variant> {
    let mut out = Vec::new();
    let mut pending: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("#EXT-X-STREAM-INF") {
            pending = Some(line.to_string());
        } else if !line.is_empty() && !line.starts_with('#') {
            if let Some(inf) = pending.take() {
                let height = attr(&inf, "RESOLUTION")
                    .and_then(|r| r.split('x').nth(1).and_then(|h| h.parse().ok()))
                    .unwrap_or(0);
                out.push(Variant {
                    url: resolve(base, line),
                    bandwidth: attr(&inf, "BANDWIDTH")
                        .and_then(|b| b.parse().ok())
                        .unwrap_or(0),
                    height,
                    codecs: attr(&inf, "CODECS").unwrap_or_default(),
                    audio_group: attr(&inf, "AUDIO"),
                });
            }
        }
    }
    out
}

/// The best variant for a Pi 3: H.264 (or unknown codecs), at most `max_h` lines.
pub fn pick(vs: &[Variant], max_h: u32) -> Option<Variant> {
    let ok = |v: &&Variant| {
        (v.codecs.is_empty() || v.codecs.contains("avc1"))
            && !v.codecs.contains("hvc1")
            && !v.codecs.contains("hev1")
    };
    let mut c: Vec<&Variant> = vs.iter().filter(ok).collect();
    if c.is_empty() {
        return None;
    }
    let fits: Vec<&Variant> = c
        .iter()
        .copied()
        .filter(|v| v.height == 0 || v.height <= max_h)
        .collect();
    if !fits.is_empty() {
        c = fits;
    } else {
        // everything is bigger: the smallest
        c.sort_by_key(|v| v.height);
        c.truncate(1);
    }
    c.into_iter()
        .max_by_key(|v| (v.height, v.bandwidth))
        .cloned()
}

struct Media {
    target: f64,
    seq: u64,
    segs: Vec<String>,
    ended: bool,
    encrypted: bool,
    fmp4: bool,
}

fn media(base: &str, text: &str) -> Media {
    let mut m = Media {
        target: 6.0,
        seq: 0,
        segs: Vec::new(),
        ended: false,
        encrypted: false,
        fmp4: false,
    };
    for line in text.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("#EXT-X-TARGETDURATION:") {
            m.target = v.trim().parse().unwrap_or(6.0);
        } else if let Some(v) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            m.seq = v.trim().parse().unwrap_or(0);
        } else if line.starts_with("#EXT-X-ENDLIST") {
            m.ended = true;
        } else if line.starts_with("#EXT-X-KEY") && !line.contains("METHOD=NONE") {
            m.encrypted = true;
        } else if line.starts_with("#EXT-X-MAP") {
            m.fmp4 = true;
        } else if !line.is_empty() && !line.starts_with('#') {
            m.segs.push(resolve(base, line));
        }
    }
    m
}

fn get_text(url: &str) -> Result<String, String> {
    let b = crate::net::get_bytes(url, 4 << 20)?;
    Ok(String::from_utf8_lossy(&b).into_owned())
}

/// Resolve ARTE's current H.264 playlist, reusing only its URL briefly.
/// The live playlist and TS segments are still fetched fresh on every tune.
fn arte_playlist(max_h: u32) -> Result<(String, String), String> {
    let cache = ARTE_PLAYLIST_CACHE.get_or_init(|| Mutex::new(None));
    let cached = cache.lock().ok().and_then(|entry| {
        entry
            .as_ref()
            .filter(|(at, _, cached_max_h)| {
                at.elapsed() < Duration::from_secs(60) && *cached_max_h == max_h
            })
            .map(|(_, url, _)| url.clone())
    });
    if let Some(url) = cached {
        let stage = Instant::now();
        if let Ok(text) = get_text(&url) {
            if text.trim_start().starts_with("#EXTM3U") && variants(&url, &text).is_empty() {
                crate::log(&format!(
                    "ARTE HLS: Cache-Playlist in {} ms",
                    stage.elapsed().as_millis()
                ));
                return Ok((url, text));
            }
        }
        if let Ok(mut entry) = cache.lock() {
            *entry = None;
        }
        crate::log("ARTE HLS: Playlist-Cache ungültig, löse erneut auf");
    }

    let stage = Instant::now();
    let config = crate::net::get_json(crate::livetv::ARTE_LIVE_CONFIG_URL)?;
    crate::log(&format!(
        "ARTE HLS: Player-API in {} ms",
        stage.elapsed().as_millis()
    ));
    let root = config
        .as_array()
        .and_then(|items| items.first())
        .unwrap_or(&config);
    let streams = root
        .pointer("/data/attributes/streams")
        .and_then(serde_json::Value::as_array)
        .ok_or("ARTE-Livestream fehlt in der Player-API")?;
    let stream = streams
        .iter()
        .find(|s| {
            s["versions"]
                .as_array()
                .map(|v| v.iter().any(|x| x["code"] == "liveDE"))
                .unwrap_or(false)
        })
        .ok_or("ARTE-Stream für Deutsch fehlt")?;
    let master_url = stream["url"]
        .as_str()
        .ok_or("ARTE-Playlist-URL fehlt")?
        .to_string();
    let stage = Instant::now();
    let master = get_text(&master_url)?;
    crate::log(&format!(
        "ARTE HLS: Master-Playlist in {} ms",
        stage.elapsed().as_millis()
    ));
    if !master.trim_start().starts_with("#EXTM3U") {
        return Err("Keine HLS-Wiedergabeliste".into());
    }
    let choices = variants(&master_url, &master);
    let media_url = if choices.is_empty() {
        master_url
    } else {
        let selected =
            pick(&choices, max_h).ok_or("Sender sendet kein H.264 (der Pi 3 kann nur H.264)")?;
        selected.url
    };
    let media_text = if choices.is_empty() {
        master
    } else {
        let stage = Instant::now();
        let text = get_text(&media_url)?;
        crate::log(&format!(
            "ARTE HLS: Varianten-Playlist in {} ms",
            stage.elapsed().as_millis()
        ));
        text
    };
    if let Ok(mut entry) = cache.lock() {
        *entry = Some((Instant::now(), media_url.clone(), max_h));
    }
    Ok((media_url, media_text))
}

/// The TS bytes of a live HLS stream, read like a file.
pub struct HlsReader {
    rx: Receiver<Result<Vec<u8>, String>>,
    cur: Vec<u8>,
    pos: usize,
    stop: Arc<AtomicBool>,
}

impl Drop for HlsReader {
    fn drop(&mut self) {
        self.stop.store(true, Relaxed);
    }
}

impl Read for HlsReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.pos >= self.cur.len() {
            match self.rx.recv_timeout(Duration::from_secs(25)) {
                Ok(Ok(seg)) => {
                    self.cur = seg;
                    self.pos = 0;
                }
                Ok(Err(e)) => return Err(std::io::Error::other(e)),
                Err(_) => return Ok(0), // stream ended or stalled
            }
        }
        let n = buf.len().min(self.cur.len() - self.pos);
        buf[..n].copy_from_slice(&self.cur[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

fn strip_id3_tag(mut data: Vec<u8>) -> Result<Vec<u8>, String> {
    while data.starts_with(b"ID3") {
        if data.len() < 10 {
            return Err("Unvollständiger ID3-Tag im AAC-Segment".into());
        }
        let size = ((data[6] as usize & 0x7f) << 21)
            | ((data[7] as usize & 0x7f) << 14)
            | ((data[8] as usize & 0x7f) << 7)
            | (data[9] as usize & 0x7f);
        let end = 10usize
            .checked_add(size)
            .filter(|end| *end <= data.len())
            .ok_or("Ungültige ID3-Länge im AAC-Segment")?;
        data.drain(..end);
    }
    Ok(data)
}

fn start_reader(
    media_url: String,
    first: Media,
    initial_sequence: Option<u64>,
    strip_id3: bool,
    service: Option<&'static str>,
    track: &'static str,
    hls_started: Instant,
) -> Result<HlsReader, String> {
    if first.encrypted {
        return Err("Sender ist verschlüsselt".into());
    }
    if first.fmp4 {
        return Err("Sender nutzt fMP4-Segmente (noch nicht unterstützt)".into());
    }
    let (tx, rx) = sync_channel::<Result<Vec<u8>, String>>(4);
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    std::thread::spawn(move || {
        let mut m = first;
        // Begin all renditions at one shared media sequence when supplied;
        // ordinary channels keep the three-segment safety buffer.
        let mut next = if m.ended {
            m.seq
        } else {
            initial_sequence
                .unwrap_or(m.seq + m.segs.len().saturating_sub(3) as u64)
                .max(m.seq)
        };
        let mut fails = 0;
        let mut first_segment = true;
        loop {
            if stop2.load(Relaxed) {
                return;
            }
            let mut fetched = false;
            for (i, seg) in m.segs.iter().enumerate() {
                let n = m.seq + i as u64;
                if n < next {
                    continue;
                }
                match crate::net::get_bytes(seg, 64 << 20) {
                    Ok(b) => {
                        let b = if strip_id3 {
                            match strip_id3_tag(b) {
                                Ok(data) => data,
                                Err(e) => {
                                    let _ = tx.send(Err(e));
                                    return;
                                }
                            }
                        } else {
                            b
                        };
                        if let Some(service) = service.filter(|_| first_segment) {
                            crate::log(&format!(
                                "{service} HLS: Erstes {track}-Segment in {} ms",
                                hls_started.elapsed().as_millis()
                            ));
                        }
                        first_segment = false;
                        fails = 0;
                        if tx.send(Ok(b)).is_err() {
                            return;
                        }
                    }
                    Err(e) => {
                        crate::log(&format!("Live TV: Segment fehlt ({e})"));
                    }
                }
                next = n + 1;
                fetched = true;
                if stop2.load(Relaxed) {
                    return;
                }
            }
            if m.ended && !fetched {
                return;
            }
            if !fetched {
                std::thread::sleep(Duration::from_secs_f64((m.target / 2.0).clamp(0.5, 5.0)));
            }
            let t = Instant::now();
            match get_text(&media_url) {
                Ok(text) => {
                    m = media(&media_url, &text);
                    // fell too far behind (sequence jumped): rejoin near the live edge
                    if next < m.seq {
                        next = initial_sequence
                            .unwrap_or(m.seq + m.segs.len().saturating_sub(3) as u64)
                            .max(m.seq);
                    }
                }
                Err(e) => {
                    fails += 1;
                    if fails > 5 {
                        let _ = tx.send(Err(format!("Sender antwortet nicht ({e})")));
                        return;
                    }
                    std::thread::sleep(Duration::from_secs(1).saturating_sub(t.elapsed()));
                }
            }
        }
    });
    Ok(HlsReader {
        rx,
        cur: Vec::new(),
        pos: 0,
        stop,
    })
}

/// Opens a channel: resolves the master playlist and starts the segment fetcher.
pub fn open(url: &str, max_h: u32) -> Result<HlsReader, String> {
    let is_arte = url == crate::livetv::ARTE_LIVE_CONFIG_URL;
    let hls_started = Instant::now();
    let (playlist_url, text) = if is_arte {
        arte_playlist(max_h)?
    } else {
        (url.to_string(), get_text(url)?)
    };
    if !text.trim_start().starts_with("#EXTM3U") {
        return Err("Keine HLS-Wiedergabeliste".into());
    }
    let vs = variants(&playlist_url, &text);
    let media_url = if vs.is_empty() {
        playlist_url.clone()
    } else {
        pick(&vs, max_h)
            .ok_or("Sender sendet kein H.264 (der Pi 3 kann nur H.264)")?
            .url
    };
    let media_text = if vs.is_empty() {
        text
    } else {
        let stage = Instant::now();
        let text = get_text(&media_url)?;
        if is_arte {
            crate::log(&format!(
                "ARTE HLS: Varianten-Playlist in {} ms",
                stage.elapsed().as_millis()
            ));
        }
        text
    };
    let first = media(&media_url, &media_text);
    start_reader(
        media_url,
        first,
        None,
        false,
        if is_arte { Some("ARTE") } else { None },
        "Video",
        hls_started,
    )
}

fn page_stream_url(page_url: &str) -> Result<String, String> {
    let page = String::from_utf8_lossy(&crate::net::get_bytes(page_url, 4 << 20)?).into_owned();
    page.split("\"contentUrl\":\"")
        .skip(1)
        .filter_map(|s| s.split('\"').next())
        .map(|s| s.replace("\\/", "/"))
        .find(|s| s.starts_with("https://zdf-hls") && s.ends_with(".m3u8"))
        .ok_or_else(|| "ZDF-Livestream-URL fehlt auf der offiziellen Live-Seite".into())
}

fn audio_playlist_uri(master: &str, group: &str) -> Option<String> {
    let is_group_audio = |line: &&str| {
        attr(line, "TYPE").as_deref() == Some("AUDIO")
            && attr(line, "GROUP-ID").as_deref() == Some(group)
    };
    let preferred = master
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("#EXT-X-MEDIA:"))
        .filter(|line| is_group_audio(line))
        .find(|line| {
            matches!(attr(line, "LANGUAGE").as_deref(), Some("deu" | "de"))
                && attr(line, "DEFAULT").as_deref() == Some("YES")
        });
    let selected = preferred.or_else(|| {
        master
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with("#EXT-X-MEDIA:"))
            .filter(|line| is_group_audio(line))
            .find(|line| attr(line, "DEFAULT").as_deref() == Some("YES"))
    })?;
    attr(selected, "URI")
}

/// Resolves ZDF's public HTML5 live page into synchronized H.264 and AAC HLS renditions.
pub fn open_zdf(page_url: &str, max_h: u32) -> Result<(HlsReader, HlsReader), String> {
    let stage = Instant::now();
    let master_url = page_stream_url(page_url)?;
    crate::log(&format!(
        "ZDF HLS: Live-Seite in {} ms",
        stage.elapsed().as_millis()
    ));
    open_separate_master(&master_url, max_h, "ZDF")
}

/// Opens an official HLS master whose H.264 video and packed AAC audio use separate playlists.
pub fn open_separate_master(
    master_url: &str,
    max_h: u32,
    service: &'static str,
) -> Result<(HlsReader, HlsReader), String> {
    let started = Instant::now();
    let stage = Instant::now();
    let master = get_text(&master_url)?;
    crate::log(&format!(
        "{service} HLS: Master-Playlist in {} ms",
        stage.elapsed().as_millis()
    ));
    if !master.trim_start().starts_with("#EXTM3U") {
        return Err(format!("{service} lieferte keine HLS-Wiedergabeliste"));
    }
    let choices = variants(&master_url, &master);
    let selected =
        pick(&choices, max_h).ok_or_else(|| format!("{service} sendet kein H.264 für den Pi"))?;
    let group = selected
        .audio_group
        .as_deref()
        .ok_or_else(|| format!("{service}-Video-Playlist hat keine Tonspur-Gruppe"))?;
    let audio_uri = audio_playlist_uri(&master, group)
        .ok_or_else(|| format!("{service}-Playlist hat keine deutsche Standard-Tonspur"))?;
    let video_url = selected.url;
    let audio_url = resolve(master_url, &audio_uri);
    let stage = Instant::now();
    let (video_result, audio_result) = std::thread::scope(|scope| {
        let video = scope.spawn(|| get_text(&video_url));
        let audio = scope.spawn(|| get_text(&audio_url));
        (video.join(), audio.join())
    });
    let video_text = video_result
        .map_err(|_| format!("{service} HLS-Video-Playlist-Ladevorgang abgebrochen"))??;
    let audio_text = audio_result
        .map_err(|_| format!("{service} HLS-Ton-Playlist-Ladevorgang abgebrochen"))??;
    crate::log(&format!(
        "{service} HLS: Video- und Ton-Playlist in {} ms",
        stage.elapsed().as_millis()
    ));
    let video_first = media(&video_url, &video_text);
    let audio_first = media(&audio_url, &audio_text);
    if video_first.segs.is_empty() || audio_first.segs.is_empty() {
        return Err(format!("{service}-Live-Playlist enthält keine Segmente"));
    }
    let video_end = video_first
        .seq
        .saturating_add(video_first.segs.len() as u64);
    let audio_end = audio_first
        .seq
        .saturating_add(audio_first.segs.len() as u64);
    let common_end = video_end.min(audio_end);
    let common_start = video_first
        .seq
        .max(audio_first.seq)
        .max(common_end.saturating_sub(3));
    let video = start_reader(
        video_url,
        video_first,
        Some(common_start),
        false,
        Some(service),
        "Video",
        started,
    )?;
    let audio = start_reader(
        audio_url,
        audio_first,
        Some(common_start),
        true,
        Some(service),
        "Ton",
        started,
    )?;
    Ok((video, audio))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn urls() {
        assert_eq!(
            resolve("https://a.de/x/y/master.m3u8?t=1", "720/i.m3u8"),
            "https://a.de/x/y/720/i.m3u8"
        );
        assert_eq!(
            resolve("https://a.de/x/y/m.m3u8", "/z/s.ts"),
            "https://a.de/z/s.ts"
        );
        assert_eq!(
            resolve("https://a.de/m.m3u8", "//cdn.de/s.ts"),
            "https://cdn.de/s.ts"
        );
    }
    #[test]
    fn picks_variant() {
        let m = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360,CODECS=\"avc1.4d401e,mp4a.40.2\"\nlow.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1920x1080,CODECS=\"avc1.640028,mp4a.40.2\"\nhd.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=3000000,RESOLUTION=1280x720,CODECS=\"avc1.4d401f,mp4a.40.2\"\nmid.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=9000000,RESOLUTION=3840x2160,CODECS=\"hvc1.2.4.L150,mp4a.40.2\"\nuhd.m3u8\n";
        let v = variants("https://tv.de/live/master.m3u8", m);
        assert_eq!(v.len(), 4);
        assert_eq!(pick(&v, 1080).unwrap().url, "https://tv.de/live/hd.m3u8");
        assert_eq!(pick(&v, 720).unwrap().url, "https://tv.de/live/mid.m3u8");
        assert_eq!(pick(&v, 300).unwrap().url, "https://tv.de/live/low.m3u8");
    }

    #[test]
    fn selects_default_audio_for_two_letter_german_language() {
        let master = "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"main\",LANGUAGE=\"en\",DEFAULT=YES,URI=\"en.m3u8\"\n\
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"main\",LANGUAGE=\"de\",DEFAULT=YES,URI=\"de.m3u8\"\n\
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"other\",LANGUAGE=\"de\",DEFAULT=YES,URI=\"other.m3u8\"\n";
        assert_eq!(
            audio_playlist_uri(master, "main").as_deref(),
            Some("de.m3u8")
        );
    }

    #[test]
    fn strips_synchsafe_id3_prefix_from_packed_aac() {
        let mut segment = b"ID3\x04\x00\x00\x00\x00\x00\x03tag".to_vec();
        segment.extend_from_slice(b"\xff\xf1\x4c");
        assert_eq!(strip_id3_tag(segment).unwrap(), b"\xff\xf1\x4c");
    }
}
