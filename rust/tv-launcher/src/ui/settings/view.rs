use super::section_icon;
use crate::gfx::{Align, Canvas, Rgba, Weight};
use std::ffi::c_void;
use std::slice;

#[repr(C)]
#[derive(Default)]
pub(super) struct Animation {
    row_y: f32,
    scroll_y: f32,
    initialized: u32,
    moving: u32,
}

impl Animation {
    pub fn moving(&self) -> bool {
        self.moving != 0
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct TextView {
    data: *const u8,
    size: usize,
}
impl TextView {
    fn new(value: &str) -> Self {
        Self {
            data: value.as_ptr(),
            size: value.len(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct RowView {
    label: TextView,
    value: TextView,
}

#[repr(C)]
struct Input {
    section: u32,
    item: u32,
    inside: u32,
    row_count: u32,
    dt: f32,
    title: TextView,
    hint: TextView,
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
    u32,
    u32,
    *const u8,
    usize,
);

unsafe extern "C" {
    fn tv_settings_draw_shell(
        input: *const Input,
        animation: *mut Animation,
        ctx: *mut c_void,
        emit: Emit,
    );
    fn tv_settings_draw_rows(
        input: *const Input,
        animation: *const Animation,
        rows: *const RowView,
        ctx: *mut c_void,
        emit: Emit,
    );
}

pub fn draw_shell(
    canvas: &mut Canvas,
    animation: &mut Animation,
    section: usize,
    item: usize,
    inside: bool,
    row_count: usize,
    title: &str,
    hint: &str,
) {
    let input = Input {
        section: section as u32,
        item: item as u32,
        inside: u32::from(inside),
        row_count: row_count.min(u32::MAX as usize) as u32,
        dt: crate::ui::frame_dt(),
        title: TextView::new(title),
        hint: TextView::new(hint),
    };
    unsafe {
        tv_settings_draw_shell(
            &input,
            animation,
            canvas as *mut Canvas as *mut c_void,
            emit_command,
        );
    }
}

pub fn draw_rows(
    canvas: &mut Canvas,
    animation: &Animation,
    section: usize,
    item: usize,
    inside: bool,
    rows: &[(String, String)],
) {
    // The panel displays eight rows at a time. A bounded stack buffer avoids
    // allocating and freeing a bridge vector on every 60 Hz frame.
    let mut native_rows = [RowView::default(); 64];
    let count = rows.len().min(native_rows.len());
    for (dst, (label, value)) in native_rows[..count].iter_mut().zip(rows.iter()) {
        *dst = RowView {
            label: TextView::new(label),
            value: TextView::new(value),
        };
    }
    let input = Input {
        section: section as u32,
        item: item as u32,
        inside: u32::from(inside),
        row_count: count as u32,
        dt: 0.0,
        title: TextView::new(""),
        hint: TextView::new(""),
    };
    unsafe {
        tv_settings_draw_rows(
            &input,
            animation,
            native_rows.as_ptr(),
            canvas as *mut Canvas as *mut c_void,
            emit_command,
        );
    }
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
    px: u32,
    weight: u32,
    align: u32,
    icon: u32,
    bytes: *const u8,
    size: usize,
) {
    if ctx.is_null() {
        return;
    }
    let canvas = &mut *(ctx as *mut Canvas);
    let color = Rgba(
        (rgba >> 24) as u8,
        (rgba >> 16) as u8,
        (rgba >> 8) as u8,
        rgba as u8,
    );
    let font_weight = match weight {
        1 => Weight::Semi,
        2 => Weight::Bold,
        _ => Weight::Regular,
    };
    let text_align = match align {
        1 => Align::Center,
        2 => Align::Right,
        _ => Align::Left,
    };
    match kind {
        0 => canvas.rect(x, y, w, h, radius, color),
        1 => canvas.stroke_rect(x, y, w, h, radius, stroke, color),
        2 => canvas.line(&[(x, y), (w, h)], stroke, color),
        3 | 6 if !bytes.is_null() => {
            let text = std::str::from_utf8_unchecked(slice::from_raw_parts(bytes, size));
            if kind == 6 {
                canvas.text_fit(text, x, y, px, font_weight, color, w);
            } else {
                canvas.text(text, x, y, px, font_weight, color, text_align);
            }
        }
        4 => section_icon(canvas, icon as usize, x, y, color),
        5 if icon == 1 => canvas.clip_y(None),
        5 => canvas.clip_y(Some((x, y))),
        _ => {}
    }
}
