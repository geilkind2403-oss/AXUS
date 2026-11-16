//! Weather: now, the next 24 hours, the week -- under a living sky.
//!
//! The upper part of the screen (the "band": title, current conditions,
//! metrics) is animated by the C++ renderer in `cpp/weather_fx.cpp`: it
//! draws the sky, the weather moving behind the glass cards and the hero
//! icon. The text and cards of the band are rasterised here once into a
//! transparent layer, which the renderer lays over the weather every frame.
//! The lower part (hours, days) is still and costs nothing per frame.
use super::*;
use crate::app::{Ctx, Nav, Screen};
use crate::gfx::{color, Align, Canvas, Image, LayerCache, Rgba, Weight};
use crate::weather::{Sky, Weather};
use crate::weather_fx::{Params, Scene};
use std::time::{Duration, Instant};

/// Height of the animated band (logical px). The hourly panel starts below.
const BAND: f32 = 350.0;
/// Hero icon: centre and size (logical px).
const ICON: (f32, f32, f32) = (154.0, 232.0, 118.0);
/// The title block (city, label, line) floats over the sky: x0, y0, x1, y1.
const TITLE: [f32; 4] = [PAD, 40.0, 330.0, 124.0];
/// The renderer is timed; slower than this per frame, it runs at 30 fps.
const SLOW_MS: f32 = 9.0;
const FAST_MS: f32 = 5.0;

/// One grid for every card: content from PAD to RIGHT, the same gap
/// between all cards and the same corners, so every edge lines up.
const RIGHT: f32 = PAD + 1152.0;
const GAP: f32 = 16.0;
const RADIUS: f32 = 22.0;
/// Top row: three metric cards on the right, the current conditions fill
/// the rest.
const METRIC_W: f32 = 180.0;
const METRIC_X: f32 = RIGHT - 3.0 * METRIC_W - 2.0 * GAP;
const HERO_W: f32 = METRIC_X - GAP - PAD;

/// The new sky fades in over the still one when its scene is ready.
const REVEAL: f32 = 0.4;

/// What a scene was made for: weather data, sunset warmth, frame size.
#[derive(Clone, Copy, PartialEq)]
struct SceneKey {
    data: (usize, u64),
    golden: i32,
    size: (u32, u32),
}

struct Fx {
    scene: Scene,
    sky: Image,
    /// The icon, still, for the frames before the band moves.
    icon: Option<Image>,
    /// The band's text and cards are in the renderer.
    layered: bool,
}

/// A scene being made on a worker thread: None until it is done, then the
/// scene (or None when it could not be made).
type Building = std::sync::Arc<std::sync::Mutex<Option<Option<Fx>>>>;

#[derive(Default)]
struct WarmScene {
    key: Option<SceneKey>,
    fx: Option<Fx>,
    building: Option<Building>,
}

fn warm_scene() -> &'static std::sync::Mutex<WarmScene> {
    static WARM: std::sync::OnceLock<std::sync::Mutex<WarmScene>> = std::sync::OnceLock::new();
    WARM.get_or_init(|| std::sync::Mutex::new(WarmScene::default()))
}

fn renderer_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .clamp(1, 4)
}

