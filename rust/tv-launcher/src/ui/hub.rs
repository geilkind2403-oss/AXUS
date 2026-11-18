//! The new Home ("hub"): a tab bar on top (For you, Live TV, Films, Apps,
//! News), and under it the tab's content. "For you" has the side panel with
//! clock, weather and what's new, a large "continue watching" card and a row
//! of apps. The other tabs show a preview of their app; OK or ▼ opens it.
//!
//! Laid out in fixed boxes so a focus change only touches the boxes involved.
use super::*;
use crate::app::{Ctx, Nav, Screen};
use crate::gfx::{Align, Canvas, Image, Rgba, Weight};
use std::sync::OnceLock;

const GOLD: Rgba = Rgba(226, 187, 116, 255);
const CREAM: Rgba = Rgba(250, 245, 234, 255);
const MUTED: Rgba = Rgba(196, 182, 156, 255);

// layout (logical 1280 x 720)
const TOP_H: f32 = 76.0;
const PAD: f32 = 28.0;
const Y0: f32 = 92.0;
const SIDE_W: f32 = 236.0;
const MAIN_X: f32 = PAD + SIDE_W + 20.0;
const MAIN_W: f32 = 1280.0 - PAD - MAIN_X;
const HERO_H: f32 = 372.0;
const ROW_LABEL_Y: f32 = Y0 + HERO_H + 22.0;
const ROW_Y: f32 = ROW_LABEL_Y + 34.0;
const BOTTOM: f32 = 720.0 - PAD;

const TABS: [&str; 5] = ["Für dich", "Live TV", "Filme", "Apps", "Nachrichten"];

#[derive(Clone, Copy, PartialEq)]
enum App {
    YouTube,
    Prime,
    Zdf,
    Arte,
    LiveTv,
    Films,
    Weather,
    News,
    Settings,
}

impl App {
    fn name(self) -> &'static str {
        match self {
            App::YouTube => "YouTube",
            App::Prime => "Prime Video",
            App::Zdf => "ZDF",
            App::Arte => "ARTE",
            App::LiveTv => "Live TV",
            App::Films => "Filme",
            App::Weather => "Wetter",
            App::News => "Nachrichten",
            App::Settings => "Einstellungen",
        }
    }
}

/// the row on "For you": the apps that are not tabs themselves
const ROW_APPS: [App; 4] = [App::YouTube, App::Prime, App::Zdf, App::Arte];
/// the Apps tab: everything
const ALL_APPS: [App; 9] = [
    App::YouTube,
    App::Prime,
    App::Zdf,
    App::Arte,
    App::LiveTv,
    App::Films,
    App::Weather,
    App::News,
    App::Settings,
];

#[derive(Clone, Copy, PartialEq, Debug)]
enum HubFocus {
    /// 0..5 tabs, 5 search, 6 settings
    Bar(usize),
    Weather,
    Resume,
    Restart,
    Row(usize),
    Grid(usize),
}

pub struct Hub {
    tab: usize,
    focus: HubFocus,
    /// PANES_DONE when last drawn: a newly finished pane means a redraw
    panes_seen: u64,
}

fn hero_image() -> Option<Image> {
    static HERO: OnceLock<Option<Image>> = OnceLock::new();
    HERO.get_or_init(|| {
        let s = crate::gfx::RENDER_SCALE;
        let mut img = crate::gfx::decode_cover_image(
            include_bytes!("../../assets/home/hero.jpg"),
            (MAIN_W * s) as u32,
            (HERO_H * s) as u32,
        )?;
        // darker towards the left so the text reads: baked in once, so it
        // costs nothing per frame (and has no visible steps)
        let pm = std::sync::Arc::make_mut(&mut img);
        let w = pm.width() as usize;
        let data = pm.data_mut();
        let k: Vec<u32> = (0..w)
            .map(|x| {
                let t = (x as f32 / w as f32 / 0.65).min(1.0);
                let dark = 0.72 * (1.0 - t * t * (3.0 - 2.0 * t));
                ((1.0 - dark) * 256.0) as u32
            })
            .collect();
        for row in data.chunks_exact_mut(w * 4) {
            for (px, k) in row.chunks_exact_mut(4).zip(&k) {
                for ch in &mut px[..3] {
                    *ch = ((*ch as u32 * k) >> 8) as u8;
                }
            }
        }
        Some(img)
    })
    .clone()
}

fn app_icon(app: App) -> Option<Image> {
    static ICONS: OnceLock<[Option<Image>; 3]> = OnceLock::new();
    let icons = ICONS.get_or_init(|| {
        let px = (64.0 * crate::gfx::RENDER_SCALE) as u32;
        [
            include_bytes!("../../assets/apps/youtube.png").as_slice(),
            include_bytes!("../../assets/apps/prime.png").as_slice(),
            include_bytes!("../../assets/apps/zdf.png").as_slice(),
        ]
        .map(|b| crate::gfx::decode_image(b, px, px))
    });
    match app {
        App::YouTube => icons[0].clone(),
        App::Prime => icons[1].clone(),
        App::Zdf => icons[2].clone(),
        _ => None,
    }
}

/// what "continue watching" shows: a film to resume, else the last channel
enum Resume {
    Film(crate::films::Film),
    Channel(String),
}

