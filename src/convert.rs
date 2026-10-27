//! Conversion du fichier affiché : audio (WAV 8 / 16 bits, flottant 32 bits, PCM brut) avec fréquence, canaux, normalisation et
//! découpe ; image courante ou séquence d'images (PNG) d'une vidéo ; copie du fichier d'origine (extraction d'une archive).
//!
//! [`ConvertUi`] porte l'état et dessine la fenêtre ; [`run`] fait le travail (dans un thread) et rend compte de sa progression.

use crate::audio::Pcm;
use crate::dialog;
use crate::ffmpeg;
use crate::i18n::{self, tr};
use crate::video::{Video, VideoDecoder};
use anyhow::{Result, anyhow, bail};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Target {
    Wav16,
    Wav8,
    WavF32,
    Raw16,
    /// MP3 / OGG / FLAC et vidéo : encodés par ffmpeg.
    Mp3,
    Ogg,
    Flac,
    Video,
    FramePng,
    Sequence,
    Original,
}

impl Target {
    pub fn label_key(self) -> &'static str {
        match self {
            Target::Wav16 => "c.t_wav16",
            Target::Wav8 => "c.t_wav8",
            Target::WavF32 => "c.t_wavf32",
            Target::Raw16 => "c.t_raw16",
            Target::Mp3 => "@MP3",
            Target::Ogg => "@OGG Vorbis",
            Target::Flac => "@FLAC",
            Target::Video => "c.t_video",
            Target::FramePng => "c.t_frame",
            Target::Sequence => "c.t_sequence",
            Target::Original => "c.t_original",
        }
    }

    pub fn is_audio(self) -> bool {
        matches!(self, Target::Wav16 | Target::Wav8 | Target::WavF32 | Target::Raw16 | Target::Mp3 | Target::Ogg | Target::Flac)
    }

    pub fn needs_ffmpeg(self) -> bool {
        matches!(self, Target::Mp3 | Target::Ogg | Target::Flac | Target::Video)
    }

    fn ext(self, s: &ConvSettings, original: &str) -> String {
        match self {
            Target::Mp3 => "mp3".to_string(),
            Target::Ogg => "ogg".to_string(),
            Target::Flac => "flac".to_string(),
            Target::Video => s.container.ext().to_string(),
            Target::Wav16 | Target::Wav8 | Target::WavF32 => "wav".to_string(),
            Target::Raw16 => "raw".to_string(),
            Target::FramePng | Target::Sequence => "png".to_string(),
            Target::Original => original.to_string(),
        }
    }
}

/// Conteneur de la vidéo convertie.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Container {
    Mp4,
    Mkv,
    Webm,
    Avi,
    Mov,
}

impl Container {
    pub const ALL: [Container; 5] = [Container::Mp4, Container::Mkv, Container::Webm, Container::Avi, Container::Mov];

    pub fn ext(self) -> &'static str {
        match self {
            Container::Mp4 => "mp4",
            Container::Mkv => "mkv",
            Container::Webm => "webm",
            Container::Avi => "avi",
            Container::Mov => "mov",
        }
    }

    pub fn label(self) -> String {
        self.ext().to_uppercase()
    }

    /// Codecs vidéo que ce conteneur accepte.
    pub fn allows(self, c: VCodec) -> bool {
        match self {
            Container::Mp4 | Container::Mkv => true,
            Container::Webm => c == VCodec::Av1,
            Container::Avi | Container::Mov => c != VCodec::Av1,
        }
    }

    /// Codecs audio possibles dans ce conteneur, par ordre de préférence.
    fn audio_codecs(self) -> &'static [&'static str] {
        match self {
            Container::Webm => &["libopus", "libvorbis"],
            Container::Avi => &["libmp3lame", "ac3"],
            _ => &["aac"],
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum VCodec {
    X264,
    X265,
    Av1,
}

impl VCodec {
    pub const ALL: [VCodec; 3] = [VCodec::X264, VCodec::X265, VCodec::Av1];

    pub fn label(self) -> &'static str {
        match self {
            VCodec::X264 => "x264",
            VCodec::X265 => "x265",
            VCodec::Av1 => "AV1",
        }
    }

    /// Encodeurs ffmpeg possibles, par ordre de préférence.
    fn encoders(self) -> &'static [&'static str] {
        match self {
            VCodec::X264 => &["libx264"],
            VCodec::X265 => &["libx265"],
            VCodec::Av1 => &["libsvtav1", "libaom-av1"],
        }
    }

    /// Valeur de `-crf` pour la qualité demandée.
    fn crf(self, q: Quality) -> u32 {
        let table = match self {
            VCodec::X264 => [18, 23, 28],
            VCodec::X265 => [22, 28, 32],
            VCodec::Av1 => [24, 32, 40],
        };
        table[q as usize]
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Quality {
    High,
    Medium,
    Low,
}

impl Quality {
    const ALL: [Quality; 3] = [Quality::High, Quality::Medium, Quality::Low];

    fn label_key(self) -> &'static str {
        match self {
            Quality::High => "c.q_high",
            Quality::Medium => "c.q_medium",
            Quality::Low => "c.q_low",
        }
    }
}

/// Réglages mémorisés d'une session à l'autre.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ConvSettings {
    pub target: Target,
    /// 0 : fréquence d'origine.
    pub rate: u32,
    /// 0 : canaux d'origine, sinon 1 (mono) ou 2 (stéréo).
    pub channels: u8,
    pub normalize: bool,
    pub out_dir: Option<PathBuf>,
    pub last_out_dir: Option<PathBuf>,
    /// ffmpeg choisi à la main (quand il n'est ni à côté de l'exécutable ni dans le PATH).
    pub ffmpeg: Option<PathBuf>,
    /// Débit du MP3 / OGG, en kb/s.
    pub kbps: u32,
    pub container: Container,
    pub codec: VCodec,
    pub quality: Quality,
}

