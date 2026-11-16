//! Screens and the widgets they share.
pub mod apps;
mod apps_wheel;
pub mod browser;
pub mod control;
pub mod films;
pub mod home;
pub mod livetv;
pub mod news;
pub mod player;
pub mod screensaver;
pub mod settings;
pub mod weather;
pub mod youtube;

use crate::app::Ctx;
use crate::gfx::{color, ease, Align, Canvas, Rgba, Weight};
use crate::platform::input::Key;

pub const PAD: f32 = 64.0;

/// Focus highlight: a crisp white edge with a soft light around it.
pub fn focus_ring(c: &mut Canvas, x: f32, y: f32, w: f32, h: f32, r: f32) {
    c.stroke_rect(
        x - 9.0,
        y - 9.0,
        w + 18.0,
        h + 18.0,
        r + 9.0,
        6.0,
        Rgba(255, 255, 255, 14),
    );
    c.stroke_rect(
        x - 6.0,
        y - 6.0,
        w + 12.0,
        h + 12.0,
        r + 6.0,
        4.0,
        Rgba(160, 190, 255, 40),
    );
    c.stroke_rect(
        x - 3.0,
        y - 3.0,
        w + 6.0,
        h + 6.0,
        r + 3.0,
        3.0,
        color::WHITE,
    );
}

/// Low-cost focus wash for Home's animated glass pane, with no hard frame.
pub fn focus_glass_fast(c: &mut Canvas, x: f32, y: f32, w: f32, h: f32) {
    c.mark_damage_rect(x, y, w, h);
    c.rect(
        x + 1.0,
        y + 1.0,
        w - 2.0,
        h - 2.0,
        21.0,
        Rgba(226, 187, 116, 20),
    );
}

/// A glass panel: translucent fill, a light top edge and a hairline border,
/// so the living backdrop shines through.
pub fn glass(c: &mut Canvas, x: f32, y: f32, w: f32, h: f32, r: f32) {
    // one translucent fill (the dark tint and the light film combined): one blend, not two
    c.rect(x, y, w, h, r, Rgba(31, 31, 37, 155));
    c.stroke_rect(
        x + 0.5,
        y + 0.5,
        w - 1.0,
        h - 1.0,
        r,
        1.0,
        Rgba(255, 255, 255, 22),
    );
    c.line(
        &[(x + r, y + 1.0), (x + w - r, y + 1.0)],
        1.0,
        Rgba(255, 255, 255, 40),
    );
}

/// "Guten Morgen" etc. by the time of day.
pub fn greeting() -> &'static str {
    match crate::app::local_hms().0 {
        5..=10 => "Guten Morgen",
        11..=13 => "Mahlzeit",
        14..=17 => "Guten Tag",
        18..=22 => "Guten Abend",
        _ => "Gute Nacht",
    }
}

static FRAME_DT_US: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(16_000);

/// Seconds since the previous frame (set by the main loop, capped).
pub fn frame_dt() -> f32 {
    FRAME_DT_US.load(std::sync::atomic::Ordering::Relaxed) as f32 / 1e6
}
pub fn set_frame_dt(s: f32) {
    // A long gap means the screen was still (nothing to animate): the first
    // frame of a new movement then counts as one ordinary frame, so it does
    // not jump ahead. Otherwise the real time is used, and a slow frame moves
    // further instead of stretching the animation.
    let s = if s > 0.12 { 1.0 / 60.0 } else { s.max(0.001) };
    FRAME_DT_US.store((s * 1e6) as u32, std::sync::atomic::Ordering::Relaxed);
}

/// The focus frame: glides from one item to the next instead of jumping.
#[derive(Default)]
pub struct Focus {
    x: Anim,
    y: Anim,
    w: Anim,
    h: Anim,
    shown: bool,
}

impl Focus {
    pub fn draw(&mut self, c: &mut Canvas, x: f32, y: f32, w: f32, h: f32, r: f32) {
        self.draw_with_motion(c, x, y, w, h, r, !cfg!(target_arch = "arm"));
    }

    /// Draws the focus frame, optionally gliding on screens with a retained
    /// base layer where animation does not trigger a full-screen reraster.
    pub fn draw_with_motion(
        &mut self,
        c: &mut Canvas,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        r: f32,
        motion: bool,
    ) -> (f32, f32, f32, f32) {
        let bounds = self.move_to(x, y, w, h, motion);
        self.draw_bounds(c, bounds, r);
        bounds
    }

