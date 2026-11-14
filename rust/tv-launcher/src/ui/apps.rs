//! Remote-first catalog of installed and web-based TV apps.
use super::*;
use crate::app::{Ctx, Nav, Screen};
use crate::gfx::{color, Align, Canvas, Rgba, Weight};

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
    wheel: bool,
    down_button: bool,
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
            wheel: false,
            down_button: false,
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
            Key::Left => (self.focus + APPS.len() - 1) % APPS.len(),
            Key::Right => (self.focus + 1) % APPS.len(),
            Key::Up if self.focus >= 3 => self.focus - 3,
            Key::Down if self.focus < 3 => self.focus + 3,
            _ => self.focus,
        };
    }
}

impl Screen for Apps {
    fn name(&self) -> &'static str {
        "apps"
    }

    fn key(&mut self, key: Key, cx: &mut Ctx) -> Nav {
        if self.wheel {
            match key {
                Key::Left => self.focus = (self.focus + APPS.len() - 1) % APPS.len(),
                Key::Right => self.focus = (self.focus + 1) % APPS.len(),
                Key::Up | Key::Back => self.wheel = false,
                Key::Ok => return self.open(cx),
                Key::Home => return Nav::Pop,
                _ => {}
            }
            return Nav::None;
        }
        if self.down_button {
            match key {
                Key::Ok => self.wheel = true,
                Key::Up => self.down_button = false,
                Key::Left | Key::Right => self.down_button = false,
                Key::Back | Key::Home => return Nav::Pop,
                _ => {}
            }
            return Nav::None;
        }
        match key {
            Key::Up | Key::Left | Key::Right => self.move_focus(key),
            Key::Down if self.focus >= 3 => self.down_button = true,
            Key::Down => self.move_focus(key),
            Key::Ok if self.focus == 2 => {
                cx.toast("ARD-Webseite löst auf diesem Pi einen WebKit-Fehler aus")
            }
            Key::Ok if self.focus <= 1 && !super::browser::installed() => {
                cx.toast("Web-App-Unterstützung ist nicht installiert")
            }
            Key::Ok => return self.open(cx),
            Key::Back | Key::Home => return Nav::Pop,
            _ => {}
        }
        Nav::None
    }

    fn animating(&self, _: &Ctx) -> bool {
        false
    }

    fn draw(&mut self, c: &mut Canvas, cx: &mut Ctx) {
        c.ambient(crate::gfx::Ambient::Color(Rgba::hex(0x121722)));
        super::status(c, cx);
        if self.wheel {
            draw_wheel(c, self.focus);
        } else {
            draw_app_grid(c, self.focus, self.down_button);
        }
    }
}

fn draw_app_grid(c: &mut Canvas, focus: usize, down_button: bool) {
    c.text(
        "Apps",
        PAD,
        42.0,
        38,
        Weight::Bold,
        color::TEXT,
        Align::Left,
    );
    c.text(
        "Wähle eine App oder öffne den App-Kreis",
        PAD,
        86.0,
        17,
        Weight::Regular,
        color::MUTED,
        Align::Left,
    );
    let xs = [240.0, 640.0, 1040.0];
    let ys = [230.0, 425.0];
    for i in 0..APPS.len() {
        let (title, _, _) = APPS[i];
        let (x, y) = (xs[i % 3], ys[i / 3]);
        let accent = app_accent(i);
        if focus == i {
            c.circle(x, y, 62.0, Rgba(accent.0, accent.1, accent.2, 40));
            c.circle(x, y, 54.0, Rgba(18, 22, 31, 255));
            c.circle(x, y, 50.0, Rgba(accent.0, accent.1, accent.2, 70));
        }
        draw_badge(c, i, x - 32.0, y - 27.0);
        c.text_fit(
            title,
            x,
            y + 67.0,
            19,
            Weight::Bold,
            if i == 2 { color::MUTED } else { color::TEXT },
            240.0,
        );
    }
    let (detail, detail_color) = app_details(focus);
    c.text_fit(
        detail,
        640.0,
        555.0,
        15,
        Weight::Regular,
        detail_color,
        820.0,
    );
    c.rect(
        570.0,
        596.0,
        140.0,
        58.0,
        29.0,
        if down_button {
            Rgba::hex(0x526fb1)
        } else {
            Rgba::hex(0x252a36)
        },
    );
    c.stroke_rect(
        570.0,
        596.0,
        140.0,
        58.0,
        29.0,
        1.5,
        Rgba(178, 202, 255, 115),
    );
    c.text(
        "APP-KREIS",
        640.0,
        611.0,
        11,
        Weight::Bold,
        color::TEXT,
        Align::Center,
    );
    c.polygon(
        &[(629.0, 630.0), (651.0, 630.0), (640.0, 642.0)],
        color::WHITE,
    );
    c.text(
        "Pfeile wählen · OK öffnen · ↓ App-Kreis",
        PAD,
        681.0,
        14,
        Weight::Regular,
        color::MUTED,
        Align::Left,
    );
}

