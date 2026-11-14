//! Watching: the picture is on its own hardware plane; this screen only
//! draws the OSD (title, progress, time) and subtitles on top -- and nothing
//! at all while you just watch.
use super::*;
use crate::app::{Ctx, Nav, Screen};
use crate::films::{Film, Origin};
use crate::gfx::{color, Align, Canvas, Rgba, Weight};
use crate::player::{Event, MediaSource, Player, StartOpts};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::time::Instant;

enum Saver {
    Local(String),
    Server { base: String, id: String },
    None,
}

pub struct PlayerScreen {
    player: Option<Player>,
    title: String,
    subtitle: String,
    osd_until: Instant,
    seek_target: Option<(f64, Instant)>,
    saver: Saver,
    last_save: Instant,
    error: Option<String>,
    menu: Option<usize>,
    ready: bool,
}

impl PlayerScreen {
    fn new(
        cx: &mut Ctx,
        src: MediaSource,
        opts: StartOpts,
        title: String,
        subtitle: String,
        saver: Saver,
    ) -> PlayerScreen {
        let player = Player::start(src, opts, cx.tx.clone(), cx.kms.clone());
        PlayerScreen {
            player: Some(player),
            title,
            subtitle,
            osd_until: Instant::now() + std::time::Duration::from_secs(5),
            seek_target: None,
            saver,
            last_save: Instant::now(),
            error: None,
            menu: None,
            ready: false,
        }
    }

    pub fn film(cx: &mut Ctx, f: &Film, start: f64, audio: usize, sub: i64) -> PlayerScreen {
        if cx.cfg.last_film_key != f.key {
            cx.cfg.last_film_key = f.key.clone();
            cx.save();
        }
        let opts = |a: Option<usize>, s: Option<usize>| StartOpts {
            start,
            audio: a,
            sub: s,
            audio_out: cx.cfg.audio_output.clone(),
            duration_hint: f.duration,
        };
        match &f.origin {
            Origin::Local(p) => {
                let o = opts(None, None);
                PlayerScreen::new(
                    cx,
                    MediaSource::File(p.clone()),
                    o,
                    f.title.clone(),
                    String::new(),
                    Saver::Local(f.key.clone()),
                )
            }
            Origin::Server {
                base,
                id,
                stream,
                convert,
                codec,
                container_ok,
                sub_url,
                ..
            } => {
                // H.264 in MP4: the Pi plays the file itself (instant seeking).
                // Anything else: the PC converts while it plays.
                let direct = codec.as_deref() == Some("h264") && *container_ok;
                let src = match convert {
                    Some(conv) if !direct => {
                        let conv = conv.clone();
                        let (b, i) = (base.clone(), id.clone());
                        MediaSource::Live {
                            url: Arc::new(move |t, a| {
                                let sep = if conv.contains('?') { '&' } else { '?' };
                                format!("{conv}{sep}t={t:.3}&a={a}&v=h264")
                            }),
                            keyframe: Some(Arc::new(move |t| {
                                crate::net::get_json(&format!("{b}/seek/{i}?t={t:.3}"))
                                    .ok()
                                    .and_then(|j| j["t"].as_f64())
                                    .unwrap_or(t)
                            })),
                        }
                    }
                    _ => MediaSource::Http(stream.clone()),
                };
                let o = opts((audio != usize::MAX).then_some(audio), None);
                let mut ps = PlayerScreen::new(
                    cx,
                    src,
                    o,
                    f.title.clone(),
                    String::new(),
                    Saver::Server {
                        base: base.clone(),
                        id: id.clone(),
                    },
                );
                ps.load_server_subs(sub_url.clone(), sub);
                ps
            }
        }
    }

    pub fn youtube(
        cx: &mut Ctx,
        v: &crate::youtube::Video,
        s: crate::youtube::Stream,
    ) -> PlayerScreen {
        let src = if !s.video.is_empty() {
            MediaSource::Dash {
                video: s.video,
                audio: s.audio,
            }
        } else {
            MediaSource::Http(s.muxed.unwrap_or_default())
        };
        let opts = StartOpts {
            start: 0.0,
            audio: None,
            sub: None,
            audio_out: cx.cfg.audio_output.clone(),
            duration_hint: s.duration,
        };
        PlayerScreen::new(
            cx,
            src,
            opts,
            v.title.clone(),
            v.channel.clone(),
            Saver::None,
        )
    }

