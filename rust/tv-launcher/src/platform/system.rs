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

/// Connects through a connman provisioning file (no interactive agent
/// needed); connman sees the file and connects within a few seconds.
pub fn wifi_connect(w: &Wifi, pass: &str) -> Result<(), String> {
    if w.secured {
        let dir = "/storage/.cache/connman";
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let esc = |s: &str| s.replace('\n', "");
        let conf = format!(
            "[service_tvlauncher]\nType = wifi\nName = {}\nPassphrase = {}\n",
            esc(&w.name),
            esc(pass)
        );
        std::fs::write(format!("{dir}/tvlauncher-wifi.config"), conf).map_err(|e| e.to_string())?;
    }
    let out = Command::new("connmanctl")
        .args(["connect", &w.service])
        .output()
        .map_err(|e| e.to_string())?;
    let text =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    if text.to_lowercase().contains("error") && !text.contains("Already connected") {
        return Err(text.trim().to_string());
    }
    Ok(())
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
