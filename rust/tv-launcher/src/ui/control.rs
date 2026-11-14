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
const PANEL_BOUNDS: (f32, f32, f32, f32) = (930.0, 100.0, 272.0, 296.0);

const FOCUS_POS: [(f32, f32); 8] = [
    (49.0, 48.0),   // power, spans the two upper rows
    (155.0, 26.5),  // Wi-Fi
    (155.0, 69.5),  // Do Not Disturb
    (49.0, 114.0),  // resume current screen
    (36.0, 165.5),  // Live TV
    (86.0, 165.5),  // weather
    (136.0, 165.5), // phone remote
    (186.0, 165.5), // settings
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
                canvas.offset = (-PANEL_BOUNDS.0, -PANEL_BOUNDS.1);
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
        let fill = if selected {
            Rgba(87, 91, 101, if active { 246 } else { 238 })
        } else {
            Rgba(48, 50, 56, if active { 226 } else { 212 })
        };
        c.rect(x, y, w, h, 11.0, fill);
        if selected {
            c.stroke_rect(x, y, w, h, 11.0, 1.5, Rgba(255, 255, 255, 226));
        } else {
            c.stroke_rect(
                x + 0.5,
                y + 0.5,
                w - 1.0,
                h - 1.0,
                10.5,
                1.0,
                Rgba(255, 255, 255, 32),
            );
        }
    }

    fn power_icon(c: &mut Canvas, x: f32, y: f32, color: Rgba) {
        let ring: Vec<(f32, f32)> = (0..=24)
            .map(|i| {
                let a = 5.0 * std::f32::consts::PI / 4.0
                    - i as f32 * 3.0 * std::f32::consts::PI / 2.0 / 24.0;
                (x + a.cos() * 13.0, y + a.sin() * 13.0)
            })
            .collect();
        c.line(&ring, 2.8, color);
        c.line(&[(x, y - 18.0), (x, y + 1.0)], 3.0, color);
    }

    fn draw_footer(c: &mut Canvas, x: f32, y: f32, idx: usize, focus: usize, remote_enabled: bool) {
        let labels = ["Live TV", "Wetter", "Handy", "Einst."];
        let bx = x + idx as f32 * 50.0;
        Self::tile(
            c,
            bx,
            y,
            46.0,
            47.0,
            4 + idx,
            focus,
            idx == 2 && remote_enabled,
        );
        let cx = bx + 23.0;
        let cy = y + 16.0;
        let icon = Rgba(238, 239, 242, 255);
        match idx {
            0 => {
                c.stroke_rect(cx - 8.5, cy - 6.0, 17.0, 12.0, 2.5, 1.5, icon);
                c.line(&[(cx - 4.0, cy + 8.0), (cx + 4.0, cy + 8.0)], 1.5, icon);
                c.line(&[(cx, cy + 6.0), (cx, cy + 8.0)], 1.5, icon);
            }
            1 => {
                c.circle(cx, cy, 5.0, Rgba(255, 207, 91, 255));
                for (dx, dy) in [
                    (0.0, -9.0),
                    (0.0, 9.0),
                    (-9.0, 0.0),
                    (9.0, 0.0),
                    (-6.4, -6.4),
                    (6.4, 6.4),
                    (6.4, -6.4),
                    (-6.4, 6.4),
                ] {
                    c.line(
                        &[(cx + dx * 0.72, cy + dy * 0.72), (cx + dx, cy + dy)],
                        1.2,
                        Rgba(255, 207, 91, 255),
                    );
                }
            }
            2 => {
                c.stroke_rect(cx - 6.0, cy - 10.0, 12.0, 20.0, 3.0, 1.4, icon);
                c.line(&[(cx - 2.2, cy + 6.5), (cx + 2.2, cy + 6.5)], 1.0, icon);
            }
            _ => super::icon_gear(c, cx, cy, 17.0, icon),
        }
        c.text(
            labels[idx],
            cx,
            y + 31.0,
            8,
            Weight::Semi,
            Rgba(238, 239, 242, 255),
            Align::Center,
        );
    }

    fn draw_focus_overlay(c: &mut Canvas, focus: usize, dnd: bool, remote_enabled: bool) {
        let (x, y, w, h, active) = match focus {
            0 => (965.0, 182.0, 80.0, 80.0, false),
            1 => (1052.0, 182.0, 115.0, 37.0, false),
            2 => (1052.0, 225.0, 115.0, 37.0, dnd),
            3 => (965.0, 270.0, 80.0, 36.0, false),
            4..=7 => (
                965.0 + (focus - 4) as f32 * 50.0,
                316.0,
                46.0,
                47.0,
                focus == 6 && remote_enabled,
            ),
            _ => return,
        };
        // The cached panel contains the unselected tile. A translucent tint
        // recreates the selected fill while only rerasterizing this one tile.
        let tint = if active {
            Rgba(98, 107, 119, 176)
        } else {
            Rgba(108, 113, 126, 154)
        };
        c.rect(x, y, w, h, 11.0, tint);
        c.stroke_rect(x, y, w, h, 11.0, 1.5, Rgba(255, 255, 255, 226));
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
        let (x, y, w, h) = (956.0, 174.0, 220.0, 214.0);
        // Roomier status capsule above the panel: time, actual network name,
        // and phone-remote state stay readable from the couch.
        let (sx, sy, sw, sh) = (940.0, 108.0, 252.0, 48.0);
        c.rect(sx + 2.0, sy + 3.0, sw, sh, 17.0, Rgba(0, 0, 0, 56));
        c.rect(sx, sy, sw, sh, 17.0, Rgba(35, 36, 42, 238));
        c.stroke_rect(sx, sy, sw, sh, 17.0, 1.0, Rgba(255, 255, 255, 64));
        c.text(
            &format!("{hh:02}:{mm:02}"),
            sx + 42.0,
            sy + 15.0,
            15,
            Weight::Semi,
            Rgba(248, 248, 250, 255),
            Align::Center,
        );
        c.line(
            &[(sx + 84.0, sy + 10.0), (sx + 84.0, sy + 38.0)],
            1.0,
            Rgba(255, 255, 255, 36),
        );
        c.circle(
            sx + 100.0,
            sy + 24.0,
            3.0,
            if connected {
                Rgba(102, 213, 158, 255)
            } else {
                Rgba(244, 102, 102, 255)
            },
        );
        draw_wifi(
            c,
            sx + 117.0,
            sy + 23.0,
            if connected {
                Rgba(89, 184, 245, 255)
            } else {
                Rgba(218, 221, 228, 255)
            },
        );
        let short_net = c.ellipsize(net_label, 9, Weight::Regular, 50.0);
        c.text(
            &short_net,
            sx + 137.0,
            sy + 19.0,
            9,
            Weight::Semi,
            Rgba(232, 233, 237, 255),
            Align::Left,
        );
        c.line(
            &[(sx + 194.0, sy + 10.0), (sx + 194.0, sy + 38.0)],
            1.0,
            Rgba(255, 255, 255, 36),
        );
        let phone_color = if remote_enabled {
            Rgba(106, 191, 255, 255)
        } else {
            Rgba(232, 233, 237, 190)
        };
        c.stroke_rect(sx + 205.0, sy + 14.0, 12.0, 20.0, 3.0, 1.4, phone_color);
        c.circle(sx + 211.0, sy + 31.0, 0.9, phone_color);
        c.text(
            if remote_enabled { "AN" } else { "AUS" },
            sx + 233.0,
            sy + 19.0,
            9,
            Weight::Semi,
            phone_color,
            Align::Center,
        );

        c.rect(x + 4.0, y + 6.0, w, h, 20.0, Rgba(0, 0, 0, 78));
        c.rect(x, y, w, h, 19.0, Rgba(48, 49, 55, 220));
        c.stroke_rect(x, y, w, h, 19.0, 1.0, Rgba(255, 255, 255, 62));
        c.line(
            &[(x + 22.0, y + 1.5), (x + w - 22.0, y + 1.5)],
            1.0,
            Rgba(255, 255, 255, 35),
        );

        let left = x + 9.0;
        let right = x + 96.0;
        let fg = Rgba(247, 247, 249, 255);
        let muted = Rgba(211, 213, 219, 255);
        Self::tile(c, left, y + 8.0, 80.0, 80.0, 0, focus, false);
        Self::power_icon(c, left + 40.0, y + 37.0, fg);
        c.text(
            "Power Off",
            left + 40.0,
            y + 62.0,
            10,
            Weight::Semi,
            fg,
            Align::Center,
        );

        Self::tile(c, right, y + 8.0, 115.0, 37.0, 1, focus, false);
        draw_wifi(c, right + 16.0, y + 22.0, Rgba(89, 184, 245, 255));
        c.text(
            if connected { "Wi-Fi" } else { "Netzwerk" },
            right + 31.0,
            y + 11.0,
            9,
            Weight::Semi,
            fg,
            Align::Left,
        );
        let short_net = c.ellipsize(net_label, 8, Weight::Regular, 77.0);
        c.text(
            &short_net,
            right + 31.0,
            y + 24.0,
            8,
            Weight::Regular,
            muted,
            Align::Left,
        );

        Self::tile(c, right, y + 51.0, 115.0, 37.0, 2, focus, dnd);
        c.circle(
            right + 16.0,
            y + 69.0,
            7.0,
            if dnd {
                Rgba(107, 180, 244, 255)
            } else {
                Rgba(206, 209, 216, 255)
            },
        );
        c.circle(right + 19.0, y + 66.0, 6.5, Rgba(48, 49, 55, 255));
        c.text(
            "Nicht stören",
            right + 31.0,
            y + 54.0,
            9,
            Weight::Semi,
            fg,
            Align::Left,
        );
        c.text(
            if dnd { "Ein" } else { "Aus" },
            right + 31.0,
            y + 67.0,
            8,
            Weight::Regular,
            muted,
            Align::Left,
        );

        Self::tile(c, left, y + 96.0, 80.0, 36.0, 3, focus, false);
        super::icon_play(c, left + 16.0, y + 113.0, 12.0, Rgba(228, 230, 235, 255));
        c.text(
            if over_video {
                "Fortsetzen"
            } else {
                "Startseite"
            },
            left + 31.0,
            y + 109.0,
            8,
            Weight::Semi,
            fg,
            Align::Left,
        );
        c.rect(right, y + 96.0, 115.0, 36.0, 10.0, Rgba(43, 44, 50, 214));
        c.circle(
            right + 14.0,
            y + 114.0,
            3.0,
            if over_video {
                Rgba(244, 77, 80, 255)
            } else {
                Rgba(102, 213, 158, 255)
            },
        );
        c.text(
            if over_video {
                "YouTube läuft"
            } else {
                "Launcher"
            },
            right + 26.0,
            y + 108.0,
            9,
            Weight::Semi,
            fg,
            Align::Left,
        );
        for idx in 0..4 {
            Self::draw_footer(c, x + 9.0, y + 142.0, idx, focus, remote_enabled);
        }
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
                self.panel_canvas.offset = (-PANEL_BOUNDS.0, -PANEL_BOUNDS.1);
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
        Self::draw_focus_overlay(c, self.focus, dnd, remote_enabled);
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
