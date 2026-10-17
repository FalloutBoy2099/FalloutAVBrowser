//! Types de fichiers reconnus par le navigateur.

use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Audio ACM (Fallout 1/2, Interplay).
    Acm,
    /// Vidéo Interplay MVE.
    Mve,
    /// Vidéo Bink (Arcanum).
    Bik,
    /// MP3 (sons d'Arcanum et musiques de mods).
    Mp3,
    /// WAV PCM (Arcanum et sons extraits).
    Wav,
}

pub fn kind_of(p: &Path) -> Option<Kind> {
    match p.extension()?.to_string_lossy().to_ascii_lowercase().as_str() {
        "acm" => Some(Kind::Acm),
        "mve" => Some(Kind::Mve),
        "wav" => Some(Kind::Wav),
        "mp3" => Some(Kind::Mp3),
        "bik" => Some(Kind::Bik),
        _ => None,
    }
}

pub fn is_supported(p: &Path) -> bool {
    kind_of(p).is_some()
}
