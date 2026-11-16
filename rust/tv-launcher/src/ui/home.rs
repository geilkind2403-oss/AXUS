//! The home screen: big tiles for what a TV is used for.
use super::*;
use crate::app::{Ctx, Nav, Screen};
use crate::gfx::{Align, Canvas, Image, LayerCache, Rgba, Weight, color};
use std::time::{Duration, Instant};

pub struct Home {
    focus: usize,
    order: [usize; 6],
    editing: bool,
    moving: Option<usize>,
    edit_started: Option<Instant>,
    frame: LiquidFocus,
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
    /// which item the focus frame was last drawn for (redraw only on change)
    focus_drawn: Option<usize>,
    orb: Orb,
    /// the Filme clapperboard: when it last started to snap, whether OK
    /// waits for it to open the app, and whether the tile had the focus
    clap: Option<Instant>,
    clap_open: bool,
    clap_focused: bool,
    /// the snap just ended: one more frame puts the board at rest
    clap_settle: bool,
    settings_gear: SettingsGear,
    settings_open_at: Option<Instant>,
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
const SETTINGS_OPEN_FOR: Duration = Duration::from_millis(420);

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
            frame: LiquidFocus::default(),
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
            focus_drawn: None,
            orb: Orb::new(),
            clap: None,
            clap_open: false,
            clap_focused: false,
            clap_settle: false,
            settings_gear: SettingsGear::default(),
            settings_open_at: None,
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

