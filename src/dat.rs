//! Archives .DAT de Fallout 1 (DAT1, LZSS) et Fallout 2 (DAT2, zlib), lues comme des dossiers virtuels.
//!
//! Un fichier contenu dans une archive est désigné par un chemin « normal » :
//! `C:\Jeux\master.dat\art\critters\hmjmpsaa.frm`. Toutes les fonctions du programme qui prennent un
//! `Path` passent par [`read_any`] / [`exists_any`] et fonctionnent donc sans traitement particulier.

use anyhow::{Result, anyhow, bail};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DatKind {
    Dat1,
    Dat2,
    /// Archives Troika (Arcanum, Temple of Elemental Evil) : index et pied de page « 1TAD » en fin de fichier.
    Troika,
    /// Archive ZIP (chaque entrée est lue par son index).
    Zip,
    /// Archive 7z (les archives « solides » se décompressent depuis le début de leur bloc).
    SevenZ,
}

#[derive(Clone, Debug)]
pub struct DatEntry {
    /// Chemin dans l'archive, séparateur `/`, casse d'origine.
    pub path: String,
    pub flags: u32,
    pub offset: u64,
    pub size: u32,
    pub packed: u32,
}

impl DatEntry {
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

#[derive(Default)]
struct DirIndex {
    subdirs: Vec<String>,
    files: Vec<usize>,
}

pub struct Dat {
    pub path: PathBuf,
    pub kind: DatKind,
    pub entries: Vec<DatEntry>,
    lookup: HashMap<String, usize>,
    dirs: HashMap<String, DirIndex>,
    /// Lecteur ZIP / 7z ouvert à la demande et conservé (ouvrir une grosse archive coûte cher).
    handle: Mutex<Option<Handle>>,
}

enum Handle {
    Zip(zip::ZipArchive<std::io::BufReader<std::fs::File>>),
    Seven(Box<sevenz_rust2::ArchiveReader<std::fs::File>>),
}

/// Chemin interne normalisé (séparateur `/`). Les composants `.`, `..` et vides sont supprimés et `:` remplacé :
/// un nom d'entrée ne peut donc pas désigner un chemin hors du dossier de sortie lors d'une extraction ou d'une conversion.
fn norm(s: &str) -> String {
    s.replace('\\', "/").split('/').filter(|c| !c.is_empty() && *c != "." && *c != "..").map(|c| c.replace(':', "_")).collect::<Vec<_>>().join("/")
}

/// Taille maximale d'une entrée décompressée (protège d'une archive piégée).
const MAX_ENTRY: u64 = 1 << 30;

fn cache() -> &'static Mutex<HashMap<PathBuf, Arc<Dat>>> {
    static C: OnceLock<Mutex<HashMap<PathBuf, Arc<Dat>>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Extension d'archive : `.dat`, `.zip`, `.7z`, ou `.patch0` à `.patch9` (correctifs d'Arcanum, même format que `.dat`).
pub fn is_dat_ext(p: &Path) -> bool {
    p.extension()
        .map(|e| {
            let e = e.to_string_lossy().to_ascii_lowercase();
            e == "dat" || e == "zip" || e == "7z" || (e.len() == 6 && e.starts_with("patch") && e.as_bytes()[5].is_ascii_digit())
        })
        .unwrap_or(false)
}

/// Découpe `…/master.dat/art/x.frm` en (archive, chemin interne `art/x.frm`).
pub fn split(path: &Path) -> Option<(PathBuf, String)> {
    for a in path.ancestors() {
        if is_dat_ext(a) && a.is_file() {
            let rest = path.strip_prefix(a).ok()?;
            let inner: Vec<String> = rest.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
            return Some((a.to_path_buf(), inner.join("/")));
        }
    }
    None
}

/// Le chemin désigne-t-il l'intérieur d'une archive (ou l'archive elle-même) ?
pub fn is_virtual(path: &Path) -> bool {
    split(path).is_some()
}

