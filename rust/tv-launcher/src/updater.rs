//! GitHub Releases updater for the installed LibreELEC build.
//!
//! A release package contains only the launcher binary plus its build stamp
//! and SHA-256 file. The running app stages and verifies it; a separate
//! systemd oneshot stops the service, swaps the binary, then rolls back if
//! the new service does not stay up.
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const INSTALLED_BINARY: &str = "/storage/tv-launcher/tv-launcher";
/// The repository was renamed from tv-launcher to AXUS; GitHub redirects the
/// old name, but the new one is asked first.
const REPOS: [&str; 2] = ["geilkind2403-oss/AXUS", "geilkind2403-oss/tv-launcher"];
const ASSET_NAME: &str = "tv-launcher-update.tar.gz";
const MAX_PACKAGE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct UpdateInfo {
    pub version: String,
    pub current: bool,
}

/// Fetch the latest GitHub release package, verify its contents and stage the
/// new executable next to the installed one.
pub fn stage_github_update() -> Result<UpdateInfo, String> {
    let running = std::env::current_exe().map_err(|e| format!("Launcher-Pfad: {e}"))?;
    if running != Path::new(INSTALLED_BINARY) {
        return Err("Updates lassen sich nur auf der installierten Pi-Version anwenden".into());
    }

    let token = token();
    let (repo, release_response) = repo_get("releases/latest", token.as_deref())
        .map_err(|e| {
            if e.contains("404") && token.is_none() {
                "GitHub liefert keine Version. Bei einem privaten Repo TV_UPDATE_TOKEN (Contents: Read-only) in /storage/tv-launcher/env eintragen.".into()
            } else if e.contains("401") || e.contains("403") {
                "GitHub-Zugriff abgelehnt. TV_UPDATE_TOKEN prüfen; benötigt wird Contents: Read-only.".into()
            } else if e.contains("404") {
                "Kein GitHub-Release gefunden. Erst eine Version mit tv-launcher-update.tar.gz veröffentlichen.".into()
            } else {
                format!("GitHub-Update konnte nicht geprüft werden: {e}")
            }
        })?;
    let release: serde_json::Value = serde_json::from_slice(&release_response)
        .map_err(|e| format!("GitHub-Antwort ist ungültig: {e}"))?;
    let tag = release["tag_name"].as_str().unwrap_or("unbekannt");
    let asset_id = release["assets"]
        .as_array()
        .and_then(|assets| {
            assets
                .iter()
                .find(|asset| asset["name"].as_str() == Some(ASSET_NAME))
        })
        .and_then(|asset| asset["id"].as_u64())
        .ok_or_else(|| format!("Release {tag} enthält {ASSET_NAME} nicht"))?;

    stage_asset(repo, asset_id, tag, token.as_deref())
}

fn token() -> Option<String> {
    std::env::var("TV_UPDATE_TOKEN")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// GET api.github.com/repos/<repo>/<path>, the new repository name first.
fn repo_get(path: &str, token: Option<&str>) -> Result<(&'static str, Vec<u8>), String> {
    let mut last = String::new();
    for repo in REPOS {
        let url = format!("https://api.github.com/repos/{repo}/{path}");
        match github_get(&url, token, "application/vnd.github+json") {
            Ok(b) => return Ok((repo, b)),
            Err(e) if e.contains("404") => last = e,
            Err(e) => return Err(e),
        }
    }
    Err(last)
}

/// Downloads one release's package, verifies it and stages it.
fn stage_asset(repo: &str, asset_id: u64, tag: &str, token: Option<&str>) -> Result<UpdateInfo, String> {
    let asset_url = format!("https://api.github.com/repos/{repo}/releases/assets/{asset_id}");
    let archive = github_get(&asset_url, token, "application/octet-stream")
        .map_err(|e| format!("Version {tag} konnte nicht geladen werden: {e}"))?;
    if archive.is_empty() || archive.len() as u64 > MAX_PACKAGE_BYTES {
        return Err("GitHub-Update hat eine unerwartete Dateigröße".into());
    }
    let install_dir = Path::new("/storage/tv-launcher");
    fs::create_dir_all(install_dir).map_err(|e| format!("Update-Ordner: {e}"))?;
    let package = install_dir.join("tv-launcher-update.download.tar.gz");
    fs::write(&package, archive).map_err(|e| format!("Update kann nicht gespeichert werden: {e}"))?;
    let result = validate_and_stage(&package);
    let _ = fs::remove_file(&package);
    result
}

/// One published version, as kept in the release cache.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Release {
    pub tag: String,
    /// the release title, e.g. "v0.0.3.7-Jenna-Ortega"
    pub name: String,
    pub date: String,
    pub repo: String,
    pub asset_id: u64,
}

