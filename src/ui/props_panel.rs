//! Panneau de droite : propriétés du fichier et boutons d'action.

use super::*;
use crate::media::Loaded;
use std::path::Path;

/// Une section du panneau : titre (clé de traduction) et lignes (clé de traduction, valeur).
type Section = (String, Vec<(String, String)>);

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

fn row(key: &str, value: impl Into<String>) -> (String, String) {
    (key.to_string(), value.into())
}

impl App {
    pub(crate) fn props_panel(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        ui.heading(tr("properties"));
        ui.separator();
        let (Some(path), Some(l)) = (self.wanted.clone(), &self.loaded) else {
            ui.label(egui::RichText::new(tr("av.no_media")).weak());
            return;
        };
        let secs = self.collect_props(&path, l);
        let sha = l.sha256.clone();
        // Un fichier d'archive peut être copié mais ni déplacé ni renommé.
        let on_disk = !crate::dat::is_virtual(&path);
        let mut action = None;
        egui::Panel::bottom("props_copy").show(ui, |ui| action = self.props_bottom_bar(ui, &sha, on_disk));
        sections_list(ui, &secs);
        if let Some(action) = action {
            self.run_prop_action(ui.ctx(), action, &path, &sha, &secs);
        }
    }

    /// Bas du panneau : comparaison de SHA-256 (si une empreinte a été copiée), puis les boutons d'action.
    fn props_bottom_bar(&self, ui: &mut Ui, sha: &str, on_disk: bool) -> Option<PropAction> {
        let mut action = None;
        ui.add_space(3.0);
        if let Some((name, reference)) = &self.ref_hash {
            // Ligne de hauteur fixe, contenu centré verticalement (texte et bouton croix).
            ui.allocate_ui_with_layout(vec2(ui.available_width(), 30.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let (color, key) = if reference == sha { (Color32::from_rgb(90, 190, 120), "t.sha_same") } else { (Color32::from_rgb(230, 120, 90), "t.sha_diff") };
                ui.colored_label(color, i18n::trf(key, &[("name", name.clone())]));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icon_button(ui, Icon::Close, &tr("t.sha_stop"), false, true).clicked() {
                        action = Some(PropAction::ShaStop);
                    }
                });
            });
            ui.separator();
        }
        // Chemin ; copier, déplacer, renommer ; convertir ; copier les informations (texte, JSON, XML) et le SHA-256.
        // Pas d'espace sous la ligne d'icônes : elle en ajoute déjà un (espacement vertical), ce qui égale la marge du haut.
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
        action
    }

    fn run_prop_action(&mut self, ctx: &egui::Context, action: PropAction, path: &Path, sha: &str, secs: &[Section]) {
        // Sections traduites pour les copies texte / JSON / XML.
        let translated = || -> crate::props::PropSections { secs.iter().map(|(t, rows)| (tr(t), rows.iter().map(|(k, v)| (tr(k), v.clone())).collect())).collect() };
        match action {
            PropAction::Text => self.copy_info(ctx, crate::props::props_text(&translated())),
            PropAction::Json => self.copy_info(ctx, crate::props::props_json(&translated())),
            PropAction::Xml => self.copy_info(ctx, crate::props::props_xml(&translated())),
            PropAction::CopyPath => {
                ctx.copy_text(path.display().to_string());
                self.toast(tr("t.path_copied"));
            }
            PropAction::CopyTo => self.copy_to(path),
            PropAction::MoveTo => self.move_to(ctx, path),
            PropAction::Rename => self.open_rename(path),
            PropAction::Convert => self.open_convert(),
            PropAction::Sha => {
                ctx.copy_text(sha.to_string());
                let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                self.ref_hash = Some((name, sha.to_string()));
                self.toast(tr("t.sha_copied"));
            }
            PropAction::ShaStop => self.ref_hash = None,
        }
    }

    fn copy_info(&mut self, ctx: &egui::Context, text: String) {
        ctx.copy_text(text);
        self.toast(tr("t.copied"));
    }

    /// Sections du panneau : fichier (avec SHA-256, dates et format), avertissement éventuel, audio, vidéo.
    pub(crate) fn collect_props(&self, path: &Path, l: &Loaded) -> Vec<Section> {
        let mut secs = vec![("sec.file".to_string(), self.file_rows(path, l))];
        if let Some(w) = &l.warning {
            secs.push(("av.warning".to_string(), vec![row("av.warning", w.clone())]));
        }
        if let Some(p) = &l.pcm {
            let title = if l.video.is_some() { "av.audio_track" } else { "av.sec_audio" };
            secs.push((title.to_string(), audio_rows(p, l)));
        }
        if let Some(m) = &l.video {
            secs.push(("av.sec_video".to_string(), self.video_rows(m)));
        }
        secs
    }

    fn file_rows(&self, path: &Path, l: &Loaded) -> Vec<(String, String)> {
        let local = |t: std::time::SystemTime| chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d %H:%M:%S").to_string();
        let mut rows = vec![
            row("p.name", path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()),
            row("p.folder", path.parent().map(|p| p.display().to_string()).unwrap_or_default()),
            row("p.size", format!("{} ({} Ko)", l.size, l.size.div_ceil(1024))),
            row("p.sha256", l.sha256.clone()),
        ];
        if let Some(m) = self.modified {
            rows.push(row("p.modified", local(m)));
        }
        if let Some(c) = self.created {
            rows.push(row("p.created", local(c)));
        }
        rows.push(row("p.format", l.info.iter().find(|(k, _)| *k == "format").map(|(_, v)| v.clone()).unwrap_or_default()));
        rows
    }

    fn video_rows(&self, m: &crate::video::Video) -> Vec<(String, String)> {
        let shown = self.shown_frame.min(m.frame_count().saturating_sub(1)) + 1;
        vec![
            row("p.dimensions", format!("{} × {}", m.width(), m.height())),
            row("p.fps", format!("{:.2}", m.fps())),
            row("p.frames", m.frame_count().to_string()),
            row("p.duration", format!("{} ({:.2} s)", fmt_time(m.duration()), m.duration())),
            row("av.frame_now", format!("{} / {}", shown, m.frame_count())),
        ]
    }
}

fn audio_rows(p: &crate::audio::Pcm, l: &Loaded) -> Vec<(String, String)> {
    let mut rows = vec![
        row("p.duration", format!("{} ({:.2} s)", fmt_time(p.duration()), p.duration())),
        row("av.rate", format!("{} Hz", p.rate)),
        row("av.channels", format!("{} ({})", p.channels, if p.channels == 1 { tr("av.mono") } else { tr("av.stereo") })),
        row("av.bits", "16"),
        row("av.samples", p.frames().to_string()),
    ];
    if let Some((_, h)) = l.info.iter().find(|(k, _)| *k == "header_channels") {
        rows.push(row("av.header_channels", h.clone()));
    }
    rows
}

/// Liste repliable des sections, une grille clé / valeur chacune (valeurs sélectionnables).
fn sections_list(ui: &mut Ui, secs: &[Section]) {
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
}