    pub fn move_to(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        motion: bool,
    ) -> (f32, f32, f32, f32) {
        self.move_to_with_tau(x, y, w, h, motion, 0.07)
    }

    /// Move the focus frame with a screen-specific response time.
    pub fn move_to_with_tau(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        motion: bool,
        tau: f32,
    ) -> (f32, f32, f32, f32) {
        if !self.shown {
            self.x.jump(x);
            self.y.jump(y);
            self.w.jump(w);
            self.h.jump(h);
            self.shown = true;
        }
        if motion {
            self.x.set(x);
            self.y.set(y);
            self.w.set(w);
            self.h.set(h);
            (
                self.x.step_with_tau(tau),
                self.y.step_with_tau(tau),
                self.w.step_with_tau(tau),
                self.h.step_with_tau(tau),
            )
        } else {
            self.x.jump(x);
            self.y.jump(y);
            self.w.jump(w);
            self.h.jump(h);
            (self.x.v, self.y.v, self.w.v, self.h.v)
        }
    }

    pub fn draw_bounds(&self, c: &mut Canvas, bounds: (f32, f32, f32, f32), r: f32) {
        focus_ring(c, bounds.0, bounds.1, bounds.2, bounds.3, r);
    }
    pub fn moving(&self) -> bool {
        self.x.moving() || self.y.moving() || self.w.moving() || self.h.moving()
    }
    /// Next draw starts at the item (no glide in from somewhere else).
    pub fn reset(&mut self) {
        self.shown = false;
    }
}

/// Smoothly follows a target (focus movement, scrolling).
#[derive(Default, Clone, Copy)]
pub struct Anim {
    pub v: f32,
    pub target: f32,
}
impl Anim {
    pub fn set(&mut self, t: f32) {
        self.target = t;
    }
    pub fn jump(&mut self, t: f32) {
        self.v = t;
        self.target = t;
    }
    /// Moves towards the target by elapsed time (not per frame), so motion
    /// takes the same ~0.25 s however fast the Pi draws.
    pub fn step(&mut self) -> f32 {
        self.step_with_tau(0.07)
    }
    pub fn step_with_tau(&mut self, tau: f32) -> f32 {
        let d = self.target - self.v;
        let k = 1.0 - (-frame_dt() / tau.max(0.001)).exp();
        let eps = (self.target.abs() * 0.002).max(0.004);
        self.v = if d.abs() < eps {
            self.target
        } else {
            self.v + d * k
        };
        self.v
    }
    pub fn moving(&self) -> bool {
        (self.target - self.v).abs() >= (self.target.abs() * 0.002).max(0.004)
    }
}

pub fn title(c: &mut Canvas, s: &str) {
    c.text(s, PAD, 40.0, 34, Weight::Bold, color::TEXT, Align::Left);
}

/// Compact system status capsule: clock, network and optional phone remote.
pub fn status(c: &mut Canvas, cx: &Ctx) {
    status_with_phone_accent(c, cx, Rgba::hex(0x75c9ff));
}