impl Default for ConvSettings {
    fn default() -> Self {
        ConvSettings { target: Target::Wav16, rate: 0, channels: 0, normalize: false, out_dir: None, last_out_dir: None, ffmpeg: None, kbps: 192, container: Container::Mp4, codec: VCodec::X264, quality: Quality::Medium }
    }
}

/// Ce qui est converti : le fichier affiché.
#[derive(Clone)]
pub struct Source {
    pub path: PathBuf,
    pub pcm: Option<Pcm>,
    pub video: Option<Arc<Video>>,
    /// Image affichée de la vidéo.
    pub frame: usize,
}

impl Source {
    fn stem(&self) -> String {
        self.path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "converti".into())
    }

    fn ext(&self) -> String {
        self.path.extension().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
    }

    /// Dossier du fichier (pour une archive : le dossier de l'archive).
    fn dir(&self) -> Option<PathBuf> {
        crate::dat::real_parent(&self.path)
    }

    /// Cibles proposées pour ce fichier.
    pub fn targets(&self) -> Vec<Target> {
        let mut v = Vec::new();
        if self.pcm.is_some() {
            v.extend([Target::Wav16, Target::Wav8, Target::WavF32, Target::Raw16, Target::Mp3, Target::Ogg, Target::Flac]);
        }
        if self.video.is_some() {
            v.extend([Target::Video, Target::FramePng, Target::Sequence]);
        }
        v.push(Target::Original);
        v
    }
}

pub struct ConvertUi {
    pub open: bool,
    pub s: ConvSettings,
    pub name: String,
    pub start: f32,
    pub end: f32,
    pub frame_first: usize,
    pub frame_last: usize,
    pub frame_step: usize,
    pub error: Option<String>,
    pub running: bool,
    /// ffmpeg trouvé (absent : les conversions qui en ont besoin sont refusées) et ses encodeurs, lus à la demande.
    pub ffmpeg: Option<PathBuf>,
    encoders: Option<HashSet<String>>,
}

pub enum Action {
    None,
    Close,
    Start,
}

/// Tâche prête à être exécutée dans un thread.
pub struct Job {
    pub source: Source,
    pub s: ConvSettings,
    pub out_dir: PathBuf,
    pub name: String,
    pub start: f32,
    pub end: f32,
    pub frame_first: usize,
    pub frame_last: usize,
    pub frame_step: usize,
}

impl ConvertUi {
    pub fn new(s: ConvSettings) -> ConvertUi {
        let ffmpeg = ffmpeg::find(s.ffmpeg.as_deref());
        ConvertUi { open: false, s, name: String::new(), start: 0.0, end: 0.0, frame_first: 0, frame_last: 0, frame_step: 1, error: None, running: false, ffmpeg, encoders: None }
    }

    /// Fait choisir l'exécutable ffmpeg.
    pub fn pick_ffmpeg(&mut self) {
        let mut d = rfd::FileDialog::new();
        if cfg!(windows) {
            d = d.add_filter("ffmpeg", &["exe"]);
        }
        if let Some(p) = d.pick_file() {
            self.s.ffmpeg = Some(p);
            self.ffmpeg = ffmpeg::find(self.s.ffmpeg.as_deref());
            self.encoders = None;
        }
    }

