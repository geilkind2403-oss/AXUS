//! Software 2D on a tiny-skia pixmap: rounded rects, gradients, text
//! (fontdue, Inter) and images.
//!
//! The pixmap is kept in the byte order the TV's scanout wants (B,G,R,A --
//! DRM's XRGB8888/ARGB8888 on a little-endian CPU): colors and images are
//! swizzled once when they are made, so presenting a frame is a plain copy.
use fontdue::{Font, FontSettings};
use std::collections::HashMap;
use std::sync::Arc;
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Rect, Transform};

pub const W: u32 = 1280;
pub const H: u32 = 720;
/// Render at the Pi's native 1080p output size; other targets keep the
/// 720p logical canvas used by the design.
pub const RENDER_SCALE: f32 = if cfg!(target_arch = "arm") { 1.5 } else { 1.0 };
// The Pi CPU cannot blend 2880x1620 ambient frames cheaply. On ARM, swap the
// already-rendered backdrop in one frame; focus and page transitions remain
// animated. Desktop keeps the smoother crossfade.
const AMBIENT_FADE: f32 = if cfg!(target_arch = "arm") { 0.0 } else { 0.35 };
// Each cached backdrop is a whole frame: 8 MB at the Pi's 1080p. Four cover
// going back and forth between neighbours; a missing one is made again in
// the background within a few frames.
const AMBIENT_KEEP: usize = if cfg!(target_arch = "arm") { 4 } else { 8 };

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

impl Rgba {
    pub const fn hex(v: u32) -> Rgba {
        Rgba((v >> 16) as u8, (v >> 8) as u8, v as u8, 255)
    }
    pub const fn a(self, a: u8) -> Rgba {
        Rgba(self.0, self.1, self.2, a)
    }
    pub fn alpha(self, f: f32) -> Rgba {
        Rgba(
            self.0,
            self.1,
            self.2,
            (self.3 as f32 * f.clamp(0.0, 1.0)) as u8,
        )
    }
    /// Swizzled for the BGRA pixmap.
    fn sk(self) -> tiny_skia::Color {
        tiny_skia::Color::from_rgba8(self.2, self.1, self.0, self.3)
    }
}

#[allow(dead_code)]
pub mod color {
    use super::Rgba;
    pub const BG1: Rgba = Rgba::hex(0x0a0a0c);
    pub const BG2: Rgba = Rgba::hex(0x151517);
    pub const PANEL: Rgba = Rgba(255, 255, 255, 13);
    pub const PANEL_SOLID: Rgba = Rgba::hex(0x18181b);
    pub const RAISED: Rgba = Rgba::hex(0x202024);
    pub const BORDER: Rgba = Rgba(255, 255, 255, 20);
    pub const TEXT: Rgba = Rgba::hex(0xf1f1f3);
    pub const MUTED: Rgba = Rgba::hex(0x8b8b94);
    pub const ACCENT: Rgba = Rgba::hex(0x3b6dff);
    pub const WHITE: Rgba = Rgba(255, 255, 255, 255);
    pub const BLACK: Rgba = Rgba(0, 0, 0, 255);
    pub const RED: Rgba = Rgba::hex(0xff453a);
    pub const GREEN: Rgba = Rgba::hex(0x30d158);
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Weight {
    Regular,
    Semi,
    Bold,
}

pub type Glyph = Arc<(fontdue::Metrics, Vec<u8>)>;

pub struct Fonts {
    regular: Font,
    semi: Font,
    bold: Font,
    cache: HashMap<(Weight, char, u32), Glyph>,
}

impl Fonts {
    pub fn load() -> Fonts {
        let f = |b: &[u8]| {
            Font::from_bytes(
                b,
                FontSettings {
                    scale: 48.0,
                    ..Default::default()
                },
            )
            .expect("font")
        };
        Fonts {
            regular: f(include_bytes!("../assets/fonts/Inter-Regular.otf")),
            semi: f(include_bytes!("../assets/fonts/Inter-SemiBold.otf")),
            bold: f(include_bytes!("../assets/fonts/Inter-Bold.otf")),
            cache: HashMap::new(),
        }
    }
    fn font(&self, w: Weight) -> &Font {
        match w {
            Weight::Regular => &self.regular,
            Weight::Semi => &self.semi,
            Weight::Bold => &self.bold,
        }
    }
    fn glyph(&mut self, w: Weight, c: char, px: u32) -> &Glyph {
        if self.cache.len() > 6000 {
            self.cache.clear();
        }
        let font = match w {
            Weight::Regular => &self.regular,
            Weight::Semi => &self.semi,
            Weight::Bold => &self.bold,
        };
        self.cache
            .entry((w, c, px))
            .or_insert_with(|| Arc::new(font.rasterize(c, px as f32)))
    }
    pub fn advance(&mut self, w: Weight, c: char, px: u32) -> f32 {
        self.glyph(w, c, px).0.advance_width
    }
    pub fn width(&mut self, s: &str, px: u32, w: Weight) -> f32 {
        s.chars().map(|c| self.advance(w, c, px)).sum()
    }
    pub fn ascent(&self, px: u32, w: Weight) -> f32 {
        self.font(w)
            .horizontal_line_metrics(px as f32)
            .map(|m| m.ascent)
            .unwrap_or(px as f32 * 0.8)
    }
}

/// A decoded image, already swizzled + premultiplied for the pixmap.
pub type Image = Arc<Pixmap>;

/// Signature cache for a static layer whose pixels are restored separately
/// from the canvas' transient command list.
#[derive(Default)]
pub struct LayerCache(Option<(u64, u64)>);

/// Screen-space regions that changed since the last presented UI frame.
#[derive(Clone, Copy, Debug)]
pub struct Damage {
    pub full: bool,
    pub rects: [(u32, u32, u32, u32); 24],
    pub len: usize,
}

impl Default for Damage {
    fn default() -> Self {
        Self {
            full: false,
            rects: [(0, 0, 0, 0); 24],
            len: 0,
        }
    }
}

impl Damage {
    pub fn add_rect(&mut self, mut rect: (u32, u32, u32, u32)) {
        if self.full || rect.2 == 0 || rect.3 == 0 {
            return;
        }
        let mut i = 0;
        while i < self.len {
            let old = self.rects[i];
            let (ax1, ay1) = (rect.0.saturating_add(rect.2), rect.1.saturating_add(rect.3));
            let (bx1, by1) = (old.0.saturating_add(old.2), old.1.saturating_add(old.3));
            let same_row = rect.1 == old.1 && rect.3 == old.3;
            let same_col = rect.0 == old.0 && rect.2 == old.2;
            if (same_row || same_col)
                && rect.0 <= bx1
                && old.0 <= ax1
                && rect.1 <= by1
                && old.1 <= ay1
            {
                let x0 = rect.0.min(old.0);
                let y0 = rect.1.min(old.1);
                let x1 = ax1.max(bx1);
                let y1 = ay1.max(by1);
                rect = (x0, y0, x1 - x0, y1 - y0);
                self.len -= 1;
                self.rects[i] = self.rects[self.len];
                i = 0;
            } else {
                i += 1;
            }
        }
        if self.len == self.rects.len() {
            self.full = true;
            self.len = 0;
        } else {
            self.rects[self.len] = rect;
            self.len += 1;
        }
    }
}

pub fn decode_image(bytes: &[u8], max_w: u32, max_h: u32) -> Option<Image> {
    let img = image::load_from_memory(bytes).ok()?;
    let img = if img.width() > max_w || img.height() > max_h {
        img.thumbnail(max_w, max_h)
    } else {
        img
    };
    image_to_pixmap(img)
}

/// Decode a thumbnail and cover-crop it to its final display dimensions.
/// Called by the background image workers, so carousel transitions never
/// resize full images on the UI thread.
pub fn decode_cover_image(bytes: &[u8], w: u32, h: u32) -> Option<Image> {
    if w == 0 || h == 0 {
        return None;
    }
    let img = image::load_from_memory(bytes).ok()?;
    let img = img.resize_to_fill(w, h, image::imageops::FilterType::Triangle);
    image_to_rounded_pixmap(img, 22.0 * RENDER_SCALE)
}

fn image_to_pixmap(img: image::DynamicImage) -> Option<Image> {
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let mut pm = Pixmap::new(w, h)?;
    let data = pm.data_mut();
    for (o, p) in data.chunks_exact_mut(4).zip(rgba.pixels()) {
        let a = p[3] as u16;
        let pre = |c: u8| ((c as u16 * a + 127) / 255) as u8;
        o[0] = pre(p[2]);
        o[1] = pre(p[1]);
        o[2] = pre(p[0]);
        o[3] = p[3];
    }
    Some(Arc::new(pm))
}

fn image_to_rounded_pixmap(img: image::DynamicImage, radius: f32) -> Option<Image> {
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let mut pm = Pixmap::new(w, h)?;
    let data = pm.data_mut();
    let r = radius.min(w.min(h) as f32 / 2.0) as f64;
    for (idx, (o, p)) in data.chunks_exact_mut(4).zip(rgba.pixels()).enumerate() {
        let x = (idx as u32 % w) as f64 + 0.5;
        let y = (idx as u32 / w) as f64 + 0.5;
        let dx = if x < r {
            r - x
        } else if x > w as f64 - r {
            x - (w as f64 - r)
        } else {
            0.0
        };
        let dy = if y < r {
            r - y
        } else if y > h as f64 - r {
            y - (h as f64 - r)
        } else {
            0.0
        };
        let coverage = if dx == 0.0 || dy == 0.0 {
            1.0
        } else {
            (r + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0)
        };
        let a = (p[3] as f64 * coverage).round() as u16;
        let pre = |c: u8| ((c as u16 * a + 127) / 255) as u8;
        o[0] = pre(p[2]);
        o[1] = pre(p[1]);
        o[2] = pre(p[0]);
        o[3] = a as u8;
    }
    Some(Arc::new(pm))
}

pub enum Align {
    Left,
    Center,
    Right,
}

/// What a span-filled rounded box is filled with (premultiplied BGRA).
#[derive(Clone)]
enum Fill {
    Solid([u32; 4]),
    VGrad(Rgba, Rgba),
    HGrad(Rgba, Rgba),
    Image {
        src: Arc<Pixmap>,
        ox: i32,
        oy: i32,
        alpha: u32,
    },
}

/// One recorded drawing operation. A frame is recorded first (cheap, on the
/// UI thread) and then drawn by several threads, each into its own band of
/// rows -- the Pi 3 has four cores.
enum Cmd {
    Clear([u8; 4]),
    Backdrop,
    /// a full-screen picture (ambient backdrop) copied, or blended with alpha
    Screen(Arc<Pixmap>, u32),
    Path {
        path: tiny_skia::Path,
        color: tiny_skia::Color,
        stroke: Option<tiny_skia::Stroke>,
    },
    RoundedStroke {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        r: f32,
        thickness: f32,
        color: [u8; 4],
    },
    Spans {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        r: f32,
        fill: Fill,
    },
    Glyph {
        x: i32,
        y: i32,
        g: Glyph,
        c: Rgba,
    },
    /// Everything recorded after this is drawn only in these pixel rows
    /// [top, bottom) -- scrolling lists stay inside their box. None = whole screen.
    ClipY(Option<(i32, i32)>),
}

pub struct Canvas {
    /// The finished frame (after `finish`).
    pub pm: Pixmap,
    pub fonts: Fonts,
    scale: f32,
    /// Global fade (screen transitions) multiplied into every draw.
    pub opacity: f32,
    /// Moves everything drawn (screen transitions, sliding panels).
    pub offset: (f32, f32),
    cmds: Vec<Cmd>,
    backdrop: Option<Arc<Pixmap>>,
    /// ambient backdrops (blurred picture/colour behind the UI), by key
    ambients: Vec<(u64, Arc<Pixmap>)>,
    /// (current key, previous key, when it changed)
    ambient_now: (u64, u64, std::time::Instant),
    /// Keep the loop alive through the final fully-composited transition frame.
    ambient_final_frame: bool,
    ambient_pending: Vec<u64>,
    ambient_done: Arc<std::sync::Mutex<Vec<(u64, Option<Arc<Pixmap>>)>>>,
    /// images at their display size: ((identity, w, h), scaled)
    scaled: Vec<((usize, u32, u32), Arc<Pixmap>)>,
    threads: usize,
    last_sig: u64,
    layer_generation: u64,
    presented_directly: bool,
    full_base_restore: bool,
    region_scratch: Vec<u8>,
    damage: Damage,
    /// Fingerprint of what was drawn into each horizontal strip of the frame
    /// by the last `finish`. Empty = unknown (pixels were changed some other
    /// way since): everything is drawn.
    strip_sigs: Vec<u64>,
}

/// The frame is compared in this many horizontal strips: a frame that only
/// differs in a few of them (the clock, a moved selection, the OSD's time)
/// is drawn and sent to the TV only there.
const STRIPS: usize = 18;

/// Fingerprint of one recorded command: everything that decides its pixels.
fn cmd_hash(c: &Cmd) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let f = |v: f32, h: &mut std::collections::hash_map::DefaultHasher| v.to_bits().hash(h);
    match c {
        Cmd::Clear(p) => (0u8, p).hash(&mut h),
        Cmd::Backdrop => 1u8.hash(&mut h),
        Cmd::Screen(pm, a) => (2u8, Arc::as_ptr(pm) as usize, a).hash(&mut h),
        Cmd::Path {
            path,
            color,
            stroke,
        } => {
            3u8.hash(&mut h);
            for p in path.points() {
                f(p.x, &mut h);
                f(p.y, &mut h);
            }
            for v in [color.red(), color.green(), color.blue(), color.alpha()] {
                f(v, &mut h);
            }
            if let Some(s) = stroke {
                f(s.width, &mut h);
            }
        }
        Cmd::RoundedStroke {
            x,
            y,
            w,
            h: hh,
            r,
            thickness,
            color,
        } => {
            (6u8, color).hash(&mut h);
            for v in [x, y, w, hh, r, thickness] {
                f(*v, &mut h);
            }
        }
        Cmd::Spans {
            x,
            y,
            w,
            h: hh,
            r,
            fill,
        } => {
            (4u8, x, y, w, hh).hash(&mut h);
            f(*r, &mut h);
            match fill {
                Fill::Solid(p) => (0u8, p).hash(&mut h),
                Fill::VGrad(a, b) => (1u8, a.0, a.1, a.2, a.3, b.0, b.1, b.2, b.3).hash(&mut h),
                Fill::HGrad(a, b) => (3u8, a.0, a.1, a.2, a.3, b.0, b.1, b.2, b.3).hash(&mut h),
                Fill::Image { src, ox, oy, alpha } => {
                    (2u8, Arc::as_ptr(src) as usize, ox, oy, alpha).hash(&mut h)
                }
            }
        }
        Cmd::Glyph { x, y, g, c } => {
            // the address alone could be a new glyph in a freed one's place
            // (the glyph cache is emptied now and then): add its shape
            let (m, bmp) = (&g.0, &g.1);
            let mid = bmp.get(bmp.len() / 2).copied().unwrap_or(0);
            (5u8, x, y, Arc::as_ptr(g) as usize, c.0, c.1, c.2, c.3).hash(&mut h);
            (m.width, m.height, m.xmin, m.ymin, mid).hash(&mut h)
        }
        Cmd::ClipY(rows) => (7u8, rows).hash(&mut h),
    }
    h.finish()
}

