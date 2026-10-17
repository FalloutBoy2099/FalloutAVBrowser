//! Vidéos Bink (Arcanum) : conteneur, codec vidéo Bink 1 (révisions f à k) et audio (DCT / RDFT).
//!
//! Le décodeur est un portage de celui de FFmpeg (Konstantin Shishkov, Peter Ross ; LGPL 2.1 ou ultérieure).
//! [`Bink::parse`] indexe le fichier ; [`BinkDecoder`] reconstruit les images dans l'ordre (la révision `b` n'est pas gérée).

use crate::bink_data::*;
use anyhow::{Result, anyhow, bail};

const FLAG_ALPHA: u32 = 0x0010_0000;
const AUD_USEDCT: u16 = 0x1000;
const AUD_STEREO: u16 = 0x2000;

pub struct BinkAudio {
    pub rate: u32,
    pub channels: u16,
    pub samples: Vec<i16>,
}

struct FrameRec {
    /// Début de l'image (audio + vidéo) et sa taille dans le fichier.
    pos: usize,
    size: usize,
    key: bool,
}

pub struct Bink {
    data: Vec<u8>,
    pub width: usize,
    pub height: usize,
    fps_num: u32,
    fps_den: u32,
    version: u8,
    flags: u32,
    frames: Vec<FrameRec>,
    pub audio: Option<BinkAudio>,
    pub warning: Option<String>,
}

fn u32le(d: &[u8], o: usize) -> Result<u32> {
    d.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).ok_or_else(|| anyhow!("fichier Bink tronqué"))
}

impl Bink {
    pub fn is_bink(d: &[u8]) -> bool {
        d.len() > 44 && &d[0..3] == b"BIK" && matches!(d[3], b'b' | b'f' | b'g' | b'h' | b'i' | b'k')
    }

    pub fn parse(data: Vec<u8>) -> Result<Bink> {
        if !Self::is_bink(&data) {
            bail!("pas un fichier Bink 1");
        }
        let version = data[3];
        if version == b'b' {
            bail!("Bink révision b non prise en charge");
        }
        let file_size = u32le(&data, 4)? as usize + 8;
        let n_frames = u32le(&data, 8)? as usize;
        let (width, height) = (u32le(&data, 20)? as usize, u32le(&data, 24)? as usize);
        let (fps_num, fps_den) = (u32le(&data, 28)?, u32le(&data, 32)?);
        let flags = u32le(&data, 36)?;
        let n_audio = u32le(&data, 40)? as usize;
        if n_frames == 0 || n_frames > 1_000_000 || width == 0 || height == 0 || width > 8192 || height > 8192 || fps_num == 0 || fps_den == 0 || n_audio > 256 {
            bail!("en-tête Bink invalide");
        }
        let mut p = 44;
        if version == b'k' {
            p += 4;
        }
        let mut tracks: Vec<(u32, u16)> = Vec::new();
        if n_audio > 0 {
            p += 4 * n_audio; // tailles maximales décodées
            for i in 0..n_audio {
                let rate = u32le(&data, p + 4 * i)?;
                tracks.push((rate & 0xFFFF, (rate >> 16) as u16));
            }
            p += 4 * n_audio; // fréquence + indicateurs
            p += 4 * n_audio; // identifiants
        }
        // Table des images : positions (bit 0 : image clé).
        let mut frames = Vec::with_capacity(n_frames);
        let mut next = u32le(&data, p)? as usize;
        p += 4;
        for i in 0..n_frames {
            let (pos, key) = (next & !1, next & 1 != 0);
            let end = if i + 1 == n_frames {
                file_size
            } else {
                let v = u32le(&data, p)? as usize;
                p += 4;
                next = v;
                v & !1
            };
            let end = end.min(data.len());
            if end <= pos || pos >= data.len() {
                bail!("table des images invalide");
            }
            frames.push(FrameRec { pos, size: end - pos, key: key || i == 0 });
        }
        let mut b = Bink { data, width, height, fps_num, fps_den, version, flags, frames, audio: None, warning: None };
        if let Some(&(rate, aflags)) = tracks.first() {
            match b.decode_audio(rate, aflags, n_audio) {
                Ok(a) => b.audio = Some(a),
                Err(e) => b.warning = Some(format!("audio : {e}")),
            }
        }
        Ok(b)
    }

    /// Portion vidéo d'une image : après les paquets audio (taille sur 4 octets + données, par piste).
    fn video_part(&self, f: &FrameRec, n_audio: usize) -> Result<(usize, usize, Vec<(usize, usize)>)> {
        let d = &self.data;
        let end = f.pos + f.size;
        let mut p = f.pos;
        let mut audio = Vec::new();
        for _ in 0..n_audio {
            let sz = u32le(d, p)? as usize;
            if p + 4 + sz > end {
                bail!("taille audio invalide");
            }
            audio.push((p + 4, sz));
            p += 4 + sz;
        }
        Ok((p, end - p, audio))
    }

    fn n_audio(&self) -> usize {
        u32le(&self.data, 40).unwrap_or(0) as usize
    }

    fn decode_audio(&self, rate: u32, aflags: u16, n_audio: usize) -> Result<BinkAudio> {
        let channels = if aflags & AUD_STEREO != 0 { 2 } else { 1 };
        let mut dec = AudioDec::new(rate, channels, aflags & AUD_USEDCT != 0, self.version == b'b')?;
        let mut out: Vec<i16> = Vec::new();
        for f in &self.frames {
            let Ok((_, _, audio)) = self.video_part(f, n_audio) else { continue };
            if let Some(&(off, sz)) = audio.first()
                && sz >= 4
            {
                dec.decode_packet(&self.data[off..off + sz], &mut out);
            }
        }
        if out.is_empty() {
            bail!("aucun échantillon décodé");
        }
        Ok(BinkAudio { rate, channels: channels as u16, samples: out })
    }

    pub fn revision(&self) -> u8 {
        self.version
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    pub fn fps(&self) -> f64 {
        self.fps_num as f64 / self.fps_den as f64
    }

    pub fn duration(&self) -> f64 {
        self.frames.len() as f64 / self.fps()
    }

    /// Dernière image clé située avant (ou à) `target`.
    pub fn key_before(&self, target: usize) -> usize {
        (0..=target.min(self.frames.len() - 1)).rev().find(|&i| self.frames[i].key).unwrap_or(0)
    }
}

// ----------------------------------------------------------------------------- lecture de bits (petit-boutiste)

struct Br<'a> {
    d: &'a [u8],
    pos: usize,
}

impl<'a> Br<'a> {
    fn new(d: &'a [u8]) -> Self {
        Br { d, pos: 0 }
    }

    #[inline]
    fn peek(&self, n: u32) -> u32 {
        let byte = self.pos >> 3;
        let mut w = 0u64;
        for i in 0..8 {
            if let Some(&b) = self.d.get(byte + i) {
                w |= (b as u64) << (8 * i);
            }
        }
        ((w >> (self.pos & 7)) & ((1u64 << n) - 1)) as u32
    }

    #[inline]
    fn get(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        let v = self.peek(n);
        self.pos += n as usize;
        v
    }

