//! Barre de menus : un menu = une méthode.

use super::*;
use crate::i18n::Lang;
use crate::media::ChannelMode;
use crate::tree::SortKey;

/// Couleurs proposées pour le signal audio.
const WAVE_COLORS: [[u8; 3]; 10] = [[90, 190, 120], [90, 150, 230], [80, 200, 210], [240, 160, 60], [230, 90, 90], [230, 110, 180], [160, 120, 230], [230, 210, 80], [230, 230, 230], [150, 150, 150]];

/// Entrée de menu avec son raccourci affiché à droite ; vrai si elle a été choisie (le menu se referme alors).
fn item(ui: &mut Ui, label: String, shortcut: &str, enabled: bool) -> bool {
    let r = ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(shortcut));
    if r.clicked() {
        ui.close();
    }
    r.clicked()
}

impl App {
    pub(crate) fn menu_bar(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(tr("m.file"), |ui| self.file_menu(ui, &ctx));
            ui.menu_button(tr("m.view"), |ui| self.view_menu(ui, &ctx));
            ui.menu_button(tr("m.navigation"), |ui| self.navigation_menu(ui, &ctx));
            ui.menu_button(tr("m.language"), |ui| self.language_menu(ui));
            ui.menu_button(tr("m.help"), |ui| self.help_menu(ui));
        });
    }

    fn file_menu(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let has_file = self.loaded.is_some();
        if item(ui, tr("m.open_folder"), "Ctrl+N", true) {
            self.pick_folder(ctx);
        }
        if item(ui, tr("m.open_file"), "Ctrl+O", true) {
            self.pick_file(ctx);
        }
        if item(ui, tr("m.open_dat"), "Ctrl+D", true) {
            self.pick_dat(ctx);
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
    }

    fn view_menu(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        ui.checkbox(&mut self.settings.show_tree, tr("t.tree"));
        ui.checkbox(&mut self.settings.show_props, tr("properties"));
        ui.separator();
        if item(ui, tr("m.fullscreen"), "F11", true) {
            self.toggle_fullscreen(ctx);
        }
        ui.separator();
        ui.menu_button(tr("av.wave_color"), |ui| self.wave_color_picker(ui));
        ui.separator();
        ui.checkbox(&mut self.win.remember_size, tr("m.remember_size"));
        ui.checkbox(&mut self.win.remember_pos, tr("m.remember_pos"));
    }

    /// Palette de couleurs du signal audio (la couleur choisie est entourée).
    fn wave_color_picker(&mut self, ui: &mut Ui) {
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
    }

    fn navigation_menu(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        if item(ui, tr("m.current_folder"), "Ctrl+L", true) {
            self.open_folder_dialog();
        }
        ui.separator();
        self.sort_options(ui);
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
        // Les canaux d'un ACM se décident au chargement : on recharge le fichier courant.
        if changed && let Some(p) = self.wanted.clone() {
            self.show_file(ctx, &p, self.playing);
        }
    }

    /// Ordre des fichiers du dossier (le précédent / suivant et l'arborescence le suivent).
    fn sort_options(&mut self, ui: &mut Ui) {
        ui.label(tr("m.sort"));
        let mut changed = false;
        for (key, label) in [(SortKey::Name, "m.sort_name"), (SortKey::Size, "m.sort_size"), (SortKey::Modified, "m.sort_date"), (SortKey::Type, "m.sort_type")] {
            changed |= ui.radio_value(&mut self.settings.sort, key, tr(label)).changed();
        }
        changed |= ui.checkbox(&mut self.settings.sort_desc, tr("m.sort_desc")).changed();
        if changed {
            self.apply_view_options();
        }
    }

    fn language_menu(&mut self, ui: &mut Ui) {
        for l in Lang::ALL {
            if ui.radio(i18n::lang() == l, l.native_name()).clicked() {
                self.set_language(l);
                ui.close();
            }
        }
    }

    fn help_menu(&mut self, ui: &mut Ui) {
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
    }
}
