//! The TV's own remote over HDMI-CEC, through the kernel CEC API
//! (/dev/cec0 -- the vc4 HDMI port has it built in; no libcec).
//!
//! The launcher registers as a playback device ("TV Launcher" in the TV's
//! source list), tells the TV it is the active source (so the TV forwards its
//! remote's keys) and turns the TV on at start.
use super::input::{InputMsg, Key};
use std::fs::OpenOptions;
use std::os::fd::AsRawFd;
use std::sync::mpsc::Sender;

const fn ioc(dir: u32, nr: u32, size: u32) -> u32 {
    (dir << 30) | (size << 16) | ((b'a' as u32) << 8) | nr
}
const R: u32 = 2;
const W: u32 = 1;
const ADAP_G_PHYS_ADDR: u32 = ioc(R, 1, 2);
const ADAP_S_LOG_ADDRS: u32 = ioc(R | W, 4, 92);
const TRANSMIT: u32 = ioc(R | W, 5, 56);
const RECEIVE: u32 = ioc(R | W, 6, 56);
const S_MODE: u32 = ioc(W, 9, 4);

static FD: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);
static ME: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(4);

/// Switches the TV off (CEC "Standby" to the TV) -- the sleep timer uses it.
pub fn tv_off() {
    let fd = FD.load(std::sync::atomic::Ordering::Relaxed);
    if fd >= 0 {
        let me = ME.load(std::sync::atomic::Ordering::Relaxed);
        transmit(fd, &[me << 4, 0x36]);
    }
}

fn ioctl(fd: i32, req: u32, arg: *mut u8) -> i32 {
    unsafe { libc::ioctl(fd, req as _, arg) }
}

fn transmit(fd: i32, bytes: &[u8]) {
    let mut m = [0u8; 56];
    m[16..20].copy_from_slice(&(bytes.len() as u32).to_ne_bytes());
    m[20..24].copy_from_slice(&1000u32.to_ne_bytes()); // timeout ms (for replies)
    m[32..32 + bytes.len()].copy_from_slice(bytes);
    ioctl(fd, TRANSMIT, m.as_mut_ptr());
}

fn cec_key(op: u8) -> Option<Key> {
    Some(match op {
        0x00 | 0x2b => Key::Ok,
        0x01 => Key::Up,
        0x02 => Key::Down,
        0x03 => Key::Left,
        0x04 => Key::Right,
        0x09 => Key::Home,
        0x0a | 0x0b | 0x10 => Key::Menu, // setup / contents / favourite menu
        0x0d | 0x91 => Key::Back,        // exit / return
        0x20..=0x29 => Key::Digit(op - 0x20),
        0x30 => Key::ChUp,
        0x31 => Key::ChDown,
        0x35 => Key::Info,
        0x41 => Key::VolUp,
        0x42 => Key::VolDown,
        0x43 => Key::Mute,
        0x44 | 0x60 => Key::Play,
        0x45 | 0x64 => Key::Stop,
        0x46 | 0x61 => Key::PlayPause,
        0x48 => Key::Rewind,
        0x49 => Key::Forward,
        0x4b => Key::Next,
        0x4c => Key::Prev,
        0x71 => Key::Blue,
        0x72 => Key::Red,
        0x73 => Key::Green,
        0x74 => Key::Yellow,
        _ => return None,
    })
}