    fn bit(&mut self) -> u32 {
        self.get(1)
    }

    fn left(&self) -> isize {
        (self.d.len() * 8) as isize - self.pos as isize
    }

    fn align32(&mut self) {
        if self.pos & 31 != 0 {
            self.pos += 32 - (self.pos & 31);
        }
    }
}

// ----------------------------------------------------------------------------- arbres de Huffman

struct Vlc {
    /// (symbole, longueur) indexé par les `maxbits` prochains bits.
    table: Vec<(u8, u8)>,
    maxbits: u32,
}

impl Vlc {
    fn new(i: usize) -> Vlc {
        let (lens, bits) = (&BINK_TREE_LENS[i], &BINK_TREE_BITS[i]);
        let maxbits = lens.iter().copied().max().unwrap_or(1).max(1) as u32;
        let mut table = vec![(0u8, 0u8); 1 << maxbits];
        for s in 0..16 {
            let l = lens[s] as u32;
            if l == 0 {
                continue;
            }
            let code = bits[s] as usize;
            for idx in 0..(1usize << maxbits) {
                if idx & ((1 << l) - 1) == code {
                    table[idx] = (s as u8, l as u8);
                }
            }
        }
        Vlc { table, maxbits }
    }
}

#[derive(Clone, Copy, Default)]
struct Tree {
    vlc_num: usize,
    syms: [u8; 16],
}

fn merge(br: &mut Br, dst: &mut [u8], src: &[u8], size: usize) {
    let (mut a, mut b, mut o) = (0usize, size, 0usize);
    let (mut n1, mut n2) = (size, size);
    loop {
        if br.bit() == 0 {
            dst[o] = src[a];
            o += 1;
            a += 1;
            n1 -= 1;
        } else {
            dst[o] = src[b];
            o += 1;
            b += 1;
            n2 -= 1;
        }
        if n1 == 0 || n2 == 0 {
            break;
        }
    }
    while n1 > 0 {
        dst[o] = src[a];
        o += 1;
        a += 1;
        n1 -= 1;
    }
    while n2 > 0 {
        dst[o] = src[b];
        o += 1;
        b += 1;
        n2 -= 1;
    }
}

fn read_tree(br: &mut Br) -> Result<Tree> {
    if br.left() < 4 {
        bail!("données Bink tronquées");
    }
    let mut t = Tree { vlc_num: br.get(4) as usize, syms: [0; 16] };
    if t.vlc_num == 0 {
        for i in 0..16 {
            t.syms[i] = i as u8;
        }
        return Ok(t);
    }
    if br.bit() != 0 {
        let mut used = [false; 16];
        let mut len = br.get(3) as usize;
        for i in 0..=len {
            t.syms[i] = br.get(4) as u8;
            used[t.syms[i] as usize] = true;
        }
        for i in 0..16 {
            if len >= 15 {
                break;
            }
            if !used[i] {
                len += 1;
                t.syms[len] = i as u8;
            }
        }
    } else {
        let len = br.get(2) as usize;
        let mut a = [0u8; 16];
        let mut b = [0u8; 16];
        for i in 0..16 {
            a[i] = i as u8;
        }
        let (mut inp, mut out) = (&mut a, &mut b);
        for i in 0..=len {
            let size = 1usize << i;
            let mut t0 = 0;
            while t0 < 16 {
                merge(br, &mut out[t0..t0 + 2 * size], &inp[t0..t0 + 2 * size], size);
                t0 += size << 1;
            }
            std::mem::swap(&mut inp, &mut out);
        }
        t.syms = *inp;
    }
    Ok(t)
}

// ----------------------------------------------------------------------------- lots de valeurs (« bundles »)

const SRC_BLOCK_TYPES: usize = 0;
const SRC_SUB_BLOCK_TYPES: usize = 1;
const SRC_COLORS: usize = 2;
const SRC_PATTERN: usize = 3;
const SRC_X_OFF: usize = 4;
const SRC_Y_OFF: usize = 5;
const SRC_INTRA_DC: usize = 6;
const SRC_INTER_DC: usize = 7;
const SRC_RUN: usize = 8;
const NB_SRC: usize = 9;

const SKIP_BLOCK: u8 = 0;
const SCALED_BLOCK: u8 = 1;
const MOTION_BLOCK: u8 = 2;
const RUN_BLOCK: u8 = 3;
const RESIDUE_BLOCK: u8 = 4;
const INTRA_BLOCK: u8 = 5;
const FILL_BLOCK: u8 = 6;
const INTER_BLOCK: u8 = 7;
const PATTERN_BLOCK: u8 = 8;
const RAW_BLOCK: u8 = 9;

const RLELENS: [usize; 4] = [4, 8, 12, 32];

struct Bundle {
    len: u32,
    tree: Tree,
    data: Vec<u8>,
    cur_dec: Option<usize>,
    cur_ptr: usize,
}

fn log2(x: usize) -> u32 {
    usize::BITS - 1 - x.max(1).leading_zeros()
}

struct Plane {
    data: Vec<u8>,
    stride: usize,
}

impl Plane {
    fn new(blocks_w: usize, blocks_h: usize) -> Plane {
        let stride = (blocks_w * 8 + 8 + 31) & !31;
        Plane { data: vec![0; stride * (blocks_h * 8 + 16)], stride }
    }
}

struct Planes {
    p: [Plane; 4],
}

pub struct BinkDecoder {
    width: usize,
    height: usize,
    version: u8,
    has_alpha: bool,
    swap_planes: bool,
    cur: Planes,
    last: Planes,
    bundles: Vec<Bundle>,
    col_high: [Tree; 16],
    col_lastval: usize,
    vlcs: Vec<Vlc>,
    next: usize,
    /// Dernière image décodée (déjà affichable).
    shown: bool,
}

/// Dimensions (blocs en largeur, en hauteur) d'un plan : luminance puis chrominance.
fn plane_blocks(w: usize, h: usize, chroma: bool) -> (usize, usize) {
    if chroma { ((w + 15) >> 4, (h + 15) >> 4) } else { ((w + 7) >> 3, (h + 7) >> 3) }
}

impl BinkDecoder {
    pub fn new(b: &Bink) -> BinkDecoder {
        let mk = || {
            let (yw, yh) = plane_blocks(b.width, b.height, false);
            let (cw, ch) = plane_blocks(b.width, b.height, true);
            Planes { p: [Plane::new(yw, yh), Plane::new(cw, ch), Plane::new(cw, ch), Plane::new(yw, yh)] }
        };
        let blocks = ((b.width + 7) >> 3) * ((b.height + 7) >> 3);
        let bundles = (0..NB_SRC).map(|_| Bundle { len: 0, tree: Tree::default(), data: vec![0; blocks * 64 + 64], cur_dec: None, cur_ptr: 0 }).collect();
        BinkDecoder {
            width: b.width,
            height: b.height,
            version: b.version,
            has_alpha: b.flags & FLAG_ALPHA != 0,
            swap_planes: b.version >= b'h',
            cur: mk(),
            last: mk(),
            bundles,
            col_high: [Tree::default(); 16],
            col_lastval: 0,
            vlcs: (0..16).map(Vlc::new).collect(),
            next: 0,
            shown: false,
        }
    }

