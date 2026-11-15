//! Live TV channel lists (M3U): from /storage/tv-launcher/livetv/*.m3u, a
//! USB drive, or a URL (Settings > Live TV). Default: the public German list
//! of iptv-org, so there's something to watch right away.
use std::path::PathBuf;

pub const DEFAULT_URL: &str = "https://iptv-org.github.io/iptv/countries/de.m3u";
/// ARTE's public player API resolves the current German HLS live stream.
pub const ARTE_LIVE_CONFIG_URL: &str = "https://api.arte.tv/api/player/v2/config/de/LIVE";
/// Official ZDF page embeds the currently active HLS master URL in JSON-LD.
pub const ZDF_LIVE_PAGE_URL: &str = "https://www.zdf.de/live-tv/zdf-live-beitrag-100";

#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    pub number: usize,
    pub name: String,
    /// country code ("DE") or the list's own group name
    pub group: String,
    pub logo: String,
    pub url: String,
}

pub fn country_name(cc: &str) -> String {
    let n = match cc {
        "DE" => "Deutschland",
        "AT" => "Österreich",
        "CH" => "Schweiz",
        "US" => "USA",
        "UK" | "GB" => "Großbritannien",
        "IT" => "Italien",
        "AU" => "Australien",
        "ES" => "Spanien",
        "FR" => "Frankreich",
        "NL" => "Niederlande",
        "SE" | "SW" => "Schweden",
        "TR" => "Türkei",
        "PL" => "Polen",
        "CA" => "Kanada",
        "IE" => "Irland",
        "JP" => "Japan",
        "KR" => "Südkorea",
        "FI" => "Finnland",
        "" => "Weitere",
        other => other,
    };
    n.to_string()
}

fn attr(line: &str, name: &str) -> String {
    let key = format!("{name}=\"");
    line.find(&key)
        .and_then(|i| line[i + key.len()..].split('"').next())
        .unwrap_or("")
        .to_string()
}

/// "#EXTINF:-1 tvg-logo=".." group-title="IPTV-DE",Das Erste" + URL line.
pub fn parse_m3u(text: &str) -> Vec<Channel> {
    let mut out = Vec::new();
    let mut meta: Option<(String, String, String)> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("#EXTINF") {
            let name = line
                .rsplit_once(',')
                .map(|x| x.1.trim().to_string())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| attr(line, "tvg-name"));
            let group_title = attr(line, "group-title");
            let country = attr(line, "tvg-country");
            // "IPTV-DE" -> DE; else tvg-country; else the group's own name
            let cc = group_title
                .rsplit_once('-')
                .map(|x| x.1)
                .filter(|c| c.len() == 2 && c.chars().all(|c| c.is_ascii_alphabetic()))
                .map(|c| c.to_uppercase());
            let group = cc
                .or_else(|| {
                    (!country.is_empty())
                        .then(|| country.split(';').next().unwrap_or("").to_uppercase())
                })
                .unwrap_or(group_title.split(';').next().unwrap_or("").to_string());
            let logo = attr(line, "tvg-logo");
            // "(1080p)", "[Geo-blocked]" etc. are noise on a TV
            let name = name
                .split(" (")
                .next()
                .unwrap_or(&name)
                .split(" [")
                .next()
                .unwrap_or(&name)
                .trim()
                .to_string();
            meta = Some((name, group, logo));
        } else if !line.starts_with('#') {
            if let Some((name, group, logo)) = meta.take() {
                if (line.starts_with("http://") || line.starts_with("https://"))
                    && !line.contains("youtube.com")
                    && !line.contains("youtu.be")
                {
                    out.push(Channel {
                        number: out.len() + 1,
                        name,
                        group,
                        logo: if logo.starts_with("http") {
                            logo
                        } else {
                            String::new()
                        },
                        url: line.to_string(),
                    });
                }
            }
        }
    }
    out
}

pub fn local_files() -> Vec<PathBuf> {
    let mut dirs = vec![
        crate::app::data_dir().join("..").join("livetv"),
        crate::app::data_dir().join("livetv"),
    ];
    dirs.extend(crate::platform::system::media_roots());
    let mut out = Vec::new();
    for d in dirs {
        if let Ok(r) = std::fs::read_dir(&d) {
            let mut v: Vec<PathBuf> = r
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    matches!(
                        p.extension()
                            .and_then(|e| e.to_str())
                            .map(|e| e.to_lowercase())
                            .as_deref(),
                        Some("m3u") | Some("m3u8")
                    )
                })
                .collect();
            v.sort();
            out.extend(v);
        }
    }
    out
}

