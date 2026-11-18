use super::*;
use crate::app::{Nav, Screen};
use crate::gfx::{Canvas, Image, Rgba};
use crate::glass_fx::{self, Glass, Rect};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

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

/// The fast menu: a liquid-glass panel that opens out of the clock on Home.
/// It never starts another window; over a film KMS keeps the picture on its
/// own plane and the glass is tinted instead of blurred.
///
/// Everything sits on Home's grid: the panel takes the clock's column
/// (992..1216, the same as the tile below the clock), starts one Home gap
/// (24) under the clock and ends with Home's bottom row (642). Inside: 12
/// padding, 8 gaps, two columns of 96 (2 * 96 + 8 = 224 - 2 * 12), and
/// corners that run parallel (panel 22 like Home's cards, tiles 22 - 12).
///
/// Rust draws the text and icons once into a transparent layer; C++
/// (`cpp/glass_fx.cpp`) draws the glass and the focus highlight. A key press
/// redraws only the rectangle the highlight glides through.
pub struct Control {
    focus: usize,
    over_video: bool,
    opened_at: Instant,
    panel_canvas: Option<Canvas>,
    panel_key: Option<PanelKey>,
    glass: Option<GlassFx>,
    /// The minute of the Home picture behind the glass (the clock is in it).
    backdrop_minute: Option<i32>,
    /// The highlight glides from here to the focused tile.
    glide_from: Rect,
    glide_at: Instant,
    pressed_at: Option<Instant>,
    /// Drawn last frame: the highlight and whether the menu was still opening.
    last_focus: Option<Rect>,
    last_open: f32,
    full_redraw: bool,
}

struct GlassFx {
    glass: Glass,
    tiles_key: (bool, bool),
}

/// Glass built while Home stood still, and a fingerprint of the Home
/// pixels it was built from.
struct Prepared {
    minute: i32,
    size: (u32, u32),
    print: u64,
    fx: Option<GlassFx>,
}
static PREPARED: OnceLock<Mutex<Option<Prepared>>> = OnceLock::new();

fn prepared() -> &'static Mutex<Option<Prepared>> {
    PREPARED.get_or_init(|| Mutex::new(None))
}

/// Gold on black glass.
const GOLD: Rgba = Rgba(232, 194, 112, 255);
const OPEN_ANIM: f32 = 0.10;
const GLIDE: f32 = 0.11;
const PRESS: f32 = 0.12;

/// Screen area the menu owns (panel and its shadow); main.rs restores and
/// presents exactly this.
pub const REGION: (f32, f32, f32, f32) = (968.0, 0.0, 312.0, 410.0);
/// The menu grows out of the clock: the clock's column and top (992, 20;
/// 224 wide), down to the bottom of the tile under it (130 + 244 = 374).
const PANEL: (f32, f32, f32, f32) = (992.0, 20.0, 224.0, 354.0);
/// The clock's corners (a 46 high capsule).
const PANEL_R: f32 = 23.0;
/// The clock itself becomes the header: same height, same content.
const HEADER_H: f32 = 46.0;
const PAD: f32 = 12.0;
const GAP: f32 = 8.0;
const TILE_R: f32 = PANEL_R - PAD;
/// Two columns: 2 * 96 + 8 = 224 - 2 * 12.
const COL_W: f32 = (PANEL.2 - 2.0 * PAD - GAP) / 2.0;
const IN_X: f32 = PANEL.0 + PAD;
const COL2: f32 = IN_X + COL_W + GAP;
/// Four rows under the header: 4 * 66 + 3 * 8 = 374 - 12 - (66 + 8).
const GRID_Y: f32 = PANEL.1 + HEADER_H + GAP;
const ROW_H: f32 = (PANEL.1 + PANEL.3 - PAD - GRID_Y - 3.0 * GAP) / 4.0;
const fn row(r: f32) -> f32 {
    GRID_Y + r * (ROW_H + GAP)
}

/// The focusable tiles in screen coordinates, by action: 0 power,
/// 1 network, 2 do not disturb, 3 home/resume, 4 live TV, 5 weather,
/// 6 phone, 7 settings. On screen, two by two:
///   network | do not disturb
///   live TV | weather
///   phone   | settings
///   home    | power
const TILES: [(f32, f32, f32, f32); 8] = [
    (COL2, row(3.0), COL_W, ROW_H),
    (IN_X, row(0.0), COL_W, ROW_H),
    (COL2, row(0.0), COL_W, ROW_H),
    (IN_X, row(3.0), COL_W, ROW_H),
    (IN_X, row(1.0), COL_W, ROW_H),
    (COL2, row(1.0), COL_W, ROW_H),
    (IN_X, row(2.0), COL_W, ROW_H),
    (COL2, row(2.0), COL_W, ROW_H),
];