    pub fn position(&self) -> usize {
        self.next
    }

    /// Repart de l'image `frame` (une image clé : elle ne dépend d'aucune image précédente).
    pub fn seek_to(&mut self, frame: usize) {
        for pl in self.last.p.iter_mut().chain(self.cur.p.iter_mut()) {
            pl.data.fill(0);
        }
        self.next = frame;
        self.shown = false;
    }

    pub fn step(&mut self, b: &Bink) -> bool {
        let Some(f) = b.frames.get(self.next) else { return false };
        self.next += 1;
        let n_audio = b.n_audio();
        if let Ok((off, sz, _)) = b.video_part(f, n_audio)
            && self.decode_frame(&b.data[off..off + sz]).is_ok()
        {
            std::mem::swap(&mut self.cur, &mut self.last);
            self.shown = true;
        }
        true
    }

    fn get_huff(&self, br: &mut Br, t: &Tree) -> u8 {
        let v = &self.vlcs[t.vlc_num];
        let (sym, len) = v.table[br.peek(v.maxbits) as usize];
        br.pos += len.max(1) as usize;
        t.syms[sym as usize & 15]
    }

    fn init_lengths(&mut self, width: usize, bw: usize) {
        let width = (width.max(8) + 7) & !7;
        let b = &mut self.bundles;
        b[SRC_BLOCK_TYPES].len = log2((width >> 3) + 511) + 1;
        b[SRC_SUB_BLOCK_TYPES].len = log2((width >> 4) + 511) + 1;
        b[SRC_COLORS].len = log2(bw * 64 + 511) + 1;
        let l = log2((width >> 3) + 511) + 1;
        b[SRC_INTRA_DC].len = l;
        b[SRC_INTER_DC].len = l;
        b[SRC_X_OFF].len = l;
        b[SRC_Y_OFF].len = l;
        b[SRC_PATTERN].len = log2((bw << 3) + 511) + 1;
        b[SRC_RUN].len = log2(bw * 48 + 511) + 1;
    }

    fn read_bundle(&mut self, br: &mut Br, n: usize) -> Result<()> {
        if n == SRC_COLORS {
            for i in 0..16 {
                self.col_high[i] = read_tree(br)?;
            }
            self.col_lastval = 0;
        }
        if n != SRC_INTRA_DC && n != SRC_INTER_DC {
            self.bundles[n].tree = read_tree(br)?;
        }
        self.bundles[n].cur_dec = Some(0);
        self.bundles[n].cur_ptr = 0;
        Ok(())
    }

    /// Début commun de la lecture d'un lot : `None` s'il n'y a rien à lire.
    fn check_read(&mut self, br: &mut Br, n: usize) -> Option<usize> {
        let b = &mut self.bundles[n];
        match b.cur_dec {
            Some(d) if d <= b.cur_ptr => {}
            _ => return None,
        }
        let t = br.get(b.len) as usize;
        if t == 0 {
            b.cur_dec = None;
            return None;
        }
        Some(t)
    }

    fn read_runs(&mut self, br: &mut Br) -> Result<()> {
        let Some(t) = self.check_read(br, SRC_RUN) else { return Ok(()) };
        let start = self.bundles[SRC_RUN].cur_dec.unwrap();
        let end = start + t;
        if end > self.bundles[SRC_RUN].data.len() {
            bail!("valeur de série hors limites");
        }
        if br.left() < 1 {
            bail!("données Bink tronquées");
        }
        if br.bit() != 0 {
            let v = br.get(4) as u8;
            self.bundles[SRC_RUN].data[start..end].fill(v);
        } else {
            let tree = self.bundles[SRC_RUN].tree;
            for i in start..end {
                let v = self.get_huff(br, &tree);
                self.bundles[SRC_RUN].data[i] = v;
            }
        }
        self.bundles[SRC_RUN].cur_dec = Some(end);
        Ok(())
    }

    fn read_motion_values(&mut self, br: &mut Br, n: usize) -> Result<()> {
        let Some(t) = self.check_read(br, n) else { return Ok(()) };
        let start = self.bundles[n].cur_dec.unwrap();
        let end = start + t;
        if end > self.bundles[n].data.len() {
            bail!("trop de valeurs de mouvement");
        }
        if br.left() < 1 {
            bail!("données Bink tronquées");
        }
        let signed = |br: &mut Br, v: i32| -> i32 {
            if v != 0 && br.bit() != 0 { -v } else { v }
        };
        if br.bit() != 0 {
            let v = br.get(4) as i32;
            let v = signed(br, v) as i8 as u8;
            self.bundles[n].data[start..end].fill(v);
        } else {
            let tree = self.bundles[n].tree;
            for i in start..end {
                let v = self.get_huff(br, &tree) as i32;
                let v = signed(br, v);
                self.bundles[n].data[i] = v as i8 as u8;
            }
        }
        self.bundles[n].cur_dec = Some(end);
        Ok(())
    }

    fn read_block_types(&mut self, br: &mut Br, n: usize) -> Result<()> {
        let Some(mut t) = self.check_read(br, n) else { return Ok(()) };
        if self.version == b'k' {
            t ^= 0xBB;
            if t == 0 {
                self.bundles[n].cur_dec = None;
                return Ok(());
            }
        }
        let start = self.bundles[n].cur_dec.unwrap();
        let end = start + t;
        if end > self.bundles[n].data.len() {
            bail!("trop de types de blocs");
        }
        if br.left() < 1 {
            bail!("données Bink tronquées");
        }
        if br.bit() != 0 {
            let v = br.get(4) as u8;
            self.bundles[n].data[start..end].fill(v);
            self.bundles[n].cur_dec = Some(end);
        } else {
            let tree = self.bundles[n].tree;
            let (mut cur, mut last) = (start, 0u8);
            while cur < end {
                let v = self.get_huff(br, &tree);
                if v < 12 {
                    last = v;
                    self.bundles[n].data[cur] = v;
                    cur += 1;
                } else {
                    let run = RLELENS[(v - 12) as usize & 3];
                    if end - cur < run {
                        bail!("série de types de blocs trop longue");
                    }
                    self.bundles[n].data[cur..cur + run].fill(last);
                    cur += run;
                }
            }
            self.bundles[n].cur_dec = Some(cur);
        }
        Ok(())
    }

    fn read_patterns(&mut self, br: &mut Br) -> Result<()> {
        let Some(t) = self.check_read(br, SRC_PATTERN) else { return Ok(()) };
        let start = self.bundles[SRC_PATTERN].cur_dec.unwrap();
        let end = start + t;
        if end > self.bundles[SRC_PATTERN].data.len() {
            bail!("trop de motifs");
        }
        let tree = self.bundles[SRC_PATTERN].tree;
        for i in start..end {
            if br.left() < 2 {
                bail!("données Bink tronquées");
            }
            let lo = self.get_huff(br, &tree);
            let hi = self.get_huff(br, &tree);
            self.bundles[SRC_PATTERN].data[i] = lo | (hi << 4);
        }
        self.bundles[SRC_PATTERN].cur_dec = Some(end);
        Ok(())
    }

