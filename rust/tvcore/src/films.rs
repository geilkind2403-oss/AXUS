//! The film library. Same schema on the PC server and on the Pi's USB drive:
//!   <root>/.../<Film>/<video>.mp4|mkv|webm|mov|m4v|...
//!                    /Name/*.txt  /Info/*.txt  /FSK/*.txt
//!                    /Personas/*.txt  /Madeby/*.txt
//!                    /Icon/*.jpg|png|webp  (/Icon_2)  /Banner/*.jpg|png|webp
use crate::util::{ext_lower, id_for, image_mime, mtime_ms, slash};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

pub fn video_mime(p: &Path) -> Option<&'static str> {
    Some(match ext_lower(p).as_str() {
        ".mp4" | ".m4v" => "video/mp4",
        ".mov" => "video/quicktime",
        ".webm" => "video/webm",
        ".mkv" => "video/x-matroska",
        ".avi" => "video/x-msvideo",
        ".ts" | ".m2ts" | ".mts" => "video/mp2t",
        ".wmv" => "video/x-ms-wmv",
        ".mpg" | ".mpeg" => "video/mpeg",
        _ => return None,
    })
}

const META_DIRS: [&str; 8] = ["banner", "fsk", "icon", "icon_2", "info", "madeby", "name", "personas"];

#[derive(Debug, Clone, Serialize)]
pub struct Film {
    pub id: String,
    pub title: String,
    #[serde(rename = "relativePath")]
    pub relative_path: String,
    #[serde(skip)]
    pub file: PathBuf,
    #[serde(skip)]
    pub mtime_ms: f64,
    pub size: u64,
    #[serde(skip)]
    pub mime: &'static str,
    pub info: Option<String>,
    pub fsk: Option<String>,
    pub cast: Option<String>,
    pub crew: Option<String>,
    pub genre: Option<String>,
    #[serde(skip)]
    pub icon_file: Option<PathBuf>,
    #[serde(skip)]
    pub banner_file: Option<PathBuf>,
    pub duration: Option<u64>,
}

fn sorted_entries(dir: &Path) -> Vec<fs::DirEntry> {
    let mut v: Vec<_> = fs::read_dir(dir).map(|r| r.flatten().collect()).unwrap_or_default();
    v.sort_by_key(|e| e.file_name());
    v
}

fn meta_dir(folder: &Path, sub: &str) -> Option<PathBuf> {
    // The folders are "Name", "Info", ... but a FAT/exFAT USB drive copied by
    // hand may have any case: match case-insensitively.
    sorted_entries(folder)
        .into_iter()
        .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(sub) && e.path().is_dir())
        .map(|e| e.path())
}

pub fn read_meta_text(folder: &Path, sub: &str) -> Option<String> {
    let dir = meta_dir(folder, sub)?;
    let f = sorted_entries(&dir).into_iter().find(|e| ext_lower(&e.path()) == ".txt")?;
    let t = crate::util::decode_text_file(&fs::read(f.path()).ok()?).trim().to_string();
    Some(t)
}

pub fn find_meta_image(folder: &Path, sub: &str) -> Option<PathBuf> {
    let dir = meta_dir(folder, sub)?;
    sorted_entries(&dir).into_iter().map(|e| e.path()).find(|p| image_mime(p).is_some())
}

/// "Star.Wars.1977.1080p.x264.mkv" -> "Star Wars 1977"
pub fn title_from_file(file: &Path) -> String {
    let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let spaced = stem.replace(['.', '_'], " ");
    let junk = ["1080p", "720p", "2160p", "4k", "x264", "x265", "h264", "h265", "hevc", "bluray", "web-dl", "webrip", "german", "dl", "ac3", "dts", "aac"];
    let words: Vec<&str> = spaced.split_whitespace().collect();
    let cut = words.iter().position(|w| junk.contains(&w.to_lowercase().as_str())).unwrap_or(words.len());
    let t = words[..cut.max(1).min(words.len())].join(" ");
    if t.is_empty() { stem } else { t }
}