fn local(r: (f32, f32, f32, f32), radius: f32, kind: i32) -> Rect {
    Rect::new(r.0 - REGION.0, r.1 - REGION.1, r.2, r.3, radius, kind)
}

fn tile_rect(i: usize) -> Rect {
    local(TILES[i], TILE_R, glass_fx::TILE)
}

fn ease_glide(t: f32) -> f32 {
    // ease-out with a small overshoot: the highlight moves like a drop
    let t = t.clamp(0.0, 1.0);
    let c = 1.35;
    1.0 + (c + 1.0) * (t - 1.0).powi(3) + c * (t - 1.0).powi(2)
}

fn mix(a: Rect, b: Rect, k: f32) -> Rect {
    let l = |x: f32, y: f32| x + (y - x) * k;
    Rect::new(
        l(a.x, b.x),
        l(a.y, b.y),
        l(a.w, b.w),
        l(a.h, b.h),
        l(a.r, b.r),
        b.kind,
    )
}

impl Control {
    /// Prepare the text layer while Home is idle so opening the menu does
    /// not spend a frame on it. Only one small, low-priority worker runs.
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
                let mut canvas = content_canvas(2);
                let image = Control::render_content(&mut canvas, &key);
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

    /// Builds the glass from the Home picture now on screen, on a worker,
    /// once per minute (Home's clock is behind it) and after every change.
    pub fn prewarm_glass(c: &Canvas, cx: &Ctx) {
        let minute = crate::app::local_hms().1;
        let size = (c.pm.width(), c.pm.height());
        let scale = c.pm.width() as f32 / crate::gfx::W as f32;
        let (pw, ph) = (
            (REGION.2 * scale).round() as u32,
            (REGION.3 * scale).round() as u32,
        );
        let (rx, ry) = (
            (REGION.0 * scale).round() as usize,
            (REGION.1 * scale).round() as usize,
        );
        let stride = c.pm.width() as usize * 4;
        let print = region_print(c, rx, ry, pw, ph);
        {
            let mut p = prepared().lock().unwrap_or_else(|e| e.into_inner());
            if matches!(&*p, Some(q) if q.print == print && q.minute == minute && q.size == size) {
                return;
            }
            // claimed: no second worker for the same picture
            *p = Some(Prepared {
                minute,
                size,
                print,
                fx: None,
            });
        }
        let mut back = Vec::with_capacity(pw as usize * ph as usize * 4);
        for y in 0..ph as usize {
            let a = (ry + y) * stride + rx * 4;
            back.extend_from_slice(&c.pm.data()[a..a + pw as usize * 4]);
        }
        let tiles_key = (cx.cfg.do_not_disturb, cx.cfg.remote_enabled);
        let _ = std::thread::Builder::new()
            .name("control-glass".into())
            .spawn(move || {
                let panel = local(PANEL, PANEL_R, glass_fx::TILE);
                let fx = Glass::new(pw, ph, scale, false, panel).and_then(|mut glass| {
                    glass.set_backdrop(Some((&back[..], pw as usize * 4)));
                    glass.set_tiles(&Control::glass_tiles(tiles_key.0, tiles_key.1));
                    glass.build().then_some(GlassFx { glass, tiles_key })
                });
                let mut p = prepared().lock().unwrap_or_else(|e| e.into_inner());
                if matches!(&*p, Some(q) if q.print == print && q.size == size && q.fx.is_none()) {
                    *p = Some(Prepared {
                        minute,
                        size,
                        print,
                        fx,
                    });
                }
            });
    }

    /// The glass for Home as it is now has been built.
    pub fn glass_ready() -> bool {
        matches!(
            &*prepared().lock().unwrap_or_else(|e| e.into_inner()),
            Some(Prepared { fx: Some(_), .. })
        )
    }

