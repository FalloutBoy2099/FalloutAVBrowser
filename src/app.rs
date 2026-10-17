//! Application egui : même interface que FalloutImageBrowser (arborescence à gauche, média au centre, propriétés à droite),
//! avec un lecteur audio / vidéo à la place de la visionneuse d'images.

use crate::audio::Output;
use crate::convert::{self, ConvSettings, ConvertUi};
use crate::i18n::{self, Lang, tr};
use crate::media::{self, ChannelMode, Loaded};
use crate::video_worker::VideoWorker;
use crate::tree::{SortKey, Tree};
use eframe::egui::{self, Rect, TextureHandle, TextureOptions, Vec2};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

pub const SETTINGS_KEY: &str = "fav_settings_v1";
const MIN_ZOOM: f32 = 0.05;
/// Favoris ajustables (les trois autres boutons sont réservés : Bureau, Documents, Téléchargements).
pub(crate) const MAX_FAVORITES: usize = 5;
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

/// État de la fenêtre « Renommer » : fichier, nom en cours de saisie, première image (pour sélectionner le nom).
pub(crate) struct RenameDlg {
    pub(crate) path: PathBuf,
    pub(crate) name: String,
    pub(crate) fresh: bool,
}

/// Conversion en cours dans un thread.
pub(crate) struct ConvJob {
    pub(crate) done: Arc<AtomicUsize>,
    pub(crate) total: Arc<AtomicUsize>,
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) rx: Receiver<Result<Vec<PathBuf>, String>>,
    pub(crate) out_dir: PathBuf,
}

pub(crate) type LoadResult = (u64, PathBuf, bool, Result<Loaded, String>);