    /// Server films: subtitles come as WebVTT from the PC.
    fn load_server_subs(&mut self, url: Option<String>, n: i64) {
        let (Some(url), Some(p)) = (url, self.player.as_ref()) else {
            return;
        };
        if n < 0 {
            p.shared.sub_sel.store(-1, Relaxed);
            return;
        }
        p.shared.sub_sel.store(n, Relaxed);
        let sh = p.shared.clone();
        let url = url.replace("{n}", &n.to_string());
        std::thread::spawn(move || {
            if let Ok(b) = crate::net::get_bytes(&url, 8 << 20) {
                let cues = tvcore::tracks::parse_vtt(&String::from_utf8_lossy(&b));
                *sh.external_subs.lock().unwrap() = cues
                    .into_iter()
                    .map(|(s, e, t)| crate::player::engine::Cue {
                        start: s,
                        end: e,
                        text: t,
                    })
                    .collect();
            }
        });
    }

    fn save(&mut self, cx: &mut Ctx, finished: bool) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let pos = p.position();
        let d = p.duration();
        let pos = if finished || (d > 0.0 && pos > d - 90.0) {
            0.0
        } else {
            pos
        };
        if pos < 0.5 && !finished && !self.ready {
            return;
        }
        match &self.saver {
            Saver::Local(k) => {
                if pos > 30.0 {
                    cx.cfg.positions.insert(k.clone(), pos)
                } else {
                    cx.cfg.positions.remove(k)
                };
                cx.save();
            }
            Saver::Server { base, id } => crate::films::report_progress(base, id, pos),
            Saver::None => {}
        }
        self.last_save = Instant::now();
    }

    fn show_osd(&mut self) {
        self.osd_until = Instant::now() + std::time::Duration::from_secs(4);
    }

    fn close(&mut self, cx: &mut Ctx) -> Nav {
        self.save(cx, false);
        self.player = None; // stops the threads, frees decoder + sound
        Nav::Pop
    }

    fn seek_by(&mut self, d: f64) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let base = self
            .seek_target
            .map(|s| s.0)
            .unwrap_or_else(|| p.position());
        let dur = p.duration();
        let t = if dur > 0.0 {
            (base + d).clamp(0.0, dur - 1.0)
        } else {
            (base + d).max(0.0)
        };
        self.seek_target = Some((t, Instant::now()));
        self.show_osd();
    }
}

