//! Application egui : même interface que FalloutImageBrowser (arborescence à gauche, média au centre, propriétés à droite),
//! avec un lecteur audio / vidéo à la place de la visionneuse d'images.

use crate::audio::{Output, Pcm};
use crate::convert::{self, ConvSettings, ConvertUi};
use crate::i18n::{self, Lang, tr};
use crate::icons::{Icon, icon_button};
use crate::media::{self, ChannelMode, Loaded};
use crate::video::VideoDecoder;
use crate::tree::{RowKind, SortKey, Tree};
use eframe::egui::{self, Align2, Color32, FontId, Id, Rect, Sense, Stroke, TextureHandle, TextureOptions, Ui, Vec2, pos2, vec2};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

pub const SETTINGS_KEY: &str = "fav_settings_v1";
const MIN_ZOOM: f32 = 0.05;
/// Favoris ajustables (les trois autres boutons sont réservés : Bureau, Documents, Téléchargements).
const MAX_FAVORITES: usize = 5;
const MAX_ZOOM: f32 = 32.0;

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    pub lang: Option<Lang>,
    pub show_tree: bool,
    pub show_props: bool,
    pub sort: SortKey,
    pub sort_desc: bool,
    pub last_dir: Option<PathBuf>,
    pub volume: f32,
    pub looping: bool,
    /// Vitesse de lecture (0,2x à 5x).
    pub speed: f32,
    pub auto_next: bool,
    /// Boutons latéraux de la souris : false = fichier précédent / suivant, true = lecture (pause) / fichier suivant.
    pub xbtn_play: bool,
    pub chan_mode: ChannelMode,
    /// Dossiers favoris (5 au plus), affichés sous le filtre de l'arborescence.
    pub favorites: Vec<PathBuf>,
    /// Couleur d'accent du signal audio (forme d'onde et trait de lecture).
    pub wave_color: [u8; 3],
    /// Réglages de la fenêtre « Convertir le fichier ».
    pub conv: ConvSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            lang: None,
            show_tree: true,
            show_props: true,
            sort: SortKey::Name,
            sort_desc: false,
            last_dir: None,
            volume: 0.8,
            looping: false,
            speed: 1.0,
            auto_next: true,
            xbtn_play: false,
            chan_mode: ChannelMode::Auto,
            favorites: Vec::new(),
            wave_color: [90, 190, 120],
            conv: ConvSettings::default(),
        }
    }
}

/// Actions des boutons du bas du panneau « Propriétés ».
#[derive(Clone, Copy, PartialEq, Eq)]
enum PropAction {
    CopyPath,
    CopyTo,
    MoveTo,
    Rename,
    Convert,
    Text,
    Json,
    Xml,
    Sha,
    ShaStop,
}

/// État de la fenêtre « Renommer » : fichier, nom en cours de saisie, première image (pour sélectionner le nom).
struct RenameDlg {
    path: PathBuf,
    name: String,
    fresh: bool,
}

/// Conversion en cours dans un thread.
struct ConvJob {
    done: Arc<AtomicUsize>,
    total: Arc<AtomicUsize>,
    cancel: Arc<AtomicBool>,
    rx: Receiver<Result<Vec<PathBuf>, String>>,
    out_dir: PathBuf,
}

type LoadResult = (u64, PathBuf, bool, Result<Loaded, String>);

