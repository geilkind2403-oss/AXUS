//! Tagesschau news in the launcher's gold-glass layout.
use super::*;
use crate::app::{load, Ctx, Nav, Screen, Slot};
use crate::gfx::{Align, Canvas, Image, LayerCache, Rgba, Weight};
use crate::news::NewsItem;
use crate::platform::input::Key;
use std::time::{Duration, Instant};

/// The preview photo, decoded at the size it is shown (physical px): a
/// picture that needs no resampling is copied, not filtered.
const PREVIEW_PX: (u32, u32) = (
    (544.0 * crate::gfx::RENDER_SCALE) as u32,
    (306.0 * crate::gfx::RENDER_SCALE) as u32,
);
const X: f32 = 650.0;
const RIGHT: f32 = 1220.0;
const ROW_W: f32 = RIGHT - X;
const ROW_Y: f32 = 154.0;
const ROW_H: f32 = 43.0;
const ROW_STEP: f32 = 55.0;
const VISIBLE_ROWS: usize = 9;
const PREVIEW: (f32, f32, f32, f32) = (64.0, 150.0, 544.0, 510.0);
const STATUS: (f32, f32, f32, f32) = (920.0, 14.0, 300.0, 54.0);
const CREAM: Rgba = Rgba(250, 247, 239, 255);
const MUTED: Rgba = Rgba(180, 166, 136, 255);
const GOLD: Rgba = Rgba(226, 187, 116, 255);

const CATEGORIES: [(&str, &str); 7] = [
    ("inland", "Inland"),
    ("ausland", "Ausland"),
    ("wirtschaft", "Wirtschaft"),
    ("sport", "Sport"),
    ("wissenschaft", "Wissen"),
    ("kultur", "Kultur"),
    ("panorama", "Panorama"),
];

fn draw_editorial_placeholder(c: &mut Canvas) {
    // A small editorial landscape and article lines while a photo is unavailable.
    c.rect(64.0, 150.0, 544.0, 306.0, 18.0, Rgba(36, 30, 21, 255));
    c.rect(78.0, 164.0, 516.0, 278.0, 14.0, Rgba(83, 60, 29, 54));
    c.rect(88.0, 178.0, 276.0, 210.0, 12.0, Rgba(51, 43, 31, 255));
    c.rect(96.0, 186.0, 260.0, 194.0, 8.0, Rgba(112, 83, 43, 95));
    c.circle(300.0, 222.0, 20.0, Rgba(226, 187, 116, 185));
    c.polygon(
        &[
            (96.0, 326.0),
            (146.0, 267.0),
            (180.0, 299.0),
            (225.0, 252.0),
            (282.0, 316.0),
            (356.0, 276.0),
            (356.0, 380.0),
            (96.0, 380.0),
        ],
        Rgba(126, 91, 43, 195),
    );
    c.polygon(
        &[
            (96.0, 344.0),
            (164.0, 302.0),
            (208.0, 333.0),
            (260.0, 291.0),
            (316.0, 342.0),
            (356.0, 320.0),
            (356.0, 380.0),
            (96.0, 380.0),
        ],
        Rgba(65, 56, 39, 240),
    );
    c.text(
        "NACHRICHTEN",
        388.0,
        202.0,
        12,
        Weight::Semi,
        GOLD,
        Align::Left,
    );
    c.rect(388.0, 226.0, 170.0, 8.0, 4.0, Rgba(226, 187, 116, 155));
    c.rect(388.0, 244.0, 150.0, 6.0, 3.0, Rgba(226, 187, 116, 82));
    c.rect(388.0, 258.0, 184.0, 6.0, 3.0, Rgba(226, 187, 116, 64));
    c.rect(388.0, 272.0, 162.0, 6.0, 3.0, Rgba(226, 187, 116, 64));
    c.rect(388.0, 286.0, 178.0, 6.0, 3.0, Rgba(226, 187, 116, 64));
    c.rect(388.0, 316.0, 176.0, 5.0, 2.5, Rgba(226, 187, 116, 40));
    c.rect(388.0, 328.0, 158.0, 5.0, 2.5, Rgba(226, 187, 116, 40));
    c.rect(388.0, 340.0, 170.0, 5.0, 2.5, Rgba(226, 187, 116, 40));
    c.text(
        "TAGESSCHAU",
        88.0,
        405.0,
        13,
        Weight::Semi,
        Rgba(226, 187, 116, 210),
        Align::Left,
    );
}

fn item_category(item: &NewsItem) -> &str {
    if item.category.is_empty() {
        "inland"
    } else {
        &item.category
    }
}

