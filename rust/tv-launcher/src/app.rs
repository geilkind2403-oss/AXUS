//! The launcher's core: shared state (Ctx), the screen stack, background
//! jobs and the image cache.
use crate::gfx::{Canvas, Image};
use crate::platform::input::Key;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Film server on the PC, e.g. "http://192.168.0.21:8420".
    pub film_server: String,
    /// "hdmi" or "analog"
    pub audio_output: String,
    pub volume: u32,
    /// Home focus for the most recently used media app (0 YouTube, 1 Live TV, 2 films, 5 Apps).
    pub last_media_tile: usize,
    /// Home-screen tile order (slot -> content ID).
    pub home_order: [u8; 6],
    /// Last web app opened from Apps, used to restore its selection.
    pub last_app_id: String,
    /// Resume positions of films on the Pi itself (id -> seconds).
    pub positions: HashMap<String, f64>,
    /// Most recently played film, used to choose the best resume action.
    pub last_film_key: String,
    pub youtube_history: Vec<String>,
    /// Extra folders with films (besides USB drives).
    pub film_dirs: Vec<String>,
    pub city: String,
    pub lat: f64,
    pub lon: f64,
    /// Live TV: channel list URL ("" = default list), favourites, last channel, max picture height
    pub livetv_url: String,
    pub livetv_favs: Vec<String>,
    pub livetv_last: String,
    pub livetv_max_h: u32,
    /// open Live TV right after switching on
    pub start_livetv: bool,
    /// minutes without a key until the screensaver (0 = never)
    pub screensaver_min: u32,
    /// 0 small, 1 normal, 2 large
    pub sub_size: u32,
    /// Whether the phone remote's web server is available on the local network.
    pub remote_enabled: bool,
    /// Suppress nonessential on-screen notifications.
    pub do_not_disturb: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            film_server: String::new(),
            audio_output: "hdmi".into(),
            volume: 70,
            last_media_tile: 0,
            home_order: [0, 1, 2, 3, 4, 5],
            last_app_id: "youtube".into(),
            positions: HashMap::new(),
            last_film_key: String::new(),
            youtube_history: Vec::new(),
            film_dirs: vec![],
            city: "Murr".into(),
            lat: 48.9744,
            lon: 9.3714,
            livetv_url: String::new(),
            livetv_favs: Vec::new(),
            livetv_last: String::new(),
            livetv_max_h: 720,
            start_livetv: false,
            screensaver_min: 10,
            sub_size: 1,
            remote_enabled: true,
            do_not_disturb: false,
        }
    }
}

pub fn data_dir() -> PathBuf {
    if let Ok(d) = std::env::var("TV_DATA") {
        return PathBuf::from(d);
    }
    if std::path::Path::new("/storage").is_dir() {
        return PathBuf::from("/storage/tv-launcher/data");
    }
    PathBuf::from("data")
}

