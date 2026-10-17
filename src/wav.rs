//! WAV : lecture PCM (8/16/24/32 bits, flottant 32 bits) et écriture PCM 16 bits.

use anyhow::{Result, bail};

pub struct Wav {
    pub channels: u16,
    pub rate: u32,
    pub samples: Vec<i16>,
}

fn u16le(d: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([d[o], d[o + 1]])
}
fn u32le(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

pub fn decode(d: &[u8]) -> Result<Wav> {
    if d.len() < 12 || &d[0..4] != b"RIFF" || &d[8..12] != b"WAVE" {
        bail!("pas un fichier WAV");
    }
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    let mut data: Option<&[u8]> = None;
    let mut p = 12;
    while p + 8 <= d.len() {
        let id = &d[p..p + 4];
        let len = u32le(d, p + 4) as usize;
        let body = p + 8;
        let end = (body + len).min(d.len());
        if id == b"fmt " && end - body >= 16 {
            fmt = Some((u16le(d, body), u16le(d, body + 2), u32le(d, body + 4), u16le(d, body + 14)));
        } else if id == b"data" {
            data = Some(&d[body..end]);
        }
        p = body + len + (len & 1);
    }
    let (tag, channels, rate, bits) = fmt.ok_or_else(|| anyhow::anyhow!("WAV sans bloc fmt"))?;
    let data = data.ok_or_else(|| anyhow::anyhow!("WAV sans données"))?;
    if channels == 0 {
        bail!("WAV sans canal");
    }
    if rate == 0 || rate > 768_000 {
        bail!("fréquence WAV invalide ({rate} Hz)");
    }
    let samples: Vec<i16> = match (tag, bits) {
        (1 | 0xFFFE, 8) => data.iter().map(|&v| ((v as i16) - 128) << 8).collect(),
        (1 | 0xFFFE, 16) => data.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect(),
        (1 | 0xFFFE, 24) => data.chunks_exact(3).map(|c| i16::from_le_bytes([c[1], c[2]])).collect(),
        (1 | 0xFFFE, 32) => data.chunks_exact(4).map(|c| i16::from_le_bytes([c[2], c[3]])).collect(),
        (3, 32) => data.chunks_exact(4).map(|c| (f32::from_le_bytes([c[0], c[1], c[2], c[3]]).clamp(-1.0, 1.0) * 32767.0) as i16).collect(),
        _ => bail!("format WAV non pris en charge (tag {tag}, {bits} bits)"),
    };
    Ok(Wav { channels, rate, samples })
}

pub fn encode(samples: &[i16], channels: u16, rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut o = Vec::with_capacity(44 + samples.len() * 2);
    o.extend_from_slice(b"RIFF");
    o.extend_from_slice(&(36 + data_len).to_le_bytes());
    o.extend_from_slice(b"WAVEfmt ");
    o.extend_from_slice(&16u32.to_le_bytes());
    o.extend_from_slice(&1u16.to_le_bytes());
    o.extend_from_slice(&channels.to_le_bytes());
    o.extend_from_slice(&rate.to_le_bytes());
    o.extend_from_slice(&(rate * channels as u32 * 2).to_le_bytes());
    o.extend_from_slice(&(channels * 2).to_le_bytes());
    o.extend_from_slice(&16u16.to_le_bytes());
    o.extend_from_slice(b"data");
    o.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        o.extend_from_slice(&s.to_le_bytes());
    }
    o
}

fn header(tag: u16, bits: u16, channels: u16, rate: u32, data_len: u32) -> Vec<u8> {
    let block = channels * bits / 8;
    let mut o = Vec::with_capacity(44);
    o.extend_from_slice(b"RIFF");
    o.extend_from_slice(&(36 + data_len).to_le_bytes());
    o.extend_from_slice(b"WAVEfmt ");
    o.extend_from_slice(&16u32.to_le_bytes());
    o.extend_from_slice(&tag.to_le_bytes());
    o.extend_from_slice(&channels.to_le_bytes());
    o.extend_from_slice(&rate.to_le_bytes());
    o.extend_from_slice(&(rate * block as u32).to_le_bytes());
    o.extend_from_slice(&block.to_le_bytes());
    o.extend_from_slice(&bits.to_le_bytes());
    o.extend_from_slice(b"data");
    o.extend_from_slice(&data_len.to_le_bytes());
    o
}

/// WAV PCM 8 bits (non signé).
pub fn encode_u8(samples: &[i16], channels: u16, rate: u32) -> Vec<u8> {
    let mut o = header(1, 8, channels, rate, samples.len() as u32);
    o.extend(samples.iter().map(|&s| ((s >> 8) + 128) as u8));
    o
}

/// WAV flottant 32 bits.
pub fn encode_f32(samples: &[i16], channels: u16, rate: u32) -> Vec<u8> {
    let mut o = header(3, 32, channels, rate, samples.len() as u32 * 4);
    for &s in samples {
        o.extend_from_slice(&(s as f32 / 32768.0).to_le_bytes());
    }
    o
}
