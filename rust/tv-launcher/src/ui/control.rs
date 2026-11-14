use super::*;
use crate::app::{Nav, Screen};
use crate::gfx::{Canvas, Image, Rgba};
use std::sync::{Mutex, OnceLock};

type PanelKey = (i32, i32, bool, bool, bool, bool, String);

enum PanelCacheState {
    Idle,
    Building(PanelKey),
    Ready(PanelKey, Image),
}

static PANEL_CACHE: OnceLock<Mutex<PanelCacheState>> = OnceLock::new();

fn panel_cache() -> &'static Mutex<PanelCacheState> {
    PANEL_CACHE.get_or_init(|| Mutex::new(PanelCacheState::Idle))
}

fn current_panel_key(cx: &Ctx, over_video: bool) -> PanelKey {
    let (hh, mm, _) = crate::app::local_hms();
    let connected = cx.net.get().map(|v| v.0).unwrap_or(false);
    let net_label = cx
        .net
        .get()
        .map(|(ok, kind, ssid)| {
            if !ok {
                "Nicht verbunden".to_string()
            } else if kind == "wifi" {
                ssid
            } else {
                "LAN verbunden".to_string()
            }
        })
        .unwrap_or_else(|| "Netzwerk prüfen".into());
    (
        hh,
        mm,
        connected,
        cx.cfg.remote_enabled,
        cx.cfg.do_not_disturb,
        over_video,
        net_label,
    )
}

/// The Control Center is a real in-frame OSD. It never starts another window;
/// while a browser or player is active, KMS keeps the picture on its own plane.
pub struct Control {
    focus: usize,
    over_video: bool,
    opened_at: std::time::Instant,
    panel_canvas: Canvas,
    panel_image: Option<Image>,
    panel_key: Option<PanelKey>,
}

const OPEN_ANIM: f32 = 0.14;
// The panel is rasterized in its own local coordinates and displayed at the
// top right, including when it overlays the browser's video plane.
const PANEL_SOURCE: (f32, f32) = (820.0, 96.0);
const PANEL_BOUNDS: (f32, f32, f32, f32) = (878.0, 16.0, 390.0, 470.0);

const FOCUS_POS: [(f32, f32); 8] = [
    (901.0, 292.0),
    (1071.0, 264.0),
    (1071.0, 319.0),
    (1015.0, 381.0),
    (890.0, 456.0),
    (973.0, 456.0),
    (1056.0, 456.0),
    (1139.0, 456.0),
];

impl Control {
    /// Prepare the static panel while Home is idle so opening the OSD does not
    /// spend a frame rasterizing it. Only one small, low-priority worker runs.
    pub fn prewarm(cx: &Ctx, over_video: bool) {
        let key = current_panel_key(cx, over_video);

        {
            let mut state = panel_cache().lock().unwrap_or_else(|e| e.into_inner());
            match &*state {
                PanelCacheState::Building(existing) | PanelCacheState::Ready(existing, _)
                    if existing == &key =>
                {
                    return
                }
                _ => *state = PanelCacheState::Building(key.clone()),
            }
        }

        let failed_key = key.clone();
        let result = std::thread::Builder::new()
            .name("control-panel-prewarm".into())
            .spawn(move || {
                let mut canvas = Canvas::new_scaled_with_threads(
                    PANEL_BOUNDS.2 as u32,
                    PANEL_BOUNDS.3 as u32,
                    crate::gfx::RENDER_SCALE,
                    2,
                );
                canvas.clear(Rgba(0, 0, 0, 0));
                canvas.offset = (-PANEL_SOURCE.0, -PANEL_SOURCE.1);
                Control::draw_panel(
                    usize::MAX,
                    key.5,
                    &mut canvas,
                    key.0,
                    key.1,
                    key.2,
                    key.3,
                    key.4,
                    &key.6,
                );
                canvas.finish();
                let image = canvas.snapshot();
                let mut state = panel_cache().lock().unwrap_or_else(|e| e.into_inner());
                if matches!(&*state, PanelCacheState::Building(existing) if existing == &key) {
                    *state = PanelCacheState::Ready(key, image);
                }
            });
        if result.is_err() {
            let mut state = panel_cache().lock().unwrap_or_else(|e| e.into_inner());
            if matches!(&*state, PanelCacheState::Building(existing) if existing == &failed_key) {
                *state = PanelCacheState::Idle;
            }
        }
    }

