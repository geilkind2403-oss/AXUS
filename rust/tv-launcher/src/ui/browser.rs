//! The real YouTube TV app: WPE WebKit + cog (built by .github/workflows/
//! browser.yml, installed to /storage/tv-launcher/browser), started in a
//! chroot so it brings all its own libraries. The launcher hands over the
//! screen (DRM master), types TV-remote and phone keys into it through a
//! virtual keyboard and comes back on Home -- or when the app exits.
use super::*;
use crate::app::{Ctx, Nav, Screen};
use crate::gfx::{color, Align, Canvas, Rgba, Weight};
#[cfg(target_os = "linux")]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::Ordering::Relaxed;
use std::time::Instant;

pub const YOUTUBE_TV: &str = "https://www.youtube.com/tv";
pub const PRIME_VIDEO: &str = "https://www.primevideo.com/";

pub fn root() -> PathBuf {
    crate::app::data_dir().join("..").join("browser")
}

pub fn installed() -> bool {
    root().join("usr/bin/cog").exists()
}

fn mounted(p: &Path) -> bool {
    let p = p.canonicalize().unwrap_or(p.to_path_buf());
    std::fs::read_to_string("/proc/mounts")
        .map(|m| {
            m.lines()
                .any(|l| l.split_whitespace().nth(1) == Some(&p.to_string_lossy()))
        })
        .unwrap_or(false)
}

/// Hide host keyboards/remotes from Cog, then expose only the launcher's
/// virtual keyboard. This lets the launcher take focus for its PiP menu while
/// the browser keeps its DRM primary plane and video playback running.
fn isolate_browser_input(root: &Path) -> Result<(), String> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let host_event = crate::platform::uinput::launcher_event_path()
        .ok_or_else(|| "Virtuelle Fernbedienung in /sys/class/input nicht gefunden".to_string())?;
    let Some(event_name) = host_event.file_name() else {
        return Err("Name des virtuellen Eingabegeräts fehlt".into());
    };
    let rdev = std::fs::metadata(&host_event)
        .map_err(|e| format!("{}: {e}", host_event.display()))?
        .rdev();
    let input_dir = root.join("dev/input");
    let _ = std::fs::create_dir_all(&input_dir);
    let chroot_event = input_dir.join(event_name);
    let canon = |p: &Path| {
        p.canonicalize()
            .unwrap_or_else(|_| p.to_path_buf())
            .to_string_lossy()
            .into_owned()
    };
    let input_mount = canon(&input_dir);
    // the old way left an "input-empty" folder with mounts on it
    let empty_mount = canon(&root.join("input-empty"));
    let mounts_below = |base: &str| -> Vec<(String, String)> {
        std::fs::read_to_string("/proc/mounts")
            .unwrap_or_default()
            .lines()
            .filter_map(|line| {
                let mut f = line.split_whitespace();
                let _src = f.next()?;
                let target = f.next()?.replace("\\040", " ");
                let fstype = f.next()?.to_string();
                Some((target, fstype))
            })
            .filter(|(t, _)| t == base || t.starts_with(&(base.to_string() + "/")))
            .collect()
    };

    // Already right: our own tmpfs on dev/input and the device node in it
    // with the launcher's current device number.
    let here = mounts_below(&input_mount);
    if here.len() == 1
        && here[0].1 == "tmpfs"
        && std::fs::metadata(&chroot_event)
            .map(|m| m.file_type().is_char_device() && m.rdev() == rdev)
            .unwrap_or(false)
    {
        return Ok(());
    }

    // Tear down what is there (deepest first, lazily: a busy one goes when
    // its last user does), including leftovers of the old bind-mount setup.
    let mut old: Vec<String> = mounts_below(&input_mount)
        .into_iter()
        .chain(mounts_below(&empty_mount))
        .map(|(t, _)| t)
        .collect();
    old.sort_by_key(|p| std::cmp::Reverse(p.len()));
    old.dedup();
    for m in old {
        let _ = Command::new("umount").arg("-l").arg(&m).status();
    }

    // The chroot's /dev is a recursive bind of the host's: with shared
    // propagation a mount on its input folder would also land on the host's
    // /dev/input (and the launcher's own remotes would vanish). Private first.
    let dev_dir = root.join("dev");
    let _ = Command::new("mount").arg("--make-rprivate").arg(&dev_dir).status();

    // A small tmpfs that only holds the launcher's remote: Cog sees no
    // physical keyboard, so the launcher keeps the real remote for its menu.
    let ok = Command::new("mount")
        .args(["-t", "tmpfs", "-o", "mode=755,size=64k", "tmpfs"])
        .arg(&input_dir)
        .status()
        .map_err(|e| e.to_string())?
        .success();
    if !ok {
        return Err("Eingabeverzeichnis für den Browser konnte nicht angelegt werden".into());
    }
    if !crate::platform::uinput::wait_until_tagged(
        &host_event,
        std::time::Duration::from_millis(1500),
    ) {
        return Err(
            "Die virtuelle Fernbedienung wurde nicht als Tastatur erkannt. Der Browser wurde zum Schutz vor einer nicht bedienbaren Ansicht nicht gestartet.".into(),
        );
    }
    let c_path = std::ffi::CString::new(chroot_event.to_string_lossy().as_bytes())
        .map_err(|e| e.to_string())?;
    let r = unsafe { libc::mknod(c_path.as_ptr(), libc::S_IFCHR | 0o660, rdev as libc::dev_t) };
    if r != 0 {
        return Err(format!(
            "Virtuelle Fernbedienung konnte Cog nicht bereitgestellt werden: {}",
            std::io::Error::last_os_error()
        ));
    }
    crate::log("Browser-Eingabe isoliert: Cog empfängt Launcher-Fernbedienung, nicht die physischen Geräte");
    Ok(())
}

