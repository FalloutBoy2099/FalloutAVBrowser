//! Vidéos Interplay MVE (Fallout 1/2) : conteneur, audio (PCM / DPCM) et codec vidéo 8 bits (opcode 0x11).
//!
//! [`Mve::parse`] indexe le fichier une seule fois ; [`MveDecoder`] reconstruit les images à la demande
//! (l'image précédente et celle d'avant sont nécessaires, d'où un décodage séquentiel depuis le début).

use anyhow::{Result, bail};

const SIGNATURE: &[u8] = b"Interplay MVE File\x1a\0";

pub struct MveAudio {
    pub rate: u32,
    pub channels: u16,
    pub samples: Vec<i16>,
}

/// Une image : les modifications de palette, la carte de décodage (0x0F) et les données (0x11).
#[derive(Default, Clone)]
struct FrameRec {
    pal: Vec<PalOp>,
    map: (usize, usize),
    data: (usize, usize),
}

#[derive(Clone)]
struct PalOp {
    start: usize,
    count: usize,
    /// Offset des données dans le fichier ; `compressed` : opcode 0x0D.
    off: usize,
    compressed: bool,
}

pub struct Mve {
    data: Vec<u8>,
    pub width: usize,
    pub height: usize,
    /// Durée d'une image en microsecondes.
    pub frame_us: u32,
    frames: Vec<FrameRec>,
    pub audio: Option<MveAudio>,
    /// Message si le fichier est tronqué ou contient une partie non gérée.
    pub warning: Option<String>,
}

const DPCM_TABLE: [i16; 256] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
    16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31,
    32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 47, 51, 56, 61,
    66, 72, 79, 86, 94, 102, 112, 122, 133, 145, 158, 173, 189, 206, 225, 245,
    267, 292, 318, 348, 379, 414, 452, 493, 538, 587, 640, 699, 763, 832, 908, 991,
    1081, 1180, 1288, 1405, 1534, 1673, 1826, 1993, 2175, 2373, 2590, 2826, 3084, 3365, 3672, 4008,
    4373, 4772, 5208, 5683, 6202, 6767, 7385, 8059, 8794, 9597, 10472, 11428, 12471, 13609, 14851, 16206,
    17685, 19298, 21060, 22981, 25078, 27367, 29864, 32589, -29973, -26728, -23186, -19322, -15105, -10503, -5481, -1,
    1, 1, 5481, 10503, 15105, 19322, 23186, 26728, 29973, -32589, -29864, -27367, -25078, -22981, -21060, -19298,
    -17685, -16206, -14851, -13609, -12471, -11428, -10472, -9597, -8794, -8059, -7385, -6767, -6202, -5683, -5208, -4772,
    -4373, -4008, -3672, -3365, -3084, -2826, -2590, -2373, -2175, -1993, -1826, -1673, -1534, -1405, -1288, -1180,
    -1081, -991, -908, -832, -763, -699, -640, -587, -538, -493, -452, -414, -379, -348, -318, -292,
    -267, -245, -225, -206, -189, -173, -158, -145, -133, -122, -112, -102, -94, -86, -79, -72,
    -66, -61, -56, -51, -47, -43, -42, -41, -40, -39, -38, -37, -36, -35, -34, -33,
    -32, -31, -30, -29, -28, -27, -26, -25, -24, -23, -22, -21, -20, -19, -18, -17,
    -16, -15, -14, -13, -12, -11, -10, -9, -8, -7, -6, -5, -4, -3, -2, -1,
];

fn u16le(d: &[u8], o: usize) -> usize {
    d.get(o..o + 2).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]) as usize)
}

impl Mve {
    pub fn is_mve(d: &[u8]) -> bool {
        d.starts_with(SIGNATURE)
    }

