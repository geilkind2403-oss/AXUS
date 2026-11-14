//! The film library: films on the Pi (USB drives, /storage/videos) and on
//! the PC server, as one list.
use crate::app::{Msg, Slot};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::mpsc::Sender;

#[derive(Clone, Debug)]
pub enum Origin {
    Local(PathBuf),
    Server {
        base: String,
        id: String,
        /// direct file (H.264 MP4: the Pi plays it as is, with Range seeking)
        stream: String,
        /// the server converts on the fly (anything else)
        convert: Option<String>,
        codec: Option<String>,
        container_ok: bool,
        audio: Vec<(String, String)>,
        subs: Vec<(u32, String, bool)>,
        sub_url: Option<String>,
        pick: (usize, i64),
    },
}

#[derive(Clone, Debug)]
pub struct Film {
    pub key: String,
    pub title: String,
    pub info: String,
    pub fsk: String,
    pub genre: String,
    pub cast: String,
    pub crew: String,
    pub duration: f64,
    pub icon: String,
    pub banner: String,
    pub origin: Origin,
    pub position: f64,
}

impl Film {
    pub fn on_server(&self) -> bool {
        matches!(self.origin, Origin::Server { .. })
    }
    /// Short text under the title: "2 Std. 1 Min. · FSK 12 · Abenteuer"
    pub fn meta(&self) -> String {
        let mut v = Vec::new();
        if self.duration >= 60.0 {
            let m = (self.duration / 60.0).round() as u64;
            v.push(if m >= 60 {
                format!("{} Std. {} Min.", m / 60, m % 60)
            } else {
                format!("{m} Min.")
            });
        }
        if !self.fsk.is_empty() {
            let f = self.fsk.trim();
            v.push(if f.to_lowercase().starts_with("fsk") {
                f.to_string()
            } else {
                format!("FSK {f}")
            });
        }
        if !self.genre.is_empty() {
            v.push(self.genre.clone());
        }
        v.join(" · ")
    }
    /// Plot text without the "Genre: ..." lines.
    pub fn plot(&self) -> String {
        self.info
            .lines()
            .filter(|l| !l.to_lowercase().starts_with("genre:"))
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string()
    }
}

#[derive(Clone, Default)]
pub struct FilmList {
    pub films: Vec<Film>,
    pub errors: Vec<String>,
}

pub struct FilmLibrary {
    pub list: Slot<FilmList>,
}

/// Where films live on the Pi.
pub fn local_roots(extra: &[String]) -> Vec<PathBuf> {
    let mut v = vec![
        PathBuf::from("/storage/videos"),
        PathBuf::from("/storage/filme"),
    ];
    v.extend(crate::platform::system::media_roots());
    v.extend(extra.iter().map(PathBuf::from));
    v.retain(|p| p.is_dir());
    v
}

impl FilmLibrary {
    pub fn new() -> FilmLibrary {
        FilmLibrary {
            list: Slot::empty(),
        }
    }

    pub fn refresh(
        &mut self,
        tx: &Sender<Msg>,
        server: &str,
        extra: &[String],
        positions: &std::collections::HashMap<String, f64>,
    ) {
        let server = server.trim_end_matches('/').to_string();
        let extra = extra.to_vec();
        let positions = positions.clone();
        let slot = self.list.clone();
        let tx = tx.clone();
        std::thread::spawn(move || {
            let mut out = FilmList::default();
            for root in local_roots(&extra) {
                for f in tvcore::films::scan(&root) {
                    let key = format!("local:{}", f.file.display());
                    let duration = tvcore::util::mp4_duration(&f.file).unwrap_or(0) as f64;
                    out.films.push(Film {
                        position: positions.get(&key).copied().unwrap_or(0.0),
                        key,
                        title: f.title.clone(),
                        info: f.info.clone().unwrap_or_default(),
                        fsk: f.fsk.clone().unwrap_or_default(),
                        genre: f.genre.clone().unwrap_or_default(),
                        cast: f.cast.clone().unwrap_or_default(),
                        crew: f.crew.clone().unwrap_or_default(),
                        duration,
                        icon: f
                            .icon_file
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_default(),
                        banner: f
                            .banner_file
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_default(),
                        origin: Origin::Local(f.file.clone()),
                    });
                }
            }
            if !server.is_empty() {
                match crate::net::get_json(&format!("{server}/api/library")) {
                    Ok(Value::Array(list)) => {
                        for f in list {
                            out.films.push(server_film(&server, &f));
                        }
                    }
                    Ok(_) => out.errors.push("PC-Server: unerwartete Antwort".into()),
                    Err(e) => out.errors.push(format!("PC-Server nicht erreichbar ({e})")),
                }
            }
            out.films
                .sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
            slot.set(out);
            let _ = tx.send(Msg::Redraw);
        });
    }
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

fn server_film(base: &str, f: &Value) -> Film {
    let id = s(&f["id"]);
    let play = &f["play"];
    let rel = s(&f["relativePath"]).to_lowercase();
    let container_ok = rel.ends_with(".mp4") || rel.ends_with(".m4v") || rel.ends_with(".mov");
    Film {
        key: format!("server:{id}"),
        title: s(&f["title"]),
        info: s(&f["info"]),
        fsk: s(&f["fsk"]),
        genre: s(&f["genre"]),
        cast: s(&f["cast"]),
        crew: s(&f["crew"]),
        duration: f["duration"].as_f64().unwrap_or(0.0),
        icon: s(&f["icon"]),
        banner: s(&f["banner"]),
        position: f["position"].as_f64().unwrap_or(0.0),
        origin: Origin::Server {
            base: base.to_string(),
            stream: format!("{base}/stream/{id}"),
            convert: f["convert"]
                .as_str()
                .map(String::from)
                .or_else(|| (play.is_object()).then(|| format!("{base}/play/{id}"))),
            codec: f["codec"].as_str().map(String::from),
            container_ok,
            audio: play["audio"]
                .as_array()
                .map(|a| a.iter().map(|t| (s(&t["lang"]), s(&t["title"]))).collect())
                .unwrap_or_default(),
            subs: play["subs"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|t| {
                            (
                                t["n"].as_u64().unwrap_or(0) as u32,
                                s(&t["lang"]),
                                t["forced"].as_bool().unwrap_or(false),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
            sub_url: play["sub"].as_str().map(String::from),
            pick: (
                play["a"].as_u64().unwrap_or(0) as usize,
                play["s"].as_i64().unwrap_or(-1),
            ),
            id,
        },
    }
}

/// Resume position back to the PC server (shared by every device).
pub fn report_progress(base: &str, id: &str, secs: f64) {
    let url = format!("{base}/api/progress/{id}");
    let body = serde_json::json!({ "seconds": secs.round() });
    std::thread::spawn(move || {
        let _ = crate::net::agent()
            .post(&url)
            .set("Content-Type", "application/json")
            .send_string(&body.to_string());
    });
}
