//! The home screen: big tiles for what a TV is used for.
use super::*;
use crate::app::{Ctx, Nav, Screen};
use crate::gfx::{color, Align, Canvas, Image, LayerCache, Rgba, Weight};
use std::time::Instant;

pub struct Home {
    focus: usize,
    order: [usize; 6],
    editing: bool,
    moving: Option<usize>,
    edit_started: Option<Instant>,
    frame: Focus,
    tile_canvas: Canvas,
    tile_images: [Option<crate::gfx::Image>; 6],
    tile_image_sources: [Vec<String>; 6],
    tile_cache_keys: [Option<(u64, u64)>; 6],
    base_cache: LayerCache,
    base_image: Option<Image>,
    status_key: Option<(i32, i32, u64, u64, String, bool, usize, bool, [usize; 6])>,
    news_overlay_key: Option<u64>,
    last_focus_rect: Option<(f32, f32, f32, f32)>,
    /// the arrange-mode outlines are on the kept picture (wipe them next frame)
    edit_drawn: bool,
}

#[derive(Clone, Copy)]
struct Tile {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

/// Home's accent: the warm gold of Settings, Live TV and the fast menu.
const GOLD: Rgba = Rgba(226, 187, 116, 255);
/// Extra cached pixels around each card for its soft lift shadow.
const TILE_BLEED: f32 = 10.0;

const TILES: [Tile; 6] = [
    // Medium, large, small across the first row.
    Tile {
        x: 64.0,
        y: 130.0,
        w: 360.0,
        h: 244.0,
    },
    Tile {
        x: 448.0,
        y: 130.0,
        w: 520.0,
        h: 244.0,
    },
    Tile {
        x: 992.0,
        y: 130.0,
        w: 224.0,
        h: 244.0,
    },
    // Large, medium, small across the second row.
    Tile {
        x: 64.0,
        y: 398.0,
        w: 520.0,
        h: 244.0,
    },
    Tile {
        x: 608.0,
        y: 398.0,
        w: 360.0,
        h: 244.0,
    },
    Tile {
        x: 992.0,
        y: 398.0,
        w: 224.0,
        h: 244.0,
    },
];

// Header controls follow the weather and apps tiles, so their positions can
// change as the user rearranges the home screen.
const STATUS_DAMAGE: (f32, f32, f32, f32) = (64.0, 16.0, 1152.0, 52.0);

impl Home {
    pub fn new(last_media_tile: usize, saved_order: [u8; 6]) -> Home {
        let mut order = saved_order.map(|v| v as usize);
        let mut sorted = order;
        sorted.sort_unstable();
        let valid_permutation = sorted == [0, 1, 2, 3, 4, 5];
        let sizes_match = valid_permutation
            && order
                .iter()
                .enumerate()
                .all(|(slot, content)| tile_size(*content) == tile_size(slot));
        if !sizes_match {
            order = [0, 1, 2, 3, 4, 5];
        }
        Home {
            focus: last_media_tile.min(5),
            order,
            editing: false,
            moving: None,
            edit_started: None,
            frame: Focus::default(),
            // Rebuilding an artwork/news tile is the expensive path on Pi 3.
            // Use its cores for this small offscreen raster so the UI thread
            // spends less time blocked when a carousel item changes.
            tile_canvas: Canvas::new_scaled_with_threads(1, 1, crate::gfx::RENDER_SCALE, 3),
            tile_images: std::array::from_fn(|_| None),
            tile_image_sources: std::array::from_fn(|_| Vec::new()),
            tile_cache_keys: [None; 6],
            base_cache: LayerCache::default(),
            base_image: None,
            status_key: None,
            news_overlay_key: None,
            last_focus_rect: None,
            edit_drawn: false,
        }
    }

    /// Cache the six content tiles separately from the changing ambient and
    /// focus ring. The thumbnails and card artwork rarely change; keeping
    /// their rasterized pixels avoids redrawing every icon and label on each
    /// remote-button press.
    fn refresh_tile_cache(&mut self, cx: &mut Ctx, i: usize) {
        let t = TILES[i];
        let content = self.order[i];
        let sources = tile_image_sources(cx, content);
        self.tile_canvas.resize(
            (t.w + TILE_BLEED * 2.0) as u32,
            (t.h + TILE_BLEED * 2.0) as u32,
        );
        self.tile_canvas.offset = (TILE_BLEED - t.x, TILE_BLEED - t.y);
        self.tile_canvas.opacity = 1.0;
        self.tile_canvas.clear(Rgba(0, 0, 0, 0));
        draw_tile_content(&mut self.tile_canvas, cx, content, &t);
        self.tile_canvas.finish();
        self.tile_images[i] = Some(self.tile_canvas.snapshot());
        self.tile_image_sources[i] = sources;
        self.tile_cache_keys[i] = Some(tile_cache_key(cx, content, &self.tile_image_sources[i]));
    }

