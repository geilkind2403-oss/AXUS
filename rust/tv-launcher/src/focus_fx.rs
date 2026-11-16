//! Home's gold focus frame, drawn by `cpp/focus_fx.cpp` in one pass over
//! the finished frame and shared between up to four cores. Five tiny-skia
//! layers on one core could not keep up with 60 pictures a second on the Pi.
use crate::gfx::Canvas;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Params {
    pub scale: f32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub r: f32,
    pub reach: f32,
    pub wash: f32,
    pub ring: f32,
    pub glow: f32,
    pub top: f32,
    pub ring_w: f32,
}

extern "C" {
    fn ffx_draw_rows(
        px: *mut u8,
        stride: i32,
        width: i32,
        height: i32,
        row0: i32,
        row1: i32,
        p: *const Params,
    );
}

/// Draws the frame (logical coordinates; `scale` is filled in here) onto
/// what the canvas shows now. Call after the canvas has finished its
/// pending drawing; the area must be marked as damage by the caller.
pub fn draw(c: &mut Canvas, mut p: Params) {
    let (w, h) = (c.pm.width() as usize, c.pm.height() as usize);
    let scale = w as f32 / crate::gfx::W as f32;
    p.scale = scale;
    // the screen fade moves and fades everything drawn through the canvas
    p.x += c.offset.0;
    p.y += c.offset.1;
    let o = c.opacity.clamp(0.0, 1.0);
    p.wash *= o;
    p.ring *= o;
    p.glow *= o;
    p.top *= o;
    let y0 = (((p.y - p.reach) * scale).floor().max(0.0) as usize).min(h);
    let y1 = (((p.y + p.h + p.reach) * scale).ceil().max(0.0) as usize).min(h);
    if y0 >= y1 {
        return;
    }
    let stride = w * 4;
    let rows = y1 - y0;
    let threads = if rows < 120 { 1 } else { 4 };
    let per = rows.div_ceil(threads);
    let data = c.pm.data_mut();
    std::thread::scope(|sc| {
        for (i, chunk) in data[stride * y0..stride * y1]
            .chunks_mut(per * stride)
            .enumerate()
        {
            let a = y0 + i * per;
            let b = a + chunk.len() / stride;
            // each worker gets its own rows; the C++ side indexes the whole
            // frame, so hand it a pointer that starts at row 0
            let base = chunk.as_mut_ptr() as usize - a * stride;
            let p = p;
            let run = move || unsafe {
                ffx_draw_rows(
                    base as *mut u8,
                    stride as i32,
                    w as i32,
                    h as i32,
                    a as i32,
                    b as i32,
                    &p,
                )
            };
            if threads == 1 {
                run();
            } else {
                sc.spawn(run);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> Params {
        Params {
            x: 100.0,
            y: 100.0,
            w: 300.0,
            h: 200.0,
            r: 22.0,
            reach: 9.0,
            wash: 0.1,
            ring: 0.8,
            glow: 0.2,
            top: 0.3,
            ring_w: 2.0,
            ..Default::default()
        }
    }

    #[test]
    fn split_rows_match_one_pass() {
        let mut one = Canvas::new_scaled(crate::gfx::W, crate::gfx::H, 1.0);
        let mut four = Canvas::new_scaled(crate::gfx::W, crate::gfx::H, 1.0);
        for c in [&mut one, &mut four] {
            for (i, p) in c.pm.data_mut().chunks_exact_mut(4).enumerate() {
                p.copy_from_slice(&[(i % 200) as u8, 40, 30, 255]);
            }
        }
        // the 4-core path splits rows; a short frame takes the 1-core path
        draw(&mut four, params());
        let mut p = params();
        p.scale = 1.0;
        unsafe {
            let (w, h) = (one.pm.width() as i32, one.pm.height() as i32);
            ffx_draw_rows(one.pm.data_mut().as_mut_ptr(), w * 4, w, h, 0, h, &p);
        }
        assert!(one.pm.data() == four.pm.data());
        let px = |x: usize, y: usize| {
            let i = (y * crate::gfx::W as usize + x) * 4;
            one.pm.data()[i..i + 4].to_vec()
        };
        // far away untouched, ring golden, everything premultiplied
        assert_eq!(px(10, 10), vec![(10 % 200) as u8, 40, 30, 255]);
        let ring = px(250, 101);
        assert!(ring[2] > ring[0], "gold: more red than blue {ring:?}");
        assert!(one
            .pm
            .data()
            .chunks_exact(4)
            .all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]));
    }
}

#[cfg(test)]
mod bench {
    use super::*;
    use crate::gfx::Rgba;

    /// cargo test --release focus_cost -- --ignored --nocapture
    #[test]
    #[ignore]
    fn focus_cost() {
        // the Pi draws Home at 1920x1080; a frame stretched over two cards
        let (x, y, w, h) = (64.0, 130.0, 900.0, 244.0);
        let mut c = Canvas::new_scaled(crate::gfx::W, crate::gfx::H, 1.5);
        let n = 50;
        let t = std::time::Instant::now();
        for _ in 0..n {
            c.mark_damage_rect(x - 9.0, y - 9.0, w + 18.0, h + 18.0);
            let g = |a: u8| Rgba(226, 187, 116, a);
            c.stroke_rect(x - 5.0, y - 5.0, w + 10.0, h + 10.0, 27.0, 4.0, g(30));
            c.stroke_rect(x - 2.5, y - 2.5, w + 5.0, h + 5.0, 24.5, 3.0, g(60));
            c.rect(x + 1.0, y + 1.0, w - 2.0, h - 2.0, 21.0, g(50));
            c.stroke_rect(x + 0.5, y + 0.5, w - 1.0, h - 1.0, 22.0, 2.0, g(200));
            c.line(&[(x + 22.0, y + 1.5), (x + w - 22.0, y + 1.5)], 1.5, g(120));
            c.finish_pending_damage();
        }
        let old = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
        let t = std::time::Instant::now();
        for _ in 0..n {
            draw(
                &mut c,
                Params {
                    x,
                    y,
                    w,
                    h,
                    r: 22.0,
                    reach: 9.0,
                    wash: 0.2,
                    ring: 0.8,
                    glow: 0.2,
                    top: 0.4,
                    ring_w: 2.0,
                    ..Default::default()
                },
            );
        }
        let new = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
        println!("Fokus bei 1080p: alt {old:.2} ms, neu {new:.2} ms");
    }
}
