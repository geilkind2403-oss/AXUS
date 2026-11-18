//! Safe wrapper around the C++ picture of the Live TV channel list
//! (`cpp/tvlist_fx.cpp`): the black-gold wave, the glass rows, the gliding
//! focus and the window the running channel plays in.
use std::os::raw::c_void;

extern "C" {
    fn tvl_create(w: i32, h: i32, scale: f32) -> *mut c_void;
    fn tvl_destroy(s: *mut c_void);
    fn tvl_set_hole(s: *mut c_void, x: f32, y: f32, w: f32, h: f32, r: f32);
    fn tvl_set_rows(s: *mut c_void, rects: *const f32, flags: *const u8, n: i32, radius: f32);
    fn tvl_set_text(s: *mut c_void, px: *const u8, stride: i32);
    fn tvl_advance(s: *mut c_void, t: f32);
    fn tvl_render_rect(
        s: *const c_void,
        dst: *mut u8,
        stride: i32,
        y0: i32,
        y1: i32,
        x0: i32,
        x1: i32,
        focus: *const f32,
    );
}

pub struct TvList {
    raw: *mut c_void,
    w: u32,
    h: u32,
    /// The text layer the C++ side reads, owned here (drawn into in place:
    /// no copy of a whole frame per key press).
    text: Option<crate::gfx::Canvas>,
}

// Changed only through &mut self; rendering takes &self and writes disjoint
// rows, so the row workers may share it.
unsafe impl Send for TvList {}
unsafe impl Sync for TvList {}

impl Drop for TvList {
    fn drop(&mut self) {
        unsafe { tvl_destroy(self.raw) }
    }
}

impl TvList {
    pub fn new(w: u32, h: u32, scale: f32) -> Option<TvList> {
        let raw = unsafe { tvl_create(w as i32, h as i32, scale) };
        (!raw.is_null()).then_some(TvList {
            raw,
            w,
            h,
            text: None,
        })
    }

    pub fn size(&self) -> (u32, u32) {
        (self.w, self.h)
    }

    /// The video window (logical px); None: no window.
    pub fn set_hole(&mut self, r: Option<(f32, f32, f32, f32, f32)>) {
        let (x, y, w, h, rr) = r.unwrap_or((0.0, 0.0, 0.0, 0.0, 0.0));
        unsafe { tvl_set_hole(self.raw, x, y, w, h, rr) }
    }

    /// Glass rows (logical) and which one is the playing channel.
    pub fn set_rows(&mut self, rows: &[(f32, f32, f32, f32, bool)], radius: f32) {
        let rects: Vec<f32> = rows.iter().flat_map(|r| [r.0, r.1, r.2, r.3]).collect();
        let flags: Vec<u8> = rows.iter().map(|r| r.4 as u8).collect();
        unsafe {
            tvl_set_rows(
                self.raw,
                rects.as_ptr(),
                flags.as_ptr(),
                rows.len() as i32,
                radius,
            )
        }
    }

    /// The text layer: a full frame, transparent where there is no text.
    /// Takes the text layer back to draw into it; the picture shows no
    /// text until it is put back.
    pub fn take_text(&mut self) -> Option<crate::gfx::Canvas> {
        unsafe { tvl_set_text(self.raw, std::ptr::null(), 0) };
        self.text.take()
    }

    /// The text layer (a frame-sized transparent canvas) to lay on top.
    pub fn put_text(&mut self, c: crate::gfx::Canvas) {
        if c.pm.width() != self.w || c.pm.height() != self.h {
            return;
        }
        // the pixels stay where they are while the canvas is kept here
        unsafe { tvl_set_text(self.raw, c.pm.data().as_ptr(), c.pm.width() as i32 * 4) };
        self.text = Some(c);
    }

    pub fn advance(&mut self, t: f32) {
        unsafe { tvl_advance(self.raw, t) }
    }

    /// The whole frame, rows shared between up to four cores.
    pub fn render(&self, frame: &mut [u8], focus: Option<[f32; 5]>, threads: usize) {
        self.render_rows(frame, 0, self.h as usize, focus, threads);
    }

    /// Only rows [y0, y1) of the frame (physical px), e.g. where the focus
    /// moved while the wave stands still.
    pub fn render_rows(
        &self,
        frame: &mut [u8],
        y0: usize,
        y1: usize,
        focus: Option<[f32; 5]>,
        threads: usize,
    ) {
        self.render_rect(frame, y0, y1, 0, self.w as usize, focus, threads)
    }