    fn open(&self, cx: &mut Ctx) -> Nav {
        match if self.focus < 6 {
            self.order[self.focus]
        } else {
            self.focus
        } {
            0 if super::browser::installed() => Nav::Push(Box::new(super::browser::Browser::new(
                super::browser::YOUTUBE_TV,
            ))),
            0 => Nav::Push(Box::new(super::youtube::YouTube::new(cx))),
            1 => Nav::Push(Box::new(super::livetv::LiveTv::new(cx))),
            2 => match films_pick(cx).filter(|f| f.position > 60.0) {
                Some(f) => Nav::Push(Box::new(super::films::FilmPage::new(f))),
                None => Nav::Push(Box::new(super::films::Films::new(cx))),
            },
            3 => Nav::Push(Box::new(super::weather::WeatherScreen::new())),
            4 => Nav::Push(Box::new(super::news::NewsScreen::new())),
            _ => Nav::Push(Box::new(super::settings::Settings::new(cx))),
        }
    }
}

fn label(c: &mut Canvas, t: &Tile, s: &str, sub: &str) {
    c.text(
        s,
        t.x + 24.0,
        t.y + t.h - 66.0,
        26,
        Weight::Bold,
        color::TEXT,
        Align::Left,
    );
    if !sub.is_empty() {
        c.text_fit(
            sub,
            t.x + 24.0,
            t.y + t.h - 34.0,
            16,
            Weight::Regular,
            color::MUTED,
            t.w - 48.0,
        );
    }
}

impl Screen for Home {
    fn name(&self) -> &'static str {
        "home"
    }

    fn key(&mut self, k: Key, cx: &mut Ctx) -> Nav {
        let f = self.focus;
        if k == Key::LongOk {
            self.editing = !self.editing;
            self.moving = None;
            if self.editing && self.focus >= 6 {
                self.focus = if self.focus == 6 { 0 } else { 2 };
            }
            self.edit_started = self.editing.then(Instant::now);
            cx.toast(if self.editing {
                "Kachel wählen · OK · mit Pfeilen verschieben · OK speichern"
            } else {
                "Anordnung gespeichert"
            });
            return Nav::None;
        }
        if self.editing {
            match k {
                Key::Back | Key::Menu | Key::Home => {
                    self.editing = false;
                    self.moving = None;
                    self.edit_started = None;
                }
                Key::Ok if self.moving.is_none() && self.focus < 6 => {
                    self.moving = Some(self.focus);
                    cx.toast("Zielposition wählen · OK zum Ablegen");
                }
                Key::Ok if self.moving.is_some() && self.focus < 6 => {
                    let from = self.moving.unwrap();
                    if tile_size(self.order[from]) == tile_size(self.order[self.focus]) {
                        self.moving = None;
                        self.order.swap(from, self.focus);
                        // Tile sizes match, so their already-rendered card
                        // images can move with them instead of being rebuilt.
                        self.tile_images.swap(from, self.focus);
                        self.tile_image_sources.swap(from, self.focus);
                        self.tile_cache_keys.swap(from, self.focus);
                        cx.cfg.home_order = self.order.map(|v| v as u8);
                        cx.save();
                        self.base_cache = LayerCache::default();
                        self.base_image = None;
                        cx.toast("Kachel verschoben");
                    } else {
                        cx.toast("Bitte eine Kachel gleicher Größe wählen");
                    }
                }
                Key::Ok => {}
                _ => {}
            }
            if matches!(k, Key::Left | Key::Right | Key::Up | Key::Down) {
                self.focus = match (k, self.focus) {
                    (Key::Right, 0..=1) | (Key::Right, 3..=4) => f + 1,
                    (Key::Left, 1..=2) | (Key::Left, 4..=5) => f - 1,
                    (Key::Down, 0) => 3,
                    (Key::Down, 1) => 4,
                    (Key::Down, 2) => 5,
                    (Key::Up, 3) => 0,
                    (Key::Up, 4) => 1,
                    (Key::Up, 5) => 2,
                    _ => f,
                };
            }
            return Nav::None;
        }
        if k == Key::Ok && self.focus >= 6 {
            return match self.focus {
                6 => Nav::Push(Box::new(super::weather::WeatherScreen::new())),
                7 => Nav::Push(Box::new(super::control::Control::new(false))),
                _ => Nav::Push(Box::new(super::apps::Apps::wheel(cx, &cx.cfg.last_app_id))),
            };
        }
        self.focus = match (k, f) {
            // top row: YouTube, Live TV, Filme -- bottom row: Wetter, Nachrichten, Einstellungen
            (Key::Right, 0..=1) | (Key::Right, 3..=4) => f + 1,
            (Key::Left, 1..=2) | (Key::Left, 4..=5) => f - 1,
            (Key::Down, 0) => 3,
            (Key::Down, 1) => 4,
            (Key::Down, 2) => 5,
            (Key::Up, 3) => 0,
            (Key::Up, 4) => 1,
            (Key::Up, 5) => 2,
            (Key::Up, 0..=2) => {
                let (weather, apps) = header_tiles(&self.order);
                let center = tile_center(TILES[f]);
                if (center - tile_center(weather)).abs() <= (center - tile_center(apps)).abs() {
                    6
                } else {
                    7
                }
            }
            (Key::Down, 6) => nearest_top_slot(tile_center(header_tiles(&self.order).0)),
            (Key::Down, 7) => nearest_top_slot(tile_center(header_tiles(&self.order).1)),
            (Key::Down, 3..=5) => 8,
            (Key::Up, 8) => nearest_bottom_slot(640.0),
            (Key::Left, 8) | (Key::Right, 8) => 8,
            (Key::Right, 6) => 7,
            (Key::Left, 7) => 6,
            (Key::ChUp, _) | (Key::ChDown, _) => {
                return Nav::Push(Box::new(super::livetv::LiveTv::new(cx)))
            }
            (Key::Ok, _) => return self.open(cx),
            (Key::Menu, _) => return Nav::Push(Box::new(super::settings::Settings::new(cx))),
            _ => f,
        };
        Nav::None
    }

    fn animating(&self, _cx: &Ctx) -> bool {
        self.frame.moving() || self.editing
    }

