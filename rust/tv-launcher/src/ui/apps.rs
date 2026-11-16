//! Fast app switcher. Rust owns input and launches; the C++ wheel is only a
//! lightweight renderer over images cached once for the life of the process.
use super::*;
use crate::app::{load, Ctx, Nav, Screen, Slot};
use crate::gfx::{Canvas, Image, Rgba};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

const ENTRY_DURATION: Duration = Duration::from_millis(250);
const LAUNCH_ANIMATION_SECONDS: f32 = 0.72;

const APPS: [(&str, &str); 5] = [
    ("YouTube TV", "youtube"),
    ("Prime Video", "prime"),
    ("ARD Mediathek", "ard"),
    ("ZDF Live", "zdf"),
    ("ARTE Live", "arte"),
];

#[derive(Clone, Default)]
pub(super) struct AppAssets {
    pub icons: [Option<Image>; 4],
    pub glass: Option<Image>,
    pub logo_glow: Option<Image>,
    pub center_well: Option<Image>,
}

static APP_ASSETS: OnceLock<AppAssets> = OnceLock::new();
static APP_ASSETS_WARMING: AtomicBool = AtomicBool::new(false);

fn warm_glass() -> Option<Image> {
    // Build the annulus as a tiny, cached software material rather than a
    // fixed outlined PNG. Broad, angle-dependent reflections and a soft rim
    // let the warm backdrop show through like liquid glass. This runs once on
    // the asset worker; navigation frames only composite the finished texture.
    const SIZE: usize = 512;
    let mut texture = tiny_skia::Pixmap::new(SIZE as u32, SIZE as u32)?;
    let data = texture.data_mut();
    let smooth = |a: f32, b: f32, x: f32| {
        let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let glow = |x: f32| (-x * x).exp();
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 + 0.5 - SIZE as f32 * 0.5;
            let dy = y as f32 + 0.5 - SIZE as f32 * 0.5;
            let radius = (dx * dx + dy * dy).sqrt();
            let angle = dy.atan2(dx);
            let outer = smooth(256.0, 246.0, radius);
            let inner = smooth(106.0, 116.0, radius);
            let mask = outer * inner;
            if mask < 0.003 {
                continue;
            }

            let top_left = glow((angle + 2.15) / 0.72);
            let top_right = glow((angle + 0.62) / 0.82);
            let lower_glint = glow((angle - 1.45) / 0.92);
            let outer_rim = glow((radius - 249.0) / 6.5);
            let inner_rim = glow((radius - 114.0) / 6.0);
            let sheen = (0.10 + top_left * 0.56 + top_right * 0.39 + lower_glint * 0.17)
                .clamp(0.0, 1.0);
            let alpha = (mask * (33.0 + sheen * 39.0 + outer_rim * sheen * 63.0
                + inner_rim * sheen * 35.0)) as u32;
            let warm = (1.0 - (dy / 256.0).clamp(-1.0, 1.0) * 0.10).clamp(0.0, 1.0);
            let red = (181.0 + 70.0 * sheen * warm).clamp(0.0, 255.0) as u32;
            let green = (159.0 + 62.0 * sheen * warm).clamp(0.0, 255.0) as u32;
            let blue = (117.0 + 53.0 * sheen * warm).clamp(0.0, 255.0) as u32;
            let offset = (y * SIZE + x) * 4;
            data[offset] = (blue * alpha / 255) as u8;
            data[offset + 1] = (green * alpha / 255) as u8;
            data[offset + 2] = (red * alpha / 255) as u8;
            data[offset + 3] = alpha.min(255) as u8;
        }
    }
    Some(Arc::new(texture))
}

fn logo_glow() -> Option<Image> {
    const SIZE: usize = 128;
    let mut texture = tiny_skia::Pixmap::new(SIZE as u32, SIZE as u32)?;
    for (index, pixel) in texture.data_mut().chunks_exact_mut(4).enumerate() {
        let x = (index % SIZE) as f32 + 0.5 - SIZE as f32 * 0.5;
        let y = (index / SIZE) as f32 + 0.5 - SIZE as f32 * 0.5;
        let r = (x * x + y * y).sqrt() / (SIZE as f32 * 0.5);
        let alpha = (68.0 * (-r * r * 8.0).exp()) as u32;
        pixel[0] = (117 * alpha / 255) as u8;
        pixel[1] = (187 * alpha / 255) as u8;
        pixel[2] = (255 * alpha / 255) as u8;
        pixel[3] = alpha as u8;
    }
    Some(Arc::new(texture))
}