fn scene_input(c: &Canvas, cx: &Ctx, w: &Weather) -> (SceneKey, Params, Sky) {
    let (sky, _) = crate::weather::describe(w.code);
    let (top, bottom, _) = palette(sky, w.day);
    let (hour, minute, _) = crate::app::local_hms();
    let golden = if w.day {
        w.days
            .first()
            .map(|d| crate::weather::golden_hour(d, hour * 60 + minute))
            .unwrap_or(0.0)
    } else {
        0.0
    };
    let key = SceneKey {
        data: (
            std::sync::Arc::as_ptr(&cx.weather.0) as usize,
            cx.weather.revision(),
        ),
        golden: (golden * 10.0).round() as i32,
        size: (c.pm.width(), c.pm.height()),
    };
    let scale = c.pm.width() as f32 / crate::gfx::W as f32;
    let rgb = |c: Rgba| (c.0 as u32) << 16 | (c.1 as u32) << 8 | c.2 as u32;
    let params = Params {
        width: c.pm.width() as i32,
        height: c.pm.height() as i32,
        band_h: (BAND * scale).round() as i32,
        scale,
        sky: sky as i32,
        day: w.day as i32,
        intensity: crate::weather::intensity(w.code),
        wind: w.wind,
        golden: key.golden as f32 / 10.0,
        sky_top: rgb(top),
        sky_bottom: rgb(bottom),
        seed: 0x5eed + sky as u32 * 7919,
        icon_x: ICON.0 * scale,
        icon_y: ICON.1 * scale,
        icon_size: ICON.2 * scale,
        scrim: TITLE.map(|v| v * scale),
    };
    (key, params, sky)
}

fn build_scene(params: Params, threads: usize) -> Option<Fx> {
    let scene = Scene::new(params)?;
    let sky = scene.render_sky(threads)?;
    let side = (ICON.2 * 1.6 * params.scale).ceil() as u32;
    let icon = scene.icon_sprite(side, 0.0);
    Some(Fx {
        scene,
        sky,
        icon,
        layered: false,
    })
}

fn spawn_scene_build(
    building: Building,
    params: Params,
    threads: usize,
    sky: Sky,
    day: bool,
    tx: std::sync::mpsc::Sender<crate::app::Msg>,
) {
    std::thread::spawn(move || {
        let started = Instant::now();
        let fx = build_scene(params, threads);
        crate::log(&format!(
            "Wetter-Szene {:?} {}: {:.1} ms{}",
            sky,
            if day { "Tag" } else { "Nacht" },
            started.elapsed().as_secs_f64() * 1000.0,
            if fx.is_some() {
                ""
            } else {
                " (fehlgeschlagen)"
            }
        ));
        *building.lock().unwrap() = Some(fx);
        let _ = tx.send(crate::app::Msg::Redraw);
    });
}

/// Builds the current weather scene as soon as its data arrives, while Home
/// is visible. Opening Weather can then reuse the finished renderer and sky.
pub fn prewarm(c: &Canvas, cx: &Ctx) {
    let Some(Ok(w)) = cx.weather.get() else {
        return;
    };
    let (key, params, sky) = scene_input(c, cx, &w);
    let building = {
        let mut warm = warm_scene().lock().unwrap();
        if warm.key == Some(key) {
            return;
        }
        let building: Building = Default::default();
        warm.key = Some(key);
        warm.fx = None;
        warm.building = Some(building.clone());
        building
    };
    spawn_scene_build(
        building,
        params,
        renderer_threads(),
        sky,
        w.day,
        cx.tx.clone(),
    );
}

pub struct WeatherScreen {
    fx: Option<Fx>,
    fx_key: Option<SceneKey>,
    building: Option<Building>,
    key: Option<SceneKey>,
    prewarmed: bool,
    base_cache: LayerCache,
    /// When the scene arrived (the sky fades in, then the weather moves).
    ready_at: Option<Instant>,
    last_fx: Option<Instant>,
    /// Average time the renderer takes (ms), and whether that means 30 fps.
    cost_ms: f32,
    half_rate: bool,
    threads: usize,
}

impl WeatherScreen {
    pub fn new() -> WeatherScreen {
        WeatherScreen {
            fx: None,
            fx_key: None,
            building: None,
            key: None,
            prewarmed: false,
            base_cache: LayerCache::default(),
            ready_at: None,
            last_fx: None,
            cost_ms: 0.0,
            half_rate: false,
            threads: renderer_threads(),
        }
    }