    fn draw(&mut self, c: &mut Canvas, cx: &mut Ctx) {
        let trace_started = Instant::now();
        for i in 0..TILES.len() {
            if self.tile_cache_keys[i]
                != Some(tile_cache_key(
                    cx,
                    self.order[i],
                    &self.tile_image_sources[i],
                ))
            {
                self.refresh_tile_cache(cx, i);
            }
        }
        // Keep the warm gold backdrop stable; only the focused content changes.
        c.ambient(crate::gfx::Ambient::LiquidGold);
        // The greeting and date are stable for the whole day, so they stay in
        // the retained base. Fast-changing status is drawn below as an overlay.
        let tm = crate::app::local_tm();
        c.text(
            greeting(),
            PAD,
            70.0,
            26,
            Weight::Bold,
            color::TEXT,
            Align::Left,
        );
        let date = format!(
            "{}, {}. {}",
            crate::app::WEEKDAYS[tm.tm_wday as usize % 7],
            tm.tm_mday,
            crate::app::MONTHS[tm.tm_mon as usize % 12]
        );
        c.text(
            &date,
            PAD,
            100.0,
            14,
            Weight::Regular,
            color::MUTED,
            Align::Left,
        );
        // Fold the six retained card images into the base command list before
        // rasterizing. Previously the background and cards each forced a
        // separate full-screen pass on Home's first frame.
        for (i, t) in TILES.iter().enumerate() {
            if let Some(img) = self.tile_images[i].as_ref() {
                c.image_exact(img, t.x - TILE_BLEED, t.y - TILE_BLEED);
            }
        }
        let base_changed = c.finish_cached(&mut self.base_cache);
        let mut base_restored = base_changed;
        if base_changed {
            self.base_image = Some(c.snapshot());
            self.last_focus_rect = None;
        } else if let Some(base) = self.base_image.as_ref() {
            // Arrange mode draws pulsing outlines over every tile: they are
            // wiped with the whole base picture (also once after it ends),
            // or they pile up frame by frame and stay.
            if c.full_base_restore_required() || self.editing || self.edit_drawn {
                c.restore_image(base);
                self.last_focus_rect = None;
                base_restored = true;
            } else if let Some(frame) = self.last_focus_rect {
                c.restore_focus(base, frame.0, frame.1, frame.2, frame.3);
                // The base picture has no headline on it. A frame gliding
                // across the news tile had its strips put back without the
                // text: lines of the headline were cut until the next
                // refresh. Draw the headline again whenever that happens.
                if focus_strips_touch(frame, news_text_area(cx, &self.order)) {
                    self.news_overlay_key = None;
                }
            }
        }
        self.edit_drawn = self.editing;
        let status_key = home_status_key(cx, self.focus, self.editing, &self.order);
        if base_restored || self.status_key.as_ref() != Some(&status_key) {
            if let Some(base) = self.base_image.as_ref() {
                c.restore_rect(
                    base,
                    STATUS_DAMAGE.0,
                    STATUS_DAMAGE.1,
                    STATUS_DAMAGE.2,
                    STATUS_DAMAGE.3,
                );
                draw_home_status(
                    c,
                    cx,
                    self.focus,
                    self.editing,
                    self.moving.is_some(),
                    &self.order,
                );
                self.status_key = Some(status_key);
            }
        }
        let news_key = cx.news.revision();
        if base_restored || self.news_overlay_key != Some(news_key) {
            if !base_restored {
                if let Some(base) = self.base_image.as_ref() {
                    let (x, y, w, h) = news_text_area(cx, &self.order);
                    c.restore_rect(base, x, y, w, h);
                }
            }
            let news_slot = self.order.iter().position(|id| *id == 4).unwrap_or(4);
            draw_news_tile_overlay(c, cx, &TILES[news_slot]);
            self.news_overlay_key = Some(news_key);
        }
        if self.focus < 6 {
            let t = &TILES[self.focus];
            let bounds = self.frame.move_to_with_tau(t.x, t.y, t.w, t.h, true, 0.035);
            super::focus_ring_fast(c, bounds.0, bounds.1, bounds.2, bounds.3);
            self.last_focus_rect = Some(bounds);
        } else {
            draw_home_status(
                c,
                cx,
                self.focus,
                self.editing,
                self.moving.is_some(),
                &self.order,
            );
        }
        draw_app_wheel_button(c, self.focus == 8);
        if self.editing {
            draw_edit_pulse(
                c,
                self.edit_started.unwrap_or_else(Instant::now),
                self.moving,
                &self.order,
            );
        }
        if base_restored {
            // The full base is already drawn. Raster the small dynamic status,
            // headline and focus overlay once without another full worker pass.
            c.finish_pending();
        } else {
            c.finish_pending_damage();
        }
        if std::env::var_os("SHOT_HOME_FRAME_TRACE").is_some() {
            eprintln!(
                "Home-Frame: {:.1} ms, base changed={base_changed}",
                trace_started.elapsed().as_secs_f64() * 1000.0
            );
        }
    }

    fn draw_overlay_region(&mut self, c: &mut Canvas, cx: &mut Ctx, rect: (f32, f32, f32, f32)) {
        let Some(base) = self.base_image.as_ref() else {
            self.draw(c, cx);
            return;
        };
        c.restore_rect(base, rect.0, rect.1, rect.2, rect.3);
        self.last_focus_rect = None;
        if self.focus < 6 {
            let t = &TILES[self.focus];
            let bounds = self.frame.move_to_with_tau(t.x, t.y, t.w, t.h, true, 0.035);
            super::focus_ring_fast(c, bounds.0, bounds.1, bounds.2, bounds.3);
        }
        draw_home_status(
            c,
            cx,
            self.focus,
            self.editing,
            self.moving.is_some(),
            &self.order,
        );
        draw_app_wheel_button(c, self.focus == 8);
        if self.editing {
            draw_edit_pulse(
                c,
                self.edit_started.unwrap_or_else(Instant::now),
                self.moving,
                &self.order,
            );
        }
        c.finish_pending_damage();
    }
}