fn center_well() -> Option<Image> {
    const SIZE: usize = 256;
    let mut texture = tiny_skia::Pixmap::new(SIZE as u32, SIZE as u32)?;
    let data = texture.data_mut();
    let smooth = |a: f32, b: f32, x: f32| {
        let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 + 0.5 - SIZE as f32 * 0.5;
            let dy = y as f32 + 0.5 - SIZE as f32 * 0.5;
            let r = (dx * dx + dy * dy).sqrt() / (SIZE as f32 * 0.5);
            let coverage = 1.0 - smooth(0.78, 1.0, r);
            if coverage <= 0.001 {
                continue;
            }
            let inner_glow = (-((r - 0.91) / 0.075).powi(2)).exp();
            let alpha = (213.0 * coverage) as u32;
            let red = (9.0 + inner_glow * 35.0) as u32;
            let green = (10.0 + inner_glow * 29.0) as u32;
            let blue = (12.0 + inner_glow * 18.0) as u32;
            let offset = (y * SIZE + x) * 4;
            data[offset] = (blue * alpha / 255) as u8;
            data[offset + 1] = (green * alpha / 255) as u8;
            data[offset + 2] = (red * alpha / 255) as u8;
            data[offset + 3] = alpha.min(255) as u8;
        }
    }
    Some(Arc::new(texture))
}

fn softened_icon(bytes: &[u8]) -> Option<Image> {
    let mut image = crate::gfx::decode_image(bytes, 128, 128)?;
    let data = Arc::make_mut(&mut image).data_mut();
    for y in 0..128usize {
        for x in 0..128usize {
            let dx = x as f32 + 0.5 - 64.0;
            let dy = y as f32 + 0.5 - 64.0;
            // Circular, feathered app badges avoid sharp square corners in
            // both the wheel and the launch zoom animation.
            let distance = dx.hypot(dy) - 52.0;
            let edge = (distance / 12.0).clamp(0.0, 1.0);
            let coverage = 1.0 - edge * edge * (3.0 - 2.0 * edge);
            let pixel = &mut data[(y * 128 + x) * 4..(y * 128 + x + 1) * 4];
            for channel in pixel {
                *channel = (*channel as f32 * coverage).round() as u8;
            }
        }
    }
    Some(image)
}

fn cached_assets() -> AppAssets {
    APP_ASSETS
        .get_or_init(|| AppAssets {
            icons: [
                include_bytes!("../../assets/apps/youtube.png").as_slice(),
                include_bytes!("../../assets/apps/prime.png").as_slice(),
                include_bytes!("../../assets/apps/ard.png").as_slice(),
                include_bytes!("../../assets/apps/zdf.png").as_slice(),
            ]
            .map(softened_icon),
            glass: warm_glass(),
            logo_glow: logo_glow(),
            center_well: center_well(),
        })
        .clone()
}

/// Build the wheel's material and app badges during Home idle time.
pub fn prewarm(cx: &Ctx) {
    if APP_ASSETS.get().is_some()
        || APP_ASSETS_WARMING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
    {
        return;
    }
    drop(load(&cx.tx, || {
        let assets = cached_assets();
        APP_ASSETS_WARMING.store(false, Ordering::Release);
        assets
    }));
}

pub struct Apps {
    focus: usize,
    assets: Slot<AppAssets>,
    loaded_assets: Option<AppAssets>,
    browser_installed: bool,
    wheel: apps_wheel::Animation,
    wheel_base: Option<Image>,
    previous_items: Option<[(f32, f32); 5]>,
    launch_started: Option<Instant>,
    entry_started: Instant,
    entry_backdrop: Option<Image>,
}

impl Apps {
    pub fn wheel(cx: &Ctx, last_app_id: &str) -> Self {
        let focus = APPS
            .iter()
            .position(|app| app.1 == last_app_id)
            .filter(|&index| index != 2)
            .unwrap_or(0);
        Self {
            focus,
            // An un-cached first visit never blocks the remote or Home frame.
            // Reopening returns the shared, already-decoded Arc image handles.
            assets: load(&cx.tx, cached_assets),
            loaded_assets: None,
            browser_installed: super::browser::installed(),
            wheel: apps_wheel::Animation::default(),
            wheel_base: None,
            previous_items: None,
            launch_started: None,
            entry_started: Instant::now(),
            entry_backdrop: None,
        }
    }

    fn open(&self, cx: &mut Ctx) -> Nav {
        match self.focus {
            4 => Nav::Push(Box::new(super::livetv::LiveTv::new_arte(cx))),
            3 => Nav::Push(Box::new(super::livetv::LiveTv::new_zdf(cx))),
            2 => Nav::None,
            0 | 1 if !self.browser_installed => Nav::None,
            0 => Nav::Push(Box::new(super::browser::Browser::new(
                super::browser::YOUTUBE_TV,
            ))),
            1 => Nav::Push(Box::new(super::browser::Browser::new(
                super::browser::PRIME_VIDEO,
            ))),
            _ => Nav::None,
        }
    }

    fn entry_progress(&self) -> Option<f32> {
        let elapsed = self.entry_started.elapsed();
        (self.entry_backdrop.is_some() && elapsed < ENTRY_DURATION)
            .then_some(elapsed.as_secs_f32() / ENTRY_DURATION.as_secs_f32())
    }

