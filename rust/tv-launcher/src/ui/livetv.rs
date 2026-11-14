//! Live TV: the picture full screen; ▲▼ zap (the stream switches once the
//! key rests), 0-9 channel number, OK opens the guide (groups | channels),
//! ▶ on a channel in the guide = favourite, Back closes the guide, then Live TV.
use super::*;
use crate::app::{Ctx, Nav, Screen};
use crate::gfx::{color, Align, Canvas, Rgba, Weight};
use crate::livetv::{groups, in_group, Channel, ARTE_LIVE_CONFIG_URL};
use crate::player::{Event, MediaSource, Player, StartOpts};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// What the guide lists. Worked out when the channel list, the favourites,
/// the group or the search changes -- not on every frame: sorting a few
/// thousand channels into groups thirty times a second cost more than the
/// drawing, while the stream was playing underneath.
#[derive(Default)]
struct GuideData {
    /// (address, length) of the channel list this was made from
    list: (usize, usize),
    favs: Vec<String>,
    /// (id, label, number of channels)
    groups: Arc<Vec<(String, String, usize)>>,
    /// (group index, search) the channel rows were made for
    rows_for: Option<(usize, String)>,
    rows: Arc<Vec<Channel>>,
}

pub struct LiveTv {
    standalone_app_id: Option<&'static str>,
    player: Option<Player>,
    current: Option<Channel>,
    tuned_at: Instant,
    banner_until: Instant,
    zap: Option<(Channel, Instant)>,
    digits: String,
    digits_at: Instant,
    guide: bool,
    /// 0 groups, 1 channels
    col: usize,
    group: usize,
    query: String,
    keyboard: Option<Keyboard>,
    list: ListView,
    group_list: ListView,
    no_signal: Option<String>,
    retried: bool,
    guide_in: Anim,
    banner_in: Anim,
    guide_data: GuideData,
}

const BANNER: Duration = Duration::from_millis(4500);

impl LiveTv {
    pub fn new(cx: &mut Ctx) -> LiveTv {
        if !cx.channels.is_set() {
            cx.reload_channels();
        }
        Self::empty(None)
    }

    /// ARTE's public HLS stream, played by the native MPEG-TS pipeline.
    pub fn new_arte(cx: &mut Ctx) -> LiveTv {
        let mut live = Self::empty(Some("arte"));
        live.tune(
            Channel {
                number: 0,
                name: "ARTE Live".into(),
                group: "Streaming".into(),
                logo: String::new(),
                url: ARTE_LIVE_CONFIG_URL.into(),
            },
            cx,
        );
        live
    }

    pub fn new_zdf(cx: &mut Ctx) -> LiveTv {
        let mut live = Self::empty(Some("zdf"));
        live.tune(
            Channel {
                number: 0,
                name: "ZDF Live".into(),
                group: "Streaming".into(),
                logo: String::new(),
                url: crate::livetv::ZDF_LIVE_PAGE_URL.into(),
            },
            cx,
        );
        live
    }

    fn empty(standalone_app_id: Option<&'static str>) -> LiveTv {
        LiveTv {
            standalone_app_id,
            player: None,
            current: None,
            tuned_at: Instant::now(),
            banner_until: Instant::now(),
            zap: None,
            digits: String::new(),
            digits_at: Instant::now(),
            guide: false,
            col: 1,
            group: 1,
            query: String::new(),
            keyboard: None,
            list: ListView::new(),
            group_list: ListView::new(),
            no_signal: None,
            retried: false,
            guide_in: Anim::default(),
            banner_in: Anim::default(),
            guide_data: GuideData::default(),
        }
    }

