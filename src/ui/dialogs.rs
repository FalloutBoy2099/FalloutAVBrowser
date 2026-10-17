//! Fenêtres annexes : dossier courant, confirmation, À propos, renommer, conversion, et message temporaire.

use super::*;
use crate::convert;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

impl App {
    pub(crate) fn open_folder_dialog(&mut self) {
        let cur = self.tree.selected_dir.clone().or_else(|| self.settings.last_dir.clone());
        self.folder_dlg = Some((cur.map(|p| p.to_string_lossy().to_string()).unwrap_or_default(), true));
    }

    pub(crate) fn folder_window(&mut self, ctx: &egui::Context) {
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
                let chars = text.chars().count();
                focus_and_select(ui, &mut out, chars);
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

    pub(crate) fn expand_confirm_window(&mut self, ctx: &egui::Context) {
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

    pub(crate) fn about_window(&mut self, ctx: &egui::Context) {
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

    /// Fenêtre « Renommer » : le nom sans l'extension est sélectionné ; un nom invalide ou déjà pris est refusé.
    pub(crate) fn rename_window(&mut self, ctx: &egui::Context) {
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
                // Le nom sans son extension est sélectionné.
                let stem_end = dlg.name.rfind('.').filter(|i| *i > 0).unwrap_or(dlg.name.len());
                let chars = dlg.name[..stem_end].chars().count();
                focus_and_select(ui, &mut out, chars);
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

    pub(crate) fn convert_window(&mut self, ctx: &egui::Context) {
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

    pub(crate) fn toast_overlay(&mut self, ctx: &egui::Context) {
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
}

/// À l'ouverture d'une fenêtre : donne le focus au champ de saisie et sélectionne ses `chars` premiers caractères.
fn focus_and_select(ui: &Ui, out: &mut egui::text_edit::TextEditOutput, chars: usize) {
    out.response.request_focus();
    let mut state = out.state.clone();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(chars))));
    state.store(ui.ctx(), out.response.id);
}
