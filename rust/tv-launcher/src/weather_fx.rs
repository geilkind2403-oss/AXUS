//! Safe wrapper around the C++ weather renderer (`cpp/weather_fx.cpp`).
//!
//! The C++ side draws pixels only: the still sky, the animated weather
//! (clouds, rain, snow, fog, stars, lightning, sun rays) and the animated
//! hero icon. Text, data, input and the screen stack stay in Rust.
use crate::gfx::Image;
use std::os::raw::c_void;
use tiny_skia::Pixmap;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub width: i32,
    pub height: i32,
    pub band_h: i32,
    pub scale: f32,
    pub sky: i32,
    pub day: i32,
    pub intensity: f32,
    pub wind: f32,
    pub golden: f32,
    pub sky_top: u32,
    pub sky_bottom: u32,
    pub seed: u32,
    pub icon_x: f32,
    pub icon_y: f32,
    pub icon_size: f32,
    /// x0, y0, x1, y1 (physical): calmer weather behind free-floating text
    pub scrim: [f32; 4],
}

extern "C" {
    fn wfx_create(p: *const Params) -> *mut c_void;
    fn wfx_destroy(s: *mut c_void);
    fn wfx_render_sky_rows(s: *const c_void, dst: *mut u8, stride: i32, y0: i32, y1: i32);
    #[cfg(test)]
    fn wfx_render_sky_ref(s: *const c_void, dst: *mut u8, stride: i32);
    fn wfx_set_layers(
        s: *mut c_void,
        sky: *const u8,
        sky_stride: i32,
        ui: *const u8,
        ui_stride: i32,
    ) -> i32;
    fn wfx_advance(s: *mut c_void, t: f32);
    fn wfx_render_rows(s: *const c_void, dst: *mut u8, stride: i32, y0: i32, y1: i32);
    fn wfx_draw_icon(
        s: *const c_void,
        dst: *mut u8,
        stride: i32,
        w: i32,
        h: i32,
        cx: f32,
        cy: f32,
        t: f32,
    );
}

/// One weather scene (one condition at one size).
pub struct Scene {
    raw: *mut c_void,
    params: Params,
    /// The still sky, kept alive while the C++ side reads it.
    sky: Option<Image>,
}

// The scene is only changed through &mut self; the render calls take &self,
// read the scene and write disjoint rows, so sharing &Scene between the
// row workers is sound.
unsafe impl Send for Scene {}
unsafe impl Sync for Scene {}

impl Drop for Scene {
    fn drop(&mut self) {
        unsafe { wfx_destroy(self.raw) }
    }
}

/// Splits `rows` of a BGRA buffer between `threads` workers, each getting
/// its own rows (the Pi 3 has four cores).
fn par_rows(
    data: &mut [u8],
    stride: usize,
    y0: usize,
    y1: usize,
    threads: usize,
    f: impl Fn(&mut [u8], usize, usize) + Sync,
) {
    let rows = y1.saturating_sub(y0);
    if rows == 0 {
        return;
    }
    let band = &mut data[y0 * stride..y1 * stride];
    let threads = threads.clamp(1, 4).min(rows);
    if threads == 1 {
        f(band, y0, y1);
        return;
    }
    let per = rows.div_ceil(threads);
    std::thread::scope(|sc| {
        for (i, chunk) in band.chunks_mut(per * stride).enumerate() {
            let a = y0 + i * per;
            let b = a + chunk.len() / stride;
            let f = &f;
            sc.spawn(move || f(chunk, a, b));
        }
    });
}

impl Scene {
    pub fn new(p: Params) -> Option<Scene> {
        let raw = unsafe { wfx_create(&p) };
        (!raw.is_null()).then_some(Scene {
            raw,
            params: p,
            sky: None,
        })
    }

    /// The still sky for the whole frame.
    pub fn render_sky(&self, threads: usize) -> Option<Image> {
        let (w, h) = (self.params.width as u32, self.params.height as u32);
        let mut pm = Pixmap::new(w, h)?;
        let stride = w as usize * 4;
        let raw = self.raw as usize;
        par_rows(
            pm.data_mut(),
            stride,
            0,
            h as usize,
            threads,
            |rows, a, b| unsafe {
                wfx_render_sky_rows(
                    raw as *const c_void,
                    rows.as_mut_ptr(),
                    stride as i32,
                    a as i32,
                    b as i32,
                )
            },
        );
        Some(std::sync::Arc::new(pm))
    }

