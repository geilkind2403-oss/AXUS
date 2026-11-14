//! Tagesschau headlines and the full article text.
use super::*;
use crate::app::{load, Ctx, Nav, Screen, Slot};
use crate::gfx::{color, Align, Canvas, Rgba, Weight};
use crate::news::NewsItem;

pub struct NewsScreen {
    list: ListView,
    article: Option<(NewsItem, Slot<Vec<(bool, String)>>, Anim)>,
}

impl NewsScreen {
    pub fn new() -> NewsScreen {
        NewsScreen {
            list: ListView::new(),
            article: None,
        }
    }
}

impl Screen for NewsScreen {
    fn name(&self) -> &'static str {
        "news"
    }
    fn animating(&self, _: &Ctx) -> bool {
        self.list.moving() || self.article.as_ref().map(|a| a.2.moving()).unwrap_or(false)
    }
    fn key(&mut self, k: Key, cx: &mut Ctx) -> Nav {
        let items = cx.news.get().unwrap_or_default();
        if let Some((_, _, scroll)) = self.article.as_mut() {
            match k {
                Key::Back => self.article = None,
                Key::Down | Key::ChDown => scroll.set(scroll.target + 220.0),
                Key::Up | Key::ChUp => scroll.set((scroll.target - 220.0).max(0.0)),
                Key::Home => return Nav::Home,
                _ => {}
            }
            return Nav::None;
        }
        match k {
            Key::Back => return Nav::Pop,
            Key::Home => return Nav::Home,
            Key::Menu => cx.news = load(&cx.tx, crate::news::fetch),
            Key::Ok if !items.is_empty() => {
                let n = items[self.list.sel.min(items.len() - 1)].clone();
                let url = n.details.clone();
                let slot = if url.is_empty() {
                    let s = Slot::empty();
                    s.set(vec![(false, n.first_sentence.clone())]);
                    s
                } else {
                    load(&cx.tx, move || crate::news::article(&url))
                };
                self.article = Some((n, slot, Anim::default()));
            }
            _ => {
                self.list.key(k, items.len());
            }
        }
        Nav::None
    }
    fn draw(&mut self, c: &mut Canvas, cx: &mut Ctx) {
        let items_now = cx.news.get().unwrap_or_default();
        let src = match &self.article {
            Some((n, _, _)) => n.image.clone(),
            None => items_now
                .get(self.list.sel)
                .map(|n| n.image.clone())
                .unwrap_or_default(),
        };
        let amb = if src.is_empty() {
            None
        } else {
            cx.images.get(&src, 320, 180)
        };
        c.ambient(
            amb.map(crate::gfx::Ambient::Image)
                .unwrap_or(crate::gfx::Ambient::Color(Rgba::hex(0x0b4fa3))),
        );
        status(c, cx);
        if let Some((n, body, scroll)) = self.article.as_mut() {
            let s = scroll.step();
            let mut y = 110.0 - s;
            if let Some(img) = cx.images.get(&n.image, 960, 540) {
                c.image_cover(&img, 840.0, 110.0, 376.0, 212.0, 16.0);
            }
            let w = 720.0;
            if !n.topline.is_empty() {
                c.text(
                    &n.topline,
                    PAD,
                    y,
                    17,
                    Weight::Semi,
                    color::ACCENT,
                    Align::Left,
                );
                y += 30.0;
            }
            for l in c.wrap(&n.title, 34, Weight::Bold, w) {
                c.text(&l, PAD, y, 34, Weight::Bold, color::TEXT, Align::Left);
                y += 44.0;
            }
            y += 16.0;
            match body.get() {
                None => {
                    c.text(
                        "Artikel wird geladen …",
                        PAD,
                        y,
                        18,
                        Weight::Regular,
                        color::MUTED,
                        Align::Left,
                    );
                }
                Some(paras) => {
                    for (head, p) in paras {
                        let (px, wt, lh) = if head {
                            (22, Weight::Bold, 32.0)
                        } else {
                            (19, Weight::Regular, 30.0)
                        };
                        for l in c.wrap(&p, px, wt, w) {
                            if y > -40.0 && y < 720.0 {
                                c.text(
                                    &l,
                                    PAD,
                                    y,
                                    px,
                                    wt,
                                    color::TEXT.a(if head { 255 } else { 220 }),
                                    Align::Left,
                                );
                            }
                            y += lh;
                        }
                        y += 14.0;
                    }
                    let max = (y + s - 600.0).max(0.0);
                    if scroll.target > max {
                        scroll.set(max);
                    }
                }
            }
            c.vgradient(
                0.0,
                0.0,
                1280.0,
                100.0,
                0.0,
                Rgba(12, 12, 16, 250),
                Rgba(12, 12, 16, 200),
            );
            title(c, "Nachrichten");
            status(c, cx);
            return;
        }
        title(c, "Nachrichten");
        let Some(items) = cx.news.get() else {
            c.text(
                "Wird geladen …",
                PAD,
                120.0,
                20,
                Weight::Regular,
                color::MUTED,
                Align::Left,
            );
            return;
        };
        if items.is_empty() {
            c.text(
                "Keine Verbindung zur Tagesschau.",
                PAD,
                120.0,
                20,
                Weight::Regular,
                color::MUTED,
                Align::Left,
            );
            return;
        }
        let n = items.len();
        self.list.draw(
            c,
            n,
            PAD - 12.0,
            104.0,
            1164.0,
            600.0,
            112.0,
            true,
            |c, i, x, y, f| {
                let it = &items[i];
                let col = if f { color::BLACK } else { color::TEXT };
                let mut tx = x + 16.0;
                if let Some(img) = cx.images.get(&it.image, 320, 180) {
                    c.image_cover(&img, x + 12.0, y + 10.0, 160.0, 86.0, 10.0);
                    tx = x + 190.0;
                }
                if !it.topline.is_empty() {
                    c.text_fit(
                        &it.topline,
                        tx,
                        y + 12.0,
                        14,
                        Weight::Semi,
                        if f {
                            Rgba::hex(0x1d4ed8)
                        } else {
                            color::ACCENT
                        },
                        820.0,
                    );
                }
                c.text_fit(&it.title, tx, y + 34.0, 21, Weight::Bold, col, 900.0);
                c.text_fit(
                    &it.first_sentence,
                    tx,
                    y + 66.0,
                    15,
                    Weight::Regular,
                    if f {
                        Rgba(60, 60, 60, 255)
                    } else {
                        color::MUTED
                    },
                    900.0,
                );
                c.text(
                    &crate::news::time_of(&it.date),
                    x + 1150.0,
                    y + 14.0,
                    14,
                    Weight::Regular,
                    if f {
                        Rgba(60, 60, 60, 255)
                    } else {
                        color::MUTED
                    },
                    Align::Right,
                );
            },
        );
    }
}