/// Every folder that holds a video is one film (its first video file).
pub fn scan(root: &Path) -> Vec<Film> {
    let mut out = Vec::new();
    walk(root, root, &mut out, 0);
    out
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<Film>, depth: u32) {
    if depth > 8 {
        return;
    }
    let entries = sorted_entries(dir);
    let videos: Vec<&fs::DirEntry> = entries.iter().filter(|e| e.path().is_file() && video_mime(&e.path()).is_some()).collect();
    // Loose files (several videos in one folder without the Name/Info schema,
    // or videos lying in the library root): each one is a film of its own.
    let schema = meta_dir(dir, "Name").is_some() || meta_dir(dir, "Icon").is_some();
    if (videos.len() > 1 && !schema) || (dir == root && !videos.is_empty()) {
        for v in videos {
            let file = v.path();
            let rel = slash(file.strip_prefix(root).unwrap_or(&file));
            let meta = fs::metadata(&file).ok();
            out.push(Film {
                id: id_for(&rel),
                title: title_from_file(&file),
                relative_path: rel,
                mime: video_mime(&file).unwrap_or("application/octet-stream"),
                size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
                mtime_ms: meta.as_ref().map(mtime_ms).unwrap_or(0.0),
                info: None,
                fsk: None,
                cast: None,
                crew: None,
                genre: None,
                icon_file: None,
                banner_file: None,
                duration: None,
                file,
            });
        }
    } else if let Some(video) = videos.first() {
        let file = video.path();
        let rel = slash(file.strip_prefix(root).unwrap_or(&file));
        let raw_name = read_meta_text(dir, "Name");
        let info = read_meta_text(dir, "Info");
        let genre = info.as_deref().and_then(|i| {
            i.lines().find_map(|l| {
                let l = l.trim();
                let lower = l.to_lowercase();
                lower.strip_prefix("genre:").map(|_| l[6..].trim().split(',').next().unwrap_or("").trim().to_string())
            })
        });
        let meta = fs::metadata(&file).ok();
        let title = raw_name
            .map(|n| n.trim_start_matches('-').trim().to_string())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        out.push(Film {
            id: id_for(&rel),
            title,
            relative_path: rel,
            mime: video_mime(&file).unwrap_or("application/octet-stream"),
            size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
            mtime_ms: meta.as_ref().map(mtime_ms).unwrap_or(0.0),
            info,
            fsk: read_meta_text(dir, "FSK"),
            cast: read_meta_text(dir, "Personas"),
            crew: read_meta_text(dir, "Madeby"),
            genre: genre.filter(|g| !g.is_empty()),
            icon_file: find_meta_image(dir, "Icon").or_else(|| find_meta_image(dir, "Icon_2")),
            banner_file: find_meta_image(dir, "Banner"),
            duration: None,
            file,
        });
    }
    for e in entries {
        let name = e.file_name().to_string_lossy().to_lowercase();
        if e.path().is_dir() && !META_DIRS.contains(&name.as_str()) && !name.starts_with('.') && !name.starts_with('$') {
            walk(root, &e.path(), out, depth + 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scans_schema() {
        let root = std::env::temp_dir().join(format!("tvcore-films-{}", std::process::id()));
        let film = root.join("Sci-Fi").join("Star Wars");
        fs::create_dir_all(film.join("Name")).unwrap();
        fs::create_dir_all(film.join("Info")).unwrap();
        fs::create_dir_all(film.join("Icon")).unwrap();
        fs::write(film.join("Name/n.txt"), "-Star Wars: Episode IV\n").unwrap();
        fs::write(film.join("Info/i.txt"), "Ein Film.\nGenre: Science-Fiction, Abenteuer\n").unwrap();
        fs::write(film.join("Icon/c.jpg"), b"x").unwrap();
        fs::write(film.join("film.mkv"), b"data").unwrap();
        let lib = scan(&root);
        assert_eq!(lib.len(), 1);
        let f = &lib[0];
        assert_eq!(f.title, "Star Wars: Episode IV");
        assert_eq!(f.relative_path, "Sci-Fi/Star Wars/film.mkv");
        assert_eq!(f.genre.as_deref(), Some("Science-Fiction"));
        assert_eq!(f.id, id_for("Sci-Fi/Star Wars/film.mkv"));
        assert!(f.icon_file.is_some());
        fs::write(root.join("Star.Wars.1977.1080p.x264.mkv"), b"x").unwrap();
        let lib = scan(&root);
        assert_eq!(lib.len(), 2);
        assert!(lib.iter().any(|f| f.title == "Star Wars 1977"));
        fs::remove_dir_all(root).ok();
    }
}