/// The same outline made of straight pieces only (curves cut finely enough
/// that the difference is far below a pixel).
fn flatten(path: &tiny_skia::Path) -> Option<tiny_skia::Path> {
    use tiny_skia::{PathSegment, Point};
    const TOLERANCE: f32 = 0.05;
    let pieces = |bend: f32| ((bend / TOLERANCE).sqrt().ceil() as usize).clamp(1, 48);
    let dist = |a: Point, b: Point| ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt();
    let mut out = PathBuilder::new();
    let mut at = Point::from_xy(0.0, 0.0);
    for seg in path.segments() {
        match seg {
            PathSegment::MoveTo(p) => {
                out.move_to(p.x, p.y);
                at = p;
            }
            PathSegment::LineTo(p) => {
                out.line_to(p.x, p.y);
                at = p;
            }
            PathSegment::QuadTo(c, p) => {
                // how far the curve bends away from its chord
                let mid = Point::from_xy((at.x + p.x) / 2.0, (at.y + p.y) / 2.0);
                let n = pieces(dist(c, mid) / 2.0);
                for i in 1..=n {
                    let t = i as f32 / n as f32;
                    let u = 1.0 - t;
                    out.line_to(
                        u * u * at.x + 2.0 * u * t * c.x + t * t * p.x,
                        u * u * at.y + 2.0 * u * t * c.y + t * t * p.y,
                    );
                }
                at = p;
            }
            PathSegment::CubicTo(c1, c2, p) => {
                let bend = dist(
                    c1,
                    Point::from_xy((2.0 * at.x + p.x) / 3.0, (2.0 * at.y + p.y) / 3.0),
                )
                .max(dist(
                    c2,
                    Point::from_xy((at.x + 2.0 * p.x) / 3.0, (at.y + 2.0 * p.y) / 3.0),
                ));
                let n = pieces(bend * 0.75);
                for i in 1..=n {
                    let t = i as f32 / n as f32;
                    let u = 1.0 - t;
                    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                    out.line_to(
                        a * at.x + b * c1.x + c * c2.x + d * p.x,
                        a * at.y + b * c1.y + c * c2.y + d * p.y,
                    );
                }
                at = p;
            }
            PathSegment::Close => out.close(),
        }
    }
    out.finish()
}

fn round_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    if r < 0.5 {
        return Some(PathBuilder::from_rect(Rect::from_xywh(x, y, w, h)?));
    }
    // Cubic approximation of quarter circles.
    let k = r * 0.552_284_8;
    let mut p = PathBuilder::new();
    p.move_to(x + r, y);
    p.line_to(x + w - r, y);
    p.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    p.line_to(x + w, y + h - r);
    p.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    p.line_to(x + r, y + h);
    p.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    p.line_to(x, y + r);
    p.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    p.close();
    p.finish()
}

fn premul(c: Rgba) -> [u32; 4] {
    let a = c.3 as u32;
    [
        c.2 as u32 * a / 255,
        c.1 as u32 * a / 255,
        c.0 as u32 * a / 255,
        a,
    ]
}

impl Canvas {
    pub fn new(w: u32, h: u32) -> Canvas {
        Self::new_scaled(w, h, 1.0)
    }

    pub fn new_scaled(w: u32, h: u32, scale: f32) -> Canvas {
        let threads = std::env::var("TV_THREADS")
            .ok()
            .and_then(|t| t.parse().ok())
            .unwrap_or_else(|| {
                std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(1)
            });
        Self::new_scaled_with_threads(w, h, scale, threads)
    }

    /// Creates a canvas with an explicit raster worker count.
    pub fn new_scaled_with_threads(w: u32, h: u32, scale: f32, threads: usize) -> Canvas {
        Canvas {
            pm: Pixmap::new(
                (w as f32 * scale).round() as u32,
                (h as f32 * scale).round() as u32,
            )
            .unwrap(),
            fonts: Fonts::load(),
            scale,
            opacity: 1.0,
            offset: (0.0, 0.0),
            cmds: Vec::new(),
            backdrop: None,
            ambients: Vec::new(),
            ambient_now: (0, 0, std::time::Instant::now()),
            ambient_final_frame: false,
            ambient_pending: Vec::new(),
            ambient_done: Default::default(),
            scaled: Vec::new(),
            threads: threads.clamp(1, 4),
            last_sig: 0,
            layer_generation: 0,
            presented_directly: false,
            full_base_restore: false,
            region_scratch: Vec::new(),
            damage: Damage::default(),
            strip_sigs: Vec::new(),
        }
    }

    /// Changes the logical canvas size while preserving its font and scaled
    /// image caches. Used to reuse one small raster canvas for home tiles.
    pub fn resize(&mut self, w: u32, h: u32) {
        let (pw, ph) = (
            (w as f32 * self.scale).round() as u32,
            (h as f32 * self.scale).round() as u32,
        );
        if self.pm.width() == pw && self.pm.height() == ph {
            return;
        }
        self.pm = Pixmap::new(pw, ph).unwrap();
        self.strip_sigs.clear();
        self.cmds.clear();
        self.backdrop = None;
        self.last_sig = 0;
        self.layer_generation = self.layer_generation.wrapping_add(1);
        self.presented_directly = false;
    }

    /// Takes an immutable copy of the rendered pixels for use as a cached layer.
    pub fn snapshot(&self) -> Image {
        Arc::new(self.pm.clone())
    }