/// Draw the shared status capsule with a screen-specific remote-control tint.
pub fn status_with_phone_accent(c: &mut Canvas, cx: &Ctx, phone_accent: Rgba) {
    let (hour, minute, _) = crate::app::local_hms();
    let right = crate::gfx::W as f32 - PAD;
    let (x, y, w, capsule_h) = (right - 284.0, 20.0, 284.0, 46.0);
    c.rect(x, y, w, capsule_h, 23.0, Rgba::hex(0x23252c).alpha(0.94));
    c.stroke_rect(x, y, w, capsule_h, 23.0, 1.0, color::BORDER.alpha(0.9));
    c.text(
        &format!("{hour:02}:{minute:02}"),
        x + 18.0,
        y + 12.0,
        19,
        Weight::Semi,
        color::TEXT,
        Align::Left,
    );
    c.line(
        &[(x + 82.0, y + 10.0), (x + 82.0, y + capsule_h - 10.0)],
        1.0,
        color::BORDER.alpha(0.9),
    );
    if let Some((ok, kind, name)) = cx.net.get() {
        let state = if ok { Rgba::hex(0x59d39a) } else { color::RED };
        c.circle(x + 99.0, y + capsule_h * 0.5, 3.2, state);
        if ok {
            if kind == "wifi" {
                icon_wifi(c, x + 119.0, y + capsule_h * 0.5 - 2.0, 17.0, color::TEXT)
            } else {
                icon_ethernet(c, x + 119.0, y + capsule_h * 0.5, 17.0, color::TEXT)
            }
            let name = c.ellipsize(&name, 12, Weight::Regular, 79.0);
            c.text(
                &name,
                x + 135.0,
                y + 16.0,
                12,
                Weight::Regular,
                color::MUTED,
                Align::Left,
            );
        } else {
            c.text(
                "Offline",
                x + 111.0,
                y + 15.0,
                13,
                Weight::Semi,
                color::RED,
                Align::Left,
            );
        }
    } else {
        c.circle(x + 99.0, y + capsule_h * 0.5, 3.2, color::MUTED);
        c.text(
            "Netzwerk",
            x + 111.0,
            y + 15.0,
            13,
            Weight::Regular,
            color::MUTED,
            Align::Left,
        );
    }
    c.line(
        &[(x + 239.0, y + 10.0), (x + 239.0, y + capsule_h - 10.0)],
        1.0,
        color::BORDER.alpha(0.9),
    );
    let phone = if cx.cfg.remote_enabled {
        phone_accent
    } else {
        color::MUTED
    };
    c.stroke_rect(x + 255.0, y + 13.0, 12.0, 20.0, 3.0, 1.4, phone);
    c.circle(x + 261.0, y + 29.0, 0.8, phone);
}

/// Where the notification is drawn this frame (it slides in from below), or
/// None when there is none. Ends a notification whose time is up. The box is
/// on even coordinates, so it lands on whole pixels at the Pi's 1.5x scale.
pub fn toast_area(c: &mut Canvas, cx: &mut Ctx) -> Option<(f32, f32, f32, f32)> {
    let t = cx.toast.as_ref()?;
    if t.at.elapsed().as_secs_f32() > 4.5 {
        cx.toast = None;
        return None;
    }
    let w = (c.fonts.width(&t.text, 20, Weight::Semi) + 56.0).min(1100.0);
    let x = (((crate::gfx::W as f32 - w) / 2.0 - 4.0) / 2.0).floor() * 2.0;
    let w = ((w + 8.0) / 2.0).ceil() * 2.0 + 2.0;
    Some((x, crate::gfx::H as f32 - 100.0, w, 92.0))
}

pub fn toast(c: &mut Canvas, cx: &mut Ctx) {
    let Some(t) = &cx.toast else { return };
    let age = t.at.elapsed().as_secs_f32();
    if age > 4.5 {
        cx.toast = None;
        return;
    }
    let a = (age / 0.25).min(1.0).min((4.5 - age) / 0.4);
    let text = t.text.clone();
    let w = (c.fonts.width(&text, 20, Weight::Semi) + 56.0).min(1100.0);
    let x = (crate::gfx::W as f32 - w) / 2.0;
    let y = crate::gfx::H as f32 - 96.0 + (1.0 - ease(a)) * 30.0;
    c.rect(x, y, w, 52.0, 26.0, Rgba::hex(0x26262b).alpha(a * 0.97));
    c.stroke_rect(x, y, w, 52.0, 26.0, 1.0, color::BORDER.alpha(a));
    let t2 = c.ellipsize(&text, 20, Weight::Semi, w - 48.0);
    c.text(
        &t2,
        x + w / 2.0,
        y + 13.0,
        20,
        Weight::Semi,
        color::TEXT.alpha(a),
        Align::Center,
    );
}

/// Grid navigation helper: index after a key in a grid of `cols`.
pub fn grid_move(i: usize, n: usize, cols: usize, k: Key) -> Option<usize> {
    if n == 0 {
        return None;
    }
    match k {
        Key::Left if i % cols > 0 => Some(i - 1),
        Key::Right if i % cols + 1 < cols && i + 1 < n => Some(i + 1),
        Key::Up if i >= cols => Some(i - cols),
        Key::Down if i + cols < n => Some(i + cols),
        Key::Down if i / cols < (n - 1) / cols => Some(n - 1),
        _ => None,
    }
}