impl Screen for PlayerScreen {
    fn name(&self) -> &'static str {
        "player"
    }
    fn video(&self) -> bool {
        self.error.is_none()
    }
    fn overlay_empty(&self) -> bool {
        let Some(p) = self.player.as_ref() else {
            return true;
        };
        Instant::now() > self.osd_until
            && self.menu.is_none()
            && self.seek_target.is_none()
            && !p.shared.paused.load(Relaxed)
            && !p.shared.buffering.load(Relaxed)
            && p.shared.subtitle_now().is_none()
    }
    fn animating(&self, _: &Ctx) -> bool {
        // OSD, spinner, subtitles: frames needed. Just watching: the launcher sleeps
        // (a check once a second catches the next subtitle).
        // with subtitles on, keep looking a few times a second so a line
        // appears on time (unchanged frames are skipped, so this is cheap)
        let subs_on = self
            .player
            .as_ref()
            .map(|p| p.shared.sub_sel.load(Relaxed) >= 0)
            .unwrap_or(false);
        self.error.is_none() && (!self.overlay_empty() || subs_on)
    }
    fn player_event(&mut self, e: &Event, cx: &mut Ctx) -> Nav {
        match e {
            Event::Ready(_) => {
                self.ready = true;
                Nav::None
            }
            Event::Ended => {
                self.save(cx, true);
                self.player = None;
                Nav::Pop
            }
            Event::Error(msg) => {
                crate::log(&format!("Wiedergabe: {msg}"));
                self.error = Some(msg.clone());
                self.player = None;
                Nav::None
            }
        }
    }
    fn key(&mut self, k: Key, cx: &mut Ctx) -> Nav {
        if self.error.is_some() {
            return if matches!(k, Key::Back | Key::Ok | Key::Home | Key::Stop) {
                Nav::Pop
            } else {
                Nav::None
            };
        }
        if let Some(m) = self.menu {
            let info = self
                .player
                .as_ref()
                .map(|p| p.shared.info.lock().unwrap().clone())
                .unwrap_or_default();
            match k {
                Key::Back | Key::Down if m == 1 || k == Key::Back => self.menu = None,
                Key::Up => self.menu = Some(0),
                Key::Down => self.menu = Some(1),
                Key::Ok | Key::Left | Key::Right => {
                    let p = self.player.as_ref().unwrap();
                    if m == 0 && info.audio.len() > 1 {
                        let cur = p.shared.audio_sel.load(Relaxed);
                        p.set_audio((cur + 1) % info.audio.len());
                    } else if m == 1 && !info.subs.is_empty() {
                        let cur = p.shared.sub_sel.load(Relaxed);
                        let next = if cur + 1 >= info.subs.len() as i64 {
                            -1
                        } else {
                            cur + 1
                        };
                        p.shared.sub_sel.store(next, Relaxed);
                        p.shared.subs.lock().unwrap().clear();
                    }
                }
                _ => {}
            }
            return Nav::None;
        }
        match k {
            Key::Back | Key::Stop => return self.close(cx),
            Key::Home => {
                self.close(cx);
                return Nav::Home;
            }
            Key::Ok | Key::PlayPause | Key::Play | Key::Pause => {
                if let Some(p) = self.player.as_ref() {
                    match k {
                        Key::Play => p.set_paused(false),
                        Key::Pause => p.set_paused(true),
                        _ => p.toggle_pause(),
                    }
                    if p.shared.paused.load(Relaxed) {
                        self.save(cx, false);
                    }
                }
                self.show_osd();
            }
            Key::Left => self.seek_by(-10.0),
            Key::Right => self.seek_by(30.0),
            Key::Rewind | Key::Prev => self.seek_by(-60.0),
            Key::Forward | Key::Next => self.seek_by(120.0),
            Key::Down | Key::Menu | Key::Info => {
                self.menu = Some(0);
                self.show_osd();
            }
            Key::Digit(d) => {
                // 0-9: jump to that tenth of the film
                if let Some(p) = self.player.as_ref() {
                    let t = p.duration() * d as f64 / 10.0;
                    self.seek_target =
                        Some((t, Instant::now() - std::time::Duration::from_secs(1)));
                }
                self.show_osd();
            }
            _ => self.show_osd(),
        }
        Nav::None
    }

    fn draw(&mut self, c: &mut Canvas, cx: &mut Ctx) {
        // apply a seek once the remote is quiet for a moment
        if let Some((t, at)) = self.seek_target {
            if at.elapsed().as_millis() > 650 {
                if let Some(p) = self.player.as_ref() {
                    p.seek(t);
                }
                self.seek_target = None;
            }
        }
        if self.last_save.elapsed().as_secs() > 15 && self.ready {
            self.save(cx, false);
        }
        if let Some(e) = &self.error {
            c.backdrop();
            c.text(
                "Abspielen ging nicht",
                PAD,
                220.0,
                34,
                Weight::Bold,
                color::TEXT,
                Align::Left,
            );
            c.paragraph(
                e,
                PAD,
                280.0,
                1000.0,
                20,
                Weight::Regular,
                color::MUTED,
                30.0,
                6,
            );
            c.text(
                "OK / Zurück",
                PAD,
                460.0,
                18,
                Weight::Semi,
                color::ACCENT,
                Align::Left,
            );
            return;
        }
        c.clear(Rgba(0, 0, 0, 0));
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let sh = p.shared.clone();
        let (w, h) = (crate::gfx::W as f32, crate::gfx::H as f32);
        // subtitles
        if let Some(s) = sh.subtitle_now() {
            let px = [24u32, 30, 38][cx.cfg.sub_size.min(2) as usize];
            let lh = px as f32 * 1.33;
            let lines: Vec<String> = s
                .lines()
                .flat_map(|l| c.wrap(l, px, Weight::Semi, 1000.0))
                .collect();
            let n = lines.len() as f32;
            let base = if Instant::now() < self.osd_until {
                h - 190.0
            } else {
                h - 60.0
            };
            for (i, l) in lines.iter().enumerate() {
                let y = base - (n - i as f32) * lh;
                let tw = c.fonts.width(l, px, Weight::Semi);
                c.rect(
                    w / 2.0 - tw / 2.0 - 12.0,
                    y - 2.0,
                    tw + 24.0,
                    lh,
                    6.0,
                    Rgba(0, 0, 0, 150),
                );
                c.text(
                    l,
                    w / 2.0,
                    y + 1.0,
                    px,
                    Weight::Semi,
                    color::WHITE,
                    Align::Center,
                );
            }
        }
        let buffering = sh.buffering.load(Relaxed);
        if buffering {
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
        let paused = sh.paused.load(Relaxed);
        if Instant::now() > self.osd_until
            && !paused
            && self.menu.is_none()
            && self.seek_target.is_none()
        {
            return;
        }
        // OSD
        c.vgradient(
            0.0,
            0.0,
            w,
            130.0,
            0.0,
            Rgba(0, 0, 0, 190),
            Rgba(0, 0, 0, 0),
        );
        c.vgradient(
            0.0,
            h - 200.0,
            w,
            200.0,
            0.0,
            Rgba(0, 0, 0, 0),
            Rgba(0, 0, 0, 220),
        );
        c.text_fit(
            &self.title,
            PAD,
            34.0,
            28,
            Weight::Bold,
            color::WHITE,
            900.0,
        );
        if !self.subtitle.is_empty() {
            c.text_fit(
                &self.subtitle,
                PAD,
                72.0,
                17,
                Weight::Regular,
                color::MUTED,
                900.0,
            );
        }
        let (hh, mm, _) = crate::app::local_hms();
        c.text(
            &format!("{hh:02}:{mm:02}"),
            w - PAD,
            34.0,
            26,
            Weight::Semi,
            color::WHITE,
            Align::Right,
        );
        let pos = self
            .seek_target
            .map(|s| s.0)
            .unwrap_or_else(|| p.position());
        let dur = p.duration();
        let y = h - 86.0;
        if paused {
            icon_pause(c, PAD + 14.0, y - 26.0, 26.0, color::WHITE);
        } else {
            icon_play(c, PAD + 14.0, y - 26.0, 26.0, color::WHITE);
        }
        c.text(
            &crate::app::fmt_time(pos),
            PAD + 44.0,
            y - 40.0,
            20,
            Weight::Semi,
            color::WHITE,
            Align::Left,
        );
        if dur > 0.0 {
            c.text(
                &format!("-{}", crate::app::fmt_time(dur - pos)),
                w - PAD,
                y - 40.0,
                20,
                Weight::Semi,
                color::MUTED,
                Align::Right,
            );
            let f = (pos / dur).clamp(0.0, 1.0) as f32;
            c.rect(PAD, y, w - 2.0 * PAD, 6.0, 3.0, Rgba(255, 255, 255, 60));
            c.rect(PAD, y, (w - 2.0 * PAD) * f, 6.0, 3.0, color::ACCENT);
            c.circle(
                PAD + (w - 2.0 * PAD) * f,
                y + 3.0,
                if self.seek_target.is_some() {
                    11.0
                } else {
                    7.0
                },
                color::WHITE,
            );
            let end = crate::app::local_hms();
            let left = (dur - pos) as i32;
            let em = end.0 * 60 + end.1 + (left + 59) / 60;
            c.text(
                &format!("Ende um {:02}:{:02}", (em / 60) % 24, em % 60),
                w - PAD,
                y + 20.0,
                15,
                Weight::Regular,
                color::MUTED,
                Align::Right,
            );
        }
        c.text(
            "◀ ▶ Springen · OK Pause · ▼ Ton & Untertitel",
            PAD,
            y + 20.0,
            15,
            Weight::Regular,
            color::MUTED,
            Align::Left,
        );
        if let Some(m) = self.menu {
            let info = sh.info.lock().unwrap().clone();
            let a = sh.audio_sel.load(Relaxed);
            let s = sh.sub_sel.load(Relaxed);
            let audio = info
                .audio
                .get(a)
                .map(|t| {
                    format!(
                        "{} · {}",
                        super::films::lang_name(&t.lang, &t.name),
                        t.codec.name()
                    )
                })
                .unwrap_or_else(|| "–".into());
            let sub = if s >= 0 {
                info.subs
                    .get(s as usize)
                    .map(|t| super::films::lang_name(&t.lang, &t.name))
                    .unwrap_or_else(|| "Ein".into())
            } else {
                "Aus".into()
            };
            let rows = [
                ("Ton", audio, info.audio.len() > 1),
                (
                    "Untertitel",
                    sub,
                    !info.subs.is_empty() || !sh.external_subs.lock().unwrap().is_empty(),
                ),
            ];
            let bx = w - PAD - 460.0;
            let by = h - 330.0;
            c.rect(bx, by, 460.0, 150.0, 18.0, Rgba(24, 24, 27, 235));
            for (i, (k, v, can)) in rows.iter().enumerate() {
                let yy = by + 16.0 + i as f32 * 62.0;
                let f = m == i;
                if f {
                    c.rect(bx + 10.0, yy, 440.0, 54.0, 12.0, color::WHITE);
                }
                let col = if f { color::BLACK } else { color::TEXT };
                c.text(k, bx + 26.0, yy + 15.0, 18, Weight::Semi, col, Align::Left);
                let vv = if *can {
                    format!("‹ {v} ›")
                } else {
                    v.clone()
                };
                let vv = c.ellipsize(&vv, 18, Weight::Regular, 290.0);
                c.text(
                    &vv,
                    bx + 434.0,
                    yy + 15.0,
                    18,
                    Weight::Regular,
                    col,
                    Align::Right,
                );
            }
        }
    }
}