impl Release {
    /// "Jenna Ortega": from the release title, else from the version number
    pub fn codename(&self) -> String {
        let from_title = self
            .name
            .strip_prefix(&self.tag)
            .map(|r| r.trim_start_matches(['-', ' ', '·']).replace('-', " "))
            .filter(|r| !r.trim().is_empty());
        from_title.unwrap_or_else(|| codename(&self.tag).to_string())
    }
    /// "0.0.3.7" for "v0.0.3.7"
    pub fn version(&self) -> &str {
        self.tag.trim_start_matches('v')
    }
    /// whether this is the version running now ("0.0.3+7" is v0.0.3.7)
    pub fn installed(&self) -> bool {
        four_part(self.version()) == running_version()
    }
}

/// The celebrity name of each 0.0.X line (like Android's sweets).
pub fn codename(version: &str) -> &'static str {
    let v = version.trim_start_matches('v');
    let line: Vec<&str> = v.splitn(4, ['.', '+']).collect();
    match line.get(2).copied().unwrap_or("") {
        "1" => "Adriana Lima",
        "2" => "Madison Beer",
        "3" => "Jenna Ortega",
        _ => "",
    }
}

/// A version always in four parts: "0.0.3+7" and "v0.0.3.7" are "0.0.3.7",
/// "v0.0.1" is "0.0.1.0".
pub fn four_part(v: &str) -> String {
    let mut parts: Vec<&str> = v
        .trim_start_matches('v')
        .split(['.', '+'])
        .filter(|p| !p.is_empty())
        .collect();
    while parts.len() < 4 {
        parts.push("0");
    }
    parts.join(".")
}

/// The running version, four parts ("0.0.3.8").
pub fn running_version() -> String {
    four_part(env!("CARGO_PKG_VERSION"))
}

/// The name of the running version, e.g. "Jenna Ortega".
pub fn running_codename() -> &'static str {
    codename(env!("CARGO_PKG_VERSION"))
}

fn release_cache() -> PathBuf {
    crate::app::data_dir().join("releases.json")
}

