//! The film server (port 8420). API:
//!   GET  /api/ping              -> { name, version, films }
//!   GET  /api/library           -> [ { id, title, info, fsk, cast, crew, genre, duration, size, icon, banner, stream, play, codec, position } ]
//!   GET  /stream/<id>           -> the file itself, with HTTP Range
//!   GET  /play/<id>?t=&a=&v=    -> remuxed live to fragmented MP4 (v=h264: convert the picture unless it is H.264)
//!   GET  /seek/<id>?t=          -> { t } where such a stream really starts (keyframe before t)
//!   GET  /sub/<id>/<n>?t=       -> a text subtitle as WebVTT, shifted to the stream start
//!   GET  /img/<id>/icon|banner  -> cover / banner
//!   POST /api/progress/<id>     -> body { seconds }: resume position, shared by every device
use crate::http::{self, Resp};
use crate::video::{needs_remux, VideoTools};
use crate::{key_query, log, preflight, Config};
use serde_json::json;
use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tiny_http::{Method, Request};
use tvcore::films::{scan, Film};
use tvcore::util::{image_mime, mp4_duration};

struct State {
    cfg: Arc<Config>,
    video: VideoTools,
    lib: Mutex<(Vec<Film>, Option<Instant>)>,
    durations: Mutex<HashMap<String, Option<u64>>>,
    progress: Mutex<serde_json::Map<String, serde_json::Value>>,
}

impl State {
    /// Rescanned at most every 30 s (on demand), so new films show up without a restart.
    fn library(&self) -> Vec<Film> {
        let mut g = self.lib.lock().unwrap();
        if g.1.map(|t| t.elapsed() < Duration::from_secs(30)).unwrap_or(false) {
            return g.0.clone();
        }
        let mut films = scan(&self.cfg.media_dir);
        let mut durs = self.durations.lock().unwrap();
        for f in &mut films {
            let key = format!("{}|{}", f.file.to_string_lossy(), f.mtime_ms);
            let d = durs.entry(key).or_insert_with(|| {
                let mp4ish = matches!(tvcore::util::ext_lower(&f.file).as_str(), ".mp4" | ".m4v" | ".mov");
                if mp4ish { mp4_duration(&f.file).or_else(|| self.video.probe(&f.file).and_then(|p| p.duration)) } else { self.video.probe(&f.file).and_then(|p| p.duration) }
            });
            f.duration = *d;
        }
        *g = (films.clone(), Some(Instant::now()));
        films
    }

    fn film(&self, id: &str) -> Option<Film> {
        self.library().into_iter().find(|f| f.id == id)
    }

    fn public(&self, f: &Film, base: &str) -> serde_json::Value {
        let key = key_query(&self.cfg.api_key);
        let remux = needs_remux(&f.file) && self.video.ffmpeg.is_some();
        let info = if self.video.ffmpeg.is_some() { self.video.probe(&f.file) } else { None };
        let play = if remux {
            let info = info.clone().unwrap_or_default();
            let (a, s) = tvcore::tracks::pick_tracks(&info);
            json!({
                "remux": true,
                "duration": info.duration,
                "audio": info.audio.iter().map(|t| json!({ "lang": t.lang, "title": t.title, "codec": t.codec, "channels": t.channels })).collect::<Vec<_>>(),
                "subs": info.subs.iter().map(|t| json!({ "n": t.n, "lang": t.lang, "title": t.title, "forced": t.forced })).collect::<Vec<_>>(),
                "a": a, "s": s,
                "sub": format!("{base}/sub/{}/{{n}}{key}", f.id),
                "hevc": info.video.as_ref().map(|v| v.codec == "hevc").unwrap_or(false),
            })
        } else {
            serde_json::Value::Null
        };
        let mut v = serde_json::to_value(f).unwrap();
        let o = v.as_object_mut().unwrap();
        o.insert("icon".into(), f.icon_file.as_ref().map(|_| json!(format!("{base}/img/{}/icon{key}", f.id))).unwrap_or(json!(null)));
        o.insert("banner".into(), f.banner_file.as_ref().map(|_| json!(format!("{base}/img/{}/banner{key}", f.id))).unwrap_or(json!(null)));
        o.insert("stream".into(), json!(format!("{base}/{}/{}{key}", if remux { "play" } else { "stream" }, f.id)));
        o.insert("play".into(), play);
        // New for the Pi: the picture's codec (it asks /play?v=h264 when this isn't "h264")
        // and the convertible stream for every film, even MP4s.
        o.insert("codec".into(), info.as_ref().and_then(|i| i.video.as_ref()).map(|v| json!(v.codec)).unwrap_or(json!(null)));
        o.insert("convert".into(), if self.video.ffmpeg.is_some() { json!(format!("{base}/play/{}{key}", f.id)) } else { json!(null) });
        o.insert("position".into(), self.progress.lock().unwrap().get(&f.id).cloned().unwrap_or(json!(0)));
        v
    }
}

