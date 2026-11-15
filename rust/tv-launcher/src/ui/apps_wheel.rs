use super::apps::AppAssets;
use crate::gfx::{Align, Canvas, Rgba, Weight};
use std::ffi::c_void;
use std::slice;

#[repr(C)]
#[derive(Default)]
pub(super) struct Animation {
    pub(super) rotation: f32,
    target: f32,
    elapsed: f32,
    last_focus: u32,
    initialized: u32,
    moving: u32,
}

impl Animation {
    pub fn moving(&self) -> bool {
        self.moving != 0
    }

    pub fn item_positions(&self) -> [(f32, f32); 5] {
        let step = std::f32::consts::TAU / 5.0;
        std::array::from_fn(|i| {
            let angle = -std::f32::consts::FRAC_PI_2 + i as f32 * step - self.rotation;
            (640.0 + angle.cos() * 164.0, 370.0 + angle.sin() * 164.0)
        })
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct TextView {
    data: *const u8,
    size: usize,
}

impl TextView {
    fn new(text: &str) -> Self {
        Self {
            data: text.as_ptr(),
            size: text.len(),
        }
    }
}

#[repr(C)]
struct Input {
    focus: u32,
    count: u32,
    pass: u32,
    dt: f32,
    launch: f32,
    entry: f32,
    detail_color: u32,
    title: TextView,
    detail: TextView,
}

type Emit = unsafe extern "C" fn(
    *mut c_void,
    u32,
    f32,
    f32,
    f32,
    f32,
    f32,
    f32,
    u32,
    u32,
    u32,
    *const u8,
    usize,
);

unsafe extern "C" {
    fn tv_apps_wheel_draw(
        input: *const Input,
        animation: *mut Animation,
        ctx: *mut c_void,
        emit: Emit,
    );
}

pub(super) fn draw_full(
    canvas: &mut Canvas,
    animation: &mut Animation,
    focus: usize,
    title: &str,
    detail: &str,
    detail_color: Rgba,
    launch: Option<f32>,
    entry: Option<f32>,
    assets: Option<&AppAssets>,
) {
    draw(
        canvas,
        animation,
        focus,
        title,
        detail,
        detail_color,
        launch,
        entry,
        assets,
        0,
    );
}

pub(super) fn draw_static(
    canvas: &mut Canvas,
    animation: &mut Animation,
    focus: usize,
    title: &str,
    detail: &str,
    detail_color: Rgba,
    assets: Option<&AppAssets>,
) {
    draw(
        canvas,
        animation,
        focus,
        title,
        detail,
        detail_color,
        None,
        None,
        assets,
        1,
    );
}

pub(super) fn draw_dynamic(
    canvas: &mut Canvas,
    animation: &mut Animation,
    focus: usize,
    title: &str,
    detail: &str,
    detail_color: Rgba,
    launch: Option<f32>,
    assets: Option<&AppAssets>,
) {
    // Advance the inexpensive wheel state while redrawing only the moving
    // icons and focus. The retained glass/background stays untouched.
    draw(
        canvas,
        animation,
        focus,
        title,
        detail,
        detail_color,
        launch,
        None,
        assets,
        2,
    );
}

fn draw(
    canvas: &mut Canvas,
    animation: &mut Animation,
    focus: usize,
    title: &str,
    detail: &str,
    detail_color: Rgba,
    launch: Option<f32>,
    entry: Option<f32>,
    assets: Option<&AppAssets>,
    pass: u32,
) {
    let input = Input {
        focus: focus as u32,
        count: 5,
        pass,
        dt: crate::ui::frame_dt(),
        launch: launch.unwrap_or(-1.0),
        entry: entry.unwrap_or(-1.0),
        detail_color: pack(detail_color),
        title: TextView::new(title),
        detail: TextView::new(detail),
    };
    let bridge = Bridge { canvas, assets };
    unsafe {
        tv_apps_wheel_draw(
            &input,
            animation,
            &bridge as *const Bridge as *mut c_void,
            emit_command,
        );
    }
}

fn pack(color: Rgba) -> u32 {
    (color.0 as u32) << 24 | (color.1 as u32) << 16 | (color.2 as u32) << 8 | color.3 as u32
}

struct Bridge<'a> {
    canvas: &'a mut Canvas,
    assets: Option<&'a AppAssets>,
}

unsafe extern "C" fn emit_command(
    ctx: *mut c_void,
    kind: u32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    radius: f32,
    stroke: f32,
    rgba: u32,
    arg: u32,
    px: u32,
    bytes: *const u8,
    size: usize,
) {
    if ctx.is_null() {
        return;
    }
    let bridge = &mut *(ctx as *mut Bridge);
    let c = &mut *bridge.canvas;
    let color = Rgba(
        (rgba >> 24) as u8,
        (rgba >> 16) as u8,
        (rgba >> 8) as u8,
        rgba as u8,
    );
    match kind {
        0 => c.circle(x, y, w, color),
        1 => c.rect(x, y, w, h, radius, color),
        2 => c.line(&[(x, y), (w, h)], stroke, color),
        3 | 4 if !bytes.is_null() => {
            let text = std::str::from_utf8_unchecked(slice::from_raw_parts(bytes, size));
            let weight = if kind == 3 { arg >> 8 } else { arg };
            let weight = match weight {
                1 => Weight::Semi,
                2 => Weight::Bold,
                _ => Weight::Regular,
            };
            if kind == 3 {
                let align = match arg & 0xff {
                    1 => Align::Center,
                    2 => Align::Right,
                    _ => Align::Left,
                };
                c.text(text, x, y, px, weight, color, align);
            } else {
                c.text_fit(text, x, y, px, weight, color, w);
            }
        }
        5 => draw_asset(c, bridge.assets, arg, x, y, w, h, radius),
        _ => {}
    }
}

fn draw_asset(
    canvas: &mut Canvas,
    assets: Option<&AppAssets>,
    index: u32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    radius: f32,
) {
    if index == 100 {
        if let Some(image) = assets.and_then(|a| a.glass.as_ref()) {
            canvas.image_fit(image, x, y, w, h, radius, false, 1.0);
        } else {
            canvas.circle(640.0, 370.0, 239.0, Rgba(235, 224, 198, 23));
            canvas.circle(640.0, 370.0, 231.0, Rgba(230, 217, 184, 42));
        }
        return;
    }
    if index == 101 {
        if let Some(image) = assets.and_then(|a| a.logo_glow.as_ref()) {
            canvas.image_fit(image, x, y, w, h, radius, false, 1.0);
        }
        return;
    }
    if index == 102 {
        if let Some(image) = assets.and_then(|a| a.center_well.as_ref()) {
            canvas.image_fit(image, x, y, w, h, radius, false, 1.0);
        }
        return;
    }
    if index == 4 {
        let cx = x + w * 0.5;
        let cy = y + h * 0.5;
        canvas.text(
            "ARTE",
            cx,
            cy - h * 0.12,
            (w * 0.27) as u32,
            Weight::Bold,
            Rgba(245, 244, 240, 255),
            Align::Center,
        );
        return;
    }
    if let Some(image) = assets
        .and_then(|a| a.icons.get(index as usize))
        .and_then(Option::as_ref)
    {
        canvas.image_fit(image, x, y, w, h, radius, false, 1.0);
    } else {
        canvas.circle(x + w * 0.5, y + h * 0.5, w * 0.42, Rgba(36, 38, 44, 235));
    }
}