impl Config {
    pub fn load() -> Config {
        std::fs::read(data_dir().join("config.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    pub fn save(&self) {
        let d = data_dir();
        std::fs::create_dir_all(&d).ok();
        let tmp = d.join("config.json.tmp");
        if std::fs::write(&tmp, serde_json::to_vec_pretty(self).unwrap_or_default()).is_ok() {
            std::fs::rename(tmp, d.join("config.json")).ok();
        }
    }
}

/// Messages into the UI thread.
pub enum Msg {
    Input(crate::platform::input::InputMsg),
    /// Something a background job finished: just draw again.
    Redraw,
    Run(Box<dyn FnOnce(&mut Ctx) + Send>),
    Player(crate::player::Event),
    Standby,
}

#[allow(dead_code)]
pub enum Nav {
    None,
    Push(Box<dyn Screen>),
    Pop,
    Replace(Box<dyn Screen>),
    Home,
}

pub trait Screen {
    fn key(&mut self, k: Key, cx: &mut Ctx) -> Nav;
    fn draw(&mut self, c: &mut Canvas, cx: &mut Ctx);
    /// Redraw only the supplied portion of a retained screen below an OSD.
    /// Screens without a retained base fall back to their normal draw path.
    fn draw_overlay_region(&mut self, c: &mut Canvas, cx: &mut Ctx, _rect: (f32, f32, f32, f32)) {
        self.draw(c, cx);
    }
    /// Wants frames (an animation runs).
    fn animating(&self, _cx: &Ctx) -> bool {
        false
    }
    /// Something glides over the running picture (guide, banner, list):
    /// draw at full pace for that moment. Otherwise an overlay over video is
    /// only looked at a few times a second, which leaves the CPU to the film.
    fn motion_over_video(&self) -> bool {
        false
    }
    /// Text from the phone remote.
    fn text(&mut self, _s: &str, _cx: &mut Ctx) {}
    /// This screen shows a film/video (UI goes on the overlay plane).
    fn video(&self) -> bool {
        false
    }
    /// Draws on top of the current screen instead of replacing it.
    fn overlay(&self) -> bool {
        false
    }
    /// The overlay has nothing to show (picture only).
    fn overlay_empty(&self) -> bool {
        false
    }
    /// Called every frame before draw: background results can open screens.
    fn poll(&mut self, _cx: &mut Ctx) -> Nav {
        Nav::None
    }
    fn player_event(&mut self, _e: &crate::player::Event, _cx: &mut Ctx) -> Nav {
        Nav::None
    }
    #[allow(dead_code)]
    fn name(&self) -> &'static str;
    /// Home tile that should be selected when returning from this app.
    fn home_focus(&self) -> Option<usize> {
        None
    }
    /// Stable app identifier for resuming a selection in the app catalog.
    fn app_id(&self) -> Option<&'static str> {
        None
    }
}

/// A value a background thread fills in.
pub struct Slot<T>(pub Arc<Mutex<Option<T>>>, Arc<std::sync::atomic::AtomicU64>);
impl<T> Clone for Slot<T> {
    fn clone(&self) -> Self {
        Slot(self.0.clone(), self.1.clone())
    }
}
impl<T: Clone> Slot<T> {
    pub fn empty() -> Slot<T> {
        Slot(
            Arc::new(Mutex::new(None)),
            Arc::new(std::sync::atomic::AtomicU64::new(0)),
        )
    }
    pub fn get(&self) -> Option<T> {
        self.0.lock().unwrap().clone()
    }
    pub fn with<R>(&self, f: impl FnOnce(Option<&T>) -> R) -> R {
        let value = self.0.lock().unwrap();
        f(value.as_ref())
    }
    pub fn set(&self, v: T) {
        *self.0.lock().unwrap() = Some(v);
        self.1.fetch_add(1, std::sync::atomic::Ordering::Release);
    }
    pub fn is_set(&self) -> bool {
        self.0.lock().unwrap().is_some()
    }
    pub fn revision(&self) -> u64 {
        self.1.load(std::sync::atomic::Ordering::Acquire)
    }
}

/// Runs `f` on a thread, stores the result, asks for a redraw.
pub fn load<T: Clone + Send + 'static>(
    tx: &Sender<Msg>,
    f: impl FnOnce() -> T + Send + 'static,
) -> Slot<T> {
    let slot = Slot::empty();
    let s = slot.clone();
    let tx = tx.clone();
    std::thread::spawn(move || {
        s.set(f());
        let _ = tx.send(Msg::Redraw);
    });
    slot
}

/// Like `load`, but into a slot that is already shown: what is there stays
/// until the new value has arrived, and a failed fetch (`good` says no)
/// never replaces a good one. The weather and the headlines no longer blank
/// out every ten minutes, or for good when the WLAN drops for a moment.
pub fn refresh<T: Clone + Send + 'static>(
    tx: &Sender<Msg>,
    slot: &Slot<T>,
    f: impl FnOnce() -> T + Send + 'static,
    good: impl Fn(&T) -> bool + Send + 'static,
) {
    let slot = slot.clone();
    let tx = tx.clone();
    std::thread::spawn(move || {
        let new = f();
        let keep_old = !good(&new) && slot.with(|old| old.map(&good).unwrap_or(false));
        if !keep_old {
            slot.set(new);
            let _ = tx.send(Msg::Redraw);
        }
    });
}

#[allow(dead_code)]
enum ImgState {
    Loading,
    Ready(Image, u64),
    /// when it failed: tried again later (the network may just not be up yet)
    Failed(Instant),
}

/// A picture that could not be loaded is asked for again after this long.
const IMG_RETRY: std::time::Duration = std::time::Duration::from_secs(60);

