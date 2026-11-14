//! Remote-first catalog of installed and web-based TV apps.
use super::*;
use crate::app::{Ctx, Nav, Screen};
use crate::gfx::{color, Align, Canvas, Image, LayerCache, Rgba, Weight};

const APPS: [(&str, &str, &str); 6] = [
    ("YouTube TV", "YouTube-TV-Oberfläche", "youtube"),
    (
        "Prime Video",
        "Webseite; Videowiedergabe unbestätigt",
        "prime",
    ),
    (
        "ARD Mediathek",
        "Nicht verfügbar · WebKit-Fehler am Pi",
        "ard",
    ),
    ("ZDF Live", "Native HLS · Bild/Ton-Test", "zdf"),
    ("ARTE Live", "Native HLS-Wiedergabe · 720p am Pi", "arte"),
    (
        "Einstellungen",
        "System, Bild und Fernbedienung",
        "settings",
    ),
];
const APP_ROW_H: f32 = 188.0;

pub struct Apps {
    focus: usize,
    frame: Focus,
    base_cache: LayerCache,
    base_image: Option<Image>,
    last_focus_rect: Option<(f32, f32, f32, f32)>,
    last_focus_note: Option<usize>,
}

impl Apps {
    pub fn new(last_app_id: &str) -> Self {
        let focus = APPS
            .iter()
            .position(|app| app.2 == last_app_id)
            .filter(|&i| i != 2)
            .unwrap_or(0);
        Self {
            focus,
            frame: Focus::default(),
            base_cache: LayerCache::default(),
            base_image: None,
            last_focus_rect: None,
            last_focus_note: None,
        }
    }

    fn open(&self, cx: &mut Ctx) -> Nav {
        if self.focus == 5 {
            return Nav::Push(Box::new(super::settings::Settings::new(cx)));
        }
        if self.focus == 4 {
            return Nav::Push(Box::new(super::livetv::LiveTv::new_arte(cx)));
        }
        if self.focus == 3 {
            return Nav::Push(Box::new(super::livetv::LiveTv::new_zdf(cx)));
        }
        if !super::browser::installed() {
            return Nav::None;
        }
        let url = match self.focus {
            0 => super::browser::YOUTUBE_TV,
            1 => super::browser::PRIME_VIDEO,
            2 => return Nav::None,
            3 => super::browser::ZDF_STREAMING,
            _ => super::browser::ARTE_LIVE,
        };
        Nav::Push(Box::new(super::browser::Browser::new(url)))
    }