pub fn run(cfg: Arc<Config>) {
    let progress_file = cfg.data_dir.join("progress.json");
    let progress = std::fs::read(&progress_file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let st = Arc::new(State {
        video: VideoTools::new(cfg.data_dir.clone()),
        lib: Mutex::new((Vec::new(), None)),
        durations: Mutex::new(HashMap::new()),
        progress: Mutex::new(progress),
        cfg: cfg.clone(),
    });
    log(&format!("Filme aus: {} ({} gefunden)", cfg.media_dir.display(), st.library().len()));
    for ip in crate::lan_ips() {
        log(&format!("Im Launcher eintragen (Filme): http://{ip}:{}", cfg.port));
    }
    let port = cfg.port;
    crate::serve(port, "Filme", move |req| {
        let st = st.clone();
        let Some((mut req, url)) = preflight(req, &st.cfg.api_key) else { return };
        let r = handle(&st, &mut req, &url).unwrap_or_else(|| http::json(404, &json!({ "error": "unbekannt" })));
        let _ = req.respond(http::cors(r));
    });
}

fn handle(st: &State, req: &mut Request, url: &http::Url) -> Option<Resp> {
    let host = http::get_header(req, "Host").unwrap_or_else(|| format!("127.0.0.1:{}", st.cfg.port));
    let base = format!("http://{host}");
    let get = matches!(req.method(), Method::Get | Method::Head);
    let p: Vec<&str> = url.parts.iter().map(String::as_str).collect();
    Some(match (req.method().clone(), p.as_slice()) {
        (Method::Get, ["api", "ping"]) => http::json(200, &json!({ "name": st.cfg.name, "version": crate::VERSION, "films": st.library().len() })),
        (Method::Get, ["api", "library"]) => {
            let lib = st.library();
            http::json(200, &lib.iter().map(|f| st.public(f, &base)).collect::<Vec<_>>())
        }
        (_, ["stream", id]) if get => {
            let Some(f) = st.film(id) else { return Some(http::text(404, "unbekannter Film")) };
            http::file(req, &f.file, f.mime, None, "no-store")
        }
        (_, ["play", id]) if get => {
            let Some(f) = st.film(id) else { return Some(http::text(404, "unbekannter Film")) };
            if *req.method() == Method::Head {
                return Some(http::stream("video/mp4", Box::new(std::io::empty())));
            }
            match st.video.remux(&f.file, url.num("t"), url.num("a") as usize, url.q("v").unwrap_or("")) {
                Some(child) => http::stream("video/mp4", Box::new(child)),
                None => http::text(501, "ffmpeg fehlt auf diesem Rechner"),
            }
        }
        (Method::Get, ["seek", id]) => {
            let Some(f) = st.film(id) else { return Some(http::text(404, "unbekannter Film")) };
            http::json(200, &json!({ "t": st.video.keyframe_before(&f.file, url.num("t")) }))
        }
        (Method::Get, ["sub", id, n]) => {
            let Some(f) = st.film(id) else { return Some(http::text(404, "unbekannter Film")) };
            match st.video.subtitle(&f.file, n.parse().unwrap_or(0), url.num("t")) {
                Some(v) => http::bytes(200, "text/vtt; charset=utf-8", v.into_bytes()),
                None => http::text(404, "kein Untertitel"),
            }
        }
        (_, ["img", id, which]) if get => {
            let f = st.film(id);
            let file = f.and_then(|f| if *which == "banner" { f.banner_file } else { f.icon_file });
            let Some(file) = file else { return Some(http::text(404, "kein Bild")) };
            http::file(req, &file, image_mime(&file).unwrap_or("application/octet-stream"), None, "max-age=86400")
        }
        (Method::Post, ["api", "progress", id]) => {
            let mut body = String::new();
            let _ = req.as_reader().take(1000).read_to_string(&mut body);
            let secs = serde_json::from_str::<serde_json::Value>(&body).ok().and_then(|v| v["seconds"].as_f64()).filter(|s| s.is_finite() && *s >= 0.0);
            let Some(secs) = secs else { return Some(http::json(400, &json!({ "error": "seconds fehlt" }))) };
            let mut p = st.progress.lock().unwrap();
            p.insert(id.to_string(), json!(secs.round() as u64));
            std::fs::create_dir_all(&st.cfg.data_dir).ok();
            if let Err(e) = std::fs::write(st.cfg.data_dir.join("progress.json"), serde_json::to_vec_pretty(&*p).unwrap()) {
                log(&format!("progress not saved: {e}"));
            }
            http::json(200, &json!({ "ok": true }))
        }
        _ => return None,
    })
}
