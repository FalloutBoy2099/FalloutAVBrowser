//! Éléments communs des fenêtres modales : titre avec bouton « fermer » à droite, et boutons d'action larges.

use crate::i18n::tr;
use crate::icons::{Icon, icon_button};
use eframe::egui::{self, Align, Layout, Response, RichText, Ui, vec2};

/// Largeur et hauteur minimales d'un bouton d'action (OK, Annuler, Convertir…) : il n'est pas limité à la largeur de son texte.
const BUTTON_MIN: (f32, f32) = (112.0, 30.0);

/// Réglages communs à toutes les fenêtres modales, pour que champs, listes, cases et boutons aient la même hauteur et les mêmes espaces.
pub fn setup(ui: &mut Ui) {
    let spacing = ui.spacing_mut();
    spacing.item_spacing = vec2(10.0, 10.0);
    spacing.interact_size.y = 30.0;
    spacing.button_padding = vec2(14.0, 6.0);
}

/// Titre de la fenêtre avec le bouton « fermer » (×) à droite ; renvoie vrai si on a cliqué dessus.
pub fn title(ui: &mut Ui, text: &str) -> bool {
    let mut closed = false;
    ui.horizontal(|ui| {
        ui.heading(text);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            closed = icon_button(ui, Icon::Close, &tr("c.close"), false, true).clicked();
        });
    });
    closed
}

/// Bouton d'action large et haut, pour le bas d'une fenêtre.
pub fn button(ui: &mut Ui, text: impl Into<String>) -> Response {
    ui.add(egui::Button::new(RichText::new(text.into()).size(14.0)).min_size(vec2(BUTTON_MIN.0, BUTTON_MIN.1)))
}

/// Comme `button`, grisé quand `enabled` est faux.
pub fn button_enabled(ui: &mut Ui, enabled: bool, text: impl Into<String>) -> Response {
    ui.add_enabled(enabled, egui::Button::new(RichText::new(text.into()).size(14.0)).min_size(vec2(BUTTON_MIN.0, BUTTON_MIN.1)))
}

/// Cadre des fenêtres modales : marge intérieure généreuse et coins arrondis.
pub fn frame(ctx: &egui::Context) -> egui::Frame {
    egui::Frame::popup(&ctx.global_style()).inner_margin(egui::Margin::same(16)).corner_radius(egui::CornerRadius::same(10))
}

/// Cadre des fenêtres non modales (À propos, aide) : même marge intérieure.
pub fn window_frame(ctx: &egui::Context) -> egui::Frame {
    egui::Frame::window(&ctx.global_style()).inner_margin(egui::Margin::same(16)).corner_radius(egui::CornerRadius::same(10))
}