    fn move_focus(&mut self, key: Key) {
        let (x, y, w, _) = app_card_rect(self.focus);
        let center = x + w * 0.5;
        let row = usize::from(y > 140.0);
        let candidate = (0..APPS.len())
            .filter(|&i| i != self.focus)
            .filter_map(|i| {
                let (nx, ny, nw, _) = app_card_rect(i);
                let nrow = usize::from(ny > 140.0);
                let nc = nx + nw * 0.5;
                let valid = match key {
                    Key::Left => nrow == row && nc < center,
                    Key::Right => nrow == row && nc > center,
                    Key::Up => row == 1 && nrow == 0,
                    Key::Down => row == 0 && nrow == 1,
                    _ => false,
                };
                valid.then_some((i, (nc - center).abs()))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((next, _)) = candidate {
            self.focus = next;
        }
    }
}

impl Screen for Apps {
    fn name(&self) -> &'static str {
        "apps"
    }

    fn key(&mut self, key: Key, cx: &mut Ctx) -> Nav {
        match key {
            Key::Up | Key::Down | Key::Left | Key::Right => self.move_focus(key),
            Key::Ok => {
                if self.focus == 5 || self.focus == 4 || self.focus == 3 {
                    return self.open(cx);
                } else if self.focus == 2 {
                    cx.toast("ARD-Webseite löst auf diesem Pi einen WebKit-Fehler aus");
                } else if super::browser::installed() {
                    return self.open(cx);
                } else {
                    cx.toast("Web-App-Unterstützung ist nicht installiert");
                }
            }
            Key::Back | Key::Home => return Nav::Pop,
            _ => {}
        }
        Nav::None
    }

    fn animating(&self, _: &Ctx) -> bool {
        self.frame.moving()
    }

    fn draw(&mut self, c: &mut Canvas, cx: &mut Ctx) {
        let trace = std::env::var_os("SHOT_APPS_TRACE").is_some();
        c.ambient(crate::gfx::Ambient::Color(Rgba::hex(0x151b27)));
        c.text(
            "Apps",
            PAD,
            38.0,
            38,
            Weight::Bold,
            color::TEXT,
            Align::Left,
        );
        c.text(
            "Mit der Fernbedienung auswählen",
            PAD,
            84.0,
            17,
            Weight::Regular,
            color::MUTED,
            Align::Left,
        );
        c.text(
            "STREAMING & LIVE-TV",
            PAD,
            116.0,
            12,
            Weight::Bold,
            Rgba::hex(0x9eabc8),
            Align::Left,
        );
        super::status(c, cx);

        for i in 0..APPS.len() {
            draw_app_card(c, i);
        }
        c.text(
            "Pfeile: auswählen   ·   OK: öffnen   ·   Zurück: Startseite",
            PAD,
            670.0,
            16,
            Weight::Regular,
            color::MUTED,
            Align::Left,
        );

        // Keep the six cards and page chrome as one retained layer. Navigation
        // then restores only the old selection and the small moving focus ring.
        let base_started = std::time::Instant::now();
        let base_changed = c.finish_cached(&mut self.base_cache);
        let base_ms = base_started.elapsed().as_secs_f64() * 1000.0;
        let full_restore = !base_changed && c.full_base_restore_required();
        if base_changed {
            self.base_image = Some(c.snapshot());
            self.last_focus_rect = None;
            self.last_focus_note = None;
        } else if let Some(base) = self.base_image.as_ref() {
            if full_restore {
                c.restore_image(base);
                self.last_focus_rect = None;
            } else if let Some((x, y, w, h)) = self.last_focus_rect {
                restore_app_focus(c, base, x, y, w, h);
            }
        }

        let focus_changed =
            base_changed || full_restore || self.last_focus_note != Some(self.focus);
        let focus_started = std::time::Instant::now();
        if focus_changed {
            if !base_changed {
                if let Some(base) = self.base_image.as_ref() {
                    c.restore_rect(base, PAD, 584.0, 1080.0, 28.0);
                }
            }
            draw_app_note(c, self.focus);
            c.finish_pending_region(PAD, 584.0, 1080.0, 28.0);
            self.last_focus_note = Some(self.focus);
        }
        let focus_ms = focus_started.elapsed().as_secs_f64() * 1000.0;

        // The same time-based glide as Home; only its edge spans are rasterized
        // between key presses, keeping the animation cheap on Pi 3.
        let ring_started = std::time::Instant::now();
        let (x, y, w, h) = app_card_rect(self.focus);
        let bounds = self.frame.move_to(x, y, w, h, true);
        super::focus_ring_fast(c, bounds.0, bounds.1, bounds.2, bounds.3);
        c.finish_pending_region(
            bounds.0 - 10.0,
            bounds.1 - 10.0,
            bounds.2 + 20.0,
            bounds.3 + 20.0,
        );
        let ring_ms = ring_started.elapsed().as_secs_f64() * 1000.0;
        if trace && focus_changed {
            eprintln!("Apps-Phase: Basis {base_ms:.2} ms, Auswahl {focus_ms:.2} ms, Rahmen {ring_ms:.2} ms");
        }
        self.last_focus_rect = Some(bounds);
    }
}

/// Erase the previous focus ring. Its rounded corners reach a few pixels
/// into the card's own box, so the strips overlap the card edge: restoring
/// only the margin outside left the corner arcs behind when the focus moved
/// on, and made them brighter with every frame it stayed.
fn restore_app_focus(c: &mut Canvas, base: &Image, x: f32, y: f32, w: f32, h: f32) {
    const OUT: f32 = 10.0;
    const IN: f32 = 8.0;
    let edge = OUT + IN;
    c.restore_rect(base, x - OUT, y - OUT, w + OUT * 2.0, edge);
    c.restore_rect(base, x - OUT, y + h - IN, w + OUT * 2.0, edge);
    c.restore_rect(base, x - OUT, y + IN, edge, (h - IN * 2.0).max(0.0));
    c.restore_rect(base, x + w - IN, y + IN, edge, (h - IN * 2.0).max(0.0));
}

fn app_card_rect(index: usize) -> (f32, f32, f32, f32) {
    let y = if index < 3 { 140.0 } else { 352.0 };
    let (x, w) = match index {
        0 => (64.0, 564.0),
        1 => (652.0, 270.0),
        2 => (946.0, 270.0),
        3 => (64.0, 270.0),
        4 => (358.0, 564.0),
        _ => (946.0, 270.0),
    };
    (x, y, w, APP_ROW_H)
}

fn draw_app_card(c: &mut Canvas, index: usize) {
    let (title, detail, _) = APPS[index];
    let (x, y, w, h) = app_card_rect(index);
    let wide = w > 400.0;
    let fill = if index == 2 {
        Rgba(25, 27, 33, 185)
    } else {
        Rgba(31, 34, 43, 238)
    };
    c.rect(x, y, w, h, 22.0, fill);
    c.stroke_rect(x, y, w, h, 22.0, 1.0, Rgba(255, 255, 255, 24));
    let accent = app_accent(index);
    c.rect(
        x + 1.0,
        y + 58.0,
        4.0,
        72.0,
        2.0,
        Rgba(accent.0, accent.1, accent.2, 220),
    );
    draw_badge(
        c,
        index,
        x + if wide { 24.0 } else { 18.0 },
        y + if wide { 22.0 } else { 18.0 },
    );
    let category = match index {
        0 | 1 | 2 => "STREAMING",
        3 | 4 => "LIVE-TV",
        _ => "SYSTEM",
    };
    c.text(
        category,
        x + if wide { 102.0 } else { 98.0 },
        y + 27.0,
        11,
        Weight::Bold,
        accent,
        Align::Left,
    );
    let ink = if index == 2 {
        color::MUTED
    } else {
        color::TEXT
    };
    c.text(
        title,
        x + if wide { 102.0 } else { 20.0 },
        y + if wide { 76.0 } else { 100.0 },
        22,
        Weight::Bold,
        ink,
        Align::Left,
    );
    c.text_fit(
        detail,
        x + if wide { 102.0 } else { 20.0 },
        y + if wide { 110.0 } else { 129.0 },
        14,
        Weight::Regular,
        color::MUTED,
        w - if wide { 126.0 } else { 40.0 },
    );
    let status = match index {
        0 | 1 => {
            if super::browser::installed() {
                "BROWSER BEREIT"
            } else {
                "BROWSER NICHT INSTALLIERT"
            }
        }
        2 => "DERZEIT DEAKTIVIERT",
        3 | 4 => "NATIVE WIEDERGABE",
        _ => "EINSTELLUNGEN ÖFFNEN",
    };
    let status_color = match index {
        2 => Rgba::hex(0xe5a66d),
        3 | 4 => Rgba::hex(0x8dd5b0),
        _ => Rgba::hex(0x91baff),
    };
    let status_x = if wide { x + 106.0 } else { x + 24.0 };
    c.circle(status_x, y + 165.0, 3.0, status_color);
    c.text(
        status,
        status_x + 11.0,
        y + 157.0,
        11,
        Weight::Bold,
        status_color,
        Align::Left,
    );
}

fn app_accent(index: usize) -> Rgba {
    match index {
        0 => Rgba::hex(0xf0444b),
        1 => Rgba::hex(0x54bfff),
        2 => Rgba::hex(0x9bb4dc),
        3 => Rgba::hex(0x9a72ff),
        4 => Rgba::hex(0xe5e8ef),
        _ => Rgba::hex(0x8eb5ff),
    }
}

fn draw_app_note(c: &mut Canvas, focus: usize) {
    let note = match focus {
        1 => "Prime Video: DRM-Wiedergabe auf diesem Pi nicht bestätigt.",
        2 => "ARD Mediathek ist wegen eines WebKit-Fehlers deaktiviert.",
        3 => "ZDF Live läuft nativ via HLS mit Bild und Ton.",
        4 => "ARTE Live läuft nativ via HLS in 720p.",
        5 => "WLAN, Bluetooth, Bild, Ton und Fernbedienung konfigurieren.",
        _ => "YouTube TV öffnet in der offiziellen TV-Oberfläche.",
    };
    c.text_fit(note, PAD, 590.0, 15, Weight::Regular, color::MUTED, 1080.0);
}

fn draw_badge(c: &mut Canvas, app: usize, x: f32, y: f32) {
    match app {
        0 => {
            c.rect(x, y + 4.0, 64.0, 46.0, 12.0, color::RED);
            c.polygon(
                &[
                    (x + 26.0, y + 14.0),
                    (x + 26.0, y + 40.0),
                    (x + 46.0, y + 27.0),
                ],
                color::WHITE,
            );
        }
        1 => {
            c.circle(x + 32.0, y + 27.0, 29.0, Rgba::hex(0x202c54));
            c.text(
                "prime",
                x + 32.0,
                y + 14.0,
                15,
                Weight::Bold,
                Rgba::hex(0x74c9ff),
                Align::Center,
            );
            c.line(
                &[
                    (x + 20.0, y + 39.0),
                    (x + 31.0, y + 44.0),
                    (x + 45.0, y + 37.0),
                ],
                2.0,
                Rgba::hex(0x74c9ff),
            );
        }
        2 => {
            c.circle(x + 32.0, y + 27.0, 29.0, Rgba::hex(0x25272d));
            c.circle(x + 32.0, y + 27.0, 20.0, Rgba::hex(0x25272d));
            c.stroke_rect(x + 8.0, y + 4.0, 48.0, 46.0, 20.0, 2.0, Rgba::hex(0x97b7df));
            c.text(
                "ARD",
                x + 32.0,
                y + 16.0,
                13,
                Weight::Bold,
                color::WHITE,
                Align::Center,
            );
        }
        4 => {
            c.circle(x + 32.0, y + 27.0, 29.0, Rgba::hex(0x151515));
            c.text(
                "ARTE",
                x + 32.0,
                y + 17.0,
                17,
                Weight::Bold,
                color::WHITE,
                Align::Center,
            );
        }
        5 => super::icon_gear(c, x + 32.0, y + 27.0, 38.0, Rgba::hex(0xe8eaf0)),
        _ => {
            c.circle(x + 32.0, y + 27.0, 29.0, Rgba::hex(0x6745ba));
            c.text(
                "ZDF",
                x + 32.0,
                y + 13.0,
                20,
                Weight::Bold,
                color::WHITE,
                Align::Center,
            );
        }
    }
}
