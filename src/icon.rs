//! Icône de l'application : l'illustration `assets/icon_source.webp` (le Vault Boy devant son terminal), préparée par
//! `assets/genicon.rs` en `assets/icon128.png` (icône de fenêtre, embarquée ici) et `assets/icon.ico` (exécutable Windows).

/// Icône de fenêtre : (pixels RGBA, largeur, hauteur).
pub fn window_icon() -> (Vec<u8>, u32, u32) {
    let img = image::load_from_memory(include_bytes!("../assets/icon128.png")).expect("assets/icon128.png").to_rgba8();
    let (w, h) = (img.width(), img.height());
    (img.into_raw(), w, h)
}

/// Fichier .ico (images BMP 32 bits non compressées) à partir d'images RGBA carrées `(côté, pixels)`.
pub fn to_ico(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut bodies: Vec<Vec<u8>> = Vec::new();
    for (s, rgba) in images {
        let s = *s;
        let mut d = Vec::new();
        d.extend_from_slice(&40u32.to_le_bytes());
        d.extend_from_slice(&s.to_le_bytes());
        d.extend_from_slice(&(s * 2).to_le_bytes());
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&32u16.to_le_bytes());
        d.extend_from_slice(&[0u8; 24]);
        for y in (0..s).rev() {
            for x in 0..s {
                let o = ((y * s + x) * 4) as usize;
                d.extend_from_slice(&[rgba[o + 2], rgba[o + 1], rgba[o], rgba[o + 3]]);
            }
        }
        // Masque AND (inutilisé avec l'alpha) : lignes alignées sur 4 octets.
        let row = (s.div_ceil(32) * 4) as usize;
        d.extend(std::iter::repeat_n(0u8, row * s as usize));
        bodies.push(d);
    }
    let mut ico = Vec::new();
    ico.extend_from_slice(&[0, 0, 1, 0]);
    ico.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len();
    for (i, (s, _)) in images.iter().enumerate() {
        ico.push(if *s >= 256 { 0 } else { *s as u8 });
        ico.push(if *s >= 256 { 0 } else { *s as u8 });
        ico.extend_from_slice(&[0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&(bodies[i].len() as u32).to_le_bytes());
        ico.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += bodies[i].len();
    }
    for b in bodies {
        ico.extend_from_slice(&b);
    }
    ico
}
