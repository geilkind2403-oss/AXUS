//! YouTube without a browser: the same internal API ("InnerTube") YouTube's
//! own TV and phone apps use. Search and feeds come from the web client's
//! API, the stream addresses from a client whose URLs need no JavaScript to
//! unlock (the Android VR app). The Pi 3 then plays H.264 (DASH) itself.
//!
//! This is unofficial: when YouTube changes something it can break, and the
//! client list below is what to update then.
use serde_json::{json, Value};
use std::sync::Mutex;

#[derive(Clone, Debug)]
pub struct Video {
    pub id: String,
    pub title: String,
    pub channel: String,
    pub length: String,
    pub views: String,
    pub age: String,
    pub thumb: String,
}

static VISITOR: Mutex<String> = Mutex::new(String::new());

const WEB_VERSION: &str = "2.20250925.01.00";

fn web_context() -> Value {
    let visitor = VISITOR.lock().unwrap().clone();
    let mut c = json!({ "client": { "clientName": "WEB", "clientVersion": WEB_VERSION, "hl": "de", "gl": "DE" } });
    if !visitor.is_empty() {
        c["client"]["visitorData"] = json!(visitor);
    }
    c
}

fn call(endpoint: &str, body: Value, headers: &[(&str, &str)]) -> Result<Value, String> {
    let url = format!("https://www.youtube.com/youtubei/v1/{endpoint}?prettyPrint=false");
    let j = crate::net::post_json(&url, &body, headers)?;
    if let Some(v) = j["responseContext"]["visitorData"].as_str() {
        *VISITOR.lock().unwrap() = v.to_string();
    }
    Ok(j)
}

fn text(v: &Value) -> String {
    if let Some(s) = v["simpleText"].as_str() {
        return s.to_string();
    }
    if let Some(s) = v["content"].as_str() {
        return s.to_string();
    }
    v["runs"]
        .as_array()
        .map(|r| {
            r.iter()
                .filter_map(|x| x["text"].as_str())
                .collect::<String>()
        })
        .unwrap_or_default()
}

/// Every video in a response, wherever YouTube put it (old videoRenderer and
/// the newer lockupViewModel layout).
fn collect(v: &Value, out: &mut Vec<Video>) {
    match v {
        Value::Object(m) => {
            if let Some(r) = m
                .get("videoRenderer")
                .or_else(|| m.get("gridVideoRenderer"))
                .or_else(|| m.get("compactVideoRenderer"))
            {
                if let Some(id) = r["videoId"].as_str() {
                    let length = text(&r["lengthText"]);
                    if !out.iter().any(|x| x.id == id) {
                        out.push(Video {
                            id: id.into(),
                            title: text(&r["title"]),
                            channel: text(&r["ownerText"]).to_string()
                                + &text(&r["shortBylineText"])
                                    .chars()
                                    .take(if text(&r["ownerText"]).is_empty() {
                                        200
                                    } else {
                                        0
                                    })
                                    .collect::<String>(),
                            length: if length.is_empty() {
                                "LIVE".into()
                            } else {
                                length
                            },
                            views: text(&r["shortViewCountText"]),
                            age: text(&r["publishedTimeText"]),
                            thumb: format!("https://i.ytimg.com/vi/{id}/mqdefault.jpg"),
                        });
                    }
                }
                return;
            }
            if let Some(l) = m.get("lockupViewModel") {
                if l["contentType"] == "LOCKUP_CONTENT_TYPE_VIDEO" {
                    if let Some(id) = l["contentId"].as_str() {
                        let meta = &l["metadata"]["lockupMetadataViewModel"];
                        let rows: Vec<String> = meta["metadata"]["contentMetadataViewModel"]
                            ["metadataRows"]
                            .as_array()
                            .map(|rows| {
                                rows.iter()
                                    .flat_map(|r| {
                                        r["metadataParts"].as_array().cloned().unwrap_or_default()
                                    })
                                    .map(|p| text(&p["text"]))
                                    .collect()
                            })
                            .unwrap_or_default();
                        let badge = find_key(&l["contentImage"], "thumbnailBadgeViewModel")
                            .map(|b| b["text"].as_str().unwrap_or("").to_string())
                            .unwrap_or_default();
                        if !out.iter().any(|x| x.id == id) {
                            out.push(Video {
                                id: id.into(),
                                title: text(&meta["title"]),
                                channel: rows.first().cloned().unwrap_or_default(),
                                length: badge,
                                views: rows.get(1).cloned().unwrap_or_default(),
                                age: rows.get(2).cloned().unwrap_or_default(),
                                thumb: format!("https://i.ytimg.com/vi/{id}/mqdefault.jpg"),
                            });
                        }
                    }
                }
                return;
            }
            for v in m.values() {
                collect(v, out);
            }
        }
        Value::Array(a) => {
            for v in a {
                collect(v, out);
            }
        }
        _ => {}
    }
}