#[derive(PartialEq, Eq, Debug)]
pub enum Stat {
    File,
    Dir,
}

/// État d'un chemin virtuel : fichier, dossier (y compris la racine de l'archive) ou inexistant.
pub fn stat(path: &Path) -> Option<Stat> {
    let (arch, inner) = split(path)?;
    let dat = Dat::open(&arch).ok()?;
    if inner.is_empty() {
        return Some(Stat::Dir);
    }
    let key = inner.to_lowercase();
    if dat.lookup.contains_key(&key) {
        Some(Stat::File)
    } else if dat.dirs.contains_key(&key) {
        Some(Stat::Dir)
    } else {
        None
    }
}

pub fn read_any(path: &Path) -> Result<Vec<u8>> {
    if let Some((arch, inner)) = split(path) {
        let dat = Dat::open(&arch)?;
        let idx = dat.find(&inner).ok_or_else(|| anyhow!("{} : absent de l'archive", path.display()))?;
        return dat.read(idx);
    }
    std::fs::read(path).map_err(|e| anyhow!("{}: {e}", path.display()))
}

pub fn exists_any(path: &Path) -> bool {
    if let Some((arch, inner)) = split(path) {
        return Dat::open(&arch).map(|d| d.find(&inner).is_some()).unwrap_or(false);
    }
    path.is_file()
}

/// Dossier réel qui contient le fichier (pour une archive : le dossier de l'archive).
pub fn real_parent(path: &Path) -> Option<PathBuf> {
    match split(path) {
        Some((arch, _)) => arch.parent().map(|p| p.to_path_buf()),
        None => path.parent().map(|p| p.to_path_buf()),
    }
}

/// Fichiers (récursifs) d'un dossier virtuel : (chemin complet, chemin relatif au dossier).
pub fn walk(dir: &Path) -> Vec<(PathBuf, PathBuf)> {
    let Some((arch, inner)) = split(dir) else { return Vec::new() };
    let Ok(dat) = Dat::open(&arch) else { return Vec::new() };
    let prefix = if inner.is_empty() { String::new() } else { format!("{}/", inner.to_lowercase()) };
    let mut v = Vec::new();
    for e in &dat.entries {
        if prefix.is_empty() || e.path.to_lowercase().starts_with(&prefix) {
            let rel = &e.path[prefix.len()..];
            let mut full = arch.clone();
            for c in e.path.split('/') {
                full.push(c);
            }
            v.push((full, PathBuf::from(rel.replace('/', std::path::MAIN_SEPARATOR_STR))));
        }
    }
    v
}

impl Dat {
    pub fn open(path: &Path) -> Result<Arc<Dat>> {
        // Le verrou reste pris pendant l'analyse : plusieurs threads qui ouvrent la même archive en même temps ne la lisent qu'une fois.
        let mut c = cache().lock().unwrap();
        if let Some(d) = c.get(path) {
            return Ok(d.clone());
        }
        let dat = Arc::new(Dat::parse(path)?);
        c.insert(path.to_path_buf(), dat.clone());
        Ok(dat)
    }

    pub fn find(&self, inner: &str) -> Option<usize> {
        self.lookup.get(&norm(inner).to_lowercase()).copied()
    }

    /// (sous-dossiers, fichiers) d'un dossier interne (`""` = racine).
    pub fn children(&self, dir: &str) -> (Vec<String>, Vec<&DatEntry>) {
        match self.dirs.get(&norm(dir).to_lowercase()) {
            Some(d) => (d.subdirs.clone(), d.files.iter().map(|&i| &self.entries[i]).collect()),
            None => (Vec::new(), Vec::new()),
        }
    }

