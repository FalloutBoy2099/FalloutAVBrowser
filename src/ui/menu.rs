//! Barre de menus : un menu = une méthode.

use super::*;
use crate::app::Theme;
use crate::i18n::Lang;
use crate::media::ChannelMode;
use crate::tree::SortKey;

/// Couleurs proposées (Material Design) : pour chaque teinte, 400 puis 800 (rouge, rose, violet, violet foncé, indigo, bleu, bleu clair, cyan, sarcelle, vert, vert clair, citron vert, jaune, ambre, orange, orange foncé, marron, gris, gris bleuté).
const WAVE_COLORS: [[u8; 3]; 38] = [[239, 83, 80], [198, 40, 40], [236, 64, 122], [173, 20, 87], [171, 71, 188], [106, 27, 154], [126, 87, 194], [69, 39, 160], [92, 107, 192], [40, 53, 147], [66, 165, 245], [21, 101, 192], [41, 182, 246], [2, 119, 189], [38, 198, 218], [0, 131, 143], [38, 166, 154], [0, 105, 92], [102, 187, 106], [46, 125, 50], [156, 204, 101], [85, 139, 47], [212, 225, 87], [158, 157, 36], [255, 238, 88], [249, 168, 37], [255, 202, 40], [255, 143, 0], [255, 167, 38], [239, 108, 0], [255, 112, 67], [216, 67, 21], [141, 110, 99], [78, 52, 46], [189, 189, 189], [66, 66, 66], [120, 144, 156], [55, 71, 79]];
/// Pastilles par ligne (nombre pair : une teinte 400 / 800 ne se coupe pas) et leur taille ; l'espacement est le même en largeur et en hauteur.
const SWATCH_COLS: f32 = 8.0;
const SWATCH: f32 = 24.0;
const SWATCH_GAP: f32 = 4.0;

/// Entrée de menu avec son raccourci affiché à droite ; vrai si elle a été choisie (le menu se referme alors).
fn item(ui: &mut Ui, label: String, shortcut: &str, enabled: bool) -> bool {
    let r = ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(shortcut));
    if r.clicked() {
        ui.close();
    }
    r.clicked()
}

/// Palette de couleurs (la couleur choisie est entourée).
fn color_picker(ui: &mut Ui, current: &mut [u8; 3]) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(SWATCH_GAP, SWATCH_GAP);
        ui.set_max_width(SWATCH_COLS * SWATCH + (SWATCH_COLS - 1.0) * SWATCH_GAP);
        for c in WAVE_COLORS {
            let (rect, resp) = ui.allocate_exact_size(vec2(SWATCH, SWATCH), Sense::click());
            ui.painter().rect_filled(rect, 4.0, Color32::from_rgb(c[0], c[1], c[2]));
            if *current == c {
                ui.painter().rect_stroke(rect, 4.0, Stroke::new(2.0, ui.visuals().strong_text_color()), egui::StrokeKind::Inside);
            }
            if resp.clicked() {
                *current = c;
            }
        }
    });
}

impl App {
    pub(crate) fn menu_bar(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(tr("m.file"), |ui| self.file_menu(ui, &ctx));
            ui.menu_button(tr("m.view"), |ui| self.view_menu(ui, &ctx));
            ui.menu_button(tr("m.navigation"), |ui| self.navigation_menu(ui, &ctx));
            ui.menu_button(tr("m.settings"), |ui| self.settings_menu(ui, &ctx));
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
        if item(ui, tr("m.ffmpeg"), "", true) {
            self.conv.pick_ffmpeg();
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
        ui.menu_button(tr("av.line_color"), |ui| color_picker(ui, &mut self.settings.line_color));
        ui.menu_button(tr("av.wave_color"), |ui| color_picker(ui, &mut self.settings.wave_color));
        ui.separator();
        ui.checkbox(&mut self.settings.show_transport, tr("av.show_transport"));
        ui.separator();
        ui.checkbox(&mut self.settings.canvas_menu, tr("m.canvas_menu"));
        ui.separator();
        ui.checkbox(&mut self.win.remember_size, tr("m.remember_size"));
        ui.checkbox(&mut self.win.remember_pos, tr("m.remember_pos"));
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

    /// Menu Réglages : langue et thème.
    fn settings_menu(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        ui.menu_button(tr("m.language"), |ui| self.language_menu(ui));
        ui.menu_button(tr("m.theme"), |ui| {
            let mut changed = false;
            for (t, k) in [(Theme::System, "m.theme_system"), (Theme::Dark, "m.theme_dark"), (Theme::Light, "m.theme_light")] {
                changed |= ui.radio_value(&mut self.settings.theme, t, tr(k)).changed();
            }
            if changed {
                ctx.set_theme(self.settings.theme.preference());
            }
        });
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