    fn detail(&self) -> (&'static str, Rgba) {
        match self.focus {
            0 | 1 if !self.browser_installed => (
                "Browser-Addon fehlt",
                Rgba::hex(0xe5bd78),
            ),
            0 => ("Suchen und ansehen", Rgba::hex(0xe7d6af)),
            1 => ("Prime Video öffnen", Rgba::hex(0xe7d6af)),
            2 => (
                "ARD am Pi nicht verfügbar",
                Rgba::hex(0xe5bd78),
            ),
            3 => ("Native HLS-Wiedergabe", Rgba::hex(0x9cdbbd)),
            4 => ("Native HLS-Wiedergabe", Rgba::hex(0xe7d6af)),
            _ => ("App auswählen", Rgba::hex(0xe7d6af)),
        }
    }
}

impl Screen for Apps {
    fn name(&self) -> &'static str {
        "apps"
    }

    fn key(&mut self, key: Key, cx: &mut Ctx) -> Nav {
        if self.launch_started.is_some() || self.entry_progress().is_some() {
            return Nav::None;
        }
        match key {
            Key::Left => self.focus = (self.focus + APPS.len() - 1) % APPS.len(),
            Key::Right => self.focus = (self.focus + 1) % APPS.len(),
            Key::Up | Key::Back | Key::Home => return Nav::Pop,
            Key::Ok if self.focus == 2 => {
                cx.toast("ARD-Webseite löst auf diesem Pi einen WebKit-Fehler aus")
            }
            Key::Ok if self.focus <= 1 && !self.browser_installed => {
                cx.toast("Web-App-Unterstützung ist nicht installiert")
            }
            Key::Ok => self.launch_started = Some(Instant::now()),
            _ => {}
        }
        Nav::None
    }

    fn animating(&self, _: &Ctx) -> bool {
        self.wheel.moving() || self.launch_started.is_some() || self.entry_progress().is_some()
    }

    fn draw(&mut self, c: &mut Canvas, cx: &mut Ctx) {
        let entry = self.entry_progress();
        let launch = self
            .launch_started
            .map(|started| started.elapsed().as_secs_f32() / LAUNCH_ANIMATION_SECONDS);
        if launch.is_some() || (entry.is_none() && self.wheel_base.is_none()) {
            c.ambient(crate::gfx::Ambient::LiquidGold);
        }
        if entry.is_none() && launch.is_some() {
            super::status(c, cx);
        }
        if self.loaded_assets.is_none() {
            self.loaded_assets = self.assets.get();
            if self.loaded_assets.is_some() {
                self.wheel_base = None;
            }
        }
        let (detail, detail_color) = self.detail();
        if entry.is_some() || launch.is_some() {
            if entry.is_some() {
                self.wheel_base = None;
                self.previous_items = None;
            }
            if launch.is_some() {
                super::status(c, cx);
            }
            apps_wheel::draw_full(
                c,
                &mut self.wheel,
                self.focus,
                APPS[self.focus].0,
                detail,
                detail_color,
                launch,
                entry,
                self.loaded_assets.as_ref(),
            );
            return;
        }

        let refresh_base = self.wheel_base.is_none();
        if refresh_base {
            super::status(c, cx);
            apps_wheel::draw_static(
                c,
                &mut self.wheel,
                self.focus,
                APPS[self.focus].0,
                detail,
                detail_color,
                self.loaded_assets.as_ref(),
            );
            c.finish();
            if let (Some(base), Some(positions)) = (self.wheel_base.as_ref(), self.previous_items) {
                restore_items(c, base, &positions);
            }
            self.wheel_base = Some(c.snapshot());
        } else if let (Some(base), Some(positions)) =
            (self.wheel_base.as_ref(), self.previous_items)
        {
            restore_items(c, base, &positions);
        }
        apps_wheel::draw_dynamic(
            c,
            &mut self.wheel,
            self.focus,
            APPS[self.focus].0,
            detail,
            detail_color,
            None,
            self.loaded_assets.as_ref(),
        );
        c.finish_pending_region(420.0, 150.0, 440.0, 440.0);
        self.previous_items = Some(self.wheel.item_positions());
    }

    fn poll(&mut self, cx: &mut Ctx) -> Nav {
        if self.entry_started.elapsed() >= ENTRY_DURATION {
            self.entry_backdrop = None;
        }
        if self
            .launch_started
            .is_some_and(|started| started.elapsed().as_secs_f32() >= LAUNCH_ANIMATION_SECONDS)
        {
            self.launch_started = None;
            return self.open(cx);
        }
        Nav::None
    }

    fn overlay(&self) -> bool {
        self.entry_started.elapsed() < ENTRY_DURATION
    }

    fn needs_backdrop(&self) -> bool {
        self.entry_started.elapsed() < ENTRY_DURATION
    }

    fn set_transition_backdrop(&mut self, image: Image) {
        self.entry_backdrop = Some(image);
    }

    fn transition_backdrop(&self) -> Option<(Image, f32)> {
        self.entry_backdrop
            .as_ref()
            .zip(self.entry_progress())
            .map(|(image, progress)| (image.clone(), progress))
    }
}

fn restore_items(canvas: &mut Canvas, base: &Image, positions: &[(f32, f32); 5]) {
    canvas.restore_rect(base, 540.0, 310.0, 200.0, 120.0);
    for &(x, y) in positions {
        canvas.restore_rect(base, x - 56.0, y - 56.0, 112.0, 112.0);
    }
}