pub struct App {
    pub settings: Settings,
    pub tree: Tree,
    pub files: Vec<PathBuf>,
    pub index: Option<usize>,
    pub wanted: Option<PathBuf>,
    out: Output,
    loaded: Option<Loaded>,
    /// Date de modification du fichier affiché (absente dans une archive).
    modified: Option<std::time::SystemTime>,
    created: Option<std::time::SystemTime>,
    conv: ConvertUi,
    conv_job: Option<ConvJob>,
    rename_dlg: Option<RenameDlg>,
    peaks: Vec<(f32, f32)>,
    error: Option<String>,
    loading: bool,
    gen_id: u64,
    tx: Sender<LoadResult>,
    rx: Receiver<LoadResult>,
    // Vidéo
    dec: Option<VideoDecoder>,
    tex: Option<TextureHandle>,
    shown_frame: usize,
    vt: f64,
    last_tick: Instant,
    playing: bool,
    zoom: f32,
    pan: Vec2,
    fit: bool,
    /// Échelle réellement appliquée à la vidéo à l'image précédente.
    cur_scale: f32,
    // Interface
    toast: Option<(String, Instant)>,
    /// Dernier SHA-256 copié (nom du fichier, empreinte) : sert à comparer deux fichiers.
    ref_hash: Option<(String, String)>,
    /// Zone de l'arborescence (la saisie au clavier n'y agit que si la souris la survole) et recherche en cours.
    tree_rect: Option<Rect>,
    /// Bureau, Documents, Téléchargements (boutons réservés de la barre de favoris).
    special: Vec<(&'static str, PathBuf)>,
    ta_buf: String,
    ta_time: Instant,
    show_about: bool,
    expand_confirm: Option<PathBuf>,
    folder_dlg: Option<(String, bool)>,
    file_filter: String,
    tree_w: (u64, f32),
    tree_scroll_to: Option<usize>,
    fullscreen: bool,
    toolbar_w: f32,
    last_title: String,
    extract_job: Option<(Receiver<Result<crate::dat::Extracted, String>>, PathBuf)>,
    pub win: crate::winprefs::WinPrefs,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, initial: Option<PathBuf>) -> App {
        let settings: Settings = cc.storage.and_then(|s| eframe::get_value(s, SETTINGS_KEY)).unwrap_or_default();
        let forced = std::env::var("FAV_LANG").ok().and_then(|c| Lang::ALL.iter().copied().find(|l| l.code() == c.to_lowercase()));
        i18n::set_lang(forced.or(settings.lang).unwrap_or_else(Lang::detect));
        let (tx, rx) = channel();
        let out = Output::new();
        out.set_volume(settings.volume);
        out.set_looping(settings.looping && !settings.auto_next);
        out.set_speed(settings.speed.clamp(0.2, 5.0));
        let mut app = App {
            tree: Tree::new(),
            files: Vec::new(),
            index: None,
            wanted: None,
            out,
            loaded: None,
            modified: None,
            created: None,
            conv: ConvertUi::new(settings.conv.clone()),
            conv_job: None,
            rename_dlg: None,
            peaks: Vec::new(),
            error: None,
            loading: false,
            gen_id: 0,
            tx,
            rx,
            dec: None,
            tex: None,
            shown_frame: usize::MAX,
            vt: 0.0,
            last_tick: Instant::now(),
            playing: false,
            zoom: 1.0,
            pan: Vec2::ZERO,
            fit: true,
            cur_scale: 1.0,
            toast: None,
            ref_hash: None,
            tree_rect: None,
            special: crate::tree::special_folders().into_iter().filter(|(_, p)| p.is_dir()).collect(),
            ta_buf: String::new(),
            ta_time: Instant::now(),
            show_about: false,
            expand_confirm: None,
            folder_dlg: None,
            file_filter: String::new(),
            tree_w: (u64::MAX, 0.0),
            tree_scroll_to: None,
            fullscreen: false,
            toolbar_w: 0.0,
            last_title: String::new(),
            extract_job: None,
            win: crate::winprefs::WinPrefs::load(),
            settings,
        };
        app.apply_fonts(&cc.egui_ctx);
        app.tree.sort = app.settings.sort;
        app.tree.descending = app.settings.sort_desc;
        match initial {
            Some(p) => app.open_path(&cc.egui_ctx, &p, true),
            None => {
                if let Some(d) = app.settings.last_dir.clone().filter(|d| d.exists()) {
                    app.open_dir(&cc.egui_ctx, &d, false);
                    app.reveal_in_tree();
                }
            }
        }
        app
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    /// Polices : celle d'egui + une police CJK du système en secours (pour « 中文 », les noms de fichiers...).
    fn apply_fonts(&mut self, ctx: &egui::Context) {
        let mut fonts = egui::FontDefinitions::default();
        let candidates = [
            r"C:\Windows\Fonts\msyh.ttc",
            r"C:\Windows\Fonts\msyh.ttf",
            r"C:\Windows\Fonts\simsun.ttc",
            r"C:\Windows\Fonts\simhei.ttf",
            "/System/Library/Fonts/PingFang.ttc",
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        ];
        for c in candidates {
            if let Ok(bytes) = std::fs::read(c) {
                fonts.font_data.insert("cjk".to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
                for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                    fonts.families.entry(fam).or_default().push("cjk".to_owned());
                }
                break;
            }
        }
        ctx.set_fonts(fonts);
    }

    fn set_language(&mut self, l: Lang) {
        self.settings.lang = Some(l);
        i18n::set_lang(l);
        self.tree.relabel();
    }

    // ------------------------------------------------------------------ navigation

    /// Ouverture depuis l'extérieur : un fichier ouvre son dossier, qui devient le dossier courant ; une archive .DAT
    /// s'ouvre comme un dossier. `play` : lancer la lecture du fichier ouvert.
    pub fn open_path(&mut self, ctx: &egui::Context, path: &Path, play: bool) {
        let kind = match crate::dat::stat(path) {
            Some(crate::dat::Stat::File) => Some(false),
            Some(crate::dat::Stat::Dir) => Some(true),
            None if path.is_dir() => Some(true),
            None if path.is_file() => Some(false),
            None => None,
        };
        match kind {
            Some(true) => self.open_dir(ctx, path, false),
            Some(false) => {
                if let Some(parent) = path.parent() {
                    self.open_dir(ctx, parent, false);
                }
                if crate::model::is_supported(path) {
                    self.show_file(ctx, path, play);
                }
            }
            None => return,
        }
        self.reveal_in_tree();
    }

    /// Affiche un dossier. `open_first` : charge aussi son premier fichier (sans le lire).
    pub fn open_dir(&mut self, ctx: &egui::Context, dir: &Path, open_first: bool) {
        self.tree.reveal_dir(dir);
        self.settings.last_dir = Some(dir.to_path_buf());
        self.files = self.tree.files_of(dir);
        self.index = None;
        if open_first {
            if let Some(first) = self.files.first().cloned() {
                self.show_file(ctx, &first, false);
            }
        } else if let Some(w) = &self.wanted {
            self.index = self.files.iter().position(|f| f == w);
        }
    }

    pub fn apply_view_options(&mut self) {
        self.tree.sort = self.settings.sort;
        self.tree.descending = self.settings.sort_desc;
        self.tree.filter = self.file_filter.clone();
        self.tree.rebuild();
        if let Some(dir) = self.tree.selected_dir.clone() {
            self.files = self.tree.files_of(&dir);
            self.index = self.wanted.as_ref().and_then(|w| self.files.iter().position(|f| f == w));
        }
    }

    fn reload_dir(&mut self) {
        if let Some(dir) = self.tree.selected_dir.clone() {
            self.tree.reload_dir(&dir);
            self.files = self.tree.files_of(&dir);
            self.index = self.wanted.as_ref().and_then(|w| self.files.iter().position(|f| f == w));
        }
    }

    fn reveal_in_tree(&mut self) {
        let target = self.wanted.clone().or_else(|| self.tree.selected_dir.clone());
        if let Some(t) = target {
            self.tree_scroll_to = self.tree.rows.iter().position(|r| r.path == t);
        }
    }

    /// Charge un fichier dans un thread ; `play` : le lire dès qu'il est prêt.
    pub fn show_file(&mut self, ctx: &egui::Context, path: &Path, play: bool) {
        self.gen_id += 1;
        self.loading = true;
        self.wanted = Some(path.to_path_buf());
        self.index = self.files.iter().position(|f| f == path);
        self.error = None;
        self.out.stop();
        self.out.set(None);
        self.playing = false;
        let meta = if crate::dat::is_virtual(path) { None } else { std::fs::metadata(path).ok() };
        self.modified = meta.as_ref().and_then(|m| m.modified().ok());
        self.created = meta.as_ref().and_then(|m| m.created().ok());
        let (tx, mode, id, ctx, path) = (self.tx.clone(), self.settings.chan_mode, self.gen_id, ctx.clone(), path.to_path_buf());
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(|| media::load(&path, mode).map_err(|e| e.to_string())).unwrap_or_else(|_| Err("erreur interne lors du décodage".to_string()));
            let _ = tx.send((id, path, play, r));
            ctx.request_repaint();
        });
    }

    fn step(&mut self, ctx: &egui::Context, delta: i32, play: bool) {
        if self.files.is_empty() {
            return;
        }
        let n = self.files.len() as i32;
        let cur = self.index.map(|i| i as i32).unwrap_or(if delta > 0 { -1 } else { 0 });
        let p = self.files[(cur + delta).rem_euclid(n) as usize].clone();
        self.show_file(ctx, &p, play);
    }

    fn goto(&mut self, ctx: &egui::Context, idx: usize) {
        if let Some(p) = self.files.get(idx).cloned() {
            self.show_file(ctx, &p, self.playing);
        }
    }

    fn poll_load(&mut self, ctx: &egui::Context) {
        while let Ok((id, path, play, res)) = self.rx.try_recv() {
            if id != self.gen_id {
                continue;
            }
            self.loading = false;
            match res {
                Ok(l) => {
                    self.peaks = l.pcm.as_ref().map(|p| peaks(p, 1600)).unwrap_or_default();
                    self.out.set(l.pcm.clone());
                    self.dec = l.video.as_ref().map(|m| VideoDecoder::new(m));
                    self.tex = None;
                    self.shown_frame = usize::MAX;
                    self.vt = 0.0;
                    self.fit = true;
                    self.pan = Vec2::ZERO;
                    self.loaded = Some(l);
                    self.update_video(ctx, true);
                    if play {
                        self.play();
                    }
                }
                Err(e) => {
                    self.loaded = None;
                    self.peaks.clear();
                    self.dec = None;
                    self.tex = None;
                    self.error = Some(format!("{} : {e}", path.display()));
                }
            }
        }
    }

    // ------------------------------------------------------------------ lecture

    fn duration(&self) -> f64 {
        match &self.loaded {
            Some(l) => match (&l.video, &l.pcm) {
                (Some(v), _) => v.duration(),
                (None, Some(p)) => p.duration(),
                _ => 0.0,
            },
            None => 0.0,
        }
    }

    fn has_audio(&self) -> bool {
        self.loaded.as_ref().is_some_and(|l| l.pcm.is_some())
    }

    fn is_video(&self) -> bool {
        self.loaded.as_ref().is_some_and(|l| l.video.is_some())
    }

    fn play(&mut self) {
        if self.loaded.is_none() {
            return;
        }
        if self.vt >= self.duration() {
            self.seek(0.0);
        }
        self.playing = true;
        self.last_tick = Instant::now();
        if self.has_audio() {
            self.out.seek(self.vt);
            self.out.play();
        }
    }

    fn pause(&mut self) {
        self.playing = false;
        self.out.pause();
    }

    /// Le moteur audio ne reboucle de lui-même que sur un fichier seul : avec « Enchaîner », la boucle se fait sur le répertoire.
    fn sync_loop(&self) {
        self.out.set_looping(self.settings.looping && !self.settings.auto_next);
    }

    fn toggle_play(&mut self) {
        if self.playing {
            self.pause();
        } else {
            self.play();
        }
    }

    fn stop(&mut self) {
        self.pause();
        self.seek(0.0);
    }

    fn seek(&mut self, t: f64) {
        self.vt = t.clamp(0.0, self.duration());
        self.out.seek(self.vt);
        self.last_tick = Instant::now();
    }

    fn tick(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f64();
        self.last_tick = now;
        if self.loaded.is_none() {
            return;
        }
        if self.playing {
            if self.has_audio() && self.out.is_playing() {
                self.vt = self.out.position();
            } else {
                self.vt += dt * self.settings.speed as f64;
            }
            let dur = self.duration();
            let audio_done = self.out.take_finished();
            let ended = if self.is_video() { self.vt >= dur } else { audio_done || (!self.out.is_playing() && self.vt >= dur) };
            if ended {
                let looping = self.settings.looping;
                let has_next = self.index.is_some_and(|i| i + 1 < self.files.len());
                if self.settings.auto_next && has_next {
                    self.vt = dur;
                    self.step(ctx, 1, true);
                } else if looping && self.settings.auto_next && self.files.len() > 1 {
                    // Enchaînement + boucle : on reboucle sur le répertoire (retour au premier fichier).
                    self.vt = dur;
                    let first = self.files[0].clone();
                    self.show_file(ctx, &first, true);
                } else if looping {
                    // Boucle sur le fichier seul.
                    self.seek(0.0);
                    if self.has_audio() {
                        self.out.play();
                    }
                } else {
                    self.playing = false;
                    self.out.pause();
                    self.vt = dur;
                }
            }
            ctx.request_repaint();
        }
        self.update_video(ctx, false);
    }

    /// Décode (si besoin) l'image qui correspond à l'instant courant et met à jour la texture.
    fn update_video(&mut self, ctx: &egui::Context, force: bool) {
        let (Some(l), Some(dec)) = (&self.loaded, &mut self.dec) else { return };
        let Some(m) = &l.video else { return };
        let n = m.frame_count();
        if n == 0 {
            return;
        }
        let target = ((self.vt * m.fps()) as usize).min(n - 1);
        if target == self.shown_frame && !force {
            return;
        }
        // Retour en arrière : on repart du début (le codec dépend des images précédentes).
        let key = m.key_before(target);
        if dec.position() > target + 1 || key > dec.position() {
            dec.seek_to(key);
        }
        let mut decoded = false;
        while dec.position() <= target {
            if !dec.step(m) {
                break;
            }
            decoded = true;
        }
        if decoded || self.tex.is_none() {
            let img = egui::ColorImage::from_rgba_unmultiplied([m.width(), m.height()], &dec.rgba());
            match &mut self.tex {
                Some(t) => t.set(img, TextureOptions::NEAREST),
                None => self.tex = Some(ctx.load_texture("mve", img, TextureOptions::NEAREST)),
            }
        }
        self.shown_frame = target;
    }

    fn zoom_by(&mut self, f: f32) {
        if self.fit {
            self.zoom = self.cur_scale;
        }
        self.fit = false;
        let old = self.zoom;
        self.zoom = (self.zoom * f).clamp(MIN_ZOOM, MAX_ZOOM);
        self.pan *= self.zoom / old;
    }

    fn zoom_actual(&mut self) {
        self.fit = false;
        let old = if self.fit { self.cur_scale } else { self.zoom };
        self.pan *= 1.0 / old.max(1e-6);
        self.zoom = 1.0;
    }

    // ------------------------------------------------------------------ fichier

    fn open_default(&mut self) {
        if let Some(p) = self.wanted.clone()
            && let Err(e) = crate::opener::open_default(&p)
        {
            self.toast(e);
        }
    }

    /// Ouvre la fenêtre « Convertir le fichier… ».
    fn open_convert(&mut self) {
        if let Some(src) = self.conv_source() {
            self.conv.open_for(&src);
        }
    }

    fn conv_source(&self) -> Option<convert::Source> {
        let (path, l) = (self.wanted.clone()?, self.loaded.as_ref()?);
        Some(convert::Source { path, pcm: l.pcm.clone(), video: l.video.clone(), frame: self.shown_frame.min(l.video.as_ref().map_or(0, |v| v.frame_count().saturating_sub(1))) })
    }

    fn start_conversion(&mut self, src: &convert::Source) {
        let Some(job) = self.conv.job(src) else { return };
        let (done, total, cancel) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(1)), Arc::new(AtomicBool::new(false)));
        let (tx, rx) = channel();
        let out_dir = job.out_dir.clone();
        let (d, t, c) = (done.clone(), total.clone(), cancel.clone());
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| convert::run(job, &d, &t, &c).map_err(|e| e.to_string()))).unwrap_or_else(|_| Err("erreur interne".to_string()));
            let _ = tx.send(r);
        });
        self.conv.error = None;
        self.conv.running = true;
        self.conv_job = Some(ConvJob { done, total, cancel, rx, out_dir });
    }

    fn poll_conversion(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.conv_job else { return };
        match job.rx.try_recv() {
            Ok(result) => {
                let out_dir = job.out_dir.clone();
                self.conv_job = None;
                self.conv.running = false;
                match result {
                    Ok(files) => {
                        self.conv.open = false;
                        self.conv.s.last_out_dir = Some(out_dir.clone());
                        let first = files.first().and_then(|f| f.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                        let label = if files.len() > 1 { format!("{first} (+{})", files.len() - 1) } else { first };
                        self.toast(i18n::trf("c.done", &[("file", label)]));
                        if self.tree.selected_dir.as_deref() == Some(out_dir.as_path()) {
                            self.reload_dir();
                        }
                    }
                    Err(e) => self.conv.error = Some(e),
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.conv_job = None;
                self.conv.running = false;
            }
        }
    }

    fn convert_window(&mut self, ctx: &egui::Context) {
        self.poll_conversion(ctx);
        if !self.conv.open {
            return;
        }
        let Some(src) = self.conv_source() else {
            self.conv.open = false;
            return;
        };
        let progress = self.conv_job.as_ref().map(|j| (j.done.load(Ordering::Relaxed), j.total.load(Ordering::Relaxed)));
        match self.conv.ui(ctx, &src, progress) {
            convert::Action::Close => match &self.conv_job {
                // Conversion en cours : on demande l'arrêt, la fenêtre se ferme à la fin du thread.
                Some(j) => j.cancel.store(true, Ordering::Relaxed),
                None => self.conv.open = false,
            },
            convert::Action::Start => self.start_conversion(&src),
            convert::Action::None => {}
        }
    }

    /// Déplace le fichier affiché vers un dossier choisi ; le fichier suivant du dossier s'affiche ensuite.
    fn move_to(&mut self, ctx: &egui::Context, path: &Path) {
        let mut d = rfd::FileDialog::new().set_title(tr("t.move_to"));
        if let Some(dir) = crate::dat::real_parent(path) {
            d = d.set_directory(dir);
        }
        let Some(dest) = d.pick_folder() else { return };
        match crate::fileops::move_to(path, &dest) {
            Ok(new) => {
                let idx = self.index.unwrap_or(0);
                self.wanted = None;
                self.reload_dir();
                match self.files.get(idx.min(self.files.len().saturating_sub(1))).cloned() {
                    Some(next) => self.show_file(ctx, &next, false),
                    None => {
                        self.out.stop();
                        self.out.set(None);
                        self.playing = false;
                        self.loaded = None;
                        self.dec = None;
                        self.tex = None;
                        self.peaks.clear();
                    }
                }
                self.toast(i18n::trf("t.moved_to", &[("path", new.display().to_string())]));
            }
            Err(e) => self.toast(i18n::trf("t.op_failed", &[("error", e.to_string())])),
        }
    }

    fn open_rename(&mut self, path: &Path) {
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        self.rename_dlg = Some(RenameDlg { path: path.to_path_buf(), name, fresh: true });
    }

    /// Fenêtre « Renommer » : le nom sans l'extension est sélectionné ; un nom invalide ou déjà pris est refusé.
    fn rename_window(&mut self, ctx: &egui::Context) {
        let Some(mut dlg) = self.rename_dlg.take() else { return };
        let name = dlg.name.trim().to_string();
        let current = dlg.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let invalid = crate::fileops::validate_name(&name).is_err();
        let exists = !invalid && !name.eq_ignore_ascii_case(&current) && dlg.path.with_file_name(&name).exists();
        let can = !invalid && !exists && name != current;
        let (mut go, mut cancel) = (false, false);
        let modal = egui::Modal::new(Id::new("rename_dlg")).frame(crate::dialog::frame(ctx)).show(ctx, |ui| {
            ui.set_width(480.0);
            crate::dialog::setup(ui);
            cancel |= crate::dialog::title(ui, &tr("t.rename"));
            ui.add_space(4.0);
            let mut out = egui::TextEdit::singleline(&mut dlg.name).desired_width(f32::INFINITY).show(ui);
            if dlg.fresh {
                out.response.request_focus();
                let stem_end = dlg.name.rfind('.').filter(|i| *i > 0).unwrap_or(dlg.name.len());
                let chars = dlg.name[..stem_end].chars().count();
                out.state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(chars))));
                out.state.store(ui.ctx(), out.response.id);
            }
            if out.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && can {
                go = true;
            }
            if invalid && !name.is_empty() {
                ui.colored_label(ui.visuals().error_fg_color, tr("t.rename_invalid"));
            } else if exists {
                ui.colored_label(ui.visuals().error_fg_color, tr("t.rename_exists"));
            }
            ui.separator();
            ui.horizontal(|ui| {
                go |= crate::dialog::button_enabled(ui, can, tr("c.ok")).clicked();
                cancel |= crate::dialog::button(ui, tr("c.cancel")).clicked();
            });
        });
        if go {
            match crate::fileops::rename(&dlg.path, &name) {
                Ok(new) => {
                    let play = self.playing;
                    self.wanted = None;
                    self.reload_dir();
                    self.show_file(ctx, &new, play);
                    self.toast(i18n::trf("t.renamed", &[("name", name)]));
                }
                Err(e) => self.toast(i18n::trf("t.op_failed", &[("error", e.to_string())])),
            }
        } else if !cancel && !modal.should_close() {
            dlg.fresh = false;
            self.rename_dlg = Some(dlg);
        }
    }

    fn copy_to(&mut self, path: &Path) {
        let mut d = rfd::FileDialog::new().set_title(tr("t.copy_to"));
        if let Some(dir) = crate::dat::real_parent(path) {
            d = d.set_directory(dir);
        }
        let Some(dest) = d.pick_folder() else { return };
        match crate::fileops::copy_to(path, &dest) {
            Ok(new) => self.toast(i18n::trf("t.copied_to", &[("path", new.display().to_string())])),
            Err(e) => self.toast(i18n::trf("t.op_failed", &[("error", e.to_string())])),
        }
    }

    fn toggle_fullscreen(&mut self, ctx: &egui::Context) {
        self.fullscreen = !self.fullscreen;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
    }

    fn associate_files(&mut self, types: &[crate::assoc::FileType]) {
        let exts = crate::assoc::list(types);
        match crate::assoc::associate(types) {
            Ok(()) => {
                let key = if cfg!(windows) { "assoc.done_win" } else { "assoc.done" };
                self.toast(i18n::trf(key, &[("exts", exts)]));
            }
            Err(e) => self.toast(i18n::trf("assoc.failed", &[("exts", exts), ("error", e)])),
        }
    }

    fn pick_folder(&mut self, ctx: &egui::Context) {
        let mut d = rfd::FileDialog::new().set_title(tr("m.open_folder"));
        if let Some(dir) = &self.settings.last_dir {
            d = d.set_directory(dir);
        }
        if let Some(p) = d.pick_folder() {
            self.open_dir(ctx, &p, true);
            self.reveal_in_tree();
        }
    }

    fn pick_file(&mut self, ctx: &egui::Context) {
        let mut d = rfd::FileDialog::new().set_title(tr("m.open_file")).add_filter("ACM / MP3 / WAV / MVE / BIK", &["acm", "mp3", "wav", "mve", "bik"]);
        if let Some(dir) = &self.settings.last_dir {
            d = d.set_directory(crate::dat::real_parent(dir).unwrap_or_else(|| dir.clone()));
        }
        if let Some(p) = d.pick_file() {
            self.open_path(ctx, &p, true);
        }
    }

    fn pick_dat(&mut self, ctx: &egui::Context) {
        let mut d = rfd::FileDialog::new().set_title(tr("m.open_dat")).add_filter("DAT / ZIP / 7z", &["zip", "7z", "dat", "DAT", "patch0", "patch1", "patch2", "patch3", "patch4", "patch5", "patch6", "patch7", "patch8", "patch9", "PATCH0", "PATCH1", "PATCH2", "PATCH3", "PATCH4", "PATCH5", "PATCH6", "PATCH7", "PATCH8", "PATCH9"]);
        if let Some(dir) = &self.settings.last_dir
            && let Some(p) = crate::dat::real_parent(dir)
        {
            d = d.set_directory(p);
        }
        if let Some(p) = d.pick_file() {
            self.open_path(ctx, &p, false);
        }
    }

    fn extract_from_archive(&mut self, path: &Path) {
        if self.extract_job.is_some() {
            return;
        }
        let mut d = rfd::FileDialog::new().set_title(tr("t.extract"));
        if let Some(dir) = &self.settings.last_dir {
            d = d.set_directory(crate::dat::real_parent(dir).unwrap_or_else(|| dir.clone()));
        }
        let Some(out) = d.pick_folder() else { return };
        let (src, dest) = (path.to_path_buf(), out.clone());
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(crate::dat::extract(&src, &dest, false).map_err(|e| e.to_string()));
        });
        self.extract_job = Some((rx, out));
        self.toast(tr("t.extracting"));
    }

    fn poll_extract(&mut self, ctx: &egui::Context) {
        let Some((rx, out)) = &self.extract_job else { return };
        match rx.try_recv() {
            Ok(result) => {
                let dir = out.display().to_string();
                self.extract_job = None;
                match result {
                    Ok(r) => self.toast(i18n::trf("t.extract_done", &[("written", r.written.to_string()), ("skipped", r.skipped.to_string()), ("dir", dir)])),
                    Err(e) => self.toast(i18n::trf("t.extract_failed", &[("error", e)])),
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.extract_job = None,
        }
    }

    // ------------------------------------------------------------------ entrées

    fn handle_input(&mut self, ctx: &egui::Context) {
        if self.folder_dlg.is_some() || self.expand_confirm.is_some() {
            return;
        }
        if self.show_about && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.show_about = false;
            return;
        }
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if let Some(p) = dropped.first() {
            self.open_path(ctx, p, true);
        }
        // Boutons latéraux de la souris : fichier précédent / suivant, ou lecture (pause) / fichier suivant.
        let (back, fwd) = ctx.input(|i| (i.pointer.button_pressed(egui::PointerButton::Extra1), i.pointer.button_pressed(egui::PointerButton::Extra2)));
        if back {
            if self.settings.xbtn_play {
                self.toggle_play();
            } else {
                self.step(ctx, -1, self.playing);
            }
        }
        if fwd {
            self.step(ctx, 1, self.playing);
        }
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        use egui::Key;
        let pressed = |k: Key| ctx.input(|i| i.key_pressed(k));
        let ctrl = ctx.input(|i| i.modifiers.command);
        // Saisie au clavier au-dessus de l'arborescence : saute au premier nom qui commence par les caractères tapés.
        let over_tree = self.tree_rect.is_some_and(|r| ctx.input(|i| i.pointer.hover_pos().is_some_and(|p| r.contains(p))));
        if over_tree && !ctrl {
            let typed: String = ctx.input(|i| i.events.iter().filter_map(|e| if let egui::Event::Text(t) = e { Some(t.clone()) } else { None }).collect());
            let empty = self.ta_buf.is_empty() || self.ta_time.elapsed() > Duration::from_millis(1200);
            let typed: String = typed.chars().filter(|c| !c.is_control() && (!c.is_whitespace() || !empty)).collect();
            if !typed.is_empty() {
                self.type_ahead(ctx, &typed);
                return;
            }
        }
        if ctrl {
            if pressed(Key::N) {
                self.pick_folder(ctx);
            } else if pressed(Key::O) {
                self.pick_file(ctx);
            } else if pressed(Key::D) {
                self.pick_dat(ctx);
            } else if pressed(Key::L) {
                self.open_folder_dialog();
            } else if pressed(Key::E) {
                self.open_convert();
            } else if pressed(Key::Enter) {
                self.open_default();
            } else if pressed(Key::Q) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            return;
        }
        if pressed(Key::F1) {
            self.show_about = true;
        }
        if pressed(Key::Space) {
            self.toggle_play();
        }
        if pressed(Key::ArrowLeft) {
            self.seek(self.vt - 5.0);
        }
        if pressed(Key::ArrowRight) {
            self.seek(self.vt + 5.0);
        }
        if pressed(Key::ArrowDown) || pressed(Key::PageDown) {
            self.step(ctx, 1, self.playing);
        }
        if pressed(Key::ArrowUp) || pressed(Key::PageUp) || pressed(Key::Backspace) {
            self.step(ctx, -1, self.playing);
        }
        if pressed(Key::Home) {
            self.goto(ctx, 0);
        }
        if pressed(Key::End) && !self.files.is_empty() {
            self.goto(ctx, self.files.len() - 1);
        }
        if pressed(Key::Plus) || pressed(Key::Equals) {
            self.zoom_by(1.25);
        }
        if pressed(Key::Minus) {
            self.zoom_by(0.8);
        }
        if pressed(Key::Enter) {
            self.zoom_actual();
        }
        if pressed(Key::F) {
            self.fit = true;
            self.pan = Vec2::ZERO;
        }
        if pressed(Key::F5) {
            self.reload_dir();
        }
        if pressed(Key::I) {
            self.settings.show_props = !self.settings.show_props;
        }
        if pressed(Key::T) {
            self.settings.show_tree = !self.settings.show_tree;
        }
        if pressed(Key::F11) || (pressed(Key::Escape) && self.fullscreen) {
            self.toggle_fullscreen(ctx);
        }
    }

    /// Recherche au clavier dans l'arborescence : les caractères tapés s'accumulent pendant 1,2 s ; la même lettre répétée
    /// passe à l'élément suivant qui commence par elle.
    fn type_ahead(&mut self, ctx: &egui::Context, typed: &str) {
        if self.ta_time.elapsed() > Duration::from_millis(1200) {
            self.ta_buf.clear();
        }
        self.ta_time = Instant::now();
        self.ta_buf.push_str(&typed.to_lowercase());
        let rows = &self.tree.rows;
        let n = rows.len();
        if n == 0 {
            return;
        }
        let cur = rows
            .iter()
            .position(|r| matches!(r.kind, RowKind::File) && self.wanted.as_deref() == Some(r.path.as_path()))
            .or_else(|| rows.iter().position(|r| self.tree.selected_dir.as_deref() == Some(r.path.as_path())));
        let first = self.ta_buf.chars().next().unwrap_or(' ');
        let repeated = self.ta_buf.chars().count() > 1 && self.ta_buf.chars().all(|c| c == first);
        let (needle, start) = if repeated {
            (first.to_string(), cur.map_or(0, |c| c + 1))
        } else if self.ta_buf.chars().count() == 1 {
            (self.ta_buf.clone(), cur.map_or(0, |c| c + 1))
        } else {
            (self.ta_buf.clone(), cur.unwrap_or(0))
        };
        let found = (0..n).map(|k| (start + k) % n).find(|&i| rows[i].name.to_lowercase().starts_with(&needle));
        let Some(i) = found else { return };
        let row = rows[i].clone();
        match row.kind {
            RowKind::File => self.show_file(ctx, &row.path, false),
            RowKind::Dir { .. } => self.open_dir(ctx, &row.path, false),
        }
        self.tree_scroll_to = self.tree.rows.iter().position(|r| r.path == row.path);
    }

    // ------------------------------------------------------------------ menu

    fn menu_bar(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        fn item(ui: &mut Ui, label: String, shortcut: &str, enabled: bool) -> bool {
            let r = ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(shortcut));
            if r.clicked() {
                ui.close();
            }
            r.clicked()
        }
        let has_file = self.loaded.is_some();
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(tr("m.file"), |ui| {
                if item(ui, tr("m.open_folder"), "Ctrl+N", true) {
                    self.pick_folder(&ctx);
                }
                if item(ui, tr("m.open_file"), "Ctrl+O", true) {
                    self.pick_file(&ctx);
                }
                if item(ui, tr("m.open_dat"), "Ctrl+D", true) {
                    self.pick_dat(&ctx);
                }
                ui.separator();
                if item(ui, tr("t.refresh"), "F5", true) {
                    self.reload_dir();
                }
                ui.separator();
                if item(ui, tr("av.export"), "Ctrl+E", has_file) {
                    self.open_convert();
                }
                ui.separator();
                if item(ui, tr("m.open_default"), "Ctrl+Enter", has_file) {
                    self.open_default();
                }
                ui.separator();
                if item(ui, tr("m.exit"), "Ctrl+Q", true) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button(tr("m.view"), |ui| {
                ui.checkbox(&mut self.settings.show_tree, tr("t.tree"));
                ui.checkbox(&mut self.settings.show_props, tr("properties"));
                ui.separator();
                if item(ui, tr("m.fullscreen"), "F11", true) {
                    self.toggle_fullscreen(&ctx);
                }
                ui.separator();
                ui.menu_button(tr("av.wave_color"), |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.set_max_width(150.0);
                        for c in WAVE_COLORS {
                            let (rect, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
                            ui.painter().rect_filled(rect, 4.0, Color32::from_rgb(c[0], c[1], c[2]));
                            if self.settings.wave_color == c {
                                ui.painter().rect_stroke(rect, 4.0, Stroke::new(2.0, ui.visuals().strong_text_color()), egui::StrokeKind::Inside);
                            }
                            if resp.clicked() {
                                self.settings.wave_color = c;
                            }
                        }
                    });
                });
                ui.separator();
                ui.checkbox(&mut self.win.remember_size, tr("m.remember_size"));
                ui.checkbox(&mut self.win.remember_pos, tr("m.remember_pos"));
            });
            ui.menu_button(tr("m.navigation"), |ui| {
                if item(ui, tr("m.current_folder"), "Ctrl+L", true) {
                    self.open_folder_dialog();
                }
                ui.separator();
                ui.label(tr("m.sort"));
                let mut sort_changed = false;
                for (key, label) in [(SortKey::Name, "m.sort_name"), (SortKey::Size, "m.sort_size"), (SortKey::Modified, "m.sort_date"), (SortKey::Type, "m.sort_type")] {
                    sort_changed |= ui.radio_value(&mut self.settings.sort, key, tr(label)).changed();
                }
                sort_changed |= ui.checkbox(&mut self.settings.sort_desc, tr("m.sort_desc")).changed();
                if sort_changed {
                    self.apply_view_options();
                }
                ui.separator();
                ui.label(tr("m.xbuttons"));
                ui.radio_value(&mut self.settings.xbtn_play, false, tr("av.xb_file"));
                ui.radio_value(&mut self.settings.xbtn_play, true, tr("av.xb_play"));
                ui.separator();
                ui.label(tr("av.channels_mode"));
                let mut changed = false;
                for (m, k) in [(ChannelMode::Auto, "av.ch_auto"), (ChannelMode::Mono, "av.ch_mono"), (ChannelMode::Stereo, "av.ch_stereo")] {
                    changed |= ui.radio_value(&mut self.settings.chan_mode, m, tr(k)).changed();
                }
                if changed && let Some(p) = self.wanted.clone() {
                    self.show_file(&ctx, &p, self.playing);
                }
            });
            ui.menu_button(tr("m.language"), |ui| {
                for l in Lang::ALL {
                    if ui.radio(i18n::lang() == l, l.native_name()).clicked() {
                        self.set_language(l);
                        ui.close();
                    }
                }
            });
            ui.menu_button(tr("m.help"), |ui| {
                let can = crate::assoc::is_supported();
                if item(ui, tr("av.assoc_audio"), "", can) {
                    self.associate_files(&crate::assoc::AUDIO);
                }
                if item(ui, tr("av.assoc_video"), "", can) {
                    self.associate_files(&crate::assoc::VIDEO);
                }
                ui.separator();
                if item(ui, tr("m.about"), "F1", true) {
                    self.show_about = true;
                }
            });
        });
    }

    // ------------------------------------------------------------------ barre d'outils

    fn toolbar(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        ui.add_space(3.0);
        let mut group_right = 0.0f32;
        let bar = ui
            .horizontal(|ui| {
                let pad = ((ui.available_width() - self.toolbar_w) / 2.0).max(0.0);
                ui.add_space(pad);
                let group = ui.horizontal(|ui| {
                    ui.spacing_mut().interact_size.y = 28.0;
                    ui.checkbox(&mut self.settings.show_tree, tr("t.tree"));
                    ui.separator();
                    if icon_button(ui, Icon::Folder, &format!("{}  (Ctrl+L)", tr("t.folder")), false, true).clicked() {
                        self.open_folder_dialog();
                    }
                    ui.separator();
                    let has_files = self.files.len() > 1;
                    if icon_button(ui, Icon::Prev, &tr("av.prev"), false, has_files).clicked() {
                        self.step(&ctx, -1, self.playing);
                    }
                    if icon_button(ui, Icon::Next, &tr("av.next"), false, has_files).clicked() {
                        self.step(&ctx, 1, self.playing);
                    }
                    ui.separator();
                    if ui.checkbox(&mut self.settings.looping, tr("av.loop")).on_hover_text(tr("av.loop_tip")).changed() {
                        self.sync_loop();
                    }
                    if ui.checkbox(&mut self.settings.auto_next, tr("av.auto_short")).on_hover_text(tr("av.auto_next")).changed() {
                        self.sync_loop();
                    }
                    ui.separator();
                    ui.label(tr("av.volume"));
                    ui.spacing_mut().slider_width = 90.0;
                    if ui.add(egui::Slider::new(&mut self.settings.volume, 0.0..=1.0).show_value(false)).changed() {
                        self.out.set_volume(self.settings.volume);
                    }
                    ui.separator();
                    ui.label(tr("av.speed"));
                    // Champ numérique (nombre seul) avec boutons - / + par pas de 0,1.
                    let mut sp = self.settings.speed;
                    let mut changed = false;
                    if ui.small_button("-").clicked() {
                        sp -= 0.1;
                        changed = true;
                    }
                    let field = ui.add(egui::DragValue::new(&mut sp).range(0.2..=5.0).speed(0.02).fixed_decimals(1));
                    changed |= field.changed();
                    field.on_hover_text(tr("av.speed_tip"));
                    if ui.small_button("+").clicked() {
                        sp += 0.1;
                        changed = true;
                    }
                    if changed {
                        self.settings.speed = ((sp.clamp(0.2, 5.0)) * 10.0).round() / 10.0;
                        self.out.set_speed(self.settings.speed);
                    }
                    ui.separator();
                    ui.checkbox(&mut self.settings.show_props, tr("properties"));
                });
                group_right = group.response.rect.right();
                let w = group.response.rect.width();
                if (w - self.toolbar_w).abs() > 0.5 {
                    self.toolbar_w = w;
                    ctx.request_repaint();
                }
            })
            .response
            .rect;
        // Informations à droite de la barre : rang du fichier dans le dossier.
        if let Some(i) = self.index {
            let info = format!("{} / {}", i + 1, self.files.len());
            let galley = ui.painter().layout_no_wrap(info, FontId::proportional(13.0), ui.visuals().weak_text_color());
            if bar.right() - 8.0 - galley.size().x > group_right + 12.0 {
                ui.painter().galley(pos2(bar.right() - 8.0 - galley.size().x, bar.center().y - galley.size().y / 2.0), galley, Color32::PLACEHOLDER);
            }
        }
        ui.add_space(3.0);
    }

    /// Commandes de lecture, en bas de la zone centrale : lecture / pause, arrêt, position / durée et barre de progression.
    fn transport(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let dur = self.duration();
        let enabled = self.loaded.is_some();
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().interact_size.y = 28.0;
            let (icon, tip) = if self.playing { (Icon::Pause, tr("av.pause")) } else { (Icon::Play, tr("av.play")) };
            if icon_button(ui, icon, &format!("{}  (Space)", tip), false, enabled).clicked() {
                self.toggle_play();
            }
            if icon_button(ui, Icon::Stop, &tr("av.stop"), false, enabled).clicked() {
                self.stop();
            }
            let time = format!("{} / {}", fmt_time(self.vt), fmt_time(dur));
            let time_w = ui.painter().layout_no_wrap(time.clone(), FontId::proportional(14.0), Color32::WHITE).size().x;
            let mut t = self.vt;
            ui.spacing_mut().slider_width = (ui.available_width() - time_w - 28.0).max(60.0);
            let slider = ui.add_enabled(enabled && dur > 0.0, egui::Slider::new(&mut t, 0.0..=dur.max(0.001)).show_value(false).trailing_fill(true));
            if slider.changed() {
                self.seek(t);
                self.update_video(&ctx, false);
            }
            ui.label(time);
        });
        ui.add_space(3.0);
    }

    // ------------------------------------------------------------------ arborescence

    /// Boutons d'accès rapide (8 au plus) : Bureau, Documents, Téléchargements (réservés) puis les favoris ;
    /// clic droit sur un favori : le retirer.
    fn favorites_bar(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let mut open: Option<PathBuf> = None;
        let mut remove: Option<usize> = None;
        let cur = self.tree.selected_dir.clone();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
            for (key, path) in &self.special {
                let label = tr(key);
                let tip = format!("{label}\n{}", path.display());
                let icon = match *key {
                    "t.desktop" => Icon::Desktop,
                    "t.documents" => Icon::Documents,
                    _ => Icon::Downloads,
                };
                if icon_button(ui, icon, &tip, cur.as_ref() == Some(path), true).clicked() {
                    open = Some(path.clone());
                }
            }
            for (i, path) in self.settings.favorites.iter().enumerate() {
                let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.display().to_string());
                let tip = format!("{name}\n{}", path.display());
                let r = letter_button(ui, &letter_of(&name), &tip, cur.as_ref() == Some(path));
                if r.clicked() {
                    open = Some(path.clone());
                }
                r.context_menu(|ui| {
                    if ui.button(tr("t.fav_remove")).clicked() {
                        remove = Some(i);
                        ui.close();
                    }
                });
            }
        });
        if let Some(i) = remove {
            self.settings.favorites.remove(i);
        }
        if let Some(p) = open {
            if crate::dat::stat(&p).is_some() || p.is_dir() {
                self.open_dir(ctx, &p, false);
                self.reveal_in_tree();
            } else {
                self.toast(tr("t.folder_missing"));
            }
        }
    }

    fn tree_panel(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.tree_rect = Some(ui.max_rect());
        ui.add_space(5.0);
        self.favorites_bar(ui, &ctx);
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            let clear = !self.file_filter.is_empty();
            let w = ui.available_width() - if clear { 30.0 } else { 0.0 };
            let edit = ui.add(egui::TextEdit::singleline(&mut self.file_filter).hint_text(tr("t.filter")).desired_width(w).margin(vec2(8.0, 7.0)));
            let cleared = clear && ui.button("×").clicked();
            if cleared {
                self.file_filter.clear();
            }
            if edit.changed() || cleared {
                self.apply_view_options();
            }
        });
        ui.add_space(5.0);
        if self.tree_w.0 != self.tree.rev {
            let font = FontId::proportional(13.5);
            let est = |r: &crate::tree::Row| r.depth as f32 * 14.0 + r.name.chars().map(|c| if c.is_ascii() { 1.0 } else { 2.0 }).sum::<f32>() * 7.0;
            let mut cand: Vec<&crate::tree::Row> = self.tree.rows.iter().collect();
            if cand.len() > 64 {
                cand.select_nth_unstable_by(63, |a, b| est(b).total_cmp(&est(a)));
                cand.truncate(64);
            }
            let widest = cand
                .iter()
                .map(|r| 4.0 + r.depth as f32 * 14.0 + 33.0 + ui.painter().layout_no_wrap(r.name.clone(), font.clone(), Color32::WHITE).size().x + 10.0)
                .fold(0.0, f32::max);
            self.tree_w = (self.tree.rev, widest);
        }
        let content_w = self.tree_w.1;
        let row_h = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
        let total = self.tree.rows.len();
        let cur = self.wanted.clone();
        let sel_dir = self.tree.selected_dir.clone();
        let mut action: Option<(usize, bool)> = None;
        let mut ctx_act: Option<(PathBuf, u8)> = None;
        let favs = self.settings.favorites.clone();
        let fav_free = favs.len() < MAX_FAVORITES;
        let mut area = egui::ScrollArea::both().auto_shrink([false, false]);
        if let Some(r) = self.tree_scroll_to.take() {
            let target = (r as f32 * row_h - ui.available_height() * 0.5).max(0.0);
            area = area.vertical_scroll_offset(target);
        }
        area.show_rows(ui, row_h, total, |ui, range| {
            ui.set_min_width(content_w);
            let w = ui.available_width();
            for i in range {
                let row = &self.tree.rows[i];
                let (rect, resp) = ui.allocate_exact_size(vec2(w, row_h), Sense::click());
                let selected = matches!(row.kind, RowKind::File) && cur.as_deref() == Some(row.path.as_path());
                let is_sel_dir = matches!(row.kind, RowKind::Dir { .. }) && sel_dir.as_deref() == Some(row.path.as_path());
                let vis = ui.visuals();
                if selected {
                    ui.painter().rect_filled(rect, 3.0, vis.selection.bg_fill);
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 3.0, vis.widgets.hovered.bg_fill.gamma_multiply(0.6));
                }
                let x0 = rect.left() + 4.0 + row.depth as f32 * 14.0;
                let cy = rect.center().y;
                let text_col = if selected { vis.selection.stroke.color } else { vis.text_color() };
                match row.kind {
                    RowKind::Dir { expanded } => {
                        let c = pos2(x0 + 6.0, cy);
                        let pts = if expanded {
                            vec![c + vec2(-4.0, -2.5), c + vec2(4.0, -2.5), c + vec2(0.0, 3.0)]
                        } else {
                            vec![c + vec2(-2.5, -4.0), c + vec2(-2.5, 4.0), c + vec2(3.0, 0.0)]
                        };
                        ui.painter().add(egui::Shape::convex_polygon(pts, vis.weak_text_color(), Stroke::NONE));
                        let fx = x0 + 15.0;
                        let fcol = if row.archive {
                            if is_sel_dir { Color32::from_rgb(120, 170, 235) } else { Color32::from_rgb(90, 140, 205) }
                        } else if is_sel_dir {
                            Color32::from_rgb(240, 190, 70)
                        } else {
                            Color32::from_rgb(200, 160, 60)
                        };
                        ui.painter().rect_filled(Rect::from_min_size(pos2(fx, cy - 4.0), vec2(13.0, 9.0)), 1.5, fcol);
                        ui.painter().rect_filled(Rect::from_min_size(pos2(fx, cy - 6.0), vec2(6.0, 3.0)), 1.0, fcol);
                    }
                    RowKind::File => {
                        let fx = x0 + 15.0;
                        let col = if selected { text_col } else { vis.weak_text_color() };
                        ui.painter().rect_stroke(Rect::from_min_size(pos2(fx + 1.0, cy - 6.0), vec2(10.0, 12.0)), 1.5, Stroke::new(1.2, col), egui::StrokeKind::Inside);
                        ui.painter().line_segment([pos2(fx + 3.0, cy + 2.0), pos2(fx + 6.0, cy - 1.0)], Stroke::new(1.0, col));
                        ui.painter().line_segment([pos2(fx + 6.0, cy - 1.0), pos2(fx + 9.0, cy + 2.0)], Stroke::new(1.0, col));
                    }
                }
                let text_x = x0 + 15.0 + 18.0;
                let clip = ui.painter().with_clip_rect(rect);
                let font = FontId::proportional(13.5);
                clip.text(pos2(text_x, cy), Align2::LEFT_CENTER, &row.name, font, if is_sel_dir && !selected { vis.strong_text_color() } else { text_col });
                if resp.clicked() {
                    let on_arrow = matches!(row.kind, RowKind::Dir { .. }) && resp.interact_pointer_pos().is_some_and(|p| p.x < x0 + 14.0);
                    action = Some((i, on_arrow));
                }
                let is_dir = matches!(row.kind, RowKind::Dir { .. });
                resp.context_menu(|ui| {
                    if ui.add_enabled(is_dir, egui::Button::new(tr("t.expand_all"))).clicked() {
                        ctx_act = Some((row.path.clone(), 0));
                        ui.close();
                    }
                    if ui.button(tr("t.collapse_others")).clicked() {
                        ctx_act = Some((row.path.clone(), 1));
                        ui.close();
                    }
                    ui.separator();
                    if ui.add(egui::Button::new(tr("t.refresh")).shortcut_text("F5")).clicked() {
                        let target = if is_dir { row.path.clone() } else { sel_dir.clone().unwrap_or_else(|| row.path.clone()) };
                        ctx_act = Some((target, 4));
                        ui.close();
                    }
                    // Favoris : grisé pour un fichier, un dossier déjà favori, ou quand les 5 places sont prises.
                    let can_fav = is_dir && fav_free && !favs.contains(&row.path);
                    if ui.add_enabled(can_fav, egui::Button::new(tr("t.fav_add"))).clicked() {
                        ctx_act = Some((row.path.clone(), 5));
                        ui.close();
                    }
                    if ui.button(tr("t.open_explorer")).clicked() {
                        ctx_act = Some((row.path.clone(), 2));
                        ui.close();
                    }
                    ui.separator();
                    if ui.add_enabled(crate::dat::is_virtual(&row.path), egui::Button::new(tr("t.extract"))).clicked() {
                        ctx_act = Some((row.path.clone(), 3));
                        ui.close();
                    }
                });
            }
        });
        if let Some((path, what)) = ctx_act {
            match what {
                0 => {
                    if self.tree.count_dirs(&path, 100) > 100 {
                        self.expand_confirm = Some(path);
                    } else {
                        self.tree.expand_all(&path);
                    }
                }
                2 => {
                    if let Err(e) = crate::opener::reveal(&path) {
                        self.toast(e);
                    }
                }
                3 => self.extract_from_archive(&path),
                4 => {
                    self.tree.reload_dir(&path);
                    self.apply_view_options();
                }
                5 => {
                    if self.settings.favorites.len() < MAX_FAVORITES && !self.settings.favorites.contains(&path) {
                        self.settings.favorites.push(path);
                    }
                }
                _ => self.tree.collapse_others(&path),
            }
        }
        if let Some((i, on_arrow)) = action {
            let row = self.tree.rows[i].clone();
            match row.kind {
                RowKind::Dir { .. } => {
                    if on_arrow {
                        self.tree.toggle(&row.path);
                    } else {
                        self.open_dir(&ctx, &row.path, false);
                    }
                }
                RowKind::File => self.show_file(&ctx, &row.path, true),
            }
        }
    }

    // ------------------------------------------------------------------ propriétés

    fn props_panel(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        ui.heading(tr("properties"));
        ui.separator();
        let (Some(path), Some(l)) = (self.wanted.clone(), &self.loaded) else {
            ui.label(egui::RichText::new(tr("av.no_media")).weak());
            return;
        };
        let secs = self.collect_props(&path, l);
        let sha = l.sha256.clone();
        let ref_hash = self.ref_hash.clone();
        // Un fichier d'archive .DAT peut être copié mais ni déplacé ni renommé.
        let on_disk = !crate::dat::is_virtual(&path);
        let mut action: Option<PropAction> = None;
        egui::Panel::bottom("props_copy").show(ui, |ui| {
            ui.add_space(3.0);
            // Comparaison avec la dernière empreinte copiée ; la croix arrête le mode comparaison.
            if let Some((name, h)) = &ref_hash {
                // Ligne de hauteur fixe, contenu centré verticalement (texte et bouton croix).
                ui.allocate_ui_with_layout(vec2(ui.available_width(), 30.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let (color, key) = if *h == sha { (Color32::from_rgb(90, 190, 120), "t.sha_same") } else { (Color32::from_rgb(230, 120, 90), "t.sha_diff") };
                    ui.colored_label(color, i18n::trf(key, &[("name", name.clone())]));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon_button(ui, Icon::Close, &tr("t.sha_stop"), false, true).clicked() {
                            action = Some(PropAction::ShaStop);
                        }
                    });
                });
                ui.separator();
            }
            // Boutons : chemin ; copier, déplacer, renommer ; convertir ; copier les informations (texte, JSON, XML) et le SHA-256.
            ui.horizontal_wrapped(|ui| {
                let mut button = |ui: &mut Ui, icon: Icon, tip: &str, enabled: bool, act: PropAction| {
                    if icon_button(ui, icon, &tr(tip), false, enabled).clicked() {
                        action = Some(act);
                    }
                };
                button(ui, Icon::CopyPath, "t.copy_path", true, PropAction::CopyPath);
                ui.separator();
                button(ui, Icon::CopyTo, "t.copy_to", true, PropAction::CopyTo);
                button(ui, Icon::MoveTo, "t.move_to", on_disk, PropAction::MoveTo);
                button(ui, Icon::Rename, "t.rename", on_disk, PropAction::Rename);
                ui.separator();
                button(ui, Icon::Convert, "av.export", true, PropAction::Convert);
                ui.separator();
                button(ui, Icon::CopyText, "t.copy_text", true, PropAction::Text);
                button(ui, Icon::CopyJson, "t.copy_json", true, PropAction::Json);
                button(ui, Icon::CopyXml, "t.copy_xml", true, PropAction::Xml);
                button(ui, Icon::Sha, "t.copy_sha", true, PropAction::Sha);
            });
            // Pas d'espace en bas : la ligne d'icônes en ajoute déjà un (espacement vertical), ce qui égale la marge du haut.
        });
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for (sid, (title, items)) in secs.iter().enumerate() {
                egui::CollapsingHeader::new(egui::RichText::new(tr(title)).strong()).id_salt(("sec", title)).default_open(true).show(ui, |ui| {
                    egui::Grid::new(("grid", sid)).num_columns(2).striped(true).spacing([10.0, 3.0]).show(ui, |ui| {
                        for (k, v) in items {
                            ui.label(egui::RichText::new(tr(k)).weak());
                            ui.add(egui::Label::new(v.clone()).wrap().selectable(true));
                            ui.end_row();
                        }
                    });
                });
            }
        });
        let ctx = ui.ctx().clone();
        // Sections traduites pour les copies texte / JSON / XML.
        let translated: crate::props::PropSections = secs.iter().map(|(t, rows)| (tr(t), rows.iter().map(|(k, v)| (tr(k), v.clone())).collect())).collect();
        let info = match action {
            Some(PropAction::Text) => Some(crate::props::props_text(&translated)),
            Some(PropAction::Json) => Some(crate::props::props_json(&translated)),
            Some(PropAction::Xml) => Some(crate::props::props_xml(&translated)),
            _ => None,
        };
        if let Some(out) = info {
            ctx.copy_text(out);
            self.toast(tr("t.copied"));
        }
        match action {
            Some(PropAction::CopyPath) => {
                ctx.copy_text(path.display().to_string());
                self.toast(tr("t.path_copied"));
            }
            Some(PropAction::CopyTo) => self.copy_to(&path),
            Some(PropAction::MoveTo) => self.move_to(&ctx, &path),
            Some(PropAction::Rename) => self.open_rename(&path),
            Some(PropAction::Convert) => self.open_convert(),
            Some(PropAction::Sha) => {
                ctx.copy_text(sha.clone());
                let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                self.ref_hash = Some((name, sha));
                self.toast(tr("t.sha_copied"));
            }
            Some(PropAction::ShaStop) => self.ref_hash = None,
            _ => {}
        }
    }

    /// Sections du panneau : (titre, [(clé, valeur)]).
    fn collect_props(&self, path: &Path, l: &Loaded) -> Vec<(String, Vec<(String, String)>)> {
        let mut secs = Vec::new();
        let mut file = vec![
            ("p.name".to_string(), path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()),
            ("p.folder".to_string(), path.parent().map(|p| p.display().to_string()).unwrap_or_default()),
            ("p.size".to_string(), format!("{} ({} Ko)", l.size, l.size.div_ceil(1024))),
            ("p.sha256".to_string(), l.sha256.clone()),
        ];
        let local = |t: std::time::SystemTime| chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d %H:%M:%S").to_string();
        if let Some(m) = self.modified {
            file.push(("p.modified".to_string(), local(m)));
        }
        if let Some(c) = self.created {
            file.push(("p.created".to_string(), local(c)));
        }
        file.push(("p.format".to_string(), l.info.iter().find(|(k, _)| *k == "format").map(|(_, v)| v.clone()).unwrap_or_default()));
        secs.push(("sec.file".to_string(), file));
        if let Some(w) = &l.warning {
            secs.push(("av.warning".to_string(), vec![("av.warning".to_string(), w.clone())]));
        }
        if let Some(p) = &l.pcm {
            let mut a = vec![
                ("p.duration".to_string(), format!("{} ({:.2} s)", fmt_time(p.duration()), p.duration())),
                ("av.rate".to_string(), format!("{} Hz", p.rate)),
                ("av.channels".to_string(), format!("{} ({})", p.channels, if p.channels == 1 { tr("av.mono") } else { tr("av.stereo") })),
                ("av.bits".to_string(), "16".to_string()),
                ("av.samples".to_string(), p.frames().to_string()),
            ];
            if let Some((_, h)) = l.info.iter().find(|(k, _)| *k == "header_channels") {
                a.push(("av.header_channels".to_string(), h.clone()));
            }
            secs.push((if l.video.is_some() { "av.audio_track" } else { "av.sec_audio" }.to_string(), a));
        }
        if let Some(m) = &l.video {
            let cur = self.shown_frame.min(m.frame_count().saturating_sub(1)) + 1;
            secs.push((
                "av.sec_video".to_string(),
                vec![
                    ("p.dimensions".to_string(), format!("{} × {}", m.width(), m.height())),
                    ("p.fps".to_string(), format!("{:.2}", m.fps())),
                    ("p.frames".to_string(), m.frame_count().to_string()),
                    ("p.duration".to_string(), format!("{} ({:.2} s)", fmt_time(m.duration()), m.duration())),
                    ("av.frame_now".to_string(), format!("{} / {}", cur, m.frame_count())),
                ],
            ));
        }
        secs
    }

    // ------------------------------------------------------------------ zone centrale

    fn canvas(&mut self, ui: &mut Ui) {
        let rect = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        let dur = self.duration();
        let weak = ui.visuals().weak_text_color();
        if let Some(e) = &self.error {
            ui.painter().rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, e, FontId::proportional(15.0), ui.visuals().error_fg_color);
            return;
        }
        let Some(l) = &self.loaded else {
            ui.painter().rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);
            let msg = if self.loading { tr("av.loading") } else { tr("av.hint") };
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, msg, FontId::proportional(16.0), weak);
            return;
        };
        let mut seek_to: Option<f64> = None;
        let (mut zoom_f, mut actual, mut play_toggle) = (1.0f32, false, false);
        if let (Some(tex), Some(m)) = (&self.tex, l.video.as_ref()) {
            ui.painter().rect_filled(rect, 0.0, Color32::BLACK);
            let (w, h) = (m.width() as f32, m.height() as f32);
            let fit_scale = (rect.width() / w).min(rect.height() / h).max(0.01);
            // Molette : zoom ; glisser : déplacement ; double-clic : ajuster / 100 %.
            if resp.hovered() {
                let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                if scroll != 0.0 {
                    zoom_f = if scroll > 0.0 { 1.1 } else { 1.0 / 1.1 };
                }
            }
            if resp.double_clicked() {
                if self.fit {
                    actual = true;
                } else {
                    self.fit = true;
                    self.pan = Vec2::ZERO;
                }
            }
            if resp.dragged() && !self.fit {
                self.pan += resp.drag_delta();
            }
            let scale = if self.fit { fit_scale } else { self.zoom };
            self.cur_scale = scale;
            let r = Rect::from_center_size(rect.center() + if self.fit { Vec2::ZERO } else { self.pan }, vec2(w * scale, h * scale));
            let filter = if scale >= 1.0 { TextureOptions::NEAREST } else { TextureOptions::LINEAR };
            let _ = filter;
            ui.painter_at(rect).image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            if resp.clicked() && !resp.double_clicked() {
                // Clic simple : lecture / pause, comme dans un lecteur vidéo.
                play_toggle = true;
            }
        } else {
            ui.painter().rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);
            if !self.peaks.is_empty() {
                let wave = Rect::from_min_max(pos2(rect.left() + 24.0, rect.top() + 56.0), pos2(rect.right() - 24.0, rect.bottom() - 24.0));
                let mid = wave.center().y;
                let n = self.peaks.len();
                let [wr, wg, wb] = self.settings.wave_color;
                let col = Color32::from_rgb(wr, wg, wb);
                let played = if dur > 0.0 { (self.vt / dur).clamp(0.0, 1.0) as f32 } else { 0.0 };
                let thick = (wave.width() / n as f32).max(1.0);
                for (i, (lo, hi)) in self.peaks.iter().enumerate() {
                    let x = wave.left() + wave.width() * (i as f32 + 0.5) / n as f32;
                    let c = if (i as f32 / n as f32) < played { col } else { col.gamma_multiply(0.45) };
                    let (y0, y1) = (mid - hi.min(1.0) * wave.height() * 0.5, mid - lo.max(-1.0) * wave.height() * 0.5);
                    ui.painter().line_segment([pos2(x, y0), pos2(x, y1.max(y0 + 1.0))], Stroke::new(thick, c));
                }
                let px = wave.left() + wave.width() * played;
                ui.painter().line_segment([pos2(px, wave.top()), pos2(px, wave.bottom())], Stroke::new(1.5, Color32::from_rgba_unmultiplied(wr, wg, wb, 170)));
                if (resp.clicked() || resp.dragged())
                    && let Some(p) = resp.interact_pointer_pos()
                {
                    seek_to = Some(((p.x - wave.left()) / wave.width()).clamp(0.0, 1.0) as f64 * dur);
                }
                if let Some(name) = self.wanted.as_ref().and_then(|p| p.file_name()) {
                    ui.painter().text(pos2(rect.left() + 24.0, rect.top() + 16.0), Align2::LEFT_TOP, name.to_string_lossy(), FontId::proportional(17.0), ui.visuals().strong_text_color());
                }
            }
        }
        if zoom_f != 1.0 {
            self.zoom_by(zoom_f);
        }
        if actual {
            self.zoom_actual();
        }
        if play_toggle {
            self.toggle_play();
        }
        if let Some(t) = seek_to {
            self.seek(t);
        }
    }

    // ------------------------------------------------------------------ fenêtres annexes

    fn open_folder_dialog(&mut self) {
        let cur = self.tree.selected_dir.clone().or_else(|| self.settings.last_dir.clone());
        self.folder_dlg = Some((cur.map(|p| p.to_string_lossy().to_string()).unwrap_or_default(), true));
    }

    fn folder_window(&mut self, ctx: &egui::Context) {
        let Some((mut text, fresh)) = self.folder_dlg.take() else { return };
        let path = PathBuf::from(text.trim().trim_matches('"'));
        let valid = !text.trim().is_empty() && (matches!(crate::dat::stat(&path), Some(crate::dat::Stat::Dir)) || path.is_dir());
        let (mut go, mut cancel) = (false, false);
        let modal = egui::Modal::new(Id::new("folder_dlg")).frame(crate::dialog::frame(ctx)).show(ctx, |ui| {
            ui.set_width(560.0);
            crate::dialog::setup(ui);
            cancel |= crate::dialog::title(ui, &tr("t.folder"));
            ui.add_space(4.0);
            let mut out = egui::TextEdit::singleline(&mut text).desired_width(f32::INFINITY).show(ui);
            if fresh {
                out.response.request_focus();
                out.state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(text.chars().count()))));
                out.state.store(ui.ctx(), out.response.id);
            }
            if out.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && valid {
                go = true;
            }
            if !valid && !text.trim().is_empty() {
                ui.colored_label(ui.visuals().error_fg_color, tr("t.folder_missing"));
            }
            ui.separator();
            ui.horizontal(|ui| {
                go |= crate::dialog::button_enabled(ui, valid, tr("c.ok")).clicked();
                cancel |= crate::dialog::button(ui, tr("c.cancel")).clicked();
            });
        });
        if go {
            self.open_path(ctx, &path, false);
        } else if !cancel && !modal.should_close() {
            self.folder_dlg = Some((text, false));
        }
    }

    fn expand_confirm_window(&mut self, ctx: &egui::Context) {
        let Some(path) = self.expand_confirm.clone() else { return };
        let (mut yes, mut no) = (false, false);
        let modal = egui::Modal::new(Id::new("expand_confirm")).frame(crate::dialog::frame(ctx)).show(ctx, |ui| {
            ui.set_width(420.0);
            crate::dialog::setup(ui);
            no |= crate::dialog::title(ui, &tr("t.expand_all"));
            ui.add(egui::Label::new(tr("t.expand_confirm")).wrap());
            ui.separator();
            ui.horizontal(|ui| {
                yes = crate::dialog::button(ui, tr("t.expand_all")).clicked();
                no |= crate::dialog::button(ui, tr("c.cancel")).clicked();
            });
        });
        if yes {
            self.tree.expand_all(&path);
        }
        if yes || no || modal.should_close() {
            self.expand_confirm = None;
        }
    }

    fn about_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_about;
        egui::Window::new(tr("m.about"))
            .open(&mut open)
            .frame(crate::dialog::window_frame(ctx))
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.set_min_width(480.0);
                ui.heading("FalloutAVBrowser");
                ui.label(egui::RichText::new(format!("v{}  ·  Rust + egui + wgpu", env!("CARGO_PKG_VERSION"))).weak());
                ui.label(egui::RichText::new(tr("av.about_based")).weak());
                ui.add_space(4.0);
                ui.separator();
                ui.label(egui::RichText::new(tr("about.formats")).strong());
                ui.label(tr("av.about_formats"));
                ui.add_space(4.0);
                ui.separator();
                ui.label(egui::RichText::new(tr("about.keys_title")).strong());
                ui.label(tr("av.about_keys"));
            });
        self.show_about = open;
    }

    fn toast_overlay(&mut self, ctx: &egui::Context) {
        let Some((msg, t)) = &self.toast else { return };
        if t.elapsed() > Duration::from_secs(6) {
            self.toast = None;
            return;
        }
        let msg = msg.clone();
        egui::Area::new(Id::new("toast")).order(egui::Order::Tooltip).anchor(Align2::CENTER_BOTTOM, vec2(0.0, -90.0)).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.label(msg);
            });
        });
        ctx.request_repaint_after(Duration::from_millis(500));
    }

    /// Retient la taille / position de la fenêtre (sauf agrandie, minimisée ou plein écran).
    fn track_window(&mut self, ctx: &egui::Context) {
        let (inner, outer, max, min, full) = ctx.input(|i| {
            let v = i.viewport();
            (v.inner_rect, v.outer_rect, v.maximized.unwrap_or(false), v.minimized.unwrap_or(false), v.fullscreen.unwrap_or(false))
        });
        if max || min || full {
            return;
        }
        if let Some(r) = inner {
            self.win.size = Some([r.width(), r.height()]);
        }
        if let Some(r) = outer {
            self.win.pos = Some([r.min.x, r.min.y]);
        }
    }
}

