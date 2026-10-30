//! Empreinte SHA-256 d'un contenu, en hexadécimal minuscule.

use sha2::{Digest, Sha256};

pub fn sha256_hex(data: &[u8]) -> String {
    sha256_hex_progress(data, |_| {})
}

/// Comme [`sha256_hex`], en signalant la part déjà traitée (0.0 à 1.0) : un fichier de plusieurs Go prend plusieurs secondes.
pub fn sha256_hex_progress(data: &[u8], mut progress: impl FnMut(f32)) -> String {
    let mut h = Sha256::new();
    for (i, chunk) in data.chunks(16 << 20).enumerate() {
        h.update(chunk);
        progress(((i + 1) * (16 << 20)).min(data.len()) as f32 / data.len().max(1) as f32);
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}
