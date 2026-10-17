//! Chargement d'un fichier multimédia (ACM, MVE, WAV) depuis le disque ou une archive .DAT.

use crate::audio::Pcm;
use crate::model::{Kind, kind_of};
use crate::bink::Bink;
use crate::mve::Mve;
use crate::video::Video;
use anyhow::{Result, anyhow};
use std::path::Path;
use std::sync::Arc;

/// Interprétation du nombre de canaux d'un son.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum ChannelMode {
    /// Dossier `music` ou signal nettement stéréo : stéréo ; sinon mono (les en-têtes ACM de Fallout ne sont pas fiables).
    #[default]
    Auto,
    Mono,
    Stereo,
}

pub struct Loaded {
    pub kind: Kind,
    pub pcm: Option<Pcm>,
    pub video: Option<Arc<Video>>,
    /// Paires (libellé, valeur) pour le panneau d'informations.
    pub info: Vec<(&'static str, String)>,
    pub warning: Option<String>,
    pub size: usize,
    /// SHA-256 du fichier tel qu'il est stocké (aussi pour un fichier d'archive).
    pub sha256: String,
}

fn in_music_dir(p: &Path) -> bool {
    p.components().any(|c| c.as_os_str().to_string_lossy().eq_ignore_ascii_case("music"))
}

/// Vrai si le flux 16 bits est nettement du stéréo entrelacé : chaque canal est lisse, alors que deux échantillons
/// consécutifs (gauche puis droite) diffèrent beaucoup. En mono, c'est l'inverse.
fn looks_stereo(s: &[i16]) -> bool {
    let s = &s[..s.len().min(400_000)];
    if s.len() < 4000 {
        return false;
    }
    let sum = |lag: usize| -> f64 { s.iter().zip(&s[lag..]).map(|(a, b)| (*a as i32 - *b as i32).unsigned_abs() as f64).sum() };
    let (d1, d2) = (sum(1), sum(2));
    d2 > 0.0 && d2 < d1 * 0.7
}

pub fn load(path: &Path, mode: ChannelMode) -> Result<Loaded> {
    let kind = kind_of(path).ok_or_else(|| anyhow!("format non pris en charge"))?;
    let data = crate::dat::read_any(path)?;
    let size = data.len();
    let sha256 = crate::hash::sha256_hex(&data);
    let mut info = vec![];
    match kind {
        Kind::Acm => {
            let a = crate::acm::decode(&data, false)?;
            let mut channels = a.channels;
            let mut samples = a.samples;
            let want = match mode {
                ChannelMode::Mono => 1,
                ChannelMode::Stereo => 2,
                // Les en-têtes ACM de Fallout ne sont pas fiables : musiques stéréo, sons et voix mono, sauf signal nettement stéréo.
                ChannelMode::Auto if in_music_dir(path) || looks_stereo(&samples) => 2,
                ChannelMode::Auto => 1,
            };
            if want != channels {
                channels = want;
                if channels > 1 {
                    let n = samples.len() - samples.len() % channels as usize;
                    samples.truncate(n);
                }
            }
            info.push(("format", "ACM".to_string()));
            info.push(("header_channels", a.header_channels.to_string()));
            let pcm = Pcm { samples: Arc::new(samples), channels, rate: a.rate };
            Ok(Loaded { kind, pcm: Some(pcm), video: None, info, warning: a.warning, size, sha256 })
        }
        Kind::Wav => {
            let w = crate::wav::decode(&data)?;
            info.push(("format", "WAV PCM".to_string()));
            Ok(Loaded { kind, pcm: Some(Pcm { samples: Arc::new(w.samples), channels: w.channels, rate: w.rate }), video: None, info, warning: None, size, sha256 })
        }
        Kind::Mp3 => {
            let m = crate::mp3::decode(&data)?;
            info.push(("format", "MP3".to_string()));
            Ok(Loaded { kind, pcm: Some(Pcm { samples: Arc::new(m.samples), channels: m.channels, rate: m.rate }), video: None, info, warning: None, size, sha256 })
        }
        Kind::Mve => {
            let m = Mve::parse(data)?;
            info.push(("format", "Interplay MVE".to_string()));
            let pcm = m.audio.as_ref().map(|a| Pcm { samples: Arc::new(a.samples.clone()), channels: a.channels, rate: a.rate });
            let warning = m.warning.clone();
            Ok(Loaded { kind, pcm, video: Some(Arc::new(Video::Mve(m))), info, warning, size, sha256 })
        }
        Kind::Bik => {
            let mut b = Bink::parse(data)?;
            info.push(("format", format!("Bink 1 (rév. {})", b.revision() as char)));
            let pcm = b.audio.take().map(|a| Pcm { samples: Arc::new(a.samples), channels: a.channels, rate: a.rate });
            let warning = b.warning.clone();
            Ok(Loaded { kind, pcm, video: Some(Arc::new(Video::Bink(b))), info, warning, size, sha256 })
        }
    }
}