fn draw_home_status(
    c: &mut Canvas,
    cx: &Ctx,
    focus: usize,
    editing: bool,
    moving: bool,
    order: &[usize; 6],
) {
    let y = 20.0;
    let (weather_tile, apps_tile) = header_tiles(order);
    let weather_x = weather_tile.x;
    let weather_w = weather_tile.w;
    let status_x = apps_tile.x;
    let status_w = apps_tile.w;
    c.rect(
        weather_x,
        y,
        weather_w,
        46.0,
        23.0,
        Rgba::hex(0x23252c).alpha(0.94),
    );
    c.stroke_rect(
        weather_x,
        y,
        weather_w,
        46.0,
        23.0,
        1.0,
        color::BORDER.alpha(0.9),
    );
    if focus == 6 {
        c.stroke_rect(weather_x, y, weather_w, 46.0, 23.0, 2.0, GOLD);
    }
    if focus == 7 {
        c.stroke_rect(status_x, y, status_w, 46.0, 23.0, 2.0, GOLD);
    }
    c.rect(
        status_x,
        y,
        status_w,
        46.0,
        23.0,
        Rgba::hex(0x23252c).alpha(0.94),
    );
    c.stroke_rect(
        status_x,
        y,
        status_w,
        46.0,
        23.0,
        1.0,
        color::BORDER.alpha(0.9),
    );

    let (hour, minute, _) = crate::app::local_hms();
    c.text(
        &format!("{hour:02}:{minute:02}"),
        status_x + 12.0,
        y + 12.0,
        17,
        Weight::Semi,
        color::TEXT,
        Align::Left,
    );
    c.line(
        &[(status_x + 63.0, y + 9.0), (status_x + 63.0, y + 37.0)],
        1.0,
        color::BORDER.alpha(0.9),
    );
    if let Some((ok, kind, name)) = cx.net.get() {
        c.circle(
            status_x + 77.0,
            y + 23.0,
            3.0,
            if ok { Rgba::hex(0x59d39a) } else { color::RED },
        );
        if ok {
            if kind == "wifi" {
                icon_wifi(c, status_x + 94.0, y + 21.0, 16.0, color::TEXT);
            } else {
                icon_ethernet(c, status_x + 94.0, y + 23.0, 16.0, color::TEXT);
            }
            c.text_fit(
                &name,
                status_x + 109.0,
                y + 16.0,
                11,
                Weight::Regular,
                color::MUTED,
                49.0,
            );
        } else {
            c.text(
                "Offline",
                status_x + 89.0,
                y + 15.0,
                11,
                Weight::Semi,
                color::RED,
                Align::Left,
            );
        }
    }
    c.line(
        &[(status_x + 164.0, y + 9.0), (status_x + 164.0, y + 37.0)],
        1.0,
        color::BORDER.alpha(0.9),
    );
    let phone = if cx.cfg.remote_enabled {
        GOLD
    } else {
        color::MUTED
    };
    c.stroke_rect(status_x + 176.0, y + 12.0, 12.0, 21.0, 3.0, 1.4, phone);
    c.circle(status_x + 182.0, y + 30.0, 0.9, phone);
    c.text(
        if cx.cfg.remote_enabled { "AN" } else { "AUS" },
        status_x + 208.0,
        y + 16.0,
        9,
        Weight::Semi,
        phone,
        Align::Center,
    );

    if let Some(Ok(w)) = cx.weather.get() {
        let (sky, _) = crate::weather::describe(w.code);
        icon_weather(c, sky, w.day, weather_x + weather_w * 0.40, y + 23.0, 28.0);
        c.text(
            &format!("{:.0}°", w.temp),
            weather_x + weather_w * 0.60,
            y + 12.0,
            18,
            Weight::Bold,
            color::TEXT,
            Align::Left,
        );
    }
    let _ = (editing, moving);
}

fn draw_app_wheel_button(c: &mut Canvas, focused: bool) {
    let (x, y, w, h) = (525.0, 661.0, 230.0, 46.0);
    if focused {
        c.circle(640.0, y + h * 0.5, 35.0, Rgba(214, 160, 70, 34));
    }
    c.rect(
        x,
        y,
        w,
        h,
        23.0,
        if focused {
            Rgba(76, 57, 25, 235)
        } else {
            Rgba::hex(0x20232c)
        },
    );
    c.stroke_rect(
        x,
        y,
        w,
        h,
        23.0,
        if focused { 2.5 } else { 1.2 },
        if focused {
            Rgba(226, 187, 116, 200)
        } else {
            Rgba(226, 187, 116, 60)
        },
    );
    // Tiny bearing mark on the button mirrors the launcher wheel.
    c.circle(554.0, 684.0, 14.0, Rgba(196, 160, 96, 255));
    c.circle(554.0, 684.0, 10.0, Rgba::hex(0x20232c));
    c.circle(554.0, 684.0, 5.0, Rgba::hex(0x9eafd2));
    c.text(
        "APP-KREIS",
        650.0,
        670.0,
        13,
        Weight::Bold,
        color::TEXT,
        Align::Center,
    );
    c.text(
        "OK ÖFFNEN",
        650.0,
        687.0,
        10,
        Weight::Regular,
        if focused {
            Rgba::hex(0xc4d7ff)
        } else {
            color::MUTED
        },
        Align::Center,
    );
}

