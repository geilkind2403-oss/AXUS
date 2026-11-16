//! Keys from everything that can control the launcher, as one stream:
//! USB/Bluetooth keyboards and remotes, IR receivers (evdev), the TV remote
//! over HDMI-CEC (cec.rs) and the phone (remote.rs). Devices plugged in
//! later (a Bluetooth keyboard that connects after boot) are picked up.
use std::sync::mpsc::Sender;

pub const LONG_BACK_HOLD: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Ok,
    LongOk,
    LongBack,
    Back,
    Home,
    Menu,
    PlayPause,
    Play,
    Pause,
    Stop,
    Next,
    Prev,
    Forward,
    Rewind,
    VolUp,
    VolDown,
    Mute,
    ChUp,
    ChDown,
    Info,
    Red,
    Green,
    Yellow,
    Blue,
    Digit(u8),
    Char(char),
    Backspace,
    Power,
}

impl Key {
    pub fn from_name(s: &str) -> Option<Key> {
        Some(match s {
            "up" => Key::Up,
            "down" => Key::Down,
            "left" => Key::Left,
            "right" => Key::Right,
            "ok" | "enter" => Key::Ok,
            "longok" => Key::LongOk,
            "longback" => Key::LongBack,
            "back" => Key::Back,
            "home" => Key::Home,
            "menu" => Key::Menu,
            "playpause" => Key::PlayPause,
            "stop" => Key::Stop,
            "next" => Key::Next,
            "prev" => Key::Prev,
            "forward" => Key::Forward,
            "rewind" => Key::Rewind,
            "volup" => Key::VolUp,
            "voldown" => Key::VolDown,
            "mute" => Key::Mute,
            "info" => Key::Info,
            "backspace" => Key::Backspace,
            "power" => Key::Power,
            _ => {
                let mut ch = s.chars();
                return match (ch.next(), ch.next()) {
                    (Some(c), None) if c.is_ascii_digit() => Some(Key::Digit(c as u8 - b'0')),
                    _ => None,
                };
            }
        })
    }
}

