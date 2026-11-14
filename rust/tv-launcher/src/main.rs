//! TV Launcher for the Raspberry Pi 3 on LibreELEC: one program that draws
//! straight to the TV (KMS), plays films and YouTube with the Pi's H.264
//! decoder, weather, news -- no Kodi, no browser, no X11.
//!
//!   tv-launcher                 run on the TV (needs /dev/dri, LibreELEC)
//!   tv-launcher --shot DIR      development: render every screen as PNG
mod app;
mod films;
mod gfx;
mod livetv;
mod net;
mod news;
mod platform;
mod player;
mod remote;
mod ui;
mod updater;
mod weather;
mod youtube;

#[cfg(target_os = "linux")]
pub use platform::alsa as platform_audio;

use app::{Ctx, Msg, Nav, Screen};
use platform::input::{InputMsg, Key};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// (seconds recording, seconds drawing) -- for the frame cost log
static TIMES: Mutex<(f64, f64)> = Mutex::new((0.0, 0.0));
static TRACE_CONTROL_PHASES: OnceLock<bool> = OnceLock::new();

/// Commit + date of this build (set by build-pi.sh).
pub const BUILD: &str = match option_env!("TV_BUILD") {
    Some(b) => b,
    None => "dev",
};

/// When the main loop last came round (seconds since start), for the watchdog.
static HEARTBEAT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// A main loop that has not come round for this long is stuck for good (it
/// normally passes at least once a second): a TV with a frozen picture that
/// ignores the remote is worse than two seconds of black.
const WATCHDOG_SECS: u64 = 120;

/// Ends the program when the UI thread hangs (waiting forever on the display
/// or the decoder, say); systemd starts it again (Restart=always).
fn start_watchdog(started: Instant) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(10));
        let silent = started
            .elapsed()
            .as_secs()
            .saturating_sub(HEARTBEAT.load(Relaxed));
        if silent > WATCHDOG_SECS {
            log(&format!(
                "Die Oberfläche reagiert seit {silent} s nicht mehr: der Launcher startet neu"
            ));
            std::process::abort();
        }
    });
}

pub fn log(msg: &str) {
    let (h, m, s) = app::local_hms();
    eprintln!("{h:02}:{m:02}:{s:02} {msg}");
}

struct App {
    cx: Ctx,
    stack: Vec<Box<dyn Screen>>,
    canvas: gfx::Canvas,
    dirty: bool,
    /// a screen change fades + slides the new screen in (dir: +1 forward, -1 back)
    fade: Option<(Instant, f32)>,
    last_input: Instant,
    restore_home_after_control: bool,
    control_blur_cache: Option<gfx::Image>,
    /// The picture under the notification, where the screen keeps its frame
    /// in the canvas (Home, Apps) instead of drawing it anew: put back before
    /// the next frame, or the notification smears and stays for good.
    toast_underlay: Option<(gfx::Image, f32, f32)>,
}

// The Pi 3 software renderer takes longer to draw a frame than newer TVs.
// Short transitions keep input feedback snappy on that target.
const FADE: f32 = if cfg!(target_arch = "arm") {
    0.10
} else {
    0.22
};
// Bound redraws to the capsule, card and their shadows. The old rectangle
// included a wide margin around the panel and added avoidable work per key.
const CONTROL_DAMAGE: (f32, f32, f32, f32) = (878.0, 16.0, 390.0, 470.0);

