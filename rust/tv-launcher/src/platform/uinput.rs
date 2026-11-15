//! A virtual keyboard (/dev/uinput): while the YouTube TV browser runs, the
//! TV remote (HDMI-CEC) and the phone type into it through this. Real USB /
//! Bluetooth keyboards reach the browser directly.
use super::input::Key;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const DEVICE_NAME: &str = "TV Launcher Fernbedienung";

const fn ioc(dir: u32, nr: u32, size: u32) -> u32 {
    (dir << 30) | (size << 16) | ((b'U' as u32) << 8) | nr
}
const UI_SET_EVBIT: u32 = ioc(1, 100, 4);
const UI_SET_KEYBIT: u32 = ioc(1, 101, 4);
const UI_DEV_CREATE: u32 = ioc(0, 1, 0);

static DEV: Mutex<Option<File>> = Mutex::new(None);

/// Linux key codes the browser understands.
fn code(k: Key) -> Option<(u16, bool)> {
    Some(match k {
        Key::Up => (103, false),
        Key::Down => (108, false),
        Key::Left => (105, false),
        Key::Right => (106, false),
        Key::Ok => (28, false),
        Key::Back => (1, false), // Escape: "back" in the YouTube TV app
        Key::Backspace => (14, false),
        Key::PlayPause | Key::Play | Key::Pause => (164, false),
        Key::Stop => (128, false),
        Key::Next => (163, false),
        Key::Prev => (165, false),
        Key::Forward => (208, false),
        Key::Rewind => (168, false),
        Key::Menu => (139, false),
        Key::Info => (358, false),
        Key::Digit(0) => (11, false),
        Key::Digit(d) => (1 + d as u16, false),
        Key::Char(c) => return char_code(c),
        _ => return None,
    })
}

/// (key code, needs shift) for typing text.
fn char_code(c: char) -> Option<(u16, bool)> {
    const ROW1: &str = "qwertyuiop";
    const ROW2: &str = "asdfghjkl";
    const ROW3: &str = "zxcvbnm";
    let lower = c.to_ascii_lowercase();
    let shift = c.is_ascii_uppercase();
    if let Some(i) = ROW1.find(lower) {
        return Some((16 + i as u16, shift));
    }
    if let Some(i) = ROW2.find(lower) {
        return Some((30 + i as u16, shift));
    }
    if let Some(i) = ROW3.find(lower) {
        return Some((44 + i as u16, shift));
    }
    Some(match c {
        '1'..='9' => (2 + (c as u16 - '1' as u16), false),
        '0' => (11, false),
        ' ' => (57, false),
        '-' => (12, false),
        '.' => (52, false),
        ',' => (51, false),
        _ => return None,
    })
}

pub fn open() -> Result<(), String> {
    let mut g = DEV.lock().unwrap();
    if g.is_some() {
        return Ok(());
    }
    let f = OpenOptions::new()
        .write(true)
        .open("/dev/uinput")
        .map_err(|e| format!("/dev/uinput: {e}"))?;
    let fd = f.as_raw_fd();
    unsafe {
        libc::ioctl(fd, UI_SET_EVBIT as _, 1 as libc::c_int); // EV_KEY
        for c in 1..=255 {
            libc::ioctl(fd, UI_SET_KEYBIT as _, c as libc::c_int);
        }
        libc::ioctl(fd, UI_SET_KEYBIT as _, 358 as libc::c_int);
    }
    // struct uinput_user_dev: name[80], input_id (4 x u16), ff_effects_max, abs arrays
    let mut dev = vec![0u8; 80 + 8 + 4 + 4 * 64 * 4];
    let name = DEVICE_NAME.as_bytes();
    dev[..name.len()].copy_from_slice(name);
    dev[80..82].copy_from_slice(&6u16.to_ne_bytes()); // BUS_VIRTUAL
    dev[82..84].copy_from_slice(&0x1209u16.to_ne_bytes());
    dev[84..86].copy_from_slice(&0x7476u16.to_ne_bytes());
    let mut f = f;
    f.write_all(&dev)
        .map_err(|e| format!("uinput setup: {e}"))?;
    if unsafe { libc::ioctl(fd, UI_DEV_CREATE as _) } < 0 {
        return Err(format!(
            "uinput create: {}",
            std::io::Error::last_os_error()
        ));
    }
    *g = Some(f);
    Ok(())
}

