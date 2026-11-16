//! LibreELEC system services, through the tools it ships: connman (network),
//! BlueZ (bluetooth), systemd (restart / power off). Everything the launcher
//! keeps lives under /storage (the only writable place on LibreELEC).
use std::process::Command;

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[derive(Clone, Debug, Default)]
pub struct Wifi {
    pub name: String,
    pub service: String,
    pub connected: bool,
    pub secured: bool,
    pub strength: u8,
}

/// Network state for the status bar: (connected?, kind "wifi"/"ethernet", ssid/ip text).
pub fn network_status() -> (bool, String, String) {
    let Some(out) = run("connmanctl", &["services"]) else {
        return (
            local_ip().is_some(),
            "ethernet".into(),
            local_ip().unwrap_or_default(),
        );
    };
    for line in out.lines() {
        let flags = line.get(..4).unwrap_or("");
        if flags.contains('O') || flags.contains('R') {
            let svc = line.split_whitespace().last().unwrap_or("");
            let name = line
                .get(4..)
                .unwrap_or("")
                .trim()
                .rsplit_once(' ')
                .map(|x| x.0.trim())
                .unwrap_or("")
                .to_string();
            let kind = if svc.starts_with("wifi_") {
                "wifi"
            } else {
                "ethernet"
            };
            return (
                true,
                kind.into(),
                if kind == "wifi" {
                    name
                } else {
                    local_ip().unwrap_or(name)
                },
            );
        }
    }
    (false, String::new(), String::new())
}

pub fn wifi_scan() -> Vec<Wifi> {
    let _ = run("connmanctl", &["enable", "wifi"]);
    let _ = run("connmanctl", &["scan", "wifi"]);
    let out = run("connmanctl", &["services"]).unwrap_or_default();
    let mut v = Vec::new();
    for line in out.lines() {
        let svc = line.split_whitespace().last().unwrap_or("");
        if !svc.starts_with("wifi_") {
            continue;
        }
        let flags = line.get(..4).unwrap_or("");
        let name = line
            .get(4..)
            .unwrap_or("")
            .trim()
            .rsplit_once(' ')
            .map(|x| x.0.trim().to_string())
            .unwrap_or_default();
        if name.is_empty() || name.starts_with('*') {
            continue; // hidden network
        }
        let strength = run("connmanctl", &["services", svc])
            .and_then(|d| {
                d.lines()
                    .find(|l| l.trim_start().starts_with("Strength"))
                    .and_then(|l| l.split('=').nth(1))
                    .and_then(|s| s.trim().parse().ok())
            })
            .unwrap_or(0);
        v.push(Wifi {
            name,
            service: svc.into(),
            connected: flags.contains('O') || flags.contains('R'),
            secured: !svc.ends_with("_none"),
            strength,
        });
    }
    v.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then(b.strength.cmp(&a.strength))
    });
    v
}

const CONNMAN_DIR: &str = "/storage/.cache/connman";
const PROVISION: &str = "/storage/.cache/connman/tvlauncher-wifi.config";

/// The wired connection: whether a cable is plugged in and its address.
#[derive(Clone, Debug, Default)]
pub struct Lan {
    pub cable: bool,
    pub connected: bool,
    pub ip: String,
}

/// One value out of `connmanctl services <svc>` ("State = ready").
fn service_value(svc: &str, key: &str) -> Option<String> {
    run("connmanctl", &["services", svc])?
        .lines()
        .find_map(|l| {
            let l = l.trim_start();
            l.strip_prefix(key)
                .and_then(|r| r.trim_start().strip_prefix('='))
                .map(|v| v.trim().to_string())
        })
}

/// The address connman got for a service (from its IPv4 block).
fn service_ip(svc: &str) -> String {
    service_value(svc, "IPv4")
        .and_then(|v| {
            v.split("Address=")
                .nth(1)
                .map(|r| r.split([',', ' ', ']']).next().unwrap_or("").to_string())
        })
        .unwrap_or_default()
}

fn service_online(svc: &str) -> bool {
    matches!(
        service_value(svc, "State").as_deref(),
        Some("ready") | Some("online")
    )
}