fn resume_item(cx: &Ctx) -> Resume {
    let film = cx.films.list.with(|list| {
        let l = list.as_ref()?;
        l.films
            .iter()
            .find(|f| {
                f.key == cx.cfg.last_film_key
                    && f.position > 60.0
                    && f.duration > 0.0
                    && f.position < f.duration - 120.0
            })
            .cloned()
    });
    match film {
        Some(f) => Resume::Film(f),
        None => Resume::Channel(cx.cfg.livetv_last.clone()),
    }
}

impl Hub {
    pub fn new() -> Hub {
        Hub {
            tab: 0,
            focus: HubFocus::Bar(0),
            panes_seen: 0,
        }
    }

    fn open_app(app: App, cx: &mut Ctx) -> Nav {
        let browser = super::browser::installed();
        match app {
            App::YouTube if browser => Nav::Push(Box::new(super::browser::Browser::new(
                super::browser::YOUTUBE_TV,
            ))),
            App::YouTube => Nav::Push(Box::new(super::youtube::YouTube::new(cx))),
            App::Prime if browser => Nav::Push(Box::new(super::browser::Browser::new(
                super::browser::PRIME_VIDEO,
            ))),
            App::Prime => {
                cx.toast("Prime Video braucht das Browser-Addon");
                Nav::None
            }
            App::Zdf => Nav::Push(Box::new(super::livetv::LiveTv::new_zdf(cx))),
            App::Arte => Nav::Push(Box::new(super::livetv::LiveTv::new_arte(cx))),
            App::LiveTv => Nav::Push(Box::new(super::livetv::LiveTv::new(cx))),
            App::Films => Nav::Push(Box::new(super::films::Films::new(cx))),
            App::Weather => Nav::Push(Box::new(super::weather::WeatherScreen::new())),
            App::News => Nav::Push(Box::new(super::news::NewsScreen::new())),
            App::Settings => Nav::Push(Box::new(super::settings::Settings::new(cx))),
        }
    }

    /// OK or ▼ on a tab other than "For you" / "Apps" opens its app
    fn open_tab(&self, cx: &mut Ctx) -> Nav {
        match self.tab {
            1 => Self::open_app(App::LiveTv, cx),
            2 => Self::open_app(App::Films, cx),
            4 => Self::open_app(App::News, cx),
            _ => Nav::None,
        }
    }

    fn resume(&self, cx: &mut Ctx) -> Nav {
        match resume_item(cx) {
            Resume::Film(f) => Nav::Push(Box::new(super::films::FilmPage::new(f))),
            Resume::Channel(_) => Self::open_app(App::LiveTv, cx),
        }
    }
}

const GRID_COLS: usize = 5;

impl Screen for Hub {
    fn name(&self) -> &'static str {
        "home"
    }

    fn key(&mut self, k: Key, cx: &mut Ctx) -> Nav {
        use HubFocus::*;
        let f = self.focus;
        match (f, k) {
            // the bar: ◀▶ moves (and switches the tab at once), ▼ goes in
            (Bar(i), Key::Left) if i > 0 => {
                self.focus = Bar(i - 1);
                if i - 1 < 5 {
                    self.tab = i - 1;
                }
            }
            (Bar(i), Key::Right) if i < 6 => {
                self.focus = Bar(i + 1);
                if i + 1 < 5 {
                    self.tab = i + 1;
                }
            }
            (Bar(5), Key::Ok) => return Self::open_app(App::YouTube, cx),
            (Bar(6), Key::Ok) => return Self::open_app(App::Settings, cx),
            (Bar(i), Key::Down | Key::Ok) if i < 5 => match self.tab {
                0 => self.focus = Resume,
                3 => self.focus = Grid(0),
                _ => return self.open_tab(cx),
            },
            (Bar(_), _) => {}
            // For you
            (Weather, Key::Right) => self.focus = Resume,
            (Weather, Key::Up) => self.focus = Bar(self.tab),
            (Weather, Key::Down) => self.focus = Row(0),
            (Weather, Key::Ok) => return Self::open_app(App::Weather, cx),
            (Resume, Key::Left) => self.focus = Weather,
            (Resume, Key::Right) => self.focus = Restart,
            (Restart, Key::Left) => self.focus = Resume,
            (Resume | Restart, Key::Up) => self.focus = Bar(self.tab),
            (Resume | Restart, Key::Down) => self.focus = Row(0),
            (Resume | Restart, Key::Ok) => return self.resume(cx),
            (Row(0), Key::Left) => self.focus = Weather,
            (Row(i), Key::Left) => self.focus = Row(i - 1),
            (Row(i), Key::Right) if i + 1 < ROW_APPS.len() => self.focus = Row(i + 1),
            (Row(_), Key::Up) => self.focus = Resume,
            (Row(i), Key::Ok) => return Self::open_app(ROW_APPS[i], cx),
            // Apps tab
            (Grid(i), Key::Left) if i % GRID_COLS > 0 => self.focus = Grid(i - 1),
            (Grid(i), Key::Right) if i % GRID_COLS + 1 < GRID_COLS && i + 1 < ALL_APPS.len() => {
                self.focus = Grid(i + 1)
            }
            (Grid(i), Key::Up) if i >= GRID_COLS => self.focus = Grid(i - GRID_COLS),
            (Grid(_), Key::Up) => self.focus = Bar(self.tab),
            (Grid(i), Key::Down) if i + GRID_COLS < ALL_APPS.len() => {
                self.focus = Grid(i + GRID_COLS)
            }
            (Grid(i), Key::Ok) => return Self::open_app(ALL_APPS[i], cx),
            (_, Key::Back) => self.focus = Bar(self.tab),
            _ => {}
        }
        Nav::None
    }

    fn animating(&self, _cx: &Ctx) -> bool {
        // while glass panes are being made, and once more when one is done
        panes_busy()
            || PANES_DONE.load(std::sync::atomic::Ordering::Acquire) != self.panes_seen
    }

    fn draw(&mut self, c: &mut Canvas, cx: &mut Ctx) {
        self.panes_seen = PANES_DONE.load(std::sync::atomic::Ordering::Acquire);
        match scene() {
            Some(bg) => c.screen(&bg),
            None => c.ambient(crate::gfx::Ambient::LiquidGold),
        }
        self.draw_bar(c);
        match self.tab {
            0 => self.draw_for_you(c, cx),
            1 => draw_livetv_preview(c, cx),
            2 => draw_films_preview(c, cx),
            3 => self.draw_apps(c),
            _ => draw_news_preview(c, cx),
        }
    }
}