    fn build(path: &Path, kind: DatKind, entries: Vec<DatEntry>) -> Dat {
        let mut lookup = HashMap::with_capacity(entries.len());
        let mut dirs: HashMap<String, DirIndex> = HashMap::new();
        let mut seen: HashSet<(String, String)> = HashSet::new();
        dirs.entry(String::new()).or_default();
        for (i, e) in entries.iter().enumerate() {
            let lower = e.path.to_lowercase();
            lookup.insert(lower.clone(), i);
            let comps: Vec<&str> = e.path.split('/').collect();
            // Dossiers parents.
            let mut parent = String::new();
            for c in &comps[..comps.len() - 1] {
                let child = if parent.is_empty() { c.to_lowercase() } else { format!("{parent}/{}", c.to_lowercase()) };
                if seen.insert((parent.clone(), c.to_lowercase())) {
                    dirs.entry(parent.clone()).or_default().subdirs.push(c.to_string());
                }
                dirs.entry(child.clone()).or_default();
                parent = child;
            }
            dirs.entry(parent).or_default().files.push(i);
        }
        Dat { path: path.to_path_buf(), kind, entries, lookup, dirs, handle: Mutex::new(None) }
    }

    fn parse(path: &Path) -> Result<Dat> {
        let mut f = std::fs::File::open(path).map_err(|e| anyhow!("{}: {e}", path.display()))?;
        let len = f.metadata()?.len();
        if len < 16 {
            bail!("archive DAT trop courte");
        }
        // ZIP et 7z se reconnaissent à leur signature de début de fichier.
        let mut magic = [0u8; 6];
        f.read_exact(&mut magic)?;
        f.seek(SeekFrom::Start(0))?;
        if magic.starts_with(b"PK\x03\x04") || magic.starts_with(b"PK\x05\x06") {
            return Ok(Dat::build(path, DatKind::Zip, parse_zip(path)?));
        }
        if magic == [b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C] {
            return Ok(Dat::build(path, DatKind::SevenZ, parse_sevenz(path)?));
        }
        // Troika : 16 octets d'empreinte, « 1TAD », un entier, puis la taille de l'index + du pied de page (28 octets).
        if len >= 32 {
            f.seek(SeekFrom::Start(len - 28))?;
            let mut tail = [0u8; 28];
            f.read_exact(&mut tail)?;
            let total = u32::from_le_bytes(tail[24..28].try_into().unwrap()) as u64;
            if &tail[16..20] == b"1TAD" && total >= 32 && total <= len {
                f.seek(SeekFrom::Start(len - total))?;
                let mut index = vec![0u8; (total - 28) as usize];
                f.read_exact(&mut index)?;
                let entries = parse_troika(&index)?;
                return Ok(Dat::build(path, DatKind::Troika, entries));
            }
        }
        // DAT2 : pied de page (taille de l'arbre, taille du fichier), petit-boutiste.
        f.seek(SeekFrom::Start(len - 8))?;
        let mut foot = [0u8; 8];
        f.read_exact(&mut foot)?;
        let tree_size = u32::from_le_bytes(foot[0..4].try_into().unwrap()) as u64;
        let data_size = u32::from_le_bytes(foot[4..8].try_into().unwrap()) as u64;
        if data_size == len && tree_size + 8 <= len && tree_size >= 4 {
            let base = len - data_size;
            f.seek(SeekFrom::Start(len - 8 - tree_size))?;
            let mut tree = vec![0u8; tree_size as usize];
            f.read_exact(&mut tree)?;
            let entries = parse_dat2(&tree, base)?;
            return Ok(Dat::build(path, DatKind::Dat2, entries));
        }
        // DAT1 : en-tête gros-boutiste ; l'index est en tête de fichier, on lit de plus en plus large jusqu'à pouvoir l'analyser.
        let mut size = (1u64 << 20).min(len);
        loop {
            f.seek(SeekFrom::Start(0))?;
            let mut head = vec![0u8; size as usize];
            f.read_exact(&mut head)?;
            match parse_dat1(&head) {
                Ok(entries) => return Ok(Dat::build(path, DatKind::Dat1, entries)),
                Err(e) if e.to_string().contains("tronqué") && size < len => size = (size * 4).min(len),
                Err(e) => bail!("{} : archive DAT non reconnue ({e})", path.display()),
            }
        }
    }