/// Première lettre (en majuscule) d'un nom, pour les boutons d'accès rapide.
fn letter_of(name: &str) -> String {
    name.chars().find(|c| c.is_alphanumeric()).or_else(|| name.chars().next()).map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "?".to_string())
}

/// Bouton carré affichant une lettre.
fn letter_button(ui: &mut Ui, letter: &str, tip: &str, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
    let v = ui.visuals();
    let down = resp.is_pointer_button_down_on();
    let fill = if selected {
        v.selection.bg_fill
    } else if down {
        v.widgets.active.bg_fill
    } else if resp.hovered() {
        v.widgets.hovered.bg_fill
    } else {
        v.widgets.inactive.weak_bg_fill
    };
    let fg = if selected { v.selection.stroke.color } else { v.widgets.inactive.fg_stroke.color };
    ui.painter().rect_filled(rect, 4.0, fill);
    ui.painter().rect_stroke(rect, 4.0, Stroke::new(1.0, v.widgets.noninteractive.bg_stroke.color), egui::StrokeKind::Inside);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, letter, FontId::proportional(15.0), fg);
    resp.on_hover_text(tip)
}

/// Couleurs proposées pour le signal audio.
const WAVE_COLORS: [[u8; 3]; 10] = [[90, 190, 120], [90, 150, 230], [80, 200, 210], [240, 160, 60], [230, 90, 90], [230, 110, 180], [160, 120, 230], [230, 210, 80], [230, 230, 230], [150, 150, 150]];