    /// Cet ffmpeg possède-t-il l'un de ces encodeurs ?
    fn has_encoder(&mut self, names: &[&str]) -> bool {
        let Some(exe) = self.ffmpeg.clone() else { return false };
        let enc = self.encoders.get_or_insert_with(|| ffmpeg::encoders(&exe));
        names.iter().any(|n| enc.contains(*n))
    }

    pub fn open_for(&mut self, src: &Source) {
        self.open = true;
        self.error = None;
        self.name = src.stem();
        self.start = 0.0;
        self.end = 0.0;
        self.frame_first = 0;
        self.frame_last = 0;
        self.frame_step = 1;
        self.ffmpeg = ffmpeg::find(self.s.ffmpeg.as_deref());
        self.encoders = None;
        if !src.targets().contains(&self.s.target) {
            self.s.target = if src.video.is_some() { Target::Video } else { src.targets()[0] };
        }
    }

    pub fn out_dir(&self, src: &Source) -> Option<PathBuf> {
        self.s.out_dir.clone().filter(|p| p.is_dir()).or_else(|| src.dir())
    }

    pub fn job(&self, src: &Source) -> Option<Job> {
        Some(Job {
            source: src.clone(),
            s: self.s.clone(),
            out_dir: self.out_dir(src)?,
            name: self.name.trim().to_string(),
            start: self.start,
            end: self.end,
            frame_first: self.frame_first,
            frame_last: self.frame_last,
            frame_step: self.frame_step.max(1),
        })
    }

    /// Taille ou nombre de fichiers estimés de la sortie.
    fn estimate(&self, src: &Source) -> String {
        match self.s.target {
            t if t.is_audio() => {
                let Some(p) = &src.pcm else { return String::new() };
                let (_, ch, rate, frames) = audio_plan(p, &self.s, self.start, self.end);
                let secs = frames as f64 / rate.max(1) as f64;
                let bytes = match t {
                    Target::Mp3 | Target::Ogg => (secs * self.s.kbps as f64 * 125.0) as usize,
                    Target::Flac => frames * ch as usize,
                    Target::Wav8 => frames * ch as usize + 44,
                    Target::WavF32 => frames * ch as usize * 4 + 44,
                    Target::Raw16 => frames * ch as usize * 2,
                    _ => frames * ch as usize * 2 + 44,
                };
                let approx = if matches!(t, Target::Mp3 | Target::Ogg | Target::Flac) { "≈ " } else { "" };
                format!("{} Hz · {} · {:.1} s · {}", rate, if ch == 1 { tr("av.mono") } else { tr("av.stereo") }, secs, format!("{approx}{}", human_size(bytes)))
            }
            Target::FramePng => i18n::trf("c.est_files", &[("n", "1".to_string())]),
            Target::Sequence => {
                let n = src.video.as_ref().map_or(0, |v| v.frame_count());
                let (a, b) = self.frame_range(n);
                i18n::trf("c.est_files", &[("n", (if b >= a { (b - a) / self.frame_step.max(1) + 1 } else { 0 }).to_string())])
            }
            _ => String::new(),
        }
    }

    fn frame_range(&self, n: usize) -> (usize, usize) {
        let last = if self.frame_last == 0 { n.saturating_sub(1) } else { (self.frame_last - 1).min(n.saturating_sub(1)) };
        (self.frame_first.saturating_sub(1).min(last), last)
    }