    pub fn reset_damage(&mut self) {
        self.damage = Damage::default();
    }
    pub fn damage(&self) -> Damage {
        self.damage
    }
    pub fn request_present(&mut self) {
        self.presented_directly = true;
    }
    fn mark_full_damage(&mut self) {
        self.damage.full = true;
        self.damage.len = 0;
    }
    pub fn mark_damage_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        if self.damage.full || w <= 0.0 || h <= 0.0 {
            return;
        }
        let x0 = ((x + self.offset.0) * self.scale).floor().max(0.0) as u32;
        let y0 = ((y + self.offset.1) * self.scale).floor().max(0.0) as u32;
        let x1 = ((x + w + self.offset.0) * self.scale)
            .ceil()
            .max(0.0)
            .min(self.pm.width() as f32) as u32;
        let y1 = ((y + h + self.offset.1) * self.scale)
            .ceil()
            .max(0.0)
            .min(self.pm.height() as f32) as u32;
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        self.damage.add_rect((x0, y0, x1 - x0, y1 - y0));
    }

    /// Restores a retained base frame into the reusable draw buffer.
    pub fn restore_image(&mut self, base: &Image) {
        if base.width() != self.pm.width() || base.height() != self.pm.height() {
            return;
        }
        self.mark_full_damage();
        self.strip_sigs.clear();
        let row = self.pm.width() as usize * 4;
        let rows_per_chunk = (self.pm.height() as usize).div_ceil(self.threads);
        let chunk_bytes = rows_per_chunk * row;
        let src = base.data();
        let dst = self.pm.data_mut();
        std::thread::scope(|scope| {
            for (i, chunk) in dst.chunks_mut(chunk_bytes).enumerate() {
                let start = i * chunk_bytes;
                let source = &src[start..start + chunk.len()];
                scope.spawn(move || chunk.copy_from_slice(source));
            }
        });
    }

    /// Restores a small logical region, used to erase a moving focus frame.
    pub fn restore_rect(&mut self, base: &Image, x: f32, y: f32, w: f32, h: f32) {
        let x0 = ((x + self.offset.0) * self.scale).floor() as i32;
        let y0 = ((y + self.offset.1) * self.scale).floor() as i32;
        let x1 = ((x + w + self.offset.0) * self.scale).ceil() as i32;
        let y1 = ((y + h + self.offset.1) * self.scale).ceil() as i32;
        let (pw, ph) = (self.pm.width() as i32, self.pm.height() as i32);
        let (x0, y0, x1, y1) = (x0.max(0), y0.max(0), x1.min(pw), y1.min(ph));
        if x0 >= x1
            || y0 >= y1
            || base.width() != self.pm.width()
            || base.height() != self.pm.height()
        {
            return;
        }
        self.mark_damage_rect(x, y, w, h);
        self.strip_sigs.clear();
        let row_bytes = self.pm.width() as usize * 4;
        let span = (x1 - x0) as usize * 4;
        let src = base.data();
        let rows = (y1 - y0) as usize;
        // a big area (a moving focus over two cards) on all cores
        let threads = if span * rows >= 512 << 10 { self.threads.clamp(1, 4) } else { 1 };
        let per = rows.div_ceil(threads);
        let dst = &mut self.pm.data_mut()[y0 as usize * row_bytes..y1 as usize * row_bytes];
        let copy = |chunk: &mut [u8], first: usize| {
            for (i, row) in chunk.chunks_mut(row_bytes).enumerate() {
                let start = (first + i) * row_bytes + x0 as usize * 4;
                let at = x0 as usize * 4;
                row[at..at + span].copy_from_slice(&src[start..start + span]);
            }
        };
        if threads == 1 {
            copy(dst, y0 as usize);
        } else {
            std::thread::scope(|sc| {
                for (i, chunk) in dst.chunks_mut(per * row_bytes).enumerate() {
                    let copy = &copy;
                    sc.spawn(move || copy(chunk, y0 as usize + i * per));
                }
            });
        }
    }

    /// Captures a compact rectangle from the rendered buffer for a transient
    /// effect such as the control-center backdrop blur.
    pub fn snapshot_region(&self, x: f32, y: f32, w: f32, h: f32) -> Option<Image> {
        let x0 = (x * self.scale).floor().max(0.0) as u32;
        let y0 = (y * self.scale).floor().max(0.0) as u32;
        let x1 = ((x + w) * self.scale).ceil().min(self.pm.width() as f32) as u32;
        let y1 = ((y + h) * self.scale).ceil().min(self.pm.height() as f32) as u32;
        if x0 >= x1 || y0 >= y1 {
            return None;
        }
        let mut region = Pixmap::new(x1 - x0, y1 - y0)?;
        let source = self.pm.data();
        let source_stride = self.pm.width() as usize * 4;
        let row_bytes = region.width() as usize * 4;
        for row in 0..region.height() as usize {
            let src = ((y0 as usize + row) * source_stride) + x0 as usize * 4;
            let dst = row * row_bytes;
            region.data_mut()[dst..dst + row_bytes].copy_from_slice(&source[src..src + row_bytes]);
        }
        Some(Arc::new(region))
    }

    /// Reuses a previously rendered region without recomputing its effect.
    pub fn restore_region_image(&mut self, region: &Image, x: f32, y: f32) {
        let x0 = (x * self.scale).round() as i32;
        let y0 = (y * self.scale).round() as i32;
        let (pw, ph) = (self.pm.width() as i32, self.pm.height() as i32);
        let sx0 = (-x0).max(0) as u32;
        let sy0 = (-y0).max(0) as u32;
        let sx1 = (region.width() as i32).min(pw - x0).max(0) as u32;
        let sy1 = (region.height() as i32).min(ph - y0).max(0) as u32;
        if sx0 >= sx1 || sy0 >= sy1 {
            return;
        }
        self.mark_damage_rect(
            x,
            y,
            region.width() as f32 / self.scale,
            region.height() as f32 / self.scale,
        );
        self.strip_sigs.clear();
        let src_stride = region.width() as usize * 4;
        let dst_stride = self.pm.width() as usize * 4;
        let row_bytes = (sx1 - sx0) as usize * 4;
        let src_data = region.data();
        let dst_data = self.pm.data_mut();
        for row in sy0..sy1 {
            let src = row as usize * src_stride + sx0 as usize * 4;
            let dst = (y0 as usize + row as usize) * dst_stride + (x0 as usize + sx0 as usize) * 4;
            dst_data[dst..dst + row_bytes].copy_from_slice(&src_data[src..src + row_bytes]);
        }
    }

    /// Updates a small rectangle in a retained image from the current canvas.
    /// Home uses this after replacing one cached tile, avoiding a full-frame
    /// snapshot just to keep its focus-restoration image current.
    pub fn update_image_region(&self, image: &mut Image, x: f32, y: f32, w: f32, h: f32) {
        let x0 = ((x + self.offset.0) * self.scale).floor() as i32;
        let y0 = ((y + self.offset.1) * self.scale).floor() as i32;
        let x1 = ((x + w + self.offset.0) * self.scale).ceil() as i32;
        let y1 = ((y + h + self.offset.1) * self.scale).ceil() as i32;
        let (pw, ph) = (self.pm.width() as i32, self.pm.height() as i32);
        let (x0, y0, x1, y1) = (x0.max(0), y0.max(0), x1.min(pw), y1.min(ph));
        if x0 >= x1
            || y0 >= y1
            || image.width() != self.pm.width()
            || image.height() != self.pm.height()
        {
            return;
        }
        let row_bytes = self.pm.width() as usize * 4;
        let span = (x1 - x0) as usize * 4;
        let src = self.pm.data();
        let dst = Arc::make_mut(image).data_mut();
        for py in y0..y1 {
            let start = py as usize * row_bytes + x0 as usize * 4;
            dst[start..start + span].copy_from_slice(&src[start..start + span]);
        }
    }

    /// Restores the narrow edge strips covered by the previous focus frame.
    pub fn restore_focus(&mut self, base: &Image, x: f32, y: f32, w: f32, h: f32) {
        const PAD: f32 = 7.0;
        const EDGE: f32 = 14.0;
        self.restore_rect(base, x - PAD, y - PAD, w + PAD * 2.0, EDGE);
        self.restore_rect(base, x - PAD, y + h - PAD, w + PAD * 2.0, EDGE);
        self.restore_rect(base, x - PAD, y + PAD, EDGE, (h - PAD * 2.0).max(0.0));
        self.restore_rect(base, x + w - PAD, y + PAD, EDGE, (h - PAD * 2.0).max(0.0));
    }

    /// Alpha-composites a small retained overlay directly into the main frame.
    pub fn composite_pixmap(&mut self, src: &Pixmap, x: f32, y: f32, w: f32, h: f32) {
        self.composite_pixmap_with_opacity(src, x, y, w, h, 1.0);
    }

    /// Alpha-composites a retained overlay while moving/fading it.
    pub fn composite_pixmap_with_opacity(
        &mut self,
        src: &Pixmap,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        opacity: f32,
    ) {
        let opacity = (opacity * self.opacity).clamp(0.0, 1.0);
        if opacity <= 0.0 {
            return;
        }
        let dx = ((x + self.offset.0) * self.scale).round() as i32;
        let dy = ((y + self.offset.1) * self.scale).round() as i32;
        let cw = ((w * self.scale).ceil() as usize).min(src.width() as usize);
        let ch = ((h * self.scale).ceil() as usize).min(src.height() as usize);
        let (dw, dh) = (self.pm.width() as i32, self.pm.height() as i32);
        let sx0 = (-dx).max(0) as usize;
        let sy0 = (-dy).max(0) as usize;
        let sx1 = cw.min((dw - dx).max(0) as usize);
        let sy1 = ch.min((dh - dy).max(0) as usize);
        if sx0 >= sx1 || sy0 >= sy1 {
            return;
        }
        self.mark_damage_rect(x, y, w, h);
        self.strip_sigs.clear();
        let opacity = (opacity * 255.0).round() as u32;
        let (src_row, dst_row) = (src.width() as usize * 4, self.pm.width() as usize * 4);
        let (src_data, dst_data) = (src.data(), self.pm.data_mut());
        for sy in sy0..sy1 {
            let py = (dy + sy as i32) as usize;
            for sx in sx0..sx1 {
                let si = sy * src_row + sx * 4;
                let di = py * dst_row + (dx + sx as i32) as usize * 4;
                let source_a = src_data[si + 3] as u32;
                let a = div255(source_a * opacity);
                if a == 0 {
                    continue;
                }
                if a == 255 {
                    dst_data[di..di + 4].copy_from_slice(&src_data[si..si + 4]);
                } else if a != 0 {
                    let inv = 255 - a;
                    for ch in 0..4 {
                        let source = div255(src_data[si + ch] as u32 * opacity);
                        dst_data[di + ch] =
                            (source + div255(dst_data[di + ch] as u32 * inv)).min(255) as u8;
                    }
                }
            }
        }
        self.last_sig = self.signature();
        self.presented_directly = true;
    }

    /// Nothing is recorded that still has to be drawn: the screen keeps its
    /// picture in the canvas and has already drawn its changes into it.
    pub fn pending_empty(&self) -> bool {
        self.cmds.is_empty()
    }

    pub fn set_full_base_restore(&mut self, required: bool) {
        self.full_base_restore = required;
    }

    pub fn full_base_restore_required(&self) -> bool {
        self.full_base_restore
    }

    fn path(&mut self, path: Option<tiny_skia::Path>, c: Rgba, stroke: Option<tiny_skia::Stroke>) {
        let c = c.alpha(self.opacity);
        if c.3 == 0 {
            return;
        }
        let scale = self.scale;
        let path = path.and_then(|p| {
            p.transform(Transform::from_scale(scale, scale))
                .and_then(|p| {
                    p.transform(Transform::from_translate(
                        self.offset.0 * scale,
                        self.offset.1 * scale,
                    ))
                })
        });
        let stroke = stroke.map(|mut s| {
            s.width *= scale;
            s
        });
        // The outline of a stroke is worked out here, once, and every curve
        // is turned into short straight pieces. Handing curves to the raster
        // threads had two costs: each band stroked the path again, and where
        // a band's edge cut through a curve (a rounded corner) tiny-skia
        // approximated it differently -- a seam, or missing pixels in 1 px
        // borders.
        let path = path.and_then(|p| match &stroke {
            Some(s) => p.stroke(s, 1.0),
            None => Some(p),
        });
        if let Some(path) = path.and_then(|p| flatten(&p)) {
            self.cmds.push(Cmd::Path {
                path,
                color: c.sk(),
                stroke: None,
            });
        }
    }

    /// Limits what is drawn next to the rows between `top` and `bottom`
    /// (a scrolling list must not run over the header above it). `None`
    /// lifts the limit again; every `Some` needs its `None` before the
    /// screen is done.
    pub fn clip_y(&mut self, rows: Option<(f32, f32)>) {
        let clip = rows.map(|(top, bottom)| {
            (
                ((top + self.offset.1) * self.scale).round() as i32,
                ((bottom + self.offset.1) * self.scale).round() as i32,
            )
        });
        self.cmds.push(Cmd::ClipY(clip));
    }

    pub fn clear(&mut self, c: Rgba) {
        // a clear hides everything recorded before it
        self.cmds.clear();
        let p = premul(c);
        self.cmds
            .push(Cmd::Clear([p[0] as u8, p[1] as u8, p[2] as u8, p[3] as u8]));
    }

    /// The launcher's backdrop: top-lit vignette like the old CSS (rendered
    /// once, then copied).
    pub fn backdrop(&mut self) {
        if self.backdrop.is_none() {
            let (w, h) = (self.pm.width(), self.pm.height());
            let mut pm = Pixmap::new(w, h).unwrap();
            Self::fill_backdrop(&mut pm, self.threads);
            self.backdrop = Some(Arc::new(pm));
        }
        self.cmds.clear();
        self.cmds.push(Cmd::Backdrop);
    }

    /// Raster the soft radial backdrop directly into the scanout-order pixel
    /// buffer. The previous tiny-skia full-screen shader took about a second on
    /// the Pi 3 during first launch. A 1024-entry radius LUT preserves the same
    /// three-stop vignette while keeping the first frame responsive.
    fn fill_backdrop(pm: &mut Pixmap, threads: usize) {
        let (w, h) = (pm.width() as usize, pm.height() as usize);
        const N: usize = 1024;
        let start = Rgba::hex(0x1b1b20);
        let middle = color::BG2;
        let end = color::BG1;
        let mut lut = Vec::with_capacity(N + 1);
        for i in 0..=N {
            let t = ((i as f32 / N as f32).sqrt()).clamp(0.0, 1.0);
            let (a, b, k) = if t <= 0.45 {
                (start, middle, t / 0.45)
            } else {
                (middle, end, (t - 0.45) / 0.55)
            };
            let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * k).round() as u8;
            lut.push([lerp(a.2, b.2), lerp(a.1, b.1), lerp(a.0, b.0), 255]);
        }
        let radius = w as f32 * 0.75;
        let radius_sq = radius * radius;
        let center_x = w as f32 * 0.5;
        let center_y = -(h as f32) * 0.1;
        // Split the radial distance into horizontal and vertical terms once.
        // The old inner pixel loop repeated floating-point division for every
        // output pixel; these compact tables keep the same 1024-step gradient
        // while leaving only integer addition and a lookup in the hot loop.
        let x_radius: Vec<u16> = (0..w)
            .map(|x| {
                let dx = x as f32 + 0.5 - center_x;
                ((dx * dx / radius_sq * N as f32).min(N as f32)) as u16
            })
            .collect();
        let y_radius: Vec<u16> = (0..h)
            .map(|y| {
                let dy = y as f32 + 0.5 - center_y;
                ((dy * dy / radius_sq * N as f32).min(N as f32)) as u16
            })
            .collect();
        let row_bytes = w * 4;
        let bands = threads.clamp(1, 4).min(h.max(1));
        let rows_per_band = h.div_ceil(bands);
        let data = pm.data_mut();
        std::thread::scope(|scope| {
            for (band, pixels) in data.chunks_mut(rows_per_band * row_bytes).enumerate() {
                let lut = &lut;
                let x_radius = &x_radius;
                let y_radius = &y_radius;
                scope.spawn(move || {
                    let first_y = band * rows_per_band;
                    for (row, pixels) in pixels.chunks_exact_mut(row_bytes).enumerate() {
                        let y_term = y_radius[first_y + row] as usize;
                        for (x, pixel) in pixels.chunks_exact_mut(4).enumerate() {
                            let i = (x_radius[x] as usize + y_term).min(N);
                            pixel.copy_from_slice(&lut[i]);
                        }
                    }
                });
            }
        });
    }

    /// The living backdrop: a strongly blurred, darkened version of a picture
    /// (or a colour glow) filling the screen; crossfades when it changes.
    /// Each one is rendered once and cached -- per frame it's a copy.
    pub fn ambient(&mut self, src: Ambient) {
        self.backdrop();
        let key = src.key();
        let liquid_gold = matches!(&src, Ambient::LiquidGold);
        let ready = self.ambient_pm(key, Some(&src)).is_some();
        // Normally switch only after the new image is ready. The gold glass
        // theme switches to the neutral backdrop immediately to avoid flashing
        // an unrelated color from the previous screen.
        if key != self.ambient_now.0 && (ready || key == 0 || liquid_gold) {
            // Settings should never flash the previous screen's unrelated
            // colors while its tiny cached gold light field is being prepared.
            let previous = if liquid_gold && !ready {
                0
            } else {
                self.ambient_now.0
            };
            self.ambient_now = (key, previous, std::time::Instant::now());
        }
        let k = if AMBIENT_FADE == 0.0 {
            1.0
        } else {
            (self.ambient_now.2.elapsed().as_secs_f32() / AMBIENT_FADE).min(1.0)
        };
        self.ambient_final_frame = k < 1.0;
        let cur = self.ambient_pm(self.ambient_now.0, None);
        let prev = if k < 1.0 {
            self.ambient_pm(self.ambient_now.1, None)
        } else {
            None
        };
        self.cmds.clear();
        match (prev, cur) {
            (Some(p), Some(c)) => {
                self.cmds.push(Cmd::Screen(p, 255));
                self.cmds.push(Cmd::Screen(c, (ease(k) * 255.0) as u32));
            }
            (None, Some(c)) if k < 1.0 => {
                self.cmds.push(Cmd::Backdrop);
                self.cmds.push(Cmd::Screen(c, (ease(k) * 255.0) as u32));
            }
            (_, Some(c)) => self.cmds.push(Cmd::Screen(c, 255)),
            _ => self.cmds.push(Cmd::Backdrop),
        }
    }

    /// The backdrop is crossfading or being made (keep looking for frames).
    pub fn ambient_moving(&self) -> bool {
        self.ambient_final_frame || !self.ambient_pending.is_empty()
    }

    /// A cached ambient, or None while it's being made in the background
    /// (never on the UI thread: that froze the whole TV on every focus move).
    fn ambient_pm(&mut self, key: u64, src: Option<&Ambient>) -> Option<Arc<Pixmap>> {
        if key == 0 {
            return None;
        }
        // finished background jobs
        for (k, pm) in self.ambient_done.lock().unwrap().drain(..) {
            self.ambient_pending.retain(|p| *p != k);
            if let Some(pm) = pm {
                if self.ambients.len() >= AMBIENT_KEEP {
                    self.ambients.remove(0);
                }
                self.ambients.push((k, pm));
            }
        }
        if let Some(i) = self.ambients.iter().position(|a| a.0 == key) {
            let e = self.ambients.remove(i);
            let pm = e.1.clone();
            self.ambients.push(e);
            return Some(pm);
        }
        let src = src?;
        if !self.ambient_pending.contains(&key) {
            let bd = self.backdrop.clone()?;
            let src = src.clone();
            let done = self.ambient_done.clone();
            self.ambient_pending.push(key);
            std::thread::spawn(move || {
                let pm = ambient_small(&src).map(|small| Arc::new(render_ambient(&bd, &small)));
                done.lock().unwrap().push((key, pm));
            });
        }
        None
    }

    /// Horizontal gradient (a picture fading into the page at its side).
    pub fn hgradient(&mut self, x: f32, y: f32, w: f32, h: f32, left: Rgba, right: Rgba) {
        let (l, r) = (left.alpha(self.opacity), right.alpha(self.opacity));
        self.spans(x, y, w, h, 0.0, Fill::HGrad(l, r));
    }

    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, c: Rgba) {
        if c.3 == 0 || w <= 0.0 || h <= 0.0 {
            return;
        }
        let c = c.alpha(self.opacity);
        if w * h >= 400.0 {
            self.spans(x, y, w, h, r, Fill::Solid(premul(c)));
        } else {
            let o = self.opacity;
            self.opacity = 1.0;
            self.path(round_rect(x, y, w, h, r), c, None);
            self.opacity = o;
        }
    }

    pub fn stroke_rect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, width: f32, c: Rgba) {
        let s = tiny_skia::Stroke {
            width,
            ..Default::default()
        };
        self.path(round_rect(x, y, w, h, r), c, Some(s));
    }

    /// Fast rounded outline rasterized as horizontal edge spans. This avoids
    /// allocating and stroking a full-screen tiny-skia path for focus rings.
    pub fn rounded_stroke_rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        r: f32,
        thickness: f32,
        c: Rgba,
    ) {
        if c.3 == 0 || w <= 0.0 || h <= 0.0 || thickness <= 0.0 {
            return;
        }
        let color = c.alpha(self.opacity);
        if color.3 == 0 {
            return;
        }
        self.cmds.push(Cmd::RoundedStroke {
            x: (x + self.offset.0) * self.scale,
            y: (y + self.offset.1) * self.scale,
            w: w * self.scale,
            h: h * self.scale,
            r: r * self.scale,
            thickness: thickness * self.scale,
            color: premul(color).map(|v| v as u8),
        });
    }

    pub fn circle(&mut self, cx: f32, cy: f32, r: f32, c: Rgba) {
        self.path(PathBuilder::from_circle(cx, cy, r), c, None);
    }

    pub fn line(&mut self, pts: &[(f32, f32)], width: f32, c: Rgba) {
        let mut pb = PathBuilder::new();
        for (i, (x, y)) in pts.iter().enumerate() {
            if i == 0 {
                pb.move_to(*x, *y)
            } else {
                pb.line_to(*x, *y)
            }
        }
        let s = tiny_skia::Stroke {
            width,
            line_cap: tiny_skia::LineCap::Round,
            line_join: tiny_skia::LineJoin::Round,
            ..Default::default()
        };
        self.path(pb.finish(), c, Some(s));
    }

    pub fn polygon(&mut self, pts: &[(f32, f32)], c: Rgba) {
        let mut pb = PathBuilder::new();
        for (i, (x, y)) in pts.iter().enumerate() {
            if i == 0 {
                pb.move_to(*x, *y)
            } else {
                pb.line_to(*x, *y)
            }
        }
        pb.close();
        self.path(pb.finish(), c, None);
    }

    /// Vertical gradient (scrims over photos), one colour per row.
    pub fn vgradient(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, top: Rgba, bottom: Rgba) {
        let (top, bottom) = (top.alpha(self.opacity), bottom.alpha(self.opacity));
        self.spans(x, y, w, h, r, Fill::VGrad(top, bottom));
    }

    /// Image scaled to cover (crop) the box, with rounded corners.
    pub fn image_cover(&mut self, img: &Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32) {
        self.image_fit(img, x, y, w, h, r, true, 1.0)
    }

    /// Composites a pre-rendered layer at native pixel size, without passing
    /// it through the general image-resampling path.
    pub fn image_exact(&mut self, img: &Image, x: f32, y: f32) {
        let w = img.width() as f32 / self.scale;
        let h = img.height() as f32 / self.scale;
        let (ox, oy) = (
            (x + self.offset.0) * self.scale,
            (y + self.offset.1) * self.scale,
        );
        let alpha = (self.opacity * 255.0).round().clamp(0.0, 255.0) as u32;
        self.spans(
            x,
            y,
            w,
            h,
            0.0,
            Fill::Image {
                src: img.clone(),
                ox: ox.round() as i32,
                oy: oy.round() as i32,
                alpha,
            },
        );
    }

    /// Image scaled once to its display size (kept in a small cache), then
    /// copied row by row with rounded corners -- no per-frame filtering.
    pub fn image_fit(
        &mut self,
        img: &Pixmap,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        r: f32,
        cover: bool,
        alpha: f32,
    ) {
        let (iw, ih) = (img.width() as f32, img.height() as f32);
        let s = (if cover {
            (w / iw).max(h / ih)
        } else {
            (w / iw).min(h / ih)
        }) * self.scale;
        let (dw, dh) = (
            (iw * s).round().max(1.0) as u32,
            (ih * s).round().max(1.0) as u32,
        );
        // pointer + size + a few sampled bytes: a freed image's address can be reused
        let d = img.data();
        let sample = (0..16).fold(
            img.width() as usize * 31 + img.height() as usize,
            |acc, k| {
                acc.wrapping_mul(131)
                    .wrapping_add(d[(d.len() / 17 * (k + 1)) & !3] as usize)
            },
        );
        let key = (img.data().as_ptr() as usize ^ sample, dw, dh);
        let src = match self.scaled.iter().position(|(k, _)| *k == key) {
            Some(i) => {
                let e = self.scaled.remove(i);
                let src = e.1.clone();
                self.scaled.push(e);
                src
            }
            None => {
                let Some(mut pm) = Pixmap::new(dw, dh) else {
                    return;
                };
                let sx = dw as f32 / iw;
                let sy = dh as f32 / ih;
                let q = if (sx - 1.0).abs() < 0.01 && (sy - 1.0).abs() < 0.01 {
                    tiny_skia::FilterQuality::Nearest
                } else {
                    tiny_skia::FilterQuality::Bilinear
                };
                let paint = tiny_skia::PixmapPaint {
                    quality: q,
                    ..Default::default()
                };
                pm.draw_pixmap(
                    0,
                    0,
                    img.as_ref(),
                    &paint,
                    Transform::from_scale(sx, sy),
                    None,
                );
                if self.scaled.len() >= 40 {
                    self.scaled.remove(0);
                }
                let pm = Arc::new(pm);
                self.scaled.push((key, pm.clone()));
                pm
            }
        };
        let (ox, oy) = (
            (x + (w - dw as f32 / self.scale) / 2.0 + self.offset.0) * self.scale,
            (y + (h - dh as f32 / self.scale) / 2.0 + self.offset.1) * self.scale,
        );
        // spans() adds the offset to the box; the picture origin has it already
        let (x, y) = (x, y);
        let (bx, by, bw, bh) = if cover {
            (x, y, w, h)
        } else {
            (
                x + (w - dw as f32 / self.scale) / 2.0,
                y + (h - dh as f32 / self.scale) / 2.0,
                dw as f32 / self.scale,
                dh as f32 / self.scale,
            )
        };
        let alpha = (alpha * self.opacity * 255.0).round().clamp(0.0, 255.0) as u32;
        self.spans(
            bx,
            by,
            bw,
            bh,
            r,
            Fill::Image {
                src,
                ox: ox.round() as i32,
                oy: oy.round() as i32,
                alpha,
            },
        );
    }

    fn spans(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, fill: Fill) {
        let scale = self.scale;
        let (w, h) = ((w * scale).round() as i32, (h * scale).round() as i32);
        if w <= 0 || h <= 0 {
            return;
        }
        let r = (r * scale).min(w as f32 / 2.0).min(h as f32 / 2.0).max(0.0);
        let (x, y) = ((x + self.offset.0) * scale, (y + self.offset.1) * scale);
        self.cmds.push(Cmd::Spans {
            x: x.round() as i32,
            y: y.round() as i32,
            w,
            h,
            r,
            fill,
        });
    }

    /// Single line of text; `y` is the top of the line box. Returns the width.
    pub fn text(
        &mut self,
        s: &str,
        x: f32,
        y: f32,
        px: u32,
        w: Weight,
        c: Rgba,
        align: Align,
    ) -> f32 {
        let scale = self.scale;
        let px = (px as f32 * scale).round().max(1.0) as u32;
        let width = self.fonts.width(s, px, w);
        let (x, y) = ((x + self.offset.0) * scale, (y + self.offset.1) * scale);
        let x0 = match align {
            Align::Left => x,
            Align::Center => x - width / 2.0,
            Align::Right => x - width,
        };
        let base = y + self.fonts.ascent(px, w);
        let col = c.alpha(self.opacity);
        if col.3 == 0 {
            return width;
        }
        let mut pen = x0;
        for ch in s.chars() {
            let g = self.fonts.glyph(w, ch, px).clone();
            let m = g.0;
            if m.width > 0 && m.height > 0 {
                let gx = (pen + m.xmin as f32).round() as i32;
                let gy = (base - m.height as f32 - m.ymin as f32).round() as i32;
                self.cmds.push(Cmd::Glyph {
                    x: gx,
                    y: gy,
                    g,
                    c: col,
                });
            }
            pen += m.advance_width;
        }
        width / scale
    }

    /// A fingerprint of the recorded frame: same fingerprint = same picture.
    fn signature(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for c in &self.cmds {
            cmd_hash(c).hash(&mut h);
        }
        h.finish()
    }

    /// One fingerprint per horizontal strip, made of the commands that draw
    /// into it (in order). Returns the strip height in rows too.
    fn strip_signatures(&self) -> (Vec<u64>, usize) {
        let h = self.pm.height() as usize;
        let rows = h.div_ceil(STRIPS).max(1);
        let n = h.div_ceil(rows);
        let mut sigs = vec![0xcbf2_9ce4_8422_2325u64; n];
        let mut clip: Option<(i32, i32)> = None;
        for c in &self.cmds {
            let (mut top, mut bottom) = match c {
                Cmd::ClipY(rows) => {
                    clip = *rows;
                    continue;
                }
                Cmd::Clear(_) | Cmd::Backdrop | Cmd::Screen(..) => (0, h as i32),
                Cmd::Path { path, stroke, .. } => {
                    let b = path.bounds();
                    let pad = stroke.as_ref().map(|s| s.width).unwrap_or(0.0) + 2.0;
                    (
                        (b.top() - pad).floor() as i32,
                        (b.bottom() + pad).ceil() as i32,
                    )
                }
                Cmd::RoundedStroke { y, h, .. } => (y.floor() as i32, (y + h).ceil() as i32),
                Cmd::Spans { y, h, .. } => (*y, *y + *h),
                Cmd::Glyph { y, g, .. } => (*y, *y + g.0.height as i32),
            };
            if let Some((a, b)) = clip {
                top = top.max(a);
                bottom = bottom.min(b);
            }
            let (top, bottom) = (top.max(0), bottom.min(h as i32));
            if top >= bottom {
                continue;
            }
            // the clip is part of what a command draws
            let ch = cmd_hash(c)
                ^ clip
                    .map(|(a, b)| (a as u64) << 32 | b as u32 as u64)
                    .unwrap_or(0);
            for sig in &mut sigs[top as usize / rows..=(bottom as usize - 1) / rows] {
                *sig = (sig.rotate_left(5) ^ ch).wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        (sigs, rows)
    }

    /// Draws the recorded frame -- unless it is exactly the one already
    /// drawn (a still menu then costs no drawing and no copy to the TV).
    /// Returns whether the picture changed.
    pub fn finish(&mut self) -> bool {
        let sig = self.signature();
        if sig == self.last_sig {
            self.cmds.clear();
            let changed = self.presented_directly;
            self.presented_directly = false;
            return changed;
        }
        self.last_sig = sig;
        self.presented_directly = false;
        let (sigs, rows) = self.strip_signatures();
        let changed: Vec<usize> = if sigs.len() == self.strip_sigs.len() {
            (0..sigs.len())
                .filter(|i| sigs[*i] != self.strip_sigs[*i])
                .collect()
        } else {
            (0..sigs.len()).collect()
        };
        // Strips can only be redrawn on their own when the frame starts from
        // a background; commands meant to go on top of kept pixels can't.
        let from_background = matches!(
            self.cmds.first(),
            Some(Cmd::Clear(_) | Cmd::Backdrop | Cmd::Screen(..))
        );
        // Mostly new anyway (scrolling, a new backdrop): one pass over everything.
        if changed.len() * 3 > sigs.len() * 2 || self.damage.full || !from_background {
            self.mark_full_damage();
            self.raster_pending();
            self.strip_sigs = sigs;
            return true;
        }
        if changed.is_empty() {
            // same pixels although the commands differ (drawn off-screen)
            self.cmds.clear();
            self.strip_sigs = sigs;
            return self.damage.len > 0;
        }
        self.raster_strips(&changed, rows);
        self.strip_sigs = sigs;
        true
    }

    /// Draws only the given strips of the recorded frame (ascending indices)
    /// and reports them as the damage.
    fn raster_strips(&mut self, strips: &[usize], rows: usize) {
        let (w, h) = (self.pm.width() as usize, self.pm.height() as usize);
        // neighbouring strips become one run of rows
        let mut runs: Vec<(usize, usize)> = Vec::new();
        for &i in strips {
            let (a, b) = (i * rows, ((i + 1) * rows).min(h));
            match runs.last_mut() {
                Some(last) if last.1 == a => last.1 = b,
                _ => runs.push((a, b)),
            }
        }
        for &(a, b) in &runs {
            self.damage
                .add_rect((0, a as u32, w as u32, (b - a) as u32));
        }
        // share the rows between the cores: long runs are cut up (a strip or
        // two is quicker done here than a thread is started)
        let total: usize = runs.iter().map(|r| r.1 - r.0).sum();
        let piece = total.div_ceil(self.threads).max(rows * 2);
        let mut pieces: Vec<(usize, usize)> = Vec::new();
        for (mut a, b) in runs {
            while a < b {
                let end = (a + piece).min(b);
                pieces.push((a, end));
                a = end;
            }
        }
        let cmds = &self.cmds;
        let bd = self.backdrop.as_deref();
        let row_bytes = w * 4;
        let mut rest = self.pm.data_mut();
        let mut at = 0usize;
        if pieces.len() <= 2 {
            for (a, b) in pieces {
                let band = &mut rest[a * row_bytes..b * row_bytes];
                raster(cmds, band, w as i32, a as i32, bd);
            }
        } else {
            std::thread::scope(|sc| {
                for (a, b) in pieces {
                    let (_, tail) = std::mem::take(&mut rest).split_at_mut((a - at) * row_bytes);
                    let (band, tail) = tail.split_at_mut((b - a) * row_bytes);
                    rest = tail;
                    at = b;
                    sc.spawn(move || raster(cmds, band, w as i32, a as i32, bd));
                }
            });
        }
        self.cmds.clear();
    }

    /// Draws or skips a static layer based on its own signature, independently
    /// of any transient overlay drawn over the retained pixels afterwards.
    /// Fingerprint of the commands drawn so far (to recognise a picture).
    pub fn layer_signature(&self) -> u64 {
        self.signature()
    }

    /// Whether `finish_cached` would find this picture already drawn.
    pub fn layer_is_cached(&self, cache: &LayerCache) -> bool {
        cache.0 == Some((self.layer_generation, self.signature()))
    }

    /// The caller restores a kept image of exactly these commands itself:
    /// take them as drawn instead of rasterizing them again.
    pub fn accept_layer(&mut self, cache: &mut LayerCache) {
        cache.0 = Some((self.layer_generation, self.signature()));
        self.cmds.clear();
    }

    pub fn finish_cached(&mut self, cache: &mut LayerCache) -> bool {
        let sig = self.signature();
        if cache.0 == Some((self.layer_generation, sig)) {
            self.cmds.clear();
            return false;
        }
        cache.0 = Some((self.layer_generation, sig));
        self.mark_full_damage();
        self.raster_pending();
        true
    }

    /// Draws the pending transient overlay over retained canvas pixels.
    pub fn finish_pending(&mut self) {
        self.mark_full_damage();
        self.raster_pending_single();
        self.last_sig = self.signature();
        self.presented_directly = true;
    }

    /// Rasterizes a small transient menu and reports only its screen damage.
    pub fn finish_pending_region(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.mark_damage_rect(x, y, w, h);
        self.raster_pending_region(x, y, w, h);
        self.last_sig = self.signature();
        self.presented_directly = true;
    }

    /// Finishes a small overlay whose precise rectangles were reported while
    /// drawing it, preserving those bounds for KMS partial updates.
    pub fn finish_pending_damage(&mut self) {
        if self.damage.full {
            // A base-layer change has already made the visible frame dirty.
            // Use the full canvas raster pool for the final overlay as well.
            self.raster_pending();
        } else {
            // Focus strokes already rasterize only their own edge spans. Run
            // them against the retained frame directly; copying a tile-sized
            // scratch rectangle costs more than drawing these few spans.
            self.raster_pending_single();
        }
        self.last_sig = self.signature();
        self.presented_directly = true;
    }

    fn raster_pending(&mut self) {
        let workers = self.threads;
        self.raster_pending_with_workers(workers);
    }

    fn raster_pending_single(&mut self) {
        self.raster_pending_with_workers(1);
    }

    fn raster_pending_region(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.strip_sigs.clear();
        if self
            .cmds
            .iter()
            .any(|cmd| matches!(cmd, Cmd::Clear(_) | Cmd::Backdrop | Cmd::Screen(_, _)))
        {
            self.raster_pending_single();
            return;
        }
        let x0 = ((x + self.offset.0) * self.scale).floor().max(0.0) as i32;
        let y0 = ((y + self.offset.1) * self.scale).floor().max(0.0) as i32;
        let x1 = (((x + w + self.offset.0) * self.scale).ceil() as i32).min(self.pm.width() as i32);
        let y1 =
            (((y + h + self.offset.1) * self.scale).ceil() as i32).min(self.pm.height() as i32);
        if x0 >= x1 || y0 >= y1 {
            self.cmds.clear();
            return;
        }
        let (rw, rh) = ((x1 - x0) as usize, (y1 - y0) as usize);
        let row_bytes = rw * 4;
        self.region_scratch.resize(row_bytes * rh, 0);
        let full_stride = self.pm.width() as usize * 4;
        {
            let src = self.pm.data();
            for row in 0..rh {
                let from = (y0 as usize + row) * full_stride + x0 as usize * 4;
                let to = row * row_bytes;
                self.region_scratch[to..to + row_bytes]
                    .copy_from_slice(&src[from..from + row_bytes]);
            }
        }
        let mut local = Vec::with_capacity(self.cmds.len());
        for cmd in &self.cmds {
            match cmd {
                Cmd::Path {
                    path,
                    color,
                    stroke,
                } => {
                    let Some(path) = path
                        .clone()
                        .transform(Transform::from_translate(-(x0 as f32), -(y0 as f32)))
                    else {
                        continue;
                    };
                    local.push(Cmd::Path {
                        path,
                        color: *color,
                        stroke: stroke.clone(),
                    });
                }
                Cmd::RoundedStroke {
                    x,
                    y,
                    w,
                    h,
                    r,
                    thickness,
                    color,
                } => local.push(Cmd::RoundedStroke {
                    x: *x - x0 as f32,
                    y: *y - y0 as f32,
                    w: *w,
                    h: *h,
                    r: *r,
                    thickness: *thickness,
                    color: *color,
                }),
                Cmd::Spans {
                    x,
                    y,
                    w,
                    h,
                    r,
                    fill,
                } => {
                    let fill = match fill {
                        Fill::Image { src, ox, oy, alpha } => Fill::Image {
                            src: Arc::clone(src),
                            ox: *ox - x0,
                            oy: *oy - y0,
                            alpha: *alpha,
                        },
                        other => other.clone(),
                    };
                    local.push(Cmd::Spans {
                        x: *x - x0,
                        y: *y - y0,
                        w: *w,
                        h: *h,
                        r: *r,
                        fill,
                    });
                }
                Cmd::Glyph { x, y, g, c } => local.push(Cmd::Glyph {
                    x: *x - x0,
                    y: *y - y0,
                    g: Arc::clone(g),
                    c: *c,
                }),
                Cmd::ClipY(rows) => local.push(Cmd::ClipY(
                    rows.map(|(top, bottom)| (top - y0, bottom - y0)),
                )),
                _ => unreachable!(),
            }
        }
        let bands = self.threads.clamp(1, 4).min(rh.max(1));
        if bands == 1 {
            raster(&local, &mut self.region_scratch, rw as i32, 0, None);
        } else {
            let rows = rh.div_ceil(bands);
            let cmds = &local;
            std::thread::scope(|scope| {
                for (i, band) in self.region_scratch.chunks_mut(rows * row_bytes).enumerate() {
                    let y = (i * rows) as i32;
                    scope.spawn(move || raster(cmds, band, rw as i32, y, None));
                }
            });
        }
        let dst = self.pm.data_mut();
        for row in 0..rh {
            let from = row * row_bytes;
            let to = (y0 as usize + row) * full_stride + x0 as usize * 4;
            dst[to..to + row_bytes].copy_from_slice(&self.region_scratch[from..from + row_bytes]);
        }
        self.cmds.clear();
    }

    fn raster_pending_with_workers(&mut self, workers: usize) {
        self.strip_sigs.clear();
        let (w, h) = (self.pm.width() as usize, self.pm.height() as usize);
        let bands = workers.clamp(1, self.threads);
        if bands == 1 {
            let cmds = &self.cmds;
            let bd = self.backdrop.as_deref();
            raster(cmds, self.pm.data_mut(), w as i32, 0, bd);
            self.cmds.clear();
            return;
        }
        let rows = h.div_ceil(bands);
        let cmds = &self.cmds;
        let bd = self.backdrop.as_deref();
        let data = self.pm.data_mut();
        std::thread::scope(|sc| {
            for (i, chunk) in data.chunks_mut(rows * w * 4).enumerate() {
                let y0 = (i * rows) as i32;
                sc.spawn(move || raster(cmds, chunk, w as i32, y0, bd));
            }
        });
        self.cmds.clear();
    }

    /// Gives back the memory of everything that can be made again (cached
    /// backdrops, pictures scaled to their display size): the browser of the
    /// YouTube TV app needs every megabyte of the Pi's 1 GB.
    pub fn trim_caches(&mut self) {
        if self.ambients.is_empty() && self.scaled.is_empty() {
            return;
        }
        self.ambients = Vec::new();
        self.scaled = Vec::new();
        self.region_scratch = Vec::new();
        self.fonts.cache = HashMap::new();
    }

    /// The next frame is drawn even if it looks the same (the screen was given away).
    pub fn invalidate(&mut self) {
        self.last_sig = 0;
        self.strip_sigs.clear();
        self.layer_generation = self.layer_generation.wrapping_add(1);
        self.presented_directly = false;
    }

    /// Apply a small frosted-glass region using the same pixel result returned
    /// by `blurred_region_image` for Home-idle prewarming.
    pub fn blur_region(&mut self, x: f32, y: f32, w: f32, h: f32) {
        if let Some(region) = self.blurred_region_image(x, y, w, h) {
            self.restore_region_image(&region, x, y);
        }
    }

    /// Fast frosted glass for a small region: downsample, box-blur, and expand
    /// into a compact image so the result can be prepared before an OSD opens.
    pub fn blurred_region_image(&self, x: f32, y: f32, w: f32, h: f32) -> Option<Image> {
        const STEP: usize = 4;
        let pw = self.pm.width() as usize;
        let ph = self.pm.height() as usize;
        let x0 = (x * self.scale).floor().max(0.0) as usize;
        let y0 = (y * self.scale).floor().max(0.0) as usize;
        let x1 = ((x + w) * self.scale).ceil().min(pw as f32) as usize;
        let y1 = ((y + h) * self.scale).ceil().min(ph as f32) as usize;
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        let rw = x1 - x0;
        let rh = y1 - y0;
        let gw = rw.div_ceil(STEP);
        let gh = rh.div_ceil(STEP);
        let src = self.pm.data();
        let mut low = vec![[0u16; 4]; gw * gh];
        for gy in 0..gh {
            for gx in 0..gw {
                let ys = gy * STEP;
                let xs = gx * STEP;
                let cell_h = ((gy + 1) * STEP).min(rh) - ys;
                let cell_w = ((gx + 1) * STEP).min(rw) - xs;
                let y_a = ys + (cell_h - 1) / 3;
                let y_b = ys + ((cell_h - 1) * 2) / 3;
                let x_a = xs + (cell_w - 1) / 3;
                let x_b = xs + ((cell_w - 1) * 2) / 3;
                let mut a = [0u32; 4];
                for (yy, xx) in [(y_a, x_a), (y_a, x_b), (y_b, x_a), (y_b, x_b)] {
                    let o = ((y0 + yy) * pw + x0 + xx) * 4;
                    for k in 0..4 {
                        a[k] += src[o + k] as u32;
                    }
                }
                low[gy * gw + gx] = [
                    (a[0] / 4) as u16,
                    (a[1] / 4) as u16,
                    (a[2] / 4) as u16,
                    (a[3] / 4) as u16,
                ];
            }
        }
        let mut blurred = vec![[0u8; 4]; gw * gh];
        for gy in 0..gh {
            for gx in 0..gw {
                let mut a = [0u32; 4];
                let mut n = 0;
                for sy in gy.saturating_sub(1)..=(gy + 1).min(gh - 1) {
                    for sx in gx.saturating_sub(1)..=(gx + 1).min(gw - 1) {
                        let p = low[sy * gw + sx];
                        for k in 0..4 {
                            a[k] += p[k] as u32;
                        }
                        n += 1;
                    }
                }
                blurred[gy * gw + gx] = [
                    (a[0] / n) as u8,
                    (a[1] / n) as u8,
                    (a[2] / n) as u8,
                    (a[3] / n) as u8,
                ];
            }
        }
        let mut region = Pixmap::new(rw as u32, rh as u32)?;
        let out = region.data_mut();
        for yy in 0..rh {
            for xx in 0..rw {
                let p = blurred[(yy / STEP) * gw + xx / STEP];
                let o = (yy * rw + xx) * 4;
                out[o..o + 4].copy_from_slice(&p);
            }
        }
        Some(Arc::new(region))
    }

    /// Text cut with "…" to fit `max` px.
    pub fn text_fit(
        &mut self,
        s: &str,
        x: f32,
        y: f32,
        px: u32,
        w: Weight,
        c: Rgba,
        max: f32,
    ) -> f32 {
        let t = self.ellipsize(s, px, w, max);
        self.text(&t, x, y, px, w, c, Align::Left)
    }

    pub fn ellipsize(&mut self, s: &str, px: u32, w: Weight, max: f32) -> String {
        if self.fonts.width(s, px, w) <= max {
            return s.to_string();
        }
        let ell = self.fonts.width("…", px, w);
        let mut out = String::new();
        let mut acc = 0.0;
        for ch in s.chars() {
            let a = self.fonts.advance(w, ch, px);
            if acc + a + ell > max {
                break;
            }
            acc += a;
            out.push(ch);
        }
        out.trim_end().to_string() + "…"
    }

    /// Word-wrapped paragraph. Returns the height used.
    pub fn paragraph(
        &mut self,
        s: &str,
        x: f32,
        y: f32,
        max_w: f32,
        px: u32,
        w: Weight,
        c: Rgba,
        line_h: f32,
        max_lines: usize,
    ) -> f32 {
        let lines = self.wrap(s, px, w, max_w);
        let n = lines.len().min(max_lines);
        for (i, l) in lines.iter().take(n).enumerate() {
            let l = if i + 1 == n && lines.len() > n {
                self.ellipsize(&(l.clone() + " …"), px, w, max_w)
            } else {
                l.clone()
            };
            self.text(&l, x, y + i as f32 * line_h, px, w, c, Align::Left);
        }
        n as f32 * line_h
    }

    pub fn wrap(&mut self, s: &str, px: u32, w: Weight, max_w: f32) -> Vec<String> {
        let mut lines = Vec::new();
        for para in s.split('\n') {
            let mut cur = String::new();
            for word in para.split_whitespace() {
                let cand = if cur.is_empty() {
                    word.to_string()
                } else {
                    format!("{cur} {word}")
                };
                if self.fonts.width(&cand, px, w) > max_w && !cur.is_empty() {
                    lines.push(std::mem::replace(&mut cur, word.to_string()));
                } else {
                    cur = cand;
                }
            }
            lines.push(cur);
        }
        while lines.last().map(|l| l.is_empty()).unwrap_or(false) {
            lines.pop();
        }
        lines
    }
}

/// What the living backdrop is made of.
#[allow(dead_code)]
#[derive(Clone)]
pub enum Ambient {
    None,
    Image(Image),
    Color(Rgba),
    /// Cached, low-resolution black-and-gold light field for glass surfaces.
    LiquidGold,
}

impl Ambient {
    fn key(&self) -> u64 {
        match self {
            Ambient::None => 0,
            Ambient::Image(img) => {
                let d = img.data();
                let s = (0..16).fold(img.width() as u64 * 31 + img.height() as u64, |acc, k| {
                    acc.wrapping_mul(131)
                        .wrapping_add(d[(d.len() / 17 * (k + 1)) & !3] as u64)
                });
                (img.data().as_ptr() as u64 ^ s) | 1
            }
            Ambient::Color(c) => {
                0x8000_0000_0000_0000 | (c.0 as u64) << 16 | (c.1 as u64) << 8 | c.2 as u64
            }
            Ambient::LiquidGold => 0x8000_0000_4c49_5155,
        }
    }
}

/// Prepare a small blurred ambient source on its background worker.
fn ambient_small(src: &Ambient) -> Option<Pixmap> {
    match src {
        Ambient::Image(img) => {
            let (sw, sh) = (48u32, 27u32);
            let mut sm = Pixmap::new(sw, sh)?;
            let s = (sw as f32 / img.width() as f32).max(sh as f32 / img.height() as f32);
            let dx = (sw as f32 - img.width() as f32 * s) / 2.0;
            let dy = (sh as f32 - img.height() as f32 * s) / 2.0;
            let paint = tiny_skia::PixmapPaint {
                quality: tiny_skia::FilterQuality::Bilinear,
                ..Default::default()
            };
            sm.draw_pixmap(
                0,
                0,
                img.as_ref().as_ref(),
                &paint,
                Transform::from_row(s, 0.0, 0.0, s, dx, dy),
                None,
            );
            box_blur(&mut sm, 3);
            box_blur(&mut sm, 3);
            Some(sm)
        }
        Ambient::Color(c) => {
            // a glow from the top-left corner in that colour
            let mut sm = Pixmap::new(48, 27)?;
            for (i, p) in sm.data_mut().chunks_exact_mut(4).enumerate() {
                let (x, y) = ((i % 48) as f32 / 48.0, (i / 48) as f32 / 27.0);
                let d = ((x * 0.8).powi(2) + y.powi(2)).sqrt();
                let a = ((1.0 - d).clamp(0.0, 1.0) * 255.0) as u32;
                p[0] = div255(c.2 as u32 * a) as u8;
                p[1] = div255(c.1 as u32 * a) as u8;
                p[2] = div255(c.0 as u32 * a) as u8;
                p[3] = a as u8;
            }
            box_blur(&mut sm, 4);
            Some(sm)
        }
        Ambient::LiquidGold => {
            // Three broad golden light fields and one soft reflection create
            // useful variation behind translucent controls. This tiny image
            // is blurred and cached once; idle frames reuse the full backdrop.
            let mut sm = Pixmap::new(48, 27)?;
            for (i, p) in sm.data_mut().chunks_exact_mut(4).enumerate() {
                let (x, y) = ((i % 48) as f32 / 47.0, (i / 48) as f32 / 26.0);
                let glow = |cx: f32, cy: f32, sx: f32, sy: f32| {
                    let dx = (x - cx) / sx;
                    let dy = (y - cy) / sy;
                    (-2.0 * (dx * dx + dy * dy)).exp()
                };
                let upper = 0.38 * glow(0.08, 0.12, 0.52, 0.52);
                let middle = 0.31 * glow(0.78, 0.30, 0.44, 0.42);
                let lower = 0.20 * glow(0.58, 0.88, 0.62, 0.38);
                let center = 0.18 * glow(0.48, 0.55, 0.46, 0.30);
                let reflection_y = 0.19 + x * 0.16;
                let reflection_a =
                    0.15 * (-((y - reflection_y) / 0.035).powi(2)).exp() * (1.0 - x * 0.55);
                let reflection_b =
                    0.20 * (-((y - (0.68 - x * 0.22)) / 0.055).powi(2)).exp() * (0.35 + x * 0.65);
                let liquid_wave_a = 0.40
                    * (-((y - (0.26 + 0.12 * (x * 5.0).sin())) / 0.045).powi(2)).exp()
                    * (0.35 + x * 0.65);
                let liquid_wave_b = 0.29
                    * (-((y - (0.77 - 0.10 * (x * 4.0).sin())) / 0.065).powi(2)).exp()
                    * (1.0 - x * 0.38);
                let alpha = ((upper
                    + middle
                    + lower
                    + center
                    + reflection_a
                    + reflection_b
                    + liquid_wave_a
                    + liquid_wave_b)
                    .clamp(0.0, 0.86)
                    * 255.0) as u32;
                p[0] = div255(78 * alpha) as u8;
                p[1] = div255(175 * alpha) as u8;
                p[2] = div255(232 * alpha) as u8;
                p[3] = alpha as u8;
            }
            box_blur(&mut sm, 1);
            Some(sm)
        }
        Ambient::None => None,
    }
}

/// Backdrop + the tiny blurred source scaled up (bilinear, fixed point) at
/// 55 %, darkened towards the bottom so text stays readable -- one integer
/// pass over the screen, on a background thread.
fn render_ambient(backdrop: &Pixmap, small: &Pixmap) -> Pixmap {
    let (w, h) = (backdrop.width() as usize, backdrop.height() as usize);
    let (sw, sh) = (small.width() as usize, small.height() as usize);
    let sd = small.data();
    let mut out = backdrop.clone();
    // per column: source x and weight (8 bits)
    let cols: Vec<(usize, u32)> = (0..w)
        .map(|x| {
            let fx = ((x as f32 + 0.5) * sw as f32 / w as f32 - 0.5).clamp(0.0, (sw - 1) as f32);
            (fx as usize, ((fx - fx.floor()) * 256.0) as u32)
        })
        .collect();
    let strength = 140u32; // 0.55 * 256
    for (y, row) in out.data_mut().chunks_exact_mut(w * 4).enumerate() {
        let fy = ((y as f32 + 0.5) * sh as f32 / h as f32 - 0.5).clamp(0.0, (sh - 1) as f32);
        let (y0, wy) = (fy as usize, ((fy - fy.floor()) * 256.0) as u32);
        let y1 = (y0 + 1).min(sh - 1);
        let shade = ((40.0 + 150.0 * (y as f32 / h as f32).powf(1.6)) as u32).min(255);
        let inv_shade = 255 - shade;
        for (x, p) in row.chunks_exact_mut(4).enumerate() {
            let (x0, wx) = cols[x];
            let x1 = (x0 + 1).min(sw - 1);
            let at = |xx: usize, yy: usize, c: usize| sd[(yy * sw + xx) * 4 + c] as u32;
            let mut s = [0u32; 4];
            for (c, v) in s.iter_mut().enumerate() {
                let top = at(x0, y0, c) * (256 - wx) + at(x1, y0, c) * wx;
                let bot = at(x0, y1, c) * (256 - wx) + at(x1, y1, c) * wx;
                *v = ((top * (256 - wy) + bot * wy) >> 16) * strength >> 8;
            }
            let inv_a = 255 - s[3].min(255);
            for c in 0..3 {
                let v = s[c] + div255(p[c] as u32 * inv_a);
                let base = if c == 2 { 12 } else { 10 };
                p[c] = (div255(v.min(255) * inv_shade) + div255(base * shade)) as u8;
            }
            p[3] = 255;
        }
    }
    out
}

fn box_blur(pm: &mut Pixmap, r: i32) {
    let (w, h) = (pm.width() as i32, pm.height() as i32);
    for pass in 0..2 {
        let src = pm.data().to_vec();
        let d = pm.data_mut();
        for y in 0..h {
            for x in 0..w {
                let mut acc = [0u32; 4];
                let mut n = 0;
                for k in -r..=r {
                    let (sx, sy) = if pass == 0 {
                        ((x + k).clamp(0, w - 1), y)
                    } else {
                        (x, (y + k).clamp(0, h - 1))
                    };
                    let i = ((sy * w + sx) * 4) as usize;
                    for c in 0..4 {
                        acc[c] += src[i + c] as u32;
                    }
                    n += 1;
                }
                let i = ((y * w + x) * 4) as usize;
                for c in 0..4 {
                    d[i + c] = (acc[c] / n) as u8;
                }
            }
        }
    }
}

/// x/255 without a division.
#[inline(always)]
fn div255(x: u32) -> u32 {
    (x * 257 + 257) >> 16
}

/// Draws the recorded commands into one band of rows [y0, y0 + band height).
fn raster(cmds: &[Cmd], band: &mut [u8], w: i32, y0: i32, backdrop: Option<&Pixmap>) {
    let row_bytes = w as usize * 4;
    let (band_y0, band_y1) = (y0, y0 + (band.len() / row_bytes) as i32);
    if band_y1 == band_y0 {
        return;
    }
    let whole_band = band;
    let mut clip: Option<(i32, i32)> = None;
    for cmd in cmds {
        if let Cmd::ClipY(rows) = cmd {
            clip = *rows;
            continue;
        }
        // the rows of this band the command may touch
        let (y0, y1) = match clip {
            Some((top, bottom)) => (top.max(band_y0), bottom.min(band_y1)),
            None => (band_y0, band_y1),
        };
        if y0 >= y1 {
            continue;
        }
        let band = &mut whole_band
            [(y0 - band_y0) as usize * row_bytes..(y1 - band_y0) as usize * row_bytes];
        let bh = y1 - y0;
        match cmd {
            Cmd::ClipY(_) => {}
            Cmd::Clear(px) => {
                for p in band.chunks_exact_mut(4) {
                    p.copy_from_slice(px);
                }
            }
            Cmd::Backdrop => {
                if let Some(bd) = backdrop {
                    let row = w as usize * 4;
                    let start = y0 as usize * row;
                    band.copy_from_slice(&bd.data()[start..start + band.len()]);
                }
            }
            Cmd::Screen(pm, a) => {
                let row = w as usize * 4;
                let start = y0 as usize * row;
                let src = &pm.data()[start..start + band.len()];
                if *a >= 255 {
                    band.copy_from_slice(src);
                } else if *a > 0 {
                    // crossfade: 8-bit weights, 4 bytes at a time
                    let a = (*a * 256 / 255).min(256);
                    let inv = 256 - a;
                    for (d, s) in band.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                        let dv = u32::from_ne_bytes([d[0], d[1], d[2], d[3]]);
                        let sv = u32::from_ne_bytes([s[0], s[1], s[2], s[3]]);
                        // two channels per multiply (0x00ff00ff masks)
                        let rb = (((sv & 0x00ff_00ff) * a + (dv & 0x00ff_00ff) * inv) >> 8)
                            & 0x00ff_00ff;
                        let ga = ((((sv >> 8) & 0x00ff_00ff) * a
                            + ((dv >> 8) & 0x00ff_00ff) * inv)
                            >> 8)
                            & 0x00ff_00ff;
                        d.copy_from_slice(&(rb | (ga << 8)).to_ne_bytes());
                    }
                }
            }
            Cmd::Path {
                path,
                color,
                stroke,
            } => {
                let b = path.bounds();
                let pad = stroke.as_ref().map(|s| s.width).unwrap_or(0.0) + 2.0;
                if b.bottom() + pad < y0 as f32 || b.top() - pad > y1 as f32 {
                    continue;
                }
                let Some(mut pm) = tiny_skia::PixmapMut::from_bytes(band, w as u32, bh as u32)
                else {
                    continue;
                };
                let mut paint = Paint::default();
                paint.set_color(*color);
                paint.anti_alias = true;
                let t = Transform::from_translate(0.0, -y0 as f32);
                match stroke {
                    Some(s) => pm.stroke_path(path, &paint, s, t, None),
                    None => pm.fill_path(path, &paint, FillRule::Winding, t, None),
                }
            }
            Cmd::RoundedStroke {
                x,
                y,
                w: rw,
                h: rh,
                r,
                thickness,
                color,
            } => {
                rounded_stroke(band, w, y0, y1, *x, *y, *rw, *rh, *r, *thickness, *color);
            }
            Cmd::Spans {
                x,
                y,
                w: bw,
                h: bhh,
                r,
                fill,
            } => span_fill(band, w, y0, y1, *x, *y, *bw, *bhh, *r, fill),
            Cmd::Glyph { x, y, g, c } => {
                let (m, bmp) = (&g.0, &g.1);
                let gh = m.height as i32;
                let gw = m.width as i32;
                if *y + gh <= y0 || *y >= y1 {
                    continue;
                }
                for row in 0..gh {
                    let py = *y + row;
                    if py < y0 || py >= y1 {
                        continue;
                    }
                    for col in 0..gw {
                        let px = *x + col;
                        if px < 0 || px >= w {
                            continue;
                        }
                        let cov = bmp[(row * gw + col) as usize] as u32;
                        if cov == 0 {
                            continue;
                        }
                        let a = div255(cov * c.3 as u32);
                        let i = (((py - y0) * w + px) * 4) as usize;
                        let inv = 255 - a;
                        let d = &mut band[i..i + 4];
                        d[0] = (div255(c.2 as u32 * a) + div255(d[0] as u32 * inv)) as u8;
                        d[1] = (div255(c.1 as u32 * a) + div255(d[1] as u32 * inv)) as u8;
                        d[2] = (div255(c.0 as u32 * a) + div255(d[2] as u32 * inv)) as u8;
                        d[3] = (a + div255(d[3] as u32 * inv)).min(255) as u8;
                    }
                }
            }
        }
    }
}