    fn channels(&self, cx: &Ctx) -> Arc<Vec<Channel>> {
        if self.standalone_app_id.is_some() {
            return Arc::new(self.current.iter().cloned().collect());
        }
        cx.channels
            .with(|r| match r {
                Some(Ok((list, _))) => Some(list.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// The guide's groups and the channels of the chosen group that match
    /// the search (see `GuideData`). Also keeps `self.group` in range.
    fn guide_rows(
        &mut self,
        all: &Arc<Vec<Channel>>,
        favs: &[String],
    ) -> (Arc<Vec<(String, String, usize)>>, Arc<Vec<Channel>>) {
        let g = &mut self.guide_data;
        let list = (Arc::as_ptr(all) as usize, all.len());
        if g.list != list || g.favs != favs || g.groups.is_empty() {
            g.list = list;
            g.favs = favs.to_vec();
            g.groups = Arc::new(
                groups(all)
                    .into_iter()
                    .map(|(id, label)| {
                        let n = in_group(all, &id, favs).len();
                        (id, label, n)
                    })
                    .collect(),
            );
            g.rows_for = None;
        }
        self.group = self.group.min(g.groups.len().saturating_sub(1));
        if g.rows_for.as_ref().map(|r| (r.0, r.1.as_str())) != Some((self.group, &self.query)) {
            let gid = g
                .groups
                .get(self.group)
                .map(|x| x.0.as_str())
                .unwrap_or("all");
            let query = self.query.to_lowercase();
            g.rows = Arc::new(
                in_group(all, gid, favs)
                    .into_iter()
                    .filter(|ch| {
                        query.is_empty()
                            || ch.name.to_lowercase().contains(&query)
                            || ch.number.to_string().contains(&query)
                    })
                    .cloned()
                    .collect(),
            );
            g.rows_for = Some((self.group, self.query.clone()));
        }
        (g.groups.clone(), g.rows.clone())
    }

    fn tune(&mut self, ch: Channel, cx: &mut Ctx) {
        self.player = None; // stop the old stream first (decoder + sound are single)
        self.no_signal = None;
        self.retried = false;
        let opts = StartOpts {
            start: 0.0,
            audio: None,
            sub: Some(usize::MAX),
            audio_out: cx.cfg.audio_output.clone(),
            duration_hint: 0.0,
        };
        let source = match self.standalone_app_id {
            Some("zdf") => MediaSource::HlsSeparate {
                page_url: ch.url.clone(),
                max_h: cx.cfg.livetv_max_h,
            },
            _ => MediaSource::Hls {
                url: ch.url.clone(),
                max_h: cx.cfg.livetv_max_h,
            },
        };
        self.player = Some(Player::start(source, opts, cx.tx.clone(), cx.kms.clone()));
        if let Some(p) = &self.player {
            p.shared.sub_sel.store(-1, Relaxed);
        }
        self.tuned_at = Instant::now();
        self.banner_until = Instant::now() + BANNER;
        if self.standalone_app_id.is_none() {
            cx.cfg.livetv_last = ch.url.clone();
            cx.save();
        }
        self.current = Some(ch);
    }

    fn zap(&mut self, dir: i64, cx: &mut Ctx) {
        let all = self.channels(cx);
        if all.is_empty() {
            return;
        }
        // zap within the group the guide shows (favourites when chosen there)
        let gs = groups(&all);
        let gid = gs
            .get(self.group)
            .map(|g| g.0.clone())
            .unwrap_or_else(|| "all".into());
        let mut list: Vec<Channel> = in_group(&all, &gid, &cx.cfg.livetv_favs)
            .into_iter()
            .cloned()
            .collect();
        if list.is_empty() {
            list = all.to_vec();
        }
        let from = self
            .zap
            .as_ref()
            .map(|z| z.0.clone())
            .or(self.current.clone());
        let i = from
            .and_then(|f| list.iter().position(|c| c.url == f.url))
            .map(|i| i as i64)
            .unwrap_or(-1);
        let n = list.len() as i64;
        let next = list[((i + dir).rem_euclid(n)) as usize].clone();
        self.zap = Some((next, Instant::now()));
        self.banner_until = Instant::now() + BANNER;
    }

    fn overlay_visible(&self) -> bool {
        self.guide
            || self.guide_in.v > 0.01
            || self.banner_in.v > 0.01
            || Instant::now() < self.banner_until
            || self.zap.is_some()
            || !self.digits.is_empty()
            || self.no_signal.is_some()
            || self.waiting()
    }

    fn waiting(&self) -> bool {
        self.player
            .as_ref()
            .map(|p| p.shared.frames_shown.load(Relaxed) == 0)
            .unwrap_or(false)
    }
}

/// A channel logo on a light plate (many logos are dark-on-transparent), or the initials.
pub fn logo(c: &mut Canvas, cx: &mut Ctx, ch: &Channel, x: f32, y: f32, w: f32, h: f32) {
    c.rect(x, y, w, h, 8.0, Rgba(238, 238, 242, 255));
    if let Some(img) = cx.images.get(&ch.logo, (w * 2.0) as u32, (h * 2.0) as u32) {
        c.image_fit(&img, x + 4.0, y + 4.0, w - 8.0, h - 8.0, 0.0, false, 1.0);
    } else {
        let ini: String = ch
            .name
            .replace(" HD", "")
            .split_whitespace()
            .take(2)
            .filter_map(|w| w.chars().next())
            .collect::<String>()
            .to_uppercase();
        c.text(
            &ini,
            x + w / 2.0,
            y + h / 2.0 - 11.0,
            18,
            Weight::Bold,
            Rgba(40, 40, 48, 255),
            Align::Center,
        );
    }
}

impl Screen for LiveTv {
    fn name(&self) -> &'static str {
        "livetv"
    }
    fn home_focus(&self) -> Option<usize> {
        if self.standalone_app_id.is_some() {
            Some(5)
        } else {
            None
        }
    }
    fn app_id(&self) -> Option<&'static str> {
        self.standalone_app_id
    }
    fn video(&self) -> bool {
        true
    }
    fn overlay_empty(&self) -> bool {
        !self.overlay_visible()
    }
    fn animating(&self, _: &Ctx) -> bool {
        self.overlay_visible()
            || self.player.is_none()
            || self.guide_in.moving()
            || self.banner_in.moving()
            || self.keyboard.is_some()
    }
    fn motion_over_video(&self) -> bool {
        self.guide_in.moving()
            || self.banner_in.moving()
            || (self.guide && (self.list.moving() || self.group_list.moving()))
    }
    fn player_event(&mut self, e: &Event, cx: &mut Ctx) -> Nav {
        match e {
            Event::Error(m) => {
                crate::log(&format!("Live TV: Streamfehler: {m}"));
                self.player = None;
                self.no_signal = Some(m.clone());
            }
            Event::Ended => {
                // a live stream that ends hiccuped: try once more
                if !self.retried {
                    if let Some(ch) = self.current.clone() {
                        self.tune(ch, cx);
                        self.retried = true;
                        return Nav::None;
                    }
                }
                self.player = None;
                self.no_signal = Some("Der Sender hat aufgehört zu senden.".into());
            }
            Event::Ready(info) => {
                crate::log(&format!(
                    "Live TV bereit nach {} ms: Video {:?}, Audio-Spuren {}",
                    self.tuned_at.elapsed().as_millis(),
                    info.video,
                    info.audio.len()
                ));
            }
        }
        Nav::None
    }

    fn poll(&mut self, cx: &mut Ctx) -> Nav {
        // first start: the last channel (or the first)
        if self.current.is_none() && self.player.is_none() && self.no_signal.is_none() {
            let all = self.channels(cx);
            if !all.is_empty() {
                let last = cx.cfg.livetv_last.clone();
                let ch = all
                    .iter()
                    .find(|c| c.url == last)
                    .cloned()
                    .unwrap_or_else(|| all[0].clone());
                self.tune(ch, cx);
            }
        }
        if let Some((ch, at)) = &self.zap {
            if at.elapsed() > Duration::from_millis(400) {
                let ch = ch.clone();
                self.zap = None;
                if self
                    .current
                    .as_ref()
                    .map(|c| c.url != ch.url)
                    .unwrap_or(true)
                {
                    self.tune(ch, cx);
                }
            }
        }
        if !self.digits.is_empty() && self.digits_at.elapsed() > Duration::from_millis(1500) {
            let n: usize = self.digits.parse().unwrap_or(0);
            self.digits.clear();
            if let Some(ch) = self.channels(cx).iter().find(|c| c.number == n).cloned() {
                self.guide = false;
                self.tune(ch, cx);
            }
        }
        // a stream that shows nothing for 15 s is as good as dead
        if self.waiting() && self.tuned_at.elapsed() > Duration::from_secs(15) {
            self.player = None;
            self.no_signal = Some("Sender momentan nicht verfügbar".into());
        }
        Nav::None
    }

    fn key(&mut self, k: Key, cx: &mut Ctx) -> Nav {
        if let Some(kb) = self.keyboard.as_mut() {
            let result = kb.key(k);
            match result {
                KbResult::Done(q) => {
                    self.query = q.trim().to_string();
                    self.list.sel = 0;
                    self.keyboard = None;
                }
                KbResult::Cancel => self.keyboard = None,
                KbResult::Changed => {
                    self.query = kb.text.clone();
                    self.list.sel = 0;
                }
                KbResult::None => {}
            }
            return Nav::None;
        }
        let all = self.channels(cx);
        if all.is_empty() && self.standalone_app_id.is_none() {
            return match k {
                Key::Back | Key::Stop => Nav::Pop,
                Key::Home => Nav::Home,
                Key::Menu => Nav::Push(Box::new(super::settings::Settings::new(cx))),
                Key::Ok | Key::Info | Key::Play => {
                    cx.reload_channels();
                    cx.toast("Senderliste wird neu geladen");
                    Nav::None
                }
                _ => Nav::None,
            };
        }
        if let Key::Digit(d) = k {
            self.digits.push((b'0' + d) as char);
            self.digits.truncate(4);
            self.digits_at = Instant::now();
            return Nav::None;
        }
        if self.guide {
            let (gs, chs) = self.guide_rows(&all, &cx.cfg.livetv_favs);
            match (k, self.col) {
                (Key::Back, _) | (Key::Menu, _) => self.guide = false,
                (Key::Info, _) => {
                    self.keyboard = Some(Keyboard::new("Sender suchen", &self.query, false));
                }
                (Key::Char(ch), _) => {
                    let mut kb = Keyboard::new("Sender suchen", &self.query, false);
                    kb.text.push(ch);
                    self.keyboard = Some(kb);
                }
                (Key::Backspace, _) => {
                    self.query.pop();
                    self.list.sel = 0;
                }
                (Key::Red, _) => {
                    self.query.clear();
                    self.list.sel = 0;
                }
                (Key::Left, 1) => self.col = 0,
                (Key::Right, 0) | (Key::Ok, 0) => {
                    self.col = 1;
                    if let Some(current) = &self.current {
                        self.list.sel = chs.iter().position(|c| c.url == current.url).unwrap_or(0);
                    } else {
                        self.list.sel = 0;
                    }
                }
                (Key::Up | Key::Down | Key::ChUp | Key::ChDown, 0) => {
                    if self.group_list.key(k, gs.len()) {
                        self.group = self.group_list.sel;
                        self.list.sel = 0;
                    }
                }
                (Key::Play | Key::Green | Key::Yellow, 1) => {
                    if let Some(ch) = chs.get(self.list.sel) {
                        let favs = &mut cx.cfg.livetv_favs;
                        if let Some(i) = favs.iter().position(|u| *u == ch.url) {
                            favs.remove(i);
                            cx.toast(format!("{} aus den Favoriten entfernt", ch.name));
                        } else {
                            favs.push(ch.url.clone());
                            cx.toast(format!("★ {} ist jetzt ein Favorit", ch.name));
                        }
                        cx.save();
                    }
                }
                (Key::Ok, 1) => {
                    if let Some(ch) = chs.get(self.list.sel).cloned() {
                        self.guide = false;
                        self.tune(ch, cx);
                    }
                }
                (Key::Home, _) => {
                    self.player = None;
                    return Nav::Home;
                }
                (k, 1) => {
                    self.list.key(k, chs.len());
                }
                _ => {}
            }
            return Nav::None;
        }
        match k {
            Key::Back | Key::Stop => {
                if !self.digits.is_empty() {
                    self.digits.clear();
                    return Nav::None;
                }
                self.player = None;
                return Nav::Pop;
            }
            Key::Home => {
                self.player = None;
                return Nav::Home;
            }
            Key::Red => {
                self.guide = true;
                self.col = 1;
                self.group = 0;
                self.list.sel = 0;
            }
            Key::Green => {
                if let Some(ch) = self.current.clone() {
                    if let Some(i) = cx.cfg.livetv_favs.iter().position(|u| *u == ch.url) {
                        cx.cfg.livetv_favs.remove(i);
                        cx.toast(format!("{} aus den Favoriten entfernt", ch.name));
                    } else {
                        cx.cfg.livetv_favs.push(ch.url.clone());
                        cx.toast(format!("★ {} ist jetzt ein Favorit", ch.name));
                    }
                    cx.save();
                }
            }
            // like any TV: up = next channel number
            Key::Up | Key::ChUp | Key::Next => self.zap(1, cx),
            Key::Down | Key::ChDown | Key::Prev => self.zap(-1, cx),
            Key::Ok | Key::Menu | Key::Left | Key::Right => {
                self.guide = true;
                self.col = 1;
                // open on the playing channel
                let gs = groups(&all);
                let gid = gs
                    .get(self.group)
                    .map(|g| g.0.clone())
                    .unwrap_or_else(|| "all".into());
                let chs = in_group(&all, &gid, &cx.cfg.livetv_favs);
                if let Some(cur) = &self.current {
                    self.list.sel = chs.iter().position(|c| c.url == cur.url).unwrap_or(0);
                }
                self.group_list.sel = self.group;
            }
            Key::Info => self.banner_until = Instant::now() + BANNER,
            Key::PlayPause | Key::Pause | Key::Play => {
                if let Some(p) = &self.player {
                    p.toggle_pause();
                }
            }
            _ => {}
        }
        Nav::None
    }

    fn draw(&mut self, c: &mut Canvas, cx: &mut Ctx) {
        c.clear(Rgba(0, 0, 0, 0));
        let (w, h) = (crate::gfx::W as f32, crate::gfx::H as f32);
        let all: Arc<Vec<Channel>> = if self.standalone_app_id.is_some() {
            Arc::new(self.current.iter().cloned().collect())
        } else {
            // a cheap handle on the shared list, or the reason there is none
            match cx.channels.with(|r| match r {
                None => None,
                Some(Ok((list, _))) => Some(Ok((list.clone(), ()))),
                Some(Err(e)) => Some(Err(e.clone())),
            }) {
                None => {
                    c.clear(color::BG1);
                    c.rect(
                        w / 2.0 - 96.0,
                        118.0,
                        192.0,
                        118.0,
                        24.0,
                        Rgba::hex(0x23252c),
                    );
                    c.stroke_rect(
                        w / 2.0 - 96.0,
                        118.0,
                        192.0,
                        118.0,
                        24.0,
                        2.0,
                        color::BORDER,
                    );
                    c.stroke_rect(w / 2.0 - 64.0, 143.0, 128.0, 68.0, 12.0, 4.0, color::ACCENT);
                    c.line(
                        &[(w / 2.0 - 24.0, 216.0), (w / 2.0 + 24.0, 216.0)],
                        4.0,
                        color::ACCENT,
                    );
                    c.text(
                        "LIVE TV",
                        w / 2.0,
                        278.0,
                        14,
                        Weight::Bold,
                        color::ACCENT,
                        Align::Center,
                    );
                    c.text(
                        "Dein Fernsehprogramm wird vorbereitet",
                        w / 2.0,
                        318.0,
                        30,
                        Weight::Bold,
                        color::TEXT,
                        Align::Center,
                    );
                    c.text(
                        "Senderlisten werden geladen …",
                        w / 2.0,
                        366.0,
                        18,
                        Weight::Regular,
                        color::MUTED,
                        Align::Center,
                    );
                    return;
                }
                Some(Err(e)) => {
                    c.clear(color::BG1);
                    c.ambient(crate::gfx::Ambient::Color(Rgba::hex(0x302019)));
                    c.rect(64.0, 80.0, w - 128.0, h - 160.0, 28.0, Rgba::hex(0x1a1a20));
                    c.stroke_rect(64.0, 80.0, w - 128.0, h - 160.0, 28.0, 1.0, color::BORDER);
                    c.rect(112.0, 132.0, 56.0, 32.0, 12.0, Rgba::hex(0x382323));
                    c.text(
                        "LIVE TV",
                        140.0,
                        140.0,
                        12,
                        Weight::Bold,
                        Rgba::hex(0xff7777),
                        Align::Center,
                    );
                    c.text(
                        "Deine Senderliste fehlt noch",
                        112.0,
                        192.0,
                        32,
                        Weight::Bold,
                        color::TEXT,
                        Align::Left,
                    );
                    c.paragraph(&format!("{e}\n\nLege eine M3U-Senderliste auf einen USB-Stick oder nach /storage/tv-launcher/livetv/. Du kannst auch unter Einstellungen › Live TV eine Playlist-Adresse eintragen."), 112.0, 250.0, 740.0, 18, Weight::Regular, color::MUTED, 27.0, 7);
                    c.rect(112.0, 484.0, 320.0, 62.0, 16.0, color::ACCENT);
                    c.text(
                        "OK   ERNEUT LADEN",
                        272.0,
                        503.0,
                        17,
                        Weight::Bold,
                        color::BLACK,
                        Align::Center,
                    );
                    c.rect(452.0, 484.0, 390.0, 62.0, 16.0, Rgba::hex(0x292a31));
                    c.stroke_rect(452.0, 484.0, 390.0, 62.0, 16.0, 1.0, color::BORDER);
                    c.text(
                        "MENÜ   LIVE-TV-EINSTELLUNGEN",
                        647.0,
                        503.0,
                        15,
                        Weight::Semi,
                        color::TEXT,
                        Align::Center,
                    );
                    c.text(
                        "Sendernummern, Favoriten und Gruppen bleiben gespeichert.",
                        112.0,
                        579.0,
                        16,
                        Weight::Regular,
                        color::MUTED,
                        Align::Left,
                    );
                    return;
                }
                Some(Ok((ch, _))) => ch,
            }
        };
        // nothing on screen yet: dark backdrop with the spinner, not a frozen menu
        if self.waiting() || self.player.is_none() {
            c.clear(Rgba(0, 0, 0, 255));
        }
        if self.waiting() && self.no_signal.is_none() {
            let t = cx.t();
            for i in 0..8 {
                let a = t * 6.0 + i as f32 * std::f32::consts::PI / 4.0;
                c.circle(
                    w / 2.0 + a.cos() * 26.0,
                    h / 2.0 + a.sin() * 26.0,
                    5.0,
                    color::WHITE.alpha(0.25 + i as f32 / 10.0),
                );
            }
        }
        if let Some(msg) = &self.no_signal {
            c.rect(
                w / 2.0 - 330.0,
                h / 2.0 - 70.0,
                660.0,
                140.0,
                20.0,
                Rgba(20, 20, 24, 235),
            );
            c.text(
                msg,
                w / 2.0,
                h / 2.0 - 44.0,
                22,
                Weight::Bold,
                color::TEXT,
                Align::Center,
            );
            c.text(
                "▲ ▼ anderen Sender wählen · OK Senderliste",
                w / 2.0,
                h / 2.0 + 6.0,
                17,
                Weight::Regular,
                color::MUTED,
                Align::Center,
            );
        }
        // channel banner (zapping shows the target at once)
        let shown = self
            .zap
            .as_ref()
            .map(|z| z.0.clone())
            .or(self.current.clone());
        let want_banner = !self.guide && (Instant::now() < self.banner_until || self.zap.is_some());
        self.banner_in.set(if want_banner { 1.0 } else { 0.0 });
        let bk = self.banner_in.step();
        if bk > 0.01 {
            if let Some(ch) = shown {
                c.offset = (0.0, (1.0 - crate::gfx::ease(bk)) * 170.0);
                let (bx, by, bw, bh) = (PAD, h - 150.0, w - 2.0 * PAD, 104.0);
                c.vgradient(
                    0.0,
                    h - 220.0,
                    w,
                    220.0,
                    0.0,
                    Rgba(0, 0, 0, 0),
                    Rgba(0, 0, 0, 200),
                );
                c.rect(bx, by, bw, bh, 20.0, Rgba(22, 22, 26, 225));
                let channel_label = match self.standalone_app_id {
                    Some("zdf") => "ZDF".into(),
                    Some("arte") => "ARTE".into(),
                    _ => ch.number.to_string(),
                };
                c.text(
                    &channel_label,
                    bx + 50.0,
                    by + 30.0,
                    27,
                    Weight::Bold,
                    color::TEXT,
                    Align::Center,
                );
                logo(c, cx, &ch, bx + 100.0, by + 16.0, 128.0, 72.0);
                c.text_fit(
                    &ch.name,
                    bx + 250.0,
                    by + 22.0,
                    28,
                    Weight::Bold,
                    color::TEXT,
                    bw - 480.0,
                );
                let fav = if cx.cfg.livetv_favs.contains(&ch.url) {
                    "★ Favorit · "
                } else {
                    ""
                };
                c.text(
                    &format!("{fav}{}", crate::livetv::country_name(&ch.group)),
                    bx + 250.0,
                    by + 62.0,
                    16,
                    Weight::Regular,
                    color::MUTED,
                    Align::Left,
                );
                let (hh, mm, _) = crate::app::local_hms();
                c.text(
                    &format!("{hh:02}:{mm:02}"),
                    bx + bw - 24.0,
                    by + 22.0,
                    28,
                    Weight::Semi,
                    color::TEXT,
                    Align::Right,
                );
                c.text(
                    "LIVE",
                    bx + bw - 24.0,
                    by + 62.0,
                    14,
                    Weight::Bold,
                    color::RED,
                    Align::Right,
                );
                c.offset = (0.0, 0.0);
            }
        }
        // number entry
        if !self.digits.is_empty() {
            let n: usize = self.digits.parse().unwrap_or(0);
            let name = all
                .iter()
                .find(|c| c.number == n)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "–".into());
            c.rect(
                w - PAD - 300.0,
                40.0,
                300.0,
                110.0,
                20.0,
                Rgba(22, 22, 26, 230),
            );
            c.text(
                &self.digits,
                w - PAD - 150.0,
                52.0,
                44,
                Weight::Bold,
                color::TEXT,
                Align::Center,
            );
            c.text_fit(
                &name,
                w - PAD - 280.0,
                112.0,
                17,
                Weight::Regular,
                color::MUTED,
                260.0,
            );
        }
        self.guide_in.set(if self.guide { 1.0 } else { 0.0 });
        let gk = self.guide_in.step();
        if gk > 0.01 {
            let e = crate::gfx::ease(gk);
            c.offset = (-(1.0 - e) * 260.0, 0.0);
            c.opacity = e;
            draw_guide(self, c, cx, &all);
            c.offset = (0.0, 0.0);
            c.opacity = 1.0;
        }
        if let Some(kb) = self.keyboard.as_ref() {
            c.rect(0.0, 0.0, w, h, 0.0, Rgba(0, 0, 0, 185));
            c.rect(132.0, 40.0, w - 264.0, h - 80.0, 28.0, Rgba::hex(0x1b1c22));
            c.stroke_rect(132.0, 40.0, w - 264.0, h - 80.0, 28.0, 1.0, color::BORDER);
            c.text(
                "Sender suchen",
                178.0,
                72.0,
                28,
                Weight::Bold,
                color::TEXT,
                Align::Left,
            );
            kb.draw(c, 178.0, 124.0, w - 356.0, cx.t());
        }
    }
}

fn draw_guide(s: &mut LiveTv, c: &mut Canvas, cx: &mut Ctx, all: &Arc<Vec<Channel>>) {
    let (w, h) = (crate::gfx::W as f32, crate::gfx::H as f32);
    let rail_w = 910.0;
    c.rect(0.0, 0.0, rail_w, h, 0.0, Rgba::hex(0x101116));
    c.rect(rail_w, 0.0, w - rail_w, h, 0.0, Rgba::hex(0x191a20));
    c.line(&[(rail_w, 0.0), (rail_w, h)], 1.0, color::BORDER.alpha(0.8));

    c.text(
        "Live TV",
        PAD,
        32.0,
        30,
        Weight::Bold,
        color::TEXT,
        Align::Left,
    );
    c.text(
        &format!("{} Sender · dein Programm", all.len()),
        PAD,
        70.0,
        15,
        Weight::Regular,
        color::MUTED,
        Align::Left,
    );

    // Search is always visible and opens the remote friendly keyboard with INFO.
    c.rect(PAD, 98.0, 828.0, 54.0, 16.0, Rgba::hex(0x24252c));
    c.stroke_rect(PAD, 98.0, 828.0, 54.0, 16.0, 1.0, color::BORDER);
    c.stroke_rect(PAD + 18.0, 116.0, 15.0, 15.0, 8.0, 1.8, color::MUTED);
    c.line(
        &[(PAD + 30.0, 129.0), (PAD + 36.0, 135.0)],
        2.0,
        color::MUTED,
    );
    c.text_fit(
        if s.query.is_empty() {
            "Sender suchen …"
        } else {
            &s.query
        },
        PAD + 52.0,
        115.0,
        17,
        if s.query.is_empty() {
            Weight::Regular
        } else {
            Weight::Semi
        },
        if s.query.is_empty() {
            color::MUTED
        } else {
            color::TEXT
        },
        610.0,
    );
    c.text(
        "INFO SUCHEN",
        PAD + 804.0,
        117.0,
        11,
        Weight::Bold,
        color::MUTED,
        Align::Right,
    );

    let (gs, chs) = s.guide_rows(all, &cx.cfg.livetv_favs);
    let favs = s.guide_data.favs.clone();
    let gid = gs
        .get(s.group)
        .map(|g| g.0.clone())
        .unwrap_or_else(|| "all".into());
    let group_x = PAD - 8.0;
    let group_w = 244.0;
    let list_x = 326.0;
    let list_w = 566.0;
    c.text(
        "KATEGORIEN",
        group_x + 10.0,
        170.0,
        12,
        Weight::Bold,
        color::MUTED,
        Align::Left,
    );
    c.text(
        &format!("{} SENDER", chs.len()),
        list_x + 8.0,
        170.0,
        12,
        Weight::Bold,
        color::MUTED,
        Align::Left,
    );

    s.group_list.sel = s.group;
    s.group_list.draw(
        c,
        gs.len(),
        group_x,
        192.0,
        group_w,
        448.0,
        52.0,
        s.col == 0,
        |c, i, x, y, focused| {
            let (id, label, count) = &gs[i];
            let fg = if focused {
                color::BLACK
            } else if i == s.group {
                color::TEXT
            } else {
                color::MUTED
            };
            let badge = match id.as_str() {
                "fav" => "★",
                "all" => "TV",
                other if other.len() == 2 => other,
                _ => "•",
            };
            c.text(
                badge,
                x + 24.0,
                y + 15.0,
                15,
                Weight::Bold,
                if focused { color::BLACK } else { color::ACCENT },
                Align::Center,
            );
            c.text_fit(label, x + 48.0, y + 14.0, 16, Weight::Semi, fg, 145.0);
            c.text(
                &count.to_string(),
                x + group_w - 12.0,
                y + 16.0,
                13,
                Weight::Regular,
                if focused {
                    Rgba(60, 60, 60, 255)
                } else {
                    color::MUTED
                },
                Align::Right,
            );
        },
    );

    if chs.is_empty() {
        let message = if !s.query.is_empty() {
            "Keine Treffer für deine Suche.\nINFO öffnet die Suche · ROT löscht den Filter."
        } else if gid == "fav" {
            "Noch keine Favoriten.\nWähle einen Sender und drücke GRÜN oder ▶."
        } else {
            "In dieser Kategorie sind keine Sender."
        };
        c.rect(list_x, 192.0, list_w, 448.0, 20.0, Rgba::hex(0x1b1c22));
        c.paragraph(
            message,
            list_x + 32.0,
            230.0,
            list_w - 64.0,
            18,
            Weight::Regular,
            color::MUTED,
            28.0,
            5,
        );
    } else {
        let playing = s.current.as_ref().map(|ch| ch.url.as_str()).unwrap_or("");
        let playing_ready = s
            .player
            .as_ref()
            .map(|p| p.shared.frames_shown.load(Relaxed) > 0)
            .unwrap_or(false);
        let stream_failed = s.no_signal.is_some();
        s.list.draw(
            c,
            chs.len(),
            list_x,
            192.0,
            list_w,
            448.0,
            64.0,
            s.col == 1,
            |c, i, x, y, focused| {
                let ch = &chs[i];
                let fg = if focused { color::BLACK } else { color::TEXT };
                c.text(
                    &format!("{:02}", ch.number),
                    x + 36.0,
                    y + 20.0,
                    15,
                    Weight::Semi,
                    if focused {
                        Rgba(65, 65, 65, 255)
                    } else {
                        color::MUTED
                    },
                    Align::Right,
                );
                logo(c, cx, ch, x + 52.0, y + 8.0, 76.0, 44.0);
                c.text_fit(
                    &ch.name,
                    x + 144.0,
                    y + 19.0,
                    18,
                    if focused {
                        Weight::Semi
                    } else {
                        Weight::Regular
                    },
                    fg,
                    310.0,
                );
                if ch.url == playing {
                    let (label, tint) = if playing_ready {
                        ("LIVE", color::RED)
                    } else if stream_failed {
                        ("FEHLER", color::RED)
                    } else {
                        ("LÄDT", Rgba::hex(0xc88d36))
                    };
                    c.rect(x + list_w - 92.0, y + 18.0, 68.0, 25.0, 12.0, tint);
                    c.text(
                        label,
                        x + list_w - 58.0,
                        y + 24.0,
                        11,
                        Weight::Bold,
                        color::WHITE,
                        Align::Center,
                    );
                } else if favs.contains(&ch.url) {
                    c.text(
                        "★",
                        x + list_w - 58.0,
                        y + 17.0,
                        20,
                        Weight::Bold,
                        Rgba::hex(0xffc93c),
                        Align::Center,
                    );
                }
            },
        );
    }

    // Details pane keeps the selected station and its actions in view while browsing.
    c.text(
        "SENDERINFO",
        950.0,
        38.0,
        12,
        Weight::Bold,
        color::MUTED,
        Align::Left,
    );
    if let Some(ch) = chs.get(s.list.sel) {
        let fav = favs.contains(&ch.url);
        c.rect(950.0, 78.0, 284.0, 160.0, 18.0, Rgba::hex(0x24252c));
        logo(c, cx, ch, 962.0, 88.0, 260.0, 140.0);
        c.text_fit(&ch.name, 950.0, 266.0, 24, Weight::Bold, color::TEXT, 310.0);
        c.text(
            &format!(
                "KANAL {:02}  ·  {}",
                ch.number,
                crate::livetv::country_name(&ch.group)
            ),
            950.0,
            305.0,
            14,
            Weight::Regular,
            color::MUTED,
            Align::Left,
        );
        c.line(&[(950.0, 338.0), (1236.0, 338.0)], 1.0, color::BORDER);
        let is_tuned = s.current.as_ref().map(|p| p.url == ch.url).unwrap_or(false);
        let is_playing = is_tuned
            && s.player
                .as_ref()
                .map(|p| p.shared.frames_shown.load(Relaxed) > 0)
                .unwrap_or(false);
        let is_connecting = is_tuned && !is_playing && s.no_signal.is_none();
        c.circle(
            960.0,
            374.0,
            4.0,
            if is_playing {
                Rgba::hex(0x59d39a)
            } else if is_connecting {
                Rgba::hex(0xc88d36)
            } else if s.no_signal.is_some() && is_tuned {
                color::RED
            } else {
                color::ACCENT
            },
        );
        c.text(
            if is_playing {
                "LÄUFT GERADE"
            } else if is_connecting {
                "VERBINDET …"
            } else if s.no_signal.is_some() && is_tuned {
                "SIGNAL FEHLT"
            } else {
                "LIVE SENDER"
            },
            974.0,
            365.0,
            12,
            Weight::Bold,
            if is_playing {
                Rgba::hex(0x59d39a)
            } else if is_connecting {
                Rgba::hex(0xc88d36)
            } else if s.no_signal.is_some() && is_tuned {
                color::RED
            } else {
                color::ACCENT
            },
            Align::Left,
        );
        c.text(
            if fav {
                "★ In deinen Favoriten"
            } else {
                "☆ Noch kein Favorit"
            },
            950.0,
            405.0,
            16,
            Weight::Semi,
            if fav {
                Rgba::hex(0xffc93c)
            } else {
                color::MUTED
            },
            Align::Left,
        );
        c.rect(950.0, 454.0, 284.0, 48.0, 14.0, color::ACCENT);
        c.text(
            if is_playing {
                "OK  ZUM PROGRAMM"
            } else {
                "OK  SENDER EINSCHALTEN"
            },
            1092.0,
            468.0,
            14,
            Weight::Bold,
            color::BLACK,
            Align::Center,
        );
        c.rect(950.0, 514.0, 284.0, 48.0, 14.0, Rgba::hex(0x292a31));
        c.stroke_rect(950.0, 514.0, 284.0, 48.0, 14.0, 1.0, color::BORDER);
        c.text(
            if fav {
                "GRÜN  FAVORIT ENTFERNEN"
            } else {
                "GRÜN  ALS FAVORIT SPEICHERN"
            },
            1092.0,
            528.0,
            12,
            Weight::Semi,
            color::TEXT,
            Align::Center,
        );
    } else {
        c.text(
            "Wähle links einen Sender aus.",
            950.0,
            102.0,
            18,
            Weight::Semi,
            color::TEXT,
            Align::Left,
        );
        c.paragraph("Mit INFO kannst du die Liste durchsuchen. Favoriten werden auf diesem Gerät gespeichert.", 950.0, 144.0, 278.0, 16, Weight::Regular, color::MUTED, 24.0, 5);
    }
    c.line(&[(950.0, 590.0), (1236.0, 590.0)], 1.0, color::BORDER);
    c.text(
        "PFEILE  NAVIGIEREN",
        950.0,
        614.0,
        11,
        Weight::Bold,
        color::MUTED,
        Align::Left,
    );
    c.text(
        "CH+/−  SEITENWEISE",
        950.0,
        638.0,
        11,
        Weight::Bold,
        color::MUTED,
        Align::Left,
    );

    c.rect(0.0, h - 48.0, rail_w, 48.0, 0.0, Rgba::hex(0x101116));
    c.text(
        "OK EINSCHALTEN  ·  GRÜN FAVORIT  ·  INFO SUCHE  ·  ZURÜCK SCHLIESSEN",
        PAD,
        h - 31.0,
        12,
        Weight::Semi,
        color::MUTED,
        Align::Left,
    );
}