    /// Dessine la fenêtre ; `progress` : (images ou étapes faites, total) pendant une conversion.
    pub fn ui(&mut self, ctx: &egui::Context, src: &Source, progress: Option<(usize, usize)>) -> Action {
        if !self.open {
            return Action::None;
        }
        let (mut start, mut close) = (false, false);
        let modal = egui::Modal::new(egui::Id::new("convert_modal")).frame(dialog::frame(ctx)).show(ctx, |ui| {
            ui.set_width(520.0);
            dialog::setup(ui);
            if dialog::title(ui, &tr("av.export")) && !self.running {
                close = true;
            }
            ui.label(egui::RichText::new(src.path.display().to_string()).weak());
            ui.add_space(6.0);
            ui.add_enabled_ui(!self.running, |ui| {
                // Fichier vidéo : on choisit d'abord entre convertir la vidéo ou son audio.
                let both = src.video.is_some() && src.pcm.is_some();
                let audio_tab = self.s.target.is_audio();
                if both {
                    ui.horizontal(|ui| {
                        if ui.selectable_label(!audio_tab, tr("c.kind_video")).clicked() && audio_tab {
                            self.s.target = Target::Video;
                        }
                        if ui.selectable_label(audio_tab, tr("c.kind_audio")).clicked() && !audio_tab {
                            self.s.target = Target::Wav16;
                        }
                    });
                    ui.add_space(4.0);
                }
                if self.s.target.needs_ffmpeg() && self.ffmpeg.is_none() {
                    ui.horizontal(|ui| {
                        ui.colored_label(egui::Color32::from_rgb(230, 160, 60), tr("c.ffmpeg_missing"));
                        if ui.button(tr("c.ffmpeg_choose")).clicked() {
                            self.pick_ffmpeg();
                        }
                    });
                    ui.add_space(4.0);
                }
                egui::Grid::new("conv_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    ui.label(tr("c.format"));
                    egui::ComboBox::from_id_salt("conv_target").selected_text(tr(self.s.target.label_key())).width(300.0).show_ui(ui, |ui| {
                        for t in src.targets().into_iter().filter(|t| !both || t.is_audio() == audio_tab) {
                            ui.selectable_value(&mut self.s.target, t, tr(t.label_key()));
                        }
                    });
                    ui.end_row();
                    if self.s.target == Target::Video {
                        ui.label(tr("c.container"));
                        egui::ComboBox::from_id_salt("conv_container").selected_text(self.s.container.label()).show_ui(ui, |ui| {
                            for c in Container::ALL {
                                ui.selectable_value(&mut self.s.container, c, c.label());
                            }
                        });
                        ui.end_row();
                        // Codecs proposés : acceptés par le conteneur et présents dans cet ffmpeg.
                        let avail: Vec<(VCodec, bool)> = VCodec::ALL.iter().map(|&c| (c, self.s.container.allows(c) && self.has_encoder(c.encoders()))).collect();
                        if !self.s.container.allows(self.s.codec)
                            && let Some(c) = VCodec::ALL.iter().copied().find(|&c| self.s.container.allows(c))
                        {
                            self.s.codec = c;
                        }
                        ui.label(tr("c.codec"));
                        egui::ComboBox::from_id_salt("conv_codec").selected_text(self.s.codec.label()).show_ui(ui, |ui| {
                            for (c, ok) in avail {
                                if ui.add_enabled(ok, egui::Button::selectable(self.s.codec == c, c.label())).clicked() {
                                    self.s.codec = c;
                                }
                            }
                        });
                        ui.end_row();
                        ui.label(tr("c.quality"));
                        egui::ComboBox::from_id_salt("conv_quality").selected_text(tr(self.s.quality.label_key())).show_ui(ui, |ui| {
                            for q in Quality::ALL {
                                ui.selectable_value(&mut self.s.quality, q, tr(q.label_key()));
                            }
                        });
                        ui.end_row();
                    }
                    if matches!(self.s.target, Target::Mp3 | Target::Ogg) {
                        ui.label(tr("c.bitrate"));
                        egui::ComboBox::from_id_salt("conv_kbps").selected_text(format!("{} kb/s", self.s.kbps)).show_ui(ui, |ui| {
                            for k in [64u32, 96, 128, 160, 192, 256, 320] {
                                ui.selectable_value(&mut self.s.kbps, k, format!("{k} kb/s"));
                            }
                        });
                        ui.end_row();
                    }
                    if self.s.target.is_audio() {
                        let (rate_src, ch_src) = src.pcm.as_ref().map_or((0, 0), |p| (p.rate, p.channels));
                        ui.label(tr("av.rate"));
                        egui::ComboBox::from_id_salt("conv_rate")
                            .selected_text(if self.s.rate == 0 { format!("{} ({rate_src} Hz)", tr("c.source")) } else { format!("{} Hz", self.s.rate) })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.s.rate, 0, format!("{} ({rate_src} Hz)", tr("c.source")));
                                for r in [8000u32, 11025, 16000, 22050, 32000, 44100, 48000] {
                                    ui.selectable_value(&mut self.s.rate, r, format!("{r} Hz"));
                                }
                            });
                        ui.end_row();
                        ui.label(tr("av.channels"));
                        egui::ComboBox::from_id_salt("conv_channels")
                            .selected_text(match self.s.channels {
                                1 => tr("av.mono"),
                                2 => tr("av.stereo"),
                                _ => format!("{} ({ch_src})", tr("c.source")),
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.s.channels, 0, format!("{} ({ch_src})", tr("c.source")));
                                ui.selectable_value(&mut self.s.channels, 1, tr("av.mono"));
                                ui.selectable_value(&mut self.s.channels, 2, tr("av.stereo"));
                            });
                        ui.end_row();
                        ui.label(tr("c.range"));
                        ui.horizontal(|ui| {
                            ui.label(tr("c.from"));
                            ui.add(egui::DragValue::new(&mut self.start).range(0.0..=36000.0).speed(0.1).fixed_decimals(1).suffix(" s"));
                            ui.label(tr("c.to"));
                            ui.add(egui::DragValue::new(&mut self.end).range(0.0..=36000.0).speed(0.1).fixed_decimals(1).suffix(" s"));
                            ui.label(egui::RichText::new(tr("c.end_hint")).weak());
                        });
                        ui.end_row();
                        ui.label("");
                        ui.checkbox(&mut self.s.normalize, tr("c.normalize"));
                        ui.end_row();
                    }
                    if self.s.target == Target::Sequence {
                        let n = src.video.as_ref().map_or(0, |v| v.frame_count());
                        ui.label(tr("c.frames"));
                        ui.horizontal(|ui| {
                            ui.label(tr("c.from"));
                            ui.add(egui::DragValue::new(&mut self.frame_first).range(0..=n));
                            ui.label(tr("c.to"));
                            ui.add(egui::DragValue::new(&mut self.frame_last).range(0..=n));
                            ui.label(tr("c.every"));
                            ui.add(egui::DragValue::new(&mut self.frame_step).range(1..=1000));
                            ui.label(egui::RichText::new(format!("/ {n}")).weak());
                        });
                        ui.end_row();
                    }
                    let est = self.estimate(src);
                    if !est.is_empty() {
                        ui.label(tr("c.estimate"));
                        ui.label(est);
                        ui.end_row();
                    }
                    ui.label(tr("c.name"));
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(280.0));
                        let ext = self.s.target.ext(&self.s, &src.ext());
                        ui.label(if self.s.target == Target::Sequence { "_00001.png".to_string() } else { format!(".{ext}") });
                    });
                    ui.end_row();
                    ui.label(tr("c.out_dir"));
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            if ui.button(tr("c.choose")).clicked() {
                                let mut d = rfd::FileDialog::new();
                                if let Some(cur) = self.out_dir(src) {
                                    d = d.set_directory(cur);
                                }
                                if let Some(p) = d.pick_folder() {
                                    self.s.out_dir = Some(p);
                                }
                            }
                            if ui.button(tr("c.out_source")).clicked() {
                                self.s.out_dir = src.dir();
                            }
                            let last = self.s.last_out_dir.clone();
                            if ui.add_enabled(last.is_some(), egui::Button::new(tr("c.out_last"))).clicked() {
                                self.s.out_dir = last;
                            }
                        });
                        if let Some(p) = self.out_dir(src) {
                            ui.label(p.display().to_string());
                        }
                    });
                    ui.end_row();
                });
            });
            if let Some(e) = &self.error {
                ui.add_space(4.0);
                ui.colored_label(egui::Color32::from_rgb(230, 110, 100), e);
            }
            if let (true, Some((done, total))) = (self.running, progress) {
                ui.add(egui::ProgressBar::new(if total == 0 { 0.0 } else { done as f32 / total as f32 }).text(format!("{done} / {total}")));
            }
            ui.separator();
            ui.horizontal(|ui| {
                let can = !self.running && (!self.s.target.needs_ffmpeg() || self.ffmpeg.is_some()) && self.out_dir(src).is_some() && !self.name.trim().is_empty() && dialog_name_ok(&self.name);
                if dialog::button_enabled(ui, can, tr("c.start")).clicked() {
                    start = true;
                }
                // Pendant la conversion, « Annuler » l'interrompt.
                if dialog::button(ui, tr("c.cancel")).clicked() {
                    close = true;
                }
                if self.running {
                    ui.spinner();
                }
            });
        });
        if modal.should_close() && !self.running {
            close = true;
        }
        if close {
            return Action::Close;
        }
        if start { Action::Start } else { Action::None }
    }
}