/// gold ring and light for the focused box
fn focus_ring(c: &mut Canvas, x: f32, y: f32, w: f32, h: f32, r: f32) {
    c.rect(x - 4.0, y - 4.0, w + 8.0, h + 8.0, r + 4.0, Rgba(226, 187, 116, 34));
    c.stroke_rect(x - 1.0, y - 1.0, w + 2.0, h + 2.0, r + 1.0, 2.5, Rgba(240, 204, 136, 235));
}

/// Liquid glass. Each pane is made once from the real picture behind it:
/// that picture, frosted (blurred), bent towards the middle near the edges
/// like the rim of a lens, lightly tinted, with a bright rim where the light
/// from the top left catches it and a shade on the opposite edge. Per frame
/// a pane is a copy, so it costs no more than a plain fill.
fn panel(c: &mut Canvas, x: f32, y: f32, w: f32, h: f32, r: f32, focused: bool) {
    // the pane image carries its shadow and dark edge in a margin around it:
    // one copy per pane, no shapes drawn on top
    match glass_pane(x, y, w, h, r, focused) {
        Some(img) => c.image_native(&img, x - SHADOW, y - SHADOW),
        // not made yet: a plain dark pane until it is
        None => {
            c.rect(x, y, w, h, r, Rgba(24, 19, 14, 190));
            c.stroke_rect(x + 0.5, y + 0.5, w - 1.0, h - 1.0, r, 1.0, Rgba(255, 238, 205, 60));
        }
    }
    if focused {
        focus_ring(c, x, y, w, h, r);
    }
}

/// The Home background: the hero picture over the whole screen, darkened so
/// text stays readable (darker towards the bottom).
fn scene() -> Option<Image> {
    static SCENE: OnceLock<Option<Image>> = OnceLock::new();
    SCENE
        .get_or_init(|| {
            let s = crate::gfx::RENDER_SCALE;
            let (w, h) = ((1280.0 * s) as u32, (720.0 * s) as u32);
            let img = image::load_from_memory(include_bytes!("../../assets/home/hero.jpg")).ok()?;
            let img = img.resize_to_fill(w, h, image::imageops::FilterType::Triangle).to_rgb8();
            let mut pm = tiny_skia::Pixmap::new(w, h)?;
            let out = pm.data_mut();
            for (i, (o, p)) in out.chunks_exact_mut(4).zip(img.pixels()).enumerate() {
                let yy = (i as u32 / w) as f32 / h as f32;
                let k = 0.70 - 0.26 * yy;
                o[0] = (p[2] as f32 * k) as u8;
                o[1] = (p[1] as f32 * k) as u8;
                o[2] = (p[0] as f32 * k) as u8;
                o[3] = 255;
            }
            Some(std::sync::Arc::new(pm))
        })
        .clone()
}

type PaneKey = (i32, i32, i32, i32, i32, bool);

fn panes() -> &'static std::sync::Mutex<std::collections::HashMap<PaneKey, Image>> {
    static PANES: OnceLock<std::sync::Mutex<std::collections::HashMap<PaneKey, Image>>> = OnceLock::new();
    PANES.get_or_init(Default::default)
}

/// panes being made on a worker, and a counter that moves when one is done
fn pending() -> &'static std::sync::Mutex<std::collections::HashSet<PaneKey>> {
    static P: OnceLock<std::sync::Mutex<std::collections::HashSet<PaneKey>>> = OnceLock::new();
    P.get_or_init(Default::default)
}
static PANES_DONE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A pane, made (and kept) on first use.
fn glass_pane_now(x: f32, y: f32, w: f32, h: f32, r: f32, focused: bool) -> Option<Image> {
    let key = (x as i32, y as i32, w as i32, h as i32, r as i32, focused);
    if let Some(img) = panes().lock().ok()?.get(&key).cloned() {
        return Some(img);
    }
    let bg = scene()?;
    let img = build_pane(&bg, x, y, w, h, r, focused)?;
    panes().lock().ok()?.insert(key, img.clone());
    PANES_DONE.fetch_add(1, std::sync::atomic::Ordering::Release);
    Some(img)
}

/// A pane if it is ready; else it is started on a worker and None comes
/// back (the caller draws a plain pane for now): a tab is never held up.
fn glass_pane(x: f32, y: f32, w: f32, h: f32, r: f32, focused: bool) -> Option<Image> {
    let key = (x as i32, y as i32, w as i32, h as i32, r as i32, focused);
    if let Some(img) = panes().lock().ok()?.get(&key).cloned() {
        return Some(img);
    }
    if pending().lock().ok()?.insert(key) {
        std::thread::spawn(move || {
            let _ = glass_pane_now(x, y, w, h, r, focused);
            if let Ok(mut p) = pending().lock() {
                p.remove(&key);
            }
        });
    }
    None
}