/// The virtual keyboard's evdev node. Cog sees this one input device in its
/// chroot, while physical devices stay with the launcher so its PiP menu can
/// take focus without YouTube also consuming the same button press.
pub fn launcher_event_path() -> Option<PathBuf> {
    let entries = std::fs::read_dir("/sys/class/input").ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("event") {
            continue;
        }
        let Ok(device_name) = std::fs::read_to_string(entry.path().join("device/name")) else {
            continue;
        };
        if device_name.trim() == DEVICE_NAME {
            return Some(Path::new("/dev/input").join(name.as_ref()));
        }
    }
    None
}

/// Waits (up to `max`) until udev has recorded the virtual keyboard as an
/// input device. The browser's libinput ignores devices udev has not tagged
/// yet: a keyboard created moments before the browser starts never types.
pub fn wait_until_tagged(event: &Path, max: std::time::Duration) -> bool {
    let started = std::time::Instant::now();
    let Some(name) = event.file_name() else { return false };
    let dev = Path::new("/sys/class/input").join(name).join("dev");
    loop {
        if let Ok(numbers) = std::fs::read_to_string(&dev) {
            let db = Path::new("/run/udev/data").join(format!("c{}", numbers.trim()));
            if std::fs::read_to_string(db)
                .map(|d| d.contains("E:ID_INPUT=1"))
                .unwrap_or(false)
            {
                return true;
            }
        }
        if started.elapsed() >= max {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// The launcher also monitors evdev devices; ignore its own synthetic device
/// to avoid feeding injected keys back into Cog forever.
pub fn is_launcher_device(path: &str) -> bool {
    let Some(event) = Path::new(path).file_name() else {
        return false;
    };
    let name = Path::new("/sys/class/input")
        .join(event)
        .join("device/name");
    std::fs::read_to_string(name)
        .map(|s| s.trim() == DEVICE_NAME)
        .unwrap_or(false)
}

fn event(f: &mut File, typ: u16, code: u16, value: i32) {
    let l = std::mem::size_of::<libc::c_long>();
    let mut ev = vec![0u8; 2 * l + 8];
    ev[2 * l..2 * l + 2].copy_from_slice(&typ.to_ne_bytes());
    ev[2 * l + 2..2 * l + 4].copy_from_slice(&code.to_ne_bytes());
    ev[2 * l + 4..].copy_from_slice(&value.to_ne_bytes());
    let _ = f.write_all(&ev);
}

fn tap(f: &mut File, code: u16, shift: bool) {
    if shift {
        event(f, 1, 42, 1);
    }
    event(f, 1, code, 1);
    event(f, 0, 0, 0);
    event(f, 1, code, 0);
    if shift {
        event(f, 1, 42, 0);
    }
    event(f, 0, 0, 0);
}

/// Presses (and releases) a key on the virtual keyboard.
pub fn press(k: Key) {
    let mut g = DEV.lock().unwrap();
    let Some(f) = g.as_mut() else { return };
    if let Some((c, shift)) = code(k) {
        tap(f, c, shift);
    }
}

/// Types text (the phone's text field).
pub fn type_text(s: &str) {
    let mut g = DEV.lock().unwrap();
    let Some(f) = g.as_mut() else { return };
    for ch in s.chars() {
        if let Some((c, shift)) = char_code(ch) {
            tap(f, c, shift);
            std::thread::sleep(std::time::Duration::from_millis(8));
        }
    }
}