/// Script the browser's extension runs in every YouTube page: the quick
/// panel (browser/control.js) and, before it, tricks that make YouTube's
/// TV interface react at once on a Pi 3. Written into the bundle at every
/// start, so it is up to date without rebuilding the browser.
const CONTROL_JS: &str = include_str!("../../../../browser/control.js");
const SPEED_JS: &str = r#"(() => {
  if (window.__tvSpeedInstalled) return;
  window.__tvSpeedInstalled = true;
  // Focus moves, row slides and zooms are animated for 0.2-0.3 s; on a
  // Pi 3 they stutter and make every key press feel slow. 1 ms instead of
  // none keeps the page's transitionend/animationend events firing.
  // Blur filters and large shadows are among the costliest things to draw.
  const css = `
    *:not(#tv-quick-panel):not(#tv-quick-panel *),
    *:not(#tv-quick-panel):not(#tv-quick-panel *)::before,
    *:not(#tv-quick-panel):not(#tv-quick-panel *)::after {
      transition-duration: 1ms !important; transition-delay: 0s !important;
      animation-duration: 1ms !important; animation-delay: 0s !important;
      scroll-behavior: auto !important;
      backdrop-filter: none !important; -webkit-backdrop-filter: none !important;
      filter: none !important; box-shadow: none !important; text-shadow: none !important;
    }`;
  const style = document.createElement('style');
  style.id = 'tv-speed';
  style.textContent = css;
  (document.head || document.documentElement).appendChild(style);
})();
"#;

fn install_page_script(root: &Path) {
    let dir = root.join("usr/lib/tv-control");
    if !dir.is_dir() {
        return;
    }
    let script = format!("{SPEED_JS}\n{CONTROL_JS}");
    let path = dir.join("control.js");
    if std::fs::read_to_string(&path).ok().as_deref() != Some(script.as_str()) {
        if let Err(e) = std::fs::write(&path, script) {
            crate::log(&format!("Browser-Skript nicht geschrieben: {e}"));
        }
    }
}

/// /dev, /proc, /sys, /run/udev and a fresh /tmp inside the bundle.
fn prepare(root: &Path) -> Result<(), String> {
    let mounts: [(&str, &str, &[&str]); 5] = [
        ("dev", "/dev", &["-o", "rbind"]),
        ("proc", "proc", &["-t", "proc"]),
        ("sys", "/sys", &["-o", "rbind"]),
        ("run/udev", "/run/udev", &["-o", "bind"]),
        ("tmp", "tmpfs", &["-t", "tmpfs"]),
    ];
    for (dir, src, opts) in mounts {
        let target = root.join(dir);
        std::fs::create_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
        if mounted(&target) {
            continue;
        }
        let ok = Command::new("mount")
            .args(opts)
            .arg(src)
            .arg(&target)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            return Err(format!("mount {src} -> {} ging nicht", target.display()));
        }
    }
    isolate_browser_input(root)?;
    install_page_script(root);
    // name resolution as the host has it right now
    let resolv = std::fs::read("/etc/resolv.conf")
        .or_else(|_| std::fs::read("/run/connman/resolv.conf"))
        .unwrap_or_default();
    let _ = std::fs::write(root.join("etc/resolv.conf"), resolv);
    Ok(())
}