fn rounded_stroke(
    band: &mut [u8],
    canvas_w: i32,
    y0: i32,
    y1: i32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    radius: f32,
    thickness: f32,
    color: [u8; 4],
) {
    if w <= 0.0 || h <= 0.0 || thickness <= 0.0 || color[3] == 0 {
        return;
    }
    let radius = radius.min(w * 0.5).min(h * 0.5).max(0.0);
    let inner_w = (w - thickness * 2.0).max(0.0);
    let inner_h = (h - thickness * 2.0).max(0.0);
    let inner_r = (radius - thickness)
        .max(0.0)
        .min(inner_w * 0.5)
        .min(inner_h * 0.5);
    let row_start = (y.floor() as i32).max(y0);
    let row_end = ((y + h).ceil() as i32).min(y1);
    let alpha = color[3] as u32;
    let inv = 255 - alpha;
    let blend_run = |row: &mut [u8], from: f32, to: f32| {
        let left = (from.floor() as i32).max(0).min(canvas_w);
        let right = (to.ceil() as i32).max(0).min(canvas_w);
        for px in left..right {
            let i = px as usize * 4;
            for ch in 0..4 {
                row[i + ch] = (color[ch] as u32 + div255(row[i + ch] as u32 * inv)).min(255) as u8;
            }
        }
    };
    let inset = |cy: f32, top: f32, bottom: f32, r: f32| -> f32 {
        let d = if cy < top + r {
            top + r - cy
        } else if cy > bottom - r {
            cy - (bottom - r)
        } else {
            0.0
        };
        if d <= 0.0 || r <= 0.0 {
            0.0
        } else {
            r - (r * r - d * d).max(0.0).sqrt()
        }
    };
    for py in row_start..row_end {
        let cy = py as f32 + 0.5;
        let outer_inset = inset(cy, y, y + h, radius);
        let outer_left = x + outer_inset;
        let outer_right = x + w - outer_inset;
        let row_offset = (py - y0) as usize * canvas_w as usize * 4;
        let row = &mut band[row_offset..row_offset + canvas_w as usize * 4];
        let iy = y + thickness;
        if inner_w > 0.0 && inner_h > 0.0 && cy >= iy && cy < iy + inner_h {
            let inner_inset = inset(cy, iy, iy + inner_h, inner_r);
            let inner_left = x + thickness + inner_inset;
            let inner_right = x + thickness + inner_w - inner_inset;
            blend_run(row, outer_left, inner_left);
            blend_run(row, inner_right, outer_right);
        } else {
            blend_run(row, outer_left, outer_right);
        }
    }
}