    /// Starts making the scene (sky, clouds, rays) on a worker when the
    /// weather or the frame size changed -- on the Pi that takes a few
    /// hundred milliseconds, too long for the UI thread -- and picks it up
    /// when it is done.
    fn prepare(&mut self, c: &Canvas, cx: &Ctx, w: &Weather) {
        let (key, params, sky) = scene_input(c, cx, w);
        if self.key != Some(key) {
            self.key = Some(key);
            let (fx, building, spawn, prewarmed) = {
                let mut warm = warm_scene().lock().unwrap();
                if warm.key != Some(key) {
                    let building: Building = Default::default();
                    warm.key = Some(key);
                    warm.fx = None;
                    warm.building = Some(building.clone());
                    (None, Some(building), true, false)
                } else if let Some(fx) = warm.fx.take() {
                    warm.building = None;
                    (Some(fx), None, false, true)
                } else if let Some(building) = warm.building.clone() {
                    (None, Some(building), false, true)
                } else {
                    let building: Building = Default::default();
                    warm.building = Some(building.clone());
                    (None, Some(building), true, false)
                }
            };
            self.prewarmed = prewarmed;
            if let Some(fx) = fx {
                self.fx = Some(fx);
                self.fx_key = Some(key);
                self.ready_at = Some(Instant::now() - Duration::from_secs_f32(REVEAL));
                self.last_fx = None;
                self.base_cache = LayerCache::default();
            }
            self.building = building;
            if spawn {
                spawn_scene_build(
                    self.building.as_ref().unwrap().clone(),
                    params,
                    self.threads,
                    sky,
                    w.day,
                    cx.tx.clone(),
                );
            }
        }
        let done = self
            .building
            .as_ref()
            .and_then(|b| b.lock().unwrap().take());
        if let Some(fx) = done {
            self.building = None;
            self.fx_key = fx.as_ref().map(|_| key);
            self.fx = fx;
            let now = Instant::now();
            self.ready_at = Some(if self.prewarmed {
                now - Duration::from_secs_f32(REVEAL)
            } else {
                now
            });
            self.last_fx = None;
            self.base_cache = LayerCache::default();
        }
    }

    /// Time for the next animated frame (60 fps, 30 when the Pi is busy).
    fn frame_due(&self, now: Instant) -> bool {
        match self.last_fx {
            Some(last) if self.half_rate => now.duration_since(last).as_millis() >= 30,
            _ => true,
        }
    }

    /// How far the new sky has faded in (0..1).
    fn reveal(&self, now: Instant) -> f32 {
        self.ready_at
            .map(|r| (now.duration_since(r).as_secs_f32() / REVEAL).min(1.0))
            .unwrap_or(1.0)
    }
}

impl Drop for WeatherScreen {
    fn drop(&mut self) {
        let (Some(key), Some(fx)) = (self.fx_key.take(), self.fx.take()) else {
            return;
        };
        let mut warm = warm_scene().lock().unwrap();
        if warm.key == Some(key) {
            warm.fx = Some(fx);
            warm.building = None;
        }
    }
}

