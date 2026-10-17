//! Arborescence des dossiers (chargement paresseux) et liste des images du dossier courant.

use crate::model::is_supported;
use std::cmp::Ordering;
use std::path::{Path, PathBuf};

pub struct FileEntry {
    pub path: PathBuf,
    pub name: String,
    /// Taille en octets et date de modification (le tri les utilise ; `None` pour un fichier d'archive).
    pub size: u64,
    pub modified: Option<std::time::SystemTime>,
}

/// Critère de tri des fichiers d'un dossier.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, serde::Serialize, serde::Deserialize)]
pub enum SortKey {
    #[default]
    Name,
    Size,
    Modified,
    Type,
}

/// Fichiers filtrés (sous-chaîne du nom, sans tenir compte de la casse) puis triés ; l'ordre « naturel » (`img2` avant `img10`) départage.
pub fn ordered_files<'a>(files: &'a [FileEntry], sort: SortKey, descending: bool, filter: &str) -> Vec<&'a FileEntry> {
    let f = filter.trim().to_lowercase();
    let ext = |e: &FileEntry| Path::new(&e.name).extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
    let mut v: Vec<&FileEntry> = files.iter().filter(|e| f.is_empty() || e.name.to_lowercase().contains(&f)).collect();
    let dir = |o: Ordering| if descending { o.reverse() } else { o };
    match sort {
        SortKey::Name => v.sort_by(|a, b| dir(natural_cmp(&a.name, &b.name))),
        SortKey::Size => v.sort_by(|a, b| dir(a.size.cmp(&b.size).then_with(|| natural_cmp(&a.name, &b.name)))),
        SortKey::Modified => v.sort_by(|a, b| dir(a.modified.cmp(&b.modified).then_with(|| natural_cmp(&a.name, &b.name)))),
        SortKey::Type => {
            // L'extension est calculée une seule fois par fichier, pas à chaque comparaison.
            let mut keyed: Vec<(String, &FileEntry)> = v.into_iter().map(|e| (ext(e), e)).collect();
            keyed.sort_by(|a, b| dir(a.0.cmp(&b.0).then_with(|| natural_cmp(&a.1.name, &b.1.name))));
            v = keyed.into_iter().map(|(_, e)| e).collect();
        }
    }
    v
}

pub struct DirNode {
    pub path: PathBuf,
    pub name: String,
    /// Archive .DAT (fichier) : seul cas affiché en bleu ; un dossier nommé « x.dat » reste un dossier.
    pub archive: bool,
    pub expanded: bool,
    pub loaded: bool,
    pub children: Vec<DirNode>,
    pub files: Vec<FileEntry>,
    /// Clé de traduction du libellé (dossiers spéciaux : Bureau, Documents, Téléchargements).
    pub label_key: Option<&'static str>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Dir { expanded: bool },
    File,
}

#[derive(Clone)]
pub struct Row {
    pub archive: bool,
    pub depth: usize,
    pub kind: RowKind,
    pub path: PathBuf,
    pub name: String,
}

pub struct Tree {
    pub roots: Vec<DirNode>,
    pub rows: Vec<Row>,
    pub selected_dir: Option<PathBuf>,
    pub sort: SortKey,
    pub descending: bool,
    /// Filtre sur les noms de fichiers du dossier courant (vide : tous).
    pub filter: String,
    /// Incrémenté à chaque reconstruction des lignes (permet de ne recalculer la largeur du contenu que si elles changent).
    pub rev: u64,
}

/// Ordre « naturel » sans tenir compte de la casse : `img2` avant `img10`. Aucune allocation (le tri d'un dossier de milliers de
/// fichiers l'appelle des centaines de milliers de fois).
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let lower = |c: char| c.to_lowercase().next().unwrap_or(c);
    let (mut i, mut j) = (0usize, 0usize);
    loop {
        match (a[i..].chars().next(), b[j..].chars().next()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                // Deux suites de chiffres : comparées comme des nombres (zéros de tête ignorés).
                let ea = i + a[i..].bytes().take_while(u8::is_ascii_digit).count();
                let eb = j + b[j..].bytes().take_while(u8::is_ascii_digit).count();
                let (ta, tb) = (a[i..ea].trim_start_matches('0'), b[j..eb].trim_start_matches('0'));
                let o = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if o != Ordering::Equal {
                    return o;
                }
                (i, j) = (ea, eb);
            }
            (Some(x), Some(y)) => {
                let (lx, ly) = (lower(x), lower(y));
                if lx != ly {
                    return lx.cmp(&ly);
                }
                i += x.len_utf8();
                j += y.len_utf8();
            }
        }
    }
}