// ---------------------------------------------------------------- icons
pub fn icon_play(c: &mut Canvas, x: f32, y: f32, s: f32, col: Rgba) {
    c.polygon(
        &[
            (x - s * 0.35, y - s * 0.5),
            (x + s * 0.5, y),
            (x - s * 0.35, y + s * 0.5),
        ],
        col,
    );
}
pub fn icon_pause(c: &mut Canvas, x: f32, y: f32, s: f32, col: Rgba) {
    c.rect(x - s * 0.4, y - s * 0.5, s * 0.28, s, 2.0, col);
    c.rect(x + s * 0.12, y - s * 0.5, s * 0.28, s, 2.0, col);
}
pub fn icon_wifi(c: &mut Canvas, x: f32, y: f32, s: f32, col: Rgba) {
    for (i, r) in [0.95f32, 0.62, 0.3].iter().enumerate() {
        let rr = s * r * 0.5;
        let pts: Vec<(f32, f32)> = (0..=12)
            .map(|k| {
                let a = std::f32::consts::PI * (1.25 + 0.5 * k as f32 / 12.0);
                (x + rr * a.cos(), y + s * 0.25 + rr * a.sin())
            })
            .collect();
        let _ = i;
        c.line(&pts, s * 0.1, col);
    }
    c.circle(x, y + s * 0.25, s * 0.07, col);
}
pub fn icon_ethernet(c: &mut Canvas, x: f32, y: f32, s: f32, col: Rgba) {
    c.stroke_rect(x - s * 0.4, y - s * 0.3, s * 0.8, s * 0.6, 3.0, 2.0, col);
    for i in 0..3 {
        c.rect(
            x - s * 0.22 + i as f32 * s * 0.18,
            y - s * 0.1,
            s * 0.08,
            s * 0.25,
            1.0,
            col,
        );
    }
}
pub fn icon_gear(c: &mut Canvas, x: f32, y: f32, s: f32, col: Rgba) {
    for i in 0..8 {
        let a = i as f32 * std::f32::consts::PI / 4.0;
        c.line(
            &[
                (x + a.cos() * s * 0.25, y + a.sin() * s * 0.25),
                (x + a.cos() * s * 0.48, y + a.sin() * s * 0.48),
            ],
            s * 0.16,
            col,
        );
    }
    c.circle(x, y, s * 0.33, col);
    c.circle(x, y, s * 0.14, color::BG2);
}
pub fn icon_film(c: &mut Canvas, x: f32, y: f32, s: f32, col: Rgba) {
    c.stroke_rect(
        x - s * 0.45,
        y - s * 0.32,
        s * 0.9,
        s * 0.64,
        4.0,
        s * 0.07,
        col,
    );
    for i in 0..4 {
        let xx = x - s * 0.33 + i as f32 * s * 0.22;
        c.rect(xx, y - s * 0.26, s * 0.08, s * 0.08, 1.0, col);
        c.rect(xx, y + s * 0.18, s * 0.08, s * 0.08, 1.0, col);
    }
    icon_play(c, x + s * 0.03, y, s * 0.26, col);
}
pub fn icon_youtube(c: &mut Canvas, x: f32, y: f32, s: f32) {
    c.rect(
        x - s * 0.5,
        y - s * 0.35,
        s,
        s * 0.7,
        s * 0.18,
        Rgba::hex(0xff0033),
    );
    icon_play(c, x + s * 0.04, y, s * 0.34, color::WHITE);
}
pub fn icon_news(c: &mut Canvas, x: f32, y: f32, s: f32, col: Rgba) {
    c.stroke_rect(
        x - s * 0.42,
        y - s * 0.36,
        s * 0.84,
        s * 0.72,
        4.0,
        s * 0.07,
        col,
    );
    c.rect(x - s * 0.28, y - s * 0.22, s * 0.24, s * 0.2, 2.0, col);
    for i in 0..3 {
        c.rect(
            x + s * 0.02,
            y - s * 0.2 + i as f32 * s * 0.08,
            s * 0.26,
            s * 0.04,
            1.0,
            col,
        );
    }
    for i in 0..2 {
        c.rect(
            x - s * 0.28,
            y + s * 0.08 + i as f32 * s * 0.1,
            s * 0.56,
            s * 0.04,
            1.0,
            col,
        );
    }
}
pub fn icon_search(c: &mut Canvas, x: f32, y: f32, s: f32, col: Rgba) {
    c.stroke_rect(
        x - s * 0.35,
        y - s * 0.35,
        s * 0.5,
        s * 0.5,
        s * 0.25,
        s * 0.09,
        col,
    );
    c.line(
        &[(x + s * 0.08, y + s * 0.08), (x + s * 0.38, y + s * 0.38)],
        s * 0.1,
        col,
    );
}