fn find_key<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Object(m) => m
            .get(key)
            .or_else(|| m.values().find_map(|x| find_key(x, key))),
        Value::Array(a) => a.iter().find_map(|x| find_key(x, key)),
        _ => None,
    }
}

pub fn search(q: &str) -> Result<Vec<Video>, String> {
    // "EgIQAQ%3D%3D": only videos (no channels / playlists)
    let j = call(
        "search",
        json!({ "context": web_context(), "query": q, "params": "EgIQAQ==" }),
        &[],
    )?;
    let mut out = Vec::new();
    collect(&j["contents"], &mut out);
    out.retain(|v| !v.title.is_empty());
    Ok(out)
}

/// The start page: YouTube's own feed when it gives one (logged out it often
/// doesn't), else popular searches.
pub fn home() -> Vec<(String, Vec<Video>)> {
    let mut rows = Vec::new();
    if let Ok(j) = call(
        "browse",
        json!({ "context": web_context(), "browseId": "FEwhat_to_watch" }),
        &[],
    ) {
        let mut v = Vec::new();
        collect(&j["contents"], &mut v);
        v.retain(|x| !x.title.is_empty() && x.length != "LIVE");
        if v.len() >= 6 {
            rows.push(("Empfohlen".to_string(), v));
        }
    }
    rows
}

#[allow(dead_code)]
#[derive(Clone)]
pub struct Stream {
    pub video: String,
    pub audio: String,
    /// a single muxed file (360p) when there's no DASH pair
    pub muxed: Option<String>,
    pub title: String,
    pub duration: f64,
    pub height: u32,
}

struct Client {
    name: &'static str,
    id: &'static str,
    version: &'static str,
    ua: &'static str,
    extra: fn(&mut Value),
}

const CLIENTS: [Client; 2] = [
    Client {
        name: "ANDROID_VR",
        id: "28",
        version: "1.62.27",
        ua: "com.google.android.apps.youtube.vr.oculus/1.62.27 (Linux; U; Android 12L; eureka-user Build/SQ3A.220605.009.A1) gzip",
        extra: |c| {
            c["deviceMake"] = json!("Oculus");
            c["deviceModel"] = json!("Quest 3");
            c["androidSdkVersion"] = json!(32);
            c["osName"] = json!("Android");
            c["osVersion"] = json!("12L");
        },
    },
    Client {
        name: "IOS",
        id: "5",
        version: "20.10.4",
        ua: "com.google.ios.youtube/20.10.4 (iPhone16,2; U; CPU iOS 18_3_2 like Mac OS X;)",
        extra: |c| {
            c["deviceMake"] = json!("Apple");
            c["deviceModel"] = json!("iPhone16,2");
            c["osName"] = json!("iPhone");
            c["osVersion"] = json!("18.3.2.22D82");
        },
    },
];