    /// The sky behind and the UI layer (`band_h` rows, transparent) in front.
    /// The UI is folded into the scene; only the sky is kept.
    pub fn set_layers(&mut self, sky: Image, ui: &Image) -> bool {
        let p = &self.params;
        if sky.width() != p.width as u32
            || sky.height() != p.height as u32
            || ui.width() != p.width as u32
            || ui.height() < p.band_h as u32
        {
            return false;
        }
        let ok = unsafe {
            wfx_set_layers(
                self.raw,
                sky.data().as_ptr(),
                sky.width() as i32 * 4,
                ui.data().as_ptr(),
                ui.width() as i32 * 4,
            )
        } != 0;
        self.sky = ok.then_some(sky);
        ok
    }

    pub fn ready(&self) -> bool {
        self.sky.is_some()
    }

    pub fn advance(&mut self, t: f32) {
        unsafe { wfx_advance(self.raw, t) }
    }

    /// Composes the animated band straight into the frame.
    pub fn render_band(&self, frame: &mut Pixmap, threads: usize) {
        if !self.ready() || frame.width() != self.params.width as u32 {
            return;
        }
        let stride = frame.width() as usize * 4;
        let rows = (self.params.band_h as usize).min(frame.height() as usize);
        let raw = self.raw as usize;
        par_rows(
            frame.data_mut(),
            stride,
            0,
            rows,
            threads,
            |chunk, a, b| unsafe {
                wfx_render_rows(
                    raw as *const c_void,
                    chunk.as_mut_ptr(),
                    stride as i32,
                    a as i32,
                    b as i32,
                )
            },
        );
    }

