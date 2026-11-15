//! The TV screen through Linux KMS (/dev/dri/cardN) -- no X11, no Wayland,
//! no Kodi. LibreELEC on the Pi 3 runs the vc4 KMS driver.
//!
//! Planes do the heavy lifting in hardware:
//!   - menus: the UI (drawn at 1280x720) sits on the primary plane, scaled to
//!     the TV's mode by the display controller -- no CPU spent on scaling;
//!   - films: the decoded picture (NV12 straight from the H.264 decoder, as
//!     DMABUF, zero copy) goes on the primary plane, letterboxed, and the UI
//!     (OSD, subtitles) on an overlay plane with per-pixel alpha above it.
use crate::gfx::Damage;
use drm::buffer::{Buffer, DrmFourcc, DrmModifier, Handle as BufHandle, PlanarBuffer};
use drm::control::{
    connector, crtc, dumbbuffer::DumbBuffer, framebuffer, plane, Device as ControlDevice,
    FbCmd2Flags, Mode,
};
use drm::{ClientCapability, Device as BasicDevice};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};

pub struct Card(File);
impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}
impl BasicDevice for Card {}
impl ControlDevice for Card {}

struct Planar {
    size: (u32, u32),
    format: DrmFourcc,
    handles: [Option<BufHandle>; 4],
    pitches: [u32; 4],
    offsets: [u32; 4],
}
impl PlanarBuffer for Planar {
    fn size(&self) -> (u32, u32) {
        self.size
    }
    fn format(&self) -> DrmFourcc {
        self.format
    }
    fn modifier(&self) -> Option<DrmModifier> {
        None
    }
    fn pitches(&self) -> [u32; 4] {
        self.pitches
    }
    fn handles(&self) -> [Option<BufHandle>; 4] {
        self.handles
    }
    fn offsets(&self) -> [u32; 4] {
        self.offsets
    }
}

/// A UI buffer: one dumb buffer, seen as XRGB (opaque, menus) or ARGB (OSD
/// over video), mapped into memory once for the whole run.
struct UiBuf {
    db: DumbBuffer,
    fb_x: framebuffer::Handle,
    fb_a: framebuffer::Handle,
    map: *mut u8,
    len: usize,
}
unsafe impl Send for UiBuf {}

/// Plane updates for the flip thread: setting a plane waits for the TV's next
/// refresh (up to 16.7 ms at 60 Hz). That wait happens there, while the UI
/// thread can draw ahead while two buffers remain safely out of scanout.
enum Flip {
    Set {
        plane: plane::Handle,
        fb: Option<framebuffer::Handle>,
        ui_buffer: Option<usize>,
        dst: (i32, i32, u32, u32),
        src: (u32, u32, u32, u32),
    },
    /// answers once everything before it is on screen
    Sync(std::sync::mpsc::Sender<()>),
}

/// A raw pointer that may cross into the copy threads (each writes its own rows).
#[derive(Clone, Copy)]
struct SendPtr(*mut u8);
unsafe impl Send for SendPtr {}
unsafe impl Sync for SendPtr {}

/// A decoded video frame handed over by the player (DMABUF from V4L2).
pub struct VideoFrameDesc {
    pub fd: OwnedFd,
    pub width: u32,
    pub height: u32,
    /// DRM fourcc: NV12 or YUV420
    pub nv12: bool,
    pub pitches: [u32; 4],
    pub offsets: [u32; 4],
}

/// Where the video plays: x, y, w, h as parts of the screen (0..1), or full
/// screen. A menu sets it; the next frame the decoder shows takes it.
static VIDEO_WINDOW: std::sync::Mutex<Option<(f32, f32, f32, f32)>> = std::sync::Mutex::new(None);

pub fn set_video_window(w: Option<(f32, f32, f32, f32)>) {
    *VIDEO_WINDOW.lock().unwrap() = w;
}

fn video_window() -> Option<(f32, f32, f32, f32)> {
    *VIDEO_WINDOW.lock().unwrap()
}