    fn read_colors(&mut self, br: &mut Br) -> Result<()> {
        let Some(t) = self.check_read(br, SRC_COLORS) else { return Ok(()) };
        let start = self.bundles[SRC_COLORS].cur_dec.unwrap();
        let end = start + t;
        if end > self.bundles[SRC_COLORS].data.len() {
            bail!("trop de couleurs");
        }
        if br.left() < 1 {
            bail!("données Bink tronquées");
        }
        let tree = self.bundles[SRC_COLORS].tree;
        let old = self.version < b'i';
        let one = |s: &mut Self, br: &mut Br| -> u8 {
            let ct = s.col_high[s.col_lastval];
            s.col_lastval = s.get_huff(br, &ct) as usize;
            let lo = s.get_huff(br, &tree);
            let mut v = ((s.col_lastval as u32) << 4) | lo as u32;
            if old {
                let sign = ((v as u8) as i8 >> 7) as i32;
                v = (((v & 0x7F) as i32 ^ sign) - sign) as u32 + 0x80;
            }
            v as u8
        };
        if br.bit() != 0 {
            let v = one(self, br);
            self.bundles[SRC_COLORS].data[start..end].fill(v);
        } else {
            for i in start..end {
                if br.left() < 2 {
                    bail!("données Bink tronquées");
                }
                let v = one(self, br);
                self.bundles[SRC_COLORS].data[i] = v;
            }
        }
        self.bundles[SRC_COLORS].cur_dec = Some(end);
        Ok(())
    }

    fn read_dcs(&mut self, br: &mut Br, n: usize, start_bits: u32, has_sign: bool) -> Result<()> {
        let Some(mut len) = self.check_read(br, n) else { return Ok(()) };
        if br.left() < (start_bits - has_sign as u32) as isize {
            bail!("données Bink tronquées");
        }
        let mut v = br.get(start_bits - has_sign as u32) as i32;
        if v != 0 && has_sign && br.bit() != 0 {
            v = -v;
        }
        let b = &mut self.bundles[n];
        let mut pos = b.cur_dec.unwrap();
        let cap = b.data.len();
        let put = |b: &mut Bundle, pos: &mut usize, v: i32| -> Result<()> {
            if *pos + 2 > cap {
                bail!("trop de valeurs DC");
            }
            b.data[*pos..*pos + 2].copy_from_slice(&(v as i16).to_le_bytes());
            *pos += 2;
            Ok(())
        };
        put(b, &mut pos, v)?;
        len -= 1;
        let mut i = 0;
        while i < len {
            let len2 = (len - i).min(8);
            let bsize = br.get(4);
            if bsize != 0 {
                for _ in 0..len2 {
                    let mut v2 = br.get(bsize) as i32;
                    if v2 != 0 && br.bit() != 0 {
                        v2 = -v2;
                    }
                    v += v2;
                    if !(-32768..=32767).contains(&v) {
                        bail!("valeur DC hors limites");
                    }
                    put(b, &mut pos, v)?;
                }
            } else {
                for _ in 0..len2 {
                    put(b, &mut pos, v)?;
                }
            }
            i += 8;
        }
        b.cur_dec = Some(pos);
        Ok(())
    }

    #[inline]
    fn get_value(&mut self, n: usize) -> i32 {
        let b = &mut self.bundles[n];
        let p = b.cur_ptr;
        match n {
            SRC_X_OFF | SRC_Y_OFF => {
                b.cur_ptr += 1;
                b.data.get(p).map_or(0, |&v| v as i8 as i32)
            }
            SRC_INTRA_DC | SRC_INTER_DC => {
                b.cur_ptr += 2;
                if p + 2 <= b.data.len() { i16::from_le_bytes([b.data[p], b.data[p + 1]]) as i32 } else { 0 }
            }
            _ => {
                b.cur_ptr += 1;
                b.data.get(p).map_or(0, |&v| v as i32)
            }
        }
    }

    fn decode_frame(&mut self, data: &[u8]) -> Result<()> {
        let mut br = Br::new(data);
        let bits_count = (data.len() * 8) as isize;
        if self.has_alpha {
            if self.version >= b'i' {
                br.pos += 32;
            }
            self.decode_plane(&mut br, 3, false)?;
        }
        if self.version >= b'i' {
            br.pos += 32;
        }
        for plane in 0..3usize {
            let idx = if plane == 0 || !self.swap_planes { plane } else { plane ^ 3 };
            self.decode_plane(&mut br, idx, plane != 0)?;
            if br.pos as isize >= bits_count {
                break;
            }
        }
        Ok(())
    }

    fn put_pixels(&mut self, pi: usize, dst: usize, prev: usize, xoff: i32, yoff: i32, bw: usize, bh: usize) -> Result<()> {
        let stride = self.cur.p[pi].stride;
        let r = prev as isize + xoff as isize + yoff as isize * stride as isize;
        let end = ((bw - 1) + stride * (bh - 1)) as isize * 8;
        if r < 0 || r > end {
            bail!("copie hors limites ({xoff}, {yoff})");
        }
        let r = r as usize;
        for j in 0..8 {
            let src: [u8; 8] = self.last.p[pi].data[r + j * stride..r + j * stride + 8].try_into().unwrap();
            self.cur.p[pi].data[dst + j * stride..dst + j * stride + 8].copy_from_slice(&src);
        }
        Ok(())
    }