    /// Lecture d'une entrée de ZIP ou de 7z (lecteur ouvert à la première demande).
    fn read_archive(&self, idx: usize) -> Result<Vec<u8>> {
        let e = &self.entries[idx];
        let mut guard = self.handle.lock().unwrap();
        if guard.is_none() {
            *guard = Some(match self.kind {
                DatKind::Zip => {
                    let f = std::fs::File::open(&self.path)?;
                    Handle::Zip(zip::ZipArchive::new(std::io::BufReader::new(f)).map_err(|e| anyhow!("zip : {e}"))?)
                }
                _ => Handle::Seven(Box::new(sevenz_rust2::ArchiveReader::open(&self.path, sevenz_rust2::Password::empty()).map_err(|e| anyhow!("7z : {e}"))?)),
            });
        }
        match guard.as_mut().unwrap() {
            Handle::Zip(z) => {
                let mut f = z.by_index(e.offset as usize).map_err(|e| anyhow!("zip : {e}"))?;
                let mut out = Vec::with_capacity((e.size as usize).min(64 << 20));
                (&mut f).take(MAX_ENTRY).read_to_end(&mut out).map_err(|e| anyhow!("zip : {e}"))?;
                Ok(out)
            }
            Handle::Seven(r) => {
                let name = r.archive().files.get(e.offset as usize).map(|f| f.name.clone()).ok_or_else(|| anyhow!("entrée 7z introuvable"))?;
                r.read_file(&name).map_err(|e| anyhow!("7z : {e}"))
            }
        }
    }

    pub fn read(&self, idx: usize) -> Result<Vec<u8>> {
        let e = &self.entries[idx];
        let mut f = std::fs::File::open(&self.path)?;
        // Les tailles viennent de l'archive : on vérifie qu'elles tiennent dans le fichier avant d'allouer.
        let flen = f.metadata()?.len();
        let room = flen.checked_sub(e.offset).ok_or_else(|| anyhow!("entrée hors de l'archive"))?;
        let fits = |n: u32| if n as u64 <= room { Ok(n as usize) } else { Err(anyhow!("entrée tronquée ou taille invalide")) };
        // Capacité initiale prudente : la taille annoncée n'est pas fiable.
        let cap = (e.size as usize).min(64 << 20);
        f.seek(SeekFrom::Start(e.offset))?;
        match self.kind {
            DatKind::Zip | DatKind::SevenZ => self.read_archive(idx),
            DatKind::Troika => {
                if e.flags == 2 {
                    let mut packed = vec![0u8; fits(e.packed)?];
                    f.read_exact(&mut packed)?;
                    let mut out = Vec::with_capacity(cap);
                    flate2::read::ZlibDecoder::new(&packed[..]).take(MAX_ENTRY).read_to_end(&mut out).map_err(|e| anyhow!("zlib : {e}"))?;
                    Ok(out)
                } else {
                    let mut out = vec![0u8; fits(e.size)?];
                    f.read_exact(&mut out)?;
                    Ok(out)
                }
            }
            DatKind::Dat2 => {
                let mut packed = vec![0u8; fits(e.packed)?];
                f.read_exact(&mut packed)?;
                if e.flags == 0 {
                    return Ok(packed);
                }
                let mut out = Vec::with_capacity(cap);
                flate2::read::ZlibDecoder::new(&packed[..]).take(MAX_ENTRY).read_to_end(&mut out).map_err(|e| anyhow!("zlib : {e}"))?;
                Ok(out)
            }
            DatKind::Dat1 => {
                let kind = if e.flags == 0 { 0x10 } else { e.flags & 0xF0 };
                match kind {
                    0x20 => {
                        let mut out = vec![0u8; fits(e.size)?];
                        f.read_exact(&mut out)?;
                        Ok(out)
                    }
                    0x10 => {
                        let mut packed = vec![0u8; fits(e.packed)?];
                        f.read_exact(&mut packed)?;
                        let mut out = Vec::with_capacity(cap);
                        lzss(&packed, &mut out);
                        Ok(out)
                    }
                    0x40 => {
                        let mut raw = vec![0u8; fits(e.packed)?];
                        f.read_exact(&mut raw)?;
                        let mut out = Vec::with_capacity(cap);
                        let mut i = 0usize;
                        while i + 2 <= raw.len() && out.len() < e.size as usize {
                            let b = u16::from_be_bytes([raw[i], raw[i + 1]]);
                            i += 2;
                            let (n, stored) = ((b & 0x7FFF) as usize, b & 0x8000 != 0);
                            let end = (i + n).min(raw.len());
                            if stored {
                                out.extend_from_slice(&raw[i..end]);
                            } else {
                                lzss(&raw[i..end], &mut out);
                            }
                            i = end;
                        }
                        out.truncate(e.size as usize);
                        Ok(out)
                    }
                    k => bail!("type de compression DAT1 inconnu ({k:#X})"),
                }
            }
        }
    }
}

