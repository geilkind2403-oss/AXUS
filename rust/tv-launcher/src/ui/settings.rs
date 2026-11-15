//! Settings: network, bluetooth, PC server, sound, phone remote, place, system.
mod view;

use super::*;
use crate::app::{load, Ctx, Nav, Screen, Slot};
use crate::gfx::{color, Align, Canvas, Rgba, Weight};
use crate::platform::system::{self, BtDevice, Wifi};
use std::sync::atomic::Ordering::Relaxed;

const SECTIONS: [&str; 9] = [
    "WLAN",
    "Bluetooth",
    "PC-Server",
    "Live TV",
    "Bild & Bedienung",
    "Ton",
    "Handy als Fernbedienung",
    "Ort fürs Wetter",
    "System",
];

enum Edit {
    LiveUrl,
    WifiPass(Wifi),
    FilmServer,
    City,
}

pub struct Settings {
    section: usize,
    inside: bool,
    item: usize,
    wifi: Slot<Vec<Wifi>>,
    wifi_requested: bool,
    bt: Slot<Vec<BtDevice>>,
    bt_requested: bool,
    busy: Slot<Result<String, String>>,
    update: Slot<Result<crate::updater::UpdateInfo, String>>,
    kb: Option<(Keyboard, Edit)>,
    confirm: Option<usize>,
    native_view: view::Animation,
}

impl Settings {
    pub fn new(cx: &Ctx) -> Settings {
        Settings {
            section: 9,
            inside: false,
            item: 0,
            // The first page is WLAN, so its initial scan should already be
            // running when the Settings screen appears.
            wifi: load(&cx.tx, system::wifi_scan),
            wifi_requested: true,
            bt: Slot::empty(),
            bt_requested: false,
            busy: Slot::empty(),
            update: Slot::empty(),
            kb: None,
            confirm: None,
            native_view: view::Animation::default(),
        }
    }

    fn items(&self, cx: &Ctx) -> usize {
        match self.section {
            9 => SECTIONS.len(),
            0 => self.wifi.get().map(|v| v.len()).unwrap_or(0) + 1,
            1 => self.bt.get().map(|v| v.len()).unwrap_or(0) + 1,
            2 => 2,
            3 => 4,
            4 => 4,
            5 => 2,
            7 => 1,
            8 => 5,
            _ => {
                let _ = cx;
                0
            }
        }
    }

    fn enter_section(&mut self, cx: &mut Ctx) {
        match self.section {
            0 if !self.wifi_requested && !self.wifi.is_set() => {
                self.wifi_requested = true;
                self.wifi = load(&cx.tx, system::wifi_scan);
            }
            1 if !self.bt_requested && !self.bt.is_set() => {
                self.bt_requested = true;
                self.bt = load(&cx.tx, || system::bt_devices(0));
            }
            _ => {}
        }
    }

