//! Films: the poster wall and a film's page.
use super::*;
use crate::app::{Ctx, Nav, Screen};
use crate::films::{Film, Origin};
use crate::gfx::{color, Align, Canvas, Rgba, Weight};

const COLS: usize = 6;
const CW: f32 = 170.0;
const CH: f32 = 255.0;
const GAP: f32 = 26.4;
const ROW_H: f32 = CH + 70.0;

pub struct Films {
    sel: usize,
    scroll: Anim,
    frame: Focus,
}

impl Films {
    pub fn new(cx: &mut Ctx) -> Films {
        let (srv, dirs, pos) = (
            cx.cfg.film_server.clone(),
            cx.cfg.film_dirs.clone(),
            cx.cfg.positions.clone(),
        );
        cx.films.refresh(&cx.tx, &srv, &dirs, &pos);
        Films {
            sel: 0,
            scroll: Anim::default(),
            frame: Focus::default(),
        }
    }
}

pub fn poster(c: &mut Canvas, cx: &mut Ctx, f: &Film, x: f32, y: f32, w: f32, h: f32) {
    if let Some(img) = cx.images.get(&f.icon, w as u32 * 2, h as u32 * 2) {
        c.image_cover(&img, x, y, w, h, 14.0);
    } else {
        glass(c, x, y, w, h, 14.0);
        icon_film(c, x + w / 2.0, y + h * 0.36, 54.0, color::MUTED);
        c.paragraph(
            &f.title,
            x + 14.0,
            y + h * 0.58,
            w - 28.0,
            16,
            Weight::Semi,
            color::TEXT,
            20.0,
            3,
        );
    }
    if f.on_server() {
        c.rect(x + w - 40.0, y + 8.0, 32.0, 22.0, 11.0, Rgba(0, 0, 0, 160));
        c.text(
            "PC",
            x + w - 24.0,
            y + 11.0,
            12,
            Weight::Bold,
            color::TEXT,
            Align::Center,
        );
    }
    if f.position > 60.0 && f.duration > 0.0 {
        let p = (f.position / f.duration).min(1.0) as f32;
        c.rect(
            x + 10.0,
            y + h - 12.0,
            w - 20.0,
            4.0,
            2.0,
            Rgba(255, 255, 255, 70),
        );
        c.rect(
            x + 10.0,
            y + h - 12.0,
            (w - 20.0) * p,
            4.0,
            2.0,
            color::ACCENT,
        );
    }
}

impl Screen for Films {
    fn name(&self) -> &'static str {
        "films"
    }
    fn key(&mut self, k: Key, cx: &mut Ctx) -> Nav {
        let list = cx.films.list.get().unwrap_or_default();
        let n = list.films.len();
        match k {
            Key::Back => return Nav::Pop,
            Key::Home => return Nav::Home,
            Key::Ok if n > 0 => {
                return Nav::Push(Box::new(FilmPage::new(
                    list.films[self.sel.min(n - 1)].clone(),
                )))
            }
            Key::Menu => {
                let (srv, dirs, pos) = (
                    cx.cfg.film_server.clone(),
                    cx.cfg.film_dirs.clone(),
                    cx.cfg.positions.clone(),
                );
                cx.films.list = crate::app::Slot::empty();
                cx.films.refresh(&cx.tx, &srv, &dirs, &pos);
            }
            Key::ChDown => self.sel = (self.sel + COLS * 2).min(n.saturating_sub(1)),
            Key::ChUp => self.sel = self.sel.saturating_sub(COLS * 2),
            _ => {
                if let Some(i) = grid_move(self.sel, n, COLS, k) {
                    self.sel = i;
                }
            }
        }
        Nav::None
    }
    fn animating(&self, _: &Ctx) -> bool {
        self.scroll.moving() || self.frame.moving()
    }
    fn draw(&mut self, c: &mut Canvas, cx: &mut Ctx) {
        let focused_icon = cx
            .films
            .list
            .get()
            .and_then(|l| l.films.get(self.sel).map(|f| f.icon.clone()))
            .unwrap_or_default();
        let amb = if focused_icon.is_empty() {
            None
        } else {
            cx.images.get(&focused_icon, 340, 510)
        };
        c.ambient(
            amb.map(crate::gfx::Ambient::Image)
                .unwrap_or(crate::gfx::Ambient::Color(Rgba::hex(0xc8963c))),
        );
        title(c, "Filme");
        status(c, cx);
        let Some(list) = cx.films.list.get() else {
            c.text(
                "Filme werden gesucht …",
                PAD,
                120.0,
                20,
                Weight::Regular,
                color::MUTED,
                Align::Left,
            );
            return;
        };
        let mut y0 = 104.0;
        for e in &list.errors {
            c.text(e, PAD, y0, 15, Weight::Regular, color::RED, Align::Left);
            y0 += 22.0;
        }
        if list.films.is_empty() {
            c.paragraph(
                "Keine Filme gefunden.\n\nAuf dem Pi: Filme auf einen USB-Stick oder nach /storage/videos legen.\nVom PC: tv-server starten und unter Einstellungen > Server die Adresse eintragen.",
                PAD, y0 + 20.0, 900.0, 19, Weight::Regular, color::MUTED, 28.0, 10,
            );
            return;
        }
        let n = list.films.len();
        self.sel = self.sel.min(n - 1);
        let row = self.sel / COLS;
        self.scroll.set((row as f32 * ROW_H - ROW_H * 0.6).max(0.0));
        let s = self.scroll.step();
        let top = y0 + 10.0;
        let first = (s / ROW_H) as usize * COLS;
        let mut focus_at = None;
        // the row above peeks in below the header, it must not cover it
        c.clip_y(Some((y0 - 8.0, crate::gfx::H as f32)));
        for i in first..(first + COLS * 4).min(n) {
            let f = &list.films[i];
            let x = PAD + (i % COLS) as f32 * (CW + GAP);
            let y = top + (i / COLS) as f32 * ROW_H - s;
            if y > crate::gfx::H as f32 || y + ROW_H < 0.0 {
                continue;
            }
            poster(c, cx, f, x, y, CW, CH);
            let focused = i == self.sel;
            if focused {
                focus_at = Some((x, y));
            }
            c.text_fit(
                &f.title,
                x + 2.0,
                y + CH + 12.0,
                16,
                if focused {
                    Weight::Semi
                } else {
                    Weight::Regular
                },
                if focused { color::TEXT } else { color::MUTED },
                CW,
            );
        }
        if let Some((x, y)) = focus_at {
            self.frame.draw(c, x, y, CW, CH, 14.0);
        }
        c.clip_y(None);
        c.text(
            "OK Öffnen · Menü Neu laden",
            crate::gfx::W as f32 - PAD,
            66.0,
            14,
            Weight::Regular,
            color::MUTED,
            Align::Right,
        );
    }
}