/// LZSS de Fallout 1 : anneau de 4096 octets rempli d'espaces, écriture à partir de 4078,
/// octet de contrôle lu du bit de poids faible vers le fort (1 = octet brut, 0 = référence sur 2 octets).
pub fn lzss(src: &[u8], out: &mut Vec<u8>) {
    let mut ring = [0x20u8; 4096];
    let mut r = 4078usize;
    let mut i = 0usize;
    while i < src.len() {
        let flags = src[i];
        i += 1;
        for bit in 0..8 {
            if i >= src.len() {
                return;
            }
            if flags >> bit & 1 == 1 {
                let c = src[i];
                i += 1;
                out.push(c);
                ring[r] = c;
                r = (r + 1) & 0xFFF;
            } else {
                if i + 1 >= src.len() {
                    return;
                }
                let (b1, b2) = (src[i] as usize, src[i + 1] as usize);
                i += 2;
                let off = b1 | ((b2 & 0xF0) << 4);
                for k in 0..(b2 & 0x0F) + 3 {
                    let c = ring[(off + k) & 0xFFF];
                    out.push(c);
                    ring[r] = c;
                    r = (r + 1) & 0xFFF;
                }
            }
        }
    }
}

/// Compresseur LZSS minimal (octets bruts uniquement) : sert aux tests.
pub fn lzss_literal_only(data: &[u8]) -> Vec<u8> {
    let mut o = Vec::new();
    for chunk in data.chunks(8) {
        o.push(((1u16 << chunk.len()) - 1) as u8);
        o.extend_from_slice(chunk);
    }
    o
}

/// Index Troika : nombre d'entrées, puis pour chacune longueur du nom, nom (terminé par un zéro), empreinte, type (1 brut,
/// 2 zlib, 0x400 dossier), taille, taille compressée, offset absolu.
/// * Arcanum : le nom est le chemin relatif complet (séparateur `\`), les dossiers n'ont rien de plus.
/// * Temple of Elemental Evil : le nom est celui d'un seul niveau, et chaque entrée porte trois entiers de plus (parent,
///   premier enfant, dernier enfant) qui donnent l'arborescence.
/// On essaie les variantes et on garde celle qui consomme exactement l'index.
fn parse_troika(idx: &[u8]) -> Result<Vec<DatEntry>> {
    for (file_extra, dir_extra) in [(0usize, 0usize), (12, 12), (0, 12)] {
        if let Some(v) = parse_troika_with(idx, file_extra, dir_extra) {
            return Ok(v);
        }
    }
    bail!("index d'archive Troika invalide")
}