    pub fn prewarm_ready(cx: &Ctx, over_video: bool) -> bool {
        let key = current_panel_key(cx, over_video);
        let state = panel_cache().lock().unwrap_or_else(|e| e.into_inner());
        matches!(&*state, PanelCacheState::Ready(cached_key, _) if cached_key == &key)
    }

    pub fn new(over_video: bool) -> Self {
        Self {
            focus: 1,
            over_video,
            opened_at: std::time::Instant::now(),
            panel_canvas: Canvas::new_scaled_with_threads(
                PANEL_BOUNDS.2 as u32,
                PANEL_BOUNDS.3 as u32,
                crate::gfx::RENDER_SCALE,
                4,
            ),
            panel_image: None,
            panel_key: None,
        }
    }

    fn move_focus(&mut self, key: Key) {
        let (x, y) = FOCUS_POS[self.focus];
        let mut next = None;
        let mut best = f32::INFINITY;
        for (i, (nx, ny)) in FOCUS_POS.iter().copied().enumerate() {
            if i == self.focus {
                continue;
            }
            let dx = nx - x;
            let dy = ny - y;
            let (forward, lateral) = match key {
                Key::Left => (-dx, dy),
                Key::Right => (dx, dy),
                Key::Up => (-dy, dx),
                Key::Down => (dy, dx),
                _ => return,
            };
            if forward <= 0.5 {
                continue;
            }
            // Favor items in the requested direction, then the closest item
            // beside that path. This keeps Up/Down from turning into a Left.
            let score = forward * forward + lateral * lateral * 2.25;
            if score < best {
                best = score;
                next = Some(i);
            }
        }
        if let Some(i) = next {
            self.focus = i;
        }
    }

    fn activate(&mut self, cx: &mut Ctx) -> Nav {
        match self.focus {
            0 => {
                #[cfg(target_os = "linux")]
                crate::platform::cec::tv_off();
                Nav::Home
            }
            1 | 7 => Nav::Replace(Box::new(settings::Settings::new(cx))),
            2 => {
                cx.cfg.do_not_disturb = !cx.cfg.do_not_disturb;
                if cx.cfg.do_not_disturb {
                    cx.toast = None;
                } else {
                    cx.toast("Nicht stören aus");
                }
                cx.save();
                Nav::None
            }
            3 => Nav::Pop,
            4 => Nav::Replace(Box::new(livetv::LiveTv::new(cx))),
            5 => Nav::Replace(Box::new(weather::WeatherScreen::new())),
            6 => {
                cx.cfg.remote_enabled = !cx.cfg.remote_enabled;
                crate::remote::set_enabled(cx.cfg.remote_enabled);
                cx.save();
                cx.toast(if cx.cfg.remote_enabled {
                    "Handy-Fernbedienung an"
                } else {
                    "Handy-Fernbedienung aus"
                });
                Nav::None
            }
            _ => Nav::Pop,
        }
    }

    fn tile(
        c: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        idx: usize,
        focus: usize,
        active: bool,
    ) {
        let selected = idx == focus;
        c.rect(
            x,
            y,
            w,
            h,
            13.0,
            if selected {
                Rgba(58, 103, 203, 248)
            } else if active {
                Rgba(47, 72, 105, 242)
            } else {
                Rgba(40, 46, 61, 245)
            },
        );
        c.stroke_rect(
            x,
            y,
            w,
            h,
            13.0,
            if selected { 2.0 } else { 1.0 },
            if selected {
                Rgba(212, 229, 255, 255)
            } else {
                Rgba(255, 255, 255, 36)
            },
        );
    }

