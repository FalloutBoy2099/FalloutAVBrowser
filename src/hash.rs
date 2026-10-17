//! Empreinte SHA-256 d'un contenu, en hexadécimal minuscule.

use sha2::{Digest, Sha256};

pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}
