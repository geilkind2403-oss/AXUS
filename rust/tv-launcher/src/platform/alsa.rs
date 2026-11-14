//! Sound straight to the kernel's ALSA PCM device -- no alsa-lib, no
//! PulseAudio. The launcher is the only thing making sound, so the volume is
//! applied to the samples here (software volume, 0..100).
//!
//! The Pi's HDMI audio (vc4-hdmi) only takes IEC958 subframes; alsa-lib
//! normally converts to that, so this does it too (iec958 plugin's rules:
//! 24-bit sample in bits 4..27, channel status bit, parity, preamble).
//! The headphone jack (bcm2835) takes plain S16.
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicU32, Ordering};

const fn ioc(dir: u32, nr: u32, size: u32) -> u32 {
    (dir << 30) | (size << 16) | ((b'A' as u32) << 8) | nr
}
const RW: u32 = 3;
const W: u32 = 1;
const R: u32 = 2;
const LONG: u32 = std::mem::size_of::<libc::c_long>() as u32;
const HW_PARAMS_SIZE: u32 = 536 + LONG + 64;
const HW_REFINE: u32 = ioc(RW, 0x10, HW_PARAMS_SIZE);
const HW_PARAMS: u32 = ioc(RW, 0x11, HW_PARAMS_SIZE);
const PREPARE: u32 = ioc(0, 0x40, 0);
const DROP: u32 = ioc(0, 0x43, 0);
const DELAY: u32 = ioc(R, 0x21, LONG);
const WRITEI: u32 = ioc(W, 0x50, 3 * LONG);
const PAUSE: u32 = ioc(W, 0x45, 4);

const FMT_S16_LE: u32 = 2;
const FMT_IEC958_LE: u32 = 18;

/// 0..=100, read by every writer.
pub static VOLUME: AtomicU32 = AtomicU32::new(70);
pub static MUTED: AtomicU32 = AtomicU32::new(0);

#[repr(C, align(8))]
struct HwParams([u8; 616]);

impl HwParams {
    fn any() -> HwParams {
        let mut p = HwParams([0; 616]);
        // masks (access, format, subformat) all-ones; reserved masks stay 0
        for i in 0..3 {
            for b in &mut p.0[4 + i * 32..4 + i * 32 + 32] {
                *b = 0xff;
            }
        }
        // intervals 8..19: [0, UINT_MAX]
        for i in 0..12 {
            let o = 260 + i * 12;
            p.0[o..o + 4].copy_from_slice(&0u32.to_ne_bytes());
            p.0[o + 4..o + 8].copy_from_slice(&u32::MAX.to_ne_bytes());
            p.0[o + 8..o + 12].copy_from_slice(&0u32.to_ne_bytes());
        }
        p.0[512..516].copy_from_slice(&u32::MAX.to_ne_bytes()); // rmask
        p
    }
    fn set_mask(&mut self, idx: usize, bit: u32) {
        let o = 4 + idx * 32;
        for b in &mut self.0[o..o + 32] {
            *b = 0;
        }
        let w = o + (bit / 32) as usize * 4;
        self.0[w..w + 4].copy_from_slice(&(1u32 << (bit % 32)).to_ne_bytes());
    }
    fn set_interval(&mut self, param: usize, min: u32, max: u32, integer: bool) {
        let o = 260 + (param - 8) * 12;
        self.0[o..o + 4].copy_from_slice(&min.to_ne_bytes());
        self.0[o + 4..o + 8].copy_from_slice(&max.to_ne_bytes());
        self.0[o + 8..o + 12].copy_from_slice(&(if integer { 4u32 } else { 0 }).to_ne_bytes());
    }
    fn interval(&self, param: usize) -> (u32, u32) {
        let o = 260 + (param - 8) * 12;
        (
            u32::from_ne_bytes(self.0[o..o + 4].try_into().unwrap()),
            u32::from_ne_bytes(self.0[o + 4..o + 8].try_into().unwrap()),
        )
    }
}

