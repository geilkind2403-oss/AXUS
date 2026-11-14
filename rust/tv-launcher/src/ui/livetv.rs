//! Live TV: the picture full screen; ▲▼ zap (the stream switches once the
//! key rests), 0-9 channel number, OK opens the guide (groups | channels),
//! ▶ on a channel in the guide = favourite, Back closes the guide, then Live TV.
use super::*;
use crate::app::{Ctx, Nav, Screen};
use crate::gfx::{color, Align, Canvas, Rgba, Weight};
use crate::livetv::{groups, in_group, Channel, ARTE_LIVE_CONFIG_URL};
use crate::player::{Event, MediaSource, Player, StartOpts};
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant};

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
    list: ListView,
    group_list: ListView,
    no_signal: Option<String>,
    retried: bool,
    guide_in: Anim,
    banner_in: Anim,
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
            list: ListView::new(),
            group_list: ListView::new(),
            no_signal: None,
            retried: false,
            guide_in: Anim::default(),
            banner_in: Anim::default(),
        }
    }

    fn channels(&self, cx: &Ctx) -> Vec<Channel> {
        if self.standalone_app_id.is_some() {
            return self.current.iter().cloned().collect();
        }
        cx.channels
            .get()
            .and_then(|r| r.ok())
            .map(|x| x.0)
            .unwrap_or_default()
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
            list = all.clone();
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
            if let Some(ch) = self.channels(cx).into_iter().find(|c| c.number == n) {
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
        let all = self.channels(cx);
        if let Key::Digit(d) = k {
            self.digits.push((b'0' + d) as char);
            self.digits.truncate(4);
            self.digits_at = Instant::now();
            return Nav::None;
        }
        if self.guide {
            let gs = groups(&all);
            let gid = gs
                .get(self.group)
                .map(|g| g.0.clone())
                .unwrap_or_else(|| "all".into());
            let chs: Vec<Channel> = in_group(&all, &gid, &cx.cfg.livetv_favs)
                .into_iter()
                .cloned()
                .collect();
            match (k, self.col) {
                (Key::Back, _) | (Key::Menu, _) => self.guide = false,
                (Key::Left, 1) => self.col = 0,
                (Key::Right, 0) | (Key::Ok, 0) => {
                    self.col = 1;
                    self.list.sel = 0;
                }
                (Key::Up | Key::Down | Key::ChUp | Key::ChDown, 0) => {
                    if self.group_list.key(k, gs.len()) {
                        self.group = self.group_list.sel;
                        self.list.sel = 0;
                    }
                }
                (Key::Right | Key::Green | Key::Yellow, 1) => {
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
        let all = if self.standalone_app_id.is_some() {
            self.current.iter().cloned().collect::<Vec<_>>()
        } else {
            match cx.channels.get() {
                None => {
                    c.clear(color::BG1);
                    c.text(
                        "Senderliste wird geladen …",
                        w / 2.0,
                        h / 2.0 - 14.0,
                        24,
                        Weight::Semi,
                        color::TEXT,
                        Align::Center,
                    );
                    return;
                }
                Some(Err(e)) => {
                    c.backdrop();
                    c.text(
                        "Keine Senderliste",
                        PAD,
                        200.0,
                        34,
                        Weight::Bold,
                        color::TEXT,
                        Align::Left,
                    );
                    c.paragraph(&format!("{e}\n\nEine .m3u-Datei nach /storage/tv-launcher/livetv/ oder auf den USB-Stick legen – oder unter Einstellungen › Live TV eine Adresse eintragen."), PAD, 260.0, 1000.0, 19, Weight::Regular, color::MUTED, 28.0, 8);
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
    }
}

fn draw_guide(s: &mut LiveTv, c: &mut Canvas, cx: &mut Ctx, all: &[Channel]) {
    let (w, h) = (crate::gfx::W as f32, crate::gfx::H as f32);
    c.vgradient(
        0.0,
        0.0,
        900.0,
        h,
        0.0,
        Rgba(10, 10, 12, 245),
        Rgba(10, 10, 12, 245),
    );
    c.vgradient(
        900.0,
        0.0,
        w - 900.0,
        h,
        0.0,
        Rgba(10, 10, 12, 200),
        Rgba(10, 10, 12, 120),
    );
    c.text(
        "Live TV",
        PAD,
        36.0,
        30,
        Weight::Bold,
        color::TEXT,
        Align::Left,
    );
    let gs = groups(all);
    s.group = s.group.min(gs.len().saturating_sub(1));
    let favs = cx.cfg.livetv_favs.clone();
    let gsel = s.group;
    s.group_list.sel = gsel;
    s.group_list.draw(
        c,
        gs.len(),
        PAD - 12.0,
        100.0,
        260.0,
        560.0,
        50.0,
        s.col == 0,
        |c, i, x, y, f| {
            let (id, label) = &gs[i];
            let n = in_group(all, id, &favs).len();
            let col = if f {
                color::BLACK
            } else if i == gsel {
                color::TEXT
            } else {
                color::MUTED
            };
            let badge = match id.as_str() {
                "fav" => "★".to_string(),
                "all" => "TV".to_string(),
                other if other.len() == 2 => other.to_string(),
                _ => "•".to_string(),
            };
            c.text(
                &badge,
                x + 30.0,
                y + 13.0,
                15,
                Weight::Bold,
                if f { color::BLACK } else { color::ACCENT },
                Align::Center,
            );
            c.text_fit(
                label,
                x + 58.0,
                y + 12.0,
                18,
                if i == gsel {
                    Weight::Semi
                } else {
                    Weight::Regular
                },
                col,
                150.0,
            );
            c.text(
                &n.to_string(),
                x + 248.0,
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
    let gid = gs.get(s.group).map(|g| g.0.clone()).unwrap_or_default();
    let chs: Vec<Channel> = in_group(all, &gid, &favs).into_iter().cloned().collect();
    let x0 = PAD + 270.0;
    if chs.is_empty() {
        let msg = if gid == "fav" {
            "Noch keine Favoriten.\nIn einer Senderliste ▶ drücken, um einen Sender hier zu speichern."
        } else {
            "Keine Sender in dieser Liste."
        };
        c.paragraph(
            msg,
            x0 + 10.0,
            110.0,
            480.0,
            18,
            Weight::Regular,
            color::MUTED,
            26.0,
            4,
        );
        return;
    }
    let playing = s
        .current
        .as_ref()
        .map(|c| c.url.clone())
        .unwrap_or_default();
    let focus_channels = s.col == 1;
    s.list.draw(
        c,
        chs.len(),
        x0,
        100.0,
        520.0,
        560.0,
        64.0,
        focus_channels,
        |c, i, x, y, f| {
            let ch = &chs[i];
            let col = if f { color::BLACK } else { color::TEXT };
            c.text(
                &ch.number.to_string(),
                x + 40.0,
                y + 18.0,
                17,
                Weight::Semi,
                if f {
                    Rgba(60, 60, 60, 255)
                } else {
                    color::MUTED
                },
                Align::Right,
            );
            logo(c, cx, ch, x + 56.0, y + 7.0, 80.0, 44.0);
            c.text_fit(
                &ch.name,
                x + 152.0,
                y + 17.0,
                19,
                if f { Weight::Semi } else { Weight::Regular },
                col,
                290.0,
            );
            if ch.url == playing {
                c.rect(x + 450.0, y + 18.0, 48.0, 22.0, 6.0, color::RED);
                c.text(
                    "LIVE",
                    x + 474.0,
                    y + 21.0,
                    12,
                    Weight::Bold,
                    color::WHITE,
                    Align::Center,
                );
            } else if favs.contains(&ch.url) {
                c.text(
                    "★",
                    x + 474.0,
                    y + 16.0,
                    20,
                    Weight::Bold,
                    Rgba::hex(0xffc93c),
                    Align::Center,
                );
            }
        },
    );
    c.rect(0.0, h - 52.0, 900.0, 52.0, 0.0, Rgba(10, 10, 12, 255));
    c.text(
        "OK Einschalten · ▶ Favorit · ◀ Gruppen · Zurück schließen · 0–9 Sendernummer",
        PAD,
        h - 36.0,
        14,
        Weight::Regular,
        color::MUTED,
        Align::Left,
    );
}