    fn power_icon(c: &mut Canvas, x: f32, y: f32, col: Rgba) {
        let ring: Vec<(f32, f32)> = (0..=24)
            .map(|i| {
                let a = 5.0 * std::f32::consts::PI / 4.0
                    - i as f32 * 3.0 * std::f32::consts::PI / 2.0 / 24.0;
                (x + a.cos() * 13.0, y + a.sin() * 13.0)
            })
            .collect();
        c.line(&ring, 2.8, col);
        c.line(&[(x, y - 18.0), (x, y + 1.0)], 3.0, col);
    }

    fn draw_focus_overlay(c: &mut Canvas, focus: usize) {
        let (x, y, w, h) = match focus {
            0 => (852.0, 238.0, 100.0, 106.0),
            1 => (966.0, 238.0, 212.0, 51.0),
            2 => (966.0, 293.0, 212.0, 51.0),
            3 => (852.0, 356.0, 326.0, 52.0),
            4..=7 => (852.0 + (focus - 4) as f32 * 83.0, 420.0, 76.0, 76.0),
            _ => return,
        };
        c.rect(x, y, w, h, 13.0, Rgba(73, 121, 230, 40));
        c.stroke_rect(x, y, w, h, 13.0, 2.0, Rgba(216, 231, 255, 255));
    }