    fn decode_plane(&mut self, br: &mut Br, pi: usize, chroma: bool) -> Result<()> {
        let (bw, bh) = plane_blocks(self.width, self.height, chroma);
        let width = self.width >> chroma as usize;
        let height = self.height >> chroma as usize;
        let stride = self.cur.p[pi].stride;
        if self.version == b'k' && br.bit() != 0 {
            let fill = br.get(8) as u8;
            for y in 0..height {
                self.cur.p[pi].data[y * stride..y * stride + width].fill(fill);
            }
            br.align32();
            return Ok(());
        }
        self.init_lengths(width, bw);
        for i in 0..NB_SRC {
            self.read_bundle(br, i)?;
        }
        let mut ublock = [0u8; 64];
        let mut dct = [0i32; 64];
        let mut blockr = [0i16; 64];
        for by in 0..bh {
            self.read_block_types(br, SRC_BLOCK_TYPES)?;
            self.read_block_types(br, SRC_SUB_BLOCK_TYPES)?;
            self.read_colors(br)?;
            self.read_patterns(br)?;
            self.read_motion_values(br, SRC_X_OFF)?;
            self.read_motion_values(br, SRC_Y_OFF)?;
            self.read_dcs(br, SRC_INTRA_DC, 11, false)?;
            self.read_dcs(br, SRC_INTER_DC, 11, true)?;
            self.read_runs(br)?;

            let mut bx = 0usize;
            while bx < bw {
                let dst = 8 * by * stride + 8 * bx;
                let prev = dst;
                let blk = self.get_value(SRC_BLOCK_TYPES) as u8;
                // Un bloc 16x16 sur une ligne ou une colonne impaire fait partie du bloc déjà décodé : on le saute.
                if ((by & 1) != 0 || (bx & 1) != 0) && blk == SCALED_BLOCK {
                    bx += 2;
                    continue;
                }
                match blk {
                    SKIP_BLOCK => {
                        for j in 0..8 {
                            let src: [u8; 8] = self.last.p[pi].data[prev + j * stride..prev + j * stride + 8].try_into().unwrap();
                            self.cur.p[pi].data[dst + j * stride..dst + j * stride + 8].copy_from_slice(&src);
                        }
                    }
                    SCALED_BLOCK => {
                        let sub = self.get_value(SRC_SUB_BLOCK_TYPES) as u8;
                        match sub {
                            RUN_BLOCK => {
                                if br.left() < 4 {
                                    bail!("données Bink tronquées");
                                }
                                let scan = &BINK_PATTERNS[br.get(4) as usize];
                                let mut i = 0usize;
                                let mut sp = 0usize;
                                loop {
                                    let run = self.get_value(SRC_RUN) as usize + 1;
                                    i += run;
                                    if i > 64 {
                                        bail!("série hors limites");
                                    }
                                    if br.bit() != 0 {
                                        let v = self.get_value(SRC_COLORS) as u8;
                                        for _ in 0..run {
                                            ublock[scan[sp] as usize] = v;
                                            sp += 1;
                                        }
                                    } else {
                                        for _ in 0..run {
                                            ublock[scan[sp] as usize] = self.get_value(SRC_COLORS) as u8;
                                            sp += 1;
                                        }
                                    }
                                    if i >= 63 {
                                        break;
                                    }
                                }
                                if i == 63 {
                                    ublock[scan[sp] as usize] = self.get_value(SRC_COLORS) as u8;
                                }
                            }
                            INTRA_BLOCK => {
                                dct.fill(0);
                                dct[0] = self.get_value(SRC_INTRA_DC);
                                let (q, cnt, idx) = read_dct_coeffs(br, &mut dct, &BINK_SCAN)?;
                                unquantize(&mut dct, &BINK_INTRA_QUANT[q], cnt, &idx);
                                let mut tmp = [0u8; 64];
                                idct_put(&mut tmp, 0, 8, &mut dct);
                                ublock = tmp;
                            }
                            FILL_BLOCK => {
                                let v = self.get_value(SRC_COLORS) as u8;
                                for j in 0..16 {
                                    let o = dst + j * stride;
                                    self.cur.p[pi].data[o..o + 16].fill(v);
                                }
                            }
                            PATTERN_BLOCK => {
                                let col = [self.get_value(SRC_COLORS) as u8, self.get_value(SRC_COLORS) as u8];
                                for j in 0..8 {
                                    let mut v = self.get_value(SRC_PATTERN);
                                    for i in 0..8 {
                                        ublock[i + j * 8] = col[(v & 1) as usize];
                                        v >>= 1;
                                    }
                                }
                            }
                            RAW_BLOCK => {
                                for j in 0..8 {
                                    for i in 0..8 {
                                        ublock[i + j * 8] = self.get_value(SRC_COLORS) as u8;
                                    }
                                }
                            }
                            _ => bail!("type de bloc 16x16 incorrect {sub}"),
                        }
                        if sub != FILL_BLOCK {
                            let d = &mut self.cur.p[pi].data;
                            for j in 0..8 {
                                for i in 0..8 {
                                    let v = ublock[j * 8 + i];
                                    let o = dst + 2 * j * stride + 2 * i;
                                    d[o] = v;
                                    d[o + 1] = v;
                                    d[o + stride] = v;
                                    d[o + stride + 1] = v;
                                }
                            }
                        }
                        bx += 1;
                    }
                    MOTION_BLOCK => {
                        let (x, y) = (self.get_value(SRC_X_OFF), self.get_value(SRC_Y_OFF));
                        self.put_pixels(pi, dst, prev, x, y, bw, bh)?;
                    }
                    RUN_BLOCK => {
                        let scan = &BINK_PATTERNS[br.get(4) as usize];
                        let mut i = 0usize;
                        let mut sp = 0usize;
                        loop {
                            let run = self.get_value(SRC_RUN) as usize + 1;
                            i += run;
                            if i > 64 {
                                bail!("série hors limites");
                            }
                            if br.bit() != 0 {
                                let v = self.get_value(SRC_COLORS) as u8;
                                for _ in 0..run {
                                    let k = scan[sp] as usize;
                                    self.cur.p[pi].data[dst + (k & 7) + (k >> 3) * stride] = v;
                                    sp += 1;
                                }
                            } else {
                                for _ in 0..run {
                                    let k = scan[sp] as usize;
                                    let v = self.get_value(SRC_COLORS) as u8;
                                    self.cur.p[pi].data[dst + (k & 7) + (k >> 3) * stride] = v;
                                    sp += 1;
                                }
                            }
                            if i >= 63 {
                                break;
                            }
                        }
                        if i == 63 {
                            let k = scan[sp] as usize;
                            let v = self.get_value(SRC_COLORS) as u8;
                            self.cur.p[pi].data[dst + (k & 7) + (k >> 3) * stride] = v;
                        }
                    }
                    RESIDUE_BLOCK => {
                        let (x, y) = (self.get_value(SRC_X_OFF), self.get_value(SRC_Y_OFF));
                        self.put_pixels(pi, dst, prev, x, y, bw, bh)?;
                        blockr.fill(0);
                        let v = br.get(7) as i32;
                        read_residue(br, &mut blockr, v);
                        let d = &mut self.cur.p[pi].data;
                        for j in 0..8 {
                            for i in 0..8 {
                                let o = dst + j * stride + i;
                                d[o] = d[o].wrapping_add(blockr[j * 8 + i] as u8);
                            }
                        }
                    }
                    INTRA_BLOCK => {
                        dct.fill(0);
                        dct[0] = self.get_value(SRC_INTRA_DC);
                        let (q, cnt, idx) = read_dct_coeffs(br, &mut dct, &BINK_SCAN)?;
                        unquantize(&mut dct, &BINK_INTRA_QUANT[q], cnt, &idx);
                        idct_put(&mut self.cur.p[pi].data, dst, stride, &mut dct);
                    }
                    FILL_BLOCK => {
                        let v = self.get_value(SRC_COLORS) as u8;
                        for j in 0..8 {
                            let o = dst + j * stride;
                            self.cur.p[pi].data[o..o + 8].fill(v);
                        }
                    }
                    INTER_BLOCK => {
                        let (x, y) = (self.get_value(SRC_X_OFF), self.get_value(SRC_Y_OFF));
                        self.put_pixels(pi, dst, prev, x, y, bw, bh)?;
                        dct.fill(0);
                        dct[0] = self.get_value(SRC_INTER_DC);
                        let (q, cnt, idx) = read_dct_coeffs(br, &mut dct, &BINK_SCAN)?;
                        unquantize(&mut dct, &BINK_INTER_QUANT[q], cnt, &idx);
                        idct_add(&mut self.cur.p[pi].data, dst, stride, &mut dct);
                    }
                    PATTERN_BLOCK => {
                        let col = [self.get_value(SRC_COLORS) as u8, self.get_value(SRC_COLORS) as u8];
                        for j in 0..8 {
                            let mut v = self.get_value(SRC_PATTERN);
                            for i in 0..8 {
                                self.cur.p[pi].data[dst + j * stride + i] = col[(v & 1) as usize];
                                v >>= 1;
                            }
                        }
                    }
                    RAW_BLOCK => {
                        let b = &mut self.bundles[SRC_COLORS];
                        for j in 0..8 {
                            for i in 0..8 {
                                self.cur.p[pi].data[dst + j * stride + i] = b.data.get(b.cur_ptr + j * 8 + i).copied().unwrap_or(0);
                            }
                        }
                        b.cur_ptr += 64;
                    }
                    _ => bail!("type de bloc inconnu {blk}"),
                }
                bx += 1;
            }
        }
        br.align32();
        Ok(())
    }

