//! Décodeur ACM (Interplay), port du décodeur de référence libacm (Marko Kreen, licence ISC).
//!
//! Flux de bits en ordre « petit-boutiste » : un en-tête, puis des blocs de `rows × (1 << level)` valeurs dont
//! chaque colonne est codée par un « filler » (code de 5 bits), puis une transformée inverse (« juggle »).

use anyhow::{Result, anyhow, bail};

const ACM_ID: u32 = 0x032897;
const WAVC_ID: u32 = 0x564157;
const MAX_BLOCK_LEN: usize = 1024 * 1024;
const MAX_SAMPLES: u32 = 1 << 30;

pub struct AcmAudio {
    pub channels: u16,
    pub rate: u32,
    /// Échantillons 16 bits entrelacés.
    pub samples: Vec<i16>,
    /// Nombre de canaux annoncé par l'en-tête (peut être faux pour les musiques).
    pub header_channels: u16,
    /// Message si le flux est corrompu ou tronqué (les échantillons déjà décodés restent utilisables).
    pub warning: Option<String>,
}

struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
    acc: u64,
    n: u32,
    /// Lecture au-delà de la fin : le dernier bloc est complété par des zéros (comme libacm).
    over: bool,
}

impl<'a> Bits<'a> {
    fn new(d: &'a [u8]) -> Self {
        Bits { d, pos: 0, acc: 0, n: 0, over: false }
    }

    /// Lit `bits` (≤ 24) bits ; `None` à la fin des données.
    fn get(&mut self, bits: u32) -> Option<u32> {
        while self.n < bits {
            let b = match self.d.get(self.pos) {
                Some(&b) => b,
                None if self.over => return None,
                None => {
                    self.over = true;
                    0
                }
            };
            self.pos += 1;
            self.acc |= (b as u64) << self.n;
            self.n += 8;
        }
        let v = (self.acc & ((1u64 << bits) - 1)) as u32;
        self.acc >>= bits;
        self.n -= bits;
        Some(v)
    }
}

const MAP_1: [i32; 2] = [-1, 1];
const MAP_2_NEAR: [i32; 4] = [-2, -1, 1, 2];
const MAP_2_FAR: [i32; 4] = [-3, -2, 2, 3];
const MAP_3: [i32; 8] = [-4, -3, -2, -1, 1, 2, 3, 4];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Fill {
    Zero,
    Bad,
    Linear,
    K13,
    K12,
    K24,
    K23,
    K35,
    K34,
    K45,
    K44,
    T15,
    T27,
    T37,
}

use Fill::*;
#[rustfmt::skip]
const FILLERS: [Fill; 32] = [
    Zero, Bad, Bad, Linear, Linear, Linear, Linear, Linear,
    Linear, Linear, Linear, Linear, Linear, Linear, Linear, Linear,
    Linear, K13, K12, T15, K24, K23, T27, K35,
    K34, Bad, K45, K44, Bad, T37, Bad, Bad,
];

const MID: usize = 0x8000;

struct Decoder {
    level: u32,
    rows: usize,
    cols: usize,
    block: Vec<i32>,
    wrap: Vec<i32>,
    mid: Vec<i32>,
}

impl Decoder {
    #[inline]
    fn set(&mut self, r: usize, c: usize, idx: i32) {
        let v = self.mid[(MID as i32 + idx) as usize];
        self.block[(r << self.level) + c] = v;
    }