struct ArticleView {
    item: NewsItem,
    body: Slot<Vec<(bool, String)>>,
    scroll: Anim,
    prepared_revision: u64,
    lines: Vec<(bool, Vec<String>)>,
    content_height: f32,
}

pub struct NewsScreen {
    selected: usize,
    category: usize,
    category_codes: Vec<String>,
    items: Vec<NewsItem>,
    data_revision: u64,
    cache_checked: bool,
    article: Option<ArticleView>,
    base_cache: LayerCache,
    base_image: Option<Image>,
    /// the last list pictures by key, so ◀▶ back to a category is a copy
    base_kept: Vec<((u64, usize, usize, bool, usize), Image)>,
    base_key: Option<(u64, usize, usize, bool, usize)>,
    ambient_until: Option<Instant>,
    last_focus_row: Option<f32>,
    last_preview_key: Option<(u64, usize, usize, u8)>,
    last_article_key: Option<(u64, i32)>,
}

impl NewsScreen {
    pub fn new() -> NewsScreen {
        NewsScreen {
            selected: 0,
            category: 0,
            category_codes: Vec::new(),
            items: Vec::new(),
            data_revision: 0,
            cache_checked: false,
            article: None,
            base_cache: LayerCache::default(),
            base_image: None,
            base_kept: Vec::new(),
            base_key: None,
            ambient_until: None,
            last_focus_row: None,
            last_preview_key: None,
            last_article_key: None,
        }
    }

    fn sync_items(&mut self, cx: &mut Ctx) {
        if !self.cache_checked {
            self.cache_checked = true;
            if !cx.news.is_set() {
                if let Some(cached) = crate::news::load_cached() {
                    cx.news.set(cached);
                }
            }
        }
        let revision = cx.news.revision();
        if revision == self.data_revision {
            return;
        }
        self.data_revision = revision;
        self.items = cx.news.get().unwrap_or_default();
        let prior = self
            .category_codes
            .get(self.category)
            .cloned()
            .unwrap_or_default();
        self.category_codes.clear();
        for (code, _) in CATEGORIES {
            if self.items.iter().any(|item| item_category(item) == code) {
                self.category_codes.push(code.to_string());
            }
        }
        if self.items.iter().any(|item| {
            !CATEGORIES
                .iter()
                .any(|(code, _)| *code == item_category(item))
        }) {
            self.category_codes.push("sonstiges".into());
        }
        self.category_codes.push(String::new()); // Alle
        self.category = self
            .category_codes
            .iter()
            .position(|code| !prior.is_empty() && *code == prior)
            .or_else(|| self.category_codes.iter().position(|code| code == "inland"))
            .unwrap_or(0);
        self.selected = 0;
        self.base_cache = LayerCache::default();
        self.base_image = None;
        self.base_key = None;
        self.last_focus_row = None;
        self.last_preview_key = None;
    }

    fn category_code(&self) -> &str {
        self.category_codes
            .get(self.category)
            .map(String::as_str)
            .unwrap_or("")
    }

    fn category_name(&self) -> &str {
        match self.category_code() {
            "" => "Alle",
            "sonstiges" => "Weitere",
            code => CATEGORIES
                .iter()
                .find(|(known, _)| *known == code)
                .map(|(_, label)| *label)
                .unwrap_or("Weitere"),
        }
    }