/// Where the headline is drawn over the news tile (beside its photo, or the
/// whole tile when there is none).
fn news_text_area(cx: &Ctx, order: &[usize; 6]) -> (f32, f32, f32, f32) {
    let news_slot = order.iter().position(|id| *id == 4).unwrap_or(4);
    let t = &TILES[news_slot];
    let iw = news_tile_photo_width(cx, t);
    if iw > 0.0 {
        (t.x + iw + 12.0, t.y + 12.0, t.w - iw - 12.0, t.h - 24.0)
    } else {
        (t.x, t.y, t.w, t.h)
    }
}

/// Whether the edge strips `Canvas::restore_focus` puts back for a focus
/// frame at `frame` reach into `area`.
fn focus_strips_touch(frame: (f32, f32, f32, f32), area: (f32, f32, f32, f32)) -> bool {
    const PAD: f32 = 7.0;
    const EDGE: f32 = 14.0;
    let (x, y, w, h) = frame;
    let strips = [
        (x - PAD, y - PAD, w + PAD * 2.0, EDGE),
        (x - PAD, y + h - PAD, w + PAD * 2.0, EDGE),
        (x - PAD, y + PAD, EDGE, (h - PAD * 2.0).max(0.0)),
        (x + w - PAD, y + PAD, EDGE, (h - PAD * 2.0).max(0.0)),
    ];
    strips.iter().any(|s| {
        s.0 < area.0 + area.2 && area.0 < s.0 + s.2 && s.1 < area.1 + area.3 && area.1 < s.1 + s.3
    })
}

fn draw_edit_pulse(c: &mut Canvas, started: Instant, moving: Option<usize>, order: &[usize; 6]) {
    let alpha = (70.0 + ((started.elapsed().as_secs_f32() * 4.0).sin() + 1.0) * 54.0) as u8;
    for (i, t) in TILES.iter().enumerate() {
        c.stroke_rect(
            t.x - 2.0,
            t.y - 2.0,
            t.w + 4.0,
            t.h + 4.0,
            24.0,
            2.0,
            if moving == Some(i) {
                Rgba(255, 202, 90, 235)
            } else {
                Rgba(128, 174, 255, alpha)
            },
        );
    }
    for tile in [header_tiles(order).0, header_tiles(order).1] {
        c.stroke_rect(
            tile.x,
            20.0,
            tile.w,
            46.0,
            23.0,
            2.0,
            Rgba(128, 174, 255, alpha),
        );
    }
    c.text(
        "ANORDNEN · OK AUSWÄHLEN · GLEICHE GRÖSSE WÄHLEN · PFEILE · OK ABLEGEN · ZURÜCK FERTIG",
        640.0,
        682.0,
        12,
        Weight::Bold,
        Rgba(168, 203, 255, alpha),
        Align::Center,
    );
}

fn tile_size(content: usize) -> u8 {
    match content {
        2 | 5 => 0, // compact: films and apps
        0 | 4 => 1, // medium: YouTube and news
        1 | 3 => 2, // large: live TV and weather
        _ => 0,
    }
}

fn header_tiles(_order: &[usize; 6]) -> (Tile, Tile) {
    // These fields sit directly above the first and third cards. Their size
    // follows those fixed slots; rearranging a tile swaps content within the
    // same size class, so the header stays aligned with the card underneath.
    (TILES[0], TILES[2])
}

fn tile_center(tile: Tile) -> f32 {
    tile.x + tile.w / 2.0
}

fn nearest_top_slot(x: f32) -> usize {
    (0..3)
        .min_by(|a, b| {
            let da = (tile_center(TILES[*a]) - x).abs();
            let db = (tile_center(TILES[*b]) - x).abs();
            da.total_cmp(&db)
        })
        .unwrap_or(0)
}

fn nearest_bottom_slot(x: f32) -> usize {
    (3..6)
        .min_by(|a, b| {
            let da = (tile_center(TILES[*a]) - x).abs();
            let db = (tile_center(TILES[*b]) - x).abs();
            da.total_cmp(&db)
        })
        .unwrap_or(4)
}

fn home_status_key(
    cx: &Ctx,
    focus: usize,
    editing: bool,
    order: &[usize; 6],
) -> (i32, i32, u64, u64, String, bool, usize, bool, [usize; 6]) {
    let (hour, minute, _) = crate::app::local_hms();
    (
        hour,
        minute,
        cx.weather.revision(),
        cx.net.revision(),
        cx.cfg.city.clone(),
        cx.cfg.remote_enabled,
        focus,
        editing,
        *order,
    )
}

fn tile_cache_key(cx: &Ctx, i: usize, image_sources: &[String]) -> (u64, u64) {
    use std::hash::{Hash, Hasher};
    let mut state = std::collections::hash_map::DefaultHasher::new();
    match i {
        1 => {
            cx.channels.revision().hash(&mut state);
            cx.cfg.livetv_last.hash(&mut state);
            cx.cfg.livetv_favs.hash(&mut state);
        }
        2 => {
            cx.films.list.revision().hash(&mut state);
            cx.cfg.last_film_key.hash(&mut state);
        }
        3 => cx.weather.revision().hash(&mut state),
        4 => cx.news.revision().hash(&mut state),
        _ => {}
    }
    for src in image_sources {
        src.hash(&mut state);
        if i == 4 {
            let w = (TILES[4].w * 0.46 * crate::gfx::RENDER_SCALE).round() as u32;
            let h = (TILES[4].h * crate::gfx::RENDER_SCALE).round() as u32;
            cx.images.cover_source_revision(src, w, h).hash(&mut state);
        } else {
            cx.images.source_revision(src).hash(&mut state);
        }
    }
    let revision = state.finish();
    (revision, 0)
}