    /// Only rows [y0, y1) and columns [x0, x1) of the frame (physical px).
    #[allow(clippy::too_many_arguments)]
    pub fn render_rect(
        &self,
        frame: &mut [u8],
        y0: usize,
        y1: usize,
        x0: usize,
        x1: usize,
        focus: Option<[f32; 5]>,
        threads: usize,
    ) {
        let (x0, x1) = (
            x0.min(self.w as usize) as i32,
            x1.min(self.w as usize) as i32,
        );
        let stride = self.w as usize * 4;
        let (y0, y1) = (y0.min(self.h as usize), y1.min(self.h as usize));
        if y0 >= y1 || frame.len() < stride * self.h as usize {
            return;
        }
        let rows = y1 - y0;
        let raw = self.raw as usize;
        // a strip of rows is not worth waking four threads for
        let threads = if rows < 200 { 1 } else { threads.clamp(1, 4) };
        if threads == 1 {
            // no thread to start for one: draw right here
            let fp = focus
                .as_ref()
                .map(|f| f.as_ptr())
                .unwrap_or(std::ptr::null());
            unsafe {
                tvl_render_rect(
                    raw as *const c_void,
                    frame[stride * y0..].as_mut_ptr(),
                    stride as i32,
                    y0 as i32,
                    y1 as i32,
                    x0,
                    x1,
                    fp,
                );
            }
            return;
        }
        let per = rows.div_ceil(threads);
        std::thread::scope(|sc| {
            for (i, chunk) in frame[stride * y0..stride * y1]
                .chunks_mut(per * stride)
                .enumerate()
            {
                let (a, b) = (y0 + i * per, y0 + i * per + chunk.len() / stride);
                sc.spawn(move || {
                    let fp = focus
                        .as_ref()
                        .map(|f| f.as_ptr())
                        .unwrap_or(std::ptr::null());
                    unsafe {
                        tvl_render_rect(
                            raw as *const c_void,
                            chunk.as_mut_ptr(),
                            stride as i32,
                            a as i32,
                            b as i32,
                            x0,
                            x1,
                            fp,
                        )
                    }
                });
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cost at the Pi's 1080p: cargo test --release -p tv-launcher
    /// tvlist_cost -- --ignored --nocapture
    #[test]
    #[ignore]
    fn tvlist_cost() {
        let (w, h) = (1920u32, 1080u32);
        let mut s = TvList::new(w, h, 1.5).unwrap();
        s.set_hole(Some((64.0, 154.0, 544.0, 306.0, 18.0)));
        let rows: Vec<_> = (0..9)
            .map(|i| (650.0, 154.0 + i as f32 * 55.0, 570.0, 43.0, i == 0))
            .collect();
        s.set_rows(&rows, 15.0);
        let mut text = crate::gfx::Canvas::new_scaled_with_threads(
            crate::gfx::W,
            crate::gfx::H,
            w as f32 / crate::gfx::W as f32,
            1,
        );
        for y in (240..1000).step_by(3) {
            for x in 1000..1700 {
                let i = (y * w as usize + x) * 4;
                text.pm.data_mut()[i..i + 4].copy_from_slice(&[200, 200, 200, 200]);
            }
        }
        s.put_text(text);
        let mut frame = vec![0u8; (w * h * 4) as usize];
        for threads in [1, 4] {
            let n = 60;
            let (mut adv, mut ren) = (0.0, 0.0);
            for i in 0..n {
                let t0 = std::time::Instant::now();
                s.advance(i as f32 / 30.0);
                adv += t0.elapsed().as_secs_f64();
                let t1 = std::time::Instant::now();
                s.render(
                    &mut frame,
                    Some([650.0, 154.0 + (i % 9) as f32 * 55.0, 570.0, 43.0, 15.0]),
                    threads,
                );
                ren += t1.elapsed().as_secs_f64();
            }
            println!(
                "{threads} Thread(s): Welle {:.2} ms, Bild {:.2} ms",
                adv * 1000.0 / n as f64,
                ren * 1000.0 / n as f64
            );
        }
    }

    #[test]
    fn frame_is_wave_with_window_and_text() {
        let (w, h) = (320u32, 180u32);
        let mut s = TvList::new(w, h, 0.25).unwrap();
        s.set_hole(Some((16.0, 40.0, 400.0, 225.0, 18.0)));
        s.set_rows(&[(650.0, 154.0, 570.0, 43.0, true)], 15.0);
        let mut text = crate::gfx::Canvas::new_scaled_with_threads(
            crate::gfx::W,
            crate::gfx::H,
            w as f32 / crate::gfx::W as f32,
            1,
        );
        let i = ((10 * w + 300) * 4) as usize;
        text.pm.data_mut()[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
        s.put_text(text);
        s.advance(3.0);
        let mut one = vec![0u8; (w * h * 4) as usize];
        let mut four = vec![0u8; (w * h * 4) as usize];
        let focus = Some([650.0, 154.0, 570.0, 43.0, 15.0]);
        s.render(&mut one, focus, 1);
        s.render(&mut four, focus, 4);
        assert!(one == four, "rows split between cores give the same frame");
        let px = |x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            [one[i], one[i + 1], one[i + 2], one[i + 3]]
        };
        assert_eq!(px(300, 10), [255, 255, 255, 255], "text on top");
        assert_eq!(
            px(60, 60),
            [0, 0, 0, 0],
            "the video shows through the window"
        );
        assert_eq!(px(250, 170)[3], 255, "the wave is opaque");
        assert!(one
            .chunks_exact(4)
            .all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]));
    }
}