    fn filtered_indices(&self) -> Vec<usize> {
        let code = self.category_code();
        self.items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                (code.is_empty() || item_category(item) == code).then_some(index)
            })
            .collect()
    }

    fn row_y(&self) -> Option<f32> {
        let first = self.selected.saturating_sub(VISIBLE_ROWS - 1);
        (self.selected >= first).then_some(ROW_Y + (self.selected - first) as f32 * ROW_STEP)
    }

    fn preview_item(&self, indices: &[usize]) -> Option<&NewsItem> {
        indices
            .get(self.selected)
            .and_then(|index| self.items.get(*index))
    }

    fn open_article(&mut self, cx: &Ctx, item: NewsItem) {
        let body = if item.details.is_empty() {
            let slot = Slot::empty();
            slot.set(vec![(false, item.first_sentence.clone())]);
            slot
        } else {
            let url = item.details.clone();
            load(&cx.tx, move || crate::news::article(&url))
        };
        self.article = Some(ArticleView {
            item,
            body,
            scroll: Anim::default(),
            prepared_revision: u64::MAX,
            lines: Vec::new(),
            content_height: 0.0,
        });
        self.last_article_key = None;
        self.last_preview_key = None;
        self.base_cache = LayerCache::default();
        self.base_image = None;
        self.base_key = None;
    }

    fn prepare_article(&mut self, c: &mut Canvas) {
        let Some(article) = self.article.as_mut() else {
            return;
        };
        let revision = article.body.revision();
        if revision == article.prepared_revision {
            return;
        }
        article.prepared_revision = revision;
        article.lines.clear();
        article.content_height = 0.0;
        if let Some(paragraphs) = article.body.get() {
            for (heading, text) in paragraphs {
                if text.trim().is_empty() {
                    article.content_height += 12.0;
                    continue;
                }
                let (size, weight, line_height) = if heading {
                    (22, Weight::Bold, 32.0)
                } else {
                    (18, Weight::Regular, 27.0)
                };
                let lines = c.wrap(&text, size, weight, ROW_W - 36.0);
                article.content_height += lines.len() as f32 * line_height + 14.0;
                article.lines.push((heading, lines));
            }
        }
        let max_scroll = (article.content_height - 450.0).max(0.0);
        if article.scroll.target > max_scroll {
            article.scroll.set(max_scroll);
        }
    }

    fn draw_preview(&self, c: &mut Canvas, cx: &mut Ctx, item: Option<&NewsItem>) {
        let Some(item) = item else {
            c.rect(64.0, 150.0, 544.0, 306.0, 18.0, Rgba(42, 34, 22, 205));
            c.text(
                if self.items.is_empty() {
                    "Meldungen werden geladen …"
                } else {
                    "Keine Meldungen in dieser Rubrik."
                },
                84.0,
                292.0,
                18,
                Weight::Regular,
                MUTED,
                Align::Left,
            );
            return;
        };
        if let Some(image) = cx.images.get_cover(&item.image, PREVIEW_PX.0, PREVIEW_PX.1) {
            c.image_cover(&image, 64.0, 150.0, 544.0, 306.0, 18.0);
        } else {
            draw_editorial_placeholder(c);
        }
        if !item.topline.is_empty() {
            c.text_fit(&item.topline, 64.0, 480.0, 15, Weight::Semi, GOLD, 544.0);
        }
        let mut y = 510.0;
        for line in c
            .wrap(&item.title, 26, Weight::Bold, 544.0)
            .into_iter()
            .take(2)
        {
            c.text(&line, 64.0, y, 26, Weight::Bold, CREAM, Align::Left);
            y += 34.0;
        }
        if !item.first_sentence.trim().is_empty() {
            y += 8.0;
            c.paragraph(
                &item.first_sentence,
                64.0,
                y,
                544.0,
                16,
                Weight::Regular,
                MUTED,
                23.0,
                3,
            );
        }
    }

    fn draw_static(&self, c: &mut Canvas, indices: &[usize]) {
        c.ambient(crate::gfx::Ambient::LiquidGold);
        let hint = if self.article.is_some() {
            format!(
                "‹ {} ›  ·  {} Meldungen",
                self.category_name(),
                indices.len()
            )
        } else {
            format!(
                "‹ {} ›  ·  {} Meldungen",
                self.category_name(),
                indices.len()
            )
        };
        c.text(
            "Nachrichten",
            RIGHT,
            70.0,
            34,
            Weight::Bold,
            CREAM,
            Align::Right,
        );
        c.text(
            &hint,
            RIGHT,
            112.0,
            15,
            Weight::Regular,
            MUTED,
            Align::Right,
        );
        if self.article.is_none() {
            let first = self.selected.saturating_sub(VISIBLE_ROWS - 1);
            for visible in 0..VISIBLE_ROWS {
                let index = first + visible;
                let Some(item) = indices.get(index).and_then(|i| self.items.get(*i)) else {
                    break;
                };
                let y = ROW_Y + visible as f32 * ROW_STEP;
                c.rect(X, y, ROW_W, ROW_H, 15.0, Rgba(9, 9, 8, 66));
                c.text_fit(
                    &item.title,
                    X + 22.0,
                    y + 11.0,
                    18,
                    Weight::Regular,
                    CREAM,
                    ROW_W - 44.0,
                );
            }
            c.text(
                "↑↓ Meldung · ←→ Rubrik · OK Lesen · Zurück",
                RIGHT,
                680.0,
                13,
                Weight::Semi,
                GOLD,
                Align::Right,
            );
        } else {
            c.text(
                "↑↓ Lesen · Zurück",
                RIGHT,
                680.0,
                13,
                Weight::Semi,
                GOLD,
                Align::Right,
            );
            c.text(
                "Artikel",
                X,
                ROW_Y - 5.0,
                18,
                Weight::Semi,
                GOLD,
                Align::Left,
            );
        }
    }

    fn draw_focused_row(&self, c: &mut Canvas, y: f32) {
        // Two low-alpha fills make a warm halo without a hard outline.
        c.rect(
            X - 3.0,
            y - 3.0,
            ROW_W + 6.0,
            ROW_H + 6.0,
            18.0,
            Rgba(226, 187, 116, 13),
        );
        c.rect(X, y, ROW_W, ROW_H, 15.0, Rgba(76, 57, 25, 92));
    }

    fn draw_article_text(&mut self, c: &mut Canvas) {
        self.prepare_article(c);
        let Some(article) = self.article.as_mut() else {
            return;
        };
        let offset = article.scroll.step();
        c.clip_y(Some((ROW_Y, 650.0)));
        let mut y = ROW_Y + 22.0 - offset;
        if article.lines.is_empty() {
            let message = if article.body.get().is_some() {
                "Für diesen Artikel liegt kein Text vor."
            } else {
                "Artikel wird geladen …"
            };
            c.text(message, X, y, 18, Weight::Regular, MUTED, Align::Left);
        } else {
            for (heading, lines) in &article.lines {
                let (size, weight, line_height, color) = if *heading {
                    (22, Weight::Bold, 32.0, GOLD)
                } else {
                    (18, Weight::Regular, 27.0, CREAM)
                };
                for line in lines {
                    if y > ROW_Y - 36.0 && y < 655.0 {
                        c.text(line, X, y, size, weight, color, Align::Left);
                    }
                    y += line_height;
                }
                y += 14.0;
            }
        }
        c.clip_y(None);
    }
}

