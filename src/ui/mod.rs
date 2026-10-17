//! Interface : mise en page de la fenêtre et sous-modules (un par zone).

mod canvas;
mod dialogs;
mod input;
mod menu;
mod props_panel;
mod toolbar;
mod tree_panel;

pub(crate) use crate::app::App;
pub(crate) use canvas::peaks;
pub(crate) use crate::i18n::{self, tr};
pub(crate) use crate::icons::{Icon, icon_button};
pub(crate) use eframe::egui::{self, Align2, Color32, FontId, Id, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};

impl App {
    /// Mise en page : menus en haut, barre d'outils en bas, arborescence à gauche, propriétés à droite, lecteur au centre.
    pub(crate) fn draw(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
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
    }
}

pub(crate) fn fmt_time(s: f64) -> String {
    let t = s.max(0.0) as u64;
    format!("{}:{:02}", t / 60, t % 60)
}
