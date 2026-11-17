//! TV Launcher server -- runs on the PC and serves the film library to the
//! Pi on port 8420, including /play/<id>?v=h264 for any film (the Pi 3 only
//! decodes H.264).
//!
//! Settings: environment variables, or a `tv-server.conf` (KEY=value lines)
//! next to the program -- handy on Windows, where it's started by double click.
//!   MEDIA_DIR   film library root          (default: ./media)
//!   DATA_DIR    progress + caches          (default: ./data)
//!   PORT        film port                  (default: 8420)
//!   SERVER_NAME shown in the launcher      (default: host name)
//!   API_KEY     optional shared key (?key=... or header X-Api-Key)
mod films;
mod http;
mod video;

use std::path::PathBuf;
use std::sync::Arc;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn log(msg: &str) {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let s = secs % 86400;
    println!("{:02}:{:02}:{:02} {msg}", s / 3600, s / 60 % 60, s % 60);
}

pub struct Config {
    pub media_dir: PathBuf,
    pub data_dir: PathBuf,
    pub port: u16,
    pub name: String,
    pub api_key: String,
}

fn load_config() -> Config {
    let base = std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.to_path_buf())).unwrap_or_else(|| PathBuf::from("."));
    let mut file_vals = std::collections::HashMap::new();
    for dir in [std::env::current_dir().unwrap_or_default(), base.clone()] {
        if let Ok(s) = std::fs::read_to_string(dir.join("tv-server.conf")) {
            for line in s.lines() {
                let line = line.trim();
                if line.starts_with('#') {
                    continue;
                }
                if let Some((k, v)) = line.split_once('=') {
                    file_vals.entry(k.trim().to_string()).or_insert_with(|| v.trim().trim_matches('"').to_string());
                }
            }
        }
    }
    let get = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty()).or_else(|| file_vals.get(k).cloned());
    let dir = |k: &str, d: &str| get(k).map(PathBuf::from).unwrap_or_else(|| base.join(d));
    Config {
        media_dir: dir("MEDIA_DIR", "media"),
        data_dir: dir("DATA_DIR", "data"),
        port: get("PORT").and_then(|p| p.parse().ok()).unwrap_or(8420),
        name: get("SERVER_NAME").unwrap_or_else(hostname),
        api_key: get("API_KEY").unwrap_or_default(),
    }
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "TV-Server".into())
}

/// LAN addresses, for the "enter this in the launcher" hint.
pub fn lan_ips() -> Vec<String> {
    // Connecting a UDP socket sends nothing but picks the outgoing interface.
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok();
    s.and_then(|s| s.connect("192.168.0.1:9").ok().and_then(|_| s.local_addr().ok()))
        .map(|a| a.ip().to_string())
        .filter(|ip| ip != "0.0.0.0")
        .into_iter()
        .collect()
}

/// Accepts connections and answers each request on its own thread (films
/// stream for hours; nothing may block the library listing meanwhile).
pub fn serve<H>(port: u16, what: &str, handler: H)
where
    H: Fn(tiny_http::Request) + Send + Sync + 'static,
{
    let server = match tiny_http::Server::http(("0.0.0.0", port)) {
        Ok(s) => s,
        Err(e) => {
            log(&format!("Port {port} ({what}) geht nicht: {e} -- läuft der Server schon?"));
            std::process::exit(1);
        }
    };
    let handler = Arc::new(handler);
    for req in server.incoming_requests() {
        let h = handler.clone();
        std::thread::spawn(move || h(req));
    }
}

/// Answers OPTIONS, checks the API key. Returns the request if it should be handled.
pub fn preflight(req: tiny_http::Request, api_key: &str) -> Option<(tiny_http::Request, http::Url)> {
    let url = http::Url::parse(req.url());
    if *req.method() == tiny_http::Method::Options {
        let mut r = http::cors(http::bytes(204, "text/plain", vec![]));
        r.add_header(http::header("Access-Control-Allow-Methods", "GET, HEAD, POST, OPTIONS"));
        let _ = req.respond(r);
        return None;
    }
    if !api_key.is_empty() && url.q("key") != Some(api_key) && http::get_header(&req, "X-Api-Key").as_deref() != Some(api_key) {
        let _ = req.respond(http::cors(http::json(401, &serde_json::json!({ "error": "Falscher API-Schlüssel" }))));
        return None;
    }
    Some((req, url))
}

pub fn key_query(api_key: &str) -> String {
    if api_key.is_empty() { String::new() } else { format!("?key={}", http::percent_encode(api_key)) }
}

fn main() {
    let cfg = Arc::new(load_config());
    log(&format!("TV Launcher Server {VERSION} läuft"));
    films::run(cfg);
}