    /// Image courante en RGBA (YUV 4:2:0, plage limitée BT.601).
    pub fn rgba(&self) -> Vec<u8> {
        let src = if self.shown { &self.last } else { &self.cur };
        let (w, h) = (self.width, self.height);
        let (y, u, v) = (&src.p[0], &src.p[1], &src.p[2]);
        let mut out = vec![255u8; w * h * 4];
        for py in 0..h {
            for px in 0..w {
                let yy = y.data[py * y.stride + px] as i32 - 16;
                let uu = u.data[(py >> 1) * u.stride + (px >> 1)] as i32 - 128;
                let vv = v.data[(py >> 1) * v.stride + (px >> 1)] as i32 - 128;
                let c = 298 * yy;
                let r = (c + 409 * vv + 128) >> 8;
                let g = (c - 100 * uu - 208 * vv + 128) >> 8;
                let b = (c + 516 * uu + 128) >> 8;
                let o = (py * w + px) * 4;
                out[o] = r.clamp(0, 255) as u8;
                out[o + 1] = g.clamp(0, 255) as u8;
                out[o + 2] = b.clamp(0, 255) as u8;
            }
        }
        out
    }
}

// ----------------------------------------------------------------------------- coefficients DCT

fn read_dct_coeffs(br: &mut Br, block: &mut [i32; 64], scan: &[u8; 64]) -> Result<(usize, usize, [usize; 64])> {
    let mut coef_list = [0i32; 128];
    let mut mode_list = [0i32; 128];
    let mut idx = [0usize; 64];
    let mut count = 0usize;
    let (mut list_start, mut list_end) = (64usize, 64usize);
    if br.left() < 4 {
        bail!("données Bink tronquées");
    }
    for (c, m) in [(4, 0), (24, 0), (44, 0), (1, 3), (2, 3), (3, 3)] {
        coef_list[list_end] = c;
        mode_list[list_end] = m;
        list_end += 1;
    }
    let mut bits = br.get(4) as i32 - 1;
    while bits >= 0 {
        let mut list_pos = list_start;
        while list_pos < list_end {
            if (mode_list[list_pos] | coef_list[list_pos]) == 0 || br.bit() == 0 {
                list_pos += 1;
                continue;
            }
            let mut ccoef = coef_list[list_pos];
            let mode = mode_list[list_pos];
            let read_val = |br: &mut Br| -> i32 {
                if bits == 0 {
                    1 - ((br.bit() as i32) << 1)
                } else {
                    let t = (br.get(bits as u32) | (1 << bits)) as i32;
                    if br.bit() != 0 { -t } else { t }
                }
            };
            match mode {
                0 | 2 => {
                    if mode == 0 {
                        coef_list[list_pos] = ccoef + 4;
                        mode_list[list_pos] = 1;
                    } else {
                        coef_list[list_pos] = 0;
                        mode_list[list_pos] = 0;
                        list_pos += 1;
                    }
                    for _ in 0..4 {
                        if br.bit() != 0 {
                            list_start -= 1;
                            coef_list[list_start] = ccoef;
                            mode_list[list_start] = 3;
                        } else {
                            let t = read_val(br);
                            block[scan[ccoef as usize & 63] as usize] = t;
                            idx[count & 63] = ccoef as usize & 63;
                            count += 1;
                        }
                        ccoef += 1;
                    }
                }
                1 => {
                    mode_list[list_pos] = 2;
                    for _ in 0..3 {
                        ccoef += 4;
                        coef_list[list_end] = ccoef;
                        mode_list[list_end] = 2;
                        list_end += 1;
                    }
                }
                _ => {
                    let t = read_val(br);
                    block[scan[ccoef as usize & 63] as usize] = t;
                    idx[count & 63] = ccoef as usize & 63;
                    count += 1;
                    coef_list[list_pos] = 0;
                    mode_list[list_pos] = 0;
                    list_pos += 1;
                }
            }
            if list_end >= 120 || list_start == 0 {
                bail!("liste de coefficients Bink trop longue");
            }
        }
        bits -= 1;
    }
    let q = br.get(4) as usize;
    Ok((q, count.min(64), idx))
}

fn unquantize(block: &mut [i32; 64], quant: &[i32; 64], count: usize, idx: &[usize; 64]) {
    block[0] = block[0].wrapping_mul(quant[0]) >> 11;
    for &i in idx.iter().take(count) {
        let s = BINK_SCAN[i] as usize;
        block[s] = block[s].wrapping_mul(quant[i]) >> 11;
    }
}

fn read_residue(br: &mut Br, block: &mut [i16; 64], mut masks_count: i32) {
    let mut coef_list = [0i32; 128];
    let mut mode_list = [0i32; 128];
    let mut nz = [0usize; 64];
    let mut nz_count = 0usize;
    let (mut list_start, mut list_end) = (64usize, 64usize);
    for (c, m) in [(4, 0), (24, 0), (44, 0), (0, 2)] {
        coef_list[list_end] = c;
        mode_list[list_end] = m;
        list_end += 1;
    }
    let mut mask = 1i32 << br.get(3);
    while mask != 0 {
        for i in 0..nz_count {
            if br.bit() == 0 {
                continue;
            }
            let p = nz[i];
            if block[p] < 0 {
                block[p] -= mask as i16;
            } else {
                block[p] += mask as i16;
            }
            masks_count -= 1;
            if masks_count < 0 {
                return;
            }
        }
        let mut list_pos = list_start;
        while list_pos < list_end {
            if (coef_list[list_pos] | mode_list[list_pos]) == 0 || br.bit() == 0 {
                list_pos += 1;
                continue;
            }
            let mut ccoef = coef_list[list_pos];
            let mode = mode_list[list_pos];
            match mode {
                0 | 2 => {
                    if mode == 0 {
                        coef_list[list_pos] = ccoef + 4;
                        mode_list[list_pos] = 1;
                    } else {
                        coef_list[list_pos] = 0;
                        mode_list[list_pos] = 0;
                        list_pos += 1;
                    }
                    for _ in 0..4 {
                        if br.bit() != 0 {
                            list_start -= 1;
                            coef_list[list_start] = ccoef;
                            mode_list[list_start] = 3;
                        } else {
                            let p = BINK_SCAN[ccoef as usize & 63] as usize;
                            nz[nz_count & 63] = p;
                            nz_count += 1;
                            let v = if br.bit() != 0 { -mask } else { mask };
                            block[p] = v as i16;
                            masks_count -= 1;
                            if masks_count < 0 {
                                return;
                            }
                        }
                        ccoef += 1;
                    }
                }
                1 => {
                    mode_list[list_pos] = 2;
                    for _ in 0..3 {
                        ccoef += 4;
                        coef_list[list_end] = ccoef;
                        mode_list[list_end] = 2;
                        list_end += 1;
                    }
                }
                _ => {
                    let p = BINK_SCAN[ccoef as usize & 63] as usize;
                    nz[nz_count & 63] = p;
                    nz_count += 1;
                    let v = if br.bit() != 0 { -mask } else { mask };
                    block[p] = v as i16;
                    coef_list[list_pos] = 0;
                    mode_list[list_pos] = 0;
                    list_pos += 1;
                    masks_count -= 1;
                    if masks_count < 0 {
                        return;
                    }
                }
            }
            if list_end >= 120 || list_start == 0 {
                return;
            }
        }
        mask >>= 1;
    }
}