/// The versions saved by the last refresh (newest first); instant, offline.
pub fn cached_releases() -> Vec<Release> {
    fs::read(release_cache())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Asks GitHub for every release with an update package and saves the list.
pub fn refresh_releases() -> Result<Vec<Release>, String> {
    let token = token();
    let (repo, body) = repo_get("releases?per_page=100", token.as_deref())
        .map_err(|e| format!("Versionsliste nicht geladen: {e}"))?;
    let list: serde_json::Value =
        serde_json::from_slice(&body).map_err(|e| format!("GitHub-Antwort ist ungültig: {e}"))?;
    let mut out = Vec::new();
    for r in list.as_array().into_iter().flatten() {
        if r["draft"].as_bool().unwrap_or(false) {
            continue;
        }
        let tag = r["tag_name"].as_str().unwrap_or("");
        // only the 0.0.x line: every one of them carries the self-updater
        // (it came with the very first), so from any of them "Software-Update
        // suchen" leads back to the newest version
        if !tag.starts_with("v0.") {
            continue;
        }
        let Some(asset_id) = r["assets"].as_array().and_then(|a| {
            a.iter()
                .find(|x| x["name"].as_str() == Some(ASSET_NAME))
                .and_then(|x| x["id"].as_u64())
        }) else {
            continue;
        };
        out.push(Release {
            tag: tag.to_string(),
            name: r["name"].as_str().unwrap_or(tag).to_string(),
            date: r["published_at"].as_str().unwrap_or("").chars().take(10).collect(),
            repo: repo.to_string(),
            asset_id,
        });
    }
    out.sort_by(|a, b| b.date.cmp(&a.date).then(b.tag.cmp(&a.tag)));
    if out.is_empty() {
        return Err("GitHub nennt keine Version mit Update-Paket".into());
    }
    let dir = crate::app::data_dir();
    let _ = fs::create_dir_all(&dir);
    if let Ok(b) = serde_json::to_vec(&out) {
        let tmp = dir.join("releases.json.tmp");
        if fs::write(&tmp, b).is_ok() {
            let _ = fs::rename(tmp, release_cache());
        }
    }
    Ok(out)
}

/// Stages a chosen (older or newer) version; installed like an update.
pub fn stage_release(r: &Release) -> Result<UpdateInfo, String> {
    let running = std::env::current_exe().map_err(|e| format!("Launcher-Pfad: {e}"))?;
    if running != Path::new(INSTALLED_BINARY) {
        return Err("Versionen lassen sich nur auf der installierten Pi-Version wechseln".into());
    }
    let token = token();
    stage_asset(&r.repo, r.asset_id, &r.tag, token.as_deref())
}

fn github_get(url: &str, token: Option<&str>, accept: &str) -> Result<Vec<u8>, String> {
    let mut request = crate::net::agent()
        .get(url)
        .set("Accept", accept)
        .set("X-GitHub-Api-Version", "2022-11-28");
    if let Some(token) = token {
        request = request.set("Authorization", &format!("Bearer {token}"));
    }
    let response = request.call().map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_PACKAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Download lesen: {e}"))?;
    if bytes.len() as u64 > MAX_PACKAGE_BYTES {
        return Err("GitHub-Antwort ist größer als 64 MB".into());
    }
    Ok(bytes)
}

fn validate_and_stage(package: &Path) -> Result<UpdateInfo, String> {

    // Listing first rejects non-tar files before asking tar for individual
    // members. We never extract archive paths directly onto the filesystem.
    let listing = Command::new("tar")
        .args(["-tzf"])
        .arg(&package)
        .output()
        .map_err(|e| format!("Archiv kann nicht geprüft werden: {e}"))?;
    if !listing.status.success() {
        return Err("Das Update-Archiv ist beschädigt oder kein gültiges tar.gz".into());
    }
    let members = String::from_utf8_lossy(&listing.stdout);
    for required in ["tv-launcher", "TV_BUILD", "tv-launcher.sha256"] {
        if !members.lines().any(|line| line.trim() == required) {
            return Err(format!("Update-Archiv enthält {required} nicht"));
        }
    }

    let version = read_member(&package, "TV_BUILD")?;
    if version.is_empty() {
        return Err("Das Update enthält keine Versionsnummer".into());
    }
    if version == crate::BUILD {
        return Ok(UpdateInfo {
            version,
            current: true,
        });
    }

    let expected = read_member(&package, "tv-launcher.sha256")?
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Die SHA-256-Prüfsumme im Update ist ungültig".into());
    }

    let staged = PathBuf::from(format!("{INSTALLED_BINARY}.new"));
    let output =
        File::create(&staged).map_err(|e| format!("Update kann nicht gespeichert werden: {e}"))?;
    let child = Command::new("tar")
        .args(["-xOzf"])
        .arg(&package)
        .arg("tv-launcher")
        .stdout(Stdio::from(output))
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Launcher-Datei kann nicht entpackt werden: {e}"))?;
    if !child
        .wait_with_output()
        .map_err(|e| format!("Update-Entpacken fehlgeschlagen: {e}"))?
        .status
        .success()
    {
        let _ = fs::remove_file(&staged);
        return Err("Launcher-Datei konnte nicht aus dem Update gelesen werden".into());
    }
    let metadata = fs::metadata(&staged).map_err(|e| format!("Update-Datei fehlt: {e}"))?;
    if metadata.len() < 1_000_000 || metadata.len() > 64 * 1024 * 1024 {
        let _ = fs::remove_file(&staged);
        return Err("Die Größe der Update-Datei ist unerwartet".into());
    }
    let mut magic = [0u8; 4];
    File::open(&staged)
        .and_then(|mut f| f.read_exact(&mut magic))
        .map_err(|e| format!("Update-Datei ist unvollständig: {e}"))?;
    if magic != [0x7f, b'E', b'L', b'F'] {
        let _ = fs::remove_file(&staged);
        return Err("Die Update-Datei ist keine ausführbare Pi-Version".into());
    }
    fs::set_permissions(&staged, fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("Ausführungsrechte setzen: {e}"))?;
    File::open(&staged)
        .and_then(|f| f.sync_all())
        .map_err(|e| format!("Update-Datei konnte nicht gesichert werden: {e}"))?;

    let actual = Command::new("sha256sum")
        .arg(&staged)
        .output()
        .map_err(|e| format!("SHA-256-Prüfung nicht verfügbar: {e}"))?;
    if !actual.status.success()
        || String::from_utf8_lossy(&actual.stdout)
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
            != expected
    {
        let _ = fs::remove_file(&staged);
        return Err("Prüfsumme stimmt nicht – das Update wurde verworfen".into());
    }

    let version_file = PathBuf::from(format!("{INSTALLED_BINARY}.new.version"));
    fs::write(&version_file, format!("{version}\n"))
        .map_err(|e| format!("Versionsdatei kann nicht gespeichert werden: {e}"))?;
    install_update_support()?;

    Ok(UpdateInfo {
        version,
        current: false,
    })
}