fn dialog_name_ok(name: &str) -> bool {
    crate::fileops::validate_name(name).is_ok()
}

fn human_size(b: usize) -> String {
    if b >= 1 << 20 { format!("{:.1} Mo", b as f64 / (1 << 20) as f64) } else { format!("{} Ko", b.div_ceil(1024)) }
}

/// Paramètres effectifs d'une conversion audio : (échantillons de départ / fin ignorés, canaux, fréquence, nombre de trames).
fn audio_plan(p: &Pcm, s: &ConvSettings, start: f32, end: f32) -> ((usize, usize), u16, u32, usize) {
    let total = p.frames();
    let a = ((start.max(0.0) as f64 * p.rate as f64) as usize).min(total);
    let b = if end > 0.0 { ((end as f64 * p.rate as f64) as usize).clamp(a, total) } else { total };
    let ch = if s.channels == 0 { p.channels } else { s.channels as u16 };
    let rate = if s.rate == 0 { p.rate } else { s.rate };
    let n_out = ((b - a) as f64 * rate as f64 / p.rate.max(1) as f64).round() as usize;
    ((a, b), ch, rate, n_out)
}

/// Audio converti : (échantillons entrelacés, canaux, fréquence).
fn process_audio(p: &Pcm, s: &ConvSettings, start: f32, end: f32) -> (Vec<i16>, u16, u32) {
    let ((a, b), ch_out, rate_out, _) = audio_plan(p, s, start, end);
    let ch_in = p.channels.max(1) as usize;
    // Canaux : moyenne vers le mono, duplication vers la stéréo, sinon copie des deux premiers.
    let mut frames: Vec<[f32; 2]> = Vec::with_capacity(b - a);
    for f in a..b {
        let l = p.samples[f * ch_in] as f32;
        let r = if ch_in > 1 { p.samples[f * ch_in + 1] as f32 } else { l };
        frames.push(if ch_out == 1 { let m = (l + r) * 0.5; [m, m] } else { [l, r] });
    }
    // Rééchantillonnage linéaire.
    if rate_out != p.rate && !frames.is_empty() {
        let n = (frames.len() as f64 * rate_out as f64 / p.rate.max(1) as f64).round().max(1.0) as usize;
        let step = p.rate.max(1) as f64 / rate_out.max(1) as f64;
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let pos = i as f64 * step;
            let j = (pos as usize).min(frames.len() - 1);
            let k = (j + 1).min(frames.len() - 1);
            let t = (pos - j as f64) as f32;
            out.push([frames[j][0] + (frames[k][0] - frames[j][0]) * t, frames[j][1] + (frames[k][1] - frames[j][1]) * t]);
        }
        frames = out;
    }
    // Normalisation : crête à -1 dB.
    if s.normalize {
        let peak = frames.iter().flat_map(|f| f.iter()).fold(0f32, |m, v| m.max(v.abs()));
        if peak > 0.0 {
            let g = 32767.0 * 0.891 / peak;
            for f in &mut frames {
                f[0] *= g;
                f[1] *= g;
            }
        }
    }
    let mut out = Vec::with_capacity(frames.len() * ch_out as usize);
    for f in &frames {
        out.push(f[0].round().clamp(-32768.0, 32767.0) as i16);
        if ch_out > 1 {
            out.push(f[1].round().clamp(-32768.0, 32767.0) as i16);
        }
    }
    (out, ch_out, rate_out)
}