    fn activate(&mut self, cx: &mut Ctx) -> Nav {
        if self.section == 9 {
            self.section = self.item.min(SECTIONS.len() - 1);
            self.item = 0;
            self.inside = true;
            self.enter_section(cx);
            return Nav::None;
        }
        match (self.section, self.item) {
            (0, 0) => {
                self.wifi_requested = true;
                self.wifi = load(&cx.tx, system::wifi_scan);
            }
            (0, i) => {
                if let Some(w) = self.wifi.get().and_then(|v| v.get(i - 1).cloned()) {
                    if w.secured && !w.connected {
                        self.kb = Some((
                            Keyboard::new(&format!("Passwort für „{}“", w.name), "", true),
                            Edit::WifiPass(w),
                        ));
                    } else {
                        let tx = cx.tx.clone();
                        self.busy = load(&tx, move || {
                            system::wifi_connect(&w, "").map(|_| "Verbunden".to_string())
                        });
                    }
                }
            }
            (1, 0) => {
                cx.toast("Suche 10 Sekunden nach Geräten – Gerät in den Kopplungsmodus bringen");
                self.bt_requested = true;
                self.bt = load(&cx.tx, || system::bt_devices(10));
            }
            (1, i) => {
                if let Some(d) = self.bt.get().and_then(|v| v.get(i - 1).cloned()) {
                    let mac = d.mac.clone();
                    self.busy = load(&cx.tx, move || {
                        system::bt_connect(&mac).map(|_| "Verbunden".to_string())
                    });
                }
            }
            (2, 0) => {
                self.kb = Some((
                    Keyboard::new(
                        "Film-Server (z. B. 192.168.0.21:8420)",
                        &cx.cfg.film_server,
                        false,
                    ),
                    Edit::FilmServer,
                ))
            }
            (2, 1) => {
                let f = cx.cfg.film_server.clone();
                self.busy = load(&cx.tx, move || {
                    let ping = |u: &str| -> String {
                        if u.is_empty() {
                            return "nicht eingetragen".into();
                        }
                        match crate::net::get_json(&format!("{u}/api/ping")) {
                            Ok(j) => format!(
                                "OK – {} ({} Filme)",
                                j["name"].as_str().unwrap_or("?"),
                                j["films"].as_u64().unwrap_or(0)
                            ),
                            Err(e) => format!("nicht erreichbar ({e})"),
                        }
                    };
                    Ok(format!("Film-Server: {}", ping(&f)))
                });
            }
            (5, 0) => {
                cx.cfg.audio_output = if cx.cfg.audio_output == "analog" {
                    "hdmi".into()
                } else {
                    "analog".into()
                };
                cx.save();
            }
            (7, 0) => {
                self.kb = Some((
                    Keyboard::new("Ort (z. B. Murr)", &cx.cfg.city, false),
                    Edit::City,
                ))
            }
            (3, 0) => {
                self.kb = Some((
                    Keyboard::new(
                        "Senderliste (Adresse einer .m3u, leer = Standardliste)",
                        &cx.cfg.livetv_url,
                        false,
                    ),
                    Edit::LiveUrl,
                ))
            }
            (3, 1) => {
                cx.reload_channels();
                cx.toast("Senderliste wird neu geladen");
            }
            (3, 2) | (3, 3) | (4, 0) | (4, 1) | (4, 2) => self.cycle(cx, 1),
            (8, 0) => self.confirm = Some(0),
            (8, 1) => self.confirm = Some(1),
            (8, 2) => self.confirm = Some(2),
            (8, 3) => {
                cx.images.trim();
                cx.toast("Bilder-Zwischenspeicher geleert");
            }
            (8, 4) => match self.update.get() {
                Some(Ok(info)) if !info.current => self.confirm = Some(3),
                Some(Ok(_)) => cx.toast("Die installierte Version ist bereits aktuell"),
                Some(Err(_)) | None => {
                    self.update = load(&cx.tx, crate::updater::stage_github_update);
                    cx.toast("Suche nach der neuesten GitHub-Version …");
                }
            },
            _ => {}
        }
        Nav::None
    }

    /// Left/Right (or OK) on a choice row.
    fn cycle(&mut self, cx: &mut Ctx, dir: i32) {
        let step = |list: &[u32], cur: u32| -> u32 {
            let i = list.iter().position(|v| *v == cur).unwrap_or(0) as i32;
            list[((i + dir).rem_euclid(list.len() as i32)) as usize]
        };
        match (self.section, self.item) {
            (3, 2) => cx.cfg.livetv_max_h = step(&[576, 720, 1080], cx.cfg.livetv_max_h),
            (3, 3) => cx.cfg.start_livetv = !cx.cfg.start_livetv,
            (4, 0) => cx.cfg.screensaver_min = step(&[0, 5, 10, 30], cx.cfg.screensaver_min),
            (4, 1) => {
                let cur = cx
                    .sleep_at
                    .map(|t| {
                        (t.saturating_duration_since(std::time::Instant::now())
                            .as_secs()
                            / 60
                            + 1) as u32
                    })
                    .unwrap_or(0);
                let opts = [0, 30, 60, 90, 120];
                let near = opts
                    .iter()
                    .copied()
                    .min_by_key(|o| (*o as i64 - cur as i64).abs())
                    .unwrap_or(0);
                let next = step(&opts, near);
                cx.sleep_at = (next > 0).then(|| {
                    std::time::Instant::now() + std::time::Duration::from_secs(next as u64 * 60)
                });
                if next > 0 {
                    cx.toast(format!("Fernseher geht in {next} Minuten aus"));
                }
            }
            (4, 2) => cx.cfg.sub_size = step(&[0, 1, 2], cx.cfg.sub_size),
            _ => return,
        }
        cx.save();
    }

