//! Remote-first catalog of installed and web-based TV apps.
use super::*;
use crate::app::{Ctx, Nav, Screen};
use crate::gfx::{color, Align, Canvas, Image, Rgba, Weight};
use std::time::Instant;

const LAUNCH_ANIMATION_SECONDS: f32 = 0.72;

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
    grid_focus: Focus,
    wheel_rotation: Anim,
    icons: [Option<Image>; 5],
    launch_started: Option<Instant>,
}

impl Apps {
    pub fn new(last_app_id: &str) -> Self {
        Self::with_mode(last_app_id, false)
    }

    pub fn wheel(last_app_id: &str) -> Self {
        Self::with_mode(last_app_id, true)
    }

    fn with_mode(last_app_id: &str, wheel: bool) -> Self {
        let focus = APPS
            .iter()
            .position(|app| app.2 == last_app_id)
            .filter(|&i| i != 2)
            .unwrap_or(0);
        let mut wheel_rotation = Anim::default();
        wheel_rotation.jump(focus as f32 * std::f32::consts::TAU / APPS.len() as f32);
        let icon_bytes: [Option<&[u8]>; 5] = [
            Some(include_bytes!("../../assets/apps/youtube.png")),
            Some(include_bytes!("../../assets/apps/prime.png")),
            Some(include_bytes!("../../assets/apps/ard.png")),
            Some(include_bytes!("../../assets/apps/zdf.png")),
            None,
        ];
        let icons =
            icon_bytes.map(|bytes| bytes.and_then(|b| crate::gfx::decode_image(b, 128, 128)));
        Self {
            focus,
            wheel,
            grid_focus: Focus::default(),
            wheel_rotation,
            icons,
            launch_started: None,
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
            if self.launch_started.is_some() {
                return Nav::None;
            }
            match key {
                Key::Left => {
                    self.focus = (self.focus + APPS.len() - 1) % APPS.len();
                    self.wheel_rotation.set(
                        self.wheel_rotation.target - std::f32::consts::TAU / APPS.len() as f32,
                    );
                }
                Key::Right => {
                    self.focus = (self.focus + 1) % APPS.len();
                    self.wheel_rotation.set(
                        self.wheel_rotation.target + std::f32::consts::TAU / APPS.len() as f32,
                    );
                }
                Key::Up | Key::Back => return Nav::Pop,
                Key::Ok if self.focus == 2 => {
                    cx.toast("ARD-Webseite löst auf diesem Pi einen WebKit-Fehler aus")
                }
                Key::Ok if self.focus <= 1 && !super::browser::installed() => {
                    cx.toast("Web-App-Unterstützung ist nicht installiert")
                }
                Key::Ok => self.launch_started = Some(Instant::now()),
                Key::Home => return Nav::Pop,
                _ => {}
            }
            return Nav::None;
        }
        match key {
            Key::Up | Key::Left | Key::Right => self.move_focus(key),
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
        if self.wheel {
            self.wheel_rotation.moving() || self.launch_started.is_some()
        } else {
            self.grid_focus.moving()
        }
    }

    fn draw(&mut self, c: &mut Canvas, cx: &mut Ctx) {
        c.ambient(crate::gfx::Ambient::Color(Rgba::hex(0x121722)));
        super::status(c, cx);
        if self.wheel {
            let rotation = self.wheel_rotation.step();
            let launch = self
                .launch_started
                .map(|started| (self.focus, started.elapsed().as_secs_f32() / LAUNCH_ANIMATION_SECONDS));
            draw_wheel(c, self.focus, rotation, &self.icons, launch);
        } else {
            draw_app_grid(c, self.focus, &mut self.grid_focus, &self.icons);
        }
    }

    fn poll(&mut self, cx: &mut Ctx) -> Nav {
        if self
            .launch_started
            .is_some_and(|started| started.elapsed().as_secs_f32() >= LAUNCH_ANIMATION_SECONDS)
        {
            self.launch_started = None;
            return self.open(cx);
        }
        Nav::None
    }
}

fn draw_app_grid(c: &mut Canvas, focus: usize, grid_focus: &mut Focus, icons: &[Option<Image>; 5]) {
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
        "Wähle eine App zum Öffnen",
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
        draw_app_icon(c, icons, i, x, y, 72.0);
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
    let (fx, fy) = (xs[focus % 3], ys[focus / 3]);
    let (fx, fy, fw, fh) = grid_focus.move_to(fx - 62.0, fy - 62.0, 124.0, 124.0, true);
    let glow = app_accent(focus);
    c.circle(
        fx + fw * 0.5,
        fy + fh * 0.5,
        fw * 0.5,
        Rgba(glow.0, glow.1, glow.2, 26),
    );
    c.circle(
        fx + fw * 0.5,
        fy + fh * 0.5,
        fw * 0.5 - 7.0,
        Rgba(18, 22, 31, 255),
    );
    draw_app_icon(c, icons, focus, fx + fw * 0.5, fy + fh * 0.5, 72.0);
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
    c.text(
        "Pfeile wählen · OK öffnen · Zurück zum Hauptmenü",
        PAD,
        681.0,
        14,
        Weight::Regular,
        color::MUTED,
        Align::Left,
    );
}

fn draw_wheel(
    c: &mut Canvas,
    focus: usize,
    rotation: f32,
    icons: &[Option<Image>; 5],
    launch: Option<(usize, f32)>,
) {
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
    let (cx, cy, orbit) = (640.0f32, 372.0f32, 158.0f32);
    // A broad milk-glass disc replaces the hard metal-looking tracks. Its
    // translucent layers keep a soft hint of the launcher backdrop visible.
    c.circle(cx, cy, 240.0, Rgba(225, 237, 255, 20));
    c.circle(cx, cy, 234.0, Rgba(177, 197, 232, 48));
    c.circle(cx, cy, 228.0, Rgba(95, 111, 145, 105));
    c.circle(cx, cy, 222.0, Rgba(43, 51, 68, 205));
    c.circle(cx, cy, 216.0, Rgba(220, 232, 250, 24));
    c.circle(cx, cy, 210.0, Rgba(45, 53, 70, 115));

    // A soft inset hub and a single circular orbit hold all six app bubbles.
    c.circle(cx, cy, 91.0, Rgba(226, 237, 255, 30));
    c.circle(cx, cy, 86.0, Rgba(53, 62, 81, 205));
    if launch.is_none() {
        c.text(
            "AUSGEWÄHLT",
            cx,
            cy - 22.0,
            11,
            Weight::Bold,
            Rgba(166, 183, 215, 190),
            Align::Center,
        );
        c.text_fit(
            APPS[focus].0,
            cx,
            cy + 2.0,
            20,
            Weight::Bold,
            color::TEXT,
            220.0,
        );
        c.text("OK · ÖFFNEN", cx, cy + 31.0, 12, Weight::Bold, Rgba::hex(0xc6d7f5), Align::Center);
    }
    for i in 0..APPS.len() {
        let angle = -std::f32::consts::FRAC_PI_2
            + i as f32 * std::f32::consts::TAU / APPS.len() as f32
            - rotation;
        let (x, y) = (cx + angle.cos() * orbit, cy + angle.sin() * orbit);
        if launch.is_some_and(|(launch_focus, _)| launch_focus == i) {
            continue;
        }
        let accent = app_accent(i);
        let selected = i == focus;
        if selected {
            c.circle(x, y, 48.0, Rgba(accent.0, accent.1, accent.2, 22));
        }
        // Equal-size frosted bubbles are the bearing balls: every app is
        // inside the glass disc and every center lies on this exact orbit.
        c.circle(x, y, 42.0, Rgba(235, 243, 255, if selected { 85 } else { 48 }));
        c.circle(x, y, 37.0, Rgba(55, 65, 83, 205));
        c.circle(x - 8.0, y - 10.0, 14.0, Rgba(235, 243, 255, 22));
        draw_app_icon(c, icons, i, x, y, 48.0);
        let label_radius = 207.0;
        let lx = cx + angle.cos() * label_radius;
        let ly = cy + angle.sin() * label_radius;
        c.text_fit(
            APPS[i].0,
            lx,
            ly,
            12,
            Weight::Bold,
            if selected { color::TEXT } else { color::MUTED },
            144.0,
        );
    }
    if let Some((launch_focus, progress)) = launch {
        let progress = progress.clamp(0.0, 1.0);
        let pull_end = 0.64;
        let accent = app_accent(launch_focus);
        if progress < pull_end {
            let t = ease(progress / pull_end);
            let y = 128.0 + (cy - 128.0) * t;
            let size = 48.0 + 38.0 * t;
            c.circle(cx, y, size * 0.76, Rgba(accent.0, accent.1, accent.2, 28));
            c.circle(cx, y, size * 0.62, Rgba(232, 241, 255, 55));
            draw_app_icon(c, icons, launch_focus, cx, y, size);
        } else {
            // The selected app blooms out of the glass hub, then the normal
            // screen transition reveals the app itself.
            let t = ease((progress - pull_end) / (1.0 - pull_end));
            c.circle(
                cx,
                cy,
                92.0 + 680.0 * t,
                Rgba(accent.0, accent.1, accent.2, (24.0 + 44.0 * t) as u8),
            );
            draw_app_icon(c, icons, launch_focus, cx, cy, 86.0 + 112.0 * t);
        }
    }
    if launch.is_none() {
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

fn draw_app_icon(
    c: &mut Canvas,
    icons: &[Option<Image>; 5],
    index: usize,
    x: f32,
    y: f32,
    size: f32,
) {
    if let Some(Some(image)) = icons.get(index) {
        c.image_fit(
            image,
            x - size * 0.5,
            y - size * 0.5,
            size,
            size,
            size * 0.18,
            false,
            1.0,
        );
    } else {
        draw_badge(c, index, x, y, size);
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

fn draw_badge(c: &mut Canvas, app: usize, x: f32, y: f32, size: f32) {
    match app {
        4 => {
            c.circle(x, y, size * 0.46, Rgba::hex(0x151515));
            c.text(
                "ARTE",
                x,
                y - size * 0.12,
                (size * 0.27) as u32,
                Weight::Bold,
                color::WHITE,
                Align::Center,
            );
        }
        5 => super::icon_gear(c, x, y, size * 0.68, Rgba::hex(0xe8eaf0)),
        _ => c.circle(x, y, size * 0.42, Rgba::hex(0x262a35)),
    }
}
