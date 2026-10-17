//! Panneau de gauche : accès rapides (Bureau, Documents, Téléchargements, favoris), filtre et arborescence.

use super::*;
use crate::app::MAX_FAVORITES;
use crate::tree::{Row, RowKind};
use std::path::PathBuf;

/// Hauteur d'une ligne de l'arborescence, indentation par niveau et largeur de la zone de la flèche (en points).
const INDENT: f32 = 14.0;
const ARROW_W: f32 = 14.0;

/// Commande choisie dans le menu contextuel d'une ligne.
enum TreeAction {
    ExpandAll(PathBuf),
    CollapseOthers(PathBuf),
    Refresh(PathBuf),
    Reveal(PathBuf),
    Extract(PathBuf),
    AddFavorite(PathBuf),
}

/// Clic sur une ligne : son numéro et, pour un dossier, si c'est sur sa flèche (déplier / replier).
struct RowClick {
    index: usize,
    on_arrow: bool,
}

impl App {
    pub(crate) fn tree_panel(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.tree_rect = Some(ui.max_rect());
        ui.add_space(5.0);
        self.favorites_bar(ui, &ctx);
        ui.add_space(5.0);
        self.filter_box(ui);
        ui.add_space(5.0);
        self.measure_tree(ui);
        let (click, action) = self.tree_rows(ui);
        if let Some(a) = action {
            self.apply_tree_action(a);
        }
        if let Some(c) = click {
            self.click_row(&ctx, c);
        }
    }