pub struct Kms {
    card: Card,
    crtc: crtc::Handle,
    conn: connector::Handle,
    mode: Mode,
    /// the browser has the screen: draw nothing
    suspended: bool,
    primary: plane::Handle,
    overlay: Option<plane::Handle>,
    pub mode_size: (u32, u32),
    /// The YouTube browser has the screen (it may run its own mode, 720p).
    browser: bool,
    pub hz: u32,
    ui: [UiBuf; 4],
    /// short queue keeps the UI thread independent of the display's vblank wait
    flips: std::sync::mpsc::SyncSender<Flip>,
    released_ui: std::sync::mpsc::Receiver<usize>,
    _black: (DumbBuffer, framebuffer::Handle),
    ui_w: u32,
    ui_h: u32,
    ui_generation: u64,
    ui_versions: [u64; 4],
    ui_damage_history: [Damage; 5],
    /// A drawn frame was not copied to any buffer (overlay hidden, screen
    /// given away): the next one that is must be copied whole, its damage
    /// only says what changed since that missed frame.
    ui_missed: bool,
    ui_free: [bool; 4],
    next_ui: usize,
    latest_ui: Option<usize>,
    video_fbs: HashMap<u32, (framebuffer::Handle, BufHandle, (u32, u32))>,
    video_on: bool,
    overlay_on: bool,
}

fn plane_type(card: &Card, p: plane::Handle) -> Option<u64> {
    let props = card.get_properties(p).ok()?;
    let map = props.as_hashmap(card).ok()?;
    let h = map.get("type")?.handle();
    let v = props.iter().find(|(ph, _)| **ph == h).map(|(_, v)| *v);
    v
}

/// The TV mode: 1080p at 60 Hz (50 Hz if that's all), never interlaced and
/// never the 24/25/30 Hz film modes -- the menus must move smoothly. 4K TVs
/// prefer 2160p, which a Pi 3 can't drive well, so at most 1920x1080.
/// Override: TV_MODE=1280x720@60 (in /storage/tv-launcher/env).
fn pick_mode(modes: &[Mode]) -> Option<Mode> {
    if let Ok(want) = std::env::var("TV_MODE") {
        let (res, hz) = want.split_once('@').unwrap_or((&want, ""));
        let (w, h) = res.split_once('x').unwrap_or(("", ""));
        let (w, h, hz) = (
            w.parse::<u16>().ok(),
            h.parse::<u16>().ok(),
            hz.parse::<u32>().ok(),
        );
        if let Some(m) = modes.iter().find(|m| {
            Some(m.size().0) == w
                && Some(m.size().1) == h
                && hz.map(|z| m.vrefresh() == z).unwrap_or(true)
                && !interlaced(m)
        }) {
            return Some(*m);
        }
        crate::log(&format!("TV_MODE={want} bietet der Fernseher nicht an"));
    }
    let score = |m: &Mode| -> u64 {
        let (w, h) = m.size();
        let fits = w <= 1920 && h <= 1080;
        let r = m.vrefresh();
        let rate = match r {
            59..=61 => 4,
            49..=51 => 3,
            _ if r > 61 => 1, // 75/120 Hz monitor modes: fine but not needed
            _ => 0,           // 24/25/30 Hz
        };
        let preferred = m
            .mode_type()
            .contains(drm::control::ModeTypeFlags::PREFERRED);
        (!interlaced(m) as u64) << 40
            | (fits as u64) << 39
            | ((w as u64 * h as u64) << 4)
            | (rate << 1)
            | preferred as u64
    };
    modes.iter().copied().max_by_key(score)
}

fn interlaced(m: &Mode) -> bool {
    m.flags().contains(drm::control::ModeFlags::INTERLACE)
}

