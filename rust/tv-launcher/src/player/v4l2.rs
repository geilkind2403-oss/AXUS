//! The Pi's H.264 hardware decoder through V4L2 (bcm2835-codec, the
//! "stateful" memory-to-memory decoder Kodi uses on LibreELEC too).
//! Bitstream goes in on the OUTPUT queue; decoded NV12/I420 pictures come out
//! on the CAPTURE queue and are exported as DMABUF, so the display shows them
//! without a single copy.
//!
//! Struct layouts are written out by hand for the kernel ABI (32-bit ARM:
//! 4-byte longs, the classic timeval), so no C headers are needed.
use crate::platform::kms::{Kms, VideoFrameDesc};
use std::fs::OpenOptions;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::{Arc, Mutex};

const L: usize = std::mem::size_of::<libc::c_long>();
const fn ioc(dir: u32, nr: u32, size: usize) -> u32 {
    (dir << 30) | ((size as u32) << 16) | ((b'V' as u32) << 8) | nr
}
const R: u32 = 2;
const W: u32 = 1;
const RW: u32 = 3;
const BUF_SIZE: usize = if L == 8 { 88 } else { 68 };
const FMT_SIZE: usize = L + 200;
const EVENT_SIZE: usize = if L == 8 { 136 } else { 128 };
const PLANE_SIZE: usize = if L == 8 { 64 } else { 60 };

const QUERYCAP: u32 = ioc(R, 0, 104);
const G_FMT: u32 = ioc(RW, 4, FMT_SIZE);
const S_FMT: u32 = ioc(RW, 5, FMT_SIZE);
const REQBUFS: u32 = ioc(RW, 8, 20);
const QUERYBUF: u32 = ioc(RW, 9, BUF_SIZE);
const QBUF: u32 = ioc(RW, 15, BUF_SIZE);
const EXPBUF: u32 = ioc(RW, 16, 64);
const DQBUF: u32 = ioc(RW, 17, BUF_SIZE);
const STREAMON: u32 = ioc(W, 18, 4);
const STREAMOFF: u32 = ioc(W, 19, 4);
const G_CTRL: u32 = ioc(RW, 27, 8);
const DQEVENT: u32 = ioc(R, 89, EVENT_SIZE);
const SUBSCRIBE_EVENT: u32 = ioc(W, 90, 32);

const fn fourcc(s: &[u8; 4]) -> u32 {
    (s[0] as u32) | ((s[1] as u32) << 8) | ((s[2] as u32) << 16) | ((s[3] as u32) << 24)
}
const PIX_H264: u32 = fourcc(b"H264");
const PIX_NV12: u32 = fourcc(b"NV12");
const PIX_YU12: u32 = fourcc(b"YU12");
const MEM_MMAP: u32 = 1;
const EVENT_SOURCE_CHANGE: u32 = 5;
const EVENT_EOS: u32 = 2;
const CID_MIN_BUFFERS_FOR_CAPTURE: u32 = 0x0098_0927;

fn w32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_ne_bytes());
}
fn r32(b: &[u8], o: usize) -> u32 {
    u32::from_ne_bytes(b[o..o + 4].try_into().unwrap())
}
fn wl(b: &mut [u8], o: usize, v: u64) {
    if L == 8 {
        b[o..o + 8].copy_from_slice(&v.to_ne_bytes())
    } else {
        w32(b, o, v as u32)
    }
}
fn rl(b: &[u8], o: usize) -> u64 {
    if L == 8 {
        u64::from_ne_bytes(b[o..o + 8].try_into().unwrap())
    } else {
        r32(b, o) as u64
    }
}

// v4l2_buffer field offsets
const B_INDEX: usize = 0;
const B_TYPE: usize = 4;
const B_BYTESUSED: usize = 8;
const B_FLAGS: usize = 12;
const B_TS_SEC: usize = if L == 8 { 24 } else { 20 };
const B_TS_USEC: usize = B_TS_SEC + L;
const B_MEMORY: usize = if L == 8 { 60 } else { 48 };
const B_M: usize = if L == 8 { 64 } else { 52 };
const B_LENGTH: usize = if L == 8 { 72 } else { 56 };
const FLAG_LAST: u32 = 0x0010_0000;
const FLAG_ERROR: u32 = 0x40;

