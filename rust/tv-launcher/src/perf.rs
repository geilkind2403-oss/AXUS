//! Performance recorder ("Messbericht"): every animation with the time of
//! each of its frames, every screen opening, every key and how fast the
//! picture answered it, the timing lines the launcher logs anyway, and once
//! a minute temperature and memory.
//!
//! Recording is a push into a String under a mutex (no allocation on most
//! frames); a worker appends to `perf.log` in the data folder every 30 s, so
//! the SD card sees one small write instead of thousands. The file is capped
//! (the older half moves to `perf.log.1`). Only screen and animation names
//! and times are kept: no channel names, addresses or passwords.
//!
//! Line format (one event per line, times in ms):
//!   HH:MM:SS.mmm KEY  <key> <screen>
//!   HH:MM:SS.mmm OPEN <from>><to> <ms since the key> ms
//!   HH:MM:SS.mmm ANIM <screen> <after key> n=<frames> dauer=<ms> max=<ms> ueber16=<n> | <frame ms> ...
//!   HH:MM:SS.mmm IDLE <screen> n=<frames> schnitt=<ms> max=<ms>
//!   HH:MM:SS.mmm SYS  temp=<°C> rss=<MB> threads=<n>
//!   HH:MM:SS.mmm LOG  <a line the launcher logged>
use std::fmt::Write as _;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const FLUSH: Duration = Duration::from_secs(30);
const MAX_FILE: u64 = 20 << 20;

struct Anim {
    screen: &'static str,
    cause: String,
    started: Instant,
    last: Instant,
    frames: Vec<u16>, // tenths of a ms
}

struct Idle {
    screen: &'static str,
    n: u32,
    sum: f32,
    max: f32,
    since: Instant,
}

struct Rec {
    out: String,
    last_key: Option<(Instant, String)>,
    key_answered: bool,
    screen: &'static str,
    anim: Option<Anim>,
    idle: Option<Idle>,
    last_sys: Instant,
}

static REC: Mutex<Option<Rec>> = Mutex::new(None);
/// Serializes log appends with a reset so a flush already in flight cannot
/// recreate telemetry files after the user clears them.
static FILES: Mutex<()> = Mutex::new(());

fn stamp(out: &mut String) {
    let (h, m, s) = crate::app::local_hms();
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_millis())
        .unwrap_or(0);
    let _ = write!(out, "{h:02}:{m:02}:{s:02}.{ms:03} ");
}

fn with(f: impl FnOnce(&mut Rec)) {
    let mut g = REC.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(r) = g.as_mut() {
        f(r);
    }
}

/// Starts the recorder and its writer (once, at launcher start).
pub fn start() {
    {
        let mut g = REC.lock().unwrap_or_else(|e| e.into_inner());
        if g.is_some() {
            return;
        }
        let mut out = String::with_capacity(64 << 10);
        stamp(&mut out);
        let _ = writeln!(
            out,
            "START TV Launcher {} Build {}",
            crate::updater::running_version(),
            crate::BUILD
        );
        *g = Some(Rec {
            out,
            last_key: None,
            key_answered: true,
            screen: "",
            anim: None,
            idle: None,
            last_sys: Instant::now(),
        });
    }
    let _ = std::thread::Builder::new()
        .name("perf-writer".into())
        .spawn(|| {
            loop {
                std::thread::sleep(FLUSH);
                flush();
            }
        });
}

/// Writes what was recorded so far to the file (also before an export).
pub fn flush() {
    let _files = FILES.lock().unwrap_or_else(|e| e.into_inner());
    let text = {
        let mut g = REC.lock().unwrap_or_else(|e| e.into_inner());
        match g.as_mut() {
            Some(r) if !r.out.is_empty() => std::mem::take(&mut r.out),
            _ => return,
        }
    };
    let dir = crate::app::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("perf.log");
    if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > MAX_FILE {
        let _ = std::fs::rename(&path, dir.join("perf.log.1"));
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = f.write_all(text.as_bytes());
    }
}