const P_CHANNELS: usize = 10;
const P_RATE: usize = 11;
const P_PERIOD_SIZE: usize = 13;
const P_BUFFER_SIZE: usize = 17;

pub struct Pcm {
    f: File,
    iec958: bool,
    pub rate: u32,
    counter: u32,
    status: [u8; 24],
    buf: Vec<u8>,
    pub written: u64,
}

fn card_ids() -> Vec<(u32, String)> {
    (0..8)
        .filter_map(|n| {
            std::fs::read_to_string(format!("/proc/asound/card{n}/id"))
                .ok()
                .map(|s| (n, s.trim().to_string()))
        })
        .collect()
}

/// "hdmi" (default) or "analog" (headphone jack).
pub fn pick_card(pref: &str) -> Option<u32> {
    let cards = card_ids();
    let is_hdmi = |id: &str| id.to_lowercase().contains("hdmi");
    let want_hdmi = pref != "analog";
    cards
        .iter()
        .find(|(_, id)| is_hdmi(id) == want_hdmi)
        .or(cards.first())
        .map(|(n, _)| *n)
}

impl Pcm {
    pub fn open(output: &str, rate: u32) -> Result<Pcm, String> {
        let card = pick_card(output).ok_or("keine Soundkarte")?;
        let path = format!("/dev/snd/pcmC{card}D0p");
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| format!("{path}: {e}"))?;
        let fd = f.as_raw_fd();
        let try_fmt = |fmt: u32| -> Option<HwParams> {
            let mut p = HwParams::any();
            p.set_mask(0, 3); // RW_INTERLEAVED
            p.set_mask(1, fmt);
            p.set_mask(2, 0);
            p.set_interval(P_CHANNELS, 2, 2, true);
            p.set_interval(P_RATE, rate, rate, true);
            if unsafe { libc::ioctl(fd, HW_REFINE as _, p.0.as_mut_ptr()) } < 0 {
                return None;
            }
            // ~21 ms periods, ~170 ms buffer: smooth on a busy Pi, small enough for A/V sync.
            let (pmin, pmax) = p.interval(P_PERIOD_SIZE);
            let period = 1024u32.clamp(pmin, pmax.max(pmin));
            p.set_interval(P_PERIOD_SIZE, period, period, true);
            let (bmin, bmax) = p.interval(P_BUFFER_SIZE);
            let buffer = (period * 8).clamp(bmin, bmax.max(bmin));
            p.set_interval(P_BUFFER_SIZE, buffer, buffer, true);
            if unsafe { libc::ioctl(fd, HW_PARAMS as _, p.0.as_mut_ptr()) } < 0 {
                // the exact sizes were refused: let the driver choose
                let mut p = HwParams::any();
                p.set_mask(0, 3);
                p.set_mask(1, fmt);
                p.set_mask(2, 0);
                p.set_interval(P_CHANNELS, 2, 2, true);
                p.set_interval(P_RATE, rate, rate, true);
                if unsafe { libc::ioctl(fd, HW_PARAMS as _, p.0.as_mut_ptr()) } < 0 {
                    return None;
                }
                return Some(p);
            }
            Some(p)
        };
        let (iec958, _p) = if let Some(p) = try_fmt(FMT_S16_LE) {
            (false, p)
        } else if let Some(p) = try_fmt(FMT_IEC958_LE) {
            (true, p)
        } else {
            return Err(format!("{path}: weder S16 noch IEC958 bei {rate} Hz"));
        };
        unsafe { libc::ioctl(fd, PREPARE as _) };
        // IEC958 channel status: consumer, PCM, original, not copyrighted, sample rate.
        let mut status = [0u8; 24];
        status[0] = 0x04;
        status[1] = 0x82;
        status[3] = match rate {
            44100 => 0x00,
            48000 => 0x02,
            32000 => 0x03,
            96000 => 0x0a,
            _ => 0x01,
        };
        Ok(Pcm {
            f,
            iec958,
            rate,
            counter: 0,
            status,
            buf: Vec::new(),
            written: 0,
        })
    }

    /// Interleaved stereo i16. Blocks while the device buffer is full.
    pub fn write(&mut self, samples: &[i16]) -> Result<(), String> {
        let vol = if MUTED.load(Ordering::Relaxed) != 0 {
            0
        } else {
            VOLUME.load(Ordering::Relaxed).min(100)
        };
        // perceptual curve: 50% sounds about half as loud
        let gain = ((vol as f32 / 100.0).powi(2) * 65536.0) as i32;
        let frames = samples.len() / 2;
        self.buf.clear();
        if self.iec958 {
            self.buf.reserve(samples.len() * 4);
            for (i, s) in samples.iter().enumerate() {
                let v = ((*s as i32 * gain) >> 16) as i16;
                let ch = i & 1;
                let mut d: u32 = ((v as i32 as u32) << 16) >> 4; // 16-bit sample in bits 12..27
                let byte = (self.counter >> 3) as usize;
                if self.status[byte] & (1 << (self.counter & 7)) != 0 {
                    d |= 0x4000_0000;
                }
                if (d & 0x7fff_fff0).count_ones() & 1 == 1 {
                    d |= 0x8000_0000;
                }
                d |= if ch == 1 {
                    0x04
                } else if self.counter == 0 {
                    0x08
                } else {
                    0x02
                };
                if ch == 1 {
                    self.counter = (self.counter + 1) % 192;
                }
                self.buf.extend_from_slice(&d.to_le_bytes());
            }
        } else {
            self.buf.reserve(samples.len() * 2);
            for s in samples {
                self.buf
                    .extend_from_slice(&(((*s as i32 * gain) >> 16) as i16).to_le_bytes());
            }
        }
        let fd = self.f.as_raw_fd();
        let bpf = if self.iec958 { 8 } else { 4 };
        let mut done = 0usize;
        while done < frames {
            #[repr(C)]
            struct Xferi {
                result: libc::c_long,
                buf: *const u8,
                frames: libc::c_ulong,
            }
            let mut x = Xferi {
                result: 0,
                buf: unsafe { self.buf.as_ptr().add(done * bpf) },
                frames: (frames - done) as libc::c_ulong,
            };
            let r = unsafe { libc::ioctl(fd, WRITEI as _, &mut x as *mut Xferi) };
            if r < 0 {
                let e = std::io::Error::last_os_error();
                match e.raw_os_error() {
                    Some(libc::EPIPE) | Some(libc::ESTRPIPE) => {
                        unsafe { libc::ioctl(fd, PREPARE as _) }; // underrun: carry on
                        continue;
                    }
                    Some(libc::EINTR) | Some(libc::EAGAIN) => continue,
                    _ => return Err(e.to_string()),
                }
            }
            done += x.result.max(0) as usize;
        }
        self.written += frames as u64;
        Ok(())
    }

    /// Frames written but not yet heard.
    pub fn delay(&self) -> u64 {
        let mut d: libc::c_long = 0;
        if unsafe { libc::ioctl(self.f.as_raw_fd(), DELAY as _, &mut d) } < 0 {
            return 0;
        }
        d.max(0) as u64
    }

    /// Seconds of audio actually played so far.
    pub fn clock(&self) -> f64 {
        self.written.saturating_sub(self.delay()) as f64 / self.rate as f64
    }

    pub fn pause(&self, on: bool) {
        let mut v: i32 = on as i32;
        unsafe { libc::ioctl(self.f.as_raw_fd(), PAUSE as _, &mut v) };
    }

    /// Throws away what's buffered (seek, stop).
    pub fn flush(&mut self) {
        let fd = self.f.as_raw_fd();
        unsafe {
            libc::ioctl(fd, DROP as _);
            libc::ioctl(fd, PREPARE as _);
        }
        self.written = 0;
        self.counter = 0;
    }
}