impl App {
    fn top(&mut self) -> &mut Box<dyn Screen> {
        self.stack.last_mut().unwrap()
    }
    fn apply(&mut self, nav: Nav) {
        let opening_overlay = matches!(&nav, Nav::Push(s) | Nav::Replace(s) if s.overlay());
        let opening_control =
            matches!(&nav, Nav::Push(s) | Nav::Replace(s) if s.name() == "control");
        let closing_overlay = matches!(&nav, Nav::Pop) && self.top().overlay();
        let restore_home_after_control = closing_overlay
            && self.top().name() == "control"
            && self.stack.len() > 1
            && self.stack[self.stack.len() - 2].name() == "home";
        let leave_browser = match &nav {
            Nav::Home => true,
            Nav::Push(s) | Nav::Replace(s) => s.name() != "control",
            _ => false,
        };
        if leave_browser {
            if let Some(index) = self.stack.iter().position(|s| s.name() == "browser") {
                // Drop Cog before restoring the launcher's primary plane. The
                // control panel itself is a Push/Pop above the browser, so it
                // never enters this path.
                self.stack.truncate(index);
                #[cfg(target_os = "linux")]
                if let Some(k) = self.cx.kms.as_ref() {
                    k.lock().unwrap().end_browser();
                }
                let action = nav;
                let nav = match action {
                    Nav::Replace(s) => Nav::Push(s),
                    other => other,
                };
                return self.apply(nav);
            }
        }
        let media_tile = match &nav {
            Nav::Push(s) | Nav::Replace(s) => s.home_focus().or_else(|| match s.name() {
                "browser" | "youtube" => Some(0),
                "livetv" => Some(1),
                "films" | "film" => Some(2),
                _ => None,
            }),
            _ => None,
        };
        if let Some(tile) = media_tile.filter(|tile| self.cx.cfg.last_media_tile != *tile) {
            self.cx.cfg.last_media_tile = tile;
            self.cx.save();
        }
        if let Nav::Push(s) | Nav::Replace(s) = &nav {
            if let Some(app_id) = s.app_id().filter(|id| self.cx.cfg.last_app_id != *id) {
                self.cx.cfg.last_app_id = app_id.into();
                self.cx.save();
            }
        }
        let dir = match nav {
            Nav::Push(_) | Nav::Replace(_) => 1.0,
            _ => -1.0,
        };
        match nav {
            Nav::None => return,
            Nav::Push(s) => self.stack.push(s),
            Nav::Pop => {
                if self.stack.len() > 1 {
                    self.stack.pop();
                }
            }
            Nav::Replace(s) => {
                self.stack.pop();
                self.stack.push(s);
            }
            Nav::Home => self.stack.truncate(1),
        }
        if restore_home_after_control {
            self.restore_home_after_control = true;
        }
        if !opening_control || closing_overlay {
            self.control_blur_cache = None;
        }
        // An overlay changes only the OSD plane. Fading the base framebuffer
        // here makes the whole screen blink black and look like a window swap.
        self.fade = if opening_overlay || closing_overlay {
            None
        } else {
            Some((Instant::now(), dir))
        };
        crate::platform::input::CONTROL_OPEN.store(
            self.top().name() == "control",
            std::sync::atomic::Ordering::Release,
        );
        self.dirty = true;
    }
    fn fading(&self) -> bool {
        self.fade
            .map(|f| f.0.elapsed().as_secs_f32() < FADE)
            .unwrap_or(false)
    }
    fn key(&mut self, k: Key) {
        self.last_input = Instant::now();
        if self.top().name() == "home" && k != Key::LongOk {
            self.control_blur_cache = None;
        }
        if k == Key::LongBack {
            self.apply(Nav::Home);
            self.dirty = true;
            return;
        }
        if k == Key::LongOk && self.top().name() == "home" {
            let mut s = self.stack.pop().unwrap();
            let nav = s.key(k, &mut self.cx);
            self.stack.push(s);
            self.apply(nav);
            self.dirty = true;
            return;
        }
        if k == Key::LongOk && self.top().name() != "control" {
            let over_video = self.top().video();
            self.apply(Nav::Push(Box::new(ui::control::Control::new(over_video))));
            self.dirty = true;
            return;
        }
        // volume is global
        match k {
            Key::VolUp | Key::VolDown => {
                let v = &mut self.cx.cfg.volume;
                *v = if k == Key::VolUp {
                    (*v + 5).min(100)
                } else {
                    v.saturating_sub(5)
                };
                platform_audio::VOLUME.store(*v, Relaxed);
                platform_audio::MUTED.store(0, Relaxed);
                let v = *v;
                self.cx.toast(format!("Lautstärke {v} %"));
                self.cx.save();
                self.dirty = true;
                return;
            }
            Key::Mute => {
                let m = platform_audio::MUTED.fetch_xor(1, Relaxed) == 0;
                self.cx.toast(if m { "Ton aus" } else { "Ton an" });
                self.dirty = true;
                return;
            }
            _ => {}
        }
        let mut s = self.stack.pop().unwrap();
        let nav = s.key(k, &mut self.cx);
        self.stack.push(s);
        self.apply(nav);
        self.dirty = true;
    }
    /// Records and draws a frame; false when it's the same picture as before.
    fn frame(&mut self) -> bool {
        self.canvas.reset_damage();
        if let Some((under, x, y)) = self.toast_underlay.take() {
            self.canvas.restore_region_image(&under, x, y);
        }
        let mut s = self.stack.pop().unwrap();
        let nav = s.poll(&mut self.cx);
        self.stack.push(s);
        self.apply(nav);
        let mut s = self.stack.pop().unwrap();
        self.cx.now = Instant::now();
        let t0 = Instant::now();
        let trace_control = *TRACE_CONTROL_PHASES
            .get_or_init(|| std::env::var_os("SHOT_PHASE_TRACE").is_some())
            && s.name() == "control";
        let mut restore_ms = 0.0f64;
        let mut underlay_raster_ms = 0.0f64;
        let mut blur_ms = 0.0f64;
        // fade the new screen in over its backdrop (not over a film picture)
        let (op, dx) = match self.fade {
            Some((f, dir)) if !s.video() => {
                let k = gfx::ease(f.elapsed().as_secs_f32() / FADE);
                (k, (1.0 - k) * 48.0 * dir)
            }
            _ => (1.0, 0.0),
        };
        self.canvas.opacity = op;
        self.canvas.offset = (dx, 0.0);
        let restoring_home = self.restore_home_after_control && s.name() == "home";
        let partial_home_restore = restoring_home && self.cx.toast.is_none();
        if restoring_home && !partial_home_restore {
            self.canvas.set_full_base_restore(true);
        }
        let mut partial_control_overlay = false;
        if s.overlay() {
            if let Some(base) = self.stack.last_mut() {
                let use_partial_restore = s.name() == "control"
                    && !s.video()
                    && self.cx.toast.is_none()
                    && base.name() == "home";
                partial_control_overlay = use_partial_restore;
                self.canvas.set_full_base_restore(!use_partial_restore);
                if use_partial_restore {
                    let phase = trace_control.then(Instant::now);
                    base.draw_overlay_region(&mut self.canvas, &mut self.cx, CONTROL_DAMAGE);
                    restore_ms = phase
                        .map(|started| started.elapsed().as_secs_f64() * 1000.0)
                        .unwrap_or_default();
                } else {
                    base.draw(&mut self.canvas, &mut self.cx);
                }
                self.canvas.set_full_base_restore(false);
                let phase = trace_control.then(Instant::now);
                self.canvas.finish();
                underlay_raster_ms = phase
                    .map(|started| started.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or_default();
                if !s.video() {
                    if use_partial_restore {
                        if let Some(blurred) = self.control_blur_cache.as_ref() {
                            let phase = trace_control.then(Instant::now);
                            self.canvas.restore_region_image(
                                blurred,
                                CONTROL_DAMAGE.0,
                                CONTROL_DAMAGE.1,
                            );
                            blur_ms = phase
                                .map(|started| started.elapsed().as_secs_f64() * 1000.0)
                                .unwrap_or_default();
                        } else {
                            let phase = trace_control.then(Instant::now);
                            self.canvas.blur_region(
                                CONTROL_DAMAGE.0,
                                CONTROL_DAMAGE.1,
                                CONTROL_DAMAGE.2,
                                CONTROL_DAMAGE.3,
                            );
                            self.control_blur_cache = self.canvas.snapshot_region(
                                CONTROL_DAMAGE.0,
                                CONTROL_DAMAGE.1,
                                CONTROL_DAMAGE.2,
                                CONTROL_DAMAGE.3,
                            );
                            blur_ms = phase
                                .map(|started| started.elapsed().as_secs_f64() * 1000.0)
                                .unwrap_or_default();
                        }
                    } else {
                        let phase = trace_control.then(Instant::now);
                        self.canvas.blur_region(
                            CONTROL_DAMAGE.0,
                            CONTROL_DAMAGE.1,
                            CONTROL_DAMAGE.2,
                            CONTROL_DAMAGE.3,
                        );
                        blur_ms = phase
                            .map(|started| started.elapsed().as_secs_f64() * 1000.0)
                            .unwrap_or_default();
                    }
                }
            }
        }
        let phase = trace_control.then(Instant::now);
        if partial_home_restore {
            s.draw_overlay_region(&mut self.canvas, &mut self.cx, CONTROL_DAMAGE);
        } else {
            s.draw(&mut self.canvas, &mut self.cx);
        }
        if restoring_home {
            self.canvas.set_full_base_restore(false);
            self.restore_home_after_control = false;
        }
        self.canvas.opacity = 1.0;
        self.canvas.offset = (0.0, 0.0);
        // A screen that has drawn itself into the kept picture already: the
        // notification goes on top of those pixels, so remember what is under it.
        let toast_over_kept = match ui::toast_area(&mut self.canvas, &mut self.cx) {
            Some(area) if self.canvas.pending_empty() => {
                self.toast_underlay = self
                    .canvas
                    .snapshot_region(area.0, area.1, area.2, area.3)
                    .map(|under| (under, area.0, area.1));
                Some(area)
            }
            _ => None,
        };
        ui::toast(&mut self.canvas, &mut self.cx);
        let panel_record_ms = phase.map(|started| started.elapsed().as_secs_f64() * 1000.0);
        self.stack.push(s);
        let t1 = Instant::now();
        let phase = trace_control.then(Instant::now);
        let changed = if partial_control_overlay && self.cx.toast.is_none() {
            self.canvas.finish_pending_region(
                CONTROL_DAMAGE.0,
                CONTROL_DAMAGE.1,
                CONTROL_DAMAGE.2,
                CONTROL_DAMAGE.3,
            );
            true
        } else if let Some((x, y, w, h)) = toast_over_kept {
            self.canvas.finish_pending_region(x, y, w, h);
            true
        } else {
            self.canvas.finish()
        };
        let final_raster_ms = phase.map(|started| started.elapsed().as_secs_f64() * 1000.0);
        TIMES.lock().unwrap().0 += (t1 - t0).as_secs_f64();
        TIMES.lock().unwrap().1 += t1.elapsed().as_secs_f64();
        if trace_control {
            log(&format!(
                "Control-Phasen: Home-Restore {restore_ms:.2} ms, Hintergrund-Raster {underlay_raster_ms:.2} ms, Blur {blur_ms:.2} ms, Panel-Aufzeichnung {:.2} ms, Panel-Raster {:.2} ms",
                panel_record_ms.unwrap_or_default(),
                final_raster_ms.unwrap_or_default(),
            ));
        }
        changed
    }
}

fn background_refresh(cx: &mut Ctx) {
    let (lat, lon) = (cx.cfg.lat, cx.cfg.lon);
    app::refresh(
        &cx.tx,
        &cx.weather,
        move || weather::fetch(lat, lon),
        |w| w.is_ok(),
    );
    app::refresh(&cx.tx, &cx.news, news::fetch, |n| !n.is_empty());
    app::refresh(&cx.tx, &cx.net, platform::system::network_status, |_| true);
    app::refresh(
        &cx.tx,
        &cx.youtube_home,
        || {
            crate::youtube::home()
                .into_iter()
                .flat_map(|(_, items)| items)
                .take(12)
                .collect()
        },
        |v: &Vec<youtube::Video>| !v.is_empty(),
    );
}

/// Weather and headlines are there (else: no network yet, try again soon).
fn feeds_ready(cx: &Ctx) -> bool {
    cx.weather.with(|w| matches!(w, Some(Ok(_))))
        && cx.news.with(|n| n.map(|n| !n.is_empty()).unwrap_or(false))
}

/// Cache the small blurred Home patch after it has been presented. Its cost
/// is paid during idle time, so the first Control Center frame can reuse it.
fn prepare_control_blur(app: &mut App, top_name: &str, idle: Duration) {
    if top_name == "home" && idle >= Duration::from_secs(3) && app.control_blur_cache.is_none() {
        app.control_blur_cache = app.canvas.blurred_region_image(
            CONTROL_DAMAGE.0,
            CONTROL_DAMAGE.1,
            CONTROL_DAMAGE.2,
            CONTROL_DAMAGE.3,
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let shot_dir = args
        .iter()
        .position(|a| a == "--shot")
        .and_then(|i| args.get(i + 1).cloned());
    let cfg = app::Config::load();
    platform_audio::VOLUME.store(cfg.volume, Relaxed);
    let (tx, rx) = std::sync::mpsc::channel::<Msg>();

    #[cfg(target_os = "linux")]
    let kms: Option<app::KmsHandle> = if shot_dir.is_some() {
        None
    } else {
        match platform::kms::Kms::open(
            (gfx::W as f32 * gfx::RENDER_SCALE) as u32,
            (gfx::H as f32 * gfx::RENDER_SCALE) as u32,
        ) {
            Ok(k) => Some(Arc::new(Mutex::new(k))),
            Err(e) => {
                log(&format!("Kein Bildschirm: {e}"));
                std::process::exit(1);
            }
        }
    };

    let mut cx = Ctx {
        tx: tx.clone(),
        images: app::Images::new(tx.clone()),
        toast: None,
        weather: app::Slot::empty(),
        news: app::Slot::empty(),
        youtube_home: app::Slot::empty(),
        films: films::FilmLibrary::new(),
        net: app::Slot::empty(),
        started: Instant::now(),
        now: Instant::now(),
        remote_url: format!(
            "http://{}:{}",
            platform::system::local_ip().unwrap_or_else(|| "?".into()),
            remote::PORT
        ),
        quit: false,
        kms,
        cfg,
        channels: app::Slot::empty(),
        sleep_at: None,
    };
    background_refresh(&mut cx);
    {
        let (s, d, p) = (
            cx.cfg.film_server.clone(),
            cx.cfg.film_dirs.clone(),
            cx.cfg.positions.clone(),
        );
        cx.films.refresh(&tx, &s, &d, &p);
    }

    cx.reload_channels();
    let home = Box::new(ui::home::Home::new(
        cx.cfg.last_media_tile,
        cx.cfg.home_order,
    ));
    let mut app = App {
        cx,
        stack: vec![home],
        canvas: gfx::Canvas::new_scaled(gfx::W, gfx::H, gfx::RENDER_SCALE),
        dirty: true,
        fade: None,
        last_input: Instant::now(),
        restore_home_after_control: false,
        control_blur_cache: None,
        toast_underlay: None,
    };

    if let Some(dir) = shot_dir {
        screenshots(&mut app, &dir, &rx);
        return;
    }

    platform::input::start_evdev(fwd_input(tx.clone()));
    {
        let t2 = tx.clone();
        platform::cec::start(fwd_input(tx.clone()), move || {
            let _ = t2.send(Msg::Standby);
        });
    }
    remote::start(tx.clone(), app.cx.cfg.remote_enabled);
    if app.cx.cfg.start_livetv {
        let s = ui::livetv::LiveTv::new(&mut app.cx);
        app.stack.push(Box::new(s));
    }
    let mut sleep_warned = false;
    log(&format!(
        "TV Launcher {} (Build {}) läuft · Handy: {}",
        env!("CARGO_PKG_VERSION"),
        BUILD,
        app.cx.remote_url
    ));

    let mut last_tick = Instant::now();
    let mut last_frame = Instant::now();
    let mut stats = (0u32, 0f32, 0f32, Instant::now()); // frames, sum ms, max ms, since
    let mut skipped = 0u32;
    let mut shown_state = (false, false);
    let mut last_top = "";
    let mut last_refresh = Instant::now();
    start_watchdog(app.cx.started);
    loop {
        HEARTBEAT.store(app.cx.started.elapsed().as_secs(), Relaxed);
        let animating = app
            .stack
            .last()
            .map(|s| s.animating(&app.cx))
            .unwrap_or(false)
            || app.cx.toast.is_some()
            || app.fading()
            || app.canvas.ambient_moving();
        let video = app.stack.last().map(|s| s.video()).unwrap_or(false);
        let timeout = if app.dirty {
            Duration::ZERO
        } else if animating {
            // One frame per refresh of the TV (60 Hz). The pause counts from
            // the start of the last frame: drawing time is part of the frame,
            // not added on top of it (a 12 ms frame plus a 16 ms wait was 35
            // pictures a second instead of 60).
            // Over a film the OSD is only looked at ten times a second, which
            // leaves the CPU to the sound; at 30 pictures while something
            // actually glides (guide, channel banner).
            let gliding = app
                .stack
                .last()
                .map(|s| s.motion_over_video())
                .unwrap_or(false);
            Duration::from_millis(match (video, gliding) {
                (true, true) => 33,
                (true, false) => 100,
                _ => 16,
            })
            .saturating_sub(last_frame.elapsed())
        } else {
            Duration::from_millis(1000)
        };
        match rx.recv_timeout(timeout) {
            Ok(Msg::Input(InputMsg::Key(k))) => app.key(k),
            Ok(Msg::Input(InputMsg::Text(t))) => {
                app.last_input = Instant::now();
                app.control_blur_cache = None;
                let mut s = app.stack.pop().unwrap();
                s.text(&t, &mut app.cx);
                app.stack.push(s);
                app.dirty = true;
            }
            Ok(Msg::Redraw) => {
                app.control_blur_cache = None;
                app.dirty = true;
            }
            Ok(Msg::Run(f)) => {
                f(&mut app.cx);
                app.control_blur_cache = None;
                app.dirty = true;
            }
            Ok(Msg::Player(e)) => {
                let mut s = app.stack.pop().unwrap();
                let nav = s.player_event(&e, &mut app.cx);
                app.stack.push(s);
                app.apply(nav);
                app.dirty = true;
            }
            Ok(Msg::Standby) => {
                // TV switched off: stop what plays, back home
                if app.top().video() {
                    app.key(Key::Stop);
                }
                app.apply(Nav::Home);
            }
            Err(_) => {}
        }
        // drain whatever else arrived, then draw once
        while let Ok(m) = rx.try_recv() {
            match m {
                Msg::Input(InputMsg::Key(k)) => app.key(k),
                Msg::Input(InputMsg::Text(t)) => {
                    app.control_blur_cache = None;
                    let mut s = app.stack.pop().unwrap();
                    s.text(&t, &mut app.cx);
                    app.stack.push(s);
                }
                Msg::Redraw => app.control_blur_cache = None,
                Msg::Run(f) => {
                    f(&mut app.cx);
                    app.control_blur_cache = None;
                }
                Msg::Player(e) => {
                    let mut s = app.stack.pop().unwrap();
                    let nav = s.player_event(&e, &mut app.cx);
                    app.stack.push(s);
                    app.apply(nav);
                }
                Msg::Standby => app.apply(Nav::Home),
            }
            app.dirty = true;
        }
        // screensaver on idle menus (never over a film or TV)
        let top_video = app.stack.last().map(|s| s.video()).unwrap_or(false);
        let top_name = app.stack.last().map(|s| s.name()).unwrap_or("");
        let idle = app.last_input.elapsed();
        // Build the Control Center's static layer during quiet Home time,
        // never while video is playing or while the user is navigating.
        if top_name == "home" && !top_video && idle >= Duration::from_secs(3) {
            ui::control::Control::prewarm(&app.cx, false);
        }
        if app.cx.cfg.screensaver_min > 0
            && !top_video
            && top_name != "screensaver"
            && idle > Duration::from_secs(app.cx.cfg.screensaver_min as u64 * 60)
        {
            app.apply(Nav::Push(Box::new(ui::screensaver::ScreenSaver)));
        }
        // sleep timer: a warning a minute before, then stop and switch the TV off
        if let Some(at) = app.cx.sleep_at {
            let left = at.saturating_duration_since(Instant::now());
            if left <= Duration::from_secs(60) && !sleep_warned {
                sleep_warned = true;
                app.cx
                    .toast("Ausschalt-Timer: der Fernseher geht in einer Minute aus");
                app.dirty = true;
            }
            if left.is_zero() {
                app.cx.sleep_at = None;
                sleep_warned = false;
                app.apply(Nav::Home); // stops films / TV
                #[cfg(target_os = "linux")]
                platform::cec::tv_off();
                log("Ausschalt-Timer: Fernseher aus");
            }
        } else {
            sleep_warned = false;
        }
        // every ten minutes; every minute while something is still missing
        // (switched on before the WLAN was up)
        let refresh_every = if feeds_ready(&app.cx) { 600 } else { 60 };
        if last_refresh.elapsed() > Duration::from_secs(refresh_every) {
            last_refresh = Instant::now();
            background_refresh(&mut app.cx);
        }
        // the clock (and rotating tiles) once a second even when idle
        if app.dirty || animating || last_tick.elapsed() >= Duration::from_secs(1) {
            let t0 = Instant::now();
            ui::set_frame_dt(t0.duration_since(last_frame).as_secs_f32());
            last_frame = t0;
            last_tick = t0;
            app.dirty = false;
            // a new screen (e.g. back from the YouTube app, which had the
            // display) is always drawn and shown in full
            let top_now = app.stack.last().map(|s| s.name()).unwrap_or("");
            if top_now != last_top {
                app.canvas.invalidate();
                last_top = top_now;
            }
            let changed = app.frame();
            let top = app.stack.last().unwrap();
            let (video, empty) = (top.video(), top.overlay_empty());
            // same picture, same planes: nothing to hand to the TV
            if !changed && shown_state == (video, empty) {
                prepare_control_blur(&mut app, top_now, idle);
                skipped += 1;
                continue;
            }
            shown_state = (video, empty);
            let draw_ms = t0.elapsed().as_secs_f64() * 1000.0;
            let present_started = Instant::now();
            #[cfg(target_os = "linux")]
            if let Some(k) = app.cx.kms.as_ref() {
                let mut k = k.lock().unwrap();
                k.present_ui(app.canvas.pm.data(), !(video && empty), app.canvas.damage());
            }
            prepare_control_blur(&mut app, top_now, idle);
            let present_ms = present_started.elapsed().as_secs_f64() * 1000.0;
            // how long a frame takes (drawing + handing it to the screen)
            let ms = t0.elapsed().as_secs_f32() * 1000.0;
            if ms >= 32.0 {
                log(&format!("UI-Spitze: {ms:.1} ms gesamt ({draw_ms:.1} ms Zeichnen, {present_ms:.1} ms Übergabe), Bereich {top_now}"));
            }
            if animating && !video {
                stats.0 += 1;
                stats.1 += ms;
                stats.2 = stats.2.max(ms);
            }
            if stats.3.elapsed() > Duration::from_secs(10) {
                if stats.0 > 10 {
                    log(&format!("UI-Framelauf: {:.1} ms im Schnitt, max {:.1} ms, {} geänderte Frames, {} unveränderte übersprungen", stats.1 / stats.0 as f32, stats.2, stats.0, skipped));
                }
                stats = (0, 0.0, 0.0, Instant::now());
                skipped = 0;
            }
        }
        if app.cx.quit {
            break;
        }
    }
}

fn fwd_input(tx: std::sync::mpsc::Sender<Msg>) -> std::sync::mpsc::Sender<InputMsg> {
    let (itx, irx) = std::sync::mpsc::channel::<InputMsg>();
    std::thread::spawn(move || {
        for m in irx {
            if tx.send(Msg::Input(m)).is_err() {
                return;
            }
        }
    });
    itx
}

/// Development: walks through the screens and saves each as PNG.
fn screenshots(app: &mut App, dir: &str, rx: &std::sync::mpsc::Receiver<Msg>) {
    std::fs::create_dir_all(dir).ok();
    // Keep the animation clock in sync with the screenshot loop. The regular
    // launcher sets this in its display loop; without it, animated selections
    // advance at the stale default 16 ms no matter how long the shot waits.
    let last_frame = std::cell::Cell::new(Instant::now());
    let draw_frame = |app: &mut App| {
        let now = Instant::now();
        ui::set_frame_dt(now.duration_since(last_frame.get()).as_secs_f32());
        last_frame.set(now);
        app.frame();
    };
    let settle = |app: &mut App, ms: u64| {
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            while let Ok(m) = rx.try_recv() {
                match m {
                    Msg::Run(f) => {
                        f(&mut app.cx);
                    }
                    Msg::Redraw => {}
                    _ => {}
                }
            }
            draw_frame(app);
            std::thread::sleep(Duration::from_millis(30));
        }
    };
    let shot = |app: &mut App, name: &str| {
        draw_frame(app);
        let p = format!("{dir}/{name}.png");
        if let Err(e) = gfx::save_png(&app.canvas.pm, &p) {
            log(&format!("{p}: {e}"));
        }
        log(&format!("Screenshot {p}"));
    };
    if std::env::var("SHOT_SAMPLE").is_ok() {
        // offline development: sample weather + news so every tile has content
        let hours = (0..25)
            .map(|i| weather::Hour {
                hour: (8 + i) % 24,
                temp: 9.0 + (i as f32 * 0.5).sin() * 4.0,
                code: [0, 2, 3, 61, 80][i as usize % 5],
                rain: (i * 7 % 60) as u32,
                day: (8 + i) % 24 < 17,
            })
            .collect();
        let days = (0..8)
            .map(|i| weather::Day {
                weekday: i % 7,
                max: 12.0 + i as f32,
                min: 3.0 + i as f32 * 0.5,
                code: [2, 61, 0, 3, 71, 95, 1, 45][i],
                rain: (i * 13 % 80) as u32,
            })
            .collect();
        app.cx.weather.set(Ok(weather::Weather {
            temp: 11.4,
            feels: 9.2,
            code: 2,
            day: true,
            wind: 14.0,
            humidity: 71.0,
            hours,
            days,
            fetched: Instant::now(),
        }));
        app.cx.news.set(
            (0..8)
                .map(|i| news::NewsItem {
                    title: [
                        "Bundestag beschließt neues Klimagesetz nach langer Debatte",
                        "Wetter: Erster Schnee in den Alpen erwartet",
                        "Bahn kündigt neue Verbindungen an",
                    ][i % 3]
                        .into(),
                    topline: ["Berlin", "Wetter", "Verkehr"][i % 3].into(),
                    date: format!("2026-11-08T0{}:15:00+01:00", i),
                    first_sentence:
                        "Die Abgeordneten stimmten am Abend mit großer Mehrheit für den Entwurf."
                            .into(),
                    ..Default::default()
                })
                .collect(),
        );
        app.cx.net.set((true, "wifi".into(), "Zuhause".into()));
    }
    let wait = std::env::var("SHOT_WAIT_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6000);
    settle(app, wait);
    shot(app, "01-home");
    if std::env::var_os("SHOT_HOME_ONLY").is_some() {
        return;
    }
    if std::env::var_os("SHOT_HOME_EDIT_ONLY").is_some() {
        app.key(Key::LongOk);
        settle(app, 250);
        shot(app, "01-home-arrange-mode");
        app.key(Key::Ok);
        settle(app, 100);
        app.key(Key::Right);
        settle(app, 100);
        shot(app, "02-home-arrange-target");
        return;
    }
    if std::env::var_os("SHOT_LIVETV_ONLY").is_some() {
        let names = [
            ("Das Erste HD", "DE"),
            ("ZDF HD", "DE"),
            ("3sat", "DE"),
            ("arte", "DE"),
            ("RTL Deutschland", "DE"),
            ("ProSieben", "DE"),
            ("ORF 1", "AT"),
            ("SRF zwei", "CH"),
            ("France 24", "FR"),
            ("BBC News", "UK"),
        ];
        let channels = names
            .iter()
            .enumerate()
            .map(|(i, (name, group))| livetv::Channel {
                number: i + 1,
                name: (*name).into(),
                group: (*group).into(),
                logo: String::new(),
                url: format!("https://example.invalid/channel-{}.m3u8", i + 1),
            })
            .collect();
        app.cx
            .channels
            .set(Ok((Arc::new(channels), "Screenshot-Vorschau".into())));
        let live = ui::livetv::LiveTv::new(&mut app.cx);
        app.stack.push(Box::new(live));
        app.key(Key::Ok);
        settle(app, 300);
        shot(app, "01-live-tv-guide");
        app.key(Key::Info);
        settle(app, 100);
        shot(app, "02-live-tv-search");
        return;
    }
    if std::env::var_os("SHOT_HOME_NAV_ONLY").is_some() {
        // the focus frame glides across the tiles: nothing it passes over
        // (headline, status, tile edges) may be damaged afterwards
        shot(app, "01-home-before");
        for (i, key) in [
            Key::Down,
            Key::Left,
            Key::Right,
            Key::Up,
            Key::Down,
            Key::Right,
        ]
        .into_iter()
        .enumerate()
        {
            app.key(key);
            settle(app, 90);
            shot(app, &format!("02-home-nav-{i}-gliding"));
            settle(app, 700);
            shot(app, &format!("02-home-nav-{i}-rest"));
        }
        return;
    }
    if std::env::var_os("SHOT_TOAST_ONLY").is_some() {
        // a notification over the retained Home (and Apps) picture must
        // leave nothing behind once it has faded
        if std::env::var_os("SHOT_TOAST_APPS").is_some() {
            app.stack
                .push(Box::new(ui::apps::Apps::new(&app.cx.cfg.last_app_id)));
            settle(app, 400);
        }
        app.key(Key::VolUp);
        settle(app, 1000);
        shot(app, "01-toast-visible");
        settle(app, 4500);
        shot(app, "02-toast-gone");
        return;
    }
    if std::env::var_os("SHOT_SCROLL_ONLY").is_some() {
        // lists and grids in the middle of a scroll: rows must stay inside
        // their box, below the header
        app.cx.news.set(
            (0..24)
                .map(|i| news::NewsItem {
                    title: format!("Meldung {} – eine längere Überschrift zum Testen", i + 1),
                    topline: ["Berlin", "Wetter", "Verkehr"][i % 3].into(),
                    date: format!("2026-11-08T{:02}:15:00+01:00", i % 24),
                    first_sentence: "Die Abgeordneten stimmten am Abend für den Entwurf.".into(),
                    ..Default::default()
                })
                .collect(),
        );
        app.stack.push(Box::new(ui::news::NewsScreen::new()));
        settle(app, 300);
        for _ in 0..4 {
            app.key(Key::Down);
            settle(app, 200);
        }
        app.key(Key::Down);
        std::thread::sleep(Duration::from_millis(40));
        shot(app, "01-news-mid-scroll");
        settle(app, 500);
        shot(app, "02-news-rest");
        app.key(Key::Home);
        let films = (0..40)
            .map(|i| films::Film {
                key: format!("local:{i}"),
                title: format!("Beispielfilm {}", i + 1),
                info: String::new(),
                fsk: String::new(),
                genre: String::new(),
                cast: String::new(),
                crew: String::new(),
                duration: 5400.0,
                icon: String::new(),
                banner: String::new(),
                origin: films::Origin::Local(format!("/tmp/film{i}.mp4").into()),
                position: 0.0,
            })
            .collect();
        let wall = ui::films::Films::new(&mut app.cx);
        settle(app, 300); // the real (empty) scan finishes first
        app.cx.films.list.set(Arc::new(films::FilmList {
            films,
            errors: Vec::new(),
        }));
        app.stack.push(Box::new(wall));
        settle(app, 300);
        app.key(Key::Down);
        std::thread::sleep(Duration::from_millis(40));
        shot(app, "03-films-mid-scroll");
        settle(app, 500);
        shot(app, "04-films-rest");
        app.key(Key::Down);
        settle(app, 500);
        shot(app, "05-films-third-row");
        return;
    }
    if std::env::var_os("SHOT_WEATHER_ONLY").is_some() {
        app.stack.push(Box::new(ui::weather::WeatherScreen::new()));
        settle(app, 300);
        shot(app, "08-weather");
        return;
    }
    if std::env::var_os("SHOT_SETTINGS_ONLY").is_some() {
        let settings = ui::settings::Settings::new(&app.cx);
        app.stack.push(Box::new(settings));
        settle(app, 300);
        shot(app, "12-settings-overview");
        app.key(Key::Right);
        settle(app, 100);
        shot(app, "13-settings-status-focus");
        return;
    }
    if std::env::var_os("SHOT_UPDATE_ONLY").is_some() {
        let settings = ui::settings::Settings::new(&app.cx);
        app.stack.push(Box::new(settings));
        for _ in 0..9 {
            app.key(Key::Down);
            settle(app, 35);
        }
        settle(app, 200);
        shot(app, "12-settings-software-update");
        return;
    }
    if std::env::var_os("SHOT_HOME_AMBIENT_BENCH").is_some() {
        // Hold one focus change long enough to include the Home ambient update
        // in the measured 60-Hz path (the regular navigation bench moves too
        // quickly for the 280-ms ambient debounce to fire).
        app.key(Key::Right);
        *TIMES.lock().unwrap() = (0.0, 0.0);
        let mut previous = Instant::now();
        let mut total_ms = 0.0f64;
        let mut max_ms = 0.0f64;
        let mut over_budget = 0usize;
        const N: usize = 48;
        const FRAME_MS: f64 = 1000.0 / 60.0;
        for _ in 0..N {
            let frame_start = Instant::now();
            ui::set_frame_dt(frame_start.duration_since(previous).as_secs_f32());
            previous = frame_start;
            let started = Instant::now();
            app.frame();
            let frame_ms = started.elapsed().as_secs_f64() * 1000.0;
            total_ms += frame_ms;
            max_ms = max_ms.max(frame_ms);
            over_budget += usize::from(frame_ms > FRAME_MS);
            let next = frame_start + Duration::from_micros(16_667);
            if let Some(remaining) = next.checked_duration_since(Instant::now()) {
                std::thread::sleep(remaining);
            }
        }
        log(&format!(
            "Home 60-Hz Ambient-Wechsel: {:.1} ms im Schnitt, max {:.1} ms, über 16,7 ms: {over_budget}/{N}",
            total_ms / N as f64,
            max_ms,
        ));
        return;
    }
    if std::env::var_os("SHOT_CONTROL_ONLY").is_some() {
        app.key(Key::LongOk);
        settle(app, 140);
        shot(app, "02-control-center");
        app.key(Key::Down);
        settle(app, 100);
        shot(app, "03-control-down-focus");
        if std::env::var_os("SHOT_CONTROL_CLOSE").is_some() {
            app.key(Key::Back);
            settle(app, 100);
            shot(app, "04-home-after-control");
        }
        return;
    }
    if std::env::var_os("SHOT_CONTROL_ANIM").is_some() {
        app.key(Key::LongOk);
        for frame in 0..9 {
            std::thread::sleep(Duration::from_millis(16));
            shot(app, &format!("control-entry-{frame:02}"));
        }
        app.key(Key::Back);
        settle(app, 80);
        shot(app, "control-after-close");
        return;
    }
    if std::env::var_os("SHOT_CONTROL_BENCH").is_some() {
        let cold = std::env::var_os("SHOT_CONTROL_COLD").is_some();
        if !cold {
            ui::control::Control::prewarm(&app.cx, false);
            let deadline = Instant::now() + Duration::from_secs(1);
            while !ui::control::Control::prewarm_ready(&app.cx, false) && Instant::now() < deadline
            {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        log(&format!(
            "Control-Center Vorwärmung bereit: {}",
            ui::control::Control::prewarm_ready(&app.cx, false)
        ));
        if !cold {
            app.control_blur_cache = app.canvas.blurred_region_image(
                CONTROL_DAMAGE.0,
                CONTROL_DAMAGE.1,
                CONTROL_DAMAGE.2,
                CONTROL_DAMAGE.3,
            );
        }
        app.key(Key::LongOk);
        let first_frame = Instant::now();
        draw_frame(app);
        log(&format!(
            "Control Öffnungsframe: {:.1} ms",
            first_frame.elapsed().as_secs_f64() * 1000.0
        ));
        settle(app, 160);
        shot(app, "02-control-center");
        *TIMES.lock().unwrap() = (0.0, 0.0);
        let mut total_ms = 0.0f64;
        let mut steady_ms = 0.0f64;
        let mut max_ms = 0.0f64;
        let mut full_damage = 0usize;
        let mut dirty_area = 0u64;
        const FRAMES: usize = 18;
        for frame in 0..FRAMES {
            let started = Instant::now();
            draw_frame(app);
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            total_ms += ms;
            if frame > 0 {
                steady_ms += ms;
            }
            max_ms = max_ms.max(ms);
            let damage = app.canvas.damage();
            full_damage += usize::from(damage.full);
            dirty_area += damage.rects[..damage.len]
                .iter()
                .map(|r| r.2 as u64 * r.3 as u64)
                .sum::<u64>();
            std::thread::sleep(Duration::from_millis(8));
        }
        let pixels = app.canvas.pm.width() as u64 * app.canvas.pm.height() as u64;
        let (record_s, raster_s) = *TIMES.lock().unwrap();
        log(&format!(
            "Control: {:.1} ms je Bild, danach {:.1} ms, max {:.1} ms; Aufnahme {:.1} ms, Raster {:.1} ms; Vollbilder {full_damage}/{FRAMES}, mittlere Schadensfläche {:.1}%",
            total_ms / FRAMES as f64,
            steady_ms / (FRAMES - 1) as f64,
            max_ms,
            record_s * 1000.0 / FRAMES as f64,
            raster_s * 1000.0 / FRAMES as f64,
            dirty_area as f64 / (pixels * FRAMES as u64) as f64 * 100.0,
        ));
        return;
    }
    if std::env::var_os("SHOT_APPS_ONLY").is_some() {
        let mut apps = ui::apps::Apps::new(&app.cx.cfg.last_app_id);
        if std::env::var_os("SHOT_APPS_FOCUS_PRIME").is_some() {
            app.cx.now = Instant::now();
            let _ = apps.key(Key::Down, &mut app.cx);
        }
        if std::env::var_os("SHOT_APPS_WHEEL_ONLY").is_some() {
            let _ = apps.key(Key::Down, &mut app.cx);
            let _ = apps.key(Key::Down, &mut app.cx);
            let _ = apps.key(Key::Ok, &mut app.cx);
        }
        app.stack.push(Box::new(apps));
        settle(app, 0);
        shot(app, "02-apps");
        if std::env::var_os("SHOT_APPS_BENCH").is_some() {
            // Exercise real remote navigation across the Apps grid at 60 Hz,
            // including the long move to Settings and its animated focus ring.
            // Let the screen's ambient crossfade and first retained-layer build
            // finish before measuring steady-state navigation frames.
            let warm_ms = std::env::var("SHOT_APPS_PREWARM_MS")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(800);
            settle(app, warm_ms);
            *TIMES.lock().unwrap() = (0.0, 0.0);
            let bench_start = Instant::now();
            let mut previous = Instant::now();
            let mut render_total_ms = 0.0f64;
            let mut render_max_ms = 0.0f64;
            let mut over_budget = 0usize;
            const N: usize = 60;
            const FRAME_MS: f64 = 1000.0 / 60.0;
            let route = [
                Key::Down,
                Key::Down,
                Key::Right,
                Key::Up,
                Key::Up,
                Key::Left,
            ];
            for i in 0..N {
                let frame_start = Instant::now();
                ui::set_frame_dt(frame_start.duration_since(previous).as_secs_f32());
                previous = frame_start;
                if i % 10 == 0 {
                    app.key(route[(i / 10) % route.len()]);
                }
                let started = Instant::now();
                app.frame();
                let frame_ms = started.elapsed().as_secs_f64() * 1000.0;
                render_total_ms += frame_ms;
                render_max_ms = render_max_ms.max(frame_ms);
                over_budget += usize::from(frame_ms > FRAME_MS);
                let next = frame_start + Duration::from_micros(16_667);
                if let Some(remaining) = next.checked_duration_since(Instant::now()) {
                    std::thread::sleep(remaining);
                }
            }
            let wall = bench_start.elapsed().as_secs_f64() * 1000.0;
            let (record, raster) = *TIMES.lock().unwrap();
            log(&format!(
                "Apps 60-Hz Navigation: {:.1} ms Render im Schnitt, max {:.1} ms, über 16,7 ms: {over_budget}/{N}, Takt {:.1} ms/Bild",
                render_total_ms / N as f64,
                render_max_ms,
                wall / N as f64,
            ));
            log(&format!(
                "  davon aufzeichnen {:.1} ms, zeichnen {:.1} ms",
                record * 1000.0 / N as f64,
                raster * 1000.0 / N as f64
            ));
        }
        return;
    }
    if std::env::var_os("SHOT_SETTINGS_ROUTE").is_some() {
        let mut apps = ui::apps::Apps::new(&app.cx.cfg.last_app_id);
        for key in [Key::Down, Key::Down, Key::Right] {
            let nav = apps.key(key, &mut app.cx);
            if !matches!(nav, Nav::None) {
                log("Settings-Routenprüfung: unerwartete Navigation im Apps-Raster");
                return;
            }
        }
        app.stack.push(Box::new(apps));
        settle(app, 180);
        shot(app, "02-apps-settings-focused");
        app.key(Key::Ok);
        let route = app.stack.last().map(|s| s.name()).unwrap_or("empty");
        log(&format!("Settings-Routenprüfung: Apps > OK = {route}"));
        let settings_wait = std::env::var("SHOT_SETTINGS_WAIT_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1200);
        settle(app, settings_wait);
        shot(app, "03-settings");
        if std::env::var_os("SHOT_SETTINGS_SCROLL").is_some() {
            app.key(Key::Right);
            for _ in 0..12 {
                app.key(Key::Down);
                settle(app, 100);
            }
            settle(app, 350);
            shot(app, "04-settings-wifi-scroll");
        }
        return;
    }
    if std::env::var("SHOT_ANIM").is_ok() {
        // frames in the middle of an animation (focus glide + screen slide)
        app.key(Key::Right);
        for k in 0..4 {
            std::thread::sleep(Duration::from_millis(45));
            shot(app, &format!("anim-focus-{k}"));
        }
        settle(app, 500);
        app.key(Key::Ok);
        for k in 0..3 {
            std::thread::sleep(Duration::from_millis(60));
            shot(app, &format!("anim-open-{k}"));
        }
        app.key(Key::Home);
        settle(app, 500);
    }
    if std::env::var("SHOT_BENCH").is_ok() {
        // Exercise Home focus animation at a 60 Hz cadence and measure actual
        // software render time, rather than flooding it with zero-gap inputs.
        *TIMES.lock().unwrap() = (0.0, 0.0);
        let bench_start = Instant::now();
        let mut previous = Instant::now();
        let mut render_total_ms = 0.0f64;
        let mut render_max_ms = 0.0f64;
        let mut over_budget = 0usize;
        const N: usize = 60;
        const FRAME_MS: f64 = 1000.0 / 60.0;
        for i in 0..N {
            if std::env::var_os("SHOT_BENCH_REFRESH").is_some() {
                // Stress one real tile dependency, as if a fresh headlines
                // response arrived while the launcher is on the Home screen.
                let mut items = app.cx.news.get().unwrap_or_default();
                if let Some(first) = items.first_mut() {
                    first.title.push('.');
                    app.cx.news.set(items);
                }
            }
            let frame_start = Instant::now();
            ui::set_frame_dt(frame_start.duration_since(previous).as_secs_f32());
            previous = frame_start;
            let started = Instant::now();
            app.key(if i % 10 < 5 { Key::Right } else { Key::Left });
            app.frame();
            let frame_ms = started.elapsed().as_secs_f64() * 1000.0;
            render_total_ms += frame_ms;
            render_max_ms = render_max_ms.max(frame_ms);
            over_budget += usize::from(frame_ms > FRAME_MS);
            let next = frame_start + Duration::from_micros(16_667);
            if let Some(remaining) = next.checked_duration_since(Instant::now()) {
                std::thread::sleep(remaining);
            }
        }
        let wall = bench_start.elapsed().as_secs_f64() * 1000.0;
        let (rec, ras) = *TIMES.lock().unwrap();
        log(&format!(
            "Home 60-Hz Navigation: {:.1} ms Render im Schnitt, max {:.1} ms, über 16,7 ms: {over_budget}/{N}, Takt {:.1} ms/Bild",
            render_total_ms / N as f64,
            render_max_ms,
            wall / N as f64,
        ));
        log(&format!(
            "  davon aufzeichnen {:.1} ms, zeichnen {:.1} ms",
            rec * 1000.0 / N as f64,
            ras * 1000.0 / N as f64
        ));
        if std::env::var_os("SHOT_BENCH_ONLY").is_some() {
            return;
        }
    }
    let script: Vec<(&str, Vec<Key>)> = vec![
        ("02-youtube", vec![Key::Ok]),
        (
            "03-youtube-keyboard",
            vec![
                Key::Up,
                Key::Up,
                Key::Ok,
                Key::Char('k'),
                Key::Char('a'),
                Key::Char('t'),
                Key::Char('z'),
            ],
        ),
        (
            "04-films",
            vec![
                Key::Back,
                Key::Back,
                Key::Home,
                Key::Right,
                Key::Right,
                Key::Ok,
            ],
        ),
        ("05-film-page", vec![Key::Ok]),
        ("06-livetv", vec![Key::Home, Key::Left, Key::Ok]),
        ("07-livetv-guide", vec![Key::Ok, Key::Down, Key::Down]),
        ("07b-livetv-groups", vec![Key::Left, Key::Down, Key::Down]),
        ("08-weather", vec![Key::Home, Key::Down, Key::Ok]),
        ("09-news", vec![Key::Home, Key::Down, Key::Right, Key::Ok]),
        (
            "10-apps",
            vec![Key::Home, Key::Down, Key::Right, Key::Right, Key::Ok],
        ),
        ("11-settings", vec![Key::Home, Key::Menu]),
        ("11a-settings-focused", vec![Key::Right]),
        ("11-settings-server", vec![Key::Back, Key::Down, Key::Down]),
        ("11b-settings-livetv", vec![Key::Down]),
        ("11c-settings-picture", vec![Key::Down]),
        ("14-screensaver-is-next", vec![Key::Home]),
        ("13-home-again", vec![Key::Home]),
    ];
    for (name, keys) in script {
        for k in keys {
            app.key(k);
            settle(app, 60);
        }
        settle(app, wait / 3);
        shot(app, name);
    }
}