pub struct App {
    pub settings: Settings,
    pub tree: Tree,
    pub files: Vec<PathBuf>,
    pub index: Option<usize>,
    pub wanted: Option<PathBuf>,
    pub(crate) out: Output,
    pub(crate) loaded: Option<Loaded>,
    /// Date de modification du fichier affiché (absente dans une archive).
    pub(crate) modified: Option<std::time::SystemTime>,
    pub(crate) created: Option<std::time::SystemTime>,
    pub(crate) conv: ConvertUi,
    pub(crate) conv_job: Option<ConvJob>,
    pub(crate) rename_dlg: Option<RenameDlg>,
    pub(crate) peaks: Vec<(f32, f32)>,
    pub(crate) error: Option<String>,
    pub(crate) loading: bool,
    pub(crate) gen_id: u64,
    pub(crate) tx: Sender<LoadResult>,
    pub(crate) rx: Receiver<LoadResult>,
    // Vidéo
    /// Thread qui décode la vidéo (l'interface ne bloque jamais sur un saut lointain).
    pub(crate) worker: Option<VideoWorker>,
    /// Dernière image demandée à ce thread.
    pub(crate) req_frame: usize,
    pub(crate) tex: Option<TextureHandle>,
    pub(crate) shown_frame: usize,
    pub(crate) vt: f64,
    pub(crate) last_tick: Instant,
    pub(crate) playing: bool,
    pub(crate) zoom: f32,
    pub(crate) pan: Vec2,
    pub(crate) fit: bool,
    /// Échelle réellement appliquée à la vidéo à l'image précédente.
    pub(crate) cur_scale: f32,
    // Interface
    pub(crate) toast: Option<(String, Instant)>,
    /// Dernier SHA-256 copié (nom du fichier, empreinte) : sert à comparer deux fichiers.
    pub(crate) ref_hash: Option<(String, String)>,
    /// Zone de l'arborescence (la saisie au clavier n'y agit que si la souris la survole) et recherche en cours.
    pub(crate) tree_rect: Option<Rect>,
    /// Bureau, Documents, Téléchargements (boutons réservés de la barre de favoris).
    pub(crate) special: Vec<(&'static str, PathBuf)>,
    pub(crate) ta_buf: String,
    pub(crate) ta_time: Instant,
    pub(crate) show_about: bool,
    pub(crate) expand_confirm: Option<PathBuf>,
    pub(crate) folder_dlg: Option<(String, bool)>,
    pub(crate) file_filter: String,
    pub(crate) tree_w: (u64, f32),
    pub(crate) tree_scroll_to: Option<usize>,
    pub(crate) fullscreen: bool,
    pub(crate) toolbar_w: f32,
    pub(crate) last_title: String,
    pub(crate) extract_job: Option<(Receiver<Result<crate::dat::Extracted, String>>, PathBuf)>,
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
            worker: None,
            req_frame: usize::MAX,
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
    pub(crate) fn apply_fonts(&mut self, ctx: &egui::Context) {
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

    pub(crate) fn set_language(&mut self, l: Lang) {
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

    pub(crate) fn reload_dir(&mut self) {
        if let Some(dir) = self.tree.selected_dir.clone() {
            // Une archive a pu changer sur le disque : on oublie son index (et on referme son fichier).
            if let Some((arch, _)) = crate::dat::split(&dir) {
                crate::dat::forget(&arch);
            }
            self.tree.reload_dir(&dir);
            self.files = self.tree.files_of(&dir);
            self.index = self.wanted.as_ref().and_then(|w| self.files.iter().position(|f| f == w));
        }
    }

    pub(crate) fn reveal_in_tree(&mut self) {
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

    pub(crate) fn step(&mut self, ctx: &egui::Context, delta: i32, play: bool) {
        if self.files.is_empty() {
            return;
        }
        let n = self.files.len() as i32;
        let cur = self.index.map(|i| i as i32).unwrap_or(if delta > 0 { -1 } else { 0 });
        let p = self.files[(cur + delta).rem_euclid(n) as usize].clone();
        self.show_file(ctx, &p, play);
    }

    pub(crate) fn goto(&mut self, ctx: &egui::Context, idx: usize) {
        if let Some(p) = self.files.get(idx).cloned() {
            self.show_file(ctx, &p, self.playing);
        }
    }

    pub(crate) fn poll_load(&mut self, ctx: &egui::Context) {
        while let Ok((id, path, play, res)) = self.rx.try_recv() {
            if id != self.gen_id {
                continue;
            }
            self.loading = false;
            match res {
                Ok(l) => {
                    self.peaks = l.pcm.as_ref().map(|p| crate::ui::peaks(p, 1600)).unwrap_or_default();
                    self.out.set(l.pcm.clone());
                    self.worker = l.video.as_ref().map(|v| VideoWorker::new(v.clone(), ctx.clone()));
                    self.req_frame = usize::MAX;
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
                    self.worker = None;
                    self.tex = None;
                    self.error = Some(format!("{} : {e}", path.display()));
                }
            }
        }
    }

    // ------------------------------------------------------------------ lecture

    pub(crate) fn duration(&self) -> f64 {
        match &self.loaded {
            Some(l) => match (&l.video, &l.pcm) {
                (Some(v), _) => v.duration(),
                (None, Some(p)) => p.duration(),
                _ => 0.0,
            },
            None => 0.0,
        }
    }

    pub(crate) fn has_audio(&self) -> bool {
        self.loaded.as_ref().is_some_and(|l| l.pcm.is_some())
    }

    pub(crate) fn is_video(&self) -> bool {
        self.loaded.as_ref().is_some_and(|l| l.video.is_some())
    }

    pub(crate) fn play(&mut self) {
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

    pub(crate) fn pause(&mut self) {
        self.playing = false;
        self.out.pause();
    }

    /// Le moteur audio ne reboucle de lui-même que sur un fichier seul : avec « Enchaîner », la boucle se fait sur le répertoire.
    pub(crate) fn sync_loop(&self) {
        self.out.set_looping(self.settings.looping && !self.settings.auto_next);
    }

    pub(crate) fn toggle_play(&mut self) {
        if self.playing {
            self.pause();
        } else {
            self.play();
        }
    }

    pub(crate) fn stop(&mut self) {
        self.pause();
        self.seek(0.0);
    }

    pub(crate) fn seek(&mut self, t: f64) {
        self.vt = t.clamp(0.0, self.duration());
        self.out.seek(self.vt);
        self.last_tick = Instant::now();
    }

    pub(crate) fn tick(&mut self, ctx: &egui::Context) {
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
    pub(crate) fn update_video(&mut self, ctx: &egui::Context, force: bool) {
        let (Some(l), Some(worker)) = (&self.loaded, &self.worker) else { return };
        let Some(m) = &l.video else { return };
        let n = m.frame_count();
        if n == 0 {
            return;
        }
        let target = ((self.vt * m.fps()) as usize).min(n - 1);
        if target != self.req_frame || force {
            worker.request(target);
            self.req_frame = target;
        }
        // Dernière image décodée par le thread, quand elle est prête.
        if let Some(f) = worker.latest() {
            let img = egui::ColorImage::from_rgba_unmultiplied([f.width, f.height], &f.rgba);
            match &mut self.tex {
                Some(t) => t.set(img, TextureOptions::NEAREST),
                None => self.tex = Some(ctx.load_texture("mve", img, TextureOptions::NEAREST)),
            }
            self.shown_frame = f.index;
        }
    }

    pub(crate) fn zoom_by(&mut self, f: f32) {
        if self.fit {
            self.zoom = self.cur_scale;
        }
        self.fit = false;
        let old = self.zoom;
        self.zoom = (self.zoom * f).clamp(MIN_ZOOM, MAX_ZOOM);
        self.pan *= self.zoom / old;
    }

    pub(crate) fn zoom_actual(&mut self) {
        self.fit = false;
        let old = if self.fit { self.cur_scale } else { self.zoom };
        self.pan *= 1.0 / old.max(1e-6);
        self.zoom = 1.0;
    }

    // ------------------------------------------------------------------ fichier

    pub(crate) fn open_default(&mut self) {
        if let Some(p) = self.wanted.clone()
            && let Err(e) = crate::opener::open_default(&p)
        {
            self.toast(e);
        }
    }

    /// Ouvre la fenêtre « Convertir le fichier… ».
    pub(crate) fn open_convert(&mut self) {
        if let Some(src) = self.conv_source() {
            self.conv.open_for(&src);
        }
    }

    pub(crate) fn conv_source(&self) -> Option<convert::Source> {
        let (path, l) = (self.wanted.clone()?, self.loaded.as_ref()?);
        Some(convert::Source { path, pcm: l.pcm.clone(), video: l.video.clone(), frame: self.shown_frame.min(l.video.as_ref().map_or(0, |v| v.frame_count().saturating_sub(1))) })
    }

    pub(crate) fn start_conversion(&mut self, src: &convert::Source) {
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

    pub(crate) fn poll_conversion(&mut self, ctx: &egui::Context) {
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

    /// Déplace le fichier affiché vers un dossier choisi ; le fichier suivant du dossier s'affiche ensuite.
    pub(crate) fn move_to(&mut self, ctx: &egui::Context, path: &Path) {
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
                        self.worker = None;
                        self.tex = None;
                        self.peaks.clear();
                    }
                }
                self.toast(i18n::trf("t.moved_to", &[("path", new.display().to_string())]));
            }
            Err(e) => self.toast(i18n::trf("t.op_failed", &[("error", e.to_string())])),
        }
    }

    pub(crate) fn open_rename(&mut self, path: &Path) {
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        self.rename_dlg = Some(RenameDlg { path: path.to_path_buf(), name, fresh: true });
    }

    pub(crate) fn copy_to(&mut self, path: &Path) {
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

    pub(crate) fn toggle_fullscreen(&mut self, ctx: &egui::Context) {
        self.fullscreen = !self.fullscreen;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
    }

    pub(crate) fn associate_files(&mut self, types: &[crate::assoc::FileType]) {
        let exts = crate::assoc::list(types);
        match crate::assoc::associate(types) {
            Ok(()) => {
                let key = if cfg!(windows) { "assoc.done_win" } else { "assoc.done" };
                self.toast(i18n::trf(key, &[("exts", exts)]));
            }
            Err(e) => self.toast(i18n::trf("assoc.failed", &[("exts", exts), ("error", e)])),
        }
    }

    pub(crate) fn pick_folder(&mut self, ctx: &egui::Context) {
        let mut d = rfd::FileDialog::new().set_title(tr("m.open_folder"));
        if let Some(dir) = &self.settings.last_dir {
            d = d.set_directory(dir);
        }
        if let Some(p) = d.pick_folder() {
            self.open_dir(ctx, &p, true);
            self.reveal_in_tree();
        }
    }

    pub(crate) fn pick_file(&mut self, ctx: &egui::Context) {
        let mut d = rfd::FileDialog::new().set_title(tr("m.open_file")).add_filter("ACM / MP3 / WAV / MVE / BIK", &["acm", "mp3", "wav", "mve", "bik"]);
        if let Some(dir) = &self.settings.last_dir {
            d = d.set_directory(crate::dat::real_parent(dir).unwrap_or_else(|| dir.clone()));
        }
        if let Some(p) = d.pick_file() {
            self.open_path(ctx, &p, true);
        }
    }

    pub(crate) fn pick_dat(&mut self, ctx: &egui::Context) {
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

    pub(crate) fn extract_from_archive(&mut self, path: &Path) {
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

    pub(crate) fn poll_extract(&mut self, ctx: &egui::Context) {
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

    /// Retient la taille / position de la fenêtre (sauf agrandie, minimisée ou plein écran).
    pub(crate) fn track_window(&mut self, ctx: &egui::Context) {
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

        self.draw(ui);
        if self.loading {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }
}