#[cfg(windows)]
fn is_hidden(e: &std::fs::DirEntry) -> bool {
    use std::os::windows::fs::MetadataExt;
    // Les attributs viennent de la lecture du dossier : pas d'appel système supplémentaire par entrée.
    e.metadata().map(|m| m.file_attributes() & 0x2 != 0 || m.file_attributes() & 0x4 != 0).unwrap_or(false)
}
#[cfg(not(windows))]
fn is_hidden(e: &std::fs::DirEntry) -> bool {
    e.file_name().to_string_lossy().starts_with('.')
}

#[cfg(windows)]
fn root_paths() -> Vec<PathBuf> {
    use windows_sys::Win32::Storage::FileSystem::GetLogicalDrives;
    let mask = unsafe { GetLogicalDrives() };
    (0..26).filter(|i| mask & (1 << i) != 0).map(|i| PathBuf::from(format!("{}:\\", (b'A' + i as u8) as char))).collect()
}
#[cfg(not(windows))]
fn root_paths() -> Vec<PathBuf> {
    let mut v = vec![PathBuf::from("/")];
    if let Some(h) = std::env::var_os("HOME") {
        v.insert(0, PathBuf::from(h));
    }
    v
}

/// Bureau, Documents et Téléchargements de l'utilisateur : (clé du libellé, dossier).
#[cfg(windows)]
pub fn special_folders() -> Vec<(&'static str, PathBuf)> {
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, SHGetKnownFolderPath};
    let get = |id: &windows_sys::core::GUID| -> Option<PathBuf> {
        let mut p: *mut u16 = std::ptr::null_mut();
        // SAFETY : `p` reçoit une chaîne allouée par le système, terminée par zéro et libérée juste après la copie.
        unsafe {
            let hr = SHGetKnownFolderPath(id, 0, std::ptr::null_mut(), &mut p);
            if hr != 0 || p.is_null() {
                return None;
            }
            let mut len = 0;
            while *p.add(len) != 0 {
                len += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
            CoTaskMemFree(p as *const _);
            Some(PathBuf::from(s))
        }
    };
    let mut v = Vec::new();
    for (key, id) in [("t.desktop", &FOLDERID_Desktop), ("t.documents", &FOLDERID_Documents), ("t.downloads", &FOLDERID_Downloads)] {
        if let Some(p) = get(id) {
            v.push((key, p));
        }
    }
    v
}

#[cfg(not(windows))]
pub fn special_folders() -> Vec<(&'static str, PathBuf)> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return Vec::new() };
    // Linux : ~/.config/user-dirs.dirs (XDG) ; repli sur les noms anglais, qui sont aussi ceux de macOS.
    let cfg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config")).join("user-dirs.dirs");
    let text = std::fs::read_to_string(cfg).unwrap_or_default();
    let xdg = |name: &str| -> Option<PathBuf> {
        let line = text.lines().find(|l| l.trim_start().starts_with(name))?;
        let v = line.split_once('=')?.1.trim().trim_matches('"');
        Some(match v.strip_prefix("$HOME/") {
            Some(rest) => home.join(rest),
            None => PathBuf::from(v),
        })
    };
    vec![
        ("t.desktop", xdg("XDG_DESKTOP_DIR").unwrap_or_else(|| home.join("Desktop"))),
        ("t.documents", xdg("XDG_DOCUMENTS_DIR").unwrap_or_else(|| home.join("Documents"))),
        ("t.downloads", xdg("XDG_DOWNLOAD_DIR").unwrap_or_else(|| home.join("Downloads"))),
    ]
}

impl DirNode {
    fn new(path: PathBuf, name: String) -> DirNode {
        DirNode { path, name, archive: false, expanded: false, loaded: false, children: Vec::new(), files: Vec::new(), label_key: None }
    }