/// Weather picture (sun, clouds, rain, ...), vector-drawn.
pub fn icon_weather(c: &mut Canvas, sky: crate::weather::Sky, day: bool, x: f32, y: f32, s: f32) {
    use crate::weather::Sky::*;
    let sun = Rgba::hex(0xffcc33);
    let moon = Rgba::hex(0xdfe6f5);
    let cloud = Rgba::hex(0xdfe3ea);
    let dark = Rgba::hex(0x9aa3b2);
    let drop = Rgba::hex(0x5ab0ff);
    let draw_cloud = |c: &mut Canvas, cx: f32, cy: f32, k: f32, col: Rgba| {
        c.circle(cx - k * 0.22, cy + k * 0.05, k * 0.2, col);
        c.circle(cx + k * 0.02, cy - k * 0.08, k * 0.27, col);
        c.circle(cx + k * 0.26, cy + k * 0.06, k * 0.18, col);
        c.rect(
            cx - k * 0.42,
            cy + k * 0.04,
            k * 0.86,
            k * 0.2,
            k * 0.1,
            col,
        );
    };
    let sun_or_moon = |c: &mut Canvas, cx: f32, cy: f32, k: f32| {
        if day {
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::PI / 4.0;
                c.line(
                    &[
                        (cx + a.cos() * k * 0.32, cy + a.sin() * k * 0.32),
                        (cx + a.cos() * k * 0.45, cy + a.sin() * k * 0.45),
                    ],
                    k * 0.07,
                    sun,
                );
            }
            c.circle(cx, cy, k * 0.24, sun);
        } else {
            c.circle(cx, cy, k * 0.34, moon.a(40));
            c.circle(cx, cy, k * 0.25, moon);
            c.circle(cx - k * 0.07, cy - k * 0.05, k * 0.05, Rgba::hex(0xc5cddd));
            c.circle(cx + k * 0.08, cy + k * 0.07, k * 0.035, Rgba::hex(0xc5cddd));
        }
    };
    match sky {
        Clear => sun_or_moon(c, x, y, s),
        PartlyCloudy => {
            sun_or_moon(c, x - s * 0.14, y - s * 0.14, s * 0.8);
            draw_cloud(c, x + s * 0.08, y + s * 0.08, s * 0.8, cloud);
        }
        Cloudy | Fog => {
            draw_cloud(c, x - s * 0.1, y - s * 0.06, s * 0.7, dark);
            draw_cloud(c, x + s * 0.06, y + s * 0.06, s * 0.85, cloud);
            if sky == Fog {
                for i in 0..2 {
                    c.line(
                        &[
                            (x - s * 0.35, y + s * 0.34 + i as f32 * s * 0.1),
                            (x + s * 0.35, y + s * 0.34 + i as f32 * s * 0.1),
                        ],
                        s * 0.05,
                        cloud,
                    );
                }
            }
        }
        Drizzle | Rain | Thunder | Snow => {
            draw_cloud(
                c,
                x,
                y - s * 0.1,
                s * 0.9,
                if sky == Thunder { dark } else { cloud },
            );
            for i in 0..3 {
                let dx = x - s * 0.2 + i as f32 * s * 0.2;
                match sky {
                    Snow => c.circle(dx, y + s * 0.3, s * 0.05, cloud),
                    Thunder if i == 1 => c.polygon(
                        &[
                            (dx, y + s * 0.12),
                            (dx - s * 0.1, y + s * 0.32),
                            (dx, y + s * 0.3),
                            (dx - s * 0.06, y + s * 0.46),
                            (dx + s * 0.1, y + s * 0.24),
                            (dx, y + s * 0.26),
                        ],
                        sun,
                    ),
                    _ => c.line(
                        &[(dx + s * 0.03, y + s * 0.2), (dx - s * 0.04, y + s * 0.38)],
                        s * 0.05,
                        drop,
                    ),
                }
            }
        }
    }
}

// ---------------------------------------------------------------- keyboard
/// On-screen keyboard (QWERTZ) for search, passwords and server addresses.
pub struct Keyboard {
    pub text: String,
    pub secret: bool,
    row: usize,
    col: usize,
    shift: bool,
    symbols: bool,
    pub title: String,
}