/// Clears old in-memory samples and both saved report files. Recording then
/// continues with fresh measurements for the next report.
pub fn reset() -> Result<(), String> {
    let _files = FILES.lock().unwrap_or_else(|e| e.into_inner());
    let dir = crate::app::data_dir();
    let failure = clear_report_files(&dir)
        .err()
        .map(|error| error.to_string());
    {
        let mut g = REC.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(r) = g.as_mut() {
            r.out.clear();
            r.last_key = None;
            r.key_answered = true;
            r.screen = "";
            r.anim = None;
            r.idle = None;
            r.last_sys = Instant::now();
            stamp(&mut r.out);
            let _ = writeln!(r.out, "RESET performance data cleared; recording continues");
        }
    }
    failure.map_or(Ok(()), Err)
}

fn clear_report_files(dir: &std::path::Path) -> std::io::Result<()> {
    let mut failure = None;
    for name in ["perf.log", "perf.log.1"] {
        if let Err(error) = std::fs::remove_file(dir.join(name)) {
            if error.kind() != std::io::ErrorKind::NotFound && failure.is_none() {
                failure = Some(error);
            }
        }
    }
    failure.map_or(Ok(()), Err)
}

/// A key press reached the launcher.
pub fn key(name: &str, screen: &str) {
    with(|r| {
        stamp(&mut r.out);
        let _ = writeln!(r.out, "KEY  {name} {screen}");
        r.last_key = Some((Instant::now(), name.to_string()));
        r.key_answered = false;
    });
}

/// A line the launcher logged (its own timing lines go into the report too).
pub fn note(line: &str) {
    with(|r| {
        stamp(&mut r.out);
        let _ = writeln!(r.out, "LOG  {line}");
    });
}

fn end_anim(r: &mut Rec) {
    if let Some(a) = r.anim.take() {
        let n = a.frames.len();
        let max = a.frames.iter().copied().max().unwrap_or(0) as f32 / 10.0;
        let over = a.frames.iter().filter(|f| **f > 167).count();
        let dauer = a.last.duration_since(a.started).as_secs_f32() * 1000.0;
        stamp(&mut r.out);
        let _ = write!(
            r.out,
            "ANIM {} {} n={n} dauer={dauer:.0} max={max:.1} ueber16={over} |",
            a.screen, a.cause
        );
        for f in &a.frames {
            let _ = write!(r.out, " {}.{}", f / 10, f % 10);
        }
        r.out.push('\n');
    }
}

fn end_idle(r: &mut Rec) {
    if let Some(i) = r.idle.take() {
        if i.n > 0 {
            stamp(&mut r.out);
            let _ = writeln!(
                r.out,
                "IDLE {} n={} schnitt={:.1} max={:.1}",
                i.screen,
                i.n,
                i.sum / i.n as f32,
                i.max
            );
        }
    }
}

/// One picture was drawn and shown: `ms` from the start of drawing until
/// the TV had it. `busy`: the screen is in a real animation (not only an
/// idle effect like the App-Kreis orb).
pub fn frame(screen: &'static str, busy: bool, ms: f32) {
    with(|r| {
        let now = Instant::now();
        if screen != r.screen {
            end_anim(r);
            end_idle(r);
            stamp(&mut r.out);
            let since = r
                .last_key
                .as_ref()
                .map(|(t, _)| format!("{:.0}", now.duration_since(*t).as_secs_f32() * 1000.0))
                .unwrap_or_else(|| "-".into());
            let _ = writeln!(r.out, "OPEN {}>{screen} {since} ms", r.screen);
            r.screen = screen;
        }
        // the first picture after a key: how long the TV took to answer
        let cause = match &r.last_key {
            Some((t, k)) if !r.key_answered => {
                r.key_answered = true;
                stamp(&mut r.out);
                let _ = writeln!(
                    r.out,
                    "ANTW {k} {screen} {:.1} ms",
                    now.duration_since(*t).as_secs_f32() * 1000.0
                );
                Some(k.clone())
            }
            _ => None,
        };
        let tenths = (ms * 10.0).round().clamp(0.0, 65535.0) as u16;
        if busy {
            // a new key during an animation starts a new entry
            if cause.is_some() {
                end_anim(r);
            }
            let a = r.anim.get_or_insert_with(|| Anim {
                screen,
                cause: cause.clone().unwrap_or_else(|| "von selbst".into()),
                started: now,
                last: now,
                frames: Vec::with_capacity(64),
            });
            a.frames.push(tenths);
            a.last = now;
        } else {
            end_anim(r);
            let i = r.idle.get_or_insert_with(|| Idle {
                screen,
                n: 0,
                sum: 0.0,
                max: 0.0,
                since: now,
            });
            i.n += 1;
            i.sum += ms;
            i.max = i.max.max(ms);
            if now.duration_since(i.since) >= Duration::from_secs(60) {
                end_idle(r);
            }
        }
        if now.duration_since(r.last_sys) >= Duration::from_secs(60) {
            r.last_sys = now;
            sys(&mut r.out);
        }
    });
}