    pub fn parse(data: Vec<u8>) -> Result<Mve> {
        if !Self::is_mve(&data) {
            bail!("pas un fichier MVE");
        }
        let mut m = Mve { data: Vec::new(), width: 0, height: 0, frame_us: 66_728, frames: Vec::new(), audio: None, warning: None };
        let d = &data;
        let mut pos = 26usize;
        let (mut a_rate, mut a_ch, mut a_16, mut a_comp) = (22050u32, 1u16, false, false);
        let mut have_audio = false;
        let mut samples: Vec<i16> = Vec::new();
        let mut cur = FrameRec::default();
        let mut pred = [0i32; 2];
        'chunks: while pos + 4 <= d.len() {
            let clen = u16le(d, pos);
            let ctype = u16le(d, pos + 2);
            pos += 4;
            let end = pos + clen;
            if end > d.len() {
                m.warning = Some("fichier MVE tronqué".to_string());
            }
            let end = end.min(d.len());
            if ctype == 5 {
                break;
            }
            let mut p = pos;
            while p + 4 <= end {
                let olen = u16le(d, p);
                let op = d[p + 2];
                let ver = d[p + 3];
                let body = p + 4;
                let bend = (body + olen).min(end);
                let b = &d[body..bend];
                match op {
                    0x00 => break 'chunks,
                    0x02 if b.len() >= 6 => {
                        let rate = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as u64;
                        let sub = u16le(b, 4) as u64;
                        if rate * sub > 0 {
                            m.frame_us = (rate * sub).min(u32::MAX as u64) as u32;
                        }
                    }
                    0x03 if b.len() >= 6 => {
                        let flags = u16le(b, 2);
                        a_rate = u16le(b, 4) as u32;
                        a_ch = if flags & 1 != 0 { 2 } else { 1 };
                        a_16 = flags & 2 != 0;
                        a_comp = ver >= 1 && flags & 4 != 0;
                        have_audio = true;
                    }
                    0x05 if b.len() >= 4 => {
                        let (w, h) = (u16le(b, 0) * 8, u16le(b, 2) * 8);
                        // Les dimensions viennent du fichier : une valeur absurde ne doit pas provoquer une allocation géante.
                        if w == 0 || h == 0 || w * h > crate::audio::MAX_PIXELS {
                            bail!("dimensions vidéo invalides ({w}x{h})");
                        }
                        (m.width, m.height) = (w, h);
                        if ver >= 2 && u16le(b, 6) != 0 {
                            m.warning = Some("vidéo 16 bits non prise en charge".to_string());
                        }
                    }
                    0x08 | 0x09 if b.len() >= 6 && have_audio => {
                        let mask = u16le(b, 2);
                        let len = u16le(b, 4);
                        // Place restante : chaque octet d'un fichier piégé pourrait annoncer des milliers d'échantillons de silence.
                        let room = crate::audio::MAX_SAMPLES.saturating_sub(samples.len());
                        if room == 0 {
                            m.warning.get_or_insert_with(|| "audio tronqué (fichier trop long)".to_string());
                        }
                        if mask & 1 != 0 {
                            if op == 0x09 {
                                let n = (if a_comp { len.saturating_sub(2 * a_ch as usize) } else if a_16 { len / 2 } else { len }).min(room);
                                samples.resize(samples.len() + n, 0);
                            } else {
                                let payload = &b[6..];
                                if a_comp {
                                    let nch = a_ch as usize;
                                    if payload.len() >= 2 * nch {
                                        for c in 0..nch {
                                            pred[c] = i16::from_le_bytes([payload[2 * c], payload[2 * c + 1]]) as i32;
                                        }
                                        let mut ch = 0usize;
                                        for &byte in payload[2 * nch..].iter().take(room) {
                                            pred[ch] = (pred[ch] + DPCM_TABLE[byte as usize] as i32).clamp(-32768, 32767);
                                            samples.push(pred[ch] as i16);
                                            ch = (ch + 1) % nch;
                                        }
                                    }
                                } else if a_16 {
                                    samples.extend(payload.chunks_exact(2).take(room).map(|c| i16::from_le_bytes([c[0], c[1]])));
                                } else {
                                    samples.extend(payload.iter().take(room).map(|&v| ((v as i16) - 128) << 8));
                                }
                            }
                        }
                    }
                    0x0C | 0x0D if b.len() >= 4 => {
                        cur.pal.push(PalOp { start: u16le(b, 0), count: u16le(b, 2), off: body + 4, compressed: op == 0x0D });
                    }
                    0x0F => cur.map = (body, bend - body),
                    0x11 => cur.data = (body, bend - body),
                    0x07 => {
                        m.frames.push(std::mem::take(&mut cur));
                    }
                    _ => {}
                }
                p = body + olen;
            }
            pos = end;
        }
        if m.width == 0 || m.height == 0 {
            bail!("MVE sans vidéo");
        }
        if have_audio && (a_rate == 0 || a_rate > 768_000) {
            m.warning.get_or_insert_with(|| format!("audio ignoré : fréquence invalide ({a_rate} Hz)"));
        } else if have_audio && !samples.is_empty() {
            m.audio = Some(MveAudio { rate: a_rate, channels: a_ch, samples });
        }
        m.data = data;
        Ok(m)
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    pub fn fps(&self) -> f64 {
        1_000_000.0 / self.frame_us as f64
    }