/// Exécute la conversion. `progress` : (faits, total) ; `cancel` : interruption demandée. Renvoie les fichiers écrits.
pub fn run(job: Job, done: &AtomicUsize, total: &AtomicUsize, cancel: &AtomicBool) -> Result<Vec<PathBuf>> {
    let ext = job.s.target.ext(&job.s, &job.source.ext());
    let mut written = Vec::new();
    match job.s.target {
        Target::Mp3 | Target::Ogg | Target::Flac => {
            written.push(encode_audio(&job, &ext, done, total)?);
        }
        Target::Video => {
            written.push(encode_video(&job, &ext, done, total, cancel)?);
        }
        t if t.is_audio() => {
            let p = job.source.pcm.as_ref().ok_or_else(|| anyhow!("pas d'audio"))?;
            total.store(1, Ordering::Relaxed);
            let (samples, ch, rate) = process_audio(p, &job.s, job.start, job.end);
            if samples.is_empty() {
                bail!("{}", tr("c.empty"));
            }
            let bytes = match t {
                Target::Wav16 => crate::wav::encode(&samples, ch, rate),
                Target::Wav8 => crate::wav::encode_u8(&samples, ch, rate),
                Target::WavF32 => crate::wav::encode_f32(&samples, ch, rate),
                _ => samples.iter().flat_map(|s| s.to_le_bytes()).collect(),
            };
            written.push(write_unique(&job.out_dir, &format!("{}.{ext}", job.name), &bytes)?);
            done.store(1, Ordering::Relaxed);
        }
        Target::Original => {
            total.store(1, Ordering::Relaxed);
            let data = crate::dat::read_any(&job.source.path)?;
            written.push(write_unique(&job.out_dir, &format!("{}.{ext}", job.name), &data)?);
            done.store(1, Ordering::Relaxed);
        }
        Target::FramePng | Target::Sequence => {
            let v = job.source.video.clone().ok_or_else(|| anyhow!("pas de vidéo"))?;
            let n = v.frame_count();
            let (first, last) = if job.s.target == Target::FramePng {
                (job.source.frame.min(n.saturating_sub(1)), job.source.frame.min(n.saturating_sub(1)))
            } else {
                let last = if job.frame_last == 0 { n.saturating_sub(1) } else { (job.frame_last - 1).min(n.saturating_sub(1)) };
                (job.frame_first.saturating_sub(1).min(last), last)
            };
            let step = if job.s.target == Target::FramePng { 1 } else { job.frame_step.max(1) };
            let count = (last - first) / step + 1;
            total.store(count, Ordering::Relaxed);
            let dir = if job.s.target == Target::Sequence {
                let d = crate::fileops::free_path(&job.out_dir, &format!("{}_images", job.name));
                std::fs::create_dir_all(&d)?;
                d
            } else {
                job.out_dir.clone()
            };
            let mut dec = VideoDecoder::new(&v);
            dec.seek_to(v.key_before(first));
            let mut idx = first;
            let mut k = 0usize;
            while idx <= last {
                if cancel.load(Ordering::Relaxed) {
                    bail!("{}", tr("c.cancelled"));
                }
                while dec.position() <= idx {
                    if !dec.step(&v) {
                        break;
                    }
                }
                let png = crate::png::encode_rgba(v.width() as u32, v.height() as u32, &dec.rgba());
                let file = format!("{}_{:05}.png", job.name, idx + 1);
                written.push(write_unique(&dir, &file, &png)?);
                k += 1;
                done.store(k, Ordering::Relaxed);
                idx += step;
            }
        }
        _ => unreachable!(),
    }
    Ok(written)
}