// ----------------------------------------------------------------------------- IDCT

const A1: i32 = 2896;
const A2: i32 = 2217;
const A3: i32 = 3784;
const A4: i32 = -5352;

#[inline]
fn mul(x: i32, y: i32) -> i32 {
    x.wrapping_mul(y) >> 11
}

#[inline]
fn idct_1d(s: [i32; 8]) -> [i32; 8] {
    let a0 = s[0].wrapping_add(s[4]);
    let a1 = s[0].wrapping_sub(s[4]);
    let a2 = s[2].wrapping_add(s[6]);
    let a3 = mul(A1, s[2].wrapping_sub(s[6]));
    let a4 = s[5].wrapping_add(s[3]);
    let a5 = s[5].wrapping_sub(s[3]);
    let a6 = s[1].wrapping_add(s[7]);
    let a7 = s[1].wrapping_sub(s[7]);
    let b0 = a4.wrapping_add(a6);
    let b1 = mul(A3, a5.wrapping_add(a7));
    let b2 = mul(A4, a5).wrapping_sub(b0).wrapping_add(b1);
    let b3 = mul(A1, a6.wrapping_sub(a4)).wrapping_sub(b2);
    let b4 = mul(A2, a7).wrapping_add(b3).wrapping_sub(b1);
    [
        a0.wrapping_add(a2).wrapping_add(b0),
        a1.wrapping_add(a3).wrapping_sub(a2).wrapping_add(b2),
        a1.wrapping_sub(a3).wrapping_add(a2).wrapping_add(b3),
        a0.wrapping_sub(a2).wrapping_sub(b4),
        a0.wrapping_sub(a2).wrapping_add(b4),
        a1.wrapping_sub(a3).wrapping_add(a2).wrapping_sub(b3),
        a1.wrapping_add(a3).wrapping_sub(a2).wrapping_sub(b2),
        a0.wrapping_add(a2).wrapping_sub(b0),
    ]
}

/// Passe verticale (colonnes) puis horizontale (lignes) ; le résultat de la passe horizontale est arrondi.
fn idct_rows(block: &[i32; 64]) -> [i32; 64] {
    let mut tmp = [0i32; 64];
    for i in 0..8 {
        let col = [block[i], block[i + 8], block[i + 16], block[i + 24], block[i + 32], block[i + 40], block[i + 48], block[i + 56]];
        if col[1..].iter().all(|&v| v == 0) {
            for r in 0..8 {
                tmp[i + 8 * r] = col[0];
            }
        } else {
            let o = idct_1d(col);
            for r in 0..8 {
                tmp[i + 8 * r] = o[r];
            }
        }
    }
    let mut out = [0i32; 64];
    for r in 0..8 {
        let row: [i32; 8] = tmp[8 * r..8 * r + 8].try_into().unwrap();
        let o = idct_1d(row);
        for c in 0..8 {
            out[8 * r + c] = o[c].wrapping_add(0x7F) >> 8;
        }
    }
    out
}

fn idct_put(dst: &mut [u8], off: usize, stride: usize, block: &mut [i32; 64]) {
    let o = idct_rows(block);
    for r in 0..8 {
        for c in 0..8 {
            dst[off + r * stride + c] = o[r * 8 + c] as u8;
        }
    }
}

fn idct_add(dst: &mut [u8], off: usize, stride: usize, block: &mut [i32; 64]) {
    let o = idct_rows(block);
    for r in 0..8 {
        for c in 0..8 {
            let p = &mut dst[off + r * stride + c];
            *p = p.wrapping_add(o[r * 8 + c] as u8);
        }
    }
}

// ----------------------------------------------------------------------------- audio

const CRITICAL_FREQS: [u32; 25] = [100, 200, 300, 400, 510, 630, 770, 920, 1080, 1270, 1480, 1720, 2000, 2320, 2700, 3150, 3700, 4400, 5300, 6400, 7700, 9500, 12000, 15500, 24500];
const RLE_LENGTH_TAB: [usize; 16] = [2, 3, 4, 5, 6, 8, 9, 10, 11, 12, 13, 14, 15, 16, 32, 64];

struct AudioDec {
    rdft: bool,
    version_b: bool,
    channels: usize,
    /// Canaux traités indépendamment (1 pour RDFT, dont le flux est déjà entrelacé).
    chans: usize,
    frame_len: usize,
    overlap: usize,
    bands: Vec<usize>,
    num_bands: usize,
    quant_table: [f32; 96],
    root: f32,
    previous: Vec<Vec<f32>>,
    first: bool,
}

impl AudioDec {
    fn new(rate: u32, channels: usize, dct: bool, version_b: bool) -> Result<AudioDec> {
        let rdft = !dct;
        let mut bits = if rate < 22050 {
            9
        } else if rate < 44100 {
            10
        } else {
            11
        };
        let mut rate_eff = rate as u64;
        let chans;
        if rdft {
            rate_eff *= channels as u64;
            chans = 1;
            if !version_b {
                bits += (channels as f64).log2() as u32;
            }
        } else {
            chans = channels;
        }
        let frame_len = 1usize << bits;
        let overlap = frame_len / 16;
        let root = if rdft { (2.0 / ((frame_len as f64).sqrt() * 32768.0)) as f32 } else { (frame_len as f64 / ((frame_len as f64).sqrt() * 32768.0)) as f32 };
        let mut quant_table = [0f32; 96];
        for (i, q) in quant_table.iter_mut().enumerate() {
            *q = (i as f32 * 0.152_891_65_f32).exp() * root;
        }
        let half = (rate_eff + 1) / 2;
        let mut num_bands = 1;
        while num_bands < 25 {
            if half <= CRITICAL_FREQS[num_bands - 1] as u64 {
                break;
            }
            num_bands += 1;
        }
        let mut bands = vec![0usize; num_bands + 1];
        bands[0] = 2;
        for i in 1..num_bands {
            bands[i] = ((CRITICAL_FREQS[i - 1] as u64 * frame_len as u64 / half) as usize) & !1;
        }
        bands[num_bands] = frame_len;
        Ok(AudioDec { rdft, version_b, channels, chans, frame_len, overlap, bands, num_bands, quant_table, root, previous: vec![vec![0.0; overlap]; 6], first: true })
    }