    /// Boutons d'accès rapide (8 au plus) : Bureau, Documents, Téléchargements (réservés) puis les favoris ;
    /// clic droit sur un favori : le retirer.
    pub(crate) fn favorites_bar(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let mut open: Option<PathBuf> = None;
        let mut remove: Option<usize> = None;
        let cur = self.tree.selected_dir.clone();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
            for (key, path) in &self.special {
                let icon = match *key {
                    "t.desktop" => Icon::Desktop,
                    "t.documents" => Icon::Documents,
                    _ => Icon::Downloads,
                };
                let tip = format!("{}\n{}", tr(key), path.display());
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

    /// Champ qui filtre les noms de fichiers du dossier courant.
    fn filter_box(&mut self, ui: &mut Ui) {
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
    }

    /// Largeur du contenu : la ligne la plus longue (indentation, icône et nom). Recalculée seulement quand les lignes changent, et
    /// seuls les 64 candidats les plus larges (estimés sans mise en page) sont mesurés : l'arborescence peut avoir des dizaines de milliers de lignes.
    fn measure_tree(&mut self, ui: &Ui) {
        if self.tree_w.0 == self.tree.rev {
            return;
        }
        let font = FontId::proportional(13.5);
        let est = |r: &Row| r.depth as f32 * INDENT + r.name.chars().map(|c| if c.is_ascii() { 1.0 } else { 2.0 }).sum::<f32>() * 7.0;
        let mut cand: Vec<&Row> = self.tree.rows.iter().collect();
        if cand.len() > 64 {
            cand.select_nth_unstable_by(63, |a, b| est(b).total_cmp(&est(a)));
            cand.truncate(64);
        }
        let widest = cand.iter().map(|r| 4.0 + r.depth as f32 * INDENT + 33.0 + ui.painter().layout_no_wrap(r.name.clone(), font.clone(), Color32::WHITE).size().x + 10.0).fold(0.0, f32::max);
        self.tree_w = (self.tree.rev, widest);
    }

    /// Dessine les lignes visibles ; renvoie le clic éventuel et la commande choisie dans un menu contextuel.
    fn tree_rows(&mut self, ui: &mut Ui) -> (Option<RowClick>, Option<TreeAction>) {
        let row_h = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
        let content_w = self.tree_w.1;
        let cur = self.wanted.clone();
        let sel_dir = self.tree.selected_dir.clone();
        let favs = self.settings.favorites.clone();
        let (mut click, mut action) = (None, None);
        let mut area = egui::ScrollArea::both().auto_shrink([false, false]);
        if let Some(r) = self.tree_scroll_to.take() {
            area = area.vertical_scroll_offset((r as f32 * row_h - ui.available_height() * 0.5).max(0.0));
        }
        area.show_rows(ui, row_h, self.tree.rows.len(), |ui, range| {
            ui.set_min_width(content_w);
            let w = ui.available_width();
            for i in range {
                let row = &self.tree.rows[i];
                let (rect, resp) = ui.allocate_exact_size(vec2(w, row_h), Sense::click());
                let is_dir = matches!(row.kind, RowKind::Dir { .. });
                let selected = !is_dir && cur.as_deref() == Some(row.path.as_path());
                let is_sel_dir = is_dir && sel_dir.as_deref() == Some(row.path.as_path());
                paint_row(ui, rect, &resp, row, selected, is_sel_dir);
                if resp.clicked() {
                    let x0 = rect.left() + 4.0 + row.depth as f32 * INDENT;
                    let on_arrow = is_dir && resp.interact_pointer_pos().is_some_and(|p| p.x < x0 + ARROW_W);
                    click = Some(RowClick { index: i, on_arrow });
                }
                resp.context_menu(|ui| {
                    if let Some(a) = row_menu(ui, row, sel_dir.as_deref(), &favs) {
                        action = Some(a);
                    }
                });
            }
        });
        (click, action)
    }

    fn apply_tree_action(&mut self, action: TreeAction) {
        match action {
            TreeAction::ExpandAll(path) => {
                // Plus de 100 dossiers : confirmation avant de tout déplier.
                if self.tree.count_dirs(&path, 100) > 100 {
                    self.expand_confirm = Some(path);
                } else {
                    self.tree.expand_all(&path);
                }
            }
            TreeAction::CollapseOthers(path) => self.tree.collapse_others(&path),
            TreeAction::Refresh(path) => {
                self.tree.reload_dir(&path);
                self.apply_view_options();
            }
            TreeAction::Reveal(path) => {
                if let Err(e) = crate::opener::reveal(&path) {
                    self.toast(e);
                }
            }
            TreeAction::Extract(path) => self.extract_from_archive(&path),
            TreeAction::AddFavorite(path) => {
                if self.settings.favorites.len() < MAX_FAVORITES && !self.settings.favorites.contains(&path) {
                    self.settings.favorites.push(path);
                }
            }
        }
    }

    fn click_row(&mut self, ctx: &egui::Context, click: RowClick) {
        let row = self.tree.rows[click.index].clone();
        match row.kind {
            RowKind::Dir { .. } if click.on_arrow => self.tree.toggle(&row.path),
            RowKind::Dir { .. } => self.open_dir(ctx, &row.path, false),
            RowKind::File => self.show_file(ctx, &row.path, true),
        }
    }
}

/// Dessine une ligne : fond, flèche et dossier (bleu pour une archive) ou page (fichier), puis le nom.
fn paint_row(ui: &Ui, rect: Rect, resp: &egui::Response, row: &Row, selected: bool, is_sel_dir: bool) {
    let vis = ui.visuals();
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 3.0, vis.selection.bg_fill);
    } else if resp.hovered() {
        painter.rect_filled(rect, 3.0, vis.widgets.hovered.bg_fill.gamma_multiply(0.6));
    }
    let x0 = rect.left() + 4.0 + row.depth as f32 * INDENT;
    let cy = rect.center().y;
    let text_col = if selected { vis.selection.stroke.color } else { vis.text_color() };
    let fx = x0 + 15.0;
    match row.kind {
        RowKind::Dir { expanded } => {
            let c = pos2(x0 + 6.0, cy);
            let pts = if expanded {
                vec![c + vec2(-4.0, -2.5), c + vec2(4.0, -2.5), c + vec2(0.0, 3.0)]
            } else {
                vec![c + vec2(-2.5, -4.0), c + vec2(-2.5, 4.0), c + vec2(3.0, 0.0)]
            };
            painter.add(egui::Shape::convex_polygon(pts, vis.weak_text_color(), Stroke::NONE));
            let folder = match (row.archive, is_sel_dir) {
                (true, true) => Color32::from_rgb(120, 170, 235),
                (true, false) => Color32::from_rgb(90, 140, 205),
                (false, true) => Color32::from_rgb(240, 190, 70),
                (false, false) => Color32::from_rgb(200, 160, 60),
            };
            painter.rect_filled(Rect::from_min_size(pos2(fx, cy - 4.0), vec2(13.0, 9.0)), 1.5, folder);
            painter.rect_filled(Rect::from_min_size(pos2(fx, cy - 6.0), vec2(6.0, 3.0)), 1.0, folder);
        }
        RowKind::File => {
            let col = if selected { text_col } else { vis.weak_text_color() };
            painter.rect_stroke(Rect::from_min_size(pos2(fx + 1.0, cy - 6.0), vec2(10.0, 12.0)), 1.5, Stroke::new(1.2, col), egui::StrokeKind::Inside);
            painter.line_segment([pos2(fx + 3.0, cy + 2.0), pos2(fx + 6.0, cy - 1.0)], Stroke::new(1.0, col));
            painter.line_segment([pos2(fx + 6.0, cy - 1.0), pos2(fx + 9.0, cy + 2.0)], Stroke::new(1.0, col));
        }
    }
    let name_col = if is_sel_dir && !selected { vis.strong_text_color() } else { text_col };
    painter.with_clip_rect(rect).text(pos2(fx + 18.0, cy), Align2::LEFT_CENTER, &row.name, FontId::proportional(13.5), name_col);
}

