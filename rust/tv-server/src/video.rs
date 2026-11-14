//! ffmpeg/ffprobe side of the film server: probing (cached), keyframe lookup,
//! live remux / transcode to fragmented MP4, subtitle extraction.
//!
//! The Raspberry Pi 3 decodes only H.264 in hardware, so it asks with ?v=h264
//! for anything else (HEVC, VP9, AV1, MPEG-4, VC-1 ...): the PC converts. The
//! picture is copied untouched whenever the client can take it.
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use tvcore::tracks::{AudioTrack, ProbeInfo, SubTrack, VideoInfo};
use tvcore::util::{ext_lower, mtime_ms, sha1_hex};

pub const REMUX_EXT: [&str; 11] = [".mkv", ".avi", ".ts", ".m2ts", ".mts", ".wmv", ".flv", ".mpg", ".mpeg", ".vob", ".divx"];
const COPY_VIDEO: [&str; 4] = ["h264", "hevc", "vp9", "av1"];
const COPY_AUDIO: [&str; 2] = ["aac", "mp3"];
const TEXT_SUBS: [&str; 7] = ["subrip", "srt", "ass", "ssa", "webvtt", "mov_text", "text"];

fn find_tool(name: &str, env: &str) -> Option<String> {
    let exe = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    let here = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_default();
    let candidates = [
        std::env::var(env).ok(),
        Some(here.join(&exe).to_string_lossy().into_owned()),
        Some(here.join("ffmpeg").join("bin").join(&exe).to_string_lossy().into_owned()),
        Some(name.to_string()),
    ];
    candidates.into_iter().flatten().find(|c| {
        let mut cmd = Command::new(c);
        cmd.arg("-version").stdout(Stdio::null()).stderr(Stdio::null());
        hide_window(&mut cmd);
        cmd.status().map(|s| s.success()).unwrap_or(false)
    })
}

#[cfg(windows)]
pub fn hide_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
}
#[cfg(not(windows))]
pub fn hide_window(_: &mut Command) {}

pub struct VideoTools {
    pub ffmpeg: Option<String>,
    ffprobe: Option<String>,
    cache_dir: PathBuf,
    probes: Mutex<HashMap<String, (String, ProbeInfo)>>,
    sub_lock: Mutex<()>,
}