/// The channel list: a local file wins over the URL. (channels, where from)
pub fn load(url: &str) -> Result<(Vec<Channel>, String), String> {
    for f in local_files() {
        if let Ok(b) = std::fs::read(&f) {
            let text = tvcore::util::decode_text_file(&b);
            // a media playlist of one stream is not a channel list
            if text.contains("#EXT-X-TARGETDURATION") {
                continue;
            }
            let ch = parse_m3u(&text);
            if !ch.is_empty() {
                return Ok((ch, f.display().to_string()));
            }
        }
    }
    let url = if url.trim().is_empty() {
        DEFAULT_URL
    } else {
        url.trim()
    };
    let b = crate::net::get_bytes(url, 16 << 20)?;
    let ch = parse_m3u(&String::from_utf8_lossy(&b));
    if ch.is_empty() {
        return Err("Die Senderliste ist leer".into());
    }
    // kept for the next start: Live TV opens at once, before the network
    let dir = crate::app::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let tmp = dir.join("livetv-cache.m3u.tmp");
    if std::fs::write(&tmp, format!("#CACHE-SOURCE:{url}\n").into_bytes().into_iter().chain(b).collect::<Vec<u8>>()).is_ok() {
        let _ = std::fs::rename(tmp, cache_path());
    }
    Ok((ch, url.to_string()))
}

fn cache_path() -> PathBuf {
    crate::app::data_dir().join("livetv-cache.m3u")
}

/// The list the last download brought, for the address `url` (the same
/// address as configured now), read from the Pi's storage: no network.
pub fn load_cached(url: &str) -> Option<(Vec<Channel>, String)> {
    let want = if url.trim().is_empty() { DEFAULT_URL } else { url.trim() };
    let b = std::fs::read(cache_path()).ok()?;
    let text = String::from_utf8_lossy(&b);
    let first = text.lines().next()?;
    if first.strip_prefix("#CACHE-SOURCE:") != Some(want) {
        return None;
    }
    let ch = parse_m3u(&text);
    (!ch.is_empty()).then(|| (ch, format!("{want} (gespeichert)")))
}

/// The channel watched longest (seconds per address), if any was watched.
pub fn most_watched<'a>(ch: &'a [Channel], watch: &std::collections::HashMap<String, u64>) -> Option<&'a Channel> {
    ch.iter()
        .filter_map(|c| watch.get(&c.url).map(|s| (c, *s)))
        .filter(|(_, s)| *s > 0)
        .max_by_key(|(_, s)| *s)
        .map(|(c, _)| c)
}

/// Groups for the guide: Favoriten, Alle, then by country (Germany first).
pub fn groups(ch: &[Channel]) -> Vec<(String, String)> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for c in ch {
        match counts.iter_mut().find(|x| x.0 == c.group) {
            Some(x) => x.1 += 1,
            None => counts.push((c.group.clone(), 1)),
        }
    }
    counts.sort_by(|a, b| {
        (b.0 == "DE")
            .cmp(&(a.0 == "DE"))
            .then(b.1.cmp(&a.1))
            .then(a.0.cmp(&b.0))
    });
    let mut g = vec![
        ("fav".to_string(), "Favoriten".to_string()),
        ("all".to_string(), "Alle Sender".to_string()),
    ];
    if counts.len() > 1 {
        g.extend(counts.into_iter().map(|(id, _)| {
            (
                id.clone(),
                if id.len() == 2 {
                    country_name(&id)
                } else if id.is_empty() {
                    "Weitere".into()
                } else {
                    id
                },
            )
        }));
    }
    g
}

pub fn in_group<'a>(ch: &'a [Channel], group: &str, favs: &[String]) -> Vec<&'a Channel> {
    match group {
        "fav" => ch.iter().filter(|c| favs.contains(&c.url)).collect(),
        "all" => ch.iter().collect(),
        g => ch.iter().filter(|c| c.group == g).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn m3u() {
        let t = "#EXTM3U\n#EXTINF:-1 tvg-id=\"a\" tvg-logo=\"https://l/a.png\" group-title=\"IPTV-DE\",Das Erste (1080p)\nhttps://x/a.m3u8\n\
#EXTINF:-1 tvg-country=\"AT\" group-title=\"News\",ORF 1 [Geo-blocked]\nhttps://x/b.m3u8\n#EXTINF:-1,YT\nhttps://www.youtube.com/watch?v=1\n\
#EXTINF:-1 group-title=\"Musik\",Radio\nhttp://x/c.m3u8\n";
        let c = parse_m3u(t);
        assert_eq!(c.len(), 3);
        assert_eq!(
            (c[0].name.as_str(), c[0].group.as_str(), c[0].number),
            ("Das Erste", "DE", 1)
        );
        assert_eq!((c[1].name.as_str(), c[1].group.as_str()), ("ORF 1", "AT"));
        assert_eq!(c[2].group, "Musik");
        let g = groups(&c);
        assert_eq!(g[2].1, "Deutschland");
        let mut watch = std::collections::HashMap::new();
        assert!(most_watched(&c, &watch).is_none());
        watch.insert("https://x/b.m3u8".to_string(), 900u64);
        watch.insert("http://x/c.m3u8".to_string(), 60u64);
        watch.insert("https://gone/x.m3u8".to_string(), 99_999u64);
        assert_eq!(most_watched(&c, &watch).unwrap().name, "ORF 1");
        assert_eq!(in_group(&c, "fav", &["http://x/c.m3u8".into()]).len(), 1);
    }
}
