//! Headlines from the Tagesschau app API (editorial order, kicker, photos),
//! the RSS feed as fallback. Article text comes from the item's details JSON.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NewsItem {
    pub title: String,
    pub topline: String,
    pub category: String,
    pub date: String,
    pub image: String,
    pub details: String,
    pub first_sentence: String,
}

fn https(v: &Value) -> String {
    v.as_str()
        .filter(|s| s.starts_with("https://"))
        .unwrap_or("")
        .to_string()
}

/// A download from Tagesschau worked this session (the cache alone is old).
pub static FRESH: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn fetch() -> Vec<NewsItem> {
    let items = fetch_online();
    if !items.is_empty() {
        FRESH.store(true, std::sync::atomic::Ordering::Relaxed);
        save_cache(&items);
        items
    } else {
        load_cached().unwrap_or_default()
    }
}

fn fetch_online() -> Vec<NewsItem> {
    let mut items = Vec::new();
    if let Ok(j) = crate::net::get_json("https://www.tagesschau.de/api2u/homepage/") {
        for n in j["news"].as_array().into_iter().flatten() {
            if n["title"].as_str().is_none() || n["type"] == "webview" {
                continue;
            }
            let share = n["shareURL"].as_str().unwrap_or("");
            if share.contains("/wetter/") {
                continue;
            }
            let v = &n["teaserImage"]["imageVariants"];
            let image = [&v["16x9-960"], &v["16x9-1280"], &v["16x9-640"]]
                .iter()
                .map(|x| https(x))
                .find(|s| !s.is_empty())
                .unwrap_or_default();
            items.push(NewsItem {
                title: n["title"].as_str().unwrap_or("").trim().into(),
                topline: n["topline"].as_str().unwrap_or("").trim().into(),
                category: category_code(n["ressort"].as_str().unwrap_or("")),
                date: n["date"].as_str().unwrap_or("").into(),
                image,
                details: https(&n["details"]),
                first_sentence: n["firstSentence"].as_str().unwrap_or("").into(),
            });
            if items.len() >= 20 {
                break;
            }
        }
    }
    if items.is_empty() {
        if let Ok(b) = crate::net::get_bytes("https://www.tagesschau.de/xml/rss2/", 4 << 20) {
            let xml = String::from_utf8_lossy(&b);
            for block in xml.split("<item>").skip(1) {
                let tag = |name: &str| -> String {
                    let open = format!("<{name}>");
                    let close = format!("</{name}>");
                    let Some(a) = block.find(&open) else {
                        return String::new();
                    };
                    let rest = &block[a + open.len()..];
                    let raw = rest[..rest.find(&close).unwrap_or(rest.len())].trim();
                    let raw = raw
                        .strip_prefix("<![CDATA[")
                        .map(|r| r.trim_end_matches("]]>"))
                        .unwrap_or(raw);
                    unescape(raw.trim())
                };
                let title = tag("title");
                if title.is_empty() {
                    continue;
                }
                items.push(NewsItem {
                    title,
                    category: "inland".into(),
                    date: tag("pubDate"),
                    first_sentence: tag("description"),
                    ..Default::default()
                });
                if items.len() >= 20 {
                    break;
                }
            }
        }
    }
    items
}

fn category_code(value: &str) -> String {
    match value.trim().to_lowercase().as_str() {
        "inland" | "ausland" | "wirtschaft" | "sport" | "wissenschaft" | "panorama" | "kultur" => {
            value.trim().to_lowercase()
        }
        _ => "sonstiges".into(),
    }
}

fn cache_path() -> PathBuf {
    crate::app::data_dir().join("news-cache.json")
}

fn save_cache(items: &[NewsItem]) {
    if let Ok(bytes) = serde_json::to_vec(items) {
        let dir = crate::app::data_dir();
        if std::fs::create_dir_all(&dir).is_ok() {
            let tmp = dir.join("news-cache.json.tmp");
            if std::fs::write(&tmp, bytes).is_ok() {
                let _ = std::fs::rename(tmp, cache_path());
            }
        }
    }
}

/// Headlines saved by the most recent successful refresh, available offline
/// before a new request to Tagesschau completes.
pub fn load_cached() -> Option<Vec<NewsItem>> {
    let bytes = std::fs::read(cache_path()).ok()?;
    let items: Vec<NewsItem> = serde_json::from_slice(&bytes).ok()?;
    (!items.is_empty()).then_some(items)
}

pub fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        if let Some(end) = tail.find(';').filter(|e| *e < 10) {
            let ent = &tail[1..end];
            let ch = match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                _ if ent.starts_with("#x") => u32::from_str_radix(&ent[2..], 16)
                    .ok()
                    .and_then(char::from_u32),
                _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
                _ => None,
            };
            if let Some(c) = ch {
                out.push(c);
                rest = &tail[end + 1..];
                continue;
            }
        }
        out.push('&');
        rest = &tail[1..];
    }
    out.push_str(rest);
    out
}

pub fn strip_html(s: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for c in s.chars() {
        match c {
            '<' => inside = true,
            '>' => inside = false,
            _ if !inside => out.push(c),
            _ => {}
        }
    }
    unescape(&out)
}

/// Paragraphs of an article: (is_heading, text).
pub fn article(details_url: &str) -> Vec<(bool, String)> {
    let Ok(j) = crate::net::get_json(details_url) else {
        return vec![(false, "Artikel konnte nicht geladen werden.".into())];
    };
    let mut out = Vec::new();
    for c in j["content"].as_array().into_iter().flatten() {
        let v = c["value"].as_str().unwrap_or("");
        match c["type"].as_str() {
            Some("text") if !v.is_empty() => out.push((false, strip_html(v))),
            Some("headline") if !v.is_empty() => out.push((true, strip_html(v))),
            _ => {}
        }
    }
    out
}

/// "2026-11-08T07:41:12.437+01:00" -> "07:41"
pub fn time_of(date: &str) -> String {
    date.get(11..16).unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn html() {
        assert_eq!(
            super::strip_html("<strong>Hallo</strong> &amp; Tsch&#252;ss"),
            "Hallo & Tschüss"
        );
    }
}