pub enum KbResult {
    None,
    Done(String),
    Cancel,
    Changed,
}

const ROWS: [&str; 4] = ["1234567890", "qwertzuiopü", "asdfghjklöä", "yxcvbnmß.-"];
const SYM: [&str; 4] = ["1234567890", "@#$%&*()+=_", ":/?!'\"~;,<>", "[]{}\\|^`€§"];
const SPECIAL: [&str; 5] = ["⇧", "?123", "Leer", "⌫", "Fertig"];

impl Keyboard {
    pub fn new(title: &str, text: &str, secret: bool) -> Keyboard {
        Keyboard {
            text: text.into(),
            secret,
            row: 1,
            col: 0,
            shift: false,
            symbols: false,
            title: title.into(),
        }
    }
    fn rows(&self) -> [&'static str; 4] {
        if self.symbols {
            SYM
        } else {
            ROWS
        }
    }
    fn row_len(&self, r: usize) -> usize {
        if r == 4 {
            SPECIAL.len()
        } else {
            self.rows()[r].chars().count()
        }
    }
    pub fn key(&mut self, k: Key) -> KbResult {
        match k {
            Key::Up if self.row > 0 => {
                self.row -= 1;
                self.col = self.col.min(self.row_len(self.row) - 1);
            }
            Key::Down if self.row < 4 => {
                self.row += 1;
                self.col = if self.row == 4 {
                    (self.col * SPECIAL.len() / 11).min(SPECIAL.len() - 1)
                } else {
                    self.col.min(self.row_len(self.row) - 1)
                };
            }
            Key::Left => {
                self.col = if self.col == 0 {
                    self.row_len(self.row) - 1
                } else {
                    self.col - 1
                }
            }
            Key::Right => self.col = (self.col + 1) % self.row_len(self.row),
            Key::Back => return KbResult::Cancel,
            Key::Backspace => {
                self.text.pop();
                return KbResult::Changed;
            }
            Key::Char(ch) => {
                self.text.push(ch);
                return KbResult::Changed;
            }
            Key::Digit(d) => {
                self.text.push((b'0' + d) as char);
                return KbResult::Changed;
            }
            Key::Ok => {
                if self.row == 4 {
                    match self.col {
                        0 => self.shift = !self.shift,
                        1 => self.symbols = !self.symbols,
                        2 => {
                            self.text.push(' ');
                            return KbResult::Changed;
                        }
                        3 => {
                            self.text.pop();
                            return KbResult::Changed;
                        }
                        _ => return KbResult::Done(self.text.clone()),
                    }
                } else {
                    let ch = self.rows()[self.row].chars().nth(self.col).unwrap();
                    let ch = if self.shift {
                        ch.to_uppercase().next().unwrap_or(ch)
                    } else {
                        ch
                    };
                    self.text.push(ch);
                    self.shift = false;
                    return KbResult::Changed;
                }
            }
            Key::Play | Key::PlayPause => return KbResult::Done(self.text.clone()),
            _ => {}
        }
        KbResult::None
    }