fn panes_busy() -> bool {
    pending().lock().map(|p| !p.is_empty()).unwrap_or(false)
}

/// Makes the background and the glass panes of "For you" on a worker at
/// start, so Home's first frame is mostly copies (on a 400 MHz Pi a pane
/// takes a while to make).
pub fn prewarm() {
    std::thread::spawn(|| {
        let _ = hero_image();
        let _ = scene();
        let n = ROW_APPS.len() as f32;
        let gap = 18.0;
        let tw = (MAIN_W - gap * (n - 1.0)) / n;
        let th = BOTTOM - ROW_Y;
        let mut jobs = vec![(PAD, Y0, SIDE_W, BOTTOM - Y0, 22.0, false)];
        for i in 0..ROW_APPS.len() {
            let tx = MAIN_X + i as f32 * (tw + gap);
            jobs.push((tx, ROW_Y, tw, th, 18.0, false));
            jobs.push((tx, ROW_Y, tw, th, 18.0, true));
        }
        for (x, y, w, h, r, f) in jobs {
            let _ = glass_pane_now(x, y, w, h, r, f);
        }
    });
}

/// Box blur, three passes (close to a gaussian), on a BGRA buffer.
fn blur(buf: &mut [u8], w: usize, h: usize, radius: usize) {
    fn pass(from: &[u8], to: &mut [u8], len: usize, stride: usize, lines: usize, step: usize, r: usize) {
        let r = r.min(len.saturating_sub(1)).max(1);
        let div = (2 * r + 1) as u32;
        for line in 0..lines {
            let base = line * step;
            let at = |i: isize| base + (i.clamp(0, len as isize - 1) as usize) * stride;
            for ch in 0..4 {
                let mut sum: u32 = (-(r as isize)..=(r as isize)).map(|i| from[at(i) + ch] as u32).sum();
                for i in 0..len as isize {
                    to[base + i as usize * stride + ch] = (sum / div) as u8;
                    sum += from[at(i + r as isize + 1) + ch] as u32;
                    sum -= from[at(i - r as isize) + ch] as u32;
                }
            }
        }
    }
    let mut tmp = vec![0u8; buf.len()];
    for _ in 0..3 {
        pass(buf, &mut tmp, w, 4, h, w * 4, radius);
        pass(&tmp, buf, h, w * 4, w, 4, radius);
    }
}

/// the margin around a pane image for its shadow (logical px)
const SHADOW: f32 = 18.0;

fn build_pane(scene: &tiny_skia::Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, focused: bool) -> Option<Image> {
    let s = crate::gfx::RENDER_SCALE;
    let sm = (SHADOW * s).round() as usize;
    let (pw, ph) = ((w * s).round() as usize, (h * s).round() as usize);
    let (x0, y0) = ((x * s).round() as isize, (y * s).round() as isize);
    let (sw, sh) = (scene.width() as isize, scene.height() as isize);
    let edge = 24.0 * s; // the bending rim
    let bend = 46.0 * s; // how far the rim pulls the picture in
    let m = (bend + 4.0 * s) as isize; // margin read around the pane
    let (bw, bh) = (pw + 2 * m as usize, ph + 2 * m as usize);
    // the picture behind the pane (with a margin), frosted
    let src = scene.data();
    let mut back = vec![0u8; bw * bh * 4];
    for by in 0..bh {
        let sy = (y0 - m + by as isize).clamp(0, sh - 1) as usize;
        for bx in 0..bw {
            let sx = (x0 - m + bx as isize).clamp(0, sw - 1) as usize;
            let si = (sy * sw as usize + sx) * 4;
            back[(by * bw + bx) * 4..][..4].copy_from_slice(&src[si..si + 4]);
        }
    }
    blur(&mut back, bw, bh, (6.0 * s) as usize);

    let (ow, oh) = (pw + 2 * sm, ph + 2 * sm);
    let mut pm = tiny_skia::Pixmap::new(ow as u32, oh as u32)?;
    let out = pm.data_mut();
    let rr = (r * s).min(pw as f32 / 2.0).min(ph as f32 / 2.0);
    let (hx, hy) = (pw as f32 / 2.0 - rr, ph as f32 / 2.0 - rr);
    let sdf = |px: f32, py: f32| {
        let qx = px.abs() - hx;
        let qy = py.abs() - hy;
        qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - rr
    };
    // shadow and dark edge, in the margin and under the pane
    let drop = 6.0 * s;
    let soft = 9.0 * s;
    for j in 0..oh {
        for i in 0..ow {
            let px = i as f32 + 0.5 - sm as f32 - pw as f32 / 2.0;
            let py = j as f32 + 0.5 - sm as f32 - ph as f32 / 2.0;
            let d = sdf(px, py - drop).max(0.0);
            let mut a = 0.50 * (-(d / soft) * (d / soft)).exp();
            let e = sdf(px, py);
            if e > -0.5 && e < 1.6 * s {
                a = a.max(0.45 * (1.0 - (e / (1.6 * s)).clamp(0.0, 1.0)));
            }
            out[(j * ow + i) * 4 + 3] = (a * 255.0) as u8; // black: only alpha
        }
    }
    let (lx, ly) = (-0.55f32, -0.83f32); // light from the top left
    let (tint, ta) = if focused {
        ([60.0, 128.0, 190.0], 0.22) // gold (BGR)
    } else {
        ([14.0, 20.0, 30.0], 0.22)
    };
    let lift = if focused { 6.0 } else { 0.0 };
    for j in 0..ph {
        for i in 0..pw {
            let px = i as f32 + 0.5 - pw as f32 / 2.0;
            let py = j as f32 + 0.5 - ph as f32 / 2.0;
            let qx = px.abs() - hx;
            let qy = py.abs() - hy;
            let outside = sdf(px, py);
            let cover = (0.5 - outside).clamp(0.0, 1.0);
            if cover <= 0.0 {
                continue;
            }
            let depth = -outside; // distance in from the edge
            // outward normal of the nearest edge
            let (nx, ny) = if qx > 0.0 && qy > 0.0 {
                let l = qx.hypot(qy).max(1e-3);
                (qx / l * px.signum(), qy / l * py.signum())
            } else if qx > qy {
                (px.signum(), 0.0)
            } else {
                (0.0, py.signum())
            };
            // lens: near the rim the picture is pulled in from further out
            let t = (1.0 - depth / edge).clamp(0.0, 1.0);
            let pull = t * t * t * bend;
            let sx = (m as f32 + i as f32 - nx * pull).clamp(0.0, (bw - 1) as f32) as usize;
            let sy = (m as f32 + j as f32 - ny * pull).clamp(0.0, (bh - 1) as f32) as usize;
            let b = &back[(sy * bw + sx) * 4..][..3];
            // light: a bright rim facing the light, a shade on the far side,
            // and a soft inner glow all along the rim
            let facing = -(nx * lx + ny * ly);
            let rim = (1.0 - depth / (1.8 * s)).clamp(0.0, 1.0);
            // only the rim catches light: no haze over the pane. A ground
            // bevel just inside it gives the glass its thickness: lit where it
            // faces the light, in shade on the far side
            let bevel = (1.0 - depth / (8.0 * s)).clamp(0.0, 1.0).powi(2);
            let spec = rim * (0.20 + 0.75 * facing.max(0.0))
                + bevel * 0.30 * facing.max(0.0)
                + t.powi(6) * 0.10;
            let shade = rim * (-facing).max(0.0) * 0.35 + bevel * 0.45 * (-facing).max(0.0);
            let o = ((j + sm) * ow + i + sm) * 4;
            let under = out[o + 3] as f32 / 255.0; // shadow already there
            let a = cover + under * (1.0 - cover);
            for ch in 0..3 {
                let v = b[ch] as f32 * (1.0 - ta) + tint[ch] * ta + lift;
                let v = v + (255.0 - v) * spec - v * shade;
                out[o + ch] = (v.clamp(0.0, 255.0) * cover) as u8;
            }
            out[o + 3] = (a * 255.0) as u8;
        }
    }
    Some(std::sync::Arc::new(pm))
}