    fn open(&mut self, cx: &mut Ctx) -> Nav {
        let content = if self.focus < 6 {
            self.order[self.focus]
        } else {
            self.focus
        };
        if content == 5 {
            self.settings_open_at = Some(Instant::now());
            return Nav::None;
        }
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
        if self.settings_open_at.is_some() {
            if k == Key::Back {
                self.settings_open_at = None;
            }
            return Nav::None;
        }
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
                return Nav::Push(Box::new(super::livetv::LiveTv::new(cx)));
            }
            (Key::Ok, _) if self.focus < 6 && self.order[self.focus] == 2 => {
                // "Cut!": the board snaps shut, then the films open
                self.clap = Some(Instant::now());
                self.clap_open = true;
                return Nav::None;
            }
            (Key::Ok, _) => return self.open(cx),
            (Key::Menu, _) => {
                self.settings_open_at = Some(Instant::now());
                return Nav::None;
            }
            _ => f,
        };
        Nav::None
    }

    fn animating(&self, _cx: &Ctx) -> bool {
        // the App-Kreis orb always turns a little
        true
    }

    fn poll(&mut self, cx: &mut Ctx) -> Nav {
        if self.clap_open
            && self
                .clap
                .is_some_and(|t| t.elapsed().as_secs_f32() >= CLAP_OPEN_AFTER)
        {
            self.clap_open = false;
            return self.open(cx);
        }
        if let Some(started) = self.settings_open_at {
            if started.elapsed() >= SETTINGS_OPEN_FOR {
                self.settings_open_at = None;
                return Nav::Push(Box::new(super::settings::Settings::new(cx)));
            }
        }
        Nav::None
    }

    fn busy(&self, _cx: &Ctx) -> bool {
        self.clap.is_some()
            || self.frame.moving()
            || self.editing
            || self.settings_open_at.is_some()
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
        // The orb keeps Home ticking; the big focus frame is only erased and
        // drawn again when it moves, pulses, changes or was painted over.
        // the clapperboard snaps when its tile gets the focus
        let films_slot = self.order.iter().position(|id| *id == 2).unwrap_or(2);
        let films_focused = self.focus == films_slot;
        if films_focused && !self.clap_focused && !self.editing {
            self.clap = Some(Instant::now());
        }
        self.clap_focused = films_focused;
        let clap_t = self.clap.map(|t| t.elapsed().as_secs_f32());
        let clap_live = clap_t.is_some_and(|t| t < CLAP_TOTAL);
        if clap_t.is_some() && !clap_live && !self.clap_open {
            self.clap = None;
            self.clap_settle = true;
        }
        let clap_redraw = clap_live || std::mem::take(&mut self.clap_settle);
        let focus_dirty = clap_redraw
            || base_changed
            || self.frame.moving()
            || self.focus_drawn != Some(self.focus)
            || self.news_overlay_key != Some(cx.news.revision());
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
            } else if let Some(frame) = self.last_focus_rect.filter(|_| focus_dirty) {
                c.restore_rect(base, frame.0, frame.1, frame.2, frame.3);
                // The focus is a soft pane wash now, so restore its full old
                // footprint before painting the new position.
                if focus_touches_area(frame, news_text_area(cx, &self.order)) {
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
        // the moving arm of the clapperboard (the board itself is cached)
        if focus_dirty || base_restored {
            let t = &TILES[films_slot];
            if !base_restored {
                if let Some(base) = self.base_image.as_ref() {
                    let a = clap_area(t);
                    c.restore_rect(base, a.0, a.1, a.2, a.3);
                }
            }
            draw_clap_arm(c, t, clap_t.filter(|_| clap_live));
        }
        let mut focus_job = None;
        if self.focus < 6 {
            if focus_dirty || base_restored {
                let t = &TILES[self.focus];
                let bounds = self.frame.step(t);
                let (reach, params) = self.frame.prepare(c, bounds);
                self.last_focus_rect = Some(reach);
                focus_job = Some(params);
            }
        } else if focus_dirty {
            self.last_focus_rect = None;
        }
        self.focus_drawn = Some(self.focus);
        if let Some(base) = self.base_image.as_ref() {
            self.orb
                .tick(c, base, self.focus == 8, base_restored || focus_dirty);
            let slot = self.order.iter().position(|id| *id == 5).unwrap_or(5);
            self.settings_gear.draw(
                c,
                base,
                TILES[slot],
                !self.editing && self.focus == slot,
                self.settings_open_at,
                base_restored,
            );
        }
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
        // the focus frame goes over the finished picture, on all cores
        if let Some(params) = focus_job {
            crate::focus_fx::draw(c, params);
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
        // everything drawn below is translucent: put back what is under it
        for r in [Some(STATUS_DAMAGE), self.last_focus_rect]
            .into_iter()
            .flatten()
        {
            c.restore_rect(base, r.0, r.1, r.2, r.3);
        }
        self.last_focus_rect = None;
        let films = &TILES[self.order.iter().position(|id| *id == 2).unwrap_or(2)];
        let a = clap_area(films);
        c.restore_rect(base, a.0, a.1, a.2, a.3);
        let clap_t = self.clap.map(|t| t.elapsed().as_secs_f32());
        draw_clap_arm(c, films, clap_t.filter(|t| *t < CLAP_TOTAL));
        let mut focus_job = None;
        if self.focus < 6 {
            let t = &TILES[self.focus];
            let bounds = self.frame.step(t);
            let (reach, params) = self.frame.prepare(c, bounds);
            self.last_focus_rect = Some(reach);
            focus_job = Some(params);
        }
        draw_home_status(
            c,
            cx,
            self.focus,
            self.editing,
            self.moving.is_some(),
            &self.order,
        );
        if let Some(base) = self.base_image.as_ref() {
            self.orb.tick(c, base, self.focus == 8, true);
            let slot = self.order.iter().position(|id| *id == 5).unwrap_or(5);
            self.settings_gear.draw(
                c,
                base,
                TILES[slot],
                !self.editing && self.focus == slot,
                self.settings_open_at,
                true,
            );
        }
        if self.editing {
            draw_edit_pulse(
                c,
                self.edit_started.unwrap_or_else(Instant::now),
                self.moving,
                &self.order,
            );
        }
        c.finish_pending_damage();
        if let Some(params) = focus_job {
            crate::focus_fx::draw(c, params);
        }
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
    header_glass(c, weather_x, y, weather_w, PILL_H, focus == 6);
    header_glass(c, status_x, y, status_w, PILL_H, focus == 7);

    let (hour, minute, _) = crate::app::local_hms();
    c.text(
        &format!("{hour:02}:{minute:02}"),
        status_x + 12.0,
        y + 12.0,
        17,
        Weight::Semi,
        if focus == 7 { GOLD } else { color::TEXT },
        Align::Left,
    );
    c.line(
        &[(status_x + 63.0, y + 11.0), (status_x + 63.0, y + 35.0)],
        1.0,
        Rgba(255, 244, 220, 34),
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
        &[(status_x + 164.0, y + 11.0), (status_x + 164.0, y + 35.0)],
        1.0,
        Rgba(255, 244, 220, 34),
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
        let (sky, text) = crate::weather::describe(w.code);
        icon_weather(c, sky, w.day, weather_x + 34.0, y + 23.0, 28.0);
        c.text(
            &format!("{:.0}°", w.temp),
            weather_x + 58.0,
            y + 12.0,
            18,
            Weight::Bold,
            color::TEXT,
            Align::Left,
        );
        c.line(
            &[(weather_x + 106.0, y + 11.0), (weather_x + 106.0, y + 35.0)],
            1.0,
            Rgba(255, 244, 220, 34),
        );
        c.text_fit(
            &cx.cfg.city,
            weather_x + 120.0,
            y + 9.0,
            12,
            Weight::Semi,
            if focus == 6 { GOLD } else { color::TEXT },
            weather_w - 140.0,
        );
        c.text_fit(
            text,
            weather_x + 120.0,
            y + 25.0,
            11,
            Weight::Regular,
            color::MUTED,
            weather_w - 140.0,
        );
    }
    let _ = (editing, moving);
}

const PILL_H: f32 = 46.0;

/// The weather and clock buttons in the glass of the big cards: a soft lift
/// shadow, the warm translucent pane, a light top and a faint rim; focused,
/// a gold wash and ring like the card focus.
fn header_glass(c: &mut Canvas, x: f32, y: f32, w: f32, h: f32, focused: bool) {
    let r = h / 2.0;
    c.rect(x - 2.0, y + 2.0, w + 4.0, h, r + 2.0, Rgba(0, 0, 0, 34));
    c.rect(x, y, w, h, r, Rgba(30, 27, 25, 150));
    c.vgradient(
        x + 1.0,
        y + 1.0,
        w - 2.0,
        h * 0.6,
        r - 1.0,
        Rgba(255, 247, 226, 46),
        Rgba(255, 236, 201, 0),
    );
    c.vgradient(
        x + 1.0,
        y + h * 0.55,
        w - 2.0,
        h * 0.44,
        r - 1.0,
        Rgba(226, 187, 116, 0),
        Rgba(255, 231, 180, 16),
    );
    if focused {
        c.rect(x, y, w, h, r, Rgba(GOLD.0, GOLD.1, GOLD.2, 34));
        c.stroke_rect(
            x + 0.5,
            y + 0.5,
            w - 1.0,
            h - 1.0,
            r,
            2.0,
            Rgba(GOLD.0, GOLD.1, GOLD.2, 220),
        );
    } else {
        c.stroke_rect(
            x + 0.5,
            y + 0.5,
            w - 1.0,
            h - 1.0,
            r,
            1.0,
            Rgba(255, 244, 220, 40),
        );
    }
    c.line(
        &[(x + r, y + 1.5), (x + w - r - 24.0, y + 1.5)],
        1.4,
        Rgba(255, 249, 232, if focused { 150 } else { 96 }),
    );
}

/// Centre and radius of the App-Kreis orb, under the cards.
const ORB_C: (f32, f32) = (640.0, 684.0);
const ORB_R: f32 = 24.0;
/// What the orb and its label cover (kept clear of the card focus glow).
const ORB_AREA: (f32, f32, f32, f32) = (598.0, 652.0, 170.0, 68.0);
/// Orb pictures per second (it turns slowly: 30 is plenty, half the work).
const ORB_FRAME: f32 = 1.0 / 30.0;

/// Settings' brass gear is an overlay over the cached tile artwork. It only
/// rerasterizes its small patch while focused/opening; at rest it is drawn once.
const GEAR_FRAME: f32 = 1.0 / 30.0;

#[derive(Default)]
struct SettingsGear {
    last: Option<Instant>,
    angle: f32,
    painted: bool,
    center: Option<(f32, f32)>,
    was_active: bool,
}

impl SettingsGear {
    fn draw(
        &mut self,
        c: &mut Canvas,
        base: &Image,
        tile: Tile,
        focused: bool,
        opening: Option<Instant>,
        base_restored: bool,
    ) {
        let now = Instant::now();
        let active = focused || opening.is_some();
        let center = (tile.x + tile.w * 0.5, tile.y + tile.h * 0.36);
        let moved = self.center != Some(center);
        let due = self.last.map_or(true, |last| {
            now.duration_since(last).as_secs_f32() >= GEAR_FRAME
        });
        let opening_active = opening.is_some_and(|t| t.elapsed() < SETTINGS_OPEN_FOR);
        if !base_restored
            && !moved
            && self.painted
            && self.was_active == active
            && (!active || !due)
        {
            return;
        }
        // Flush the other Home overlays first. The small restore below must
        // replay only the gear commands, or an earlier translucent layer can
        // leave a rectangular damage patch on the cached glass tile.
        c.finish_pending_damage();
        if moved {
            if let Some((x, y)) = self.center {
                c.restore_rect(base, x - 66.0, y - 66.0, 132.0, 132.0);
            }
        }
        c.restore_rect(base, center.0 - 66.0, center.1 - 66.0, 132.0, 132.0);
        let dt = self.last.map_or(GEAR_FRAME, |last| {
            now.duration_since(last).as_secs_f32().min(0.08)
        });
        if active {
            self.angle += dt * if opening_active { 16.0 } else { 2.7 };
        }
        let pulse = if opening_active {
            ((opening.unwrap().elapsed().as_secs_f32() / SETTINGS_OPEN_FOR.as_secs_f32())
                * std::f32::consts::PI)
                .sin()
                .max(0.0)
        } else {
            0.0
        };
        draw_settings_gear(c, center.0, center.1, self.angle, pulse);
        self.center = Some(center);
        self.last = Some(now);
        self.painted = true;
        self.was_active = active;
    }
}

fn draw_settings_gear(c: &mut Canvas, x: f32, y: f32, angle: f32, pulse: f32) {
    let scale = 1.0 + pulse * 0.08;
    let outer = 36.0 * scale;
    let inner = 29.0 * scale;
    let mut teeth = Vec::with_capacity(32);
    for i in 0..32 {
        let a = angle + i as f32 * std::f32::consts::TAU / 32.0;
        let r = if i % 4 == 0 || i % 4 == 1 {
            outer
        } else {
            inner
        };
        teeth.push((x + r * a.cos(), y + r * a.sin()));
    }
    teeth.push(teeth[0]);
    if pulse > 0.0 {
        c.circle(
            x,
            y,
            46.0 * scale,
            Rgba(226, 187, 116, (pulse * 22.0) as u8),
        );
    }
    c.line(&teeth, 5.5, Rgba(226, 187, 116, 170 + (pulse * 70.0) as u8));
    let ring = arc_points(x, y, 21.0 * scale, 0.0, std::f32::consts::TAU);
    c.line(&ring, 4.0, Rgba(250, 226, 178, 235));
    c.circle(x, y, 11.5 * scale, Rgba(45, 34, 18, 235));
    c.circle(x, y, 5.0 * scale, Rgba(250, 226, 178, 245));
    // A moving glint makes the rotation readable even when the gear silhouette
    // is symmetric, while keeping all edges soft and free of hard box outlines.
    let glint = angle + 0.45;
    c.circle(
        x + outer * glint.cos(),
        y + outer * glint.sin(),
        3.0,
        Rgba(255, 245, 217, 245),
    );
}

/// The App-Kreis button: a small gold glass orb that always turns -- two
/// light arcs circle its rim, six app-coloured beads orbit the core and the
/// core breathes. Focused, it spins up, glows and shows its name.
struct Orb {
    started: Instant,
    last: Option<Instant>,
    /// the rotation, accumulated (the speed changes with the focus)
    spin: f32,
    /// 0 unfocused .. 1 focused, eased over a quarter second
    focus_k: f32,
}

impl Orb {
    fn new() -> Orb {
        Orb {
            started: Instant::now(),
            last: None,
            spin: 0.0,
            focus_k: 0.0,
        }
    }

    /// Draws the next orb picture when it is due (or `force`d because what
    /// is under it was just redrawn). Puts back the Home picture under it
    /// first: the orb is translucent and would pile up otherwise.
    fn tick(&mut self, c: &mut Canvas, base: &Image, focused: bool, force: bool) {
        let now = Instant::now();
        let dt = self
            .last
            .map(|l| now.duration_since(l).as_secs_f32())
            .unwrap_or(ORB_FRAME);
        let target = if focused { 1.0 } else { 0.0 };
        let settling = (self.focus_k - target).abs() > 0.001;
        if !force && !settling && dt < ORB_FRAME {
            return;
        }
        if force && !settling && dt < ORB_FRAME * 0.5 {
            // forced twice in one tick: same picture, just redraw it
        } else {
            let dt = dt.min(0.1);
            let step = (dt / 0.22).min(1.0);
            self.focus_k += (target - self.focus_k).clamp(-step, step);
            self.spin += dt * (0.55 + 1.6 * self.focus_k);
            self.last = Some(now);
        }
        let (x, y, w, h) = ORB_AREA;
        c.restore_rect(base, x, y, w, h);
        draw_orb(
            c,
            self.started.elapsed().as_secs_f32(),
            self.spin,
            crate::gfx::ease(self.focus_k),
        );
    }
}

/// Points along a circle arc (angles in radians).
fn arc_points(cx: f32, cy: f32, r: f32, from: f32, to: f32) -> Vec<(f32, f32)> {
    let n = (((to - from).abs() * r / 4.0).ceil() as usize).clamp(6, 48);
    (0..=n)
        .map(|i| {
            let a = from + (to - from) * i as f32 / n as f32;
            (cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

fn draw_orb(c: &mut Canvas, t: f32, spin: f32, k: f32) {
    use std::f32::consts::TAU;
    let (ox, oy) = ORB_C;
    let gold = |a: f32| Rgba(GOLD.0, GOLD.1, GOLD.2, a.clamp(0.0, 255.0) as u8);
    let breathe = 0.5 + 0.5 * (t * TAU / 3.2).sin();
    // halo: breathes, wider and brighter when focused
    let halo = ORB_R + 4.0 + 2.0 * breathe + 3.0 * k;
    c.circle(ox, oy, halo, gold(10.0 + 12.0 * breathe + 30.0 * k));
    c.circle(ox, oy, ORB_R + 2.5, gold(16.0 + 8.0 * breathe + 30.0 * k));
    // the glass body: dark warm pane, light from above
    c.circle(ox, oy, ORB_R, Rgba(28, 25, 23, 235));
    c.vgradient(
        ox - ORB_R + 1.0,
        oy - ORB_R + 1.0,
        2.0 * ORB_R - 2.0,
        2.0 * ORB_R - 2.0,
        ORB_R - 1.0,
        Rgba(255, 245, 222, 44 + (20.0 * k) as u8),
        Rgba(226, 187, 116, 10),
    );
    // the rim
    c.line(
        &arc_points(ox, oy, ORB_R - 0.5, 0.0, TAU),
        1.0 + k,
        gold(70.0 + 120.0 * k),
    );
    // two light arcs chase each other round the rim
    for (i, len, alpha) in [(0.0, 1.9, 235.0), (0.5, 0.9, 120.0)] {
        let a0 = spin * 1.7 + i * TAU;
        c.line(
            &arc_points(ox, oy, ORB_R - 0.5, a0, a0 + len),
            2.2 + 0.8 * k,
            Rgba(255, 226, 160, alpha as u8),
        );
    }
    // six app-coloured beads orbit the core, the other way round
    const BEADS: [Rgba; 6] = [
        Rgba(255, 52, 72, 255),
        Rgba(120, 180, 255, 255),
        Rgba(255, 150, 60, 255),
        Rgba(226, 187, 116, 255),
        Rgba(110, 215, 160, 255),
        Rgba(200, 140, 255, 255),
    ];
    let orbit = 13.5 + 1.0 * k;
    for (i, col) in BEADS.iter().enumerate() {
        let a = -spin + i as f32 * TAU / 6.0;
        // a faint trail behind each bead
        c.line(
            &arc_points(ox, oy, orbit, a, a + 0.55),
            1.6,
            Rgba(col.0, col.1, col.2, 60),
        );
        c.circle(
            ox + orbit * a.cos(),
            oy + orbit * a.sin(),
            2.4 + 0.5 * k,
            *col,
        );
    }
    // the core breathes
    let core = 4.6 + 1.0 * breathe + 1.2 * k;
    c.circle(ox, oy, core + 2.5, gold(40.0 + 40.0 * k));
    c.circle(ox, oy, core, Rgba(255, 222, 150, 255));
    c.circle(
        ox - core * 0.3,
        oy - core * 0.35,
        core * 0.38,
        Rgba(255, 252, 240, 220),
    );
    // glint on the glass, top left
    c.line(
        &arc_points(ox, oy, ORB_R - 4.0, TAU * 0.58, TAU * 0.68),
        1.6,
        Rgba(255, 252, 240, 120 + (60.0 * k) as u8),
    );
    // the name slides in beside it when focused
    if k > 0.02 {
        let a = (k * 255.0) as u8;
        let dx = 10.0 * (1.0 - k);
        c.text(
            "APP-KREIS",
            ox + ORB_R + 14.0 - dx,
            oy - 15.0,
            13,
            Weight::Bold,
            Rgba(GOLD.0, GOLD.1, GOLD.2, a),
            Align::Left,
        );
        c.text(
            "OK öffnen",
            ox + ORB_R + 14.0 - dx,
            oy + 2.0,
            11,
            Weight::Regular,
            Rgba(200, 192, 178, a),
            Align::Left,
        );
    }
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
fn focus_touches_area(frame: (f32, f32, f32, f32), area: (f32, f32, f32, f32)) -> bool {
    frame.0 < area.0 + area.2
        && area.0 < frame.0 + frame.2
        && frame.1 < area.1 + area.3
        && area.1 < frame.1 + frame.3
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
    match i {
        0 => {
            c.vgradient(
                t.x,
                t.y,
                t.w,
                t.h,
                22.0,
                Rgba(150, 20, 30, 8),
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
            label(c, t, "Einstellungen", "System · Bild · Bedienung");
            home_chip(c, t.x + 18.0, t.y + 18.0, "SYSTEM", Rgba(128, 96, 42, 220));
        }
    }
    home_glass_reflection(c, t);
}

/// Front-surface reflections sit above the artwork, so each card reads as
/// translucent glass instead of a dark tile with a glass-coloured fill.
fn home_glass_reflection(c: &mut Canvas, t: &Tile) {
    // Broad, low-alpha light fields add volume while leaving the card edges
    // unoutlined and the Liquid Gold backdrop visible through the pane.
    c.vgradient(
        t.x + 1.0,
        t.y + 1.0,
        t.w - 2.0,
        t.h * 0.70,
        30.0,
        Rgba(255, 250, 238, 26),
        Rgba(255, 236, 201, 0),
    );
    c.vgradient(
        t.x + 2.0,
        t.y + t.h * 0.58,
        t.w - 4.0,
        t.h * 0.40,
        26.0,
        Rgba(226, 187, 116, 0),
        Rgba(255, 231, 180, 22),
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
        Rgba(30, 110, 180, 8),
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
    draw_clap_board(c, t, drawn);
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
                    Rgba(60, 120, 200, 10)
                } else {
                    Rgba(30, 40, 90, 10)
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

/// Home's focus: a liquid gold frame. The edge in the direction of travel
/// races ahead and the trailing edge follows later, so the frame stretches
/// towards the next card like a drop and pulls itself together there; on
/// arrival the ring flashes brighter and settles in from a slightly bigger
/// size.
#[derive(Default)]
struct LiquidFocus {
    /// left, top, right, bottom
    edges: [Anim; 4],
    shown: bool,
    target: (f32, f32, f32, f32),
    /// the frame has reached the card it is heading to
    landed: bool,
    /// when it landed (the arrival pulse)
    arrived: Option<Instant>,
}

/// How far glow and pulse reach outside the frame (logical px).
const FOCUS_REACH: f32 = 9.0;
const LEAD_TAU: f32 = 0.024;
const TRAIL_TAU: f32 = 0.06;
const PULSE: f32 = 0.30;
/// Close enough to count as arrived: the pulse fires as the frame lands,
/// not after its last pixel has crept into place.
const LAND_PX: f32 = 6.0;

impl LiquidFocus {
    fn step(&mut self, t: &Tile) -> (f32, f32, f32, f32) {
        let goal = [t.x, t.y, t.x + t.w, t.y + t.h];
        if !self.shown {
            for (e, g) in self.edges.iter_mut().zip(goal) {
                e.jump(g);
            }
            self.shown = true;
            self.landed = true;
        }
        if (t.x, t.y, t.w, t.h) != self.target {
            self.target = (t.x, t.y, t.w, t.h);
            self.landed = false;
            self.arrived = None;
        }
        for (i, (e, g)) in self.edges.iter_mut().zip(goal).enumerate() {
            e.set(g);
            // leading edge: moving outward on its own side (left/top shrink,
            // right/bottom grow)
            let leading = if i < 2 { g < e.v } else { g > e.v };
            e.step_with_tau(if leading { LEAD_TAU } else { TRAIL_TAU });
        }
        let off = self
            .edges
            .iter()
            .zip(goal)
            .map(|(e, g)| (e.v - g).abs())
            .fold(0.0f32, f32::max);
        if !self.landed && off < LAND_PX {
            self.landed = true;
            self.arrived = Some(Instant::now());
        }
        let [l, tp, r, b] = self.edges.map(|e| e.v);
        (l, tp, r - l, b - tp)
    }

    fn moving(&self) -> bool {
        self.edges.iter().any(|e| e.moving())
            || self
                .arrived
                .is_some_and(|a| a.elapsed().as_secs_f32() < PULSE)
    }

    /// What to draw for these bounds: the area it covers (marked as damage
    /// here, for erasing next time) and the C++ frame's parameters.
    fn prepare(
        &self,
        c: &mut Canvas,
        (x, y, w, h): (f32, f32, f32, f32),
    ) -> ((f32, f32, f32, f32), crate::focus_fx::Params) {
        // 0 at rest, 1 right on arrival
        let pulse = self
            .arrived
            .map(|a| 1.0 - (a.elapsed().as_secs_f32() / PULSE).min(1.0))
            .unwrap_or(0.0);
        let pulse = pulse * pulse;
        // stretched: how much bigger than the card it is heading to
        let (tw, th) = (self.target.2.max(1.0), self.target.3.max(1.0));
        let stretch = ((w * h) / (tw * th) - 1.0).clamp(0.0, 1.0);
        let grow = 4.0 * pulse;
        let (x, y, w, h) = (x - grow, y - grow, w + 2.0 * grow, h + 2.0 * grow);
        let reach = (
            x - FOCUS_REACH,
            y - FOCUS_REACH,
            w + 2.0 * FOCUS_REACH,
            h + 2.0 * FOCUS_REACH,
        );
        c.mark_damage_rect(reach.0, reach.1, reach.2, reach.3);
        let params = crate::focus_fx::Params {
            x,
            y,
            w,
            h,
            r: 22.0,
            reach: FOCUS_REACH,
            wash: (22.0 + 30.0 * stretch + 26.0 * pulse) / 255.0,
            ring: ((185.0 + 70.0 * pulse) / 255.0).min(1.0),
            glow: (34.0 + 45.0 * stretch + 60.0 * pulse) / 255.0,
            top: ((70.0 + 120.0 * pulse) / 255.0).min(1.0),
            ring_w: 2.0,
            ..Default::default()
        };
        (reach, params)
    }
}

/// The Filme clapperboard: board and lower stick are part of the cached
/// card; the striped arm on top is drawn live so it can snap shut.
const CLAP_S: f32 = 96.0;
/// The whole snap with its stars, and when OK opens the films (just after
/// the board has closed).
const CLAP_TOTAL: f32 = 0.66;
const CLAP_OPEN_AFTER: f32 = 0.50;

fn clap_center(t: &Tile) -> (f32, f32) {
    (t.x + t.w / 2.0, t.y + t.h * 0.40)
}

/// Where the arm and the stars can be (restored before each frame of it).
fn clap_area(t: &Tile) -> (f32, f32, f32, f32) {
    let (x, y) = clap_center(t);
    let s = CLAP_S;
    (x - s * 0.52, y - s * 0.98, s * 1.40, s * 0.93)
}

/// The board's geometry: (left, width, top of the lower stick, stick height).
fn clap_geometry(t: &Tile) -> (f32, f32, f32, f32) {
    let (x, y) = clap_center(t);
    let s = CLAP_S;
    (x - s * 0.45, s * 0.9, y - s * 0.30, s * 0.15)
}

/// Gold bar with black diagonal stripes; `rot` turns every point (the arm).
fn striped_bar(
    c: &mut Canvas,
    bx: f32,
    by: f32,
    bw: f32,
    bh: f32,
    rot: impl Fn((f32, f32)) -> (f32, f32),
) {
    let corners = [(bx, by), (bx + bw, by), (bx + bw, by + bh), (bx, by + bh)];
    c.polygon(&corners.map(&rot), GOLD);
    let period = bw / 4.6;
    let (sw, slant) = (period * 0.5, bh * 0.6);
    for k in 0..4 {
        let xs = bx + period * 0.35 + k as f32 * period;
        let p = [
            (xs, by + bh),
            (xs + sw, by + bh),
            (xs + sw + slant, by),
            (xs + slant, by),
        ];
        c.polygon(&p.map(&rot), Rgba(22, 20, 18, 255));
    }
}

/// The board and the lower stick (into the cached card).
fn draw_clap_board(c: &mut Canvas, t: &Tile, over_poster: bool) {
    let (bx, bw, stick_y, bh) = clap_geometry(t);
    let s = CLAP_S;
    let body_y = stick_y + bh + 2.0;
    let body_h = s * 0.50;
    if over_poster {
        c.rect(
            bx - 6.0,
            stick_y - s * 0.24,
            bw + 12.0,
            body_y + body_h - stick_y + s * 0.30,
            14.0,
            Rgba(0, 0, 0, 110),
        );
    }
    c.rect(bx, body_y, bw, body_h, 6.0, Rgba(30, 27, 24, 245));
    c.stroke_rect(
        bx + 0.5,
        body_y + 0.5,
        bw - 1.0,
        body_h - 1.0,
        6.0,
        1.5,
        Rgba(GOLD.0, GOLD.1, GOLD.2, 150),
    );
    // the chalk lines: scene, take, roll
    let line = Rgba(GOLD.0, GOLD.1, GOLD.2, 90);
    let mid = bx + bw * 0.5;
    c.line(
        &[
            (bx + 8.0, body_y + body_h * 0.42),
            (bx + bw - 8.0, body_y + body_h * 0.42),
        ],
        1.2,
        line,
    );
    c.line(
        &[(mid, body_y + body_h * 0.42), (mid, body_y + body_h - 7.0)],
        1.2,
        line,
    );
    c.rect(
        bx + 10.0,
        body_y + 7.0,
        bw * 0.34,
        3.0,
        1.5,
        Rgba(GOLD.0, GOLD.1, GOLD.2, 120),
    );
    c.rect(
        bx + 10.0,
        body_y + body_h * 0.6,
        bw * 0.22,
        3.0,
        1.5,
        Rgba(GOLD.0, GOLD.1, GOLD.2, 80),
    );
    c.rect(
        mid + 8.0,
        body_y + body_h * 0.6,
        bw * 0.22,
        3.0,
        1.5,
        Rgba(GOLD.0, GOLD.1, GOLD.2, 80),
    );
    striped_bar(c, bx, stick_y, bw, bh, |p| p);
}

/// The arm's angle (radians, negative = open) and the stars' progress
/// (0..1, None before the board has closed) at `t` seconds into the snap.
fn clap_motion(t: f32) -> (f32, Option<f32>) {
    let open = -0.52; // about 30 degrees
    let ease_out = |k: f32| 1.0 - (1.0 - k).powi(3);
    let angle = if t < 0.20 {
        open * ease_out(t / 0.20)
    } else if t < 0.30 {
        open
    } else if t < 0.36 {
        // the snap: fast, accelerating into the board
        let k = (t - 0.30) / 0.06;
        open * (1.0 - k * k)
    } else if t < 0.46 {
        // a small bounce off the board
        let k = (t - 0.36) / 0.10;
        -0.07 * (std::f32::consts::PI * k).sin()
    } else {
        0.0
    };
    let stars = (t >= 0.36).then(|| ((t - 0.36) / (CLAP_TOTAL - 0.36)).min(1.0));
    (angle, stars)
}

/// The striped arm, hinged at the left, and the three little stars that
/// flash beside it as it closes. `t`: seconds into a snap, None at rest.
fn draw_clap_arm(c: &mut Canvas, t: &Tile, snap: Option<f32>) {
    let (bx, bw, stick_y, bh) = clap_geometry(t);
    let gap = 1.5;
    let (angle, stars) = snap.map(clap_motion).unwrap_or((0.0, None));
    let hinge = (bx, stick_y - gap);
    let (sn, cs) = angle.sin_cos();
    let rot = move |(x, y): (f32, f32)| {
        let (dx, dy) = (x - hinge.0, y - hinge.1);
        (hinge.0 + cs * dx - sn * dy, hinge.1 + sn * dx + cs * dy)
    };
    striped_bar(c, bx, stick_y - gap - bh, bw, bh, rot);
    c.circle(hinge.0 + 3.0, hinge.1 - 1.0, 2.6, Rgba(60, 52, 40, 255));
    if let Some(k) = stars {
        // three tiny four-pointed stars at the closing end, popping outward
        let tip = (bx + bw, stick_y - gap * 0.5);
        let pop = (std::f32::consts::PI * k).sin();
        for (dx, dy, size) in [(11.0, -13.0, 6.0), (17.0, -1.0, 4.6), (9.0, 10.0, 3.6)] {
            let d = 1.0 + 0.6 * k;
            let (sx, sy) = (tip.0 + dx * d, tip.1 + dy * d);
            let r = size * pop;
            if r < 0.3 {
                continue;
            }
            let q = r * 0.28;
            let pts = [
                (sx, sy - r),
                (sx + q, sy - q),
                (sx + r, sy),
                (sx + q, sy + q),
                (sx, sy + r),
                (sx - q, sy + q),
                (sx - r, sy),
                (sx - q, sy - q),
            ];
            c.polygon(&pts, Rgba(255, 236, 190, (255.0 * pop) as u8));
        }
    }
}
