//! Audio decoding (symphonia: AAC, MP3, FLAC, Vorbis, PCM -- pure Rust) to
//! interleaved stereo i16 for the ALSA output; 5.1 is mixed down.
use super::{ACodec, AudioTrack};
use symphonia::core::audio::{Channels, SampleBuffer, SignalSpec};
use symphonia::core::codecs::{
    CodecParameters, Decoder, DecoderOptions, CODEC_TYPE_AAC, CODEC_TYPE_FLAC, CODEC_TYPE_MP3,
    CODEC_TYPE_PCM_S16LE, CODEC_TYPE_VORBIS,
};

pub struct AudioDec {
    dec: Box<dyn Decoder>,
    buf: Option<SampleBuffer<f32>>,
    pub rate: u32,
}

/// AudioSpecificConfig for AAC-LC when the container has none.
pub fn make_asc(rate: u32, channels: u32) -> Vec<u8> {
    const RATES: [u32; 13] = [
        96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
    ];
    let fi = RATES.iter().position(|r| *r == rate).unwrap_or(3) as u8;
    let ch = channels.clamp(1, 7) as u8;
    vec![(2 << 3) | (fi >> 1), ((fi & 1) << 7) | (ch << 3)]
}

fn channel_mask(n: u32) -> Channels {
    match n {
        1 => Channels::FRONT_LEFT,
        2 => Channels::FRONT_LEFT | Channels::FRONT_RIGHT,
        6 => {
            Channels::FRONT_LEFT
                | Channels::FRONT_RIGHT
                | Channels::FRONT_CENTRE
                | Channels::LFE1
                | Channels::REAR_LEFT
                | Channels::REAR_RIGHT
        }
        _ => Channels::FRONT_LEFT | Channels::FRONT_RIGHT,
    }
}

impl AudioDec {
    pub fn new(t: &AudioTrack) -> Result<AudioDec, String> {
        let mut p = CodecParameters::new();
        match &t.codec {
            ACodec::Aac { asc } => {
                let asc = if asc.is_empty() {
                    make_asc(t.rate, t.channels)
                } else {
                    asc.clone()
                };
                p.for_codec(CODEC_TYPE_AAC)
                    .with_extra_data(asc.into_boxed_slice());
            }
            ACodec::Mp3 => {
                p.for_codec(CODEC_TYPE_MP3);
            }
            ACodec::Flac => {
                p.for_codec(CODEC_TYPE_FLAC);
            }
            ACodec::Vorbis => {
                p.for_codec(CODEC_TYPE_VORBIS);
            }
            ACodec::Pcm16 => {
                p.for_codec(CODEC_TYPE_PCM_S16LE)
                    .with_bits_per_coded_sample(16)
                    .with_bits_per_sample(16)
                    .with_max_frames_per_packet(1 << 16);
            }
            other => {
                return Err(format!(
                    "Ton im Format {} kann der Pi nicht abspielen",
                    other.name()
                ))
            }
        }
        p.with_sample_rate(t.rate)
            .with_channels(channel_mask(t.channels));
        let dec = symphonia::default::get_codecs()
            .make(&p, &DecoderOptions::default())
            .map_err(|e| e.to_string())?;
        Ok(AudioDec {
            dec,
            buf: None,
            rate: t.rate,
        })
    }

    /// Appends the decoded packet as stereo i16 to `out`.
    pub fn decode(&mut self, data: &[u8], out: &mut Vec<i16>) -> Result<(), String> {
        let pkt = symphonia::core::formats::Packet::new_from_slice(0, 0, 0, data);
        let decoded = match self.dec.decode(&pkt) {
            Ok(d) => d,
            Err(symphonia::core::errors::Error::DecodeError(_)) => return Ok(()), // skip a broken frame
            Err(e) => return Err(e.to_string()),
        };
        let spec: SignalSpec = *decoded.spec();
        self.rate = spec.rate;
        let frames = decoded.frames();
        if self
            .buf
            .as_ref()
            .map(|b| b.capacity() < frames * spec.channels.count())
            .unwrap_or(true)
        {
            self.buf = Some(SampleBuffer::new(decoded.capacity() as u64, spec));
        }
        let buf = self.buf.as_mut().unwrap();
        buf.copy_interleaved_ref(decoded);
        downmix(buf.samples(), spec.channels, out);
        Ok(())
    }

    pub fn reset(&mut self) {
        self.dec.reset();
    }
}

/// Any layout -> stereo i16 (centre and surrounds at -3 dB, LFE dropped).
pub fn downmix(s: &[f32], ch: Channels, out: &mut Vec<i16>) {
    let n = ch.count().max(1);
    let to16 = |v: f32| (v.clamp(-1.0, 1.0) * 32767.0) as i16;
    if n == 2 {
        out.extend(s.iter().map(|v| to16(*v)));
        return;
    }
    if n == 1 {
        for v in s {
            let x = to16(*v);
            out.push(x);
            out.push(x);
        }
        return;
    }
    // positions of the channels in the interleaved frame, in symphonia's bit order
    let mut idx = std::collections::HashMap::new();
    for (i, c) in ch.iter().enumerate() {
        idx.insert(c.bits(), i);
    }
    let at = |c: Channels| idx.get(&c.bits()).copied();
    let (fl, fr, fc) = (
        at(Channels::FRONT_LEFT),
        at(Channels::FRONT_RIGHT),
        at(Channels::FRONT_CENTRE),
    );
    let (rl, rr) = (
        at(Channels::REAR_LEFT).or(at(Channels::SIDE_LEFT)),
        at(Channels::REAR_RIGHT).or(at(Channels::SIDE_RIGHT)),
    );
    let k = std::f32::consts::FRAC_1_SQRT_2;
    let norm = 1.0 / (1.0 + k + if rl.is_some() { k } else { 0.0 });
    for f in s.chunks_exact(n) {
        let g = |i: Option<usize>| i.map(|i| f[i]).unwrap_or(0.0);
        let c = g(fc) * k;
        let l = (g(fl) + c + g(rl) * k) * norm;
        let r = (g(fr) + c + g(rr) * k) * norm;
        out.push(to16(l * 1.4));
        out.push(to16(r * 1.4));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn asc() {
        assert_eq!(super::make_asc(48000, 2), vec![0x11, 0x90]);
        assert_eq!(super::make_asc(44100, 2), vec![0x12, 0x10]);
    }
}