impl Hub {
    fn draw_bar(&self, c: &mut Canvas) {
        // logo
        c.text("AXUS", PAD + 6.0, 26.0, 28, Weight::Bold, CREAM, Align::Left);
        c.text("TV", PAD + 92.0, 26.0, 28, Weight::Regular, MUTED, Align::Left);
        // tabs, in one glass capsule
        let widths: Vec<f32> = TABS.iter().map(|n| c.text_width(n, 19, Weight::Semi) + 44.0).collect();
        let total: f32 = widths.iter().sum::<f32>() + 14.0 * (TABS.len() as f32 - 1.0);
        panel(c, 282.0, 16.0, total + 16.0, 52.0, 26.0, false);
        let mut x = 290.0;
        for (i, name) in TABS.iter().enumerate() {
            let w = c.text_width(name, 19, Weight::Semi) + 44.0;
            let active = self.tab == i;
            let focused = self.focus == HubFocus::Bar(i);
            if active {
                c.vgradient(
                    x,
                    22.0,
                    w,
                    40.0,
                    20.0,
                    Rgba(240, 204, 136, if focused { 255 } else { 210 }),
                    Rgba(196, 150, 78, if focused { 255 } else { 210 }),
                );
            }
            if focused {
                focus_ring(c, x, 22.0, w, 40.0, 20.0);
            }
            c.text(
                name,
                x + w / 2.0,
                31.0,
                19,
                Weight::Semi,
                if active { Rgba(40, 28, 12, 255) } else { CREAM },
                Align::Center,
            );
            x += w + 14.0;
        }
        // search and settings
        for (i, cxp) in [(5usize, 1166.0f32), (6, 1222.0)] {
            let focused = self.focus == HubFocus::Bar(i);
            panel(c, cxp - 24.0, 18.0, 48.0, 48.0, 24.0, focused);
            if i == 5 {
                icon_search(c, cxp, 42.0, 18.0, CREAM);
            } else {
                icon_gear(c, cxp, 42.0, 20.0, CREAM);
            }
        }
        c.line(&[(0.0, TOP_H), (1280.0, TOP_H)], 1.0, Rgba(255, 236, 200, 22));
    }