/// Blends one premultiplied source pixel (scaled by coverage/256) into d.
#[inline(always)]
fn blend(d: &mut [u8], src: [u32; 4], cov: u32) {
    let sa = (src[3] * cov) >> 8;
    if sa == 0 {
        return;
    }
    if sa >= 255 {
        d[0] = src[0] as u8;
        d[1] = src[1] as u8;
        d[2] = src[2] as u8;
        d[3] = 255;
        return;
    }
    let inv = 255 - sa;
    d[0] = (((src[0] * cov) >> 8) + div255(d[0] as u32 * inv)).min(255) as u8;
    d[1] = (((src[1] * cov) >> 8) + div255(d[1] as u32 * inv)).min(255) as u8;
    d[2] = (((src[2] * cov) >> 8) + div255(d[2] as u32 * inv)).min(255) as u8;
    d[3] = (sa + div255(d[3] as u32 * inv)).min(255) as u8;
}

/// A rounded box filled row by row, clipped to the band, anti-aliased
/// corners. The inside of each row is done in bulk: opaque colours and
/// opaque photos are straight copies.
fn span_fill(
    band: &mut [u8],
    pw: i32,
    y0: i32,
    y1: i32,
    x0: i32,
    by: i32,
    w: i32,
    h: i32,
    r: f32,
    fill: &Fill,
) {
    let rows_from = (y0 - by).max(0);
    let rows_to = (y1 - by).min(h);
    if rows_from >= rows_to || x0 >= pw || x0 + w <= 0 {
        return;
    }
    let hf = h.max(1) as f32;
    for row in rows_from..rows_to {
        let py = by + row;
        let dy = if (row as f32) < r {
            r - row as f32 - 0.5
        } else if (row as f32) >= h as f32 - r {
            row as f32 + 0.5 - (h as f32 - r)
        } else {
            -1.0
        };
        let inset = if dy >= 0.0 {
            r - (r * r - dy * dy).max(0.0).sqrt()
        } else {
            0.0
        };
        let first = inset.floor() as i32;
        let edge = if inset > 0.0 {
            ((1.0 - (inset - inset.floor())) * 256.0) as u32
        } else {
            256
        };
        let (c0, c1) = (first.max(0), w - first);
        if c1 <= c0 {
            continue;
        }
        let line = ((py - y0) * pw * 4) as usize;
        // pixel range on screen; the outermost pixel of a curved row is the anti-aliased edge
        let (sx0, sx1) = (x0 + c0, x0 + c1);
        let aa = inset > 0.0;
        let (in0, in1) = if aa { (sx0 + 1, sx1 - 1) } else { (sx0, sx1) };
        let (in0c, in1c) = (in0.max(0), in1.min(pw));
        match fill {
            Fill::Solid(_) | Fill::VGrad(..) => {
                let p = match fill {
                    Fill::Solid(p) => *p,
                    Fill::VGrad(t, b) => {
                        let k = (row as f32 + 0.5) / hf;
                        let l = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * k) as u32;
                        let a = l(t.3, b.3);
                        [
                            div255(l(t.2, b.2) * a),
                            div255(l(t.1, b.1) * a),
                            div255(l(t.0, b.0) * a),
                            a,
                        ]
                    }
                    _ => unreachable!(),
                };
                if aa {
                    let edges: &[i32] = if sx1 - 1 > sx0 {
                        &[sx0, sx1 - 1]
                    } else {
                        &[sx0]
                    };
                    for &ex in edges {
                        if ex >= 0 && ex < pw {
                            let i = line + ex as usize * 4;
                            blend(&mut band[i..i + 4], p, edge);
                        }
                    }
                }
                if in1c > in0c {
                    let seg = &mut band[line + in0c as usize * 4..line + in1c as usize * 4];
                    if p[3] >= 255 {
                        let px = [p[0] as u8, p[1] as u8, p[2] as u8, 255];
                        for d in seg.chunks_exact_mut(4) {
                            d.copy_from_slice(&px);
                        }
                    } else if p[3] > 0 {
                        let inv = 255 - p[3];
                        for d in seg.chunks_exact_mut(4) {
                            d[0] = (p[0] + div255(d[0] as u32 * inv)) as u8;
                            d[1] = (p[1] + div255(d[1] as u32 * inv)) as u8;
                            d[2] = (p[2] + div255(d[2] as u32 * inv)) as u8;
                            d[3] = (p[3] + div255(d[3] as u32 * inv)).min(255) as u8;
                        }
                    }
                }
            }
            Fill::HGrad(l, r) => {
                let wf = w.max(1) as f32;
                for px in sx0.max(0)..sx1.min(pw) {
                    let k = ((px - x0) as f32 + 0.5) / wf;
                    let li = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * k) as u32;
                    let a = li(l.3, r.3);
                    let p = [
                        div255(li(l.2, r.2) * a),
                        div255(li(l.1, r.1) * a),
                        div255(li(l.0, r.0) * a),
                        a,
                    ];
                    let cov = if aa && (px == sx0 || px == sx1 - 1) {
                        edge
                    } else {
                        256
                    };
                    let i = line + px as usize * 4;
                    blend(&mut band[i..i + 4], p, cov);
                }
            }
            Fill::Image { src, ox, oy, alpha } => {
                let (ox, oy) = (*ox, *oy);
                let sy = py - oy;
                if sy < 0 || sy >= src.height() as i32 {
                    continue;
                }
                let sw = src.width() as i32;
                let srow = &src.data()[(sy * sw * 4) as usize..((sy + 1) * sw * 4) as usize];
                let at = |x: i32| -> Option<[u32; 4]> {
                    let sx = x - ox;
                    if sx < 0 || sx >= sw {
                        return None;
                    }
                    let q = &srow[sx as usize * 4..sx as usize * 4 + 4];
                    Some(if *alpha == 255 {
                        [q[0] as u32, q[1] as u32, q[2] as u32, q[3] as u32]
                    } else {
                        [
                            div255(q[0] as u32 * alpha),
                            div255(q[1] as u32 * alpha),
                            div255(q[2] as u32 * alpha),
                            div255(q[3] as u32 * alpha),
                        ]
                    })
                };
                if aa {
                    let edges: &[i32] = if sx1 - 1 > sx0 {
                        &[sx0, sx1 - 1]
                    } else {
                        &[sx0]
                    };
                    for &ex in edges {
                        if ex >= 0 && ex < pw {
                            if let Some(p) = at(ex) {
                                let i = line + ex as usize * 4;
                                blend(&mut band[i..i + 4], p, edge);
                            }
                        }
                    }
                }
                // inside: clip to the source row too
                let a = in0c.max(ox);
                let b = in1c.min(ox + sw);
                if b <= a {
                    continue;
                }
                let dst = &mut band[line + a as usize * 4..line + b as usize * 4];
                let s = &srow[(a - ox) as usize * 4..(b - ox) as usize * 4];
                if *alpha == 255 {
                    for (d, q) in dst.chunks_exact_mut(4).zip(s.chunks_exact(4)) {
                        if q[3] == 255 {
                            d.copy_from_slice(q);
                        } else if q[3] != 0 {
                            let inv = 255 - q[3] as u32;
                            d[0] = (q[0] as u32 + div255(d[0] as u32 * inv)) as u8;
                            d[1] = (q[1] as u32 + div255(d[1] as u32 * inv)) as u8;
                            d[2] = (q[2] as u32 + div255(d[2] as u32 * inv)) as u8;
                            d[3] = (q[3] as u32 + div255(d[3] as u32 * inv)).min(255) as u8;
                        }
                    }
                } else {
                    for (d, q) in dst.chunks_exact_mut(4).zip(s.chunks_exact(4)) {
                        blend(
                            d,
                            [
                                div255(q[0] as u32 * alpha),
                                div255(q[1] as u32 * alpha),
                                div255(q[2] as u32 * alpha),
                                div255(q[3] as u32 * alpha),
                            ],
                            256,
                        );
                    }
                }
            }
        }
    }
}