/// Écrit `data` dans `dir` sans rien écraser (un nom pris reçoit « (2) »).
fn write_unique(dir: &Path, name: &str, data: &[u8]) -> Result<PathBuf> {
    let dest = crate::fileops::free_path(dir, name);
    std::fs::write(&dest, data).map_err(|e| anyhow!("{} : {e}", dest.display()))?;
    Ok(dest)
}

/// Fichier temporaire supprimé quand il sort de la portée.
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Fichier de sortie à supprimer si la conversion échoue ou est annulée.
struct Partial(Option<PathBuf>);

impl Partial {
    fn keep(mut self) -> PathBuf {
        self.0.take().unwrap_or_default()
    }
}

impl Drop for Partial {
    fn drop(&mut self) {
        if let Some(p) = &self.0 {
            let _ = std::fs::remove_file(p);
        }
    }
}

fn strs(a: &mut Vec<String>, xs: &[&str]) {
    a.extend(xs.iter().map(|x| x.to_string()));
}

/// Audio MP3 / OGG / FLAC : les échantillons traités sont envoyés à ffmpeg.
fn encode_audio(job: &Job, ext: &str, done: &AtomicUsize, total: &AtomicUsize) -> Result<PathBuf> {
    let exe = ffmpeg::find(job.s.ffmpeg.as_deref()).ok_or_else(ffmpeg::missing)?;
    let p = job.source.pcm.as_ref().ok_or_else(|| anyhow!("pas d'audio"))?;
    total.store(1, Ordering::Relaxed);
    let (samples, ch, rate) = process_audio(p, &job.s, job.start, job.end);
    if samples.is_empty() {
        bail!("{}", tr("c.empty"));
    }
    let enc = ffmpeg::encoders(&exe);
    let kbps = format!("{}k", job.s.kbps);
    let mut codec: Vec<String> = Vec::new();
    match job.s.target {
        Target::Mp3 => strs(&mut codec, &["-c:a", ffmpeg::pick(&enc, &["libmp3lame"])?, "-b:a", &kbps]),
        Target::Ogg => {
            let c = ffmpeg::pick(&enc, &["libvorbis", "vorbis"])?;
            // Vorbis : qualité (VBR) plutôt que débit imposé, que l'encodeur refuse aux basses fréquences.
            let q = match job.s.kbps {
                0..=64 => "0",
                65..=96 => "2",
                97..=128 => "4",
                129..=160 => "5",
                161..=192 => "6",
                193..=256 => "8",
                _ => "9",
            };
            strs(&mut codec, &["-c:a", c, "-q:a", q]);
            if c == "vorbis" {
                strs(&mut codec, &["-strict", "-2"]);
            }
        }
        _ => strs(&mut codec, &["-c:a", "flac"]),
    }
    let dest = crate::fileops::free_path(&job.out_dir, &format!("{}.{ext}", job.name));
    let mut args: Vec<String> = Vec::new();
    strs(&mut args, &["-hide_banner", "-loglevel", "error", "-y", "-f", "s16le", "-ar", &rate.to_string(), "-ac", &ch.to_string(), "-i", "pipe:0"]);
    args.extend(codec);
    args.push(dest.to_string_lossy().to_string());
    let partial = Partial(Some(dest));
    let mut proc = ffmpeg::spawn(&exe, &args)?;
    let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    if proc.write(&bytes).is_err() {
        // ffmpeg s'est arrêté : son message d'erreur explique pourquoi.
        proc.finish()?;
        bail!("ffmpeg");
    }
    proc.finish()?;
    done.store(1, Ordering::Relaxed);
    Ok(partial.keep())
}