/// A film's page: backdrop, story, play buttons, sound / subtitle choice.
pub struct FilmPage {
    film: Film,
    sel: usize,
    audio: usize,
    sub: i64,
}

impl FilmPage {
    pub fn new(film: Film) -> FilmPage {
        let (audio, sub) = match &film.origin {
            Origin::Server { pick, .. } => *pick,
            _ => (usize::MAX, -2),
        };
        FilmPage {
            film,
            sel: 0,
            audio,
            sub,
        }
    }
    fn buttons(&self) -> Vec<String> {
        let mut b = Vec::new();
        if self.film.position > 60.0 {
            b.push(format!(
                "Fortsetzen ({})",
                crate::app::fmt_time(self.film.position)
            ));
            b.push("Von vorne".into());
        } else {
            b.push("Abspielen".into());
        }
        if let Origin::Server { audio, subs, .. } = &self.film.origin {
            if audio.len() > 1 {
                let (l, t) = audio.get(self.audio).cloned().unwrap_or_default();
                b.push(format!("Ton: {}", lang_name(&l, &t)));
            }
            if !subs.is_empty() {
                let s = subs
                    .iter()
                    .find(|s| s.0 as i64 == self.sub)
                    .map(|s| {
                        format!(
                            "{}{}",
                            lang_name(&s.1, ""),
                            if s.2 { " (erzwungen)" } else { "" }
                        )
                    })
                    .unwrap_or_else(|| "Aus".into());
                b.push(format!("Untertitel: {s}"));
            }
        }
        b
    }
}

pub fn lang_name(code: &str, title: &str) -> String {
    let n = match code.to_lowercase().as_str() {
        "ger" | "deu" | "de" => "Deutsch",
        "eng" | "en" => "Englisch",
        "fre" | "fra" | "fr" => "Französisch",
        "spa" | "es" => "Spanisch",
        "ita" | "it" => "Italienisch",
        "jpn" | "ja" => "Japanisch",
        "" | "und" => "Unbekannt",
        other => {
            return if title.is_empty() {
                other.to_string()
            } else {
                format!("{other} ({title})")
            }
        }
    };
    if title.is_empty() {
        n.into()
    } else {
        format!("{n} – {title}")
    }
}