    pub fn duration(&self) -> f64 {
        self.frames.len() as f64 * self.frame_us as f64 / 1e6
    }
}

/// Reconstruit les images dans l'ordre. `step` décode l'image suivante ; `reset` revient au début.
pub struct MveDecoder {
    cur: Vec<u8>,
    prev: Vec<u8>,
    pal: [[u8; 3]; 256],
    next: usize,
}

impl MveDecoder {
    pub fn new(m: &Mve) -> MveDecoder {
        let n = m.width * m.height;
        MveDecoder { cur: vec![0; n], prev: vec![0; n], pal: [[0; 3]; 256], next: 0 }
    }

    pub fn reset(&mut self) {
        self.cur.fill(0);
        self.prev.fill(0);
        self.pal = [[0; 3]; 256];
        self.next = 0;
    }

    /// Index de la prochaine image à décoder.
    pub fn position(&self) -> usize {
        self.next
    }

    /// Décode l'image suivante ; `false` à la fin.
    pub fn step(&mut self, m: &Mve) -> bool {
        let Some(f) = m.frames.get(self.next) else { return false };
        self.next += 1;
        let d = &m.data;
        for op in &f.pal {
            self.apply_pal(d, op);
        }
        if f.data.1 > 14 && f.map.1 > 0 {
            self.decode_video(m, &d[f.map.0..f.map.0 + f.map.1], &d[f.data.0 + 14..f.data.0 + f.data.1]);
            std::mem::swap(&mut self.cur, &mut self.prev);
            // Après l'échange, `prev` est la dernière image affichée et `cur` celle d'il y a deux images.
        }
        true
    }

    fn apply_pal(&mut self, d: &[u8], op: &PalOp) {
        let scale = |v: u8| {
            let v = v & 63;
            (v << 2) | (v >> 4)
        };
        if !op.compressed {
            for i in 0..op.count {
                let Some(rgb) = d.get(op.off + i * 3..op.off + i * 3 + 3) else { break };
                if let Some(e) = self.pal.get_mut(op.start + i) {
                    *e = [scale(rgb[0]), scale(rgb[1]), scale(rgb[2])];
                }
            }
        } else {
            // 32 groupes de 8 entrées : un masque d'un octet puis un triplet par bit levé.
            let mut p = op.off;
            for g in 0..32 {
                let Some(&mask) = d.get(p) else { break };
                p += 1;
                for bit in 0..8 {
                    if mask & (1 << bit) != 0 {
                        let Some(rgb) = d.get(p..p + 3) else { return };
                        p += 3;
                        self.pal[g * 8 + bit] = [scale(rgb[0]), scale(rgb[1]), scale(rgb[2])];
                    }
                }
            }
        }
    }

