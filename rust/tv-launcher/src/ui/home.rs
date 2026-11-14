//! The home screen: big tiles for what a TV is used for.
use super::*;
use crate::app::{Ctx, Nav, Screen};
use crate::gfx::{color, Align, Canvas, Image, LayerCache, Rgba, Weight};
use std::time::Instant;

pub struct Home {
    focus: usize,
    frame: Focus,
    tile_canvas: Canvas,
    tile_images: [Option<crate::gfx::Image>; 6],
    tile_image_sources: [Vec<String>; 6],
    tile_cache_keys: [Option<(u64, u64)>; 6],
    base_cache: LayerCache,
    base_image: Option<Image>,
    status_key: Option<(i32, i32, u64, u64, String, bool)>,
    news_overlay_key: Option<u64>,
    last_focus_rect: Option<(f32, f32, f32, f32)>,
    ambient_focus: usize,
    focus_changed_at: Instant,
}

#[derive(Clone, Copy)]
struct Tile {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

const TILES: [Tile; 6] = [
    Tile {
        x: 64.0,
        y: 118.0,
        w: 368.0,
        h: 252.0,
    },
    Tile {
        x: 456.0,
        y: 118.0,
        w: 368.0,
        h: 252.0,
    },
    Tile {
        x: 848.0,
        y: 118.0,
        w: 368.0,
        h: 252.0,
    },
    Tile {
        x: 64.0,
        y: 394.0,
        w: 368.0,
        h: 236.0,
    },
    Tile {
        x: 456.0,
        y: 394.0,
        w: 520.0,
        h: 236.0,
    },
    Tile {
        x: 1000.0,
        y: 394.0,
        w: 216.0,
        h: 236.0,
    },
];

// Clock, network and the small weather summary change independently from
// the Home artwork. Keep their raster damage to this compact top-right area.
const STATUS_DAMAGE: (f32, f32, f32, f32) = (880.0, 16.0, 352.0, 96.0);

impl Home {
    pub fn new(last_media_tile: usize) -> Home {
        Home {
            focus: last_media_tile.min(5),
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
            ambient_focus: last_media_tile.min(5),
            focus_changed_at: Instant::now(),
        }
    }

    /// Cache the six content tiles separately from the changing ambient and
    /// focus ring. The thumbnails and card artwork rarely change; keeping
    /// their rasterized pixels avoids redrawing every icon and label on each
    /// remote-button press.
    fn refresh_tile_cache(&mut self, cx: &mut Ctx, i: usize) {
        let t = TILES[i];
        let sources = tile_image_sources(cx, i);
        self.tile_canvas.resize(t.w as u32, t.h as u32);
        self.tile_canvas.offset = (-t.x, -t.y);
        self.tile_canvas.opacity = 1.0;
        self.tile_canvas.clear(Rgba(0, 0, 0, 0));
        draw_tile_content(&mut self.tile_canvas, cx, i, &t);
        self.tile_canvas.finish();
        self.tile_images[i] = Some(self.tile_canvas.snapshot());
        self.tile_image_sources[i] = sources;
        self.tile_cache_keys[i] = Some(tile_cache_key(cx, i, &self.tile_image_sources[i]));
    }

    /// What the backdrop glows with: the focused tile's picture or colour.
    fn ambient(&self, cx: &mut Ctx, focus: usize) -> crate::gfx::Ambient {
        use crate::gfx::Ambient;
        let img = |cx: &mut Ctx, src: &str| {
            if src.is_empty() {
                None
            } else {
                cx.images.get(src, 760, 520)
            }
        };
        match focus {
            0 => Ambient::Color(Rgba::hex(0xd0202c)),
            1 => Ambient::Color(Rgba::hex(0x2a7fd8)),
            2 => {
                let pick = films_pick(cx);
                let src = pick
                    .map(|f| {
                        if f.banner.is_empty() {
                            f.icon
                        } else {
                            f.banner
                        }
                    })
                    .unwrap_or_default();
                img(cx, &src)
                    .map(Ambient::Image)
                    .unwrap_or(Ambient::Color(Rgba::hex(0xc8963c)))
            }
            3 => match cx.weather.get() {
                Some(Ok(w)) if !w.day => Ambient::Color(Rgba::hex(0x2c3a8a)),
                _ => Ambient::Color(Rgba::hex(0x3d8be0)),
            },
            4 => {
                let src = cx.news.with(|items| {
                    let Some(items) = items.as_ref() else {
                        return String::new();
                    };
                    let n = items.len().min(6);
                    if n == 0 {
                        String::new()
                    } else {
                        // Home shows the lead headline; the News page contains
                        // the full list. Avoid replacing this large tile on a
                        // timer while the user is navigating the launcher.
                        items[0].image.clone()
                    }
                });
                img(cx, &src)
                    .map(Ambient::Image)
                    .unwrap_or(Ambient::Color(Rgba::hex(0x0b4fa3)))
            }
            _ => Ambient::Color(Rgba::hex(0x6b6b78)),
        }
    }

