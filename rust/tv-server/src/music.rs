//! The music server (port 8421). API:
//!   GET /api/ping              -> { name, version, type: 'music', artists, albums, tracks }
//!   GET /api/library           -> { server, artists, albums, tracks }
//!   GET /stream/<trackId>      -> the audio file, with HTTP Range
//!   GET /img/album/<albumId>   -> Folder.jpg (?size=small -> AlbumArtSmall.jpg), or the embedded cover
//!   GET /img/artist/<artistId> -> artist.jpg / artist.png
//!   GET /lyrics/<trackId>      -> { type: 'lrc' | 'text', text }
use crate::http::{self, Resp};
use crate::{key_query, log, preflight, Config};
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tiny_http::{Method, Request};
use tvcore::music::{read_lyrics, Library, Scanner};
use tvcore::util::image_mime;

struct State {
    cfg: Arc<Config>,
    scanner: Mutex<Scanner>,
    lib: Mutex<(Arc<Library>, Option<Instant>)>,
}

impl State {
    fn library(&self) -> Arc<Library> {
        let mut g = self.lib.lock().unwrap();
        if g.1.map(|t| t.elapsed() < Duration::from_secs(30)).unwrap_or(false) {
            return g.0.clone();
        }
        let lib = Arc::new(self.scanner.lock().unwrap().scan());
        *g = (lib.clone(), Some(Instant::now()));
        lib
    }
}

pub fn public_library(l: &Library, base: &str, server: &str, api_key: &str) -> serde_json::Value {
    let key = key_query(api_key);
    let amp = if key.is_empty() { "?" } else { "&" };
    json!({
        "server": server,
        "artists": l.artists.iter().map(|a| json!({
            "id": a.id, "name": a.name, "albumIds": a.album_ids,
            "image": a.image_file.as_ref().map(|_| format!("{base}/img/artist/{}{key}", a.id)),
        })).collect::<Vec<_>>(),
        "albums": l.albums.iter().map(|a| {
            let has = a.cover_file.is_some() || a.picture.is_some();
            let cover = has.then(|| format!("{base}/img/album/{}{key}", a.id));
            let mut v = serde_json::to_value(a).unwrap();
            v["cover"] = json!(cover);
            v["coverSmall"] = json!(if a.cover_small_file.is_some() { Some(format!("{base}/img/album/{}{key}{amp}size=small", a.id)) } else { cover.clone() });
            v
        }).collect::<Vec<_>>(),
        "tracks": l.tracks.iter().map(|t| {
            let mut v = serde_json::to_value(t).unwrap();
            v["stream"] = json!(format!("{base}/stream/{}{key}", t.id));
            v["lyrics"] = json!((t.lyrics_file.is_some() || t.lyrics_tag.is_some()).then(|| format!("{base}/lyrics/{}{key}", t.id)));
            v
        }).collect::<Vec<_>>(),
    })
}

pub fn run(cfg: Arc<Config>) {
    let st = Arc::new(State {
        scanner: Mutex::new(Scanner::new(cfg.music_dir.clone(), Some(cfg.data_dir.join("music-cache.json")))),
        lib: Mutex::new((Arc::new(Library::default()), None)),
        cfg: cfg.clone(),
    });
    let l = st.library();
    log(&format!("Musik aus: {} ({} Interpreten, {} Alben, {} Titel)", cfg.music_dir.display(), l.artists.len(), l.albums.len(), l.tracks.len()));
    for ip in crate::lan_ips() {
        log(&format!("Im Launcher eintragen (Musik): http://{ip}:{}", cfg.music_port));
    }
    crate::serve(cfg.music_port, "Musik", move |req| {
        let st = st.clone();
        let Some((req, url)) = preflight(req, &st.cfg.api_key) else { return };
        let r = handle(&st, &req, &url).unwrap_or_else(|| http::json(404, &json!({ "error": "unbekannt" })));
        let _ = req.respond(http::cors(r));
    });
}

fn handle(st: &State, req: &Request, url: &http::Url) -> Option<Resp> {
    let host = http::get_header(req, "Host").unwrap_or_else(|| format!("127.0.0.1:{}", st.cfg.music_port));
    let base = format!("http://{host}");
    let get = matches!(req.method(), Method::Get | Method::Head);
    if !get {
        return None;
    }
    let p: Vec<&str> = url.parts.iter().map(String::as_str).collect();
    let l = st.library();
    Some(match p.as_slice() {
        ["api", "ping"] => http::json(200, &json!({ "name": st.cfg.name, "version": crate::VERSION, "type": "music", "artists": l.artists.len(), "albums": l.albums.len(), "tracks": l.tracks.len() })),
        ["api", "library"] => http::json(200, &public_library(&l, &base, &st.cfg.name, &st.cfg.api_key)),
        ["stream", id] => match l.track(id) {
            Some(t) => http::file(req, &t.file, t.mime, None, "private, max-age=86400"),
            None => http::text(404, "unbekannter Titel"),
        },
        ["img", "artist", id] => match l.artist(id).and_then(|a| a.image_file.clone()) {
            Some(f) => http::file(req, &f, image_mime(&f).unwrap_or("image/jpeg"), None, "private, max-age=86400"),
            None => http::text(404, "kein Bild"),
        },
        ["img", _, id] => {
            let Some(a) = l.album(id) else { return Some(http::text(404, "kein Bild")) };
            let file = if url.q("size") == Some("small") && a.cover_small_file.is_some() { a.cover_small_file.clone() } else { a.cover_file.clone() };
            if let Some(f) = file {
                http::file(req, &f, image_mime(&f).unwrap_or("image/jpeg"), None, "private, max-age=86400")
            } else if let Some((f, b)) = &a.picture {
                http::file(req, f, &b.mime, Some((b.pos, b.len)), "private, max-age=86400")
            } else {
                http::text(404, "kein Bild")
            }
        }
        ["lyrics", id] => match l.track(id).and_then(read_lyrics) {
            Some(ly) => http::json(200, &ly),
            None => http::json(404, &json!({ "error": "kein Songtext" })),
        },
        _ => return None,
    })
}