    /// Décode tous les blocs d'un paquet audio ; les échantillons sont ajoutés (entrelacés) à `out`.
    fn decode_packet(&mut self, pkt: &[u8], out: &mut Vec<i16>) {
        let mut br = Br::new(pkt);
        br.pos += 32; // taille décodée annoncée
        while br.left() > 0 {
            let mut planes: Vec<Vec<f32>> = Vec::new();
            let mut off = 0;
            let mut failed = false;
            while off < self.chans {
                let n = (self.chans - off).min(2);
                match self.decode_block(&mut br, n, off) {
                    Ok(mut p) => planes.append(&mut p),
                    Err(_) => {
                        failed = true;
                        break;
                    }
                }
                br.align32();
                off += 2;
            }
            if failed || planes.is_empty() {
                return;
            }
            let n = self.frame_len - self.overlap;
            if self.rdft {
                out.extend(planes[0][..n].iter().map(|&v| to_i16(v)));
            } else {
                for i in 0..n {
                    for p in &planes {
                        out.push(to_i16(p[i]));
                    }
                }
            }
            if br.left() <= 0 {
                return;
            }
        }
    }

    fn decode_block(&mut self, br: &mut Br, channels: usize, ch_offset: usize) -> Result<Vec<Vec<f32>>> {
        let n = self.frame_len;
        let mut outs: Vec<Vec<f32>> = Vec::new();
        let quant_idx_size = 8;
        if !self.rdft {
            br.pos += 2;
        }
        for _ in 0..channels {
            let mut coeffs = vec![0f32; n + 2];
            if self.version_b {
                if br.left() < 64 {
                    bail!("bloc audio tronqué");
                }
                coeffs[0] = f32::from_bits(br.get(32)) * self.root;
                coeffs[1] = f32::from_bits(br.get(32)) * self.root;
            } else {
                if br.left() < 58 {
                    bail!("bloc audio tronqué");
                }
                coeffs[0] = get_float(br) * self.root;
                coeffs[1] = get_float(br) * self.root;
            }
            if br.left() < (self.num_bands * quant_idx_size) as isize {
                bail!("bloc audio tronqué");
            }
            let mut quant = [0f32; 25];
            for q in quant.iter_mut().take(self.num_bands) {
                *q = self.quant_table[(br.get(quant_idx_size as u32) as usize).min(95)];
            }
            let mut k = 0usize;
            let mut q = quant[0];
            let mut i = 2usize;
            while i < n {
                let j = if self.version_b {
                    i + 16
                } else if br.bit() != 0 {
                    i + RLE_LENGTH_TAB[br.get(4) as usize] * 8
                } else {
                    i + 8
                }
                .min(n);
                let width = br.get(4);
                if width == 0 {
                    coeffs[i..j].fill(0.0);
                    i = j;
                    while k < self.bands.len() && self.bands[k] < i {
                        q = quant[k.min(24)];
                        k += 1;
                    }
                } else {
                    while i < j {
                        if k < self.bands.len() && self.bands[k] == i {
                            q = quant[k.min(24)];
                            k += 1;
                        }
                        let c = br.get(width);
                        coeffs[i] = if c != 0 {
                            if br.bit() != 0 { -q * c as f32 } else { q * c as f32 }
                        } else {
                            0.0
                        };
                        i += 1;
                    }
                }
                if br.left() < 0 {
                    bail!("bloc audio tronqué");
                }
            }
            outs.push(if self.rdft { self.inverse_rdft(&mut coeffs) } else { self.inverse_dct(&mut coeffs) });
        }
        // Fondu enchaîné sur la zone de recouvrement avec la fin du bloc précédent.
        let count = (self.overlap * channels) as f32;
        for ch in 0..channels {
            if !self.first {
                let mut j = ch;
                for i in 0..self.overlap {
                    outs[ch][i] = (self.previous[ch + ch_offset][i] * (count - j as f32) + outs[ch][i] * j as f32) / count;
                    j += channels;
                }
            }
            self.previous[ch + ch_offset].copy_from_slice(&outs[ch][n - self.overlap..n]);
        }
        self.first = false;
        Ok(outs)
    }

    /// RDFT inverse (réelle, N échantillons) : somme hermitienne, facteur 0,5.
    fn inverse_rdft(&self, c: &mut [f32]) -> Vec<f32> {
        let n = self.frame_len;
        let mut re = vec![0f64; n];
        let mut im = vec![0f64; n];
        re[0] = c[0] as f64;
        re[n / 2] = c[1] as f64;
        for k in 1..n / 2 {
            let (a, b) = (c[2 * k] as f64, -(c[2 * k + 1] as f64));
            re[k] = a;
            im[k] = b;
            re[n - k] = a;
            im[n - k] = -b;
        }
        fft(&mut re, &mut im);
        re.iter().map(|&v| (0.5 * v) as f32).collect()
    }

    /// DCT-III : y[i] = (1/N) (x0' + 2 Σ x_k cos(π k (2i+1) / 2N)), avec x0' = 2·coeffs[0].
    fn inverse_dct(&self, c: &mut [f32]) -> Vec<f32> {
        let n = self.frame_len;
        let mut re = vec![0f64; 2 * n];
        let mut im = vec![0f64; 2 * n];
        for k in 0..n {
            let x = if k == 0 { c[0] as f64 / 0.5 } else { c[k] as f64 };
            let w = if k == 0 { 1.0 } else { 2.0 };
            let ang = std::f64::consts::PI * k as f64 / (2 * n) as f64;
            re[k] = w * x * ang.cos();
            im[k] = w * x * ang.sin();
        }
        fft(&mut re, &mut im);
        re[..n].iter().map(|&v| (v / n as f64) as f32).collect()
    }
}

fn get_float(br: &mut Br) -> f32 {
    let power = br.get(5) as i32;
    let mant = br.get(23) as f32;
    let f = mant * 2f32.powi(power - 23);
    if br.bit() != 0 { -f } else { f }
}

fn to_i16(v: f32) -> i16 {
    (v * 32768.0).round().clamp(-32768.0, 32767.0) as i16
}

/// FFT complexe inverse (exposant positif, non normalisée), taille puissance de 2.
fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = 2.0 * std::f64::consts::PI / len as f64;
        let (wr, wi) = (ang.cos(), ang.sin());
        let mut i = 0;
        while i < n {
            let (mut cr, mut ci) = (1.0f64, 0.0f64);
            for k in 0..len / 2 {
                let (a, b) = (i + k, i + k + len / 2);
                let (tr, ti) = (re[b] * cr - im[b] * ci, re[b] * ci + im[b] * cr);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let nr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = nr;
            }
            i += len;
        }
        len <<= 1;
    }
}