    pub fn load(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        self.children.clear();
        self.files.clear();
        // Archive .DAT (ou dossier à l'intérieur) : contenu virtuel.
        if let Some((arch, inner)) = crate::dat::split(&self.path) {
            if let Ok(dat) = crate::dat::Dat::open(&arch) {
                let (dirs, files) = dat.children(&inner);
                for d in dirs {
                    self.children.push(DirNode::new(self.path.join(&d), d));
                }
                for e in files {
                    let p = self.path.join(e.name());
                    if is_supported(&p) {
                        self.files.push(FileEntry { path: p, name: e.name().to_string(), size: e.size as u64, modified: None });
                    }
                }
            }
            self.children.sort_by(|a, b| natural_cmp(&a.name, &b.name));
            self.files.sort_by(|a, b| natural_cmp(&a.name, &b.name));
            return;
        }
        let Ok(rd) = std::fs::read_dir(&self.path) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() || (ft.is_symlink() && p.is_dir()) {
                if name.starts_with('$') || name == "System Volume Information" || is_hidden(&e) {
                    continue;
                }
                self.children.push(DirNode::new(p, name));
            } else if crate::dat::is_dat_ext(&p) {
                let mut n = DirNode::new(p, name);
                n.archive = true;
                self.children.push(n);
            } else if is_supported(&p) {
                let md = e.metadata().ok();
                self.files.push(FileEntry { path: p, name, size: md.as_ref().map_or(0, |m| m.len()), modified: md.and_then(|m| m.modified().ok()) });
            }
        }
        self.children.sort_by(|a, b| natural_cmp(&a.name, &b.name));
        self.files.sort_by(|a, b| natural_cmp(&a.name, &b.name));
    }

    fn reload(&mut self) {
        let expanded: Vec<(PathBuf, bool)> = self.children.iter().map(|c| (c.path.clone(), c.expanded)).collect();
        self.loaded = false;
        self.load();
        for c in &mut self.children {
            if let Some((_, e)) = expanded.iter().find(|(p, _)| *p == c.path) {
                c.expanded = *e;
            }
        }
    }
}

impl Default for Tree {
    fn default() -> Self {
        Self::new()
    }
}

impl Tree {
    pub fn new() -> Tree {
        let roots: Vec<DirNode> = root_paths()
            .into_iter()
            .map(|p| {
                let name = p.display().to_string().trim_end_matches(['\\', '/']).to_string();
                let name = if name.is_empty() { "/".to_string() } else { name };
                DirNode::new(p, name)
            })
            .collect();

        let mut t = Tree { roots, rows: Vec::new(), selected_dir: None, sort: SortKey::Name, descending: false, filter: String::new(), rev: 0 };
        t.rebuild();
        t
    }

    /// Retraduit les libellés des dossiers spéciaux (après un changement de langue).
    pub fn relabel(&mut self) {
        for r in &mut self.roots {
            if let Some(k) = r.label_key {
                r.name = crate::i18n::tr(k);
            }
        }
        self.rebuild();
    }

