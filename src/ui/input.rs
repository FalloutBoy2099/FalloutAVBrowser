//! Entrées : glisser-déposer, boutons de la souris, raccourcis clavier et saisie au clavier dans l'arborescence.

use super::*;
use crate::tree::RowKind;
use std::path::PathBuf;
use std::time::{Duration, Instant};

impl App {
    pub(crate) fn handle_input(&mut self, ctx: &egui::Context) {
        // Une fenêtre modale ouverte garde la main.
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
        self.side_buttons(ctx);
        // Un champ de saisie (le filtre) garde le clavier.
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        if self.type_in_tree(ctx) {
            return;
        }
        if ctx.input(|i| i.modifiers.command) {
            self.ctrl_shortcuts(ctx);
        } else {
            self.plain_shortcuts(ctx);
        }
    }

    /// Boutons latéraux de la souris : fichier précédent / suivant, ou lecture (pause) / fichier suivant.
    fn side_buttons(&mut self, ctx: &egui::Context) {
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
    }

    /// Saisie au clavier au-dessus de l'arborescence : saute au premier nom qui commence par les caractères tapés.
    /// Vrai si des caractères ont été saisis (les raccourcis à une lettre sont alors ignorés).
    fn type_in_tree(&mut self, ctx: &egui::Context) -> bool {
        let over_tree = self.tree_rect.is_some_and(|r| ctx.input(|i| i.pointer.hover_pos().is_some_and(|p| r.contains(p))));
        if !over_tree || ctx.input(|i| i.modifiers.command) {
            return false;
        }
        let typed: String = ctx.input(|i| i.events.iter().filter_map(|e| if let egui::Event::Text(t) = e { Some(t.clone()) } else { None }).collect());
        // Une espace n'est un caractère de recherche qu'au milieu d'un mot : seule, elle reste le raccourci lecture / pause.
        let empty = self.ta_buf.is_empty() || self.ta_time.elapsed() > Duration::from_millis(1200);
        let typed: String = typed.chars().filter(|c| !c.is_control() && (!c.is_whitespace() || !empty)).collect();
        if typed.is_empty() {
            return false;
        }
        self.type_ahead(ctx, &typed);
        true
    }

    fn ctrl_shortcuts(&mut self, ctx: &egui::Context) {
        use egui::Key;
        let pressed = |k: Key| ctx.input(|i| i.key_pressed(k));
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
    }

    fn plain_shortcuts(&mut self, ctx: &egui::Context) {
        use egui::Key;
        let pressed = |k: Key| ctx.input(|i| i.key_pressed(k));
        if pressed(Key::F1) {
            self.show_about = true;
        }
        // Lecture.
        if pressed(Key::Space) {
            self.toggle_play();
        }
        if pressed(Key::ArrowLeft) {
            self.seek(self.vt - 5.0);
        }
        if pressed(Key::ArrowRight) {
            self.seek(self.vt + 5.0);
        }
        // Fichiers du dossier.
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
        // Zoom de la vidéo.
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
        // Fenêtre.
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
    pub(crate) fn type_ahead(&mut self, ctx: &egui::Context, typed: &str) {
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
}