    fn draw_for_you(&self, c: &mut Canvas, cx: &mut Ctx) {
        // side panel: clock, date, weather, what's new
        let (sx, sw, sh) = (PAD, SIDE_W, BOTTOM - Y0);
        panel(c, sx, Y0, sw, sh, 22.0, false);
        let tm = crate::app::local_tm();
        c.text(
            &format!("{:02}:{:02}", tm.tm_hour, tm.tm_min),
            sx + 24.0,
            Y0 + 22.0,
            52,
            Weight::Semi,
            CREAM,
            Align::Left,
        );
        c.text(
            &format!(
                "{}, {}. {}",
                crate::app::WEEKDAYS[tm.tm_wday.clamp(0, 6) as usize],
                tm.tm_mday,
                crate::app::MONTHS[tm.tm_mon.clamp(0, 11) as usize]
            ),
            sx + 24.0,
            Y0 + 88.0,
            15,
            Weight::Regular,
            MUTED,
            Align::Left,
        );
        c.line(&[(sx + 24.0, Y0 + 124.0), (sx + sw - 24.0, Y0 + 124.0)], 1.0, Rgba(255, 236, 200, 30));
        // weather (focusable: opens the weather app)
        let wy = Y0 + 136.0;
        if self.focus == HubFocus::Weather {
            focus_ring(c, sx + 10.0, wy, sw - 20.0, 92.0, 16.0);
            c.rect(sx + 10.0, wy, sw - 20.0, 92.0, 16.0, Rgba(92, 70, 34, 110));
        }
        cx.weather.with(|w| match w {
            Some(Ok(w)) => {
                let (sky, text) = crate::weather::describe(w.code);
                icon_weather(c, sky, w.day, sx + 52.0, wy + 46.0, 52.0);
                c.text(&format!("{:.0}°", w.temp), sx + 92.0, wy + 14.0, 38, Weight::Semi, CREAM, Align::Left);
                c.text_fit(text, sx + 92.0, wy + 62.0, 13, Weight::Regular, MUTED, sw - 110.0);
            }
            Some(Err(_)) => {
                c.text("Wetter", sx + 24.0, wy + 22.0, 20, Weight::Semi, CREAM, Align::Left);
                c.text("Keine Verbindung", sx + 24.0, wy + 54.0, 14, Weight::Regular, MUTED, Align::Left);
            }
            None => {
                c.text("Wetter", sx + 24.0, wy + 22.0, 20, Weight::Semi, CREAM, Align::Left);
                c.text("Wird geladen …", sx + 24.0, wy + 54.0, 14, Weight::Regular, MUTED, Align::Left);
            }
        });
        c.line(&[(sx + 24.0, wy + 106.0), (sx + sw - 24.0, wy + 106.0)], 1.0, Rgba(255, 236, 200, 30));
        // what's new: the newest headlines (and later: updates)
        let ny = wy + 122.0;
        c.text("Neuigkeiten", sx + 24.0, ny, 18, Weight::Semi, CREAM, Align::Left);
        let items: Vec<(String, String)> = cx.news.with(|n| {
            n.map(|n| {
                n.iter()
                    .take(3)
                    .map(|i| (i.title.clone(), if i.topline.is_empty() { "Tagesschau".into() } else { i.topline.clone() }))
                    .collect()
            })
            .unwrap_or_default()
        });
        let mut y = ny + 38.0;
        if items.is_empty() {
            c.text("Noch keine Meldungen", sx + 24.0, y, 13, Weight::Regular, MUTED, Align::Left);
        }
        for (title, top) in items {
            c.rect(sx + 24.0, y + 2.0, 30.0, 30.0, 8.0, Rgba(255, 236, 200, 22));
            icon_news(c, sx + 39.0, y + 17.0, 16.0, GOLD);
            c.text_fit(&title, sx + 64.0, y, 13, Weight::Semi, CREAM, sw - 84.0);
            c.text_fit(&top, sx + 64.0, y + 19.0, 11, Weight::Regular, MUTED, sw - 84.0);
            y += 56.0;
            if y > BOTTOM - 40.0 {
                break;
            }
        }

        // continue watching
        let (hx, hy, hw, hh) = (MAIN_X, Y0, MAIN_W, HERO_H);
        c.rect(hx, hy, hw, hh, 22.0, Rgba(24, 18, 12, 255));
        if let Some(img) = hero_image() {
            c.image_cover(&img, hx, hy, hw, hh, 22.0);
        }
        c.vgradient(hx, hy + hh * 0.5, hw, hh * 0.5, 22.0, Rgba(0, 0, 0, 0), Rgba(0, 0, 0, 70));
        c.stroke_rect(hx + 0.5, hy + 0.5, hw - 1.0, hh - 1.0, 22.0, 1.0, Rgba(255, 236, 200, 40));
        let (title, sub) = match resume_item(cx) {
            Resume::Film(f) => (
                f.title.clone(),
                format!("Noch {} · Filme", crate::app::fmt_time(f.duration - f.position)),
            ),
            Resume::Channel(ch) if !ch.is_empty() => ("Live TV".to_string(), format!("Zuletzt gesehen · {ch}")),
            Resume::Channel(_) => ("Live TV".to_string(), "Deine Sender".to_string()),
        };
        c.text("Weitersehen", hx + 40.0, hy + 132.0, 26, Weight::Regular, CREAM, Align::Left);
        c.text_fit(&title, hx + 38.0, hy + 166.0, 56, Weight::Bold, CREAM, hw * 0.6);
        c.text_fit(&sub, hx + 40.0, hy + 238.0, 17, Weight::Regular, Rgba(235, 222, 198, 255), hw * 0.6);
        // buttons
        let (bx, by, bw, bh) = (hx + 40.0, hy + 286.0, 188.0, 48.0);
        let resume_f = self.focus == HubFocus::Resume;
        c.vgradient(bx, by, bw, bh, 24.0, Rgba(244, 210, 142, 255), Rgba(200, 152, 78, 255));
        if resume_f {
            focus_ring(c, bx, by, bw, bh, 24.0);
        }
        icon_play(c, bx + 40.0, by + bh / 2.0, 16.0, Rgba(40, 28, 12, 255));
        c.text("Fortsetzen", bx + 62.0, by + 13.0, 18, Weight::Semi, Rgba(40, 28, 12, 255), Align::Left);
        let rx = bx + bw + 22.0;
        let restart_f = self.focus == HubFocus::Restart;
        if restart_f {
            c.rect(rx - 6.0, by, 220.0, bh, 24.0, Rgba(255, 236, 200, 40));
            focus_ring(c, rx - 6.0, by, 220.0, bh, 24.0);
        }
        c.line(&arc_open(rx + 18.0, by + bh / 2.0, 10.0), 1.8, CREAM);
        c.text("Von vorne starten", rx + 38.0, by + 14.0, 17, Weight::Regular, CREAM, Align::Left);

        // apps row
        c.text("Apps", MAIN_X + 4.0, ROW_LABEL_Y, 22, Weight::Semi, CREAM, Align::Left);
        let n = ROW_APPS.len() as f32;
        let gap = 18.0;
        let tw = (MAIN_W - gap * (n - 1.0)) / n;
        let th = BOTTOM - ROW_Y;
        for (i, app) in ROW_APPS.iter().enumerate() {
            let tx = MAIN_X + i as f32 * (tw + gap);
            draw_app_tile(c, *app, tx, ROW_Y, tw, th, self.focus == HubFocus::Row(i));
        }
    }