/// While the YouTube TV browser runs: real keyboards/remotes go to it
/// directly, CEC + phone keys are typed into it via uinput; the launcher
/// only reacts to Home (to come back).
pub static BROWSER_ACTIVE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
/// A browser keeps playing under the launcher control panel. While that panel
/// is open, remote keys belong to the launcher instead of the web page.
pub static CONTROL_OPEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn browser_active() -> bool {
    BROWSER_ACTIVE.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn control_open() -> bool {
    CONTROL_OPEN.load(std::sync::atomic::Ordering::Acquire)
}

/// Keys from the TV remote / phone: to the browser when it runs, else to the launcher.
pub fn route(tx: &Sender<InputMsg>, k: Key) -> bool {
    #[cfg(target_os = "linux")]
    if browser_active()
        && !control_open()
        && k != Key::Home
        && k != Key::Power
        && k != Key::Back
        && k != Key::LongOk
        && k != Key::LongBack
    {
        super::uinput::press(k);
        return true;
    }
    tx.send(InputMsg::Key(k)).is_ok()
}

#[derive(Debug)]
pub enum InputMsg {
    Key(Key),
    /// Text typed on the phone (search fields).
    Text(String),
}

/// Linux key code -> Key. Letters/space come through as Char for search fields.
pub fn map_evdev(code: u16, shift: bool) -> Option<Key> {
    const ROW1: &str = "qwertyuiop";
    const ROW2: &str = "asdfghjkl";
    const ROW3: &str = "zxcvbnm";
    let letter = |row: &str, base: u16| {
        row.chars()
            .nth((code - base) as usize)
            .map(|c| Key::Char(if shift { c.to_ascii_uppercase() } else { c }))
    };
    Some(match code {
        1 | 158 | 174 => Key::Back,             // ESC, BACK, EXIT
        2..=10 => Key::Digit((code - 1) as u8), // 1..9
        11 => Key::Digit(0),
        14 => Key::Backspace,
        16..=25 => return letter(ROW1, 16),
        30..=38 => return letter(ROW2, 30),
        44..=50 => return letter(ROW3, 44),
        12 => Key::Char('-'),
        52 => Key::Char('.'),
        57 => Key::Char(' '),
        28 | 96 | 352 | 353 => Key::Ok, // ENTER, KPENTER, OK, SELECT
        103 => Key::Up,
        105 => Key::Left,
        106 => Key::Right,
        108 => Key::Down,
        102 | 172 => Key::Home, // HOME, HOMEPAGE
        104 => Key::ChUp,       // PAGEUP
        109 => Key::ChDown,     // PAGEDOWN
        113 => Key::Mute,
        114 => Key::VolDown,
        115 => Key::VolUp,
        116 => Key::Power,
        119 | 164 => Key::PlayPause, // PAUSE, PLAYPAUSE
        139 | 127 => Key::Menu,      // MENU, COMPOSE
        163 | 407 => Key::Next,      // NEXTSONG, NEXT
        165 | 412 => Key::Prev,      // PREVIOUSSONG, PREVIOUS
        128 | 166 => Key::Stop,
        168 => Key::Rewind,
        208 | 159 => Key::Forward, // FASTFORWARD, FORWARD
        200 | 207 => Key::Play,
        201 => Key::Pause,
        358 => Key::Info,
        398 => Key::Red,
        399 => Key::Green,
        400 => Key::Yellow,
        401 => Key::Blue,
        402 => Key::ChUp,
        403 => Key::ChDown,
        0x200..=0x209 => Key::Digit((code - 0x200) as u8),
        _ => return None,
    })
}

#[cfg(target_os = "linux")]
pub fn start_evdev(tx: Sender<InputMsg>) {
    use std::collections::HashSet;
    use std::io::Read;
    use std::sync::{Arc, Mutex};
    let open: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    std::thread::spawn(move || loop {
        let mut paths: Vec<String> = std::fs::read_dir("/dev/input")
            .map(|r| {
                r.flatten()
                    .map(|e| e.path().to_string_lossy().into_owned())
                    .filter(|p| p.contains("/event"))
                    .collect()
            })
            .unwrap_or_default();
        paths.sort();
        for p in paths {
            if super::uinput::is_launcher_device(&p) {
                continue;
            }
            if open.lock().unwrap().contains(&p) {
                continue;
            }
            let Ok(mut f) = std::fs::File::open(&p) else {
                continue;
            };
            open.lock().unwrap().insert(p.clone());
            let tx = tx.clone();
            let open = open.clone();
            std::thread::spawn(move || {
                // struct input_event: timeval (2 longs) + u16 type + u16 code + i32 value
                let sz = 2 * std::mem::size_of::<libc::c_long>() + 8;
                let off = sz - 8;
                let mut buf = vec![0u8; sz * 32];
                let mut shift = false;
                let ok_held = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                let ok_long = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                let back_held = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                let back_long = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                loop {
                    let n = match f.read(&mut buf) {
                        Ok(0) | Err(_) => break, // unplugged
                        Ok(n) => n,
                    };
                    for ev in buf[..n].chunks_exact(sz) {
                        let typ = u16::from_ne_bytes([ev[off], ev[off + 1]]);
                        let code = u16::from_ne_bytes([ev[off + 2], ev[off + 3]]);
                        let value = i32::from_ne_bytes([
                            ev[off + 4],
                            ev[off + 5],
                            ev[off + 6],
                            ev[off + 7],
                        ]);
                        if typ != 1 {
                            continue; // EV_KEY only
                        }
                        if code == 42 || code == 54 {
                            shift = value != 0;
                            continue;
                        }
                        if matches!(map_evdev(code, shift), Some(Key::Ok)) {
                            if value == 1 {
                                ok_held.store(true, std::sync::atomic::Ordering::Relaxed);
                                ok_long.store(false, std::sync::atomic::Ordering::Relaxed);
                                let held = ok_held.clone();
                                let long = ok_long.clone();
                                let tx = tx.clone();
                                std::thread::spawn(move || {
                                    std::thread::sleep(std::time::Duration::from_millis(650));
                                    if held.load(std::sync::atomic::Ordering::Relaxed) {
                                        long.store(true, std::sync::atomic::Ordering::Relaxed);
                                        let _ = tx.send(InputMsg::Key(Key::LongOk));
                                    }
                                });
                            } else if value == 0 {
                                ok_held.store(false, std::sync::atomic::Ordering::Relaxed);
                                if !ok_long.swap(false, std::sync::atomic::Ordering::Relaxed) {
                                    if browser_active() && !control_open() {
                                        super::uinput::press(Key::Ok);
                                    } else {
                                        let _ = tx.send(InputMsg::Key(Key::Ok));
                                    }
                                }
                            }
                            continue;
                        }
                        if matches!(map_evdev(code, shift), Some(Key::Back)) {
                            if value == 1 {
                                back_held.store(true, std::sync::atomic::Ordering::Relaxed);
                                back_long.store(false, std::sync::atomic::Ordering::Relaxed);
                                let held = back_held.clone();
                                let long = back_long.clone();
                                let tx = tx.clone();
                                std::thread::spawn(move || {
                                    std::thread::sleep(LONG_BACK_HOLD);
                                    if held.load(std::sync::atomic::Ordering::Relaxed) {
                                        long.store(true, std::sync::atomic::Ordering::Relaxed);
                                        let _ = tx.send(InputMsg::Key(Key::LongBack));
                                    }
                                });
                            } else if value == 0 {
                                back_held.store(false, std::sync::atomic::Ordering::Relaxed);
                                if !back_long.swap(false, std::sync::atomic::Ordering::Relaxed) {
                                    let _ = tx.send(InputMsg::Key(Key::Back));
                                }
                            }
                            continue;
                        }
                        if value == 0 {
                            continue;
                        } // release
                          // Key repeat is useful for navigation, but holding OK
                          // must not repeatedly open/tune screens on the Pi.
                        if let Some(k) = map_evdev(code, shift) {
                            // The browser is isolated from physical input and
                            // receives keys from our virtual keyboard instead.
                            if browser_active()
                                && !control_open()
                                && k != Key::Home
                                && k != Key::Power
                                && k != Key::LongOk
                                && k != Key::LongBack
                            {
                                super::uinput::press(k);
                                continue;
                            }
                            if tx.send(InputMsg::Key(k)).is_err() {
                                return;
                            }
                        }
                    }
                }
                open.lock().unwrap().remove(&p);
            });
        }
        std::thread::sleep(std::time::Duration::from_secs(3));
    });
}

#[cfg(not(target_os = "linux"))]
pub fn start_evdev(_tx: Sender<InputMsg>) {}