/// Vidéo : les images décodées sont envoyées à ffmpeg (avec l'audio, écrit dans un WAV temporaire).
fn encode_video(job: &Job, ext: &str, done: &AtomicUsize, total: &AtomicUsize, cancel: &AtomicBool) -> Result<PathBuf> {
    let exe = ffmpeg::find(job.s.ffmpeg.as_deref()).ok_or_else(ffmpeg::missing)?;
    let v = job.source.video.clone().ok_or_else(|| anyhow!("pas de vidéo"))?;
    let enc = ffmpeg::encoders(&exe);
    let (cont, codec, q) = (job.s.container, job.s.codec, job.s.quality);
    if !cont.allows(codec) {
        bail!("{} / {}", cont.label(), codec.label());
    }
    let n = v.frame_count();
    total.store(n, Ordering::Relaxed);

    // Audio : tout le fichier, tel quel.
    let wav = match job.source.pcm.as_ref().filter(|p| p.frames() > 0) {
        Some(p) => {
            let (samples, ch, rate) = process_audio(p, &ConvSettings::default(), 0.0, 0.0);
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
            let path = std::env::temp_dir().join(format!("favb_{}_{nanos}.wav", std::process::id()));
            std::fs::write(&path, crate::wav::encode(&samples, ch, rate))?;
            Some(TempFile(path))
        }
        None => None,
    };

    let mut args: Vec<String> = Vec::new();
    strs(&mut args, &["-hide_banner", "-loglevel", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgba"]);
    strs(&mut args, &["-s", &format!("{}x{}", v.width(), v.height()), "-framerate", &v.fps().to_string(), "-i", "pipe:0"]);
    if let Some(w) = &wav {
        strs(&mut args, &["-i", &w.0.to_string_lossy()]);
    }
    strs(&mut args, &["-map", "0:v:0"]);
    if wav.is_some() {
        strs(&mut args, &["-map", "1:a:0"]);
    }
    // Dimensions paires et chrominance 4:2:0 : lisible partout.
    strs(&mut args, &["-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2,format=yuv420p"]);
    let crf = codec.crf(q).to_string();
    let venc = ffmpeg::pick(&enc, codec.encoders())?;
    strs(&mut args, &["-c:v", venc, "-crf", &crf]);
    match venc {
        "libx264" => strs(&mut args, &["-preset", "medium"]),
        "libx265" => strs(&mut args, &["-preset", "medium", "-x265-params", "log-level=error"]),
        "libsvtav1" => strs(&mut args, &["-preset", "8"]),
        _ => strs(&mut args, &["-b:v", "0", "-cpu-used", "6"]),
    }
    if codec == VCodec::X265 && matches!(cont, Container::Mp4 | Container::Mov) {
        strs(&mut args, &["-tag:v", "hvc1"]);
    }
    if wav.is_some() {
        let aenc = ffmpeg::pick(&enc, cont.audio_codecs())?;
        strs(&mut args, &["-c:a", aenc, "-b:a", "192k"]);
    }
    if matches!(cont, Container::Mp4 | Container::Mov) {
        strs(&mut args, &["-movflags", "+faststart"]);
    }
    let dest = crate::fileops::free_path(&job.out_dir, &format!("{}.{ext}", job.name));
    args.push(dest.to_string_lossy().to_string());
    let partial = Partial(Some(dest));

    let mut proc = ffmpeg::spawn(&exe, &args)?;
    let mut dec = VideoDecoder::new(&v);
    dec.seek_to(0);
    for k in 0..n {
        if cancel.load(Ordering::Relaxed) {
            proc.abort();
            bail!("{}", tr("c.cancelled"));
        }
        if !dec.step(&v) {
            break;
        }
        if proc.write(&dec.rgba()).is_err() {
            proc.finish()?;
            bail!("ffmpeg");
        }
        done.store(k + 1, Ordering::Relaxed);
    }
    proc.finish()?;
    Ok(partial.keep())
}