impl Kms {
    pub fn open(ui_w: u32, ui_h: u32) -> Result<Kms, String> {
        let mut last = String::from("kein /dev/dri/card*");
        for i in 0..4 {
            let path = format!("/dev/dri/card{i}");
            let Ok(f) = OpenOptions::new().read(true).write(true).open(&path) else {
                continue;
            };
            match Self::init(Card(f), ui_w, ui_h) {
                Ok(k) => {
                    crate::log(&format!(
                        "Bildschirm: {path}, {}x{} @ {} Hz",
                        k.mode_size.0, k.mode_size.1, k.hz
                    ));
                    return Ok(k);
                }
                Err(e) => last = format!("{path}: {e}"),
            }
        }
        Err(last)
    }

    fn init(card: Card, ui_w: u32, ui_h: u32) -> Result<Kms, String> {
        fn e(s: &'static str) -> impl Fn(std::io::Error) -> String {
            move |err| format!("{s}: {err}")
        }
        card.set_client_capability(ClientCapability::UniversalPlanes, true)
            .map_err(e("universal planes"))?;
        let res = card.resource_handles().map_err(e("resources"))?;
        let conn = res
            .connectors()
            .iter()
            .filter_map(|c| card.get_connector(*c, true).ok())
            .find(|c| c.state() == connector::State::Connected && !c.modes().is_empty())
            .ok_or("kein Fernseher angeschlossen (HDMI)")?;
        let mode = pick_mode(conn.modes()).ok_or("kein Bildmodus")?;
        let crtc = conn
            .current_encoder()
            .and_then(|enc| card.get_encoder(enc).ok())
            .and_then(|enc| enc.crtc())
            .or_else(|| {
                conn.encoders()
                    .iter()
                    .filter_map(|h| card.get_encoder(*h).ok())
                    .find_map(|enc| res.filter_crtcs(enc.possible_crtcs()).first().copied())
            })
            .ok_or("kein CRTC")?;
        let (mw, mh) = (mode.size().0 as u32, mode.size().1 as u32);

        // Black full-size buffer for the modeset itself.
        let mut black = card
            .create_dumb_buffer((mw, mh), DrmFourcc::Xrgb8888, 32)
            .map_err(e("dumb"))?;
        if let Ok(mut m) = card.map_dumb_buffer(&mut black) {
            m.as_mut().fill(0);
        }
        let black_fb = card.add_framebuffer(&black, 24, 32).map_err(e("fb"))?;
        card.set_crtc(crtc, Some(black_fb), (0, 0), &[conn.handle()], Some(mode))
            .map_err(e("modeset"))?;

        // Planes on our CRTC: the primary one + an overlay that takes ARGB.
        let mut primary = None;
        let mut overlay = None;
        for p in card.plane_handles().map_err(e("planes"))? {
            let Ok(info) = card.get_plane(p) else {
                continue;
            };
            if !res.filter_crtcs(info.possible_crtcs()).contains(&crtc) {
                continue;
            }
            match plane_type(&card, p) {
                Some(1) if primary.is_none() => primary = Some(p),
                Some(0)
                    if overlay.is_none()
                        && info.formats().contains(&(DrmFourcc::Argb8888 as u32)) =>
                {
                    overlay = Some(p)
                }
                _ => {}
            }
        }
        let primary = primary.ok_or("keine Primary-Plane")?;

        let mk = |card: &Card| -> Result<UiBuf, String> {
            let mut db = card
                .create_dumb_buffer((ui_w, ui_h), DrmFourcc::Xrgb8888, 32)
                .map_err(e("ui dumb"))?;
            if let Ok(mut m) = card.map_dumb_buffer(&mut db) {
                m.as_mut().fill(0);
            }
            let planar = |fmt| Planar {
                size: (ui_w, ui_h),
                format: fmt,
                handles: [Some(db.handle()), None, None, None],
                pitches: [db.pitch(), 0, 0, 0],
                offsets: [0; 4],
            };
            let fb_x = card
                .add_planar_framebuffer(&planar(DrmFourcc::Xrgb8888), FbCmd2Flags::empty())
                .map_err(e("ui fb"))?;
            let fb_a = card
                .add_planar_framebuffer(&planar(DrmFourcc::Argb8888), FbCmd2Flags::empty())
                .map_err(e("ui fb argb"))?;
            let info = drm_ffi::mode::dumbbuffer::map(card.as_fd(), db.handle().into(), 0, 0)
                .map_err(|err| format!("map: {err}"))?;
            let len = db.pitch() as usize * ui_h as usize;
            let ptr = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    len,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_SHARED,
                    card.as_fd().as_raw_fd(),
                    info.offset as libc::off_t,
                )
            };
            if ptr == libc::MAP_FAILED {
                return Err("mmap UI-Puffer".into());
            }
            Ok(UiBuf {
                db,
                fb_x,
                fb_a,
                map: ptr as *mut u8,
                len,
            })
        };
        let ui = [mk(&card)?, mk(&card)?, mk(&card)?, mk(&card)?];
        // the flip thread gets its own handle on the same device
        let card2 = Card(card.0.try_clone().map_err(|e| format!("dup: {e}"))?);
        // Keep a short queue so drawing can overlap vblank. Repeated updates
        // to one plane are coalesced to the newest frame before each commit.
        let (flips, rx) = std::sync::mpsc::sync_channel::<Flip>(3);
        let (release_ui_tx, released_ui) = std::sync::mpsc::channel::<usize>();
        std::thread::spawn(move || {
            let mut current_ui: HashMap<plane::Handle, usize> = HashMap::new();
            let mut commit_count = 0u32;
            let mut coalesced_count = 0u32;
            let mut failed_count = 0u32;
            let mut commit_sum_ms = 0.0f64;
            let mut commit_max_ms = 0.0f64;
            let mut commit_window = std::time::Instant::now();
            while let Ok(first) = rx.recv() {
                let mut sets: Vec<(
                    plane::Handle,
                    Option<framebuffer::Handle>,
                    Option<usize>,
                    (i32, i32, u32, u32),
                    (u32, u32, u32, u32),
                )> = Vec::with_capacity(2);
                let mut waiters = Vec::new();
                let mut collect = |f| match f {
                    Flip::Set {
                        plane,
                        fb,
                        ui_buffer,
                        dst,
                        src,
                    } => {
                        if let Some(existing) = sets.iter_mut().find(|entry| entry.0 == plane) {
                            coalesced_count += 1;
                            if let Some(old) = existing.2.filter(|old| Some(*old) != ui_buffer) {
                                let _ = release_ui_tx.send(old);
                            }
                            *existing = (plane, fb, ui_buffer, dst, src);
                        } else {
                            sets.push((plane, fb, ui_buffer, dst, src));
                        }
                    }
                    Flip::Sync(done) => waiters.push(done),
                };
                collect(first);
                while let Ok(next) = rx.try_recv() {
                    collect(next);
                }
                drop(collect);
                for (plane, fb, ui_buffer, dst, src) in sets {
                    let old = current_ui.get(&plane).copied();
                    let commit_started = std::time::Instant::now();
                    match card2.set_plane(plane, crtc, fb, 0, dst, src) {
                        Ok(()) => {
                            let commit_ms = commit_started.elapsed().as_secs_f64() * 1000.0;
                            commit_count += 1;
                            commit_sum_ms += commit_ms;
                            commit_max_ms = commit_max_ms.max(commit_ms);
                            if old != ui_buffer {
                                if let Some(old) = old {
                                    let _ = release_ui_tx.send(old);
                                }
                                if let Some(index) = ui_buffer {
                                    current_ui.insert(plane, index);
                                } else {
                                    current_ui.remove(&plane);
                                }
                            }
                        }
                        Err(e) => {
                            failed_count += 1;
                            crate::log(&format!("set_plane: {e}"));
                            if let Some(index) = ui_buffer.filter(|index| Some(*index) != old) {
                                let _ = release_ui_tx.send(index);
                            }
                        }
                    }
                }
                if commit_window.elapsed() >= std::time::Duration::from_secs(10) {
                    if commit_count > 0 {
                        let elapsed = commit_window.elapsed().as_secs_f64();
                        crate::log(&format!(
                            "KMS: {:.1} erfolgreiche Plane-Commits/s, {:.2} ms je Commit im Schnitt, max {:.1} ms, {} veraltete Frames gebündelt, {} Fehler",
                            commit_count as f64 / elapsed,
                            commit_sum_ms / commit_count as f64,
                            commit_max_ms,
                            coalesced_count,
                            failed_count,
                        ));
                    }
                    commit_count = 0;
                    coalesced_count = 0;
                    failed_count = 0;
                    commit_sum_ms = 0.0;
                    commit_max_ms = 0.0;
                    commit_window = std::time::Instant::now();
                }
                for done in waiters {
                    let _ = done.send(());
                }
            }
        });
        Ok(Kms {
            conn: conn.handle(),
            mode,
            suspended: false,
            card,
            crtc,
            primary,
            overlay,
            mode_size: (mw, mh),
            browser: false,
            hz: mode.vrefresh(),
            ui,
            ui_free: [true; 4],
            next_ui: 0,
            latest_ui: None,
            flips,
            released_ui,
            _black: (black, black_fb),
            ui_w,
            ui_h,
            ui_generation: 0,
            ui_versions: [0; 4],
            ui_damage_history: [Damage::default(); 5],
            ui_missed: false,
            video_fbs: HashMap::new(),
            video_on: false,
            overlay_on: false,
        })
    }

    fn queue(
        &self,
        plane: plane::Handle,
        fb: Option<framebuffer::Handle>,
        ui_buffer: Option<usize>,
        dst: (i32, i32, u32, u32),
        src: (u32, u32, u32, u32),
    ) -> bool {
        self.flips
            .send(Flip::Set {
                plane,
                fb,
                ui_buffer,
                dst,
                src,
            })
            .is_ok()
    }

    /// Only hand a framebuffer to KMS while it is neither scanning out nor
    /// waiting in the flip queue. This prevents a fast producer from drawing
    /// into a frame that the display thread has not consumed yet.
    fn acquire_ui_buffer(&mut self) -> Option<usize> {
        loop {
            while let Ok(index) = self.released_ui.try_recv() {
                if index < self.ui_free.len() {
                    self.ui_free[index] = true;
                }
            }
            for offset in 0..self.ui_free.len() {
                let index = (self.next_ui + offset) % self.ui_free.len();
                if self.ui_free[index] {
                    self.ui_free[index] = false;
                    self.next_ui = (index + 1) % self.ui_free.len();
                    return Some(index);
                }
            }
            match self.released_ui.recv() {
                Ok(index) if index < self.ui_free.len() => self.ui_free[index] = true,
                _ => {
                    crate::log("KMS-Flip-Thread meldet keine freien UI-Puffer mehr");
                    return None;
                }
            }
        }
    }

    /// Waits until every queued plane change is on screen.
    fn sync(&self) {
        let (tx, rx) = std::sync::mpsc::channel();
        if self.flips.send(Flip::Sync(tx)).is_ok() {
            let _ = rx.recv_timeout(std::time::Duration::from_secs(1));
        }
    }

    fn src(&self) -> (u32, u32, u32, u32) {
        (0, 0, self.ui_w << 16, self.ui_h << 16)
    }

    /// Shows a finished UI frame (BGRA bytes, ui_w x ui_h). While a film
    /// plays, the UI goes to the overlay plane with alpha; `visible=false`
    /// hides the overlay altogether (nothing on top of the picture).
    /// Gives the screen away (the YouTube TV browser takes DRM master).
    pub fn suspend(&mut self) {
        self.sync();
        self.suspended = true;
        if let Err(e) = self.card.release_master_lock() {
            crate::log(&format!("DRM master abgeben: {e}"));
        }
    }

    /// Takes the screen back and sets our mode and planes again.
    pub fn resume(&mut self) {
        let acquired = (0..30).any(|_| {
            if self.card.acquire_master_lock().is_ok() {
                true
            } else {
                std::thread::sleep(std::time::Duration::from_millis(25));
                false
            }
        });
        if !acquired {
            crate::log("DRM master konnte nach 750 ms nicht zurückgeholt werden");
        }
        if let Err(e) = self.card.set_crtc(
            self.crtc,
            Some(self._black.1),
            (0, 0),
            &[self.conn],
            Some(self.mode),
        ) {
            crate::log(&format!(
                "Bildschirmmodus nach YouTube TV nicht wiederhergestellt: {e}"
            ));
        }
        self.video_on = false;
        self.overlay_on = false;
        if let Some(ov) = self.overlay {
            self.queue(ov, None, None, (0, 0, 0, 0), (0, 0, 0, 0));
        }
        self.suspended = false;
    }

    /// Keep DRM master and let Cog share the same DRM file. Cog updates the
    /// primary plane while the launcher draws its menu on the ARGB overlay.
    pub fn start_browser(&mut self) -> i32 {
        if self.video_on {
            self.stop_video();
        }
        self.sync();
        if let Some(ov) = self.overlay {
            self.queue(ov, None, None, (0, 0, 0, 0), (0, 0, 0, 0));
        }
        self.overlay_on = false;
        self.video_on = true;
        self.browser = true;
        self.card.as_fd().as_raw_fd()
    }

    /// Restore the launcher's full-screen menu after Cog exits.
    pub fn end_browser(&mut self) {
        self.browser = false;
        // the browser may have switched the TV to 720p: our mode again
        if self.screen_size() != self.mode_size {
            if let Err(e) = self.card.set_crtc(
                self.crtc,
                Some(self._black.1),
                (0, 0),
                &[self.conn],
                Some(self.mode),
            ) {
                crate::log(&format!("Bildschirmmodus nach YouTube nicht wiederhergestellt: {e}"));
            } else {
                crate::log("Bildschirmmodus nach YouTube wiederhergestellt");
            }
        }
        self.stop_video();
    }

    /// The size of the mode the screen runs now (the browser may change it).
    fn screen_size(&self) -> (u32, u32) {
        self.card
            .get_crtc(self.crtc)
            .ok()
            .and_then(|c| c.mode())
            .map(|m| {
                let (w, h) = m.size();
                (w as u32, h as u32)
            })
            .unwrap_or(self.mode_size)
    }

    pub fn present_ui(&mut self, bgra: &[u8], visible: bool, mut damage: Damage) {
        let changed = damage.full || damage.len > 0;
        if self.suspended {
            self.ui_missed |= changed;
            return;
        }
        // over the browser the menu plane fills the screen as it runs now
        // (the display controller scales it, e.g. down to 720p)
        let (mw, mh) = if self.browser { self.screen_size() } else { self.mode_size };
        if self.video_on {
            let Some(ov) = self.overlay else {
                self.ui_missed |= changed;
                return;
            };
            if !visible {
                if self.overlay_on {
                    self.queue(ov, None, None, (0, 0, 0, 0), (0, 0, 0, 0));
                    self.overlay_on = false;
                }
                self.ui_missed |= changed;
                return;
            }
        }
        // Nothing new to copy -- unless the overlay was switched off in the
        // meantime and has to come back with the picture it had.
        let overlay_back = self.video_on && !self.overlay_on;
        if !changed && !self.ui_missed && !overlay_back {
            return;
        }
        let acquire_started = std::time::Instant::now();
        let Some(b) = self.acquire_ui_buffer() else {
            self.ui_missed = true;
            return;
        };
        if std::mem::take(&mut self.ui_missed) {
            damage.full = true;
            damage.len = 0;
        }
        let acquire_ms = acquire_started.elapsed().as_secs_f64() * 1000.0;
        self.ui_generation = self.ui_generation.wrapping_add(1);
        let generation = self.ui_generation;
        self.ui_damage_history[generation as usize % self.ui_damage_history.len()] = damage;
        let mut pending = Damage::default();
        let last = self.ui_versions[b];
        if last == 0 || generation.saturating_sub(last) > self.ui_damage_history.len() as u64 {
            pending.full = true;
        } else {
            for revision in last + 1..=generation {
                let change =
                    self.ui_damage_history[revision as usize % self.ui_damage_history.len()];
                if change.full {
                    pending.full = true;
                    pending.len = 0;
                    break;
                }
                for rect in change.rects[..change.len].iter().copied() {
                    pending.add_rect(rect);
                    if pending.full {
                        break;
                    }
                }
                if pending.full {
                    break;
                }
            }
        }
        let source_row = self.ui_w as usize * 4;
        let full = pending.full;
        // A buffer is updated only while it is not being scanned out. When
        // reused, apply every recorded dirty region since its prior frame.
        let buffer = &mut self.ui[b];
        let pitch = buffer.db.pitch() as usize;
        let dst = SendPtr(buffer.map);
        let len = buffer.len;
        let rects = if full { 1 } else { pending.len };
        let copy_started = std::time::Instant::now();
        if full
            && pitch == source_row
            && bgra.len() == source_row * self.ui_h as usize
            && len >= bgra.len()
        {
            // The common 1280x720 mode has no row padding. Copy the whole
            // surface in one pass instead of issuing 720 tiny copy calls.
            unsafe {
                std::ptr::copy_nonoverlapping(bgra.as_ptr(), dst.0, bgra.len());
            }
        } else {
            for index in 0..rects {
                let (x, y, mut w, mut h) = if full {
                    (0, 0, self.ui_w, self.ui_h)
                } else {
                    pending.rects[index]
                };
                if x >= self.ui_w || y >= self.ui_h {
                    continue;
                }
                w = w.min(self.ui_w - x);
                h = h.min(self.ui_h - y);
                let bytes = w as usize * 4;
                for py in y..y + h {
                    let src = py as usize * source_row + x as usize * 4;
                    let dst_offset = py as usize * pitch + x as usize * 4;
                    if src + bytes <= bgra.len() && dst_offset + bytes <= len {
                        unsafe {
                            std::ptr::copy_nonoverlapping(
                                bgra.as_ptr().add(src),
                                dst.0.add(dst_offset),
                                bytes,
                            )
                        };
                    }
                }
            }
        }
        let copy_ms = copy_started.elapsed().as_secs_f64() * 1000.0;
        if acquire_ms >= 4.0 {
            crate::log(&format!("KMS-Pufferwartezeit: {acquire_ms:.1} ms"));
        }
        if copy_ms >= 8.0 {
            crate::log(&format!(
                "KMS-Teilbildkopie: {copy_ms:.1} ms, {} Bereiche{}",
                rects,
                if full { ", Vollbild" } else { "" }
            ));
        }
        self.ui_versions[b] = generation;
        let src = self.src();
        // hand over to the flip thread; this returns as soon as it took the
        // frame, the wait for the TV's refresh happens there
        let queued = if self.video_on {
            let ov = self.overlay.unwrap();
            self.overlay_on = true;
            self.queue(ov, Some(self.ui[b].fb_a), Some(b), (0, 0, mw, mh), src)
        } else {
            self.queue(
                self.primary,
                Some(self.ui[b].fb_x),
                Some(b),
                (0, 0, mw, mh),
                src,
            )
        };
        if queued {
            self.latest_ui = Some(b);
        } else {
            self.ui_free[b] = true;
        }
    }

    /// Registers a decoder buffer (once per buffer) as a framebuffer.
    pub fn import_video(&mut self, id: u32, f: &VideoFrameDesc) -> Result<(), String> {
        let h = self
            .card
            .prime_fd_to_buffer(f.fd.as_fd())
            .map_err(|e| format!("prime: {e}"))?;
        let handles = if f.nv12 {
            [Some(h), Some(h), None, None]
        } else {
            [Some(h), Some(h), Some(h), None]
        };
        let p = Planar {
            size: (f.width, f.height),
            format: if f.nv12 {
                DrmFourcc::Nv12
            } else {
                DrmFourcc::Yuv420
            },
            handles,
            pitches: f.pitches,
            offsets: f.offsets,
        };
        let fb = self
            .card
            .add_planar_framebuffer(&p, FbCmd2Flags::empty())
            .map_err(|e| format!("video fb: {e}"))?;
        self.video_fbs.insert(id, (fb, h, (f.width, f.height)));
        Ok(())
    }

    /// Shows decoder buffer `id` full screen, keeping the aspect ratio
    /// (`crop`: the visible part of the coded picture, e.g. 1920x1080 of 1920x1088).
    pub fn show_video(&mut self, id: u32, crop: (u32, u32), sar: f32) {
        let Some((fb, _, _)) = self.video_fbs.get(&id).copied() else {
            return;
        };
        let (mw, mh) = self.mode_size;
        // full screen, or the window a menu asked for (scaled by the display
        // controller: costs the CPU nothing)
        let (bx, by, bw, bh) = video_window()
            .map(|(x, y, w, h)| {
                let k = |v: f32, m: u32| (v * m as f32).round() as u32;
                (k(x, mw), k(y, mh), k(w, mw).max(2), k(h, mh).max(2))
            })
            .unwrap_or((0, 0, mw, mh));
        let aspect = crop.0 as f32 * sar / crop.1 as f32;
        let (mut w, mut h) = (bw, (bw as f32 / aspect).round() as u32);
        if h > bh {
            h = bh;
            w = (bh as f32 * aspect).round() as u32;
        }
        let (x, y) = (bx + (bw - w) / 2, by + (bh - h) / 2);
        if !self.video_on {
            self.video_on = true;
            // The menu frame on the primary plane would flash: blank it first.
            if let Some(ov) = self.overlay {
                self.queue(ov, None, None, (0, 0, 0, 0), (0, 0, 0, 0));
                self.overlay_on = false;
            }
            self.sync();
        }
        if let Err(e) = self.card.set_plane(
            self.primary,
            self.crtc,
            Some(fb),
            0,
            (x as i32, y as i32, w, h),
            (0, 0, crop.0 << 16, crop.1 << 16),
        ) {
            crate::log(&format!("video plane: {e}"));
        }
    }

    /// Back to menus: forget the decoder's buffers.
    pub fn stop_video(&mut self) {
        self.video_on = false;
        if let Some(ov) = self.overlay {
            self.queue(ov, None, None, (0, 0, 0, 0), (0, 0, 0, 0));
        }
        self.overlay_on = false;
        let (mw, mh) = self.mode_size;
        let src = self.src();
        // The last displayed frame may still be held by the overlay plane.
        // Copy it to a free buffer before handing it to the primary plane so
        // both plane transitions have independent, correctly tracked leases.
        if let Some(last) = self.latest_ui {
            if let Some(b) = self.acquire_ui_buffer() {
                if b != last {
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            self.ui[last].map,
                            self.ui[b].map,
                            self.ui[b].len.min(self.ui[last].len),
                        );
                    }
                }
                self.ui_versions[b] = self.ui_versions[last];
                if self.queue(
                    self.primary,
                    Some(self.ui[b].fb_x),
                    Some(b),
                    (0, 0, mw, mh),
                    src,
                ) {
                    self.latest_ui = Some(b);
                } else {
                    self.ui_free[b] = true;
                }
            }
        }
        // the video buffers are freed next: the menu must be on screen first
        self.sync();
        for (_, (fb, h, _)) in self.video_fbs.drain() {
            let _ = self.card.destroy_framebuffer(fb);
            let _ = self.card.close_buffer(h);
        }
    }
}