impl Screen for WeatherScreen {
    fn name(&self) -> &'static str {
        "weather"
    }
    fn key(&mut self, k: Key, cx: &mut Ctx) -> Nav {
        match k {
            Key::Back => Nav::Pop,
            Key::Home => Nav::Home,
            Key::Ok | Key::Menu => {
                let (lat, lon) = (cx.cfg.lat, cx.cfg.lon);
                cx.weather = crate::app::load(&cx.tx, move || crate::weather::fetch(lat, lon));
                Nav::None
            }
            _ => Nav::None,
        }
    }
    fn animating(&self, cx: &Ctx) -> bool {
        self.fx.is_some() && cx.weather.with(|w| matches!(w, Some(Ok(_))))
    }
    fn draw(&mut self, c: &mut Canvas, cx: &mut Ctx) {
        let w = match cx.weather.get() {
            Some(Ok(w)) => w,
            other => {
                self.fx = None;
                self.fx_key = None;
                self.building = None;
                self.key = None;
                draw_unavailable(c, cx, other);
                return;
            }
        };
        let (sky, desc) = crate::weather::describe(w.code);
        let (top, bottom, accent) = palette(sky, w.day);
        self.prepare(c, cx, &w);
        if self.fx.is_none() {
            // being made (or no memory for it): the still screen as before
            draw_still(c, cx, &w, desc, accent);
            return;
        }
        let now = Instant::now();
        let sliding = c.opacity < 1.0 || c.offset != (0.0, 0.0);
        if c.full_base_restore_required() {
            // an overlay (control centre) is drawn over the last frame
            self.base_cache = LayerCache::default();
        }

        // Once per weather: the band's text and cards into a transparent
        // layer for the renderer (needs a still canvas: not while sliding).
        let fx = self.fx.as_mut().unwrap();
        if !fx.layered && !sliding {
            c.invalidate();
            c.clear(Rgba(0, 0, 0, 0));
            c.clip_y(Some((0.0, BAND)));
            draw_band(c, &w, desc, accent, &cx.cfg.city);
            c.clip_y(None);
            c.finish();
            fx.layered = c
                .snapshot_region(0.0, 0.0, crate::gfx::W as f32, BAND)
                .map(|ui| fx.scene.set_layers(fx.sky.clone(), &ui))
                .unwrap_or(false);
            if !fx.layered {
                // keep the still screen; the scene is not tried again
                self.fx = None;
                draw_still(c, cx, &w, desc, accent);
                return;
            }
            self.base_cache = LayerCache::default();
        }

        // The whole still frame (cached: drawn again only when it changes).
        // While the scene's sky is new, it fades in over the plain gradient.
        let reveal = self.reveal(now);
        let fx = self.fx.as_ref().unwrap();
        c.clear(bottom);
        if reveal < 1.0 {
            c.vgradient(0.0, 0.0, 1280.0, 720.0, 0.0, top, bottom);
            let opacity = c.opacity;
            c.opacity *= crate::gfx::ease(reveal);
            c.image_exact(&fx.sky, 0.0, 0.0);
            c.opacity = opacity;
        } else {
            c.image_exact(&fx.sky, 0.0, 0.0);
        }
        draw_band(c, &w, desc, accent, &cx.cfg.city);
        status(c, cx);
        draw_lower(c, &w, accent);
        let still = sliding || !fx.layered || reveal < 1.0;
        if still {
            if let Some(icon) = fx.icon.as_ref() {
                let half = icon.width() as f32 / c_scale(c) * 0.5;
                c.image_exact(icon, ICON.0 - half, ICON.1 - half);
            }
        }
        let base_changed = c.finish_cached(&mut self.base_cache);
        if still {
            return;
        }

        // The living band, straight into the frame. Its clock starts when
        // the sky has faded in; the weather then fades in itself.
        if !base_changed && !self.frame_due(now) {
            return;
        }
        let t = self
            .ready_at
            .map(|r| (now.duration_since(r).as_secs_f32() - REVEAL).max(0.0))
            .unwrap_or(0.0);
        let fx = self.fx.as_mut().unwrap();
        fx.scene.advance(t);
        let t0 = Instant::now();
        fx.scene.render_band(&mut c.pm, self.threads);
        let ms = t0.elapsed().as_secs_f32() * 1000.0;
        self.cost_ms = if self.last_fx.is_none() {
            ms
        } else {
            self.cost_ms * 0.92 + ms * 0.08
        };
        if !self.half_rate && self.cost_ms > SLOW_MS {
            self.half_rate = true;
            crate::log(&format!(
                "Wetter-Animation: {:.1} ms pro Bild, weiter mit 30 fps",
                self.cost_ms
            ));
        } else if self.half_rate && self.cost_ms < FAST_MS {
            self.half_rate = false;
        }
        self.last_fx = Some(now);
        c.mark_damage_rect(0.0, 0.0, crate::gfx::W as f32, BAND);
        // the clock capsule stays live over the moving sky
        status(c, cx);
        c.finish_pending_damage();
    }
}

fn c_scale(c: &Canvas) -> f32 {
    c.pm.width() as f32 / crate::gfx::W as f32
}