fn fmt_time(s: f64) -> String {
    let t = s.max(0.0) as u64;
    format!("{}:{:02}", t / 60, t % 60)
}

/// Enveloppe (min, max) de la forme d'onde, canaux moyennés.
fn peaks(p: &Pcm, n: usize) -> Vec<(f32, f32)> {
    let ch = p.channels.max(1) as usize;
    let frames = p.frames();
    if frames == 0 {
        return Vec::new();
    }
    (0..n)
        .map(|b| {
            let (a, z) = (b * frames / n, ((b + 1) * frames / n).max(b * frames / n + 1).min(frames));
            let (mut lo, mut hi) = (0f32, 0f32);
            let step = ((z - a) / 64).max(1);
            let mut f = a;
            while f < z {
                let v = (0..ch).map(|c| p.samples[f * ch + c] as f32).sum::<f32>() / ch as f32 / 32768.0;
                lo = lo.min(v);
                hi = hi.max(v);
                f += step;
            }
            (lo, hi)
        })
        .collect()
}

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.settings.conv = self.conv.s.clone();
        eframe::set_value(storage, SETTINGS_KEY, &self.settings);
        self.win.save();
        // eframe relit toujours cette clé au démarrage : on la neutralise, la géométrie est gérée par WinPrefs.
        storage.set_string("window", String::new());
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_load(&ctx);
        self.track_window(&ctx);
        self.handle_input(&ctx);
        self.tick(&ctx);
        self.poll_extract(&ctx);

        let title = match &self.wanted {
            Some(p) => format!("{} - FalloutAVBrowser", p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()),
            None => "FalloutAVBrowser".to_string(),
        };
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("toolbar").show(ui, |ui| self.toolbar(ui));
        if self.settings.show_tree {
            egui::Panel::left("tree").resizable(true).default_size(270.0).size_range(120.0..=700.0).show(ui, |ui| self.tree_panel(ui));
        }
        if self.settings.show_props {
            egui::Panel::right("props").resizable(true).default_size(320.0).size_range(160.0..=800.0).show(ui, |ui| self.props_panel(ui));
        }
        egui::CentralPanel::no_frame().show(ui, |ui| {
            egui::Panel::bottom("transport").show(ui, |ui| self.transport(ui));
            self.canvas(ui);
        });
        self.about_window(&ctx);
        self.expand_confirm_window(&ctx);
        self.folder_window(&ctx);
        self.rename_window(&ctx);
        self.convert_window(&ctx);
        self.toast_overlay(&ctx);
        if self.loading {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }
}