fn parse_troika_with(idx: &[u8], file_extra: usize, dir_extra: usize) -> Option<Vec<DatEntry>> {
    let rd = |p: usize| -> Option<u32> { idx.get(p..p + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap())) };
    let count = rd(0)? as usize;
    let mut p = 4usize;
    // (nom, type, taille, taille compressée, offset, parent)
    let mut raw: Vec<(String, u32, u32, u32, u32, i64)> = Vec::with_capacity(count.min(1_000_000));
    for _ in 0..count {
        let nl = rd(p)? as usize;
        p += 4;
        let name = idx.get(p..p.checked_add(nl)?)?;
        p += nl;
        let (_hash, ty, size, packed, offset) = (rd(p)?, rd(p + 4)?, rd(p + 8)?, rd(p + 12)?, rd(p + 16)?);
        p += 20;
        let is_dir = ty & 0x400 != 0;
        if !is_dir && ty != 1 && ty != 2 {
            return None;
        }
        let extra = if is_dir { dir_extra } else { file_extra };
        let parent = if extra >= 4 { rd(p)? as i32 as i64 } else { -1 };
        p += extra;
        raw.push((String::from_utf8_lossy(name).trim_end_matches('\0').to_string(), ty, size, packed, offset, parent));
    }
    if p != idx.len() {
        return None;
    }
    let hierarchical = file_extra > 0 || dir_extra > 0;
    let mut v = Vec::new();
    for (i, (name, ty, size, packed, offset, _)) in raw.iter().enumerate() {
        if ty & 0x400 != 0 {
            continue;
        }
        let mut full = name.clone();
        if hierarchical {
            // Remonte la chaîne des parents (garde-fou contre les cycles).
            let mut cur = raw[i].5;
            let mut depth = 0;
            while cur >= 0 && (cur as usize) < raw.len() && depth < 64 {
                full = format!("{}/{}", raw[cur as usize].0, full);
                cur = raw[cur as usize].5;
                depth += 1;
            }
        }
        let path = norm(&full);
        if !path.is_empty() {
            v.push(DatEntry { path, flags: *ty, offset: *offset as u64, size: *size, packed: *packed });
        }
    }
    Some(v)
}

/// Index d'une archive ZIP : les entrées de type fichier, par numéro d'ordre (stocké dans `offset`).
fn parse_zip(path: &Path) -> Result<Vec<DatEntry>> {
    let f = std::fs::File::open(path)?;
    let mut z = zip::ZipArchive::new(std::io::BufReader::new(f)).map_err(|e| anyhow!("{} : archive ZIP illisible ({e})", path.display()))?;
    let mut v = Vec::with_capacity(z.len());
    for i in 0..z.len() {
        let Ok(f) = z.by_index_raw(i) else { continue };
        if f.is_dir() {
            continue;
        }
        let Ok(name) = f.name() else { continue };
        let p = norm(&name);
        if !p.is_empty() {
            v.push(DatEntry { path: p, flags: 0, offset: i as u64, size: f.size().min(u32::MAX as u64) as u32, packed: f.compressed_size().min(u32::MAX as u64) as u32 });
        }
    }
    Ok(v)
}

/// Index d'une archive 7z : les fichiers, par numéro d'ordre (stocké dans `offset`).
fn parse_sevenz(path: &Path) -> Result<Vec<DatEntry>> {
    let r = sevenz_rust2::ArchiveReader::open(path, sevenz_rust2::Password::empty()).map_err(|e| anyhow!("{} : archive 7z illisible ({e})", path.display()))?;
    let mut v = Vec::new();
    for (i, f) in r.archive().files.iter().enumerate() {
        if f.is_directory || f.is_anti_item {
            continue;
        }
        let p = norm(&f.name);
        if !p.is_empty() {
            v.push(DatEntry { path: p, flags: 0, offset: i as u64, size: f.size.min(u32::MAX as u64) as u32, packed: f.compressed_size.min(u32::MAX as u64) as u32 });
        }
    }
    Ok(v)
}