    fn finish_edit(&mut self, text: String, e: Edit, cx: &mut Ctx) {
        let norm = |s: &str, port: u16| -> String {
            let s = s.trim().trim_end_matches('/');
            if s.is_empty() {
                return String::new();
            }
            let s = if s.starts_with("http") {
                s.to_string()
            } else {
                format!("http://{s}")
            };
            let host_part = s.splitn(3, '/').nth(2).unwrap_or("");
            if host_part.contains(':') {
                s
            } else {
                format!("{s}:{port}")
            }
        };
        match e {
            Edit::WifiPass(w) => {
                self.busy = load(&cx.tx, move || {
                    system::wifi_connect(&w, &text)
                        .map(|_| "Verbindet … (ein paar Sekunden)".to_string())
                });
                let tx = cx.tx.clone();
                let slot = self.wifi.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(6));
                    slot.set(system::wifi_scan());
                    let _ = tx.send(crate::app::Msg::Redraw);
                });
            }
            Edit::FilmServer => {
                cx.cfg.film_server = norm(&text, 8420);
                cx.save();
                let (s, d, p) = (
                    cx.cfg.film_server.clone(),
                    cx.cfg.film_dirs.clone(),
                    cx.cfg.positions.clone(),
                );
                cx.films.refresh(&cx.tx, &s, &d, &p);
            }
            Edit::LiveUrl => {
                cx.cfg.livetv_url = text.trim().to_string();
                cx.save();
                cx.reload_channels();
            }
            Edit::City => {
                let q = text.trim().to_string();
                let tx = cx.tx.clone();
                std::thread::spawn(move || {
                    let url = format!("https://geocoding-api.open-meteo.com/v1/search?name={}&count=1&language=de", crate::net::url_encode(&q));
                    let r = crate::net::get_json(&url);
                    let _ = tx.send(crate::app::Msg::Run(Box::new(move |cx| {
                        match r.ok().and_then(|j| j["results"].get(0).cloned()) {
                            Some(p) => {
                                cx.cfg.city = p["name"].as_str().unwrap_or(&q).to_string();
                                cx.cfg.lat = p["latitude"].as_f64().unwrap_or(cx.cfg.lat);
                                cx.cfg.lon = p["longitude"].as_f64().unwrap_or(cx.cfg.lon);
                                cx.save();
                                let (lat, lon) = (cx.cfg.lat, cx.cfg.lon);
                                cx.weather = load(&cx.tx, move || crate::weather::fetch(lat, lon));
                                cx.toast(format!("Wetter jetzt für {}", cx.cfg.city));
                            }
                            None => cx.toast(format!("„{q}“ nicht gefunden")),
                        }
                    })));
                });
            }
        }
    }
}