fn draw_wheel(c: &mut Canvas, focus: usize) {
    c.text(
        "App-Kreis",
        PAD,
        42.0,
        38,
        Weight::Bold,
        color::TEXT,
        Align::Left,
    );
    c.text(
        "Links und rechts drehen den Kreis",
        PAD,
        86.0,
        17,
        Weight::Regular,
        color::MUTED,
        Align::Left,
    );
    let (cx, cy, radius) = (640.0f32, 372.0f32, 218.0f32);
    c.circle(cx, cy, radius + 5.0, Rgba(99, 137, 215, 22));
    c.circle(cx, cy, radius, Rgba(76, 103, 159, 90));
    c.circle(cx, cy, radius - 3.0, Rgba(15, 19, 28, 255));
    c.circle(cx, cy, 112.0, Rgba(37, 47, 69, 230));
    c.circle(cx, cy, 110.0, Rgba(255, 255, 255, 18));
    c.text_fit(
        APPS[focus].0,
        cx,
        cy - 15.0,
        24,
        Weight::Bold,
        color::TEXT,
        220.0,
    );
    c.text(
        "OK · ÖFFNEN",
        cx,
        cy + 22.0,
        12,
        Weight::Bold,
        Rgba::hex(0xaecaff),
        Align::Center,
    );
    for i in 0..APPS.len() {
        let offset = (i + APPS.len() - focus) % APPS.len();
        let angle = -std::f32::consts::FRAC_PI_2
            + offset as f32 * std::f32::consts::TAU / APPS.len() as f32;
        let (x, y) = (cx + angle.cos() * radius, cy + angle.sin() * radius);
        let accent = app_accent(i);
        let selected = i == focus;
        c.circle(
            x,
            y,
            if selected { 48.0 } else { 37.0 },
            Rgba(
                accent.0,
                accent.1,
                accent.2,
                if selected { 125 } else { 48 },
            ),
        );
        c.circle(
            x,
            y,
            if selected { 42.0 } else { 33.0 },
            Rgba(18, 22, 31, 255),
        );
        draw_badge(c, i, x - 32.0, y - 27.0);
        if !selected {
            c.text_fit(
                APPS[i].0,
                x,
                y + 48.0,
                12,
                Weight::Bold,
                color::MUTED,
                132.0,
            );
        }
    }
    c.text(
        "← / → WEITERBLÄTTERN · ↑ ZURÜCK · OK STARTEN",
        640.0,
        674.0,
        14,
        Weight::Bold,
        color::MUTED,
        Align::Center,
    );
}

fn app_details(index: usize) -> (&'static str, Rgba) {
    match index {
        0 if !super::browser::installed() => (
            "YouTube TV · Browser-Unterstützung nicht installiert",
            Rgba::hex(0xe5a66d),
        ),
        1 if !super::browser::installed() => (
            "Prime Video · Browser-Unterstützung nicht installiert",
            Rgba::hex(0xe5a66d),
        ),
        2 => (
            "ARD Mediathek · derzeit wegen eines WebKit-Fehlers deaktiviert",
            Rgba::hex(0xe5a66d),
        ),
        3 => ("ZDF Live · native HLS-Wiedergabe", Rgba::hex(0x8dd5b0)),
        4 => ("ARTE Live · native HLS-Wiedergabe", Rgba::hex(0x8dd5b0)),
        5 => (
            "Einstellungen · WLAN, Bild, Ton und Bedienung",
            Rgba::hex(0x91baff),
        ),
        _ => ("App auswählen", Rgba::hex(0x91baff)),
    }
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