    /// Remplit une colonne. `None` : données épuisées ; `Some(false)` : code invalide.
    fn fill_col(&mut self, br: &mut Bits, kind: Fill, ind: u32, col: usize) -> Option<bool> {
        let rows = self.rows;
        let mut i = 0usize;
        match kind {
            Zero => {
                for r in 0..rows {
                    self.set(r, col, 0);
                }
            }
            Bad => return Some(false),
            Linear => {
                let middle = 1i32 << (ind - 1);
                for r in 0..rows {
                    let v = br.get(ind)? as i32;
                    self.set(r, col, v - middle);
                }
            }
            // Préfixe commun : 0 = deux zéros, 10 = un zéro ; le reste dépend du filler.
            K13 | K24 | K35 | K45 => {
                while i < rows {
                    if br.get(1)? == 0 {
                        self.set(i, col, 0);
                        i += 1;
                        if i >= rows {
                            break;
                        }
                        self.set(i, col, 0);
                        i += 1;
                        continue;
                    }
                    if br.get(1)? == 0 {
                        self.set(i, col, 0);
                        i += 1;
                        continue;
                    }
                    let v = match kind {
                        K13 => MAP_1[br.get(1)? as usize],
                        K24 => MAP_2_NEAR[br.get(2)? as usize],
                        K35 => {
                            if br.get(1)? == 0 {
                                MAP_1[br.get(1)? as usize]
                            } else {
                                MAP_2_FAR[br.get(2)? as usize]
                            }
                        }
                        _ => MAP_3[br.get(3)? as usize],
                    };
                    self.set(i, col, v);
                    i += 1;
                }
            }
            // Préfixe commun : 0 = un zéro.
            K12 | K23 | K34 | K44 => {
                while i < rows {
                    if br.get(1)? == 0 {
                        self.set(i, col, 0);
                        i += 1;
                        continue;
                    }
                    let v = match kind {
                        K12 => MAP_1[br.get(1)? as usize],
                        K23 => MAP_2_NEAR[br.get(2)? as usize],
                        K34 => {
                            if br.get(1)? == 0 {
                                MAP_1[br.get(1)? as usize]
                            } else {
                                MAP_2_FAR[br.get(2)? as usize]
                            }
                        }
                        _ => MAP_3[br.get(3)? as usize],
                    };
                    self.set(i, col, v);
                    i += 1;
                }
            }
            T15 | T27 => {
                let (nbits, base, off) = if kind == T15 { (5, 3u32, 1i32) } else { (7, 5u32, 2i32) };
                while i < rows {
                    let v = br.get(nbits)?;
                    if v >= base * base * base {
                        return Some(false);
                    }
                    let n1 = (v % base) as i32 - off;
                    let t = v / base;
                    let n2 = (t % base) as i32 - off;
                    let n3 = (t / base) as i32 - off;
                    for n in [n1, n2, n3] {
                        if i >= rows {
                            break;
                        }
                        self.set(i, col, n);
                        i += 1;
                    }
                }
            }
            T37 => {
                while i < rows {
                    let v = br.get(7)?;
                    if v >= 121 {
                        return Some(false);
                    }
                    let n1 = (v % 11) as i32 - 5;
                    let n2 = (v / 11) as i32 - 5;
                    for n in [n1, n2] {
                        if i >= rows {
                            break;
                        }
                        self.set(i, col, n);
                        i += 1;
                    }
                }
            }
        }
        Some(true)
    }

    fn juggle(wrap: &mut [i32], w0: usize, block: &mut [i32], b0: usize, sub_len: usize, sub_count: usize) {
        let mut w = w0;
        for i in 0..sub_len {
            let mut p = b0 + i;
            let mut r0 = wrap[w];
            let mut r1 = wrap[w + 1];
            for _ in 0..sub_count / 2 {
                let r2 = block[p];
                block[p] = r1.wrapping_mul(2).wrapping_add(r0.wrapping_add(r2));
                p += sub_len;
                let r3 = block[p];
                block[p] = r2.wrapping_mul(2).wrapping_sub(r1.wrapping_add(r3));
                p += sub_len;
                r0 = r2;
                r1 = r3;
            }
            wrap[w] = r0;
            wrap[w + 1] = r1;
            w += 2;
        }
    }

    fn juggle_block(&mut self) {
        if self.level == 0 {
            return;
        }
        let step = if self.level > 9 { 1 } else { (2048usize >> self.level) - 2 };
        let mut todo = self.rows;
        let mut block_p = 0usize;
        loop {
            let mut wrap_p = 0usize;
            let mut sub_count = step.min(todo);
            let mut sub_len = self.cols / 2;
            sub_count *= 2;
            Self::juggle(&mut self.wrap, wrap_p, &mut self.block, block_p, sub_len, sub_count);
            wrap_p += sub_len * 2;
            let mut p = block_p;
            for _ in 0..sub_count {
                self.block[p] = self.block[p].wrapping_add(1);
                p += sub_len;
            }
            while sub_len > 1 {
                sub_len /= 2;
                sub_count *= 2;
                Self::juggle(&mut self.wrap, wrap_p, &mut self.block, block_p, sub_len, sub_count);
                wrap_p += sub_len * 2;
            }
            if todo <= step {
                break;
            }
            todo -= step;
            block_p += step << self.level;
        }
    }