impl Screen for Settings {
    fn name(&self) -> &'static str {
        "settings"
    }
    fn animating(&self, _: &Ctx) -> bool {
        self.kb.is_some() || self.native_view.moving()
    }
    fn text(&mut self, s: &str, cx: &mut Ctx) {
        if let Some((kb, _)) = self.kb.as_mut() {
            kb.text = s.to_string();
            let (kb, e) = self.kb.take().unwrap();
            self.finish_edit(kb.text, e, cx);
        }
    }
    fn key(&mut self, k: Key, cx: &mut Ctx) -> Nav {
        if let Some((kb, _)) = self.kb.as_mut() {
            match kb.key(k) {
                KbResult::Done(t) => {
                    let (_, e) = self.kb.take().unwrap();
                    self.finish_edit(t, e, cx);
                }
                KbResult::Cancel => self.kb = None,
                _ => {}
            }
            return Nav::None;
        }
        if let Some(which) = self.confirm {
            if k == Key::Ok {
                match which {
                    0 => system::reboot(),
                    1 => system::poweroff(),
                    2 => system::switch_to_kodi(),
                    _ => match crate::updater::apply_staged_update() {
                        Ok(()) => cx.toast("Update wird installiert; der Launcher startet neu"),
                        Err(e) => cx.toast(e),
                    },
                }
            }
            self.confirm = None;
            return Nav::None;
        }
        if k == Key::Home {
            return Nav::Home;
        }
        if self.section == 9 && !self.inside {
            match k {
                Key::Back => return Nav::Pop,
                Key::Up if self.item > 0 => self.item -= 1,
                Key::Down if self.item + 1 < SECTIONS.len() => self.item += 1,
                Key::Right | Key::Ok => return self.activate(cx),
                _ => {}
            }
            return Nav::None;
        }
        if !self.inside {
            match k {
                Key::Back if self.section == 9 => return Nav::Pop,
                Key::Back => {
                    self.section = 9;
                    self.item = 0;
                }
                Key::Up if self.section == 0 => self.section = 9,
                Key::Up if self.section > 0 => self.section -= 1,
                Key::Down if self.section == 9 => self.section = 0,
                Key::Down if self.section + 1 < SECTIONS.len() => self.section += 1,
                Key::Right | Key::Ok if self.items(cx) > 0 => {
                    self.inside = true;
                    self.item = 0;
                    self.enter_section(cx);
                }
                _ => {}
            }
            if !self.inside {
                self.enter_section(cx);
            }
            return Nav::None;
        }
        let n = self.items(cx);
        match k {
            Key::Back => {
                let section = self.section;
                self.section = 9;
                self.inside = false;
                self.item = section;
            }
            Key::Left | Key::Right
                if (self.section == 3 && self.item >= 2)
                    || (self.section == 4 && self.item <= 2) =>
            {
                self.cycle(cx, if k == Key::Left { -1 } else { 1 });
            }
            Key::Left if self.section == 5 && self.item == 1 => {
                let v = cx.cfg.volume.saturating_sub(5);
                cx.cfg.volume = v;
                crate::platform_audio::VOLUME.store(v, Relaxed);
                cx.save();
            }
            Key::Right if self.section == 5 && self.item == 1 => {
                let v = (cx.cfg.volume + 5).min(100);
                cx.cfg.volume = v;
                crate::platform_audio::VOLUME.store(v, Relaxed);
                cx.save();
            }
            Key::Left => {
                let section = self.section;
                self.section = 9;
                self.inside = false;
                self.item = section;
            }
            Key::Up if self.item > 0 => self.item -= 1,
            Key::Down if self.item + 1 < n => self.item += 1,
            Key::Ok => return self.activate(cx),
            _ => {}
        }
        Nav::None
    }

    fn draw(&mut self, c: &mut Canvas, cx: &mut Ctx) {
        // A softly layered, cached gold light field gives glass something to
        // pick up while keeping the left side free of UI and CPU blur passes.
        c.ambient(crate::gfx::Ambient::LiquidGold);
        status_with_phone_accent(c, cx, Rgba(231, 190, 103, 255));
        let heading = if self.section == 9 {
            "Einstellungen"
        } else {
            SECTIONS[self.section]
        };
        let row_count = self.items(cx);
        view::draw_shell(
            c,
            &mut self.native_view,
            self.section,
            self.item,
            self.inside,
            row_count,
            heading,
            "",
        );
        let x = 650.0;
        let w = 570.0;
        let mut rows: Vec<(String, String)> = Vec::new();
        match self.section {
            9 => {
                rows.extend(
                    SECTIONS
                        .iter()
                        .map(|name| ((*name).to_string(), String::new())),
                );
            }
            0 => {
                let (ok, _, name) = cx.net.get().unwrap_or_default();
                rows.push((
                    "Netzwerke suchen".into(),
                    if ok {
                        format!("Verbunden: {name}")
                    } else {
                        "Nicht verbunden".into()
                    },
                ));
                match self.wifi.get() {
                    Some(list) => {
                        for wf in list {
                            rows.push((
                                format!(
                                    "{}{}",
                                    wf.name,
                                    if wf.secured { " · geschützt" } else { "" }
                                ),
                                if wf.connected {
                                    "Verbunden".into()
                                } else {
                                    format!("{} %", wf.strength)
                                },
                            ));
                        }
                    }
                    None => rows.push(("Suche …".into(), String::new())),
                }
            }
            1 => {
                rows.push((
                    "Nach Geräten suchen".into(),
                    "Kopfhörer, Tastatur, Fernbedienung".into(),
                ));
                for d in self.bt.get().unwrap_or_default() {
                    rows.push((
                        d.name.clone(),
                        if d.connected {
                            "Verbunden".into()
                        } else if d.paired {
                            "Gekoppelt".into()
                        } else {
                            "Verbinden".into()
                        },
                    ));
                }
            }
            2 => {
                rows.push((
                    "Film-Server".into(),
                    if cx.cfg.film_server.is_empty() {
                        "nicht eingetragen".into()
                    } else {
                        cx.cfg.film_server.clone()
                    },
                ));
                rows.push(("Verbindung testen".into(), String::new()));
            }
            3 => {
                let src = match cx.channels.get() {
                    Some(Ok((ch, from))) => format!(
                        "{} Sender · {}",
                        ch.len(),
                        from.rsplit('/').next().unwrap_or(&from)
                    ),
                    Some(Err(e)) => e,
                    None => "wird geladen …".into(),
                };
                rows.push(("Senderliste".into(), src));
                rows.push(("Senderliste neu laden".into(), String::new()));
                rows.push((
                    "Bildqualität".into(),
                    format!("‹ bis {}p ›", cx.cfg.livetv_max_h),
                ));
                rows.push((
                    "Beim Einschalten".into(),
                    if cx.cfg.start_livetv {
                        "‹ Live TV ›".into()
                    } else {
                        "‹ Startseite ›".into()
                    },
                ));
            }
            4 => {
                rows.push((
                    "Bildschirmschoner".into(),
                    if cx.cfg.screensaver_min == 0 {
                        "‹ Aus ›".into()
                    } else {
                        format!("‹ nach {} Min. ›", cx.cfg.screensaver_min)
                    },
                ));
                let sleep = match cx.sleep_at {
                    Some(t) => format!(
                        "‹ aus in {} Min. ›",
                        (t.saturating_duration_since(std::time::Instant::now())
                            .as_secs()
                            + 59)
                            / 60
                    ),
                    None => "‹ Aus ›".into(),
                };
                rows.push(("Ausschalt-Timer".into(), sleep));
                rows.push((
                    "Untertitel-Größe".into(),
                    format!(
                        "‹ {} ›",
                        ["Klein", "Normal", "Groß"][cx.cfg.sub_size.min(2) as usize]
                    ),
                ));
                let mode = screen_mode(cx);
                rows.push(("Bildschirm".into(), mode));
            }
            5 => {
                rows.push((
                    "Tonausgang".into(),
                    if cx.cfg.audio_output == "analog" {
                        "Kopfhörerbuchse".into()
                    } else {
                        "HDMI (Fernseher)".into()
                    },
                ));
                rows.push(("Lautstärke".into(), format!("‹ {} % ›", cx.cfg.volume)));
            }
            6 => {
                let url = cx.remote_url.clone();
                c.text(&url, x, 192.0, 26, Weight::Bold, color::TEXT, Align::Left);
                draw_qr(c, &url, x, 240.0, 240.0);
            }
            7 => {
                rows.push((
                    "Ort ändern".into(),
                    format!("{} ({:.2}, {:.2})", cx.cfg.city, cx.cfg.lat, cx.cfg.lon),
                ));
            }
            _ => {
                rows.push(("Neu starten".into(), String::new()));
                rows.push(("Ausschalten".into(), String::new()));
                rows.push(("Zu Kodi wechseln".into(), "LibreELEC wie vorher".into()));
                rows.push(("Zwischenspeicher leeren".into(), String::new()));
                let update_status = match self.update.get() {
                    Some(Ok(info)) if info.current => format!("Bereits aktuell · {}", info.version),
                    Some(Ok(info)) => format!("Update bereit · {}", info.version),
                    Some(Err(e)) => e,
                    None => "Noch nicht geprüft · OK zum Suchen".into(),
                };
                rows.push(("Software-Update suchen".into(), update_status));
                let temp = system::cpu_temp()
                    .map(|t| format!(" · CPU {t:.0} °C"))
                    .unwrap_or_default();
                let ip = system::local_ip().unwrap_or_else(|| "keine IP".into());
                c.text(
                    &format!(
                        "TV Launcher {} · Build {} · {ip}{temp}",
                        env!("CARGO_PKG_VERSION"),
                        crate::BUILD
                    ),
                    x,
                    654.0,
                    14,
                    Weight::Regular,
                    color::MUTED,
                    Align::Left,
                );
            }
        }
        view::draw_rows(
            c,
            &self.native_view,
            self.section,
            self.item,
            self.inside,
            &rows,
        );
        if let Some(r) = self.busy.get() {
            let (msg, col) = match r {
                Ok(m) => (m, color::GREEN),
                Err(e) => (e, color::RED),
            };
            c.paragraph(
                &msg,
                x + 18.0,
                595.0,
                w - 36.0,
                16,
                Weight::Semi,
                col,
                22.0,
                2,
            );
        }
        if let Some(which) = self.confirm {
            c.rect(0.0, 0.0, 1280.0, 720.0, 0.0, Rgba(0, 0, 0, 190));
            c.rect(340.0, 250.0, 600.0, 200.0, 22.0, color::PANEL_SOLID);
            c.stroke_rect(
                340.0,
                250.0,
                600.0,
                200.0,
                22.0,
                1.2,
                Rgba(218, 177, 94, 150),
            );
            let update_question;
            let q = match which {
                0 => "Jetzt neu starten?",
                1 => "Jetzt ausschalten?",
                2 => "Den Launcher beenden und Kodi starten?",
                _ => {
                    update_question = format!(
                        "Update {} installieren?",
                        self.update
                            .get()
                            .and_then(Result::ok)
                            .map(|v| v.version)
                            .unwrap_or_else(|| "unbekannt".into())
                    );
                    &update_question
                }
            };
            c.text(
                q,
                640.0,
                300.0,
                24,
                Weight::Bold,
                color::TEXT,
                Align::Center,
            );
            c.text(
                "OK = Ja · jede andere Taste = Nein",
                640.0,
                360.0,
                17,
                Weight::Regular,
                Rgba(190, 171, 132, 255),
                Align::Center,
            );
        }
        if let Some((kb, _)) = &self.kb {
            c.rect(0.0, 0.0, 1280.0, 720.0, 0.0, Rgba(0, 0, 0, 200));
            c.rect(190.0, 150.0, 900.0, 470.0, 24.0, color::PANEL_SOLID);
            c.stroke_rect(
                190.0,
                150.0,
                900.0,
                470.0,
                24.0,
                1.2,
                Rgba(218, 177, 94, 150),
            );
            kb.draw(c, 220.0, 176.0, 840.0, cx.t());
        }
    }
}

