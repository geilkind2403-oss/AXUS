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

#[derive(Clone, Copy, Debug, Default)]
pub struct CpuCounters {
    total: u64,
    idle: u64,
}

#[derive(Clone, Debug, Default)]
pub struct SystemStats {
    pub memory_total_mb: u64,
    pub memory_available_mb: u64,
    pub process_rss_mb: u64,
    pub cpu_percent: Option<u8>,
    pub cpu_temp_c: Option<f32>,
    pub cpu_mhz: Option<u32>,
    pub storage_total_gb: Option<f32>,
    pub storage_available_gb: Option<f32>,
    pub uptime: String,
    pub load_1m: Option<f32>,
    pub kernel: String,
    pub hardware: String,
    pub cores: usize,
    pub removable_drives: usize,
}

fn cpu_counters() -> CpuCounters {
    std::fs::read_to_string("/proc/stat")
        .ok()
        .and_then(|contents| {
            let mut fields = contents.lines().next()?.split_whitespace();
            if fields.next()? != "cpu" {
                return None;
            }
            let mut total = 0u64;
            let mut idle = 0u64;
            for (i, value) in fields.enumerate() {
                let value = value.parse::<u64>().ok()?;
                total = total.saturating_add(value);
                if i == 3 || i == 4 {
                    idle = idle.saturating_add(value);
                }
            }
            Some(CpuCounters { total, idle })
        })
        .unwrap_or_default()
}

/// Read a small, bounded set of Linux telemetry for the Settings dashboard.
/// This runs on a worker thread and is refreshed only while the System page is open.
pub fn system_stats(previous_cpu: Option<CpuCounters>) -> (SystemStats, CpuCounters) {
    let mut stats = SystemStats::default();
    let first_cpu = cpu_counters();
    if previous_cpu.is_none() {
        // On the first visit, take a short second sample so CPU usage is
        // available immediately instead of showing an empty placeholder.
        std::thread::sleep(std::time::Duration::from_millis(180));
    }
    if let Ok(mem) = std::fs::read_to_string("/proc/meminfo") {
        for line in mem.lines() {
            let mut words = line.split_whitespace();
            let Some(key) = words.next() else { continue };
            let value = words
                .next()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);
            match key {
                "MemTotal:" => stats.memory_total_mb = value / 1024,
                "MemAvailable:" => stats.memory_available_mb = value / 1024,
                _ => {}
            }
        }
    }
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        stats.process_rss_mb = status
            .lines()
            .find(|line| line.starts_with("VmRSS:"))
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0)
            / 1024;
    }
    let counters = cpu_counters();
    stats.cpu_percent = previous_cpu.or(Some(first_cpu)).and_then(|old| {
        let total = counters.total.saturating_sub(old.total);
        let idle = counters.idle.saturating_sub(old.idle);
        (total > 0)
            .then(|| (100_u64.saturating_mul(total.saturating_sub(idle)) / total).min(100) as u8)
    });
    stats.cpu_temp_c = cpu_temp();
    stats.cpu_mhz =
        std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq")
            .ok()
            .and_then(|v| v.trim().parse::<u32>().ok())
            .map(|khz| khz / 1000);
    let storage_path = if std::path::Path::new("/storage").is_dir() {
        "/storage"
    } else {
        "/"
    };
    let path = std::ffi::CString::new(storage_path).ok();
    if let Some(path) = path {
        // SAFETY: `path` is NUL-terminated and `space` is a valid output buffer.
        unsafe {
            let mut space: libc::statvfs = std::mem::zeroed();
            if libc::statvfs(path.as_ptr(), &mut space) == 0 {
                let block = space.f_frsize as u64;
                let total = block.saturating_mul(space.f_blocks as u64);
                let available = block.saturating_mul(space.f_bavail as u64);
                stats.storage_total_gb = Some(total as f32 / 1_073_741_824.0);
                stats.storage_available_gb = Some(available as f32 / 1_073_741_824.0);
            }
        }
    }
    stats.uptime = std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|v| v.split_whitespace().next()?.parse::<f64>().ok())
        .map(|seconds| seconds as u64)
        .map(|seconds| {
            let days = seconds / 86_400;
            let hours = (seconds / 3_600) % 24;
            let minutes = (seconds / 60) % 60;
            if days > 0 {
                format!("{days} T. {hours} Std.")
            } else {
                format!("{hours} Std. {minutes} Min.")
            }
        })
        .unwrap_or_else(|| "unbekannt".into());
    stats.load_1m = std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|v| v.split_whitespace().next()?.parse::<f32>().ok());
    stats.kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|v| v.trim().to_string())
        .unwrap_or_else(|_| "LibreELEC".into());
    stats.hardware = std::fs::read_to_string("/proc/device-tree/model")
        .map(|v| v.trim_matches('\0').trim().to_string())
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "Linux-Gerät".into());
    stats.cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    stats.removable_drives = media_roots().len();
    (stats, counters)
}