/// Images by URL or file path, decoded off the UI thread, kept in RAM up to
/// a budget (the Pi 3 has 1 GB for everything), least recently used go first.
pub struct Images {
    map: Arc<Mutex<HashMap<String, (ImgState, u64)>>>,
    queue: Arc<(
        Mutex<Vec<(String, String, u32, u32, bool)>>,
        std::sync::Condvar,
    )>,
    tick: u64,
}

// On the Pi the pictures share 1 GB with the frame buffers, the backdrops
// and (in the YouTube TV app) a whole browser.
const IMG_BUDGET: usize = if cfg!(target_arch = "arm") {
    96 << 20
} else {
    160 << 20
};

impl Images {
    pub fn new(tx: Sender<Msg>) -> Images {
        let map: Arc<Mutex<HashMap<String, (ImgState, u64)>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let queue: Arc<(
            Mutex<Vec<(String, String, u32, u32, bool)>>,
            std::sync::Condvar,
        )> = Arc::new((Mutex::new(Vec::new()), std::sync::Condvar::new()));
        for _ in 0..3 {
            let (map, queue, tx) = (map.clone(), queue.clone(), tx.clone());
            std::thread::spawn(move || loop {
                let job = {
                    let (lock, cv) = &*queue;
                    let mut q = lock.lock().unwrap();
                    while q.is_empty() {
                        q = cv.wait(q).unwrap();
                    }
                    q.pop().unwrap() // newest first: what's on screen now
                };
                let (key, src, w, h, cover) = job;
                let bytes = if src.starts_with("http") {
                    crate::net::get_bytes(&src, 20 << 20).ok()
                } else {
                    std::fs::read(&src).ok()
                };
                let img = bytes.and_then(|b| {
                    if cover {
                        crate::gfx::decode_cover_image(&b, w, h)
                    } else {
                        crate::gfx::decode_image(&b, w, h)
                    }
                });
                let mut m = map.lock().unwrap();
                let t = m.get(&key).map(|e| e.1).unwrap_or(0);
                m.insert(
                    key,
                    (
                        img.map(|i| ImgState::Ready(i, 0))
                            .unwrap_or_else(|| ImgState::Failed(Instant::now())),
                        t,
                    ),
                );
                drop(m);
                let _ = tx.send(Msg::Redraw);
            });
        }
        Images {
            map,
            queue,
            tick: 0,
        }
    }

    /// A small stable signature for one image source, so artwork completion
    /// refreshes only the tile that displays that source.
    pub fn source_revision(&self, src: &str) -> u8 {
        let map = self.map.lock().unwrap();
        match map.get(src).map(|(state, _)| state) {
            None => 0,
            Some(ImgState::Loading) => 1,
            Some(ImgState::Ready(..)) => 2,
            Some(ImgState::Failed(_)) => 3,
        }
    }

    fn cover_key(src: &str, w: u32, h: u32) -> String {
        format!("\0cover:{w}x{h}:{src}")
    }

    pub fn cover_source_revision(&self, src: &str, w: u32, h: u32) -> u8 {
        self.source_revision(&Self::cover_key(src, w, h))
    }

    /// The image if it's ready; starts loading it otherwise. `w`,`h`: the
    /// largest size it will be drawn at (decoded no bigger than that).
    pub fn get(&mut self, src: &str, w: u32, h: u32) -> Option<Image> {
        self.get_variant(src, src, w, h, false)
    }

    /// Decode, cover-crop and resize an image on the image worker. This avoids
    /// doing a large thumbnail resample in the UI thread when a news card turns.
    pub fn get_cover(&mut self, src: &str, w: u32, h: u32) -> Option<Image> {
        let key = Self::cover_key(src, w, h);
        self.get_variant(&key, src, w, h, true)
    }

