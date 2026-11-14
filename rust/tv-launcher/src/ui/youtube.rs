//! YouTube: search (on-screen keyboard or the phone), results as a grid,
//! playback through the Pi's own player.
use super::*;
use crate::app::{load, Ctx, Nav, Screen, Slot};
use crate::gfx::{color, Align, Canvas, Rgba, Weight};
use crate::youtube::{Stream, Video};

const COLS: usize = 4;
const TW: f32 = 270.0;
const TH: f32 = 152.0;
const GAP: f32 = 24.0;
const RH: f32 = TH + 96.0;

pub struct YouTube {
    query: String,
    results: Slot<Result<Vec<Video>, String>>,
    /// -1: search bar, -2..: chips; >=0 grid
    sel: i64,
    chip: usize,
    kb: Option<Keyboard>,
    opening: Option<(Video, Slot<Result<Stream, String>>)>,
    scroll: Anim,
    frame: Focus,
}

const DEFAULT_CHIPS: [&str; 6] = [
    "Musik",
    "Nachrichten",
    "Dokumentation",
    "Comedy",
    "Fußball",
    "Gaming",
];

impl YouTube {
    pub fn new(cx: &mut Ctx) -> YouTube {
        let tx = cx.tx.clone();
        let first = cx.cfg.youtube_history.first().cloned();
        let results = if let Some(home) = cx.youtube_home.get().filter(|v| !v.is_empty()) {
            let ready = Slot::empty();
            ready.set(Ok(home));
            ready
        } else {
            load(&tx, move || {
                let home: Vec<Video> = crate::youtube::home()
                    .into_iter()
                    .flat_map(|r| r.1)
                    .collect();
                if home.len() >= 8 {
                    return Ok(home);
                }
                crate::youtube::search(first.as_deref().unwrap_or("Musik Neuheiten"))
            })
        };
        YouTube {
            query: String::new(),
            results,
            sel: 0,
            chip: 0,
            kb: None,
            opening: None,
            scroll: Anim::default(),
            frame: Focus::default(),
        }
    }

    fn chips(&self, cx: &Ctx) -> Vec<String> {
        let mut v: Vec<String> = cx.cfg.youtube_history.iter().take(4).cloned().collect();
        for d in DEFAULT_CHIPS {
            if v.len() >= 8 {
                break;
            }
            if !v.iter().any(|x| x.eq_ignore_ascii_case(d)) {
                v.push(d.to_string());
            }
        }
        v
    }

    fn search(&mut self, q: &str, cx: &mut Ctx) {
        let q = q.trim().to_string();
        if q.is_empty() {
            return;
        }
        self.query = q.clone();
        cx.cfg
            .youtube_history
            .retain(|x| !x.eq_ignore_ascii_case(&q));
        cx.cfg.youtube_history.insert(0, q.clone());
        cx.cfg.youtube_history.truncate(12);
        cx.save();
        self.results = load(&cx.tx, move || crate::youtube::search(&q));
        self.sel = 0;
    }
}