fn sys(out: &mut String) {
    let temp = crate::platform::system::cpu_temp()
        .map(|t| format!("{t:.0}"))
        .unwrap_or_else(|| "-".into());
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let field = |k: &str| {
        status
            .lines()
            .find(|l| l.starts_with(k))
            .and_then(|l| l.split_whitespace().nth(1))
            .unwrap_or("-")
            .to_string()
    };
    let rss = field("VmRSS:")
        .parse::<f32>()
        .map(|kb| format!("{:.1}", kb / 1024.0))
        .unwrap_or_else(|_| "-".into());
    let mem = meminfo();
    let m = |k: &str| mem.iter().find(|(n, _)| n == k).map(|(_, v)| format!("{}", v / 1024)).unwrap_or_else(|| "-".into());
    stamp(out);
    let _ = writeln!(
        out,
        "SYS  temp={temp} rss={rss} threads={} mhz={} frei={} cma_frei={} drossel={}",
        field("Threads:"),
        cpu_mhz().map(|v| v.to_string()).unwrap_or_else(|| "-".into()),
        m("MemAvailable"),
        m("CmaFree"),
        throttled().unwrap_or_else(|| "-".into()),
    );
}

/// /proc/meminfo as (name, kB).
fn meminfo() -> Vec<(String, u64)> {
    std::fs::read_to_string("/proc/meminfo")
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            Some((k.to_string(), v.split_whitespace().next()?.parse().ok()?))
        })
        .collect()
}

fn read_trim(p: &str) -> Option<String> {
    std::fs::read_to_string(p).ok().map(|s| s.trim_matches(|c: char| c.is_whitespace() || c == '\0').to_string())
}

/// Current CPU clock in MHz.
fn cpu_mhz() -> Option<u64> {
    read_trim("/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq")?
        .parse::<u64>()
        .ok()
        .map(|k| k / 1000)
}

/// The Pi firmware's throttle flags (0x0 = never slowed down).
fn throttled() -> Option<String> {
    read_trim("/sys/devices/platform/soc/soc:firmware/get_throttled")
        .or_else(|| {
            let out = std::process::Command::new("vcgencmd").arg("get_throttled").output().ok()?;
            let t = String::from_utf8_lossy(&out.stdout).trim().to_string();
            Some(t.trim_start_matches("throttled=").to_string())
        })
        .filter(|t| !t.is_empty())
}

