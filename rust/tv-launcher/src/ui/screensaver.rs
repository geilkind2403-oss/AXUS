//! Screensaver: after a few idle minutes on a menu, a dark screen with the
//! time, date and weather that moves a little every minute (no burn-in on
//! OLED/plasma). Any key comes back -- that key does nothing else.
use super::*;
use crate::app::{Ctx, Nav, Screen};
use crate::gfx::{color, Align, Canvas, Rgba, Weight};

pub struct ScreenSaver;

impl Screen for ScreenSaver {
    fn name(&self) -> &'static str {
        "screensaver"
    }
    fn key(&mut self, _k: Key, _cx: &mut Ctx) -> Nav {
        Nav::Pop
    }
    fn draw(&mut self, c: &mut Canvas, cx: &mut Ctx) {
        c.clear(Rgba(0, 0, 0, 255));
        let tm = crate::app::local_tm();
        // a slow walk over the screen, one step a minute
        let step = (tm.tm_hour * 60 + tm.tm_min) as f32;
        let x = 640.0 + (step * 0.71).sin() * 300.0;
        let y = 300.0 + (step * 0.53).cos() * 140.0;
        c.text(
            &format!("{:02}:{:02}", tm.tm_hour, tm.tm_min),
            x,
            y - 60.0,
            120,
            Weight::Bold,
            Rgba(235, 235, 240, 255),
            Align::Center,
        );
        let date = format!(
            "{}, {}. {}",
            crate::app::WEEKDAYS[tm.tm_wday as usize % 7],
            tm.tm_mday,
            crate::app::MONTHS[tm.tm_mon as usize % 12]
        );
        c.text(
            &date,
            x,
            y + 90.0,
            24,
            Weight::Regular,
            color::MUTED,
            Align::Center,
        );
        if let Some(Ok(w)) = cx.weather.get() {
            let (sky, desc) = crate::weather::describe(w.code);
            icon_weather(c, sky, w.day, x - 90.0, y + 160.0, 44.0);
            c.text(
                &format!("{:.0}° · {desc}", w.temp),
                x - 58.0,
                y + 146.0,
                22,
                Weight::Semi,
                Rgba(210, 210, 216, 255),
                Align::Left,
            );
        }
    }
}