fn tile_image_sources(cx: &Ctx, i: usize) -> Vec<String> {
    match i {
        1 => {
            let (last, favorites, _) = live_tile_selection(cx);
            last.into_iter()
                .chain(favorites)
                .map(|channel| channel.logo)
                .filter(|src| !src.is_empty())
                .collect()
        }
        2 => films_pick(cx)
            .map(|film| {
                let src = if film.banner.is_empty() {
                    film.icon
                } else {
                    film.banner
                };
                if src.is_empty() {
                    Vec::new()
                } else {
                    vec![src]
                }
            })
            .unwrap_or_default(),
        4 => cx.news.with(|items| {
            let Some(items) = items.as_ref() else {
                return Vec::new();
            };
            if items.is_empty() {
                return Vec::new();
            }
            let image = items[0].image.clone();
            if image.is_empty() {
                Vec::new()
            } else {
                vec![image]
            }
        }),
        _ => Vec::new(),
    }
}

fn draw_tile_content(c: &mut Canvas, cx: &mut Ctx, i: usize, t: &Tile) {
    home_glass(c, t);
    match i {
        0 => {
            c.vgradient(
                t.x,
                t.y,
                t.w,
                t.h,
                22.0,
                Rgba(150, 20, 30, 90),
                Rgba(20, 20, 24, 0),
            );
            icon_youtube(c, t.x + t.w / 2.0, t.y + t.h * 0.36, 96.0);
            home_chip(c, t.x + 24.0, t.y + 20.0, "FÜR DICH", Rgba::hex(0xff3448));
            label(c, t, "YouTube", "Suchen, ansehen");
        }
        1 => draw_livetv_tile(c, cx, t),
        2 => draw_films_tile(c, cx, t),
        3 => draw_weather_tile(c, cx, t),
        4 => draw_news_tile_background(c, cx, t),
        _ => {
            let (cx, cy) = (t.x + t.w / 2.0, t.y + t.h * 0.36);
            super::icon_gear(c, cx, cy, 86.0, Rgba(226, 187, 116, 245));
            label(c, t, "Einstellungen", "System · Bild · Bedienung");
            home_chip(c, t.x + 18.0, t.y + 18.0, "SYSTEM", Rgba(128, 96, 42, 220));
        }
    }
}

/// Home cards use a warmer, more dimensional glass surface. The expanded
/// per-card cache means these extra layers cost nothing while navigating.
fn home_glass(c: &mut Canvas, t: &Tile) {
    // Closely spaced, low-contrast shadows make each card float above the
    // ambient light without adding a hard dark outline.
    c.rect(
        t.x - 6.0,
        t.y + 6.0,
        t.w + 12.0,
        t.h + 4.0,
        29.0,
        Rgba(0, 0, 0, 26),
    );
    c.rect(
        t.x - 3.0,
        t.y + 3.0,
        t.w + 6.0,
        t.h + 2.0,
        26.0,
        Rgba(0, 0, 0, 38),
    );
    c.rect(t.x, t.y, t.w, t.h, 22.0, Rgba(27, 25, 25, 82));
    c.vgradient(
        t.x + 1.0,
        t.y + 1.0,
        t.w - 2.0,
        t.h * 0.55,
        21.0,
        Rgba(255, 247, 226, 50),
        Rgba(255, 236, 201, 0),
    );
    c.vgradient(
        t.x + 1.0,
        t.y + t.h * 0.68,
        t.w - 2.0,
        t.h * 0.31,
        19.0,
        Rgba(226, 187, 116, 0),
        Rgba(226, 187, 116, 11),
    );
    c.stroke_rect(
        t.x + 0.5,
        t.y + 0.5,
        t.w - 1.0,
        t.h - 1.0,
        22.0,
        1.0,
        Rgba(255, 244, 220, 22),
    );
    c.line(
        &[(t.x + 28.0, t.y + 1.0), (t.x + t.w - 28.0, t.y + 1.0)],
        1.0,
        Rgba(255, 247, 227, 46),
    );
}

fn home_chip(c: &mut Canvas, x: f32, y: f32, text: &str, fill: Rgba) {
    let width = c.fonts.width(text, 10, Weight::Bold) + 22.0;
    c.rect(x, y, width, 23.0, 11.5, fill);
    c.text(
        text,
        x + width / 2.0,
        y + 5.0,
        10,
        Weight::Bold,
        color::WHITE,
        Align::Center,
    );
}

fn draw_livetv_tile(c: &mut Canvas, cx: &mut Ctx, t: &Tile) {
    c.vgradient(
        t.x,
        t.y,
        t.w,
        t.h,
        22.0,
        Rgba(30, 110, 180, 80),
        Rgba(20, 20, 24, 0),
    );
    let (last, favs, channel_count) = live_tile_selection(cx);
    match &last {
        Some(ch) => {
            super::livetv::logo(c, cx, ch, t.x + 24.0, t.y + 40.0, 150.0, 84.0);
            for (k, f) in favs.iter().take(3).enumerate() {
                super::livetv::logo(
                    c,
                    cx,
                    f,
                    t.x + 190.0 + k as f32 * 56.0,
                    t.y + 62.0,
                    48.0,
                    40.0,
                );
            }
        }
        None => {
            // a little TV set
            let (cxp, cy) = (t.x + t.w / 2.0, t.y + t.h * 0.36);
            c.stroke_rect(
                cxp - 52.0,
                cy - 34.0,
                104.0,
                68.0,
                10.0,
                6.0,
                Rgba::hex(0x7cc4ff),
            );
            c.line(
                &[(cxp - 22.0, cy + 46.0), (cxp + 22.0, cy + 46.0)],
                6.0,
                Rgba::hex(0x7cc4ff),
            );
        }
    }
    let sub = match (&last, channel_count) {
        (Some(ch), _) => format!("Zuletzt: {}", ch.name),
        (None, Some(count)) => format!("{count} Sender"),
        _ => String::new(),
    };
    label(c, t, "Live TV", &sub);
    c.rect(t.x + t.w - 70.0, t.y + 20.0, 48.0, 22.0, 6.0, color::RED);
    c.text(
        "LIVE",
        t.x + t.w - 46.0,
        t.y + 23.0,
        12,
        Weight::Bold,
        color::WHITE,
        Align::Center,
    );
    c.text(
        "DEINE SENDER",
        t.x + 24.0,
        t.y + 22.0,
        11,
        Weight::Bold,
        Rgba(218, 236, 255, 220),
        Align::Left,
    );
}