/// YouTube shows its TV interface (steered with arrow keys, light enough for
/// a Pi) only to TV browsers; any other browser gets the desktop site, made
/// for a mouse. Cog says it is a smart TV.
const TV_USER_AGENT: &str =
    "Mozilla/5.0 (SMART-TV; Linux; Tizen 5.0) AppleWebKit/538.1 (KHTML, like Gecko) Version/5.0 TV Safari/538.1";

/// Options the launcher adds to Cog: the TV identity and a cookie jar on the
/// Pi's storage (logins survive). `plain` leaves them out, for a Cog build
/// that does not know one of them.
fn extra_options(plain: bool) -> Vec<String> {
    if plain {
        return Vec::new();
    }
    vec![
        format!("--user-agent={TV_USER_AGENT}"),
        "--cookie-store=always".into(),
        // no smooth scrolling: the Pi draws far fewer frames per key press
        "--enable-smooth-scrolling=false".into(),
        "--cookie-jar=sqlite:/root/.local/share/cog/cookies.db".into(),
    ]
}

/// Settings › Bild & Bedienung › "YouTube in 720p": the browser draws the
/// interface at 1280x720 (less than half the pixels: faster menus); the TV
/// scales it up. Read by name, so it also works before the switch exists.
fn youtube_720p(cx: &Ctx) -> bool {
    serde_json::to_value(&cx.cfg)
        .ok()
        .and_then(|v| v.get("youtube_720p").and_then(|b| b.as_bool()))
        .unwrap_or(false)
}