impl Screen for NewsScreen {
    fn name(&self) -> &'static str {
        "news"
    }

    fn animating(&self, _: &Ctx) -> bool {
        self.article
            .as_ref()
            .map(|article| article.scroll.moving())
            .unwrap_or(false)
    }

    fn key(&mut self, key: Key, cx: &mut Ctx) -> Nav {
        self.sync_items(cx);
        if let Some(article) = self.article.as_mut() {
            match key {
                Key::Back => {
                    self.article = None;
                    self.base_cache = LayerCache::default();
                    self.base_image = None;
                    self.base_key = None;
                    self.last_preview_key = None;
                }
                Key::Up | Key::ChUp => article.scroll.set((article.scroll.target - 220.0).max(0.0)),
                Key::Down | Key::ChDown => {
                    let max = (article.content_height - 450.0).max(0.0);
                    article.scroll.set((article.scroll.target + 220.0).min(max));
                }
                Key::Home => return Nav::Home,
                _ => {}
            }
            return Nav::None;
        }
        let indices = self.filtered_indices();
        match key {
            Key::Back => return Nav::Pop,
            Key::Home => return Nav::Home,
            Key::Menu => {
                cx.news = load(&cx.tx, crate::news::fetch);
                self.cache_checked = false;
            }
            Key::Up if self.selected > 0 => {
                self.selected -= 1;
                self.last_preview_key = None;
            }
            Key::Down if self.selected + 1 < indices.len() => {
                self.selected += 1;
                self.last_preview_key = None;
            }
            Key::Left if !self.category_codes.is_empty() => {
                self.category =
                    (self.category + self.category_codes.len() - 1) % self.category_codes.len();
                self.selected = 0;
                self.base_cache = LayerCache::default();
                self.base_image = None;
                self.base_key = None;
                self.last_preview_key = None;
                self.last_focus_row = None;
            }
            Key::Right if !self.category_codes.is_empty() => {
                self.category = (self.category + 1) % self.category_codes.len();
                self.selected = 0;
                self.base_cache = LayerCache::default();
                self.base_image = None;
                self.base_key = None;
                self.last_preview_key = None;
                self.last_focus_row = None;
            }
            Key::Ok => {
                if let Some(item) = self.preview_item(&indices).cloned() {
                    self.open_article(cx, item);
                }
            }
            _ => {}
        }
        Nav::None
    }

    fn draw(&mut self, c: &mut Canvas, cx: &mut Ctx) {
        self.sync_items(cx);
        let indices = self.filtered_indices();
        let is_article = self.article.is_some();
        let first_visible = if is_article {
            0
        } else {
            self.selected.saturating_sub(VISIBLE_ROWS - 1)
        };
        let key = (
            self.data_revision,
            self.category,
            first_visible,
            is_article,
            indices.len(),
        );
        let now = Instant::now();
        let first_frame = self.base_image.is_none();
        if self.ambient_until.is_none() {
            self.ambient_until = Some(now + Duration::from_millis(400));
        }
        // only while the backdrop really changes (it does not fade on the Pi)
        let ambient_moving = self.ambient_until.is_some_and(|until| now < until) && c.ambient_moving();
        let mut base_changed = false;
        let kept = (!ambient_moving && self.base_key != Some(key))
            .then(|| self.base_kept.iter().find(|(k, _)| *k == key).map(|(_, i)| i.clone()))
            .flatten();
        if let Some(img) = kept {
            c.restore_image(&img);
            self.base_image = Some(img);
            self.base_key = Some(key);
            self.base_cache = LayerCache::default();
            base_changed = true;
            self.last_focus_row = None;
            self.last_preview_key = None;
            self.last_article_key = None;
        } else if first_frame || self.base_key != Some(key) || ambient_moving {
            c.ambient(crate::gfx::Ambient::LiquidGold);
            self.draw_static(c, &indices);
            base_changed = c.finish_cached(&mut self.base_cache);
            if base_changed || self.base_image.is_none() {
                let img = c.snapshot();
                self.base_kept.retain(|(k, _)| *k != key);
                self.base_kept.push((key, img.clone()));
                if self.base_kept.len() > 4 {
                    self.base_kept.remove(0);
                }
                self.base_image = Some(img);
            }
            self.base_key = Some(key);
            if base_changed {
                self.last_focus_row = None;
                self.last_preview_key = None;
                self.last_article_key = None;
            }
        }
        let Some(base) = self.base_image.as_ref() else {
            return;
        };

        // The status clock is the only always-changing shell element.
        c.restore_rect(base, STATUS.0, STATUS.1, STATUS.2, STATUS.3);
        status_with_phone_accent(c, cx, GOLD);

        if self.article.is_none() {
            let selected_original = indices.get(self.selected).copied().unwrap_or(usize::MAX);
            let item = self.items.get(selected_original);
            let source = item.map(|i| i.image.as_str()).unwrap_or("");
            let image_revision = if source.is_empty() {
                0
            } else {
                cx.images.get_cover(source, PREVIEW_PX.0, PREVIEW_PX.1);
                cx.images.cover_source_revision(source, PREVIEW_PX.0, PREVIEW_PX.1)
            };
            let preview_key = (
                self.data_revision,
                self.category,
                selected_original,
                image_revision,
            );
            if base_changed || self.last_preview_key != Some(preview_key) {
                c.restore_rect(base, PREVIEW.0, PREVIEW.1, PREVIEW.2, PREVIEW.3);
                self.draw_preview(c, cx, item);
                self.last_preview_key = Some(preview_key);
            }

            let row_y = self.row_y();
            if base_changed || row_y != self.last_focus_row {
                if let Some(previous_y) = self.last_focus_row {
                    c.restore_rect(base, X - 4.0, previous_y - 4.0, ROW_W + 8.0, ROW_H + 8.0);
                }
                if let Some(y) = row_y {
                    c.restore_rect(base, X - 4.0, y - 4.0, ROW_W + 8.0, ROW_H + 8.0);
                    self.draw_focused_row(c, y);
                }
                self.last_focus_row = row_y;
            }
            self.last_article_key = None;
        } else {
            let article_item = self.article.as_ref().map(|article| article.item.clone());
            let image_revision = article_item
                .as_ref()
                .filter(|item| !item.image.is_empty())
                .map(|item| {
                    cx.images.get_cover(&item.image, PREVIEW_PX.0, PREVIEW_PX.1);
                    cx.images.cover_source_revision(&item.image, PREVIEW_PX.0, PREVIEW_PX.1)
                })
                .unwrap_or(0);
            let preview_key = (
                self.data_revision,
                self.category,
                usize::MAX,
                image_revision,
            );
            if base_changed || self.last_preview_key != Some(preview_key) {
                c.restore_rect(base, PREVIEW.0, PREVIEW.1, PREVIEW.2, PREVIEW.3);
                self.draw_preview(c, cx, article_item.as_ref());
                self.last_preview_key = Some(preview_key);
            }
            let article_revision = self.article.as_ref().unwrap().body.revision();
            let scroll_y = self.article.as_ref().unwrap().scroll.v.round() as i32;
            let key = (article_revision, scroll_y);
            if base_changed || self.last_article_key != Some(key) {
                c.restore_rect(base, X, ROW_Y, ROW_W, 500.0);
                self.draw_article_text(c);
                let new_scroll_y = self
                    .article
                    .as_ref()
                    .map(|article| article.scroll.v.round() as i32)
                    .unwrap_or(scroll_y);
                self.last_article_key = Some((article_revision, new_scroll_y));
            }
            self.last_focus_row = None;
            self.last_preview_key = None;
        }
        c.finish_pending_damage();
    }
}