/// Menu contextuel d'une ligne : développer / replier, actualiser, favoris, explorateur, extraire.
fn row_menu(ui: &mut Ui, row: &Row, sel_dir: Option<&std::path::Path>, favs: &[PathBuf]) -> Option<TreeAction> {
    let is_dir = matches!(row.kind, RowKind::Dir { .. });
    let mut action = None;
    if ui.add_enabled(is_dir, egui::Button::new(tr("t.expand_all"))).clicked() {
        action = Some(TreeAction::ExpandAll(row.path.clone()));
    }
    if ui.button(tr("t.collapse_others")).clicked() {
        action = Some(TreeAction::CollapseOthers(row.path.clone()));
    }
    ui.separator();
    if ui.add(egui::Button::new(tr("t.refresh")).shortcut_text("F5")).clicked() {
        // Sur un fichier : on actualise le dossier qui le contient (le dossier courant).
        let target = if is_dir { row.path.clone() } else { sel_dir.map_or_else(|| row.path.clone(), |d| d.to_path_buf()) };
        action = Some(TreeAction::Refresh(target));
    }
    // Favoris : grisé pour un fichier, un dossier déjà favori, ou quand les 5 places sont prises.
    let can_fav = is_dir && favs.len() < MAX_FAVORITES && !favs.contains(&row.path);
    if ui.add_enabled(can_fav, egui::Button::new(tr("t.fav_add"))).clicked() {
        action = Some(TreeAction::AddFavorite(row.path.clone()));
    }
    if ui.button(tr("t.open_explorer")).clicked() {
        action = Some(TreeAction::Reveal(row.path.clone()));
    }
    ui.separator();
    if ui.add_enabled(crate::dat::is_virtual(&row.path), egui::Button::new(tr("t.extract"))).clicked() {
        action = Some(TreeAction::Extract(row.path.clone()));
    }
    if action.is_some() {
        ui.close();
    }
    action
}

/// Première lettre (en majuscule) d'un nom, pour les boutons d'accès rapide.
fn letter_of(name: &str) -> String {
    name.chars().find(|c| c.is_alphanumeric()).or_else(|| name.chars().next()).map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "?".to_string())
}

/// Bouton carré affichant une lettre.
fn letter_button(ui: &mut Ui, letter: &str, tip: &str, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
    let v = ui.visuals();
    let fill = if selected {
        v.selection.bg_fill
    } else if resp.is_pointer_button_down_on() {
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