    fn get_variant(&mut self, key: &str, src: &str, w: u32, h: u32, cover: bool) -> Option<Image> {
        if src.is_empty() {
            return None;
        }
        self.tick += 1;
        let mut m = self.map.lock().unwrap();
        match m.get_mut(key) {
            Some((ImgState::Ready(i, _), t)) => {
                *t = self.tick;
                return Some(i.clone());
            }
            Some((ImgState::Failed(at), _)) if at.elapsed() >= IMG_RETRY => {}
            Some(_) => return None,
            None => {}
        }
        m.insert(key.to_string(), (ImgState::Loading, self.tick));
        // evict over budget
        let used: usize = m
            .values()
            .map(|(s, _)| {
                if let ImgState::Ready(i, _) = s {
                    i.data().len()
                } else {
                    0
                }
            })
            .sum();
        if used > IMG_BUDGET {
            let mut v: Vec<(String, u64)> = m
                .iter()
                .filter(|(_, (s, _))| matches!(s, ImgState::Ready(..)))
                .map(|(k, (_, t))| (k.clone(), *t))
                .collect();
            v.sort_by_key(|x| x.1);
            let mut freed = 0;
            for (k, _) in v {
                if used - freed < IMG_BUDGET * 3 / 4 {
                    break;
                }
                if let Some((ImgState::Ready(i, _), _)) = m.remove(&k) {
                    freed += i.data().len();
                }
            }
        }
        drop(m);
        let (lock, cv) = &*self.queue;
        let mut q = lock.lock().unwrap();
        q.retain(|j| j.0 != key);
        q.push((key.to_string(), src.to_string(), w, h, cover));
        if q.len() > 64 {
            let extra = q.len() - 64;
            let dropped: Vec<_> = q.drain(..extra).collect();
            let mut m = self.map.lock().unwrap();
            for d in dropped {
                m.remove(&d.0);
            }
        }
        cv.notify_one();
        None
    }

    pub fn trim(&mut self) {
        self.map.lock().unwrap().clear();
    }
}

pub struct Toast {
    pub text: String,
    pub at: Instant,
}

/// Everything screens share.
pub struct Ctx {
    pub cfg: Config,
    pub tx: Sender<Msg>,
    pub images: Images,
    pub toast: Option<Toast>,
    pub weather: Slot<Result<crate::weather::Weather, String>>,
    pub news: Slot<Vec<crate::news::NewsItem>>,
    /// A small preloaded YouTube top shelf, reused when YouTube TV opens.
    pub youtube_home: Slot<Vec<crate::youtube::Video>>,
    pub films: crate::films::FilmLibrary,
    pub net: Slot<(bool, String, String)>,
    pub started: Instant,
    pub now: Instant,
    /// Phone remote pairing code / address, for the settings page.
    pub remote_url: String,
    pub quit: bool,
    pub kms: Option<KmsHandle>,
    /// (channels, where from). The list is shared, not copied: Live TV asks
    /// for it on every frame and a list can have thousands of entries.
    pub channels: Slot<Result<(Arc<Vec<crate::livetv::Channel>>, String), String>>,
    /// sleep timer: when to stop and switch the TV off
    pub sleep_at: Option<Instant>,
}

#[cfg(target_os = "linux")]
pub type KmsHandle = Arc<Mutex<crate::platform::kms::Kms>>;
#[cfg(not(target_os = "linux"))]
pub type KmsHandle = Arc<Mutex<()>>;

impl Ctx {
    pub fn reload_channels(&mut self) {
        let url = self.cfg.livetv_url.clone();
        self.channels = load(&self.tx, move || {
            crate::livetv::load(&url).map(|(list, from)| (Arc::new(list), from))
        });
    }
    pub fn toast(&mut self, s: impl Into<String>) {
        if self.cfg.do_not_disturb {
            return;
        }
        self.toast = Some(Toast {
            text: s.into(),
            at: Instant::now(),
        });
    }
    pub fn save(&self) {
        self.cfg.save();
    }
    pub fn t(&self) -> f32 {
        self.now.duration_since(self.started).as_secs_f32()
    }
}

/// "1:05:09" / "5:09"
pub fn fmt_time(s: f64) -> String {
    let s = s.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Local time without a tz database: LibreELEC sets TZ / /etc/localtime;
/// libc's localtime_r reads it.
pub fn local_tm() -> libc::tm {
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm
    }
}

pub fn local_hms() -> (i32, i32, i32) {
    let tm = local_tm();
    (tm.tm_hour, tm.tm_min, tm.tm_sec)
}

pub const WEEKDAYS: [&str; 7] = [
    "Sonntag",
    "Montag",
    "Dienstag",
    "Mittwoch",
    "Donnerstag",
    "Freitag",
    "Samstag",
];
pub const MONTHS: [&str; 12] = [
    "Januar",
    "Februar",
    "März",
    "April",
    "Mai",
    "Juni",
    "Juli",
    "August",
    "September",
    "Oktober",
    "November",
    "Dezember",
];