/// Starts the CEC thread. `standby` is called when the TV goes off.
pub fn start(tx: Sender<InputMsg>, on_standby: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        let Ok(f) = OpenOptions::new().read(true).write(true).open("/dev/cec0") else {
            crate::log("HDMI-CEC: /dev/cec0 fehlt -- TV-Fernbedienung geht nicht (USB-Fernbedienung, Tastatur oder Handy gehen)");
            return;
        };
        let fd = f.as_raw_fd();
        let mut mode = 0x01u32 | 0x10; // initiator + follower
        ioctl(fd, S_MODE, &mut mode as *mut u32 as *mut u8);

        let mut la = [0u8; 92];
        // Clear first (EBUSY when something configured it already), then claim a playback address.
        ioctl(fd, ADAP_S_LOG_ADDRS, la.as_mut_ptr());
        la[6] = 5; // CEC 1.4
        la[7] = 1; // one logical address
        la[8..12].copy_from_slice(&0xffff_ffffu32.to_ne_bytes()); // no vendor id
        let name = b"TV Launcher";
        la[16..16 + name.len()].copy_from_slice(name);
        la[31] = 4; // primary device type: playback
        la[35] = 2; // logical address type: playback
        la[39] = 0x10; // all device types: playback
        if ioctl(fd, ADAP_S_LOG_ADDRS, la.as_mut_ptr()) < 0 {
            crate::log(&format!(
                "HDMI-CEC: Anmelden ging nicht ({})",
                std::io::Error::last_os_error()
            ));
        }
        let me = if la[0] == 0xff { 4 } else { la[0] & 0x0f };
        ME.store(me, std::sync::atomic::Ordering::Relaxed);
        FD.store(fd, std::sync::atomic::Ordering::Relaxed);
        std::mem::forget(f.try_clone().ok()); // keep the fd valid for tv_off
        let mut pa = [0u8; 2];
        ioctl(fd, ADAP_G_PHYS_ADDR, pa.as_mut_ptr());
        let pa = u16::from_ne_bytes(pa);
        let (pa_hi, pa_lo) = ((pa >> 8) as u8, pa as u8);
        let active_source = |fd| transmit(fd, &[(me << 4) | 0x0f, 0x82, pa_hi, pa_lo]);
        transmit(fd, &[me << 4, 0x04]); // Image View On: TV on
        active_source(fd);
        crate::log(&format!(
            "HDMI-CEC bereit (Adresse {me}, {:x}.{:x}.{:x}.{:x})",
            pa_hi >> 4,
            pa_hi & 15,
            pa_lo >> 4,
            pa_lo & 15
        ));

        loop {
            let ok_held = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let ok_long = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let back_held = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let back_long = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            loop {
                let mut m = [0u8; 56];
                if ioctl(fd, RECEIVE, m.as_mut_ptr()) < 0 {
                    let err = std::io::Error::last_os_error();
                    if err.raw_os_error() == Some(libc::EINTR) {
                        continue;
                    }
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    continue;
                }
                let len = u32::from_ne_bytes([m[16], m[17], m[18], m[19]]) as usize;
                let msg = &m[32..32 + len.min(16)];
                if msg.len() < 2 {
                    continue;
                }
                let from = msg[0] >> 4;
                match msg[1] {
                    0x44 if msg.len() >= 3 => {
                        if let Some(k) = cec_key(msg[2]) {
                            if k == Key::Ok {
                                if !ok_held.swap(true, std::sync::atomic::Ordering::Relaxed) {
                                    ok_long.store(false, std::sync::atomic::Ordering::Relaxed);
                                    let held = ok_held.clone();
                                    let long = ok_long.clone();
                                    let tx = tx.clone();
                                    std::thread::spawn(move || {
                                        std::thread::sleep(std::time::Duration::from_millis(650));
                                        if held.load(std::sync::atomic::Ordering::Relaxed) {
                                            long.store(true, std::sync::atomic::Ordering::Relaxed);
                                            let _ = super::input::route(&tx, Key::LongOk);
                                        }
                                    });
                                }
                            } else if k == Key::Back {
                                if !back_held.swap(true, std::sync::atomic::Ordering::Relaxed) {
                                    back_long.store(false, std::sync::atomic::Ordering::Relaxed);
                                    let held = back_held.clone();
                                    let long = back_long.clone();
                                    let tx = tx.clone();
                                    std::thread::spawn(move || {
                                        std::thread::sleep(std::time::Duration::from_secs(3));
                                        if held.load(std::sync::atomic::Ordering::Relaxed) {
                                            long.store(true, std::sync::atomic::Ordering::Relaxed);
                                            let _ = super::input::route(&tx, Key::LongBack);
                                        }
                                    });
                                }
                            } else if !super::input::route(&tx, k) {
                                return;
                            }
                        }
                    }
                    0x45 => {
                        if ok_held.swap(false, std::sync::atomic::Ordering::Relaxed)
                            && !ok_long.swap(false, std::sync::atomic::Ordering::Relaxed)
                            && !super::input::route(&tx, Key::Ok)
                        {
                            return;
                        }
                        if back_held.swap(false, std::sync::atomic::Ordering::Relaxed)
                            && !back_long.swap(false, std::sync::atomic::Ordering::Relaxed)
                            && !super::input::route(&tx, Key::Back)
                        {
                            return;
                        }
                    }
                    0x8d => transmit(fd, &[(me << 4) | from, 0x8e, 0x00]), // menu request -> activated
                    0x86 | 0x85 => active_source(fd), // set stream path / request active source
                    0x36 => on_standby(),
                    _ => {}
                }
            }
        }
    });
}
