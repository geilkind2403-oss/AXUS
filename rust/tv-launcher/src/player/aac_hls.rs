//! Demuxer for HLS packed AAC audio (ID3 is stripped by the segment reader).
use super::{ACodec, AudioTrack, Demuxer, Kind, Packet, StreamInfo};
use std::io::Read;

pub struct AacHls {
    src: Box<dyn Read + Send>,
    buf: Vec<u8>,
    info: StreamInfo,
    next_pts: f64,
    pending: Option<Packet>,
}

impl AacHls {
    pub fn open(src: Box<dyn Read + Send>) -> Result<AacHls, String> {
        let mut out = AacHls {
            src,
            buf: Vec::with_capacity(64 * 1024),
            info: StreamInfo {
                audio: vec![AudioTrack {
                    codec: ACodec::Aac { asc: Vec::new() },
                    rate: 0,
                    channels: 0,
                    lang: "deu".into(),
                    name: "TV Ton".into(),
                    default: true,
                }],
                ..StreamInfo::default()
            },
            next_pts: 0.0,
            pending: None,
        };
        out.pending = Some(
            out.read_packet()?
                .ok_or("ZDF-AAC-Playlist enthält keinen AAC-Ton")?,
        );
        Ok(out)
    }

    fn fill(&mut self, need: usize) -> Result<bool, String> {
        while self.buf.len() < need {
            let mut chunk = [0u8; 16 * 1024];
            match self.src.read(&mut chunk) {
                Ok(0) => return Ok(false),
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(e) => return Err(format!("ZDF-AAC-Stream: {e}")),
            }
        }
        Ok(true)
    }

    fn read_packet(&mut self) -> Result<Option<Packet>, String> {
        const RATES: [u32; 13] = [
            96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
        ];
        loop {
            if !self.fill(7)? {
                return if self.buf.is_empty() {
                    Ok(None)
                } else {
                    Err("Abgeschnittenes ADTS-Frame im ZDF-AAC-Stream".into())
                };
            }
            if self.buf[0] != 0xff || self.buf[1] & 0xf6 != 0xf0 {
                self.buf.remove(0);
                continue;
            }
            let profile = (self.buf[2] >> 6) + 1;
            let rate_index = ((self.buf[2] >> 2) & 0x0f) as usize;
            let rate = *RATES
                .get(rate_index)
                .ok_or("Unbekannte AAC-Samplerate in ZDF-Stream")?;
            let channels = (((self.buf[2] & 1) << 2) | (self.buf[3] >> 6)).max(1) as u32;
            let frame_len = (((self.buf[3] as usize & 3) << 11)
                | ((self.buf[4] as usize) << 3)
                | ((self.buf[5] as usize) >> 5)) as usize;
            let header_len = if self.buf[1] & 1 != 0 { 7 } else { 9 };
            if frame_len < header_len {
                self.buf.remove(0);
                continue;
            }
            if !self.fill(frame_len)? {
                return Err("Unvollständiges AAC-Frame im ZDF-Stream".into());
            }

            let asc = vec![
                (profile << 3) | ((rate_index as u8) >> 1),
                (((rate_index as u8) & 1) << 7) | ((channels as u8) << 3),
            ];
            let track = &mut self.info.audio[0];
            track.codec = ACodec::Aac { asc };
            track.rate = rate;
            track.channels = channels;
            let pts = self.next_pts;
            self.next_pts += 1024.0 / rate as f64;
            let payload = self.buf[header_len..frame_len].to_vec();
            self.buf.drain(..frame_len);
            return Ok(Some(Packet {
                kind: Kind::Audio(0),
                pts,
                dur: 0.0,
                key: true,
                data: payload,
            }));
        }
    }
}

impl Demuxer for AacHls {
    fn info(&self) -> &StreamInfo {
        &self.info
    }
    fn next(&mut self) -> Result<Option<Packet>, String> {
        if self.pending.is_some() {
            return Ok(self.pending.take());
        }
        self.read_packet()
    }
    fn seek(&mut self, _: f64) -> Result<f64, String> {
        Err("ZDF-Live-Ton kann nicht gesucht werden".into())
    }
    fn can_seek(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adts(payload: &[u8]) -> Vec<u8> {
        let len = payload.len() + 7;
        let mut frame = vec![
            0xff,
            0xf1,
            0x4c,
            0x80 | (((len >> 11) & 3) as u8),
            (len >> 3) as u8,
            ((len & 7) as u8) << 5 | 0x1f,
            0xfc,
        ];
        frame.extend_from_slice(payload);
        frame
    }

    #[test]
    fn parses_packed_aac_frames_and_clock() {
        let mut bytes = adts(&[1, 2, 3]);
        bytes.extend(adts(&[4, 5, 6]));
        let mut d = AacHls::open(Box::new(std::io::Cursor::new(bytes))).unwrap();
        assert_eq!(d.info().audio[0].rate, 48_000);
        assert_eq!(d.info().audio[0].channels, 2);
        assert_eq!(
            d.info().audio[0].codec,
            ACodec::Aac {
                asc: vec![0x11, 0x90]
            }
        );
        let first = d.next().unwrap().unwrap();
        let second = d.next().unwrap().unwrap();
        assert_eq!(first.pts, 0.0);
        assert!((second.pts - 1024.0 / 48_000.0).abs() < 1e-9);
        assert_eq!(first.data, vec![1, 2, 3]);
        assert_eq!(second.data, vec![4, 5, 6]);
        assert!(d.next().unwrap().is_none());
    }
}