    /// Image courante en RGBA.
    pub fn rgba(&self) -> Vec<u8> {
        // Après `step`, l'image affichée est dans `prev`.
        let shown = if self.next == 0 { &self.cur } else { &self.prev };
        let mut out = Vec::with_capacity(shown.len() * 4);
        for &i in shown {
            let c = self.pal[i as usize];
            out.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
        out
    }

    fn decode_video(&mut self, m: &Mve, map: &[u8], data: &[u8]) {
        let (w, h) = (m.width, m.height);
        let bw = w / 8;
        let mut s = Stream { d: data, p: 0 };
        let nblocks = bw * (h / 8);
        let mut skip = 0usize;
        for bi in 0..nblocks {
            let nib = match map.get(bi / 2) {
                Some(&b) => {
                    if bi & 1 == 0 {
                        b & 15
                    } else {
                        b >> 4
                    }
                }
                None => return,
            };
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let (bx, by) = ((bi % bw) * 8, (bi / bw) * 8);
            match nib {
                0 => self.copy_block(true, bx, by, 0, 0, w),
                1 => {}
                2 | 3 => {
                    let b = s.u8() as i32;
                    let (x, y) = if b < 56 { (8 + b % 7, b / 7) } else { (-14 + (b - 56) % 29, 8 + (b - 56) / 29) };
                    let (x, y) = if nib == 3 { (-x, -y) } else { (x, y) };
                    self.copy_block(false, bx, by, x, y, w);
                }
                4 => {
                    let b = s.u8() as i32;
                    self.copy_block(true, bx, by, (b & 15) - 8, (b >> 4) - 8, w);
                }
                5 => {
                    let x = s.u8() as i8 as i32;
                    let y = s.u8() as i8 as i32;
                    self.copy_block(true, bx, by, x, y, w);
                }
                6 => skip = 2,
                7 => {
                    let (p0, p1) = (s.u8(), s.u8());
                    if p0 <= p1 {
                        for y in 0..8 {
                            let bits = s.u8();
                            for x in 0..8 {
                                self.put(bx + x, by + y, w, if bits & (1 << x) != 0 { p1 } else { p0 });
                            }
                        }
                    } else {
                        let bits = u16::from_le_bytes([s.u8(), s.u8()]);
                        for k in 0..16 {
                            self.fill(bx + (k % 4) * 2, by + (k / 4) * 2, 2, 2, w, if bits & (1 << k) != 0 { p1 } else { p0 });
                        }
                    }
                }
                8 => {
                    let (p0, p1) = (s.u8(), s.u8());
                    if p0 <= p1 {
                        // Quatre quadrants (haut-gauche, bas-gauche, haut-droite, bas-droite), 2 couleurs et 16 bits chacun.
                        let quad = |s: &mut Stream, qx: usize, qy: usize, p0: u8, p1: u8, me: &mut Self| {
                            let (b0, b1) = (s.u8(), s.u8());
                            let bits = u16::from_le_bytes([b0, b1]);
                            for i in 0..16 {
                                me.put(bx + qx + i % 4, by + qy + i / 4, w, if bits & (1 << i) != 0 { p1 } else { p0 });
                            }
                        };
                        quad(&mut s, 0, 0, p0, p1, self);
                        let (p2, p3) = (s.u8(), s.u8());
                        quad(&mut s, 0, 4, p2, p3, self);
                        let (p4, p5) = (s.u8(), s.u8());
                        quad(&mut s, 4, 0, p4, p5, self);
                        let (p6, p7) = (s.u8(), s.u8());
                        quad(&mut s, 4, 4, p6, p7, self);
                    } else {
                        let b = s.take::<4>();
                        let (p2, p3) = (s.u8(), s.u8());
                        let b2 = s.take::<4>();
                        let left_right = p2 <= p3;
                        let half = |pa: u8, pb: u8, bytes: [u8; 4], idx: usize, me: &mut Self| {
                            let bits = u32::from_le_bytes(bytes);
                            for i in 0..32 {
                                let (x, y) = if left_right { (idx * 4 + i % 4, i / 4) } else { (i % 8, idx * 4 + i / 8) };
                                me.put(bx + x, by + y, w, if bits & (1u32 << i) != 0 { pb } else { pa });
                            }
                        };
                        half(p0, p1, b, 0, self);
                        half(p2, p3, b2, 1, self);
                    }
                }
                9 => {
                    let p = [s.u8(), s.u8(), s.u8(), s.u8()];
                    let (a, b) = (p[0] <= p[1], p[2] <= p[3]);
                    // (largeur, hauteur) d'un « pixel » et nombre d'octets de motif.
                    let (pw, ph, n) = match (a, b) {
                        (true, true) => (1, 1, 16),
                        (true, false) => (2, 2, 4),
                        (false, true) => (2, 1, 8),
                        (false, false) => (1, 2, 8),
                    };
                    let per_row = 8 / pw;
                    let mut cell = 0usize;
                    for _ in 0..n {
                        let byte = s.u8();
                        for k in 0..4 {
                            let v = (byte >> (2 * k)) & 3;
                            let (cx, cy) = (cell % per_row, cell / per_row);
                            self.fill(bx + cx * pw, by + cy * ph, pw, ph, w, p[v as usize]);
                            cell += 1;
                        }
                    }
                }
                10 => {
                    let p = [s.u8(), s.u8(), s.u8(), s.u8()];
                    if p[0] <= p[1] {
                        // Quatre quadrants (haut-gauche, bas-gauche, haut-droite, bas-droite), 4 couleurs, 1x1.
                        let mut first = Some(p);
                        for (qx, qy) in [(0, 0), (0, 4), (4, 0), (4, 4)] {
                            let pp = first.take().unwrap_or_else(|| [s.u8(), s.u8(), s.u8(), s.u8()]);
                            let b = s.take::<4>();
                            for i in 0..16 {
                                let v = (b[i / 4] >> (2 * (i % 4))) & 3;
                                self.put(bx + qx + i % 4, by + qy + i / 4, w, pp[v as usize]);
                            }
                        }
                    } else {
                        let b0 = s.take::<8>();
                        let q = [s.u8(), s.u8(), s.u8(), s.u8()];
                        let b1 = s.take::<8>();
                        let left_right = q[0] <= q[1];
                        for (idx, (pp, bytes)) in [(p, b0), (q, b1)].into_iter().enumerate() {
                            for i in 0..32 {
                                let v = (bytes[i / 4] >> (2 * (i % 4))) & 3;
                                let (x, y) = if left_right { (idx * 4 + i % 4, i / 4) } else { (i % 8, idx * 4 + i / 8) };
                                self.put(bx + x, by + y, w, pp[v as usize]);
                            }
                        }
                    }
                }
                11 => {
                    for y in 0..8 {
                        for x in 0..8 {
                            let v = s.u8();
                            self.put(bx + x, by + y, w, v);
                        }
                    }
                }
                12 => {
                    for y in 0..4 {
                        for x in 0..4 {
                            let v = s.u8();
                            self.fill(bx + x * 2, by + y * 2, 2, 2, w, v);
                        }
                    }
                }
                13 => {
                    for y in 0..2 {
                        for x in 0..2 {
                            let v = s.u8();
                            self.fill(bx + x * 4, by + y * 4, 4, 4, w, v);
                        }
                    }
                }
                14 => {
                    let v = s.u8();
                    self.fill(bx, by, 8, 8, w, v);
                }
                _ => {
                    let (p0, p1) = (s.u8(), s.u8());
                    for y in 0..8 {
                        for x in 0..8 {
                            self.put(bx + x, by + y, w, if (x + y) & 1 == 0 { p0 } else { p1 });
                        }
                    }
                }
            }
        }
    }

    #[inline]
    fn put(&mut self, x: usize, y: usize, w: usize, v: u8) {
        self.cur[y * w + x] = v;
    }

    fn fill(&mut self, x: usize, y: usize, bw: usize, bh: usize, w: usize, v: u8) {
        for j in 0..bh {
            let o = (y + j) * w + x;
            self.cur[o..o + bw].fill(v);
        }
    }

    /// Copie un bloc 8x8 depuis l'image précédente (`from_prev`) ou depuis le tampon en cours de construction.
    fn copy_block(&mut self, from_prev: bool, bx: usize, by: usize, dx: i32, dy: i32, w: usize) {
        let h = self.cur.len() / w;
        let (sx, sy) = (bx as i32 + dx, by as i32 + dy);
        if sx < 0 || sy < 0 || sx as usize + 8 > w || sy as usize + 8 > h {
            return;
        }
        let (sx, sy) = (sx as usize, sy as usize);
        for j in 0..8 {
            let (so, dof) = ((sy + j) * w + sx, (by + j) * w + bx);
            if from_prev {
                let src: [u8; 8] = self.prev[so..so + 8].try_into().unwrap();
                self.cur[dof..dof + 8].copy_from_slice(&src);
            } else {
                self.cur.copy_within(so..so + 8, dof);
            }
        }
    }
}

struct Stream<'a> {
    d: &'a [u8],
    p: usize,
}

impl Stream<'_> {
    /// Octet suivant ; 0 une fois les données épuisées (flux corrompu : l'image reste affichable).
    fn u8(&mut self) -> u8 {
        let v = self.d.get(self.p).copied().unwrap_or(0);
        self.p += 1;
        v
    }

    fn take<const N: usize>(&mut self) -> [u8; N] {
        let mut a = [0u8; N];
        for v in a.iter_mut() {
            *v = self.u8();
        }
        a
    }
}
