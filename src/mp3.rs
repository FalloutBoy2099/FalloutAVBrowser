//! MP3 (Arcanum, mods) décodé en PCM 16 bits entrelacé avec symphonia.

use anyhow::{Result, anyhow, bail};
use std::io::Cursor;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub struct Mp3 {
    pub channels: u16,
    pub rate: u32,
    pub samples: Vec<i16>,
}

pub fn decode(data: &[u8]) -> Result<Mp3> {
    let mss = MediaSourceStream::new(Box::new(Cursor::new(data.to_vec())), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("mp3");
    let probed = symphonia::default::get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default()).map_err(|e| anyhow!("MP3 : {e}"))?;
    let mut format = probed.format;
    let track = format.default_track().ok_or_else(|| anyhow!("MP3 sans piste audio"))?;
    let id = track.id;
    let mut dec = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default()).map_err(|e| anyhow!("MP3 : {e}"))?;
    let (mut channels, mut rate) = (0u16, 0u32);
    let mut samples: Vec<i16> = Vec::new();
    let mut buf: Option<SampleBuffer<i16>> = None;
    while let Ok(pkt) = format.next_packet() {
        if pkt.track_id() != id {
            continue;
        }
        match dec.decode(&pkt) {
            Ok(audio) => {
                let spec = *audio.spec();
                channels = spec.channels.count() as u16;
                rate = spec.rate;
                if buf.as_ref().is_none_or(|b| b.capacity() < audio.capacity() * channels as usize) {
                    buf = Some(SampleBuffer::<i16>::new(audio.capacity() as u64, spec));
                }
                let b = buf.as_mut().unwrap();
                b.copy_interleaved_ref(audio);
                samples.extend_from_slice(b.samples());
                if samples.len() >= crate::audio::MAX_SAMPLES {
                    break;
                }
            }
            // Trame corrompue : on la saute.
            Err(Error::DecodeError(_)) => continue,
            Err(_) => break,
        }
    }
    if samples.is_empty() || channels == 0 || rate == 0 {
        bail!("MP3 vide ou illisible");
    }
    Ok(Mp3 { channels, rate, samples })
}