fn ioctl(fd: i32, req: u32, arg: *mut u8) -> Result<(), std::io::Error> {
    loop {
        let r = unsafe { libc::ioctl(fd, req as _, arg) };
        if r >= 0 {
            return Ok(());
        }
        let e = std::io::Error::last_os_error();
        if e.raw_os_error() != Some(libc::EINTR) {
            return Err(e);
        }
    }
}

struct Mapped {
    ptr: *mut u8,
    len: usize,
}
unsafe impl Send for Mapped {}
impl Drop for Mapped {
    fn drop(&mut self) {
        unsafe { libc::munmap(self.ptr as *mut _, self.len) };
    }
}

pub struct Frame {
    pub index: u32,
    pub pts: f64,
}

pub struct Decoder {
    fd: OwnedFd,
    mplane: bool,
    out_type: u32,
    cap_type: u32,
    out_bufs: Vec<Mapped>,
    out_free: Vec<usize>,
    cap_count: u32,
    /// capture buffers owned by us (shown or waiting) -- not queued in the decoder
    held: Vec<u32>,
    pub coded: (u32, u32),
    kms: Arc<Mutex<Kms>>,
    ids_base: u32,
    pub eos: bool,
}

/// The decoder's /dev/videoN, found by name.
fn find_device() -> Option<std::fs::File> {
    for n in [10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 31, 0, 1, 2, 3] {
        let path = format!("/dev/video{n}");
        let Ok(f) = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&path)
        else {
            continue;
        };
        let mut cap = [0u8; 104];
        if ioctl(f.as_raw_fd(), QUERYCAP, cap.as_mut_ptr()).is_ok() {
            let card = String::from_utf8_lossy(&cap[16..48]).to_string();
            if card.contains("decode") || card.contains("h264") {
                crate::log(&format!(
                    "H.264-Decoder: {path} ({})",
                    card.trim_end_matches('\0')
                ));
                return Some(f);
            }
        }
    }
    None
}
use std::os::unix::fs::OpenOptionsExt;

impl Decoder {
    pub fn open(width: u32, height: u32, kms: Arc<Mutex<Kms>>) -> Result<Decoder, String> {
        let f = find_device().ok_or("kein H.264-Hardwaredecoder (/dev/video10)")?;
        let fd = f.as_raw_fd();
        let mut cap = [0u8; 104];
        ioctl(fd, QUERYCAP, cap.as_mut_ptr()).map_err(|e| e.to_string())?;
        let caps = r32(&cap, 84);
        let dev_caps = if caps & 0x8000_0000 != 0 {
            r32(&cap, 88)
        } else {
            caps
        };
        let mplane = dev_caps & 0x4000 != 0;
        let (out_type, cap_type) = if mplane { (10, 9) } else { (2, 1) };
        let fd_owned: OwnedFd = f.into();
        let mut d = Decoder {
            fd: fd_owned,
            mplane,
            out_type,
            cap_type,
            out_bufs: Vec::new(),
            out_free: Vec::new(),
            cap_count: 0,
            held: Vec::new(),
            coded: (0, 0),
            kms,
            ids_base: 1000,
            eos: false,
        };
        d.setup_output(width.max(64), height.max(64))?;
        Ok(d)
    }

    fn raw(&self) -> i32 {
        self.fd.as_raw_fd()
    }