impl Screen for FilmPage {
    fn name(&self) -> &'static str {
        "film"
    }
    fn key(&mut self, k: Key, cx: &mut Ctx) -> Nav {
        let b = self.buttons();
        match k {
            Key::Back => return Nav::Pop,
            Key::Home => return Nav::Home,
            Key::Up if self.sel > 0 => self.sel -= 1,
            Key::Down if self.sel + 1 < b.len() => self.sel += 1,
            Key::Ok | Key::Play | Key::PlayPause => {
                let label = &b[self.sel];
                if label.starts_with("Ton:") {
                    if let Origin::Server { audio, .. } = &self.film.origin {
                        self.audio = (self.audio + 1) % audio.len().max(1);
                    }
                } else if label.starts_with("Untertitel") {
                    if let Origin::Server { subs, .. } = &self.film.origin {
                        let ids: Vec<i64> = std::iter::once(-1)
                            .chain(subs.iter().map(|s| s.0 as i64))
                            .collect();
                        let i = ids.iter().position(|x| *x == self.sub).unwrap_or(0);
                        self.sub = ids[(i + 1) % ids.len()];
                    }
                } else {
                    let start = if label.starts_with("Fortsetzen") {
                        (self.film.position - 5.0).max(0.0)
                    } else {
                        0.0
                    };
                    return Nav::Push(Box::new(super::player::PlayerScreen::film(
                        cx, &self.film, start, self.audio, self.sub,
                    )));
                }
            }
            _ => {}
        }
        Nav::None
    }
    fn draw(&mut self, c: &mut Canvas, cx: &mut Ctx) {
        // cinema style: the film's picture big on the right, fading into the page
        let f = self.film.clone();
        let src = if f.banner.is_empty() {
            f.icon.clone()
        } else {
            f.banner.clone()
        };
        let img = cx.images.get(&src, 1280, 720);
        c.ambient(
            img.clone()
                .map(crate::gfx::Ambient::Image)
                .unwrap_or(crate::gfx::Ambient::Color(Rgba::hex(0x3a4a7a))),
        );
        let hero = img.is_some() && !f.banner.is_empty();
        if let (true, Some(img)) = (hero, img.as_ref()) {
            c.image_fit(img, 380.0, 0.0, 900.0, 540.0, 0.0, true, 1.0);
            c.hgradient(
                380.0,
                0.0,
                420.0,
                540.0,
                Rgba(14, 14, 18, 255),
                Rgba(14, 14, 18, 0),
            );
            c.vgradient(
                380.0,
                300.0,
                900.0,
                240.0,
                0.0,
                Rgba(14, 14, 18, 0),
                Rgba(14, 14, 18, 255),
            );
            c.rect(0.0, 0.0, 380.0, 540.0, 0.0, Rgba(14, 14, 18, 255));
            c.rect(0.0, 540.0, 1280.0, 180.0, 0.0, Rgba(14, 14, 18, 255));
        }
        status(c, cx);
        let (x, w) = if hero {
            (PAD, 600.0)
        } else {
            (PAD + 260.0, 1280.0 - PAD - 260.0 - PAD)
        };
        if !hero {
            poster(c, cx, &f, PAD, 110.0, 220.0, 330.0);
        }
        let mut y = 112.0;
        for line in c.wrap(&f.title, 44, Weight::Bold, w).into_iter().take(2) {
            c.text(&line, x, y, 44, Weight::Bold, color::TEXT, Align::Left);
            y += 54.0;
        }
        // meta as small chips
        let mut cxp = x;
        for m in f.meta().split(" · ").filter(|m| !m.is_empty()) {
            let tw = c.fonts.width(m, 15, Weight::Semi) + 24.0;
            c.rect(cxp, y + 4.0, tw, 28.0, 14.0, Rgba(255, 255, 255, 26));
            c.text(
                m,
                cxp + tw / 2.0,
                y + 9.0,
                15,
                Weight::Semi,
                color::TEXT,
                Align::Center,
            );
            cxp += tw + 8.0;
        }
        y += 50.0;
        y += c.paragraph(
            &f.plot(),
            x,
            y,
            w.min(760.0),
            18,
            Weight::Regular,
            color::TEXT.a(225),
            27.0,
            if hero { 6 } else { 7 },
        ) + 14.0;
        if !f.cast.is_empty() {
            c.text_fit(
                &format!("Mit: {}", f.cast.replace('\n', ", ")),
                x,
                y,
                15,
                Weight::Regular,
                color::MUTED,
                w.min(760.0),
            );
            y += 24.0;
        }
        if !f.crew.is_empty() {
            c.text_fit(
                &format!("Von: {}", f.crew.replace('\n', ", ")),
                x,
                y,
                15,
                Weight::Regular,
                color::MUTED,
                w.min(760.0),
            );
        }
        let by = 470.0;
        for (i, b) in self.buttons().iter().enumerate() {
            let yy = by + i as f32 * 58.0;
            let focused = i == self.sel;
            let bw = 360.0;
            c.rect(
                PAD,
                yy,
                bw,
                46.0,
                23.0,
                if focused {
                    color::WHITE
                } else {
                    color::WHITE.a(28)
                },
            );
            if i == 0 {
                icon_play(
                    c,
                    PAD + 30.0,
                    yy + 23.0,
                    18.0,
                    if focused { color::BLACK } else { color::TEXT },
                );
            }
            c.text_fit(
                b,
                PAD + 52.0,
                yy + 12.0,
                18,
                Weight::Semi,
                if focused { color::BLACK } else { color::TEXT },
                bw - 70.0,
            );
        }
        let origin = match &f.origin {
            Origin::Local(p) => format!("Auf dem Pi: {}", p.display()),
            Origin::Server { codec, .. } => format!(
                "Vom PC-Server{}",
                codec
                    .as_ref()
                    .map(|c| if c == "h264" {
                        String::new()
                    } else {
                        format!(" · {} wird für den Pi umgewandelt", c.to_uppercase())
                    })
                    .unwrap_or_default()
            ),
        };
        c.text_fit(
            &origin,
            PAD,
            690.0,
            13,
            Weight::Regular,
            color::MUTED,
            1100.0,
        );
    }
}
