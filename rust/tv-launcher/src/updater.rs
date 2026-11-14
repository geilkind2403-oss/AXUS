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
const RELEASE_API: &str = "https://api.github.com/repos/geilkind2403-oss/tv-launcher/releases/latest";
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

    let token = std::env::var("TV_UPDATE_TOKEN")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let release_response = github_get(RELEASE_API, token.as_deref(), "application/vnd.github+json")
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

    let asset_url = format!(
        "https://api.github.com/repos/geilkind2403-oss/tv-launcher/releases/assets/{asset_id}"
    );
    let archive = github_get(&asset_url, token.as_deref(), "application/octet-stream")
        .map_err(|e| format!("Update {tag} konnte nicht geladen werden: {e}"))?;
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
