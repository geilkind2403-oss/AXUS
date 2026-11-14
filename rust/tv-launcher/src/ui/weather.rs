//! Weather: now, the next 24 hours, the week.
use super::*;
use crate::app::{Ctx, Nav, Screen};
use crate::gfx::{color, Align, Canvas, Rgba, Weight};

pub struct WeatherScreen;

impl WeatherScreen {
    pub fn new() -> WeatherScreen {
        WeatherScreen
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
    fn draw(&mut self, c: &mut Canvas, cx: &mut Ctx) {
        let w = match cx.weather.get() {
            Some(Ok(w)) => w,
            other => {
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
                return;
            }
        };
        let (sky, desc) = crate::weather::describe(w.code);
        let (top, bottom, accent) = match (sky, w.day) {
            (_, false) => (
                Rgba::hex(0x17213d),
                Rgba::hex(0x090d1b),
                Rgba::hex(0x9cbcff),
            ),
            (crate::weather::Sky::Clear, _) => (
                Rgba::hex(0x276aa4),
                Rgba::hex(0x101d38),
                Rgba::hex(0xffcf64),
            ),
            (crate::weather::Sky::PartlyCloudy, _) => (
                Rgba::hex(0x315e8b),
                Rgba::hex(0x111d35),
                Rgba::hex(0x9ed1ff),
            ),
            (crate::weather::Sky::Thunder, _) => (
                Rgba::hex(0x373248),
                Rgba::hex(0x10101a),
                Rgba::hex(0xc6b5ff),
            ),
            _ => (
                Rgba::hex(0x35475d),
                Rgba::hex(0x101722),
                Rgba::hex(0x9bc7eb),
            ),
        };
        c.vgradient(0.0, 0.0, 1280.0, 720.0, 0.0, top, bottom);
        status(c, cx);
        c.text(
            &cx.cfg.city,
            PAD,
            42.0,
            34,
            Weight::Bold,
            color::WHITE,
            Align::Left,
        );
        c.text(
            "WETTER JETZT",
            PAD,
            84.0,
            13,
            Weight::Bold,
            accent,
            Align::Left,
        );
        c.text(
            "Vorhersage für die nächsten Tage",
            PAD,
            105.0,
            15,
            Weight::Regular,
            Rgba(228, 235, 248, 190),
            Align::Left,
        );

        c.vgradient(
            PAD,
            142.0,
            560.0,
            190.0,
            24.0,
            Rgba(255, 255, 255, 38),
            Rgba(255, 255, 255, 17),
        );
        c.stroke_rect(PAD, 142.0, 560.0, 190.0, 24.0, 1.0, Rgba(255, 255, 255, 45));
        c.circle(526.0, 232.0, 86.0, Rgba(255, 255, 255, 13));
        icon_weather(c, sky, w.day, 154.0, 232.0, 112.0);
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
            let x = 650.0 + i as f32 * 190.0;
            c.rect(x, 142.0, 174.0, 190.0, 21.0, Rgba(255, 255, 255, 19));
            c.stroke_rect(x, 142.0, 174.0, 190.0, 21.0, 1.0, Rgba(255, 255, 255, 32));
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

        c.rect(PAD, 356.0, 1152.0, 137.0, 22.0, Rgba(18, 25, 42, 167));
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
        let dw = 1152.0 / days.len().max(1) as f32;
        for (i, d) in days.iter().enumerate() {
            let x = PAD + dw * i as f32;
            let y = 539.0;
            c.rect(
                x + 4.0,
                y,
                dw - 8.0,
                134.0,
                17.0,
                Rgba(255, 255, 255, if i == 0 { 31 } else { 17 }),
            );
            c.stroke_rect(
                x + 4.0,
                y,
                dw - 8.0,
                134.0,
                17.0,
                1.0,
                Rgba(255, 255, 255, 25),
            );
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
            let bw = dw - 48.0;
            let bx = x + 24.0;
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
}