/// What this machine is: the head of every report.
fn system_section() -> String {
    let mut s = String::from("== System ==\n");
    let mem = meminfo();
    let mb = |k: &str| mem.iter().find(|(n, _)| n == k).map(|(_, v)| format!("{} MB", v / 1024)).unwrap_or_else(|| "-".into());
    let model = read_trim("/proc/device-tree/model").unwrap_or_else(|| "-".into());
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let maxf = read_trim("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq")
        .and_then(|v| v.parse::<u64>().ok())
        .map(|k| format!("{} MHz", k / 1000))
        .unwrap_or_else(|| "-".into());
    let gov = read_trim("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor").unwrap_or_else(|| "-".into());
    let kernel = read_trim("/proc/sys/kernel/osrelease").unwrap_or_else(|| "-".into());
    let uptime = read_trim("/proc/uptime")
        .and_then(|u| u.split_whitespace().next()?.parse::<f32>().ok())
        .map(|u| format!("{:.0} min", u / 60.0))
        .unwrap_or_else(|| "-".into());
    let temp = crate::platform::system::cpu_temp().map(|t| format!("{t:.0} °C")).unwrap_or_else(|| "-".into());
    let _ = writeln!(s, "Gerät: {model}");
    let _ = writeln!(s, "Kerne: {cores}, Takt jetzt {} MHz, max {maxf}, Regler {gov}", cpu_mhz().map(|v| v.to_string()).unwrap_or_else(|| "-".into()));
    let _ = writeln!(s, "Drosselung (0x0 = nie): {}", throttled().unwrap_or_else(|| "-".into()));
    let _ = writeln!(s, "Temperatur: {temp}");
    let _ = writeln!(s, "RAM: gesamt {}, verfügbar {}, Swap frei {}", mb("MemTotal"), mb("MemAvailable"), mb("SwapFree"));
    let _ = writeln!(s, "Videospeicher (CMA): gesamt {}, frei {}", mb("CmaTotal"), mb("CmaFree"));
    let _ = writeln!(s, "Kernel: {kernel}, läuft seit {uptime}");
    let _ = writeln!(
        s,
        "Zeichnen: {}x{} (Skala {}), Sparmodus-Erkennung {}",
        (crate::gfx::W as f32 * crate::gfx::render_scale()) as u32,
        (crate::gfx::H as f32 * crate::gfx::render_scale()) as u32,
        crate::gfx::render_scale(),
        if crate::gfx::low_end() { "schwache Hardware" } else { "normal" }
    );
    let mut status = String::new();
    sys(&mut status);
    let _ = writeln!(s, "Jetzt: {}", status.get(13..).unwrap_or("").trim());
    if let Ok(c) = std::fs::read_to_string(crate::app::data_dir().join("crash.log")) {
        let _ = writeln!(s, "\n== Abstürze (crash.log, neueste zuletzt) ==");
        let lines: Vec<&str> = c.lines().collect();
        for l in &lines[lines.len().saturating_sub(40)..] {
            let _ = writeln!(s, "{l}");
        }
    } else {
        let _ = writeln!(s, "\nAbstürze: keine aufgezeichnet");
    }
    s
}

/// Records a crash (panic) with where it happened, kept across restarts.
pub fn install_crash_log() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let screen = REC
            .try_lock()
            .ok()
            .and_then(|g| g.as_ref().map(|r| r.screen))
            .unwrap_or("?");
        let mut line = String::new();
        stamp(&mut line);
        let _ = write!(
            line,
            "ABSTURZ v{} Build {} Bereich {screen}: {info}",
            crate::updater::running_version(),
            crate::BUILD
        );
        let line = line.replace('\n', " ");
        let path = crate::app::data_dir().join("crash.log");
        let _ = std::fs::create_dir_all(crate::app::data_dir());
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            use std::io::Write;
            let _ = writeln!(f, "{line}");
        }
        // never block here: the panic may have happened under the lock
        if let Ok(mut g) = REC.try_lock() {
            if let Some(r) = g.as_mut() {
                r.out.push_str(&line);
                r.out.push('\n');
            }
        }
        prev(info);
    }));
}

/// The report for the phone: a summary of the slowest things, then all
/// recorded lines (older file first).
pub fn report() -> String {
    flush();
    let dir = crate::app::data_dir();
    let mut raw = String::new();
    for name in ["perf.log.1", "perf.log"] {
        if let Ok(t) = std::fs::read_to_string(dir.join(name)) {
            raw.push_str(&t);
        }
    }
    summarize(&raw) + "\n===== ALLE MESSUNGEN =====\n" + &raw
}