    /// Draws the field + keys at (x, y), width w. Returns the height.
    pub fn draw(&self, c: &mut Canvas, x: f32, y: f32, w: f32, t: f32) -> f32 {
        c.text(
            &self.title,
            x,
            y,
            18,
            Weight::Semi,
            color::MUTED,
            Align::Left,
        );
        c.rect(x, y + 30.0, w, 58.0, 14.0, color::RAISED);
        let shown = if self.secret {
            "•".repeat(self.text.chars().count())
        } else {
            self.text.clone()
        };
        let tw = c.text_fit(
            &shown,
            x + 20.0,
            y + 46.0,
            24,
            Weight::Regular,
            color::TEXT,
            w - 50.0,
        );
        if (t * 2.0) as i32 % 2 == 0 {
            c.rect(x + 22.0 + tw, y + 44.0, 2.0, 30.0, 1.0, color::ACCENT);
        }
        let ky = y + 110.0;
        let kw = (w - 10.0 * 10.0) / 11.0;
        let kh = 50.0;
        for (r, row) in self.rows().iter().enumerate() {
            let n = row.chars().count() as f32;
            let off = x + (w - (n * kw + (n - 1.0) * 10.0)) / 2.0;
            for (i, ch) in row.chars().enumerate() {
                let kx = off + i as f32 * (kw + 10.0);
                let yy = ky + r as f32 * (kh + 10.0);
                let focused = self.row == r && self.col == i;
                c.rect(
                    kx,
                    yy,
                    kw,
                    kh,
                    10.0,
                    if focused { color::WHITE } else { color::RAISED },
                );
                let label = if self.shift {
                    ch.to_uppercase().to_string()
                } else {
                    ch.to_string()
                };
                c.text(
                    &label,
                    kx + kw / 2.0,
                    yy + 12.0,
                    22,
                    Weight::Semi,
                    if focused { color::BLACK } else { color::TEXT },
                    Align::Center,
                );
            }
        }
        let yy = ky + 4.0 * (kh + 10.0);
        let widths = [1.4f32, 1.6, 4.0, 1.6, 2.4];
        let total: f32 = widths.iter().sum();
        let unit = (w - 4.0 * 10.0) / total;
        let mut kx = x;
        for (i, label) in SPECIAL.iter().enumerate() {
            let kw2 = widths[i] * unit;
            let focused = self.row == 4 && self.col == i;
            let active = (i == 0 && self.shift) || (i == 1 && self.symbols);
            let bg = if focused {
                color::WHITE
            } else if i == 4 {
                color::ACCENT
            } else if active {
                Rgba::hex(0x3a3a42)
            } else {
                color::RAISED
            };
            c.rect(kx, yy, kw2, kh, 10.0, bg);
            let l = if i == 1 && self.symbols { "abc" } else { label };
            c.text(
                l,
                kx + kw2 / 2.0,
                yy + 13.0,
                20,
                Weight::Semi,
                if focused { color::BLACK } else { color::TEXT },
                Align::Center,
            );
            kx += kw2 + 10.0;
        }
        yy + kh - y
    }
}

/// A simple vertical list with a focus bar.
pub struct ListView {
    pub sel: usize,
    pub scroll: Anim,
    /// the highlight bar slides to the selected row
    bar: Anim,
}

impl ListView {
    pub fn new() -> ListView {
        ListView {
            sel: 0,
            scroll: Anim::default(),
            bar: Anim::default(),
        }
    }
    pub fn moving(&self) -> bool {
        self.scroll.moving() || self.bar.moving()
    }
    pub fn key(&mut self, k: Key, n: usize) -> bool {
        match k {
            Key::Up if self.sel > 0 => self.sel -= 1,
            Key::Down if self.sel + 1 < n => self.sel += 1,
            Key::ChUp => self.sel = self.sel.saturating_sub(8),
            Key::ChDown if n > 0 => self.sel = (self.sel + 8).min(n - 1),
            _ => return false,
        }
        true
    }
    /// Calls `row(c, i, x, y, focused)` for the visible rows.
    pub fn draw(
        &mut self,
        c: &mut Canvas,
        n: usize,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        rh: f32,
        focused_list: bool,
        mut row: impl FnMut(&mut Canvas, usize, f32, f32, bool),
    ) {
        if n == 0 {
            return;
        }
        self.sel = self.sel.min(n - 1);
        let visible = (h / rh).floor().max(1.0);
        let target =
            (self.sel as f32 - visible / 2.0 + 0.5).clamp(0.0, (n as f32 - visible).max(0.0)) * rh;
        self.scroll.set(target);
        let s = self.scroll.step();
        let first = (s / rh).floor() as usize;
        // The bar glides; the row's text turns dark once the bar is mostly there
        let target_bar = self.sel as f32 * rh;
        if (self.bar.v - target_bar).abs() > rh * 6.0 {
            self.bar.jump(target_bar); // long jumps (page keys) don't fly across the list
        }
        self.bar.set(target_bar);
        let b = self.bar.step();
        // rows sliding in or out stay inside the list's box instead of
        // running over the title above or the footer below
        c.clip_y(Some((y, y + h)));
        if focused_list {
            let by = y + b - s;
            if by > y - rh && by < y + h {
                c.rect(x, by, w, rh - 6.0, 12.0, color::WHITE.a(235));
            }
        }
        for i in first..(first + visible as usize + 2).min(n) {
            let yy = y + i as f32 * rh - s;
            if yy < y - rh || yy > y + h {
                continue;
            }
            let f = focused_list && i == self.sel && (b - target_bar).abs() < rh * 0.5;
            row(c, i, x, yy, f);
        }
        c.clip_y(None);
    }
}
