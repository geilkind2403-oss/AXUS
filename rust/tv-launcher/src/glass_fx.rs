//! Safe wrapper around the C++ liquid-glass renderer of the fast menu
//! (`cpp/glass_fx.cpp`). Rust draws Home and the menu's text; C++ draws the
//! glass, the tiles and the gliding focus highlight.
use std::os::raw::c_void;

pub const TILE: i32 = 0;
pub const TILE_ACTIVE: i32 = 1;
pub const INSET: i32 = 2;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub r: f32,
    pub kind: i32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32, r: f32, kind: i32) -> Rect {
        Rect {
            x,
            y,
            w,
            h,
            r,
            kind,
        }
    }
}

#[repr(C)]
struct Params {
    width: i32,
    height: i32,
    scale: f32,
    over_video: i32,
    panel: Rect,
}

extern "C" {
    fn glass_create(p: *const Params) -> *mut c_void;
    fn glass_destroy(s: *mut c_void);
    fn glass_set_backdrop(s: *mut c_void, px: *const u8, stride: i32) -> i32;
    fn glass_set_tiles(s: *mut c_void, tiles: *const Rect, n: i32) -> i32;
    fn glass_set_content(s: *mut c_void, px: *const u8, stride: i32) -> i32;
    fn glass_build_begin(s: *mut c_void) -> i32;
    fn glass_build_rows(s: *const c_void, y0: i32, y1: i32);
    fn glass_build_end(s: *mut c_void) -> i32;
    fn glass_render(
        s: *const c_void,
        dst: *mut u8,
        stride: i32,
        focus: *const Rect,
        opacity: f32,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
    );
}

/// The glass of one menu at one size. Not shared between threads.
pub struct Glass {
    raw: *mut c_void,
    width: u32,
    height: u32,
    built: bool,
}

// One owner at a time: built on a worker, then handed to the UI thread.
unsafe impl Send for Glass {}

impl Drop for Glass {
    fn drop(&mut self) {
        unsafe { glass_destroy(self.raw) }
    }
}

impl Glass {
    /// `width` x `height` physical pixels; `panel` in logical pixels.
    pub fn new(
        width: u32,
        height: u32,
        scale: f32,
        over_video: bool,
        panel: Rect,
    ) -> Option<Glass> {
        let p = Params {
            width: width as i32,
            height: height as i32,
            scale,
            over_video: over_video as i32,
            panel,
        };
        let raw = unsafe { glass_create(&p) };
        (!raw.is_null()).then_some(Glass {
            raw,
            width,
            height,
            built: false,
        })
    }

    fn fits(&self, data: &[u8], stride: usize) -> bool {
        stride >= self.width as usize * 4
            && data.len() >= stride * (self.height as usize - 1) + self.width as usize * 4
    }

    /// What is behind the menu (None: the video plane, transparent).
    pub fn set_backdrop(&mut self, px: Option<(&[u8], usize)>) -> bool {
        self.built = false;
        match px {
            Some((data, stride)) if self.fits(data, stride) => unsafe {
                glass_set_backdrop(self.raw, data.as_ptr(), stride as i32) != 0
            },
            Some(_) => false,
            None => unsafe { glass_set_backdrop(self.raw, std::ptr::null(), 0) != 0 },
        }
    }

    pub fn set_tiles(&mut self, tiles: &[Rect]) -> bool {
        self.built = false;
        unsafe { glass_set_tiles(self.raw, tiles.as_ptr(), tiles.len() as i32) != 0 }
    }

    /// Text and icons (transparent, premultiplied BGRA).
    pub fn set_content(&mut self, data: &[u8], stride: usize) -> bool {
        self.fits(data, stride)
            && unsafe { glass_set_content(self.raw, data.as_ptr(), stride as i32) != 0 }
    }

    /// Composes the still glass; the rows are shared by up to four cores.
    pub fn build(&mut self) -> bool {
        self.built = false;
        if unsafe { glass_build_begin(self.raw) } == 0 {
            return false;
        }
        let rows = self.height as usize;
        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .clamp(1, 4);
        let per = rows.div_ceil(threads);
        let raw = self.raw as usize;
        std::thread::scope(|sc| {
            for i in 0..threads {
                let (a, b) = (i * per, ((i + 1) * per).min(rows));
                if a < b {
                    sc.spawn(move || unsafe {
                        glass_build_rows(raw as *const c_void, a as i32, b as i32)
                    });
                }
            }
        });
        self.built = unsafe { glass_build_end(self.raw) } != 0;
        self.built
    }