/// Worst animations, slowest openings and answers, per-screen frame figures.
fn summarize(raw: &str) -> String {
    use std::collections::BTreeMap;
    let mut anims: Vec<(f32, &str)> = Vec::new();
    let mut opens: Vec<(f32, &str)> = Vec::new();
    let mut answers: Vec<(f32, &str)> = Vec::new();
    // per screen: frames, sum, over 16 ms, worst
    let mut screens: BTreeMap<String, (u64, f64, u64, f32)> = BTreeMap::new();
    let mut all_frames: BTreeMap<String, Vec<f32>> = BTreeMap::new();
    // per test step (self test): order, frames, worst answer
    let mut steps: Vec<(String, Vec<f32>, f32)> = Vec::new();
    let mut in_test = false;
    for line in raw.lines() {
        let body = line.get(13..).unwrap_or("");
        if let Some(name) = body.strip_prefix("LOG  TEST ") {
            if name.starts_with("ende") {
                in_test = false;
            } else {
                in_test = true;
                steps.push((name.to_string(), Vec::new(), 0.0));
            }
            continue;
        }
        if in_test {
            if let Some(st) = steps.last_mut() {
                if let Some(rest) = body.strip_prefix("ANTW ") {
                    if let Some(ms) = rest.split_whitespace().nth(2).and_then(|v| v.parse::<f32>().ok()) {
                        st.2 = st.2.max(ms);
                    }
                }
                if let Some(rest) = body.strip_prefix("ANIM ") {
                    if let Some(fr) = rest.split('|').nth(1) {
                        st.1.extend(fr.split_whitespace().filter_map(|v| v.parse::<f32>().ok()));
                    }
                }
            }
        }
        if let Some(rest) = body.strip_prefix("ANIM ") {
            if let Some(fr) = rest.split('|').nth(1) {
                let screen = rest.split_whitespace().next().unwrap_or("").to_string();
                all_frames
                    .entry(screen)
                    .or_default()
                    .extend(fr.split_whitespace().filter_map(|v| v.parse::<f32>().ok()));
            }
        }
        if let Some(rest) = body.strip_prefix("ANIM ") {
            let screen = rest.split_whitespace().next().unwrap_or("").to_string();
            let max = rest
                .split("max=")
                .nth(1)
                .and_then(|v| v.split_whitespace().next())
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(0.0);
            anims.push((max, line));
            let e = screens.entry(screen).or_default();
            if let Some(frames) = rest.split('|').nth(1) {
                for f in frames
                    .split_whitespace()
                    .filter_map(|v| v.parse::<f32>().ok())
                {
                    e.0 += 1;
                    e.1 += f as f64;
                    e.2 += (f > 16.7) as u64;
                    e.3 = e.3.max(f);
                }
            }
        } else if let Some(rest) = body.strip_prefix("OPEN ") {
            if let Some(ms) = rest
                .split_whitespace()
                .nth(1)
                .and_then(|v| v.parse::<f32>().ok())
            {
                opens.push((ms, line));
            }
        } else if let Some(rest) = body.strip_prefix("ANTW ") {
            if let Some(ms) = rest
                .split_whitespace()
                .nth(2)
                .and_then(|v| v.parse::<f32>().ok())
            {
                answers.push((ms, line));
            }
        }
    }
    let by = |v: &mut Vec<(f32, &str)>| v.sort_by(|a, b| b.0.total_cmp(&a.0));
    by(&mut anims);
    by(&mut opens);
    by(&mut answers);
    let mut s = String::new();
    let _ = writeln!(
        s,
        "MESSBERICHT TV Launcher {} Build {}\n",
        crate::updater::running_version(),
        crate::BUILD
    );
    if !cfg!(test) {
        s.push_str(&system_section());
        s.push('\n');
    }
    let pct = |v: &mut Vec<f32>, p: f32| -> f32 {
        if v.is_empty() {
            return 0.0;
        }
        v.sort_by(|a, b| a.total_cmp(b));
        v[((v.len() - 1) as f32 * p).round() as usize]
    };
    let _ = writeln!(s, "== Bildzeiten je Bereich (Median / 95 % / 99 %) ==");
    for (name, v) in all_frames.iter_mut() {
        let (a, b, c) = (pct(v, 0.5), pct(v, 0.95), pct(v, 0.99));
        let _ = writeln!(s, "{name}: {} Bilder, Median {a:.1} ms, 95 % unter {b:.1} ms, 99 % unter {c:.1} ms", v.len());
    }
    if !steps.is_empty() {
        let _ = writeln!(s, "\n== Testlauf je Schritt (Bilder, Median, schlechtestes Bild, langsamste Antwort) ==");
        for (name, v, ans) in steps.iter_mut() {
            let n = v.len();
            let worst = v.iter().cloned().fold(0.0f32, f32::max);
            let med = pct(v, 0.5);
            let flag = if worst > 16.7 || *ans > 16.7 { "  <-- zu langsam" } else { "" };
            let _ = writeln!(s, "{name}: {n} Bilder, Median {med:.1} ms, schlechtestes {worst:.1} ms, Antwort {ans:.1} ms{flag}");
        }
    }
    s.push('\n');
    let _ = writeln!(
        s,
        "== Animationen je Bereich (Bilder, Schnitt, über 16,7 ms, schlechtestes) =="
    );
    for (name, (n, sum, over, worst)) in &screens {
        if *n > 0 {
            let _ = writeln!(
                s,
                "{name}: {n} Bilder, Schnitt {:.1} ms, {over} über 16,7 ms ({:.0} %), schlechtestes {worst:.1} ms",
                sum / *n as f64,
                *over as f64 * 100.0 / *n as f64
            );
        }
    }
    for (title, list) in [
        (
            "== Langsamste Animationen (nach schlechtestem Bild) ==",
            &anims,
        ),
        ("== Langsamste Öffnungen (Taste bis erstes Bild) ==", &opens),
        ("== Langsamste Antworten auf Tasten ==", &answers),
    ] {
        let _ = writeln!(s, "\n{title}");
        for (_, l) in list.iter().take(25) {
            let cut: String = l.chars().take(220).collect();
            let _ = writeln!(s, "{cut}");
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_reads_its_own_lines() {
        let raw = "\
12:00:00.000 ANIM home Right n=3 dauer=40 max=31.0 ueber16=1 | 14.0 31.0 15.5
12:00:01.000 OPEN home>apps 420 ms
12:00:01.000 ANTW Ok home 22.5 ms
";
        let s = summarize(raw);
        assert!(s.contains("home: 3 Bilder"), "{s}");
        assert!(s.contains("1 über 16,7 ms"), "{s}");
        assert!(s.contains("OPEN home>apps 420 ms"), "{s}");
        assert!(s.contains("ANTW Ok home 22.5 ms"), "{s}");
    }

    #[test]
    fn test_steps_are_summarized() {
        let raw = "\
12:00:00.000 LOG  TEST Live TV Liste
12:00:00.100 ANTW Down livetv 40.0 ms
12:00:00.200 ANIM livetv Down n=2 dauer=20 max=40.0 ueber16=1 | 12.0 40.0
12:00:01.000 LOG  TEST Wetter Animation
12:00:01.200 ANIM weather x n=2 dauer=20 max=9.0 ueber16=0 | 8.0 9.0
12:00:02.000 LOG  TEST ende
";
        let s = summarize(raw);
        assert!(s.contains("Live TV Liste: 2 Bilder"), "{s}");
        assert!(s.contains("Antwort 40.0 ms  <-- zu langsam"), "{s}");
        assert!(s.contains("Wetter Animation: 2 Bilder, Median 9.0 ms"), "{s}");
        assert!(s.contains("livetv: 2 Bilder, Median"), "{s}");
    }

    #[test]
    fn reset_removes_current_and_rotated_reports() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("perf-reset-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("perf.log"), "old current data").unwrap();
        std::fs::write(dir.join("perf.log.1"), "old rotated data").unwrap();

        clear_report_files(&dir).unwrap();

        assert!(!dir.join("perf.log").exists());
        assert!(!dir.join("perf.log.1").exists());
        clear_report_files(&dir).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod flow {
    use super::*;

    #[test]
    fn key_animation_and_open_are_recorded() {
        let dir = std::env::temp_dir().join(format!("perf-test-{}", std::process::id()));
        std::env::set_var("TV_DATA", &dir);
        start();
        key("Right", "home");
        frame("home", true, 12.0);
        frame("home", true, 31.0);
        frame("home", false, 2.0);
        key("Ok", "home");
        frame("apps", true, 9.0);
        frame("apps", false, 1.0);
        let r = report();
        assert!(r.contains("ANTW Right home"), "{r}");
        assert!(r.contains("ANIM home Right n=2"), "{r}");
        assert!(r.contains("| 12.0 31.0"), "{r}");
        assert!(r.contains("OPEN home>apps"), "{r}");
        assert!(r.contains("ANIM apps Ok n=1"), "{r}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