fn ethernet_service() -> Option<(String, bool)> {
    let out = run("connmanctl", &["services"])?;
    out.lines().find_map(|line| {
        let svc = line.split_whitespace().last()?;
        svc.starts_with("ethernet_").then(|| {
            let flags = line.get(..4).unwrap_or("");
            (svc.to_string(), flags.contains('O') || flags.contains('R'))
        })
    })
}

/// Cable state for Settings. connman lists the wired service only while a
/// cable is plugged in.
pub fn lan_status() -> Lan {
    match ethernet_service() {
        Some((svc, connected)) => Lan {
            cable: true,
            connected,
            ip: if connected {
                service_ip(&svc)
            } else {
                String::new()
            },
        },
        None => Lan::default(),
    }
}

/// Asks the wired connection to connect again (cable plugged in later,
/// router restarted).
pub fn lan_reconnect() -> Result<String, String> {
    let Some((svc, _)) = ethernet_service() else {
        return Err("Kein Kabel erkannt – Kabel und Router prüfen".into());
    };
    let _ = run("connmanctl", &["disconnect", &svc]);
    let _ = run("connmanctl", &["connect", &svc]);
    for _ in 0..12 {
        if service_online(&svc) {
            let ip = service_ip(&svc);
            return Ok(format!("LAN verbunden · {ip}"));
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    Err("Das Kabel hat keine Adresse bekommen – Router prüfen".into())
}

/// connman's English error in plain words.
fn connman_error(text: &str) -> String {
    let t = text.to_lowercase();
    if t.contains("invalid-key") || t.contains("input/output error") {
        "Verbindung abgelehnt – Passwort prüfen oder Router neu starten".into()
    } else if t.contains("not registered") || t.contains("not found") {
        "Netzwerk nicht gefunden – bitte neu suchen".into()
    } else if t.contains("in progress") {
        "Es läuft schon ein Verbindungsversuch – kurz warten".into()
    } else if t.contains("no carrier") {
        "Kein Signal vom Netzwerk".into()
    } else {
        let detail = text
            .trim()
            .rsplit(": ")
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        format!("Verbindung fehlgeschlagen: {detail}")
    }
}

fn clear_provisioning() {
    let _ = std::fs::remove_file(PROVISION);
}

/// Connects to a network. A new password goes into a connman provisioning
/// file (no interactive agent needed); an empty one never does, so a network
/// connman already knows keeps its saved password. Returns once the network
/// has an address, or says why not. A wrong password is wiped again, or
/// connman keeps trying it for good.
pub fn wifi_connect(w: &Wifi, pass: &str) -> Result<String, String> {
    if w.secured && !pass.is_empty() {
        std::fs::create_dir_all(CONNMAN_DIR).map_err(|e| e.to_string())?;
        let esc = |s: &str| s.replace('\n', "");
        let conf = format!(
            "[service_tvlauncher]\nType = wifi\nName = {}\nPassphrase = {}\n",
            esc(&w.name),
            esc(pass)
        );
        std::fs::write(PROVISION, conf).map_err(|e| e.to_string())?;
        // connman reads the file by itself; give it a moment
        std::thread::sleep(std::time::Duration::from_millis(1500));
    }
    let out = Command::new("connmanctl")
        .args(["connect", &w.service])
        .output()
        .map_err(|e| e.to_string())?;
    let text =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    let failed = text.to_lowercase().contains("error") && !text.contains("Already connected");
    if !failed {
        for _ in 0..15 {
            if service_online(&w.service) {
                clear_provisioning();
                return Ok(format!(
                    "Verbunden mit {} · {}",
                    w.name,
                    service_ip(&w.service)
                ));
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }
    // never leave a password that did not work behind
    clear_provisioning();
    if !pass.is_empty() {
        let _ = run("connmanctl", &["remove", &w.service]);
    }
    if failed {
        Err(connman_error(&text))
    } else {
        Err("Keine Verbindung – Passwort prüfen".into())
    }
}

/// Forgets a network: disconnects and drops its saved password.
pub fn wifi_forget(w: &Wifi) -> Result<String, String> {
    let _ = run("connmanctl", &["disconnect", &w.service]);
    let _ = run("connmanctl", &["remove", &w.service]);
    clear_provisioning();
    Ok(format!("„{}“ vergessen", w.name))
}

/// Wipes connman's saved networks and settings (wired and wireless) and
/// restarts it: the way out when connecting stopped working altogether.
pub fn network_reset() -> Result<String, String> {
    std::fs::remove_dir_all(CONNMAN_DIR).map_err(|e| format!("Zurücksetzen: {e}"))?;
    let _ = Command::new("systemctl")
        .args(["restart", "connman"])
        .status();
    std::thread::sleep(std::time::Duration::from_secs(3));
    let _ = run("connmanctl", &["enable", "wifi"]);
    Ok("Netzwerk zurückgesetzt – jetzt das WLAN neu auswählen".into())
}

#[derive(Clone, Debug)]
pub struct BtDevice {
    pub mac: String,
    pub name: String,
    pub paired: bool,
    pub connected: bool,
}

pub fn bt_devices(scan_secs: u32) -> Vec<BtDevice> {
    let _ = run("bluetoothctl", &["power", "on"]);
    if scan_secs > 0 {
        let _ = run(
            "bluetoothctl",
            &["--timeout", &scan_secs.to_string(), "scan", "on"],
        );
    }
    let out = run("bluetoothctl", &["devices"]).unwrap_or_default();
    let paired = run("bluetoothctl", &["devices", "Paired"]).unwrap_or_default();
    let mut v: Vec<BtDevice> = out
        .lines()
        .filter_map(|l| {
            let mut it = l.splitn(3, ' ');
            (it.next()? == "Device").then_some(())?;
            let mac = it.next()?.to_string();
            let name = it.next().unwrap_or("").to_string();
            let info = run("bluetoothctl", &["info", &mac]).unwrap_or_default();
            Some(BtDevice {
                connected: info.contains("Connected: yes"),
                paired: paired.contains(&mac),
                name,
                mac,
            })
        })
        .filter(|d| !d.name.is_empty() && d.name.replace('-', ":") != d.mac)
        .collect();
    v.sort_by(|a, b| b.connected.cmp(&a.connected).then(b.paired.cmp(&a.paired)));
    v
}

pub fn bt_connect(mac: &str) -> Result<(), String> {
    let _ = run("bluetoothctl", &["pair", mac]);
    let _ = run("bluetoothctl", &["trust", mac]);
    let out = run("bluetoothctl", &["connect", mac]).unwrap_or_default();
    if out.contains("successful") {
        Ok(())
    } else {
        Err("Verbinden ging nicht -- ist das Gerät im Kopplungsmodus?".into())
    }
}

pub fn local_ip() -> Option<String> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.168.0.1:9").ok()?;
    let ip = s.local_addr().ok()?.ip().to_string();
    (ip != "0.0.0.0").then_some(ip)
}

pub fn reboot() {
    let _ = Command::new("systemctl").arg("reboot").spawn();
}
pub fn poweroff() {
    let _ = Command::new("systemctl").arg("poweroff").spawn();
}

/// While this file exists Kodi stays off and the launcher runs (see
/// deploy/install.sh: a drop-in gives kodi.service the opposite condition).
pub const ENABLED_FLAG: &str = "/storage/.config/tv-launcher.enabled";

/// Starts Kodi again and stops the launcher (escape hatch, Settings > System).
/// To come back: `touch /storage/.config/tv-launcher.enabled` and reboot.
pub fn switch_to_kodi() {
    let _ = std::fs::remove_file(ENABLED_FLAG);
    let _ = Command::new("sh")
        .args([
            "-c",
            "systemctl start pulseaudio eventlircd kodi; systemctl stop tv-launcher",
        ])
        .spawn();
}

/// USB drives LibreELEC mounted (/var/media/<label>), plus /storage itself.
pub fn media_roots() -> Vec<std::path::PathBuf> {
    let mut v: Vec<std::path::PathBuf> = std::fs::read_dir("/var/media")
        .map(|r| r.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    v.sort();
    v
}

pub fn cpu_temp() -> Option<f32> {
    std::fs::read_to_string("/sys/class/thermal/thermal_zone0/temp")
        .ok()?
        .trim()
        .parse::<f32>()
        .ok()
        .map(|t| t / 1000.0)
}