    fn find_mut<'a>(nodes: &'a mut [DirNode], path: &Path) -> Option<&'a mut DirNode> {
        for n in nodes.iter_mut() {
            if n.path == path {
                return Some(n);
            }
            if path.starts_with(&n.path) {
                n.load();
                if let Some(f) = Self::find_mut(&mut n.children, path) {
                    return Some(f);
                }
            }
        }
        None
    }

    fn find<'a>(nodes: &'a [DirNode], path: &Path) -> Option<&'a DirNode> {
        for n in nodes {
            if n.path == path {
                return Some(n);
            }
            if path.starts_with(&n.path)
                && let Some(f) = Self::find(&n.children, path)
            {
                return Some(f);
            }
        }
        None
    }

    /// Développe tous les ancêtres de `dir`, le sélectionne et charge son contenu.
    pub fn reveal_dir(&mut self, dir: &Path) {
        let mut chain: Vec<PathBuf> = dir.ancestors().map(|p| p.to_path_buf()).collect();
        chain.reverse();
        for p in &chain {
            if p.as_os_str().is_empty() {
                continue;
            }
            if let Some(n) = Self::find_mut(&mut self.roots, p) {
                n.load();
                n.expanded = true;
            }
        }
        self.selected_dir = Some(dir.to_path_buf());
        self.rebuild();
    }

    pub fn toggle(&mut self, path: &Path) {
        if let Some(n) = Self::find_mut(&mut self.roots, path) {
            n.load();
            n.expanded = !n.expanded;
        }
        self.rebuild();
    }

    pub fn select_dir(&mut self, path: &Path) {
        if let Some(n) = Self::find_mut(&mut self.roots, path) {
            n.load();
            n.expanded = true;
        }
        self.selected_dir = Some(path.to_path_buf());
        self.rebuild();
    }

    pub fn reload_dir(&mut self, path: &Path) {
        if let Some(n) = Self::find_mut(&mut self.roots, path) {
            n.reload();
        }
        self.rebuild();
    }

    pub fn files_of(&self, dir: &Path) -> Vec<PathBuf> {
        Self::find(&self.roots, dir).map(|n| ordered_files(&n.files, self.sort, self.descending, &self.filter).iter().map(|f| f.path.clone()).collect()).unwrap_or_default()
    }

    pub fn rebuild(&mut self) {
        let mut rows = Vec::new();
        let sel = self.selected_dir.clone();
        let (sort, desc, filter) = (self.sort, self.descending, self.filter.clone());
        fn walk(n: &DirNode, depth: usize, sel: &Option<PathBuf>, rows: &mut Vec<Row>, view: (SortKey, bool, &str)) {
            rows.push(Row { archive: n.archive, depth, kind: RowKind::Dir { expanded: n.expanded }, path: n.path.clone(), name: n.name.clone() });
            if n.expanded {
                for c in &n.children {
                    walk(c, depth + 1, sel, rows, view);
                }
                if sel.as_deref() == Some(n.path.as_path()) {
                    for f in ordered_files(&n.files, view.0, view.1, view.2) {
                        rows.push(Row { archive: false, depth: depth + 1, kind: RowKind::File, path: f.path.clone(), name: f.name.clone() });
                    }
                }
            }
        }
        for r in &self.roots {
            walk(r, 0, &sel, &mut rows, (sort, desc, &filter));
        }
        self.rows = rows;
        self.rev += 1;
    }
}

impl DirNode {
    /// Compte les sous-dossiers (récursivement, en chargeant le contenu) en s'arrêtant au-delà de `limit`.
    fn count_dirs(&mut self, limit: usize, acc: &mut usize) {
        self.load();
        for c in &mut self.children {
            *acc += 1;
            if *acc > limit {
                return;
            }
            c.count_dirs(limit, acc);
            if *acc > limit {
                return;
            }
        }
    }

    fn expand_all(&mut self, budget: &mut usize, deadline: std::time::Instant) {
        self.load();
        self.expanded = true;
        for c in &mut self.children {
            // Plafond de dossiers et de temps : la lecture a lieu dans le thread de l'interface (disque lent, dossier réseau).
            if *budget == 0 || std::time::Instant::now() > deadline {
                return;
            }
            *budget -= 1;
            c.expand_all(budget, deadline);
        }
    }

    fn collapse_subtree(&mut self) {
        self.expanded = false;
        for c in &mut self.children {
            c.collapse_subtree();
        }
    }

    /// Replie tout ce qui n'est ni un ancêtre, ni l'élément `target`, ni l'un de ses descendants.
    fn collapse_others(&mut self, target: &Path) {
        if self.path == target || self.path.starts_with(target) {
            return; // l'élément et ses descendants gardent leur état
        }
        if target.starts_with(&self.path) {
            for c in &mut self.children {
                c.collapse_others(target); // ancêtre : reste déplié
            }
        } else {
            self.collapse_subtree();
        }
    }
}

impl Tree {
    /// Nombre de sous-dossiers de `path` (plafonné à `limit + 1`).
    pub fn count_dirs(&mut self, path: &Path, limit: usize) -> usize {
        let mut n = 0;
        if let Some(node) = Self::find_mut(&mut self.roots, path) {
            node.count_dirs(limit, &mut n);
        }
        n
    }

    /// Déplie tous les sous-dossiers de `path` (plafond de sécurité : 20 000 dossiers).
    pub fn expand_all(&mut self, path: &Path) {
        let mut budget = 20_000usize;
        if let Some(node) = Self::find_mut(&mut self.roots, path) {
            node.expand_all(&mut budget, std::time::Instant::now() + std::time::Duration::from_secs(3));
        }
        self.rebuild();
    }

    pub fn collapse_others(&mut self, path: &Path) {
        for r in &mut self.roots {
            r.collapse_others(path);
        }
        self.rebuild();
    }
}