/// Select only the channels shown in the Home tile. The full IPTV list can be
/// thousands of entries, so keep it borrowed and clone at most four items.
fn live_tile_selection(
    cx: &Ctx,
) -> (
    Option<crate::livetv::Channel>,
    Vec<crate::livetv::Channel>,
    Option<usize>,
) {
    cx.channels.with(|value| {
        let Some(Ok((list, _))) = value.as_ref() else {
            return (None, Vec::new(), None);
        };
        let last = list
            .iter()
            .find(|channel| channel.url == cx.cfg.livetv_last)
            .cloned();
        let favorites: Vec<_> = list
            .iter()
            .filter(|channel| {
                cx.cfg.livetv_favs.contains(&channel.url)
                    && Some(&channel.url) != last.as_ref().map(|item| &item.url)
            })
            .take(3)
            .cloned()
            .collect();
        let favorites = if favorites.is_empty() {
            list.iter()
                .filter(|channel| Some(&channel.url) != last.as_ref().map(|item| &item.url))
                .take(3)
                .cloned()
                .collect()
        } else {
            favorites
        };
        (last, favorites, Some(list.len()))
    })
}

/// Continue watching first, else keep the home preview stable until the feed changes.
fn films_pick(cx: &Ctx) -> Option<crate::films::Film> {
    cx.films.list.with(|list| {
        let l = list.as_ref()?;
        let resumable = |f: &&crate::films::Film| {
            f.position > 60.0 && f.duration > 0.0 && f.position < f.duration - 120.0
        };
        l.films
            .iter()
            .find(|f| f.key == cx.cfg.last_film_key && resumable(f))
            .cloned()
            .or_else(|| {
                l.films
                    .iter()
                    .filter(resumable)
                    .max_by_key(|f| f.position as u64)
                    .cloned()
            })
            .or_else(|| l.films.first().cloned())
    })
}

fn draw_films_tile(c: &mut Canvas, cx: &mut Ctx, t: &Tile) {
    let has_list = cx.films.list.is_set();
    let pick = films_pick(cx);
    let mut drawn = false;
    if let Some(f) = &pick {
        let src = if f.banner.is_empty() {
            f.icon.clone()
        } else {
            f.banner.clone()
        };
        if let Some(img) = cx.images.get(&src, 760, 520) {
            c.image_cover(&img, t.x, t.y, t.w, t.h, 22.0);
            c.vgradient(
                t.x,
                t.y + t.h * 0.35,
                t.w,
                t.h * 0.65,
                22.0,
                Rgba(0, 0, 0, 0),
                Rgba(0, 0, 0, 220),
            );
            drawn = true;
        }
    }
    if !drawn {
        icon_film(c, t.x + t.w / 2.0, t.y + t.h * 0.36, 96.0, color::MUTED);
    }
    home_chip(
        c,
        t.x + 18.0,
        t.y + 18.0,
        "MEDIATHEK",
        Rgba(20, 24, 34, 176),
    );
    let sub = match &pick {
        Some(f) if f.position > 60.0 => format!("Weiter: {}", f.title),
        Some(f) => f.title.clone(),
        None if !has_list => "Wird geladen …".into(),
        None => "Keine Filme gefunden".into(),
    };
    label(c, t, "Filme", &sub);
    if let Some(f) = pick.filter(|f| f.position > 60.0 && f.duration > 0.0) {
        let p = (f.position / f.duration) as f32;
        c.rect(
            t.x + 24.0,
            t.y + t.h - 12.0,
            t.w - 48.0,
            4.0,
            2.0,
            color::WHITE.a(50),
        );
        c.rect(
            t.x + 24.0,
            t.y + t.h - 12.0,
            (t.w - 48.0) * p,
            4.0,
            2.0,
            GOLD,
        );
    }
}