    fn draw_panel(
        focus: usize,
        over_video: bool,
        c: &mut Canvas,
        hh: i32,
        mm: i32,
        connected: bool,
        remote_enabled: bool,
        dnd: bool,
        net_label: &str,
    ) {
        let fg = Rgba(248, 250, 255, 255);
        let muted = Rgba(177, 188, 211, 255);
        c.rect(841.0, 117.0, 358.0, 446.0, 23.0, Rgba(0, 0, 0, 90));
        c.rect(836.0, 111.0, 358.0, 446.0, 22.0, Rgba(24, 29, 43, 250));
        c.stroke_rect(
            836.0,
            111.0,
            358.0,
            446.0,
            22.0,
            1.0,
            Rgba(169, 196, 249, 81),
        );
        c.rect(852.0, 135.0, 4.0, 32.0, 2.0, Rgba(100, 160, 255, 255));
        c.text(
            "Schnellmenü",
            870.0,
            130.0,
            22,
            Weight::Bold,
            fg,
            Align::Left,
        );
        c.text(
            if over_video {
                "WIEDERGABE"
            } else {
                "STARTSEITE"
            },
            1176.0,
            140.0,
            10,
            Weight::Semi,
            Rgba(128, 187, 255, 255),
            Align::Right,
        );
        c.text(
            "Aktionen und Verbindungen",
            870.0,
            158.0,
            12,
            Weight::Regular,
            muted,
            Align::Left,
        );
        c.rect(852.0, 180.0, 326.0, 45.0, 12.0, Rgba(38, 49, 70, 255));
        c.text(
            &format!("{hh:02}:{mm:02}"),
            868.0,
            190.0,
            19,
            Weight::Bold,
            fg,
            Align::Left,
        );
        c.line(
            &[(940.0, 188.0), (940.0, 217.0)],
            1.0,
            Rgba(255, 255, 255, 40),
        );
        c.circle(
            955.0,
            203.0,
            4.0,
            if connected {
                Rgba(91, 216, 166, 255)
            } else {
                Rgba(246, 115, 115, 255)
            },
        );
        c.text_fit(net_label, 968.0, 194.0, 13, Weight::Semi, fg, 125.0);
        c.text(
            if remote_enabled {
                "Handy an"
            } else {
                "Handy aus"
            },
            1163.0,
            196.0,
            11,
            Weight::Regular,
            muted,
            Align::Right,
        );

        Self::tile(c, 852.0, 238.0, 100.0, 106.0, 0, focus, false);
        Self::power_icon(c, 902.0, 275.0, fg);
        c.text(
            "Ausschalten",
            902.0,
            314.0,
            11,
            Weight::Semi,
            fg,
            Align::Center,
        );
        Self::tile(c, 966.0, 238.0, 212.0, 51.0, 1, focus, false);
        draw_wifi(c, 989.0, 264.0, Rgba(119, 189, 255, 255));
        c.text("Netzwerk", 1009.0, 244.0, 13, Weight::Semi, fg, Align::Left);
        c.text_fit(net_label, 1009.0, 263.0, 11, Weight::Regular, muted, 156.0);
        Self::tile(c, 966.0, 293.0, 212.0, 51.0, 2, focus, dnd);
        c.circle(989.0, 318.0, 8.0, Rgba(151, 185, 252, 255));
        c.circle(993.0, 314.0, 8.0, Rgba(40, 46, 61, 255));
        c.text(
            "Nicht stören",
            1009.0,
            299.0,
            13,
            Weight::Semi,
            fg,
            Align::Left,
        );
        c.text(
            if dnd { "Ein" } else { "Aus" },
            1009.0,
            319.0,
            11,
            Weight::Regular,
            muted,
            Align::Left,
        );
        Self::tile(c, 852.0, 356.0, 326.0, 52.0, 3, focus, false);
        super::icon_play(c, 877.0, 381.0, 20.0, Rgba(126, 194, 255, 255));
        c.text(
            if over_video {
                "Wiedergabe fortsetzen"
            } else {
                "Zur Startseite"
            },
            902.0,
            365.0,
            15,
            Weight::Semi,
            fg,
            Align::Left,
        );
        c.text(
            if over_video {
                "Menü schließen"
            } else {
                "Launcher anzeigen"
            },
            902.0,
            385.0,
            11,
            Weight::Regular,
            muted,
            Align::Left,
        );
        let labels = ["Live TV", "Wetter", "Handy", "Einstell."];
        for idx in 0..4 {
            let x = 852.0 + idx as f32 * 83.0;
            Self::tile(
                c,
                x,
                420.0,
                76.0,
                76.0,
                idx + 4,
                focus,
                idx == 2 && remote_enabled,
            );
            let cx = x + 38.0;
            let icon = Rgba(233, 240, 255, 255);
            match idx {
                0 => {
                    c.stroke_rect(cx - 11.0, 439.0, 22.0, 16.0, 3.0, 2.0, icon);
                    c.line(&[(cx - 5.0, 458.0), (cx + 5.0, 458.0)], 1.8, icon);
                }
                1 => {
                    c.circle(cx, 448.0, 9.0, Rgba(255, 201, 77, 255));
                }
                2 => {
                    c.stroke_rect(cx - 8.0, 435.0, 16.0, 26.0, 4.0, 2.0, icon);
                }
                _ => super::icon_gear(c, cx, 448.0, 23.0, icon),
            }
            c.text(
                labels[idx],
                cx,
                473.0,
                11,
                Weight::Semi,
                icon,
                Align::Center,
            );
        }
        c.text(
            "PFEILE  Navigieren    ·    OK  Wählen    ·    ZURÜCK  Schließen",
            852.0,
            522.0,
            10,
            Weight::Regular,
            muted,
            Align::Left,
        );
    }
}

fn draw_wifi(c: &mut Canvas, x: f32, y: f32, color: Rgba) {
    c.line(
        &[
            (x - 8.0, y - 3.0),
            (x - 5.0, y - 6.0),
            (x, y - 8.0),
            (x + 5.0, y - 6.0),
            (x + 8.0, y - 3.0),
        ],
        1.7,
        color,
    );
    c.line(
        &[
            (x - 5.5, y + 1.0),
            (x - 3.0, y - 1.0),
            (x, y - 2.0),
            (x + 3.0, y - 1.0),
            (x + 5.5, y + 1.0),
        ],
        1.7,
        color,
    );
    c.line(
        &[(x - 2.5, y + 4.0), (x, y + 2.5), (x + 2.5, y + 4.0)],
        1.7,
        color,
    );
    c.circle(x, y + 7.0, 1.3, color);
}