    fn draw_apps(&self, c: &mut Canvas) {
        let gap = 20.0;
        let cols = GRID_COLS as f32;
        let w = (1280.0 - 2.0 * PAD - gap * (cols - 1.0)) / cols;
        let rows = ALL_APPS.len().div_ceil(GRID_COLS) as f32;
        let h = (BOTTOM - 30.0 - Y0 - gap * (rows - 1.0)) / rows;
        for (i, app) in ALL_APPS.iter().enumerate() {
            let (col, row) = ((i % GRID_COLS) as f32, (i / GRID_COLS) as f32);
            let x = PAD + col * (w + gap);
            let y = Y0 + row * (h + gap);
            draw_app_tile(c, *app, x, y, w, h, self.focus == HubFocus::Grid(i));
        }
        hint(c, "◀▶▲▼ Auswählen  ·  OK Öffnen");
    }
}

fn hint(c: &mut Canvas, s: &str) {
    c.text(s, 1280.0 - PAD, 720.0 - PAD - 6.0, 13, Weight::Semi, Rgba(190, 160, 104, 230), Align::Right);
}

fn draw_app_tile(c: &mut Canvas, app: App, x: f32, y: f32, w: f32, h: f32, focused: bool) {
    panel(c, x, y, w, h, 18.0, focused);
    let (icx, icy) = (x + w / 2.0, y + h * 0.42);
    match app_icon(app) {
        Some(img) => {
            let s = 58.0;
            c.image_fit(&img, icx - s / 2.0, icy - s / 2.0, s, s, 12.0, false, 1.0);
        }
        None => match app {
            App::Arte => {
                c.text("ARTE", icx, icy - 16.0, 26, Weight::Bold, Rgba(255, 120, 40, 255), Align::Center);
            }
            App::LiveTv => {
                c.stroke_rect(icx - 26.0, icy - 18.0, 52.0, 34.0, 6.0, 2.5, GOLD);
                c.line(&[(icx - 12.0, icy + 24.0), (icx + 12.0, icy + 24.0)], 2.5, GOLD);
            }
            App::Films => icon_film(c, icx, icy, 50.0, GOLD),
            App::Weather => icon_weather(c, crate::weather::Sky::PartlyCloudy, true, icx, icy, 54.0),
            App::News => icon_news(c, icx, icy, 46.0, GOLD),
            App::Settings => icon_gear(c, icx, icy, 46.0, GOLD),
            _ => {}
        },
    }
    c.text(app.name(), x + w / 2.0, y + h - 40.0, 18, Weight::Semi, CREAM, Align::Center);
}

fn draw_livetv_preview(c: &mut Canvas, cx: &mut Ctx) {
    let list = cx.channels.with(|c| match c {
        Some(Ok((list, _))) => Some(list.clone()),
        _ => None,
    });
    match list {
        Some(list) => {
            let favs = &cx.cfg.livetv_favs;
            let mut chans: Vec<&crate::livetv::Channel> =
                list.iter().filter(|ch| favs.contains(&ch.name)).collect();
            if chans.len() < 12 {
                chans.extend(list.iter().filter(|ch| !favs.contains(&ch.name)).take(12 - chans.len()));
            }
            let (cols, gap) = (4usize, 18.0);
            let w = (1280.0 - 2.0 * PAD - gap * 3.0) / cols as f32;
            let h = (BOTTOM - 30.0 - Y0 - gap * 2.0) / 3.0;
            for (i, ch) in chans.iter().take(12).enumerate() {
                let x = PAD + (i % cols) as f32 * (w + gap);
                let y = Y0 + (i / cols) as f32 * (h + gap);
                panel(c, x, y, w, h, 18.0, false);
                c.text(&format!("{}", ch.number), x + 22.0, y + 20.0, 30, Weight::Bold, GOLD, Align::Left);
                c.text_fit(&ch.name, x + 22.0, y + 66.0, 17, Weight::Semi, CREAM, w - 44.0);
            }
        }
        None => {
            c.text("Senderliste wird geladen …", PAD + 4.0, Y0 + 60.0, 18, Weight::Regular, MUTED, Align::Left);
        }
    }
    hint(c, "OK oder ▼ Live TV öffnen");
}