fn read_member(package: &Path, member: &str) -> Result<String, String> {
    let output = Command::new("tar")
        .args(["-xOzf"])
        .arg(package)
        .arg(member)
        .output()
        .map_err(|e| format!("{member} kann nicht gelesen werden: {e}"))?;
    if !output.status.success() {
        return Err(format!("Update-Archiv enthält {member} nicht"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn install_update_support() -> Result<(), String> {
    let launcher_dir = Path::new("/storage/tv-launcher");
    let units_dir = Path::new("/storage/.config/system.d");
    fs::create_dir_all(launcher_dir).map_err(|e| format!("Update-Ordner: {e}"))?;
    fs::create_dir_all(units_dir).map_err(|e| format!("Systemdienst-Ordner: {e}"))?;

    let script = launcher_dir.join("apply-update.sh");
    fs::write(&script, include_str!("../../deploy/apply-update.sh"))
        .map_err(|e| format!("Rollback-Skript installieren: {e}"))?;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("Rollback-Skript aktivieren: {e}"))?;
    fs::write(
        units_dir.join("tv-launcher-update.service"),
        include_str!("../../deploy/tv-launcher-update.service"),
    )
    .map_err(|e| format!("Update-Dienst installieren: {e}"))?;

    let reload = Command::new("systemctl")
        .arg("daemon-reload")
        .status()
        .map_err(|e| format!("systemd kann nicht neu geladen werden: {e}"))?;
    if !reload.success() {
        return Err("systemd hat den Update-Dienst nicht geladen".into());
    }
    Ok(())
}

/// Ask the one-shot service to replace the running binary and perform health
/// checking/rollback outside the launcher service's process group.
pub fn apply_staged_update() -> Result<(), String> {
    let status = Command::new("systemctl")
        .args(["start", "--no-block", "tv-launcher-update.service"])
        .status()
        .map_err(|e| format!("Update-Dienst kann nicht gestartet werden: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("Update-Dienst konnte nicht gestartet werden".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(tag: &str, name: &str) -> Release {
        Release {
            tag: tag.into(),
            name: name.into(),
            date: String::new(),
            repo: REPOS[0].into(),
            asset_id: 1,
        }
    }

    #[test]
    fn codenames_by_line() {
        assert_eq!(codename("0.0.1"), "Adriana Lima");
        assert_eq!(codename("v0.0.1.1"), "Adriana Lima");
        assert_eq!(codename("0.0.2"), "Madison Beer");
        assert_eq!(codename("0.0.3+7"), "Jenna Ortega");
        assert_eq!(codename("v0.0.3.7"), "Jenna Ortega");
        assert_eq!(codename("0.0.9"), "");
    }

    #[test]
    fn versions_have_four_parts() {
        assert_eq!(four_part("0.0.3+8"), "0.0.3.8");
        assert_eq!(four_part("v0.0.3.7"), "0.0.3.7");
        assert_eq!(four_part("v0.0.1"), "0.0.1.0");
        assert_eq!(four_part("0.0.2"), "0.0.2.0");
        assert_eq!(four_part("v0.0.2.0"), "0.0.2.0");
    }

    #[test]
    fn codename_comes_from_the_release_title() {
        assert_eq!(rel("v0.0.3.7", "v0.0.3.7-Jenna-Ortega").codename(), "Jenna Ortega");
        assert_eq!(rel("v0.0.4", "v0.0.4-Some-New-Name").codename(), "Some New Name");
        // no name in the title: from the version line
        assert_eq!(rel("v0.0.2", "v0.0.2").codename(), "Madison Beer");
    }
}