/// Small line icons for the section list.
fn section_icon(c: &mut Canvas, i: usize, x: f32, y: f32, col: Rgba) {
    match i {
        0 => icon_wifi(c, x, y - 2.0, 20.0, col),
        1 => c.line(
            &[
                (x - 5.0, y - 5.0),
                (x + 5.0, y + 5.0),
                (x, y + 10.0),
                (x, y - 10.0),
                (x + 5.0, y - 5.0),
                (x - 5.0, y + 5.0),
            ],
            2.0,
            col,
        ),
        2 => {
            for k in 0..3 {
                c.stroke_rect(x - 9.0, y - 9.0 + k as f32 * 6.5, 18.0, 5.0, 1.5, 1.6, col);
            }
        }
        3 => {
            c.stroke_rect(x - 10.0, y - 7.0, 20.0, 13.0, 2.5, 1.8, col);
            c.line(&[(x - 5.0, y + 10.0), (x + 5.0, y + 10.0)], 1.8, col);
        }
        4 => {
            c.stroke_rect(x - 10.0, y - 8.0, 20.0, 16.0, 2.5, 1.8, col);
            c.polygon(
                &[
                    (x - 7.0, y + 5.0),
                    (x - 2.0, y - 1.0),
                    (x + 2.0, y + 3.0),
                    (x + 4.0, y + 1.0),
                    (x + 7.0, y + 5.0),
                ],
                col,
            );
        }
        5 => {
            c.polygon(
                &[
                    (x - 9.0, y - 3.0),
                    (x - 5.0, y - 3.0),
                    (x, y - 8.0),
                    (x, y + 8.0),
                    (x - 5.0, y + 3.0),
                    (x - 9.0, y + 3.0),
                ],
                col,
            );
            c.line(
                &[(x + 4.0, y - 4.0), (x + 6.0, y), (x + 4.0, y + 4.0)],
                1.8,
                col,
            );
            c.line(
                &[(x + 7.0, y - 7.0), (x + 10.0, y), (x + 7.0, y + 7.0)],
                1.8,
                col,
            );
        }
        6 => {
            c.stroke_rect(x - 6.0, y - 10.0, 12.0, 20.0, 3.0, 1.8, col);
            c.circle(x, y + 6.0, 1.3, col);
        }
        7 => {
            c.polygon(
                &[(x - 6.0, y - 2.0), (x, y + 10.0), (x + 6.0, y - 2.0)],
                col,
            );
            c.circle(x, y - 3.0, 6.5, col);
        }
        _ => icon_gear(c, x, y, 20.0, col),
    }
}

fn screen_mode(cx: &Ctx) -> String {
    #[cfg(target_os = "linux")]
    if let Some(k) = cx.kms.as_ref() {
        let k = k.lock().unwrap();
        return format!("{}x{} @ {} Hz", k.mode_size.0, k.mode_size.1, k.hz);
    }
    let _ = cx;
    "–".into()
}

/// QR code of the phone remote's address.
fn draw_qr(c: &mut Canvas, text: &str, x: f32, y: f32, size: f32) {
    let Ok(qr) = qrcodegen::QrCode::encode_text(text, qrcodegen::QrCodeEcc::Medium) else {
        return;
    };
    let n = qr.size();
    let quiet = 2;
    let cell = (size / (n + quiet * 2) as f32).floor();
    let total = cell * (n + quiet * 2) as f32;
    c.rect(x, y, total, total, 12.0, color::WHITE);
    for yy in 0..n {
        for xx in 0..n {
            if qr.get_module(xx, yy) {
                c.rect(
                    x + (xx + quiet) as f32 * cell,
                    y + (yy + quiet) as f32 * cell,
                    cell,
                    cell,
                    0.0,
                    color::BLACK,
                );
            }
        }
    }
}