/// Kills ffmpeg when the response is done or the player went away.
pub struct ChildReader(pub Child);
impl std::io::Read for ChildReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.stdout.as_mut().map(|s| s.read(buf)).unwrap_or(Ok(0))
    }
}
impl Drop for ChildReader {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub fn needs_remux(file: &Path) -> bool {
    REMUX_EXT.contains(&ext_lower(file).as_str())
}

impl VideoTools {
    pub fn new(cache_dir: PathBuf) -> Self {
        let ffmpeg = find_tool("ffmpeg", "FFMPEG_PATH");
        if ffmpeg.is_none() {
            crate::log("Hinweis: ffmpeg nicht gefunden -- MKV-Filme und Umwandlung für den Pi gehen dann nicht (siehe ANLEITUNG.md).");
        }
        let probes = fs::read(cache_dir.join("probe-cache.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<HashMap<String, serde_json::Value>>(&b).ok())
            .map(|m| {
                m.into_iter()
                    .filter_map(|(k, v)| Some((k, (v.get("key")?.as_str()?.to_string(), serde_json::from_value(v.get("info")?.clone()).ok()?))))
                    .collect()
            })
            .unwrap_or_default();
        VideoTools { ffmpeg, ffprobe: find_tool("ffprobe", "FFPROBE_PATH"), cache_dir, probes: Mutex::new(probes), sub_lock: Mutex::new(()) }
    }

    fn save_probes(&self, p: &HashMap<String, (String, ProbeInfo)>) {
        let m: serde_json::Map<String, serde_json::Value> =
            p.iter().map(|(k, (key, info))| (k.clone(), serde_json::json!({ "key": key, "info": info }))).collect();
        fs::create_dir_all(&self.cache_dir).ok();
        fs::write(self.cache_dir.join("probe-cache.json"), serde_json::to_vec(&m).unwrap_or_default()).ok();
    }

    pub fn probe(&self, file: &Path) -> Option<ProbeInfo> {
        let meta = fs::metadata(file).ok()?;
        let key = format!("{}|{}", meta.len(), mtime_ms(&meta).round());
        let k = file.to_string_lossy().into_owned();
        if let Some((ck, info)) = self.probes.lock().unwrap().get(&k) {
            if *ck == key {
                return Some(info.clone());
            }
        }
        let ffprobe = self.ffprobe.as_ref()?;
        let mut cmd = Command::new(ffprobe);
        cmd.args(["-v", "quiet", "-print_format", "json", "-show_format", "-show_streams"]).arg(file);
        hide_window(&mut cmd);
        let out = cmd.output().ok()?;
        let j: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
        let info = parse_probe(&j);
        let mut p = self.probes.lock().unwrap();
        p.insert(k, (key, info.clone()));
        self.save_probes(&p);
        Some(info)
    }

    /// A copied picture can only start on a keyframe: the stream for time t
    /// starts at the keyframe just before t.
    pub fn keyframe_before(&self, file: &Path, t: f64) -> f64 {
        let Some(ffprobe) = &self.ffprobe else { return 0.0 };
        if t <= 0.0 {
            return 0.0;
        }
        let mut cmd = Command::new(ffprobe);
        cmd.args(["-v", "error", "-select_streams", "v:0", "-read_intervals"])
            .arg(format!("{}%{}", (t - 30.0).max(0.0), t + 0.2))
            .args(["-show_entries", "packet=pts_time,flags", "-of", "csv=p=0"])
            .arg(file);
        hide_window(&mut cmd);
        let Ok(out) = cmd.output() else { return t };
        let mut best = -1.0f64;
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let mut it = line.split(',');
            let (Some(pts), Some(flags)) = (it.next(), it.next()) else { continue };
            if let Ok(x) = pts.parse::<f64>() {
                if flags.contains('K') && x <= t + 0.05 && x > best {
                    best = x;
                }
            }
        }
        if best >= 0.0 { (best * 1000.0).round() / 1000.0 } else { t }
    }

    /// Spawns ffmpeg writing fragmented MP4 to stdout.
    /// `v == "h264"`: convert the picture unless it already is H.264.
    pub fn remux(&self, file: &Path, t: f64, a: usize, v: &str) -> Option<ChildReader> {
        let ffmpeg = self.ffmpeg.as_ref()?;
        let info = self.probe(file).unwrap_or_default();
        let vcodec = info.video.as_ref().map(|v| v.codec.as_str()).unwrap_or("");
        let audio = info.audio.get(a);
        let copy_video = if v == "h264" { vcodec == "h264" } else { vcodec.is_empty() || COPY_VIDEO.contains(&vcodec) };
        let copy_audio = audio.map(|t| COPY_AUDIO.contains(&t.codec.as_str())).unwrap_or(false);
        let mut args: Vec<String> = ["-hide_banner", "-loglevel", "error", "-nostdin"].iter().map(|s| s.to_string()).collect();
        if t > 0.0 {
            args.extend(["-ss".into(), format!("{:.3}", t + 0.1)]);
        }
        args.extend(["-i".into(), file.to_string_lossy().into_owned(), "-map".into(), "0:v:0".into(), "-map".into(), format!("0:a:{a}?")]);
        if copy_video {
            args.extend(["-c:v", "copy"].map(String::from));
            if vcodec == "hevc" {
                args.extend(["-tag:v", "hvc1"].map(String::from));
            }
        } else {
            // The Pi 3's decoder: H.264 High@4.1 at most, 1080p.
            args.extend(
                ["-c:v", "libx264", "-preset", "veryfast", "-crf", "20", "-pix_fmt", "yuv420p", "-profile:v", "high", "-level", "4.1", "-vf", "scale='min(1920,iw)':-2"]
                    .map(String::from),
            );
        }
        if copy_audio {
            args.extend(["-c:a", "copy"].map(String::from));
        } else {
            args.extend(["-c:a", "aac", "-b:a", "192k", "-ac", "2"].map(String::from));
        }
        args.extend(
            ["-sn", "-dn", "-map_chapters", "-1", "-avoid_negative_ts", "make_zero", "-movflags", "frag_keyframe+empty_moov+default_base_moof", "-f", "mp4", "pipe:1"]
                .map(String::from),
        );
        let mut cmd = Command::new(ffmpeg);
        cmd.args(&args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
        hide_window(&mut cmd);
        cmd.spawn().ok().map(ChildReader)
    }

    /// Extracted once per file and track, then served shifted to the stream start.
    pub fn subtitle(&self, file: &Path, n: u32, t: f64) -> Option<String> {
        let ffmpeg = self.ffmpeg.as_ref()?;
        let meta = fs::metadata(file).ok()?;
        let hash = sha1_hex(&format!("{}|{}|{}|{}", file.to_string_lossy(), meta.len(), mtime_ms(&meta), n), 16);
        let out = self.cache_dir.join("subs").join(format!("{hash}.vtt"));
        {
            let _g = self.sub_lock.lock().unwrap();
            if !out.exists() {
                fs::create_dir_all(out.parent()?).ok();
                let tmp = out.with_extension("vtt.part");
                let mut cmd = Command::new(ffmpeg);
                cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i"]).arg(file).args(["-map", &format!("0:s:{n}"), "-f", "webvtt"]).arg(&tmp);
                hide_window(&mut cmd);
                let ok = cmd.status().map(|s| s.success()).unwrap_or(false);
                if !ok || fs::rename(&tmp, &out).is_err() {
                    return None;
                }
            }
        }
        Some(tvcore::tracks::shift_vtt(&fs::read_to_string(out).ok()?, t))
    }
}

fn parse_probe(j: &serde_json::Value) -> ProbeInfo {
    let empty = vec![];
    let streams = j["streams"].as_array().unwrap_or(&empty);
    let tag = |s: &serde_json::Value, k: &str| -> String {
        s["tags"][k].as_str().or_else(|| s["tags"][k.to_uppercase()].as_str()).unwrap_or("").to_string()
    };
    let disp = |s: &serde_json::Value, k: &str| s["disposition"][k].as_i64().unwrap_or(0) != 0;
    let v = streams.iter().find(|s| s["codec_type"] == "video" && !disp(s, "attached_pic"));
    let mut sub_n = 0u32;
    let mut subs = Vec::new();
    for s in streams.iter().filter(|s| s["codec_type"] == "subtitle") {
        let codec = s["codec_name"].as_str().unwrap_or("");
        if TEXT_SUBS.contains(&codec) {
            let title = tag(s, "title");
            let tl = title.to_lowercase();
            subs.push(SubTrack {
                n: sub_n,
                lang: tag(s, "language"),
                forced: disp(s, "forced") || tl.contains("forced") || tl.contains("erzwungen"),
                default: disp(s, "default"),
                title,
            });
        }
        sub_n += 1;
    }
    let dur = j["format"]["duration"].as_str().and_then(|d| d.parse::<f64>().ok()).map(|d| d.round() as u64).filter(|d| *d > 0);
    ProbeInfo {
        duration: dur,
        video: v.map(|v| VideoInfo {
            codec: v["codec_name"].as_str().unwrap_or("").into(),
            width: v["width"].as_u64().unwrap_or(0) as u32,
            height: v["height"].as_u64().unwrap_or(0) as u32,
        }),
        audio: streams
            .iter()
            .filter(|s| s["codec_type"] == "audio")
            .map(|s| AudioTrack {
                lang: tag(s, "language"),
                title: tag(s, "title"),
                codec: s["codec_name"].as_str().unwrap_or("").into(),
                channels: s["channels"].as_u64().unwrap_or(0) as u32,
                default: disp(s, "default"),
            })
            .collect(),
        subs,
    }
}