impl Screen for YouTube {
    fn name(&self) -> &'static str {
        "youtube"
    }
    fn animating(&self, _: &Ctx) -> bool {
        self.scroll.moving()
            || self.frame.moving()
            || self.kb.is_some()
            || self.opening.is_some()
            || !self.results.is_set()
    }
    fn text(&mut self, s: &str, cx: &mut Ctx) {
        // the phone sends whole words: search right away
        self.kb = None;
        self.search(s, cx);
    }
    fn poll(&mut self, cx: &mut Ctx) -> Nav {
        if let Some((v, slot)) = &self.opening {
            if let Some(r) = slot.get() {
                let v = v.clone();
                self.opening = None;
                match r {
                    Ok(s) => {
                        return Nav::Push(Box::new(super::player::PlayerScreen::youtube(cx, &v, s)))
                    }
                    Err(e) => cx.toast(format!("Video geht nicht: {e}")),
                }
            }
        }
        Nav::None
    }
    fn key(&mut self, k: Key, cx: &mut Ctx) -> Nav {
        if let Some(kb) = self.kb.as_mut() {
            match kb.key(k) {
                KbResult::Done(q) => {
                    self.kb = None;
                    self.search(&q, cx);
                }
                KbResult::Cancel => self.kb = None,
                _ => {}
            }
            return Nav::None;
        }
        if self.opening.is_some() {
            if k == Key::Back {
                self.opening = None;
            }
            return Nav::None;
        }
        let vids = self.results.get().and_then(|r| r.ok()).unwrap_or_default();
        let chips = self.chips(cx);
        match k {
            Key::Back => return Nav::Pop,
            Key::Home => return Nav::Home,
            Key::Char(ch) => {
                let mut kb = Keyboard::new("YouTube durchsuchen", "", false);
                kb.text.push(ch);
                self.kb = Some(kb);
                return Nav::None;
            }
            _ => {}
        }
        if self.sel < 0 {
            match k {
                Key::Ok if self.sel == -1 => {
                    self.kb = Some(Keyboard::new("YouTube durchsuchen", &self.query, false))
                }
                Key::Ok => {
                    let q = chips[self.chip].clone();
                    self.search(&q, cx);
                }
                Key::Down if self.sel == -1 => self.sel = -2,
                Key::Down if !vids.is_empty() => self.sel = 0,
                Key::Up if self.sel == -2 => self.sel = -1,
                Key::Left if self.sel == -2 && self.chip > 0 => self.chip -= 1,
                Key::Right if self.sel == -2 && self.chip + 1 < chips.len() => self.chip += 1,
                _ => {}
            }
            return Nav::None;
        }
        let i = self.sel as usize;
        match k {
            Key::Ok | Key::Play | Key::PlayPause if i < vids.len() => {
                let v = vids[i].clone();
                let id = v.id.clone();
                self.opening = Some((v, load(&cx.tx, move || crate::youtube::stream(&id))));
            }
            Key::Up if i < COLS => self.sel = -2,
            _ => {
                if let Some(j) = grid_move(i, vids.len(), COLS, k) {
                    self.sel = j as i64;
                }
            }
        }
        Nav::None
    }

    fn draw(&mut self, c: &mut Canvas, cx: &mut Ctx) {
        let thumb = if self.sel >= 0 {
            self.results
                .get()
                .and_then(|r| r.ok())
                .and_then(|v| v.get(self.sel as usize).map(|x| x.thumb.clone()))
        } else {
            None
        };
        let amb = thumb.and_then(|t| cx.images.get(&t, 320, 180));
        c.ambient(
            amb.map(crate::gfx::Ambient::Image)
                .unwrap_or(crate::gfx::Ambient::Color(Rgba::hex(0xd0202c))),
        );
        icon_youtube(c, PAD + 22.0, 58.0, 44.0);
        c.text(
            "YouTube",
            PAD + 56.0,
            40.0,
            30,
            Weight::Bold,
            color::TEXT,
            Align::Left,
        );
        status(c, cx);
        // search bar + chips
        let top = self.sel < 0;
        let sx = 330.0;
        let f = self.sel == -1;
        c.rect(
            sx,
            40.0,
            520.0,
            46.0,
            23.0,
            if f { color::WHITE } else { color::WHITE.a(22) },
        );
        icon_search(
            c,
            sx + 28.0,
            63.0,
            22.0,
            if f { color::BLACK } else { color::MUTED },
        );
        let q = if self.query.is_empty() {
            "Suchen (oder am Handy tippen)".to_string()
        } else {
            self.query.clone()
        };
        c.text_fit(
            &q,
            sx + 52.0,
            52.0,
            18,
            Weight::Regular,
            if f {
                color::BLACK
            } else if self.query.is_empty() {
                color::MUTED
            } else {
                color::TEXT
            },
            440.0,
        );
        let mut x = PAD;
        for (i, ch) in self.chips(cx).iter().enumerate() {
            let w = c.fonts.width(ch, 16, Weight::Semi) + 32.0;
            let foc = self.sel == -2 && self.chip == i;
            c.rect(
                x,
                104.0,
                w,
                36.0,
                18.0,
                if foc {
                    color::WHITE
                } else {
                    color::WHITE.a(18)
                },
            );
            c.text(
                ch,
                x + w / 2.0,
                113.0,
                16,
                Weight::Semi,
                if foc { color::BLACK } else { color::TEXT },
                Align::Center,
            );
            x += w + 12.0;
        }
        let y0 = 162.0;
        match self.results.get() {
            None => {
                c.text(
                    "Wird geladen …",
                    PAD,
                    y0 + 20.0,
                    20,
                    Weight::Regular,
                    color::MUTED,
                    Align::Left,
                );
            }
            Some(Err(e)) => {
                c.text(
                    "YouTube ist gerade nicht erreichbar.",
                    PAD,
                    y0 + 20.0,
                    22,
                    Weight::Semi,
                    color::TEXT,
                    Align::Left,
                );
                c.text_fit(
                    &e,
                    PAD,
                    y0 + 56.0,
                    15,
                    Weight::Regular,
                    color::MUTED,
                    1100.0,
                );
            }
            Some(Ok(vids)) => {
                let sel = if top { 0 } else { self.sel as usize };
                self.scroll
                    .set(((sel / COLS) as f32 * RH - RH * 0.5).max(0.0));
                let s = self.scroll.step();
                let first = (s / RH) as usize * COLS;
                let mut focus_at = None;
                for i in first..(first + COLS * 3).min(vids.len()) {
                    let v = &vids[i];
                    let x = PAD + (i % COLS) as f32 * (TW + GAP);
                    let y = y0 + (i / COLS) as f32 * RH - s;
                    if y > 720.0 {
                        continue;
                    }
                    if let Some(img) = cx.images.get(&v.thumb, 320, 180) {
                        c.image_cover(&img, x, y, TW, TH, 12.0);
                    } else {
                        glass(c, x, y, TW, TH, 12.0);
                    }
                    if !v.length.is_empty() {
                        let lw = c.fonts.width(&v.length, 13, Weight::Semi) + 12.0;
                        c.rect(
                            x + TW - lw - 8.0,
                            y + TH - 28.0,
                            lw,
                            20.0,
                            5.0,
                            if v.length == "LIVE" {
                                Rgba::hex(0xcc0000)
                            } else {
                                Rgba(0, 0, 0, 200)
                            },
                        );
                        c.text(
                            &v.length,
                            x + TW - 8.0 - lw / 2.0,
                            y + TH - 26.0,
                            13,
                            Weight::Semi,
                            color::WHITE,
                            Align::Center,
                        );
                    }
                    let foc = !top && i == sel;
                    if foc {
                        focus_at = Some((x, y));
                    }
                    c.paragraph(
                        &v.title,
                        x,
                        y + TH + 10.0,
                        TW,
                        16,
                        Weight::Semi,
                        if foc {
                            color::WHITE
                        } else {
                            color::TEXT.a(220)
                        },
                        21.0,
                        2,
                    );
                    let meta = [v.channel.as_str(), v.views.as_str(), v.age.as_str()]
                        .iter()
                        .filter(|s| !s.is_empty())
                        .copied()
                        .collect::<Vec<_>>()
                        .join(" · ");
                    c.text_fit(
                        &meta,
                        x,
                        y + TH + 56.0,
                        13,
                        Weight::Regular,
                        color::MUTED,
                        TW,
                    );
                }
                match focus_at {
                    Some((x, y)) => self.frame.draw(c, x, y, TW, TH, 12.0),
                    None => self.frame.reset(),
                }
                if vids.is_empty() {
                    c.text(
                        "Nichts gefunden.",
                        PAD,
                        y0 + 20.0,
                        20,
                        Weight::Regular,
                        color::MUTED,
                        Align::Left,
                    );
                }
            }
        }
        if let Some((v, _)) = &self.opening {
            c.rect(0.0, 0.0, 1280.0, 720.0, 0.0, Rgba(0, 0, 0, 170));
            c.text(
                "Video wird geöffnet …",
                640.0,
                320.0,
                26,
                Weight::Semi,
                color::WHITE,
                Align::Center,
            );
            let t = c.ellipsize(&v.title, 18, Weight::Regular, 900.0);
            c.text(
                &t,
                640.0,
                364.0,
                18,
                Weight::Regular,
                color::MUTED,
                Align::Center,
            );
        }
        if let Some(kb) = &self.kb {
            c.rect(0.0, 0.0, 1280.0, 720.0, 0.0, Rgba(0, 0, 0, 200));
            c.rect(190.0, 150.0, 900.0, 470.0, 24.0, color::PANEL_SOLID);
            kb.draw(c, 220.0, 176.0, 840.0, cx.t());
        }
    }
}