fn launch(url: &str, drm_fd: Option<i32>, plain: bool, small: bool) -> Result<Child, String> {
    let root = root();
    let prepare_started = Instant::now();
    prepare(&root)?;
    crate::log(&format!(
        "Browser-Umgebung vorbereitet in {} ms",
        prepare_started.elapsed().as_millis()
    ));
    let log = std::fs::File::create(crate::app::data_dir().join("browser.log"))
        .map_err(|e| e.to_string())?;
    let log2 = log.try_clone().map_err(|e| e.to_string())?;
    let mut cmd = Command::new("chroot");
    cmd.arg(&root)
        .args([
            "/usr/bin/env",
            "-i",
            "HOME=/root",
            "XDG_RUNTIME_DIR=/tmp",
            // WebKit's page cache and site data on the Pi's storage (the
            // chroot's /root), not in the tmpfs: YouTube starts faster the
            // second time and stays logged in
            "XDG_CACHE_HOME=/root/.cache",
            "XDG_DATA_HOME=/root/.local/share",
            "LANG=de_DE.UTF-8",
            "TZ=CET-1CEST,M3.5.0,M10.5.0/3",
        ])
        // the launcher's mode (the menu over YouTube aligns pixel for
        // pixel), or 720p when chosen in Settings: the launcher sets its
        // mode again when the browser ends
        .arg(if small {
            "COG_PLATFORM_DRM_MODE_MAX=1280x720@60"
        } else {
            "COG_PLATFORM_DRM_MODE_MAX=1920x1080@60"
        })
        .args(["WEBKIT_GST_ENABLE_HLS_SUPPORT=1", "GST_V4L2_USE_LIBV4L2=0"]);
    #[cfg(target_os = "linux")]
    if let Some(fd) = drm_fd {
        cmd.arg("TV_DRM_SHARED_FD=198")
            .arg("LD_PRELOAD=/usr/lib/tv-control/libdrmshare.so");
        unsafe {
            cmd.pre_exec(move || {
                if fd != 198 && libc::dup2(fd, 198) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::fcntl(198, libc::F_SETFD, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = drm_fd;
    for dir in ["root/.cache", "root/.local/share/cog"] {
        let _ = std::fs::create_dir_all(root.join(dir));
    }
    cmd.args([
        "/usr/bin/cog",
        "--platform=drm",
        "--enable-mediasource=true",
        "--enable-webaudio=true",
    ])
    .args(extra_options(plain))
    .arg(url)
    .stdin(Stdio::null())
    .stdout(log)
    .stderr(log2)
    .spawn()
    .map_err(|e| format!("Browser startet nicht: {e}"))
}

pub struct Browser {
    url: String,
    requested: Instant,
    child: Option<Child>,
    started: Option<Instant>,
    error: Option<String>,
    frames: u32,
    cpu_governor: Option<String>,
    /// Started again without the launcher's extra options (Cog refused one).
    plain: bool,
}

impl Browser {
    pub fn new(url: &str) -> Browser {
        Browser {
            url: url.into(),
            requested: Instant::now(),
            child: None,
            started: None,
            error: None,
            frames: 0,
            cpu_governor: None,
            plain: false,
        }
    }

    fn service_name(&self) -> &'static str {
        if self.url.contains("primevideo.com") {
            "Prime Video"
        } else if self.url.contains("ardmediathek.de") {
            "ARD Mediathek"
        } else if self.url.contains("zdf.de") {
            "ZDF"
        } else if self.url.contains("arte.tv") {
            "ARTE Live"
        } else {
            "YouTube TV"
        }
    }

    fn service_id(&self) -> &'static str {
        if self.url.contains("primevideo.com") {
            "prime"
        } else if self.url.contains("ardmediathek.de") {
            "ard"
        } else if self.url.contains("zdf.de") {
            "zdf"
        } else if self.url.contains("arte.tv") {
            "arte"
        } else {
            "youtube"
        }
    }

    fn boost_cpu(&mut self) {
        #[cfg(target_os = "linux")]
        {
            let path = Path::new("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor");
            if let Ok(old) = std::fs::read_to_string(path) {
                let old = old.trim().to_owned();
                if old != "performance" && std::fs::write(path, "performance").is_ok() {
                    self.cpu_governor = Some(old);
                }
            }
        }
    }

    fn restore_cpu(&mut self) {
        #[cfg(target_os = "linux")]
        if let Some(old) = self.cpu_governor.take() {
            let path = Path::new("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor");
            let _ = std::fs::write(path, old);
        }
    }

    fn stop(&mut self, cx: &mut Ctx) {
        let stopped_at = Instant::now();
        if let Some(mut c) = self.child.take() {
            unsafe { libc::kill(c.id() as i32, libc::SIGTERM) };
            // Cog normally exits promptly. Bound the wait so a wedged WebKit
            // process cannot leave the launcher stuck for three seconds.
            for _ in 0..10 {
                if matches!(c.try_wait(), Ok(Some(_))) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            let _ = c.kill();
            let _ = c.wait();
        }
        crate::platform::input::BROWSER_ACTIVE.store(false, Relaxed);
        self.restore_cpu();
        #[cfg(target_os = "linux")]
        if let Some(k) = cx.kms.as_ref() {
            k.lock().unwrap().end_browser();
        }
        crate::log(&format!(
            "{} beendet, Launcher-Bildschirm zurück in {} ms",
            self.service_name(),
            stopped_at.elapsed().as_millis()
        ));
        let _ = cx;
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        // Keep the child and temporary CPU policy tidy if the screen is dropped
        // by an unexpected navigation path.
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        crate::platform::input::BROWSER_ACTIVE.store(false, Relaxed);
        self.restore_cpu();
    }
}

impl Screen for Browser {
    fn name(&self) -> &'static str {
        "browser"
    }
    fn home_focus(&self) -> Option<usize> {
        Some(if self.service_id() == "youtube" { 0 } else { 5 })
    }
    fn app_id(&self) -> Option<&'static str> {
        Some(self.service_id())
    }
    fn animating(&self, _: &Ctx) -> bool {
        // while the app runs the launcher sleeps (1 check a second): all CPU for YouTube
        self.child.is_none() && self.error.is_none()
    }
    fn poll(&mut self, cx: &mut Ctx) -> Nav {
        if self.error.is_some() {
            return Nav::None;
        }
        // show the splash for a couple of frames, then hand over the screen
        if self.child.is_none() {
            self.frames += 1;
            if self.frames < 3 {
                return Nav::None;
            }
            #[cfg(target_os = "linux")]
            if let Err(e) = crate::platform::uinput::open() {
                crate::log(&format!(
                    "Virtuelle Tastatur: {e} (TV-Fernbedienung geht dann im Browser nicht)"
                ));
            }
            #[cfg(target_os = "linux")]
            let drm_fd = cx.kms.as_ref().map(|k| k.lock().unwrap().start_browser());
            #[cfg(not(target_os = "linux"))]
            let drm_fd = None;
            self.boost_cpu();
            // decoded thumbnails and posters come back when they are needed again
            cx.images.trim();
            let small = youtube_720p(cx);
            if small {
                crate::log("YouTube in 720p (Einstellungen)");
            }
            match launch(&self.url, drm_fd, self.plain, small) {
                Ok(c) => {
                    crate::log(&format!(
                        "{} Browserprozess gestartet nach {} ms",
                        self.service_name(),
                        self.requested.elapsed().as_millis()
                    ));
                    self.child = Some(c);
                    self.started = Some(Instant::now());
                    crate::platform::input::BROWSER_ACTIVE.store(true, Relaxed);
                }
                Err(e) => {
                    self.restore_cpu();
                    crate::log(&e);
                    #[cfg(target_os = "linux")]
                    if let Some(k) = cx.kms.as_ref() {
                        k.lock().unwrap().end_browser();
                    }
                    self.error = Some(e);
                }
            }
            return Nav::None;
        }
        // the app quit on its own (YouTube's "exit")
        if let Some(c) = self.child.as_mut() {
            if let Ok(Some(status)) = c.try_wait() {
                let quick = self
                    .started
                    .map(|s| s.elapsed().as_secs() < 8)
                    .unwrap_or(false);
                self.child = None;
                self.stop(cx);
                // a Cog that does not know one of the extra options: once
                // more without them instead of an error
                let refused = std::fs::read_to_string(crate::app::data_dir().join("browser.log"))
                    .map(|l| {
                        let l = l.to_lowercase();
                        l.contains("unknown option") || l.contains("unrecognized option")
                    })
                    .unwrap_or(false);
                if quick && !self.plain && refused {
                    crate::log("Cog kennt eine Zusatzoption nicht: Start ohne Zusatzoptionen");
                    self.plain = true;
                    self.frames = 0;
                    return Nav::None;
                }
                if quick && !status.success() {
                    self.error = Some(format!(
                        "Der Browser ist gleich wieder ausgegangen ({status}). Details: {}",
                        crate::app::data_dir().join("browser.log").display()
                    ));
                    return Nav::None;
                }
                return Nav::Pop;
            }
        }
        Nav::None
    }
    fn key(&mut self, k: Key, cx: &mut Ctx) -> Nav {
        // Back is kept as an emergency exit if browser input injection fails.
        if self.child.is_some() {
            if k == Key::Back || k == Key::Home || k == Key::Power {
                self.stop(cx);
                return Nav::Home;
            }
            return Nav::None;
        }
        if self.error.is_some() {
            return Nav::Pop;
        }
        Nav::None
    }
    fn draw(&mut self, c: &mut Canvas, cx: &mut Ctx) {
        if self.child.is_some() {
            c.clear(Rgba(0, 0, 0, 0)); // Cog owns the primary plane; ARGB is only for the menu.
            c.trim_caches(); // the browser needs the memory more than we do
            return;
        }
        c.ambient(crate::gfx::Ambient::Color(Rgba::hex(0xd0202c)));
        if let Some(e) = &self.error {
            c.text(
                &format!("{} startet nicht", self.service_name()),
                PAD,
                220.0,
                34,
                Weight::Bold,
                color::TEXT,
                Align::Left,
            );
            c.paragraph(
                e,
                PAD,
                280.0,
                1100.0,
                18,
                Weight::Regular,
                color::MUTED,
                27.0,
                8,
            );
            c.text(
                "Eine Taste: zurück",
                PAD,
                480.0,
                18,
                Weight::Semi,
                color::ACCENT,
                Align::Left,
            );
            return;
        }
        icon_youtube(c, 640.0, 300.0, 140.0);
        c.text(
            &format!("{} wird gestartet …", self.service_name()),
            640.0,
            420.0,
            24,
            Weight::Semi,
            color::TEXT,
            Align::Center,
        );
        c.text(
            "Home-Taste: zurück zum Launcher",
            640.0,
            460.0,
            16,
            Weight::Regular,
            color::MUTED,
            Align::Center,
        );
        let _ = cx;
    }
    fn video(&self) -> bool {
        self.child.is_some()
    }
    fn overlay_empty(&self) -> bool {
        self.child.is_some()
    }
}