fn draw_films_preview(c: &mut Canvas, cx: &mut Ctx) {
    let films: Vec<crate::films::Film> =
        cx.films.list.with(|l| l.map(|l| l.films.iter().take(10).cloned().collect()).unwrap_or_default());
    if films.is_empty() {
        panel(c, PAD, Y0, 1280.0 - 2.0 * PAD, 200.0, 22.0, false);
        icon_film(c, PAD + 80.0, Y0 + 100.0, 64.0, GOLD);
        c.text("Noch keine Filme", PAD + 140.0, Y0 + 64.0, 24, Weight::Semi, CREAM, Align::Left);
        c.text(
            "USB-Stick einstecken, nach /storage/videos kopieren oder den PC-Server eintragen.",
            PAD + 140.0,
            Y0 + 104.0,
            15,
            Weight::Regular,
            MUTED,
            Align::Left,
        );
    }
    let (cols, gap) = (5usize, 18.0);
    let w = (1280.0 - 2.0 * PAD - gap * 4.0) / cols as f32;
    let h = w * 1.42;
    for (i, f) in films.iter().enumerate().take(cols) {
        let x = PAD + i as f32 * (w + gap);
        let y = Y0;
        panel(c, x, y, w, h, 16.0, false);
        let s = crate::gfx::RENDER_SCALE;
        if let Some(img) = (!f.icon.is_empty())
            .then(|| cx.images.get_cover(&f.icon, (w * s) as u32, (h * s) as u32))
            .flatten()
        {
            c.image_cover(&img, x, y, w, h, 16.0);
        } else {
            icon_film(c, x + w / 2.0, y + h / 2.0 - 20.0, 48.0, GOLD);
        }
        c.text_fit(&f.title, x + 4.0, y + h + 12.0, 15, Weight::Semi, CREAM, w - 8.0);
    }
    hint(c, "OK oder ▼ Filme öffnen");
}

fn draw_news_preview(c: &mut Canvas, cx: &mut Ctx) {
    let items: Vec<crate::news::NewsItem> =
        cx.news.with(|n| n.map(|n| n.iter().take(6).cloned().collect()).unwrap_or_default());
    let content_bottom = BOTTOM - 30.0;
    if items.is_empty() {
        panel(c, PAD, Y0, 1280.0 - 2.0 * PAD, 160.0, 22.0, false);
        icon_news(c, PAD + 70.0, Y0 + 80.0, 56.0, GOLD);
        c.text("Meldungen werden geladen …", PAD + 130.0, Y0 + 66.0, 20, Weight::Semi, CREAM, Align::Left);
        hint(c, "OK oder ▼ Nachrichten öffnen");
        return;
    }
    // left: the top story, photo above, headline and first sentence below
    let (lx, ly, lw) = (PAD, Y0, 640.0);
    let lh = content_bottom - ly;
    panel(c, lx, ly, lw, lh, 22.0, false);
    let top = &items[0];
    let s = crate::gfx::RENDER_SCALE;
    let (ix, iy, iw, ih) = (lx + 10.0, ly + 10.0, lw - 20.0, lh - 200.0);
    match (!top.image.is_empty())
        .then(|| cx.images.get_cover(&top.image, (iw * s) as u32, (ih * s) as u32))
        .flatten()
    {
        Some(img) => c.image_cover(&img, ix, iy, iw, ih, 16.0),
        None => {
            c.vgradient(ix, iy, iw, ih, 16.0, Rgba(84, 64, 34, 150), Rgba(46, 35, 22, 150));
            icon_news(c, ix + iw / 2.0, iy + ih / 2.0, 72.0, GOLD);
        }
    }
    let ty = iy + ih + 22.0;
    c.text_fit(&top.topline, lx + 28.0, ty, 13, Weight::Semi, GOLD, lw - 56.0);
    c.paragraph(&top.title, lx + 28.0, ty + 24.0, lw - 56.0, 24, Weight::Bold, CREAM, 30.0, 2);
    c.paragraph(&top.first_sentence, lx + 28.0, ty + 92.0, lw - 56.0, 14, Weight::Regular, MUTED, 20.0, 1);
    // right: the next headlines, rows filling the same height
    let rx = lx + lw + 20.0;
    let rw = 1280.0 - PAD - rx;
    let rest: Vec<&crate::news::NewsItem> = items.iter().skip(1).take(5).collect();
    let gap = 14.0;
    let n = rest.len().max(1) as f32;
    let rh = (lh - gap * (n - 1.0)) / n;
    for (i, it) in rest.iter().enumerate() {
        let y = ly + i as f32 * (rh + gap);
        panel(c, rx, y, rw, rh, 16.0, false);
        c.text_fit(&it.topline, rx + 22.0, y + rh / 2.0 - 26.0, 12, Weight::Semi, GOLD, rw - 44.0);
        c.text_fit(&it.title, rx + 22.0, y + rh / 2.0 - 6.0, 17, Weight::Semi, CREAM, rw - 44.0);
    }
    hint(c, "OK oder ▼ Nachrichten öffnen");
}

/// a circle open at the top right, for "start over"
fn arc_open(cx: f32, cy: f32, r: f32) -> Vec<(f32, f32)> {
    (0..=36)
        .map(|i| {
            let a = -0.6 + i as f32 / 36.0 * (std::f32::consts::TAU - 1.0);
            (cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