    fn open(&self, cx: &mut Ctx) -> Nav {
        match self.focus {
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
            _ => Nav::Push(Box::new(super::apps::Apps::new(&cx.cfg.last_app_id))),
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
        self.focus = match (k, f) {
            // top row: YouTube, Live TV, Filme -- bottom row: Wetter, Nachrichten, Apps
            (Key::Right, 0..=1) | (Key::Right, 3..=4) => f + 1,
            (Key::Left, 1..=2) | (Key::Left, 4..=5) => f - 1,
            (Key::Down, 0) => 3,
            (Key::Down, 1) => 4,
            (Key::Down, 2) => 5,
            (Key::Up, 3) => 0,
            (Key::Up, 4) => 1,
            (Key::Up, 5) => 2,
            (Key::ChUp, _) | (Key::ChDown, _) => {
                return Nav::Push(Box::new(super::livetv::LiveTv::new(cx)))
            }
            (Key::Ok, _) => return self.open(cx),
            (Key::Menu, _) => return Nav::Push(Box::new(super::settings::Settings::new(cx))),
            _ => f,
        };
        if self.focus != f {
            self.focus_changed_at = Instant::now();
        }
        Nav::None
    }

    fn animating(&self, _cx: &Ctx) -> bool {
        self.frame.moving()
    }

    fn draw(&mut self, c: &mut Canvas, cx: &mut Ctx) {
        let trace_started = Instant::now();
        for i in 0..TILES.len() {
            if self.tile_cache_keys[i] != Some(tile_cache_key(cx, i, &self.tile_image_sources[i])) {
                self.refresh_tile_cache(cx, i);
            }
        }
        // Let rapid remote navigation finish before changing the full-screen
        // ambience; the focus itself can then move over a retained base layer.
        if self.ambient_focus != self.focus
            && self.focus_changed_at.elapsed() >= std::time::Duration::from_millis(280)
        {
            self.ambient_focus = self.focus;
        }
        let amb = self.ambient(cx, self.ambient_focus);
        c.ambient(amb);
        // The greeting and date are stable for the whole day, so they stay in
        // the retained base. Fast-changing status is drawn below as an overlay.
        let tm = crate::app::local_tm();
        c.text(
            greeting(),
            PAD,
            30.0,
            34,
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
            74.0,
            16,
            Weight::Regular,
            color::MUTED,
            Align::Left,
        );
        // Fold the six retained card images into the base command list before
        // rasterizing. Previously the background and cards each forced a
        // separate full-screen pass on Home's first frame.
        for (i, t) in TILES.iter().enumerate() {
            if let Some(img) = self.tile_images[i].as_ref() {
                c.image_exact(img, t.x, t.y);
            }
        }
        let base_changed = c.finish_cached(&mut self.base_cache);
        let mut base_restored = base_changed;
        if base_changed {
            self.base_image = Some(c.snapshot());
            self.last_focus_rect = None;
        } else if let Some(base) = self.base_image.as_ref() {
            if c.full_base_restore_required() {
                c.restore_image(base);
                self.last_focus_rect = None;
                base_restored = true;
            } else if let Some((x, y, w, h)) = self.last_focus_rect {
                c.restore_focus(base, x, y, w, h);
            }
        }
        let status_key = home_status_key(cx);
        if base_restored || self.status_key.as_ref() != Some(&status_key) {
            if let Some(base) = self.base_image.as_ref() {
                c.restore_rect(
                    base,
                    STATUS_DAMAGE.0,
                    STATUS_DAMAGE.1,
                    STATUS_DAMAGE.2,
                    STATUS_DAMAGE.3,
                );
                draw_home_status(c, cx);
                self.status_key = Some(status_key);
            }
        }
        let news_key = cx.news.revision();
        if base_restored || self.news_overlay_key != Some(news_key) {
            if !base_restored {
                if let Some(base) = self.base_image.as_ref() {
                    let t = &TILES[4];
                    let iw = news_tile_photo_width(cx, t);
                    if iw > 0.0 {
                        c.restore_rect(
                            base,
                            t.x + iw + 12.0,
                            t.y + 12.0,
                            t.w - iw - 12.0,
                            t.h - 24.0,
                        );
                    } else {
                        c.restore_rect(base, t.x, t.y, t.w, t.h);
                    }
                }
            }
            draw_news_tile_overlay(c, cx, &TILES[4]);
            self.news_overlay_key = Some(news_key);
        }
        let t = &TILES[self.focus];
        let bounds = self.frame.move_to(t.x, t.y, t.w, t.h, true);
        super::focus_ring_fast(c, bounds.0, bounds.1, bounds.2, bounds.3);
        if base_restored {
            // The full base is already drawn. Raster the small dynamic status,
            // headline and focus overlay once without another full worker pass.
            c.finish_pending();
        } else {
            c.finish_pending_damage();
        }
        self.last_focus_rect = Some(bounds);
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
        let t = &TILES[self.focus];
        let bounds = self.frame.move_to(t.x, t.y, t.w, t.h, true);
        super::focus_ring_fast(c, bounds.0, bounds.1, bounds.2, bounds.3);
        c.finish_pending_damage();
        self.last_focus_rect = Some(bounds);
    }
}

fn draw_home_status(c: &mut Canvas, cx: &Ctx) {
    status(c, cx);
    if let Some(Ok(w)) = cx.weather.get() {
        let (sky, _) = crate::weather::describe(w.code);
        let right = crate::gfx::W as f32 - PAD;
        let t = format!("{:.0}°", w.temp);
        let tw = c.fonts.width(&t, 18, Weight::Semi);
        c.text(&t, right, 72.0, 18, Weight::Semi, color::TEXT, Align::Right);
        icon_weather(c, sky, w.day, right - tw - 18.0, 84.0, 26.0);
        c.text(
            &cx.cfg.city,
            right - tw - 38.0,
            74.0,
            15,
            Weight::Regular,
            color::MUTED,
            Align::Right,
        );
    }
}

fn home_status_key(cx: &Ctx) -> (i32, i32, u64, u64, String, bool) {
    let (hour, minute, _) = crate::app::local_hms();
    (
        hour,
        minute,
        cx.weather.revision(),
        cx.net.revision(),
        cx.cfg.city.clone(),
        cx.cfg.remote_enabled,
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
    glass(c, t.x, t.y, t.w, t.h, 22.0);
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
            label(c, t, "YouTube", "Suchen, ansehen");
        }
        1 => draw_livetv_tile(c, cx, t),
        2 => draw_films_tile(c, cx, t),
        3 => draw_weather_tile(c, cx, t),
        4 => draw_news_tile_background(c, cx, t),
        _ => {
            let (cx, cy) = (t.x + t.w / 2.0, t.y + t.h * 0.36);
            for (dx, dy) in [(-14.0, -14.0), (14.0, -14.0), (-14.0, 14.0), (14.0, 14.0)] {
                c.rect(cx + dx - 9.0, cy + dy - 9.0, 18.0, 18.0, 5.0, color::MUTED);
            }
            label(c, t, "Apps", "Apps · Einstellungen");
        }
    }
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
            color::ACCENT,
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
        c.text_fit(
            &n.topline,
            x,
            t.y + 58.0,
            14,
            Weight::Semi,
            color::ACCENT,
            w,
        );
    }
    c.paragraph(
        &n.title,
        x,
        t.y + 80.0,
        w,
        20,
        Weight::Bold,
        color::TEXT,
        26.0,
        4,
    );
    c.text(
        &crate::news::time_of(&n.date),
        x,
        t.y + t.h - 34.0,
        14,
        Weight::Regular,
        color::MUTED,
        Align::Left,
    );
    for k in 0..count {
        c.circle(
            x + w - 70.0 + k as f32 * 12.0,
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