    /// The hero icon alone (transparent), `size` physical pixels square.
    pub fn icon_sprite(&self, size: u32, t: f32) -> Option<Image> {
        let mut pm = Pixmap::new(size, size)?;
        let c = size as f32 * 0.5;
        unsafe {
            wfx_draw_icon(
                self.raw,
                pm.data_mut().as_mut_ptr(),
                size as i32 * 4,
                size as i32,
                size as i32,
                c,
                c,
                t,
            )
        };
        Some(std::sync::Arc::new(pm))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(sky: i32, day: bool) -> Params {
        Params {
            width: 320,
            height: 180,
            band_h: 90,
            scale: 0.25,
            sky,
            day: day as i32,
            intensity: 0.8,
            wind: 18.0,
            golden: 0.3,
            sky_top: 0x276aa4,
            sky_bottom: 0x101d38,
            seed: 7,
            icon_x: 40.0,
            icon_y: 58.0,
            icon_size: 28.0,
            scrim: [4.0, 4.0, 120.0, 30.0],
        }
    }

    fn ui_layer(w: u32, h: u32) -> Image {
        let mut pm = Pixmap::new(w, h).unwrap();
        // an opaque white box and a half-transparent one
        for y in 10..20 {
            for x in 200..240 {
                let i = (y * w as usize + x) * 4;
                pm.data_mut()[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
            for x in 250..300 {
                let i = (y * w as usize + x) * 4;
                pm.data_mut()[i..i + 4].copy_from_slice(&[0, 0, 64, 128]);
            }
        }
        std::sync::Arc::new(pm)
    }

    #[test]
    fn fast_sky_matches_plain_sky() {
        for sky in 0..8 {
            for day in [true, false] {
                for golden in [0.0, 0.6] {
                    let mut p = params(sky, day);
                    p.golden = golden;
                    p.width = 1280;
                    p.height = 720;
                    p.band_h = 430;
                    p.scale = 1.0;
                    p.icon_x = 1000.0;
                    p.icon_y = 120.0;
                    let s = Scene::new(p).expect("scene");
                    let fast = s.render_sky(1).unwrap();
                    let mut plain = Pixmap::new(1280, 720).unwrap();
                    unsafe { wfx_render_sky_ref(s.raw, plain.data_mut().as_mut_ptr(), 1280 * 4) };
                    let worst = fast
                        .data()
                        .iter()
                        .zip(plain.data())
                        .map(|(a, b)| (*a as i32 - *b as i32).abs())
                        .max()
                        .unwrap();
                    assert!(
                        worst <= 1,
                        "sky {sky} day {day} golden {golden}: differs by {worst}"
                    );
                }
            }
        }
    }

    #[test]
    fn every_sky_renders_and_threads_agree() {
        for sky in 0..8 {
            for day in [true, false] {
                let mut s = Scene::new(params(sky, day)).expect("scene");
                let still = s.render_sky(1).unwrap();
                assert!(
                    still.data() == s.render_sky(3).unwrap().data(),
                    "sky {sky} rows split"
                );
                assert!(
                    still.data().chunks_exact(4).all(|p| p[3] == 255),
                    "sky is opaque"
                );
                let ui = ui_layer(320, 90);
                assert!(s.set_layers(still.clone(), &ui));
                for t in [0.0, 1.7, 9.31, 64.0] {
                    s.advance(t);
                    let mut one = Pixmap::new(320, 180).unwrap();
                    let mut four = Pixmap::new(320, 180).unwrap();
                    s.render_band(&mut one, 1);
                    s.render_band(&mut four, 4);
                    assert!(
                        one.data() == four.data(),
                        "sky {sky} day {day} t {t}: split rows differ"
                    );
                    let px = |x: usize, y: usize| {
                        let i = (y * 320 + x) * 4;
                        [
                            one.data()[i],
                            one.data()[i + 1],
                            one.data()[i + 2],
                            one.data()[i + 3],
                        ]
                    };
                    // the opaque UI is on top of all weather
                    assert_eq!(px(220, 15), [255, 255, 255, 255]);
                    // below the band nothing is touched
                    assert_eq!(px(5, 120), [0, 0, 0, 0]);
                    assert_eq!(px(5, 50)[3], 255);
                }
            }
        }
    }

    #[test]
    fn icon_is_drawn_and_transparent_around() {
        for sky in 0..8 {
            let s = Scene::new(params(sky, true)).unwrap();
            let icon = s.icon_sprite(64, 0.5).unwrap();
            let opaque = icon.data().chunks_exact(4).filter(|p| p[3] > 200).count();
            assert!(opaque > 100, "sky {sky}: icon has {opaque} solid pixels");
            assert_eq!(icon.data()[3], 0, "corner stays clear");
            // premultiplied: no channel above alpha
            assert!(icon
                .data()
                .chunks_exact(4)
                .all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]));
        }
    }

    /// Frame cost at the Pi's 1080p:  cargo test --release -p tv-launcher
    /// weather_fx_cost -- --ignored --nocapture
    #[test]
    #[ignore]
    fn weather_fx_cost() {
        let names = [
            "klar", "wolkig", "bedeckt", "nebel", "niesel", "regen", "schnee", "gewitter",
        ];
        for (sky, name) in names.iter().enumerate() {
            let p = Params {
                width: 1920,
                height: 1080,
                band_h: 525,
                scale: 1.5,
                icon_x: 231.0,
                icon_y: 348.0,
                icon_size: 177.0,
                intensity: 1.0,
                scrim: [96.0, 60.0, 495.0, 186.0],
                ..params(sky as i32, true)
            };
            let t0 = std::time::Instant::now();
            let mut s = Scene::new(p).unwrap();
            let sky_img = s.render_sky(4).unwrap();
            let make = t0.elapsed().as_secs_f64() * 1000.0;
            // a UI layer like the real one: glass cards over a third of the band
            let mut ui = Pixmap::new(1920, 525).unwrap();
            for y in 213..498 {
                for x in 96..1806 {
                    let i = (y * 1920 + x) * 4;
                    ui.data_mut()[i..i + 4].copy_from_slice(&[24, 24, 24, 24]);
                }
            }
            s.set_layers(sky_img, &std::sync::Arc::new(ui));
            let mut frame = Pixmap::new(1920, 1080).unwrap();
            for threads in [1, 4] {
                let n = 120;
                let t0 = std::time::Instant::now();
                for i in 0..n {
                    s.advance(i as f32 / 60.0);
                    s.render_band(&mut frame, threads);
                }
                let ms = t0.elapsed().as_secs_f64() * 1000.0 / n as f64;
                println!("{name:9} {threads} Thread(s): {ms:5.2} ms/Bild   (Szene {make:.1} ms)");
            }
        }
    }

    #[test]
    fn bad_params_are_refused() {
        let mut p = params(0, true);
        p.band_h = 999;
        assert!(Scene::new(p).is_none());
        p.band_h = 0;
        assert!(Scene::new(p).is_none());
    }
}