    fn setup_output(&mut self, w: u32, h: u32) -> Result<(), String> {
        let fd = self.raw();
        let mut f = vec![0u8; FMT_SIZE];
        w32(&mut f, 0, self.out_type);
        let o = L;
        w32(&mut f, o, w);
        w32(&mut f, o + 4, h);
        w32(&mut f, o + 8, PIX_H264);
        let size = (2u32 << 20).max(w * h);
        if self.mplane {
            w32(&mut f, o + 20, size); // plane_fmt[0].sizeimage
            f[o + 180] = 1; // num_planes
        } else {
            w32(&mut f, o + 20, size); // sizeimage
        }
        ioctl(fd, S_FMT, f.as_mut_ptr()).map_err(|e| format!("Decoder-Format: {e}"))?;

        let mut rb = [0u8; 20];
        w32(&mut rb, 0, 6);
        w32(&mut rb, 4, self.out_type);
        w32(&mut rb, 8, MEM_MMAP);
        ioctl(fd, REQBUFS, rb.as_mut_ptr()).map_err(|e| format!("Decoder-Puffer: {e}"))?;
        let n = r32(&rb, 0);
        for i in 0..n {
            let (off, len) = self.query(self.out_type, i)?;
            let ptr = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    len as usize,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_SHARED,
                    fd,
                    off as libc::off_t,
                )
            };
            if ptr == libc::MAP_FAILED {
                return Err("mmap".into());
            }
            self.out_bufs.push(Mapped {
                ptr: ptr as *mut u8,
                len: len as usize,
            });
            self.out_free.push(i as usize);
        }
        for ev in [EVENT_SOURCE_CHANGE, EVENT_EOS] {
            let mut s = [0u8; 32];
            w32(&mut s, 0, ev);
            let _ = ioctl(fd, SUBSCRIBE_EVENT, s.as_mut_ptr());
        }
        let mut t = self.out_type.to_ne_bytes();
        ioctl(fd, STREAMON, t.as_mut_ptr()).map_err(|e| format!("Decoder-Start: {e}"))?;
        Ok(())
    }

    /// (mmap offset, length) of buffer i.
    fn query(&self, typ: u32, i: u32) -> Result<(u64, u32), String> {
        let mut b = vec![0u8; BUF_SIZE];
        let mut planes = vec![0u8; PLANE_SIZE * 8];
        w32(&mut b, B_INDEX, i);
        w32(&mut b, B_TYPE, typ);
        w32(&mut b, B_MEMORY, MEM_MMAP);
        if self.mplane {
            wl(&mut b, B_M, planes.as_mut_ptr() as usize as u64);
            w32(&mut b, B_LENGTH, 8);
        }
        ioctl(self.raw(), QUERYBUF, b.as_mut_ptr()).map_err(|e| format!("QUERYBUF: {e}"))?;
        Ok(if self.mplane {
            (r32(&planes, 8) as u64, r32(&planes, 4))
        } else {
            (r32(&b, B_M) as u64, r32(&b, B_LENGTH))
        })
    }

    fn qbuf(&self, typ: u32, i: u32, bytes: u32, pts_us: i64) -> Result<(), std::io::Error> {
        let mut b = vec![0u8; BUF_SIZE];
        let mut planes = vec![0u8; PLANE_SIZE];
        w32(&mut b, B_INDEX, i);
        w32(&mut b, B_TYPE, typ);
        w32(&mut b, B_MEMORY, MEM_MMAP);
        let us = pts_us.max(0) as u64;
        wl(&mut b, B_TS_SEC, us / 1_000_000);
        wl(&mut b, B_TS_USEC, us % 1_000_000);
        if self.mplane {
            w32(&mut planes, 0, bytes);
            wl(&mut b, B_M, planes.as_mut_ptr() as usize as u64);
            w32(&mut b, B_LENGTH, 1);
        } else {
            w32(&mut b, B_BYTESUSED, bytes);
        }
        ioctl(self.raw(), QBUF, b.as_mut_ptr())
    }

    /// (index, bytesused, flags, pts µs) or None when nothing is ready.
    fn dqbuf(&self, typ: u32) -> Option<(u32, u32, u32, i64)> {
        let mut b = vec![0u8; BUF_SIZE];
        let mut planes = vec![0u8; PLANE_SIZE * 8];
        w32(&mut b, B_TYPE, typ);
        w32(&mut b, B_MEMORY, MEM_MMAP);
        if self.mplane {
            wl(&mut b, B_M, planes.as_mut_ptr() as usize as u64);
            w32(&mut b, B_LENGTH, 8);
        }
        ioctl(self.raw(), DQBUF, b.as_mut_ptr()).ok()?;
        let used = if self.mplane {
            r32(&planes, 0)
        } else {
            r32(&b, B_BYTESUSED)
        };
        let pts = rl(&b, B_TS_SEC) as i64 * 1_000_000 + rl(&b, B_TS_USEC) as i64;
        Some((r32(&b, B_INDEX), used, r32(&b, B_FLAGS), pts))
    }

    /// Hands one access unit (Annex B) to the decoder. false: no free buffer yet.
    pub fn push(&mut self, data: &[u8], pts: f64) -> Result<bool, String> {
        self.reclaim();
        let Some(i) = self.out_free.pop() else {
            return Ok(false);
        };
        let m = &self.out_bufs[i];
        let n = data.len().min(m.len);
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), m.ptr, n) };
        if let Err(e) = self.qbuf(self.out_type, i as u32, n as u32, (pts * 1e6) as i64) {
            self.out_free.push(i);
            return Err(format!("QBUF: {e}"));
        }
        Ok(true)
    }

    fn reclaim(&mut self) {
        while let Some((i, ..)) = self.dqbuf(self.out_type) {
            self.out_free.push(i as usize);
        }
    }

    /// Waits up to `ms` for decoder activity; handles a resolution change.
    pub fn wait(&mut self, ms: i32) -> Result<(), String> {
        let mut p = libc::pollfd {
            fd: self.raw(),
            events: libc::POLLIN | libc::POLLOUT | libc::POLLPRI,
            revents: 0,
        };
        unsafe { libc::poll(&mut p, 1, ms) };
        if p.revents & libc::POLLPRI != 0 {
            let mut ev = vec![0u8; EVENT_SIZE];
            while ioctl(self.raw(), DQEVENT, ev.as_mut_ptr()).is_ok() {
                match r32(&ev, 0) {
                    EVENT_SOURCE_CHANGE => self.setup_capture()?,
                    EVENT_EOS => self.eos = true,
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// A decoded picture, if one is ready.
    pub fn frame(&mut self) -> Option<Frame> {
        if self.cap_count == 0 {
            return None;
        }
        loop {
            let (i, used, flags, pts) = self.dqbuf(self.cap_type)?;
            if flags & FLAG_LAST != 0 {
                self.eos = true;
            }
            if used == 0 || flags & FLAG_ERROR != 0 {
                let _ = self.qbuf(self.cap_type, i, 0, 0);
                continue;
            }
            self.held.push(i);
            return Some(Frame {
                index: i,
                pts: pts as f64 / 1e6,
            });
        }
    }

    /// Gives a picture buffer back to the decoder (after it was replaced on screen).
    pub fn recycle(&mut self, index: u32) {
        if let Some(p) = self.held.iter().position(|h| *h == index) {
            self.held.remove(p);
            let _ = self.qbuf(self.cap_type, index, 0, 0);
        }
    }

    pub fn display_id(&self, index: u32) -> u32 {
        self.ids_base + index
    }

    fn setup_capture(&mut self) -> Result<(), String> {
        let fd = self.raw();
        if self.cap_count > 0 {
            let mut t = self.cap_type.to_ne_bytes();
            let _ = ioctl(fd, STREAMOFF, t.as_mut_ptr());
            let mut rb = [0u8; 20];
            w32(&mut rb, 4, self.cap_type);
            w32(&mut rb, 8, MEM_MMAP);
            let _ = ioctl(fd, REQBUFS, rb.as_mut_ptr());
            self.held.clear();
            self.ids_base += 100; // the display forgets the old buffers' ids
        }
        let mut f = vec![0u8; FMT_SIZE];
        w32(&mut f, 0, self.cap_type);
        ioctl(fd, G_FMT, f.as_mut_ptr()).map_err(|e| format!("G_FMT: {e}"))?;
        let o = L;
        let (w, h, pix) = (r32(&f, o), r32(&f, o + 4), r32(&f, o + 8));
        let bpl = if self.mplane {
            r32(&f, o + 24)
        } else {
            r32(&f, o + 16)
        };
        if pix != PIX_NV12 && pix != PIX_YU12 {
            // ask for NV12 (what the display prefers)
            w32(&mut f, o + 8, PIX_NV12);
            ioctl(fd, S_FMT, f.as_mut_ptr()).map_err(|e| format!("S_FMT capture: {e}"))?;
        }
        let pix = r32(&f, o + 8);
        let bpl = if self.mplane {
            r32(&f, o + 24)
        } else {
            r32(&f, o + 16)
        }
        .max(bpl)
        .max(w);
        self.coded = (w, h);
        let mut ctrl = [0u8; 8];
        w32(&mut ctrl, 0, CID_MIN_BUFFERS_FOR_CAPTURE);
        let min = if ioctl(fd, G_CTRL, ctrl.as_mut_ptr()).is_ok() {
            r32(&ctrl, 4)
        } else {
            4
        };
        let mut rb = [0u8; 20];
        w32(&mut rb, 0, min + 4);
        w32(&mut rb, 4, self.cap_type);
        w32(&mut rb, 8, MEM_MMAP);
        ioctl(fd, REQBUFS, rb.as_mut_ptr()).map_err(|e| format!("Bildpuffer: {e}"))?;
        self.cap_count = r32(&rb, 0);
        let nv12 = pix == PIX_NV12;
        let (pitches, offsets) = if nv12 {
            ([bpl, bpl, 0, 0], [0, bpl * h, 0, 0])
        } else {
            let u = bpl * h;
            (
                [bpl, bpl / 2, bpl / 2, 0],
                [0, u, u + (bpl / 2) * (h / 2), 0],
            )
        };
        let mut kms = self.kms.lock().unwrap();
        for i in 0..self.cap_count {
            let mut e = [0u8; 64];
            w32(&mut e, 0, self.cap_type);
            w32(&mut e, 4, i);
            w32(&mut e, 12, (libc::O_CLOEXEC | libc::O_RDWR) as u32);
            ioctl(fd, EXPBUF, e.as_mut_ptr()).map_err(|e| format!("EXPBUF: {e}"))?;
            let dmabuf = unsafe { OwnedFd::from_raw_fd(r32(&e, 16) as i32) };
            kms.import_video(
                self.ids_base + i,
                &VideoFrameDesc {
                    fd: dmabuf,
                    width: w,
                    height: h,
                    nv12,
                    pitches,
                    offsets,
                },
            )?;
            self.qbuf(self.cap_type, i, 0, 0)
                .map_err(|e| format!("QBUF capture: {e}"))?;
        }
        drop(kms);
        let mut t = self.cap_type.to_ne_bytes();
        ioctl(fd, STREAMON, t.as_mut_ptr()).map_err(|e| format!("STREAMON capture: {e}"))?;
        crate::log(&format!(
            "Decoder: {w}x{h} {}",
            if nv12 { "NV12" } else { "I420" }
        ));
        Ok(())
    }

    /// Seek: drop everything in flight; `keep` stays on screen (not requeued).
    pub fn flush(&mut self, keep: Option<u32>) {
        let fd = self.raw();
        for typ in [self.out_type, self.cap_type] {
            let mut t = typ.to_ne_bytes();
            let _ = ioctl(fd, STREAMOFF, t.as_mut_ptr());
        }
        self.out_free = (0..self.out_bufs.len()).collect();
        let mut t = self.out_type.to_ne_bytes();
        let _ = ioctl(fd, STREAMON, t.as_mut_ptr());
        self.eos = false;
        if self.cap_count > 0 {
            self.held.clear();
            for i in 0..self.cap_count {
                if Some(i) == keep {
                    self.held.push(i);
                    continue;
                }
                let _ = self.qbuf(self.cap_type, i, 0, 0);
            }
            let mut t = self.cap_type.to_ne_bytes();
            let _ = ioctl(fd, STREAMON, t.as_mut_ptr());
        }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        let fd = self.raw();
        for typ in [self.out_type, self.cap_type] {
            let mut t = typ.to_ne_bytes();
            let _ = ioctl(fd, STREAMOFF, t.as_mut_ptr());
        }
        self.out_bufs.clear();
        self.kms.lock().unwrap().stop_video();
    }
}
