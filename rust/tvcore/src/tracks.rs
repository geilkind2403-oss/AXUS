//! Which audio track / subtitle a film starts with, and WebVTT time shifting.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AudioTrack {
    pub lang: String,
    pub title: String,
    pub codec: String,
    pub channels: u32,
    #[serde(default)]
    pub default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubTrack {
    /// Index among all subtitle streams of the file.
    pub n: u32,
    pub lang: String,
    pub title: String,
    pub forced: bool,
    #[serde(default)]
    pub default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VideoInfo {
    pub codec: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProbeInfo {
    pub duration: Option<u64>,
    pub video: Option<VideoInfo>,
    pub audio: Vec<AudioTrack>,
    pub subs: Vec<SubTrack>,
}

pub fn is_german(lang: &str) -> bool {
    matches!(lang.to_lowercase().as_str(), "ger" | "deu" | "de" | "german" | "deutsch")
}

/// German audio when there is one; German forced subs to German audio,
/// German subs to foreign audio. Returns (audio index, subtitle n or -1).
pub fn pick_tracks(info: &ProbeInfo) -> (usize, i64) {
    let audio = &info.audio;
    let a = audio
        .iter()
        .position(|t| is_german(&t.lang))
        .or_else(|| audio.iter().position(|t| t.default))
        .unwrap_or(0);
    let german_audio = audio.get(a).map(|t| is_german(&t.lang)).unwrap_or(false);
    let subs: Vec<&SubTrack> = info.subs.iter().filter(|s| is_german(&s.lang)).collect();
    let sub = if german_audio {
        subs.iter().find(|s| s.forced).copied()
    } else {
        subs.iter().find(|s| !s.forced).copied().or(subs.first().copied())
    };
    (a, sub.map(|s| s.n as i64).unwrap_or(-1))
}

fn to_sec(s: &str) -> f64 {
    let p: Vec<f64> = s.split(':').map(|x| x.parse().unwrap_or(0.0)).collect();
    if p.len() == 3 { p[0] * 3600.0 + p[1] * 60.0 + p[2] } else { p[0] * 60.0 + p.get(1).copied().unwrap_or(0.0) }
}

pub fn fmt_vtt(x: f64) -> String {
    let ms = (x.max(0.0) * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02}.{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

/// One WebVTT cue line "a --> b": (start, end) seconds.
pub fn parse_cue_times(line: &str) -> Option<(f64, f64)> {
    let (a, rest) = line.split_once("-->")?;
    let b = rest.split_whitespace().next()?;
    let a = a.trim();
    let ok = |s: &str| s.contains(':') && s.contains('.') && s.chars().all(|c| c.is_ascii_digit() || c == ':' || c == '.');
    if !ok(a) || !ok(b) {
        return None;
    }
    Some((to_sec(a), to_sec(b)))
}

/// Shifts every cue by -t, drops cues that ended before t.
pub fn shift_vtt(text: &str, t: f64) -> String {
    if t <= 0.0 {
        return text.to_string();
    }
    let norm = text.replace('\r', "");
    let mut blocks = norm.split("\n\n").filter(|b| !b.trim().is_empty());
    let mut out = vec![blocks.next().unwrap_or("WEBVTT").to_string()];
    for cue in blocks {
        let mut lines: Vec<String> = cue.trim_matches('\n').lines().map(String::from).collect();
        let Some(i) = lines.iter().position(|l| parse_cue_times(l).is_some()) else { continue };
        let (s, e) = parse_cue_times(&lines[i]).unwrap();
        if e - t <= 0.0 {
            continue;
        }
        let tail = lines[i].split_once("-->").unwrap().1.trim().splitn(2, char::is_whitespace).nth(1).map(|x| format!(" {x}")).unwrap_or_default();
        lines[i] = format!("{} --> {}{}", fmt_vtt(s - t), fmt_vtt(e - t), tail);
        out.push(lines.join("\n"));
    }
    out.join("\n\n") + "\n"
}

/// Cues of a WebVTT file: (start, end, text) -- the Pi renders subtitles itself.
pub fn parse_vtt(text: &str) -> Vec<(f64, f64, String)> {
    let norm = text.replace('\r', "");
    let mut out = Vec::new();
    for cue in norm.split("\n\n") {
        let lines: Vec<&str> = cue.trim_matches('\n').lines().collect();
        if let Some(i) = lines.iter().position(|l| parse_cue_times(l).is_some()) {
            let (s, e) = parse_cue_times(lines[i]).unwrap();
            let body = lines[i + 1..].join("\n");
            let body = strip_tags(&body);
            if !body.trim().is_empty() {
                out.push((s, e, body));
            }
        }
    }
    out
}

fn strip_tags(s: &str) -> String {
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
    out.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("\\N", "\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn t(lang: &str, default: bool) -> AudioTrack {
        AudioTrack { lang: lang.into(), default, ..Default::default() }
    }
    #[test]
    fn picks_german() {
        let info = ProbeInfo {
            audio: vec![t("eng", true), t("ger", false)],
            subs: vec![SubTrack { n: 0, lang: "ger".into(), forced: true, ..Default::default() }, SubTrack { n: 1, lang: "ger".into(), ..Default::default() }],
            ..Default::default()
        };
        assert_eq!(pick_tracks(&info), (1, 0));
        let info2 = ProbeInfo { audio: vec![t("eng", true)], subs: info.subs.clone(), ..Default::default() };
        assert_eq!(pick_tracks(&info2), (0, 1));
    }
    #[test]
    fn shifts() {
        let v = "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nalt\n\n00:01:00.500 --> 00:01:02.000 line:90%\nneu\n";
        let s = shift_vtt(v, 30.0);
        assert_eq!(s, "WEBVTT\n\n00:00:30.500 --> 00:00:32.000 line:90%\nneu\n");
        let cues = parse_vtt(v);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[1].2, "neu");
    }
}