fn draw_weather_tile(c: &mut Canvas, cx: &mut Ctx, t: &Tile) {
    match cx.weather.get() {
        Some(Ok(w)) => {
            let (sky, desc) = crate::weather::describe(w.code);
            c.vgradient(
                t.x,
                t.y,
                t.w,
                t.h,
                22.0,
                if w.day {
                    Rgba(60, 120, 200, 110)
                } else {
                    Rgba(30, 40, 90, 110)
                },
                Rgba(20, 20, 24, 0),
            );
            icon_weather(c, sky, w.day, t.x + t.w - 70.0, t.y + 64.0, 86.0);
            c.text(
                &format!("{:.0}°", w.temp),
                t.x + 24.0,
                t.y + 20.0,
                64,
                Weight::Bold,
                color::TEXT,
                Align::Left,
            );
            c.text(
                desc,
                t.x + 24.0,
                t.y + 100.0,
                18,
                Weight::Semi,
                color::TEXT,
                Align::Left,
            );
            if let Some(d) = w.days.first() {
                c.text(
                    &format!(
                        "H {:.0}°  T {:.0}°  ·  gefühlt {:.0}°",
                        d.max, d.min, w.feels
                    ),
                    t.x + 24.0,
                    t.y + 126.0,
                    15,
                    Weight::Regular,
                    color::MUTED,
                    Align::Left,
                );
            }
            for (i, h) in w.hours.iter().skip(1).step_by(3).take(4).enumerate() {
                let x = t.x + 24.0 + i as f32 * 82.0;
                c.text(
                    &format!("{:02}", h.hour),
                    x + 18.0,
                    t.y + 160.0,
                    13,
                    Weight::Regular,
                    color::MUTED,
                    Align::Center,
                );
                icon_weather(
                    c,
                    crate::weather::describe(h.code).0,
                    h.day,
                    x + 18.0,
                    t.y + 190.0,
                    28.0,
                );
                c.text(
                    &format!("{:.0}°", h.temp),
                    x + 18.0,
                    t.y + 208.0,
                    14,
                    Weight::Semi,
                    color::TEXT,
                    Align::Center,
                );
            }
        }
        Some(Err(_)) => label(c, t, "Wetter", "Keine Verbindung"),
        None => label(c, t, "Wetter", "Wird geladen …"),
    }
}

fn news_tile_story(cx: &Ctx) -> (bool, usize, Option<crate::news::NewsItem>) {
    cx.news.with(|items| match items.as_ref() {
        Some(items) if !items.is_empty() => (true, items.len().min(6), Some(items[0].clone())),
        Some(_) => (true, 0, None),
        None => (false, 0, None),
    })
}

fn news_tile_photo_width(cx: &Ctx, t: &Tile) -> f32 {
    cx.news.with(|items| {
        items
            .as_ref()
            .and_then(|items| items.first())
            .filter(|item| !item.image.is_empty())
            .map(|_| t.w * 0.46)
            .unwrap_or(0.0)
    })
}

/// The photo and its seam are cached with the tile. Headline changes do not
/// force the Pi to rasterize the large cover image again.
fn draw_news_tile_background(c: &mut Canvas, cx: &mut Ctx, t: &Tile) {
    let (_, _, story) = news_tile_story(cx);
    let Some(story) = story else {
        return;
    };
    if story.image.is_empty() {
        return;
    }
    let iw = t.w * 0.46;
    let pixel_w = (iw * crate::gfx::RENDER_SCALE).round() as u32;
    let pixel_h = (t.h * crate::gfx::RENDER_SCALE).round() as u32;
    if let Some(img) = cx.images.get_cover(&story.image, pixel_w, pixel_h) {
        c.image_exact(&img, t.x, t.y);
        c.rect(t.x + iw - 22.0, t.y, 22.0, t.h, 0.0, color::PANEL_SOLID);
    }
}

/// Text is redrawn only when the news data changes; the cached photo remains
/// untouched. This keeps headline refreshes cheap on the Pi 3.
fn draw_news_tile_overlay(c: &mut Canvas, cx: &mut Ctx, t: &Tile) {
    let (is_set, count, story) = news_tile_story(cx);
    let Some(n) = story else {
        icon_news(c, t.x + t.w / 2.0, t.y + t.h * 0.38, 70.0, color::MUTED);
        label(
            c,
            t,
            "Nachrichten",
            if is_set {
                "Keine Verbindung"
            } else {
                "Wird geladen …"
            },
        );
        return;
    };
    let iw = if n.image.is_empty() { 0.0 } else { t.w * 0.46 };
    let x = t.x + iw + 24.0;
    let w = t.w - iw - 48.0;
    c.rect(x, t.y + 22.0, 104.0, 24.0, 12.0, Rgba::hex(0x0b4fa3));
    c.text(
        "tagesschau",
        x + 52.0,
        t.y + 25.0,
        13,
        Weight::Bold,
        color::WHITE,
        Align::Center,
    );
    if !n.topline.is_empty() {
        c.text_fit(&n.topline, x, t.y + 58.0, 14, Weight::Semi, GOLD, w);
    }
    let title_y = t.y + 80.0;
    let title_bottom = t.y + t.h - 48.0;
    let line_h = 24.0;
    let max_lines = (((title_bottom - title_y) / line_h).floor() as usize).clamp(1, 4);
    let lines = c.wrap(&n.title, 20, Weight::Bold, w);
    c.clip_y(Some((title_y - 20.0, title_bottom)));
    for (i, line) in lines.iter().take(max_lines).enumerate() {
        let text = if i + 1 == max_lines && lines.len() > max_lines {
            c.ellipsize(&(line.clone() + " …"), 20, Weight::Bold, w)
        } else {
            c.ellipsize(line, 20, Weight::Bold, w)
        };
        c.text(
            &text,
            x,
            title_y + i as f32 * line_h,
            20,
            Weight::Bold,
            color::TEXT,
            Align::Left,
        );
    }
    c.clip_y(None);
    let dots = count.min(5);
    let dots_width = dots as f32 * 12.0;
    let date = crate::news::time_of(&n.date);
    let date = c.ellipsize(
        &date,
        14,
        Weight::Regular,
        (w - dots_width - 12.0).max(14.0),
    );
    c.text(
        &date,
        x,
        t.y + t.h - 34.0,
        14,
        Weight::Regular,
        color::MUTED,
        Align::Left,
    );
    for k in 0..dots {
        c.circle(
            x + w - dots_width + k as f32 * 12.0,
            t.y + t.h - 26.0,
            3.0,
            if k == 0 {
                color::TEXT
            } else {
                color::MUTED.a(120)
            },
        );
    }
}