/// Sky gradient (top, bottom) and the accent of labels and bars.
fn palette(sky: Sky, day: bool) -> (Rgba, Rgba, Rgba) {
    match (sky, day) {
        (_, false) => (
            Rgba::hex(0x17213d),
            Rgba::hex(0x090d1b),
            Rgba::hex(0x9cbcff),
        ),
        (Sky::Clear, _) => (
            Rgba::hex(0x276aa4),
            Rgba::hex(0x101d38),
            Rgba::hex(0xffcf64),
        ),
        (Sky::PartlyCloudy, _) => (
            Rgba::hex(0x315e8b),
            Rgba::hex(0x111d35),
            Rgba::hex(0x9ed1ff),
        ),
        (Sky::Thunder, _) => (
            Rgba::hex(0x373248),
            Rgba::hex(0x10101a),
            Rgba::hex(0xc6b5ff),
        ),
        _ => (
            Rgba::hex(0x35475d),
            Rgba::hex(0x101722),
            Rgba::hex(0x9bc7eb),
        ),
    }
}

fn clock(min: u16) -> String {
    format!("{:02}:{:02}", min / 60, min % 60)
}

/// Title, current conditions and metrics (the animated band), without the
/// hero icon (the renderer draws it).
fn draw_band(c: &mut Canvas, w: &Weather, desc: &str, accent: Rgba, city: &str) {
    // the title floats over moving weather: a soft shadow keeps it legible
    let titles: [(&str, f32, u32, Weight, Rgba); 3] = [
        (city, 42.0, 34, Weight::Bold, color::WHITE),
        ("WETTER JETZT", 84.0, 13, Weight::Bold, accent),
        (
            "Vorhersage für die nächsten Tage",
            105.0,
            15,
            Weight::Regular,
            Rgba(228, 235, 248, 190),
        ),
    ];
    for (text, y, px, weight, col) in titles {
        let shadow = Rgba(6, 12, 28, 46);
        c.text(text, PAD, y + 1.0, px, weight, shadow, Align::Left);
        c.text(text, PAD, y, px, weight, col, Align::Left);
    }

    // current conditions: a glass card the weather shows through
    c.vgradient(
        PAD,
        142.0,
        HERO_W,
        190.0,
        RADIUS,
        Rgba(255, 255, 255, 34),
        Rgba(255, 255, 255, 14),
    );
    c.stroke_rect(
        PAD,
        142.0,
        HERO_W,
        190.0,
        RADIUS,
        1.0,
        Rgba(255, 255, 255, 45),
    );
    c.text(
        &format!("{:.0}°", w.temp),
        242.0,
        163.0,
        86,
        Weight::Bold,
        color::WHITE,
        Align::Left,
    );
    c.text(
        desc,
        252.0,
        255.0,
        21,
        Weight::Semi,
        color::WHITE,
        Align::Left,
    );
    let today = w.days.first();
    c.text(
        &today
            .map(|d| format!("Heute  ·  {:.0}° / {:.0}°", d.max, d.min))
            .unwrap_or_else(|| "Aktuelle Bedingungen".into()),
        252.0,
        289.0,
        15,
        Weight::Regular,
        Rgba(237, 242, 251, 190),
        Align::Left,
    );
    // sunrise and sunset, at the right of the card
    if let Some((rise, set)) = today.and_then(|d| Some((d.sunrise?, d.sunset?))) {
        let x = PAD + HERO_W - 130.0;
        c.line(
            &[(x - 20.0, 172.0), (x - 20.0, 302.0)],
            1.0,
            Rgba(255, 255, 255, 34),
        );
        for (i, (label, value)) in [("AUFGANG", rise), ("UNTERGANG", set)].iter().enumerate() {
            let y = 176.0 + i as f32 * 66.0;
            c.text(
                label,
                x,
                y,
                11,
                Weight::Bold,
                Rgba(230, 237, 250, 165),
                Align::Left,
            );
            c.text(
                &clock(*value),
                x,
                y + 18.0,
                22,
                Weight::Semi,
                color::WHITE,
                Align::Left,
            );
        }
    }

    let metrics = [
        ("GEFÜHLT", format!("{:.0}°", w.feels), "Temperatur"),
        ("WIND", format!("{:.0}", w.wind), "km/h"),
        (
            "LUFTFEUCHTE",
            format!("{:.0}%", w.humidity),
            "Relative Feuchte",
        ),
    ];
    for (i, (label, value, unit)) in metrics.iter().enumerate() {
        let x = METRIC_X + i as f32 * (METRIC_W + GAP);
        c.rect(x, 142.0, METRIC_W, 190.0, RADIUS, Rgba(255, 255, 255, 19));
        c.stroke_rect(
            x,
            142.0,
            METRIC_W,
            190.0,
            RADIUS,
            1.0,
            Rgba(255, 255, 255, 32),
        );
        c.rect(x + 18.0, 161.0, 28.0, 3.0, 1.5, accent);
        c.text(
            label,
            x + 18.0,
            180.0,
            12,
            Weight::Bold,
            Rgba(230, 237, 250, 175),
            Align::Left,
        );
        c.text(
            value,
            x + 18.0,
            215.0,
            35,
            Weight::Bold,
            color::WHITE,
            Align::Left,
        );
        c.text(
            unit,
            x + 18.0,
            273.0,
            13,
            Weight::Regular,
            Rgba(230, 237, 250, 165),
            Align::Left,
        );
    }
}