#[derive(Clone, Debug, Default)]
pub struct NetworkDetails {
    pub connected: bool,
    pub kind: String,
    pub name: String,
    pub state: String,
    pub security: String,
    pub interface: String,
    pub hostname: String,
    pub ip: String,
    pub gateway: String,
    pub dns: String,
    pub strength: Option<u8>,
}

/// Read the active ConnMan service details without triggering a Wi-Fi scan.
pub fn network_details() -> NetworkDetails {
    let mut info = NetworkDetails::default();
    info.hostname = std::fs::read_to_string("/etc/hostname")
        .map(|v| v.trim().to_string())
        .unwrap_or_default();
    let Some(services) = run("connmanctl", &["services"]) else {
        info.ip = local_ip().unwrap_or_default();
        info.connected = !info.ip.is_empty();
        return info;
    };
    for line in services.lines() {
        let flags = line.get(..4).unwrap_or("");
        if !flags.contains('O') && !flags.contains('R') {
            continue;
        }
        let service = line.split_whitespace().last().unwrap_or("");
        let details = run("connmanctl", &["services", service]).unwrap_or_default();
        let value = |key: &str| {
            details.lines().find_map(|line| {
                line.trim_start()
                    .strip_prefix(key)
                    .and_then(|v| v.trim_start().strip_prefix('='))
                    .map(str::trim)
                    .map(str::to_string)
            })
        };
        let kind = if service.starts_with("wifi_") {
            "WLAN"
        } else {
            "LAN"
        };
        let raw_name = line
            .get(4..)
            .unwrap_or("")
            .trim()
            .rsplit_once(' ')
            .map(|part| part.0.trim())
            .unwrap_or("");
        let ipv4 = value("IPv4").unwrap_or_default();
        let inside = |key: &str| {
            ipv4.split_once(&format!("{key}="))
                .and_then(|(_, rest)| rest.split([',', '}', ' ']).next())
                .unwrap_or("")
                .trim_matches(['[', ']'])
                .to_string()
        };
        info.connected = true;
        info.kind = kind.into();
        info.name = raw_name.into();
        info.state = value("State").unwrap_or_else(|| "online".into());
        info.security = value("Security")
            .unwrap_or_default()
            .trim_matches(['[', ']'])
            .replace(',', " · ");
        info.interface = value("Ethernet")
            .and_then(|ethernet| {
                ethernet
                    .split_once("Interface=")
                    .and_then(|(_, rest)| rest.split([',', '}', ' ']).next())
                    .map(str::to_string)
            })
            .unwrap_or_default();
        info.ip = inside("Address");
        info.gateway = inside("Gateway");
        info.dns = value("Nameservers")
            .unwrap_or_default()
            .trim_matches(['[', ']'])
            .replace(',', " · ");
        info.strength = value("Strength").and_then(|v| v.parse().ok());
        if info.ip.is_empty() {
            info.ip = local_ip().unwrap_or_default();
        }
        break;
    }
    info
}