    pub fn built(&self) -> bool {
        self.built
    }

    /// Draws the logical rectangle `area` of the menu into `dst` (the
    /// region's top left pixel and the frame's row stride).
    pub fn render(
        &self,
        dst: &mut [u8],
        stride: usize,
        focus: Option<Rect>,
        opacity: f32,
        area: (f32, f32, f32, f32),
    ) {
        if !self.built || !self.fits(dst, stride) {
            return;
        }
        let f = focus
            .as_ref()
            .map(|r| r as *const Rect)
            .unwrap_or(std::ptr::null());
        unsafe {
            glass_render(
                self.raw,
                dst.as_mut_ptr(),
                stride as i32,
                f,
                opacity,
                area.0,
                area.1,
                area.0 + area.2,
                area.1 + area.3,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = 200;
    const H: u32 = 160;

    fn glass(video: bool) -> Glass {
        let mut g = Glass::new(
            W,
            H,
            1.0,
            video,
            Rect::new(20.0, 20.0, 160.0, 120.0, 16.0, TILE),
        )
        .unwrap();
        let mut back = vec![0u8; (W * H * 4) as usize];
        for (i, p) in back.chunks_exact_mut(4).enumerate() {
            let x = (i as u32 % W) as u8;
            p.copy_from_slice(&[x, 255 - x, 90, 255]);
        }
        g.set_backdrop((!video).then_some((&back[..], W as usize * 4)));
        g.set_tiles(&[
            Rect::new(40.0, 40.0, 50.0, 40.0, 10.0, TILE),
            Rect::new(110.0, 40.0, 50.0, 40.0, 10.0, TILE_ACTIVE),
        ]);
        let mut text = vec![0u8; (W * H * 4) as usize];
        let i = ((60 * W + 60) * 4) as usize;
        text[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
        assert!(g.set_content(&text, W as usize * 4));
        assert!(g.build());
        g
    }

    fn px(buf: &[u8], x: u32, y: u32) -> [u8; 4] {
        let i = ((y * W + x) * 4) as usize;
        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
    }

    #[test]
    fn region_update_matches_full_render() {
        for video in [false, true] {
            let g = glass(video);
            let focus = Some(Rect::new(40.0, 40.0, 50.0, 40.0, 10.0, TILE));
            let mut full = vec![7u8; (W * H * 4) as usize];
            g.render(
                &mut full,
                W as usize * 4,
                focus,
                1.0,
                (0.0, 0.0, W as f32, H as f32),
            );
            // drawn in two pieces: the same pixels
            let mut parts = vec![7u8; (W * H * 4) as usize];
            g.render(
                &mut parts,
                W as usize * 4,
                focus,
                1.0,
                (0.0, 0.0, W as f32, 70.0),
            );
            g.render(
                &mut parts,
                W as usize * 4,
                focus,
                1.0,
                (0.0, 70.0, W as f32, 90.0),
            );
            assert!(full == parts, "video {video}");
            // text over everything, premultiplied everywhere
            assert_eq!(px(&full, 60, 60), [255, 255, 255, 255]);
            assert!(full
                .chunks_exact(4)
                .all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]));
            if video {
                assert_eq!(
                    px(&full, 1, 1)[3],
                    0,
                    "video stays visible outside the glass"
                );
                assert!(px(&full, 100, 120)[3] > 150, "tinted glass");
            } else {
                // far outside the panel: Home untouched
                assert_eq!(px(&full, 1, 1), [1, 254, 90, 255]);
            }
            // the highlight changes its tile only
            let mut none = vec![7u8; (W * H * 4) as usize];
            g.render(
                &mut none,
                W as usize * 4,
                None,
                1.0,
                (0.0, 0.0, W as f32, H as f32),
            );
            assert_ne!(px(&full, 45, 75), px(&none, 45, 75));
            assert_eq!(px(&full, 150, 125), px(&none, 150, 125));
        }
    }

    #[test]
    fn opening_fades_from_the_backdrop() {
        let g = glass(false);
        let mut buf = vec![0u8; (W * H * 4) as usize];
        g.render(
            &mut buf,
            W as usize * 4,
            None,
            0.0,
            (0.0, 0.0, W as f32, H as f32),
        );
        let x = 100u32;
        assert_eq!(px(&buf, x, 80), [x as u8, 255 - x as u8, 90, 255]);
    }
}