/// Hours and days (still).
fn draw_lower(c: &mut Canvas, w: &Weather, accent: Rgba) {
    c.rect(
        PAD,
        356.0,
        RIGHT - PAD,
        137.0,
        RADIUS,
        Rgba(18, 25, 42, 167),
    );
    c.text(
        "STUNDENVERLAUF",
        PAD + 22.0,
        370.0,
        12,
        Weight::Bold,
        accent,
        Align::Left,
    );
    let hours: Vec<_> = w.hours.iter().take(25).step_by(3).take(8).collect();
    let step = 1108.0 / hours.len().max(1) as f32;
    for (i, h) in hours.iter().enumerate() {
        let x = PAD + 22.0 + step * (i as f32 + 0.5);
        let time = if i == 0 {
            "JETZT".to_string()
        } else {
            format!("{:02}:00", h.hour)
        };
        c.text(
            &time,
            x,
            394.0,
            12,
            Weight::Semi,
            Rgba(235, 241, 251, 210),
            Align::Center,
        );
        icon_weather(c, crate::weather::describe(h.code).0, h.day, x, 435.0, 34.0);
        c.text(
            &format!("{:.0}°", h.temp),
            x,
            455.0,
            16,
            Weight::Bold,
            color::WHITE,
            Align::Center,
        );
        if h.rain >= 20 {
            c.text(
                &format!("{}% Regen", h.rain),
                x,
                476.0,
                10,
                Weight::Regular,
                Rgba(151, 204, 255, 255),
                Align::Center,
            );
        }
    }

    c.text(
        "DIE NÄCHSTEN 7 TAGE",
        PAD,
        514.0,
        13,
        Weight::Bold,
        accent,
        Align::Left,
    );
    let days: Vec<_> = w.days.iter().take(7).collect();
    let (lo, hi) = days.iter().fold((f32::MAX, f32::MIN), |a, d| {
        (a.0.min(d.min), a.1.max(d.max))
    });
    let n = days.len().max(1) as f32;
    let dw = (RIGHT - PAD - (n - 1.0) * GAP) / n;
    for (i, d) in days.iter().enumerate() {
        let x = PAD + (dw + GAP) * i as f32;
        let y = 539.0;
        c.rect(
            x,
            y,
            dw,
            134.0,
            RADIUS,
            Rgba(255, 255, 255, if i == 0 { 31 } else { 17 }),
        );
        c.stroke_rect(x, y, dw, 134.0, RADIUS, 1.0, Rgba(255, 255, 255, 25));
        let name = if i == 0 {
            "HEUTE".to_string()
        } else {
            crate::app::WEEKDAYS[d.weekday][..2]
                .to_string()
                .to_uppercase()
        };
        c.text(
            &name,
            x + dw / 2.0,
            y + 12.0,
            12,
            Weight::Bold,
            Rgba(228, 235, 248, 195),
            Align::Center,
        );
        icon_weather(
            c,
            crate::weather::describe(d.code).0,
            true,
            x + dw / 2.0,
            y + 53.0,
            36.0,
        );
        c.text(
            &format!("{:.0}°", d.max),
            x + dw / 2.0 - 22.0,
            y + 82.0,
            16,
            Weight::Bold,
            color::WHITE,
            Align::Center,
        );
        c.text(
            &format!("{:.0}°", d.min),
            x + dw / 2.0 + 23.0,
            y + 83.0,
            14,
            Weight::Regular,
            Rgba(220, 228, 241, 160),
            Align::Center,
        );
        let bw = dw - 40.0;
        let bx = x + 20.0;
        let span = (hi - lo).max(1.0);
        c.rect(bx, y + 112.0, bw, 5.0, 3.0, Rgba(255, 255, 255, 32));
        let start = (d.min - lo) / span;
        let width = (d.max - d.min) / span * bw;
        c.rect(bx + start * bw, y + 112.0, width.max(8.0), 5.0, 3.0, accent);
        if d.rain >= 30 {
            c.text(
                &format!("{}%", d.rain),
                x + dw - 12.0,
                y + 13.0,
                10,
                Weight::Regular,
                Rgba(151, 204, 255, 255),
                Align::Right,
            );
        }
    }
    c.text(
        "OK  Aktualisieren     ·     ZURÜCK  Schließen",
        640.0,
        699.0,
        12,
        Weight::Semi,
        Rgba(235, 241, 251, 155),
        Align::Center,
    );
}