/// Stream URLs the Pi 3 can play: H.264 up to 1080p (30 fps preferred --
/// 1080p60 is too much for its decoder), AAC sound.
pub fn stream(id: &str) -> Result<Stream, String> {
    let mut last_err = String::from("kein Stream");
    for c in &CLIENTS {
        let mut client =
            json!({ "clientName": c.name, "clientVersion": c.version, "hl": "de", "gl": "DE" });
        (c.extra)(&mut client);
        let visitor = VISITOR.lock().unwrap().clone();
        if !visitor.is_empty() {
            client["visitorData"] = json!(visitor);
        }
        let body = json!({ "context": { "client": client }, "videoId": id, "contentCheckOk": true, "racyCheckOk": true });
        let j = match call(
            "player",
            body,
            &[
                ("User-Agent", c.ua),
                ("X-YouTube-Client-Name", c.id),
                ("X-YouTube-Client-Version", c.version),
            ],
        ) {
            Ok(j) => j,
            Err(e) => {
                last_err = e;
                continue;
            }
        };
        let status = j["playabilityStatus"]["status"].as_str().unwrap_or("");
        if status != "OK" {
            last_err = j["playabilityStatus"]["reason"]
                .as_str()
                .unwrap_or("Video nicht verfügbar")
                .to_string();
            continue;
        }
        let sd = &j["streamingData"];
        let fmts = sd["adaptiveFormats"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mime = |f: &Value| f["mimeType"].as_str().unwrap_or("").to_string();
        let mut videos: Vec<&Value> = fmts
            .iter()
            .filter(|f| {
                mime(f).starts_with("video/mp4") && mime(f).contains("avc1") && f["url"].is_string()
            })
            .collect();
        videos.retain(|f| f["height"].as_u64().unwrap_or(0) <= 1080);
        videos.sort_by_key(|f| {
            let h = f["height"].as_u64().unwrap_or(0);
            let fps = f["fps"].as_u64().unwrap_or(30);
            // 1080p only at <=30 fps; otherwise as high as possible, lower fps first
            let ok = !(h > 720 && fps > 30);
            (ok, h, std::cmp::Reverse(fps))
        });
        let audio = fmts
            .iter()
            .filter(|f| mime(f).starts_with("audio/mp4") && f["url"].is_string())
            .max_by_key(|f| f["bitrate"].as_u64().unwrap_or(0).min(200_000));
        let muxed = sd["formats"]
            .as_array()
            .and_then(|a| a.iter().find(|f| f["itag"] == 18))
            .and_then(|f| f["url"].as_str())
            .map(String::from);
        let details = &j["videoDetails"];
        let duration = details["lengthSeconds"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        if let (Some(v), Some(a)) = (videos.last(), audio) {
            return Ok(Stream {
                video: v["url"].as_str().unwrap().into(),
                audio: a["url"].as_str().unwrap().into(),
                muxed,
                title: details["title"].as_str().unwrap_or("").into(),
                duration,
                height: v["height"].as_u64().unwrap_or(0) as u32,
            });
        }
        if muxed.is_some() {
            return Ok(Stream {
                video: String::new(),
                audio: String::new(),
                muxed,
                title: details["title"].as_str().unwrap_or("").into(),
                duration,
                height: 360,
            });
        }
        if sd["hlsManifestUrl"].is_string() {
            last_err = "Livestreams gehen noch nicht".into();
        }
    }
    Err(last_err)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn collects_both_layouts() {
        let j = json!({ "a": [ { "videoRenderer": { "videoId": "abc", "title": { "runs": [{ "text": "Hallo" }] }, "lengthText": { "simpleText": "3:21" }, "ownerText": { "runs": [{ "text": "Kanal" }] } } },
            { "lockupViewModel": { "contentType": "LOCKUP_CONTENT_TYPE_VIDEO", "contentId": "xyz", "metadata": { "lockupMetadataViewModel": { "title": { "content": "Neu" } } } } } ] });
        let mut v = Vec::new();
        collect(&j, &mut v);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].title, "Hallo");
        assert_eq!(v[0].channel, "Kanal");
        assert_eq!(v[1].id, "xyz");
    }
}