fn parse_dat2(tree: &[u8], base: u64) -> Result<Vec<DatEntry>> {
    let mut p = 0usize;
    let rd32 = |p: &mut usize| -> Result<u32> {
        let b = tree.get(*p..*p + 4).ok_or_else(|| anyhow!("arbre tronqué"))?;
        *p += 4;
        Ok(u32::from_le_bytes(b.try_into().unwrap()))
    };
    let count = rd32(&mut p)? as usize;
    let mut v = Vec::with_capacity(count.min(1_000_000));
    for _ in 0..count {
        let nl = rd32(&mut p)? as usize;
        let name = tree.get(p..p + nl).ok_or_else(|| anyhow!("arbre tronqué"))?;
        p += nl;
        let comp = *tree.get(p).ok_or_else(|| anyhow!("arbre tronqué"))?;
        p += 1;
        let size = rd32(&mut p)?;
        let packed = rd32(&mut p)?;
        let offset = rd32(&mut p)?;
        v.push(DatEntry { path: norm(&String::from_utf8_lossy(name)), flags: comp as u32, offset: base + offset as u64, size, packed });
    }
    Ok(v)
}

fn parse_dat1(d: &[u8]) -> Result<Vec<DatEntry>> {
    let mut p = 0usize;
    let rd32 = |p: &mut usize| -> Result<u32> {
        let b = d.get(*p..*p + 4).ok_or_else(|| anyhow!("en-tête tronqué"))?;
        *p += 4;
        Ok(u32::from_be_bytes(b.try_into().unwrap()))
    };
    let ndirs = rd32(&mut p)? as usize;
    let (a, b, c) = (rd32(&mut p)?, rd32(&mut p)?, rd32(&mut p)?);
    if ndirs == 0 || ndirs > 100_000 || a as usize > 1_000_000 || b > 0x100 || c > 0x7FFF_FFFF {
        bail!("en-tête invalide");
    }
    let mut names = Vec::with_capacity(ndirs);
    for _ in 0..ndirs {
        let l = *d.get(p).ok_or_else(|| anyhow!("tronqué"))? as usize;
        p += 1;
        let n = d.get(p..p + l).ok_or_else(|| anyhow!("tronqué"))?;
        p += l;
        names.push(String::from_utf8_lossy(n).to_string());
    }
    let mut v = Vec::new();
    for dn in &names {
        let nfiles = rd32(&mut p)? as usize;
        p += 12; // allocation, taille des métadonnées, horodatage
        if nfiles > 5_000_000 {
            bail!("nombre de fichiers invalide");
        }
        let dir = if dn == "." { String::new() } else { norm(dn) };
        for _ in 0..nfiles {
            let l = *d.get(p).ok_or_else(|| anyhow!("tronqué"))? as usize;
            p += 1;
            let n = d.get(p..p + l).ok_or_else(|| anyhow!("tronqué"))?;
            p += l;
            let name = String::from_utf8_lossy(n).to_string();
            let flags = rd32(&mut p)?;
            let offset = rd32(&mut p)?;
            let size = rd32(&mut p)?;
            let packed = rd32(&mut p)?;
            let path = if dir.is_empty() { norm(&name) } else { format!("{dir}/{}", norm(&name)) };
            v.push(DatEntry { path, flags, offset: offset as u64, size, packed });
        }
    }
    Ok(v)
}

/// Écrit une archive DAT2 (tests).
pub fn write_dat2(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
    use std::io::Write;
    let mut out = Vec::new();
    let mut tree = Vec::new();
    tree.extend_from_slice(&(files.len() as u32).to_le_bytes());
    for (name, data, compress) in files {
        let offset = out.len() as u32;
        let packed: Vec<u8> = if *compress {
            let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            e.write_all(data).unwrap();
            e.finish().unwrap()
        } else {
            data.to_vec()
        };
        out.extend_from_slice(&packed);
        tree.extend_from_slice(&(name.len() as u32).to_le_bytes());
        tree.extend_from_slice(name.as_bytes());
        tree.push(*compress as u8);
        tree.extend_from_slice(&(data.len() as u32).to_le_bytes());
        tree.extend_from_slice(&(packed.len() as u32).to_le_bytes());
        tree.extend_from_slice(&offset.to_le_bytes());
    }
    let tree_size = tree.len() as u32;
    out.extend_from_slice(&tree);
    let total = out.len() as u32 + 8;
    out.extend_from_slice(&tree_size.to_le_bytes());
    out.extend_from_slice(&total.to_le_bytes());
    out
}