/// Without the C++ scene: the still screen (gradient, vector icon).
fn draw_still(c: &mut Canvas, cx: &Ctx, w: &Weather, desc: &str, accent: Rgba) {
    let (sky, _) = crate::weather::describe(w.code);
    let (top, bottom, _) = palette(sky, w.day);
    c.vgradient(0.0, 0.0, 1280.0, 720.0, 0.0, top, bottom);
    status(c, cx);
    draw_band(c, w, desc, accent, &cx.cfg.city);
    icon_weather(c, sky, w.day, ICON.0, ICON.1, 112.0);
    draw_lower(c, w, accent);
}

/// Loading, or the service could not be reached.
fn draw_unavailable(c: &mut Canvas, cx: &Ctx, other: Option<Result<Weather, String>>) {
    c.backdrop();
    status(c, cx);
    c.text(
        "Wetter",
        PAD,
        44.0,
        34,
        Weight::Bold,
        color::TEXT,
        Align::Left,
    );
    c.rect(PAD, 116.0, 1152.0, 480.0, 26.0, Rgba(255, 255, 255, 13));
    let (heading, msg) = match other {
        Some(Err(e)) => (
            "Wetter gerade nicht verfügbar",
            format!("{e}\n\nPrüfe die Netzwerkverbindung oder lade die Daten erneut."),
        ),
        _ => (
            "Vorhersage wird geladen",
            "Die Wetterdaten für deinen Standort werden abgerufen.".into(),
        ),
    };
    c.circle(640.0, 275.0, 72.0, Rgba(103, 169, 255, 22));
    c.text(
        heading,
        640.0,
        374.0,
        28,
        Weight::Bold,
        color::TEXT,
        Align::Center,
    );
    c.paragraph(
        &msg,
        320.0,
        423.0,
        640.0,
        18,
        Weight::Regular,
        color::MUTED,
        28.0,
        3,
    );
    c.text(
        "OK  Aktualisieren     ·     ZURÜCK  Schließen",
        640.0,
        625.0,
        15,
        Weight::Semi,
        color::MUTED,
        Align::Center,
    );
}