impl Screen for Control {
    fn animating(&self, _: &Ctx) -> bool {
        self.opened_at.elapsed().as_secs_f32() < OPEN_ANIM
    }

    fn key(&mut self, key: Key, cx: &mut Ctx) -> Nav {
        match key {
            Key::Back | Key::Menu | Key::LongOk => Nav::Pop,
            Key::Home | Key::LongBack => Nav::Home,
            Key::Left | Key::Right | Key::Up | Key::Down => {
                self.move_focus(key);
                Nav::None
            }
            Key::Ok => self.activate(cx),
            _ => Nav::None,
        }
    }

    fn draw(&mut self, c: &mut Canvas, cx: &mut Ctx) {
        let (hh, mm, _) = crate::app::local_hms();
        let connected = cx.net.get().map(|v| v.0).unwrap_or(false);
        let remote_enabled = cx.cfg.remote_enabled;
        let dnd = cx.cfg.do_not_disturb;
        let net_label = cx
            .net
            .get()
            .map(|(ok, kind, ssid)| {
                if !ok {
                    "Nicht verbunden".to_string()
                } else if kind == "wifi" {
                    ssid
                } else {
                    "LAN verbunden".to_string()
                }
            })
            .unwrap_or_else(|| "Netzwerk prüfen".into());
        let key = current_panel_key(cx, self.over_video);
        if self.panel_key.as_ref() != Some(&key) {
            let prepared = {
                let state = panel_cache().lock().unwrap_or_else(|e| e.into_inner());
                match &*state {
                    PanelCacheState::Ready(cached_key, image) if cached_key == &key => {
                        Some(image.clone())
                    }
                    _ => None,
                }
            };
            if let Some(image) = prepared {
                self.panel_image = Some(image);
            } else {
                self.panel_canvas.clear(Rgba(0, 0, 0, 0));
                self.panel_canvas.offset = (-PANEL_SOURCE.0, -PANEL_SOURCE.1);
                self.panel_canvas.opacity = 1.0;
                Self::draw_panel(
                    usize::MAX,
                    self.over_video,
                    &mut self.panel_canvas,
                    hh,
                    mm,
                    connected,
                    remote_enabled,
                    dnd,
                    &net_label,
                );
                self.panel_canvas.finish();
                self.panel_image = Some(self.panel_canvas.snapshot());
            }
            self.panel_key = Some(key);
        }

        let progress = (self.opened_at.elapsed().as_secs_f32() / OPEN_ANIM).clamp(0.0, 1.0);
        let entrance = crate::gfx::ease(progress);
        let previous_opacity = c.opacity;
        let previous_offset = c.offset;
        c.opacity *= entrance;
        c.offset = (
            previous_offset.0 + (1.0 - entrance) * 18.0,
            previous_offset.1,
        );
        if let Some(image) = self.panel_image.as_ref() {
            c.image_exact(image, PANEL_BOUNDS.0, PANEL_BOUNDS.1);
        }
        c.offset.0 += PANEL_BOUNDS.0 - PANEL_SOURCE.0;
        c.offset.1 += PANEL_BOUNDS.1 - PANEL_SOURCE.1;
        Self::draw_focus_overlay(c, self.focus);
        c.opacity = previous_opacity;
        c.offset = previous_offset;
    }

    fn video(&self) -> bool {
        self.over_video
    }
    fn overlay(&self) -> bool {
        true
    }
    fn name(&self) -> &'static str {
        "control"
    }
}
