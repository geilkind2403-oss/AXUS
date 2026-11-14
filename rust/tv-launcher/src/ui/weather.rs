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
                title(c, "Wetter");
                let msg = match other {
                    Some(Err(e)) => format!("Wetter nicht erreichbar: {e}"),
                    _ => "Wird geladen …".into(),
                };
                c.text(
                    &msg,
                    PAD,
                    130.0,
                    20,
                    Weight::Regular,
                    color::MUTED,
                    Align::Left,
                );
                return;
            }
        };
        let (sky, desc) = crate::weather::describe(w.code);
        let (top, bottom) = match (sky, w.day) {
            (_, false) => (Rgba::hex(0x0d1330), Rgba::hex(0x05070f)),
            (crate::weather::Sky::Clear, _) | (crate::weather::Sky::PartlyCloudy, _) => {
                (Rgba::hex(0x2b6cb0), Rgba::hex(0x10233f))
            }
            (crate::weather::Sky::Thunder, _) => (Rgba::hex(0x2d2a3e), Rgba::hex(0x0f0e16)),
            _ => (Rgba::hex(0x4a5568), Rgba::hex(0x1a202c)),
        };
        c.vgradient(0.0, 0.0, 1280.0, 720.0, 0.0, top, bottom);
        status(c, cx);
        c.text(
            &cx.cfg.city.clone(),
            PAD,
            40.0,
            30,
            Weight::Bold,
            color::WHITE,
            Align::Left,
        );
        icon_weather(c, sky, w.day, PAD + 90.0, 190.0, 170.0);
        c.text(
            &format!("{:.0}°", w.temp),
            PAD + 210.0,
            100.0,
            110,
            Weight::Bold,
            color::WHITE,
            Align::Left,
        );
        c.text(
            desc,
            PAD + 214.0,
            238.0,
            26,
            Weight::Semi,
            color::WHITE,
            Align::Left,
        );
        let today = w.days.first();
        let line = format!(
            "{}Gefühlt {:.0}° · Wind {:.0} km/h · Luftfeuchte {:.0} %",
            today
                .map(|d| format!("H {:.0}° · T {:.0}° · ", d.max, d.min))
                .unwrap_or_default(),
            w.feels,
            w.wind,
            w.humidity
        );
        c.text(
            &line,
            PAD + 214.0,
            276.0,
            17,
            Weight::Regular,
            color::WHITE.a(200),
            Align::Left,
        );
        // hours
        let y = 330.0;
        c.rect(PAD, y, 1152.0, 150.0, 20.0, Rgba(255, 255, 255, 22));
        let hours: Vec<_> = w.hours.iter().take(25).step_by(2).take(12).collect();
        let step = 1152.0 / hours.len().max(1) as f32;
        for (i, h) in hours.iter().enumerate() {
            let x = PAD + step * (i as f32 + 0.5);
            c.text(
                &if i == 0 {
                    "Jetzt".to_string()
                } else {
                    format!("{:02}", h.hour)
                },
                x,
                y + 16.0,
                15,
                Weight::Semi,
                color::WHITE.a(200),
                Align::Center,
            );
            icon_weather(
                c,
                crate::weather::describe(h.code).0,
                h.day,
                x,
                y + 70.0,
                40.0,
            );
            c.text(
                &format!("{:.0}°", h.temp),
                x,
                y + 98.0,
                19,
                Weight::Semi,
                color::WHITE,
                Align::Center,
            );
            if h.rain >= 20 {
                c.text(
                    &format!("{} %", h.rain),
                    x,
                    y + 124.0,
                    13,
                    Weight::Regular,
                    Rgba::hex(0x8cc8ff),
                    Align::Center,
                );
            }
        }
        // days
        let y = 500.0;
        let days: Vec<_> = w.days.iter().take(7).collect();
        let (lo, hi) = days.iter().fold((f32::MAX, f32::MIN), |a, d| {
            (a.0.min(d.min), a.1.max(d.max))
        });
        let dw = 1152.0 / days.len().max(1) as f32;
        for (i, d) in days.iter().enumerate() {
            let x = PAD + dw * i as f32;
            c.rect(
                x + 4.0,
                y,
                dw - 8.0,
                180.0,
                18.0,
                Rgba(255, 255, 255, if i == 0 { 34 } else { 18 }),
            );
            let name = if i == 0 {
                "Heute".to_string()
            } else {
                crate::app::WEEKDAYS[d.weekday][..2].to_string()
            };
            c.text(
                &name,
                x + dw / 2.0,
                y + 14.0,
                17,
                Weight::Semi,
                color::WHITE,
                Align::Center,
            );
            icon_weather(
                c,
                crate::weather::describe(d.code).0,
                true,
                x + dw / 2.0,
                y + 66.0,
                46.0,
            );
            c.text(
                &format!("{:.0}°", d.max),
                x + dw / 2.0,
                y + 100.0,
                20,
                Weight::Bold,
                color::WHITE,
                Align::Center,
            );
            c.text(
                &format!("{:.0}°", d.min),
                x + dw / 2.0,
                y + 126.0,
                16,
                Weight::Regular,
                color::WHITE.a(170),
                Align::Center,
            );
            // temperature range bar
            let span = (hi - lo).max(1.0);
            let bx = x + 20.0;
            let bw = dw - 40.0;
            c.rect(bx, y + 156.0, bw, 5.0, 2.5, Rgba(255, 255, 255, 40));
            let a = (d.min - lo) / span;
            let b = (d.max - lo) / span;
            c.rect(
                bx + bw * a,
                y + 156.0,
                (bw * (b - a)).max(6.0),
                5.0,
                2.5,
                Rgba::hex(0xffb347),
            );
            if d.rain >= 30 {
                c.text(
                    &format!("{} %", d.rain),
                    x + dw - 14.0,
                    y + 14.0,
                    12,
                    Weight::Regular,
                    Rgba::hex(0x8cc8ff),
                    Align::Right,
                );
            }
        }
    }
}