/// Eases t (0..1) like CSS ease-out.
pub fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Screenshot of a BGRA premultiplied pixmap as PNG (development / --shot).
pub fn save_png(pm: &Pixmap, path: &str) -> Result<(), String> {
    let mut out = image::RgbaImage::new(pm.width(), pm.height());
    for (o, p) in out.pixels_mut().zip(pm.data().chunks_exact(4)) {
        let a = p[3].max(1) as u16;
        let un = |c: u8| ((c as u16 * 255 + a / 2) / a).min(255) as u8;
        *o = if p[3] == 0 {
            image::Rgba([0, 0, 0, 0])
        } else {
            image::Rgba([un(p[2]), un(p[1]), un(p[0]), p[3]])
        };
    }
    out.save(path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn draw(c: &mut Canvas, x: f32) {
        c.backdrop();
        c.rect(x, 100.0, 300.0, 200.0, 20.0, color::ACCENT);
        c.text(
            "Hallo",
            120.0,
            120.0,
            24,
            Weight::Bold,
            color::WHITE,
            Align::Left,
        );
        c.circle(400.0, 400.0, 10.0, color::WHITE);
    }
    #[test]
    fn unchanged_frames_are_skipped() {
        let mut c = Canvas::new(W, H);
        draw(&mut c, 100.0);
        assert!(c.finish(), "first frame is drawn");
        let px = c.pm.data()[((150 * W + 200) * 4) as usize];
        draw(&mut c, 100.0);
        assert!(!c.finish(), "same frame again: skipped");
        assert_eq!(
            c.pm.data()[((150 * W + 200) * 4) as usize],
            px,
            "picture kept"
        );
        draw(&mut c, 101.0);
        assert!(c.finish(), "moved by a pixel: drawn");
        c.invalidate();
        draw(&mut c, 101.0);
        assert!(c.finish(), "after invalidate: drawn");
    }

    /// A busy little scene: a scrolled list, a selection, a clock.
    fn scene(c: &mut Canvas, scroll: u32, sel: u32, clock: u32) {
        c.backdrop();
        c.text(
            "Titel",
            20.0,
            10.0,
            22,
            Weight::Bold,
            color::TEXT,
            Align::Left,
        );
        c.clip_y(Some((60.0, 300.0)));
        for i in 0..8 {
            let y = 40.0 + i as f32 * 44.0 - scroll as f32 * 9.0;
            c.rect(20.0, y, 500.0, 38.0, 10.0, color::PANEL_SOLID);
            let focused = i == sel;
            c.text(
                &format!("Zeile {i}"),
                34.0,
                y + 9.0,
                16,
                if focused {
                    Weight::Bold
                } else {
                    Weight::Regular
                },
                if focused { color::WHITE } else { color::MUTED },
                Align::Left,
            );
        }
        c.clip_y(None);
        c.stroke_rect(
            540.0,
            60.0 + sel as f32 * 30.0,
            80.0,
            40.0,
            8.0,
            2.0,
            color::ACCENT,
        );
        c.circle(600.0, 330.0, 6.0 + (clock % 2) as f32, color::RED);
        c.vgradient(
            0.0,
            320.0,
            640.0,
            40.0,
            0.0,
            Rgba(0, 0, 0, 0),
            Rgba(0, 0, 0, 200),
        );
        c.text(
            &format!("12:{clock:02}"),
            620.0,
            10.0,
            16,
            Weight::Semi,
            color::TEXT,
            Align::Right,
        );
    }

    #[test]
    fn strips_draw_the_same_picture_as_a_full_pass() {
        // (scroll, selection, clock): steps that change a little, a lot, nothing
        let steps = [
            (0, 0, 0),
            (0, 0, 0),
            (0, 0, 1),
            (0, 1, 1),
            (0, 2, 1),
            (1, 2, 1),
            (2, 3, 2),
            (2, 3, 3),
            (2, 7, 3),
            (0, 0, 4),
            (0, 0, 5),
            (0, 5, 5),
        ];
        for scale in [1.0, 1.5] {
            let mut part = Canvas::new_scaled(640, 360, scale);
            let mut full = Canvas::new_scaled(640, 360, scale);
            let mut partial_frames = 0;
            for (i, (scroll, sel, clock)) in steps.into_iter().enumerate() {
                part.reset_damage();
                scene(&mut part, scroll, sel, clock);
                part.finish();
                partial_frames += usize::from(!part.damage().full && part.damage().len > 0);
                full.invalidate();
                scene(&mut full, scroll, sel, clock);
                full.finish();
                // Where a band's edge crosses a slanted outline tiny-skia
                // may round one of its 16 coverage steps the other way; a
                // real mistake leaves whole rows or shapes wrong.
                let worst = part
                    .pm
                    .data()
                    .iter()
                    .zip(full.pm.data())
                    .map(|(a, b)| a.abs_diff(*b))
                    .max()
                    .unwrap_or(0);
                assert!(
                    worst <= 20,
                    "step {i} at scale {scale}: strips differ from a full pass by {worst}"
                );
            }
            assert!(partial_frames >= 4, "small changes only touch a few strips");
        }
    }

    #[test]
    fn clip_keeps_rows_inside_their_box() {
        let mut c = Canvas::new(64, 64);
        c.clear(color::BLACK);
        c.clip_y(Some((20.0, 40.0)));
        c.rect(0.0, 0.0, 64.0, 64.0, 0.0, color::WHITE);
        c.text("X", 30.0, 12.0, 20, Weight::Bold, color::RED, Align::Left);
        c.clip_y(None);
        c.rect(0.0, 60.0, 64.0, 4.0, 0.0, color::WHITE);
        assert!(c.finish());
        let px = |x: usize, y: usize| c.pm.data()[(y * 64 + x) * 4];
        assert_eq!(px(5, 19), 0, "above the clip stays untouched");
        assert_eq!(px(5, 20), 255);
        assert_eq!(px(5, 39), 255);
        assert_eq!(px(5, 40), 0, "below the clip stays untouched");
        assert_eq!(px(5, 61), 255, "drawing after the clip is lifted is whole");
    }

    #[test]
    fn rounded_stroke_uses_a_transparent_center_and_rounded_corners() {
        let mut c = Canvas::new(80, 60);
        c.clear(Rgba(0, 0, 0, 0));
        c.rounded_stroke_rect(8.0, 8.0, 64.0, 44.0, 14.0, 4.0, color::WHITE);
        assert!(c.finish());
        let alpha = |x: usize, y: usize| c.pm.data()[(y * 80 + x) * 4 + 3];
        assert!(alpha(9, 30) > 0, "the side stroke is present");
        assert_eq!(alpha(40, 30), 0, "the center remains transparent");
        assert_eq!(alpha(8, 8), 0, "the rounded corner stays outside the ring");
    }

    #[test]
    fn ambient_redraw_stops_after_final_transition_frame() {
        let mut c = Canvas::new(16, 16);
        let ambient = Ambient::Color(Rgba::hex(0x2255aa));
        let key = ambient.key();
        c.ambients.push((key, Arc::new(Pixmap::new(1, 1).unwrap())));

        c.ambient(ambient);
        assert!(c.ambient_moving(), "transition needs another frame");

        c.ambient_now.2 = std::time::Instant::now() - std::time::Duration::from_secs(1);
        c.ambient(Ambient::Color(Rgba::hex(0x2255aa)));
        assert!(!c.ambient_moving(), "no redraws after the final frame");
    }

    #[test]
    fn retained_layer_restores_pixels_and_presents_overlay() {
        let mut c = Canvas::new(64, 64);
        let mut cache = LayerCache::default();
        c.clear(color::BLACK);
        c.rect(10.0, 10.0, 40.0, 40.0, 4.0, color::ACCENT);
        assert!(c.finish_cached(&mut cache));
        let base = c.snapshot();

        c.circle(30.0, 3.0, 3.0, color::WHITE);
        c.finish_pending();
        assert!(c.finish(), "the retained overlay must trigger presentation");

        c.clear(color::BLACK);
        c.rect(10.0, 10.0, 40.0, 40.0, 4.0, color::ACCENT);
        assert!(!c.finish_cached(&mut cache), "unchanged base is reused");
        c.restore_focus(&base, 10.0, 10.0, 40.0, 40.0);
        let center = ((3 * 64 + 30) * 4) as usize;
        assert_eq!(
            &c.pm.data()[center..center + 4],
            &base.data()[center..center + 4],
            "old focus pixels are restored locally"
        );
        c.restore_image(&base);
        assert_eq!(
            c.pm.data(),
            base.data(),
            "restoring the base removes old focus pixels"
        );
        c.circle(46.0, 46.0, 5.0, color::WHITE);
        c.finish_pending();
        assert!(
            c.finish(),
            "the next animated overlay must be presented too"
        );
    }

    #[test]
    fn local_overlay_composites_into_retained_frame() {
        let mut c = Canvas::new(32, 32);
        c.clear(color::BLACK);
        assert!(c.finish());
        let mut overlay = Pixmap::new(8, 8).unwrap();
        overlay.data_mut()[((4 * 8 + 4) * 4) as usize..((4 * 8 + 4) * 4 + 4) as usize]
            .copy_from_slice(&[255; 4]);
        c.composite_pixmap(&overlay, 10.0, 10.0, 8.0, 8.0);
        let center = ((14 * 32 + 14) * 4) as usize;
        assert_eq!(&c.pm.data()[center..center + 4], &[255; 4]);
        assert!(c.finish(), "a directly composited layer must be presented");
    }
}

#[cfg(test)]
mod ambient_cost {
    use super::*;
    #[test]
    #[ignore]
    fn ambient_render_time() {
        let mut c = Canvas::new(W, H);
        c.backdrop();
        let bd = c.backdrop.clone().unwrap();
        let t = std::time::Instant::now();
        for i in 0..5 {
            let small = ambient_small(&Ambient::Color(Rgba(200, 30 + i * 10, 40, 255))).unwrap();
            let _ = render_ambient(&bd, &small);
        }
        eprintln!(
            "render_ambient: {:.1} ms",
            t.elapsed().as_secs_f64() * 1000.0 / 5.0
        );
    }
}

#[cfg(test)]
mod strip_cost {
    use super::*;
    /// cargo test --release -p tv-launcher strip_cost -- --ignored --nocapture
    #[test]
    #[ignore]
    fn small_changes_cost_less_than_a_full_frame() {
        let draw = |c: &mut Canvas, clock: u32| {
            c.backdrop();
            for i in 0..9 {
                let y = 100.0 + i as f32 * 64.0;
                c.rect(64.0, y, 1150.0, 56.0, 12.0, Rgba(255, 255, 255, 12));
                c.stroke_rect(64.0, y, 1150.0, 56.0, 12.0, 1.0, color::BORDER);
                c.text_fit(
                    "Eine Zeile mit etwas Text darin, wie in einer Liste",
                    84.0,
                    y + 16.0,
                    18,
                    Weight::Semi,
                    color::TEXT,
                    900.0,
                );
            }
            c.text(
                &format!("12:{clock:02}"),
                1200.0,
                30.0,
                19,
                Weight::Semi,
                color::TEXT,
                Align::Right,
            );
        };
        for (name, full) in [("alles neu", true), ("nur geänderte Streifen", false)] {
            let mut c = Canvas::new_scaled(W, H, 1.5);
            draw(&mut c, 0);
            c.finish();
            let t = std::time::Instant::now();
            let mut bytes = 0u64;
            for k in 1..=60u32 {
                c.reset_damage();
                if full {
                    c.invalidate();
                }
                draw(&mut c, k);
                c.finish();
                let d = c.damage();
                bytes += if d.full {
                    c.pm.data().len() as u64
                } else {
                    d.rects[..d.len]
                        .iter()
                        .map(|r| r.2 as u64 * r.3 as u64 * 4)
                        .sum()
                };
            }
            eprintln!(
                "{name}: {:.2} ms je Bild, {:.1} MB zum Fernseher je Bild",
                t.elapsed().as_secs_f64() * 1000.0 / 60.0,
                bytes as f64 / 60.0 / 1e6
            );
        }
    }
}