/// Écrit une archive DAT1 avec fichiers bruts (0x20) ou LZSS par blocs (0x40) (tests).
pub fn write_dat1(dirs: &[(&str, Vec<(&str, Vec<u8>, bool)>)]) -> Vec<u8> {
    let mut idx = Vec::new();
    let be = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&x.to_be_bytes());
    be(&mut idx, dirs.len() as u32);
    be(&mut idx, 0x0A);
    be(&mut idx, 0);
    be(&mut idx, 0);
    for (n, _) in dirs {
        idx.push(n.len() as u8);
        idx.extend_from_slice(n.as_bytes());
    }
    // Calcul de la taille de l'index pour connaître les décalages de données.
    let mut idx_len = idx.len();
    for (_, files) in dirs {
        idx_len += 16;
        for (n, _, _) in files {
            idx_len += 1 + n.len() + 16;
        }
    }
    let mut data = Vec::new();
    let mut entries = Vec::new();
    for (_, files) in dirs {
        let mut es = Vec::new();
        for (n, content, comp) in files {
            let off = (idx_len + data.len()) as u32;
            if *comp {
                let mut packed = Vec::new();
                for chunk in content.chunks(1000) {
                    let z = lzss_literal_only(chunk);
                    packed.extend_from_slice(&(z.len() as u16).to_be_bytes());
                    packed.extend_from_slice(&z);
                }
                es.push((n.to_string(), 0x40u32, off, content.len() as u32, packed.len() as u32));
                data.extend_from_slice(&packed);
            } else {
                es.push((n.to_string(), 0x20u32, off, content.len() as u32, 0));
                data.extend_from_slice(content);
            }
        }
        entries.push(es);
    }
    for es in &entries {
        be(&mut idx, es.len() as u32);
        be(&mut idx, 0x0A);
        be(&mut idx, 0x10);
        be(&mut idx, 0);
        for (n, fl, off, size, packed) in es {
            idx.push(n.len() as u8);
            idx.extend_from_slice(n.as_bytes());
            be(&mut idx, *fl);
            be(&mut idx, *off);
            be(&mut idx, *size);
            be(&mut idx, *packed);
        }
    }
    assert_eq!(idx.len(), idx_len);
    idx.extend_from_slice(&data);
    idx
}

/// Résultat d'une extraction.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Extracted {
    /// Fichiers écrits.
    pub written: usize,
    /// Fichiers déjà présents, laissés tels quels (sauf `overwrite`).
    pub skipped: usize,
}

/// Extrait un fichier, un dossier (récursivement) ou toute une archive vers `out_dir`, en conservant l'arborescence
/// relative au dossier extrait. Un fichier existant n'est remplacé que si `overwrite` est vrai.
pub fn extract(path: &Path, out_dir: &Path, overwrite: bool) -> Result<Extracted> {
    let items: Vec<(PathBuf, PathBuf)> = match stat(path) {
        Some(Stat::File) => vec![(path.to_path_buf(), PathBuf::from(path.file_name().ok_or_else(|| anyhow!("nom de fichier invalide"))?))],
        Some(Stat::Dir) => walk(path),
        None => bail!("{} : introuvable dans l'archive", path.display()),
    };
    let mut report = Extracted::default();
    for (full, rel) in items {
        // Les noms d'entrée sont déjà nettoyés (voir `norm`) ; on vérifie quand même que rien ne sort du dossier de sortie.
        if rel.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
            bail!("chemin d'entrée refusé : {}", rel.display());
        }
        let dest = out_dir.join(&rel);
        if dest.exists() && !overwrite {
            report.skipped += 1;
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| anyhow!("{} : {e}", parent.display()))?;
        }
        std::fs::write(&dest, read_any(&full)?).map_err(|e| anyhow!("{} : {e}", dest.display()))?;
        report.written += 1;
    }
    Ok(report)
}
