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
        self.focus = match key {
            Key::Left => {
                if self.focus % 2 == 1 {
                    self.focus - 1
                } else {
                    self.focus
                }
            }
            Key::Right => {
                if self.focus % 2 == 0 && self.focus + 1 < APPS.len() {
                    self.focus + 1
                } else {
                    self.focus
                }
            }
            Key::Up => self.focus.saturating_sub(2),
            Key::Down => (self.focus + 2).min(APPS.len() - 1),
            _ => self.focus,
        };
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
        c.ambient(crate::gfx::Ambient::Color(Rgba::hex(0x354a78)));
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
                    c.restore_rect(base, PAD, 558.0, 1080.0, 28.0);
                }
            }
            draw_app_note(c, self.focus);
            c.finish_pending_region(PAD, 558.0, 1080.0, 28.0);
            self.last_focus_note = Some(self.focus);
        }
        let focus_ms = focus_started.elapsed().as_secs_f64() * 1000.0;

        // The same time-based glide as Home; only its edge spans are rasterized
        // between key presses, keeping the animation cheap on Pi 3.
        let ring_started = std::time::Instant::now();
        let (x, y) = app_card_origin(self.focus);
        let bounds = self.frame.move_to(x, y, 532.0, 126.0, true);
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

fn app_card_origin(index: usize) -> (f32, f32) {
    (
        PAD + (index % 2) as f32 * 572.0,
        132.0 + (index / 2) as f32 * 142.0,
    )
}

fn draw_app_card(c: &mut Canvas, index: usize) {
    let (title, detail, _) = APPS[index];
    let (x, y) = app_card_origin(index);
    let fill = if index == 2 {
        Rgba(28, 29, 34, 180)
    } else {
        Rgba(34, 36, 43, 220)
    };
    c.rect(x, y, 532.0, 126.0, 18.0, fill);
    draw_badge(c, index, x + 20.0, y + 31.0);
    let ink = if index == 2 {
        color::MUTED
    } else {
        color::TEXT
    };
    c.text(
        title,
        x + 108.0,
        y + 27.0,
        25,
        Weight::Bold,
        ink,
        Align::Left,
    );
    c.text_fit(
        detail,
        x + 108.0,
        y + 73.0,
        15,
        Weight::Regular,
        color::MUTED,
        398.0,
    );
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
    c.text_fit(note, PAD, 558.0, 15, Weight::Regular, color::MUTED, 1080.0);
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