    /// Décode un bloc. `Ok(false)` : fin normale des données ; `Err` : flux corrompu ou tronqué.
    fn decode_block(&mut self, br: &mut Bits) -> Result<bool> {
        if br.over || (br.pos >= br.d.len() && br.n == 0) {
            return Ok(false);
        }
        let (Some(pwr), Some(val)) = (br.get(4), br.get(16)) else { return Ok(false) };
        let count = 1usize << pwr;
        let val = val as i32;
        let mut x = 0i32;
        for i in 0..count {
            self.mid[MID + i] = x;
            x = x.wrapping_add(val);
        }
        let mut x = val.wrapping_neg();
        for i in 1..=count {
            self.mid[MID - i] = x;
            x = x.wrapping_sub(val);
        }
        for col in 0..self.cols {
            let ind = br.get(5).ok_or_else(|| anyhow!("données ACM tronquées"))?;
            match self.fill_col(br, FILLERS[ind as usize], ind, col) {
                Some(true) => {}
                Some(false) => bail!("bloc ACM corrompu"),
                None => bail!("données ACM tronquées"),
            }
        }
        self.juggle_block();
        Ok(true)
    }
}

/// Décode un fichier ACM (ou WAVC). `force_stereo` : traite un flux déclaré mono comme stéréo (musiques de Fallout).
pub fn decode(data: &[u8], force_stereo: bool) -> Result<AcmAudio> {
    let mut br = Bits::new(data);
    let hdr = |br: &mut Bits, n| br.get(n).ok_or_else(|| anyhow!("en-tête ACM tronqué"));
    let mut id = hdr(&mut br, 24)?;
    if id == WAVC_ID {
        // WAVC : 'WAV' + 'C' + 24 octets, puis l'ACM.
        if hdr(&mut br, 8)? != b'C' as u32 {
            bail!("pas un fichier ACM");
        }
        for _ in 0..12 {
            hdr(&mut br, 16)?;
        }
        id = hdr(&mut br, 24)?;
    }
    if id != ACM_ID {
        bail!("pas un fichier ACM");
    }
    if hdr(&mut br, 8)? != 1 {
        bail!("version ACM inconnue");
    }
    let lo = hdr(&mut br, 16)?;
    let hi = hdr(&mut br, 16)?;
    let total = lo + (hi << 16);
    if total == 0 || total > MAX_SAMPLES {
        bail!("nombre d'échantillons ACM invalide");
    }
    let header_channels = hdr(&mut br, 16)? as u16;
    if !(1..=2).contains(&header_channels) {
        bail!("nombre de canaux ACM invalide");
    }
    let rate = hdr(&mut br, 16)?;
    if rate < 4096 {
        bail!("fréquence ACM invalide");
    }
    let level = hdr(&mut br, 4)?;
    let rows = hdr(&mut br, 12)? as usize;
    if rows == 0 {
        bail!("en-tête ACM invalide");
    }
    let cols = 1usize << level;
    let block_len = rows * cols;
    if block_len > MAX_BLOCK_LEN {
        bail!("taille de bloc ACM invalide");
    }
    let mut d = Decoder { level, rows, cols, block: vec![0; block_len], wrap: vec![0; (2 * cols).saturating_sub(2).max(2)], mid: vec![0; 0x10000] };
    let total = total as usize;
    let mut samples: Vec<i16> = Vec::with_capacity(total);
    let mut warning = None;
    while samples.len() < total {
        match d.decode_block(&mut br) {
            Ok(true) => {}
            Ok(false) => break,
            Err(e) => {
                warning = Some(e.to_string());
                break;
            }
        }
        let take = (total - samples.len()).min(block_len);
        samples.extend(d.block[..take].iter().map(|&v| (v >> level) as i16));
    }
    if samples.is_empty() {
        return Err(anyhow!(warning.unwrap_or_else(|| "ACM vide".to_string())));
    }
    let channels = if force_stereo { 2 } else { header_channels };
    if channels > 1 {
        let n = samples.len() - samples.len() % channels as usize;
        samples.truncate(n);
    }
    Ok(AcmAudio { channels, rate, samples, header_channels, warning })
}