    /// Home changes: the prepared glass shows an old picture.
    pub fn discard_glass() {
        *prepared().lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    pub fn prewarm_ready(cx: &Ctx, over_video: bool) -> bool {
        let key = current_panel_key(cx, over_video);
        let state = panel_cache().lock().unwrap_or_else(|e| e.into_inner());
        matches!(&*state, PanelCacheState::Ready(cached_key, _) if cached_key == &key)
    }

    pub fn new(over_video: bool) -> Self {
        Self::over(over_video, true)
    }

    /// The menu over another screen than Home: the glass prepared from
    /// Home's picture must not be used there (it showed Home's wallpaper).
    pub fn over_screen(over_video: bool) -> Self {
        Self::over(over_video, false)
    }

    fn over(over_video: bool, from_home: bool) -> Self {
        let now = Instant::now();
        // the glass built while Home stood still, if it shows Home as it is
        let minute = crate::app::local_hms().1;
        let ready = if over_video || !from_home {
            None
        } else {
            let mut p = prepared().lock().unwrap_or_else(|e| e.into_inner());
            match p.take() {
                Some(Prepared {
                    minute: m,
                    fx: Some(fx),
                    ..
                }) if m == minute => Some(fx),
                other => {
                    *p = other;
                    None
                }
            }
        };
        let backdrop_minute = ready.as_ref().map(|_| minute);
        Self {
            focus: 1,
            over_video,
            opened_at: now,
            panel_canvas: None,
            panel_key: None,
            glass: ready,
            backdrop_minute,
            glide_from: tile_rect(1),
            glide_at: now - std::time::Duration::from_secs(1),
            pressed_at: None,
            last_focus: None,
            last_open: 0.0,
            full_redraw: true,
        }
    }

    fn move_focus(&mut self, key: Key) {
        let centre = |i: usize| (TILES[i].0 + TILES[i].2 / 2.0, TILES[i].1 + TILES[i].3 / 2.0);
        let (x, y) = centre(self.focus);
        let mut next = None;
        let mut best = f32::INFINITY;
        for i in 0..TILES.len() {
            if i == self.focus {
                continue;
            }
            let (nx, ny) = centre(i);
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
            // glide on from wherever the highlight is right now
            self.glide_from = self.focus_rect(Instant::now());
            self.glide_at = Instant::now();
            self.pressed_at = None;
            self.focus = i;
        }
    }

    /// The highlight now: gliding, with a short squeeze after OK.
    fn focus_rect(&self, now: Instant) -> Rect {
        let k = ease_glide(now.duration_since(self.glide_at).as_secs_f32() / GLIDE);
        let mut r = mix(self.glide_from, tile_rect(self.focus), k);
        if let Some(p) = self.pressed_at {
            let t = now.duration_since(p).as_secs_f32() / PRESS;
            if t < 1.0 {
                let s = 1.0 - 0.05 * (t * std::f32::consts::PI).sin();
                let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
                r.w *= s;
                r.h *= s;
                r.x = cx - r.w / 2.0;
                r.y = cy - r.h / 2.0;
            }
        }
        r
    }

    fn opening(&self, now: Instant) -> f32 {
        crate::gfx::ease(now.duration_since(self.opened_at).as_secs_f32() / OPEN_ANIM)
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

    /// Power: an open ring and a bar (line icon, stroke 2).
    fn power_icon(c: &mut Canvas, x: f32, y: f32, col: Rgba) {
        let ring: Vec<(f32, f32)> = (0..=28)
            .map(|i| {
                let a = -std::f32::consts::FRAC_PI_2
                    + 0.62
                    + i as f32 / 28.0 * (2.0 * std::f32::consts::PI - 1.24);
                (x + a.cos() * 9.0, y + 1.0 + a.sin() * 9.0)
            })
            .collect();
        c.line(&ring, 2.0, col);
        c.line(&[(x, y - 10.0), (x, y - 1.0)], 2.0, col);
    }

    /// A filled crescent: the part of one disc outside another.
    fn moon_icon(c: &mut Canvas, x: f32, y: f32, col: Rgba) {
        use std::f32::consts::PI;
        let (r1, r2, off) = (9.0f32, 7.6f32, (4.6f32, -3.8f32));
        let mid = (off.1).atan2(off.0) + PI; // away from the cut
        let mut pts = Vec::new();
        for i in 0..=60 {
            let a = mid - PI + i as f32 / 60.0 * 2.0 * PI;
            let (px, py) = (a.cos() * r1, a.sin() * r1);
            if (px - off.0).hypot(py - off.1) >= r2 {
                pts.push((x + px, y + py));
            }
        }
        for i in (0..=60).rev() {
            let a = mid - PI + i as f32 / 60.0 * 2.0 * PI;
            let (px, py) = (off.0 + a.cos() * r2, off.1 + a.sin() * r2);
            if px.hypot(py) <= r1 {
                pts.push((x + px, y + py));
            }
        }
        c.polygon(&pts, col);
    }

    /// The other line icons of the menu, all 2 px gold strokes.
    fn line_icon(c: &mut Canvas, which: usize, x: f32, y: f32, col: Rgba) {
        match which {
            // TV: a screen on a small foot
            4 => {
                c.stroke_rect(x - 12.0, y - 9.0, 24.0, 16.0, 3.0, 2.0, col);
                c.line(&[(x - 5.0, y + 11.0), (x + 5.0, y + 11.0)], 2.0, col);
                c.line(&[(x, y + 7.0), (x, y + 11.0)], 2.0, col);
            }
            // weather: a sun
            5 => {
                let ring: Vec<(f32, f32)> = (0..=24)
                    .map(|i| {
                        let a = i as f32 / 24.0 * std::f32::consts::TAU;
                        (x + a.cos() * 5.0, y + a.sin() * 5.0)
                    })
                    .collect();
                c.line(&ring, 2.0, col);
                for i in 0..8 {
                    let a = i as f32 * std::f32::consts::FRAC_PI_4;
                    let (dx, dy) = (a.cos(), a.sin());
                    c.line(
                        &[(x + dx * 8.5, y + dy * 8.5), (x + dx * 11.5, y + dy * 11.5)],
                        2.0,
                        col,
                    );
                }
            }
            // phone
            6 => {
                c.stroke_rect(x - 7.0, y - 11.0, 14.0, 22.0, 3.5, 2.0, col);
                c.line(&[(x - 2.5, y + 7.0), (x + 2.5, y + 7.0)], 1.8, col);
            }
            // home: a house
            3 => {
                c.line(
                    &[(x - 11.0, y - 1.0), (x, y - 10.0), (x + 11.0, y - 1.0)],
                    2.0,
                    col,
                );
                c.line(
                    &[
                        (x - 8.0, y - 3.0),
                        (x - 8.0, y + 10.0),
                        (x + 8.0, y + 10.0),
                        (x + 8.0, y - 3.0),
                    ],
                    2.0,
                    col,
                );
                c.line(
                    &[
                        (x - 2.5, y + 10.0),
                        (x - 2.5, y + 4.0),
                        (x + 2.5, y + 4.0),
                        (x + 2.5, y + 10.0),
                    ],
                    2.0,
                    col,
                );
            }
            _ => super::icon_gear(c, x, y, 22.0, col),
        }
    }

    /// Text and icons of the menu, transparent (the glass is drawn in C++).
    fn render_content(canvas: &mut Canvas, key: &PanelKey) -> Image {
        canvas.clear(Rgba(0, 0, 0, 0));
        canvas.offset = (-REGION.0, -REGION.1);
        canvas.opacity = 1.0;
        Self::draw_content(canvas, key);
        canvas.finish();
        canvas.snapshot()
    }

    fn draw_content(c: &mut Canvas, key: &PanelKey) {
        let (hh, mm, connected, remote_enabled, dnd, over_video, net_label) =
            (key.0, key.1, key.2, key.3, key.4, key.5, &key.6);
        let cream = Rgba(246, 236, 214, 255);
        let muted = Rgba(176, 162, 136, 255);
        let (x, y) = (PANEL.0, PANEL.1);
        // the header is the clock it grew out of, at the same places
        c.text(
            &format!("{hh:02}:{mm:02}"),
            x + 18.0,
            y + 12.0,
            19,
            Weight::Semi,
            cream,
            Align::Left,
        );
        c.line(
            &[(x + 82.0, y + 12.0), (x + 82.0, y + HEADER_H - 12.0)],
            1.0,
            Rgba(GOLD.0, GOLD.1, GOLD.2, 70),
        );
        c.circle(
            x + 99.0,
            y + HEADER_H * 0.5,
            3.2,
            if connected {
                GOLD
            } else {
                Rgba(236, 110, 96, 255)
            },
        );
        c.text_fit(
            net_label,
            x + 111.0,
            y + 16.0,
            12,
            Weight::Regular,
            muted,
            PANEL.2 - 111.0 - 16.0,
        );

        // the buttons: a gold line icon over the label
        let labels = [
            "Ausschalten",
            "Netzwerk",
            if dnd { "Ruhe an" } else { "Nicht stören" },
            if over_video {
                "Weiter ansehen"
            } else {
                "Startseite"
            },
            "Live TV",
            "Wetter",
            if remote_enabled {
                "Handy an"
            } else {
                "Handy aus"
            },
            "Einstellungen",
        ];
        for (i, label) in labels.iter().enumerate() {
            let (tx, ty, tw, th) = TILES[i];
            let (cx, cy) = (tx + tw / 2.0, ty + 25.0);
            match i {
                0 => Self::power_icon(c, cx, cy, GOLD),
                1 => draw_wifi(c, cx, cy - 1.0, GOLD),
                2 => Self::moon_icon(c, cx, cy, GOLD),
                _ => Self::line_icon(c, i, cx, cy, GOLD),
            }
            let label = c.ellipsize(label, 11, Weight::Semi, tw - 12.0);
            c.text(
                &label,
                cx,
                ty + th - 21.0,
                11,
                Weight::Semi,
                cream,
                Align::Center,
            );
        }
    }

    /// The tiles of the glass (their state changes the glass, not the text).
    fn glass_tiles(dnd: bool, remote: bool) -> Vec<Rect> {
        let mut v = Vec::with_capacity(TILES.len());
        for i in 0..TILES.len() {
            let active = (i == 2 && dnd) || (i == 6 && remote);
            v.push(local(
                TILES[i],
                TILE_R,
                if active {
                    glass_fx::TILE_ACTIVE
                } else {
                    glass_fx::TILE
                },
            ));
        }
        v
    }
}

/// A cheap fingerprint of the Home pixels behind the menu (every 7th pixel
/// of every 5th row: a moved focus ring or a new headline changes it).
fn region_print(c: &Canvas, rx: usize, ry: usize, pw: u32, ph: u32) -> u64 {
    let stride = c.pm.width() as usize * 4;
    let d = c.pm.data();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for y in (0..ph as usize).step_by(5) {
        let row = (ry + y) * stride + rx * 4;
        for x in (0..pw as usize).step_by(7) {
            let i = row + x * 4;
            let v = u32::from_ne_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]]);
            h = (h ^ v as u64).wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

fn content_canvas(threads: usize) -> Canvas {
    Canvas::new_scaled_with_threads(
        REGION.2 as u32,
        REGION.3 as u32,
        crate::gfx::render_scale(),
        threads,
    )
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
        let now = Instant::now();
        now.duration_since(self.opened_at).as_secs_f32() < OPEN_ANIM
            || now.duration_since(self.glide_at).as_secs_f32() < GLIDE
            || self
                .pressed_at
                .map(|p| now.duration_since(p).as_secs_f32() < PRESS)
                .unwrap_or(false)
    }

    fn key(&mut self, key: Key, cx: &mut Ctx) -> Nav {
        match key {
            Key::Back | Key::Menu | Key::LongOk => Nav::Pop,
            Key::Home | Key::LongBack => Nav::Home,
            Key::Left | Key::Right | Key::Up | Key::Down => {
                self.move_focus(key);
                Nav::None
            }
            Key::Ok => {
                self.pressed_at = Some(Instant::now());
                self.activate(cx)
            }
            _ => Nav::None,
        }
    }

    fn glass(&self) -> bool {
        true
    }

    fn needs_backdrop(&self) -> bool {
        // the first frame, and once a minute: Home's clock is behind the glass
        match self.backdrop_minute {
            None => true,
            Some(_) if self.over_video => false,
            Some(m) => m != crate::app::local_hms().1,
        }
    }

    fn draw(&mut self, c: &mut Canvas, cx: &mut Ctx) {
        let now = Instant::now();
        let scale = c.pm.width() as f32 / crate::gfx::W as f32;
        let (pw, ph) = (
            (REGION.2 * scale).round() as u32,
            (REGION.3 * scale).round() as u32,
        );
        let (rx, ry) = (
            (REGION.0 * scale).round() as usize,
            (REGION.1 * scale).round() as usize,
        );
        let stride = c.pm.width() as usize * 4;

        // the glass, once per menu
        if self.glass.is_none() {
            let panel = local(PANEL, PANEL_R, glass_fx::TILE);
            self.glass = Glass::new(pw, ph, scale, self.over_video, panel).map(|glass| GlassFx {
                glass,
                tiles_key: (!cx.cfg.do_not_disturb, false),
            });
            self.full_redraw = true;
        }
        // Over a film or the browser the menu plane must be see-through
        // everywhere but the menu: an old picture left in it (Home's
        // wallpaper) covered the browser, which then looked frozen.
        if self.over_video && self.full_redraw {
            c.pm.data_mut().fill(0);
            c.mark_damage_rect(0.0, 0.0, crate::gfx::W as f32, crate::gfx::H as f32);
        }
        // Home behind it (main.rs has just drawn it, sharp, when we asked)
        let minute = crate::app::local_hms().1;
        let fresh_backdrop = self.needs_backdrop();
        if let Some(g) = self.glass.as_mut() {
            if fresh_backdrop {
                let start = ry * stride + rx * 4;
                let backdrop = (!self.over_video).then(|| (&c.pm.data()[start..], stride));
                g.glass.set_backdrop(backdrop);
                self.backdrop_minute = Some(minute);
                self.full_redraw = true;
            }
            let tiles_key = (cx.cfg.do_not_disturb, cx.cfg.remote_enabled);
            if g.tiles_key != tiles_key {
                g.glass
                    .set_tiles(&Self::glass_tiles(tiles_key.0, tiles_key.1));
                g.tiles_key = tiles_key;
                g.glass.build();
                self.full_redraw = true;
            }
        }
        // text and icons: prepared in the background while Home was idle
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
            let image = prepared.unwrap_or_else(|| {
                let canvas = self.panel_canvas.get_or_insert_with(|| content_canvas(1));
                Self::render_content(canvas, &key)
            });
            if let Some(g) = self.glass.as_mut() {
                g.glass
                    .set_content(image.data(), image.width() as usize * 4);
            }
            self.panel_key = Some(key);
            self.full_redraw = true;
        }
        let open = self.opening(now);
        let focus = self.focus_rect(now);
        let Some(g) = self.glass.as_mut() else {
            // no memory for the glass: a plain panel
            c.rect(
                PANEL.0,
                PANEL.1,
                PANEL.2,
                PANEL.3,
                PANEL_R,
                Rgba(24, 29, 43, 245),
            );
            let r = TILES[self.focus];
            c.stroke_rect(r.0, r.1, r.2, r.3, TILE_R, 2.0, Rgba(216, 231, 255, 255));
            return;
        };
        if !g.glass.built() && !g.glass.build() {
            return;
        }

        // What changed: everything (opening, new glass) or only the path of
        // the highlight since the last frame.
        let area = if self.full_redraw || open < 1.0 || self.last_open < 1.0 {
            Some((0.0, 0.0, REGION.2, REGION.3))
        } else if self.last_focus != Some(focus) {
            let a = self.last_focus.unwrap_or(focus);
            let m = 14.0; // the glow around the highlight
            let x0 = a.x.min(focus.x) - m;
            let y0 = a.y.min(focus.y) - m;
            let x1 = (a.x + a.w).max(focus.x + focus.w) + m;
            let y1 = (a.y + a.h).max(focus.y + focus.h) + m;
            Some((x0, y0, x1 - x0, y1 - y0))
        } else {
            None
        };
        let Some(area) = area else { return };
        let start = ry * stride + rx * 4;
        let dst = &mut c.pm.data_mut()[start..];
        if open < 1.0 {
            // grows down out of the clock: above the edge the menu, below
            // it still Home (opacity 0 shows what was behind)
            let edge = PANEL.1 + HEADER_H + (PANEL.3 - HEADER_H) * open - REGION.1;
            g.glass.render(
                dst,
                stride,
                Some(focus),
                1.0,
                (area.0, area.1, area.2, edge - area.1),
            );
            g.glass.render(
                dst,
                stride,
                None,
                0.0,
                (area.0, edge, area.2, area.1 + area.3 - edge),
            );
        } else {
            g.glass.render(dst, stride, Some(focus), 1.0, area);
        }
        c.mark_damage_rect(REGION.0 + area.0, REGION.1 + area.1, area.2, area.3);
        c.request_present();
        self.last_focus = Some(focus);
        self.last_open = open;
        self.full_redraw = false;
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
