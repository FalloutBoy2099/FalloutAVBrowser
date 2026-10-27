//! Barre d'outils (bas de la fenêtre) et barre de lecture (bas de la zone centrale).

use super::*;

impl App {
    pub(crate) fn toolbar(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        ui.add_space(3.0);
        let mut group_right = 0.0f32;
        let bar = ui
            .horizontal(|ui| {
                let pad = ((ui.available_width() - self.toolbar_w) / 2.0).max(0.0);
                ui.add_space(pad);
                let group = ui.horizontal(|ui| {
                    ui.spacing_mut().interact_size.y = 28.0;
                    ui.checkbox(&mut self.settings.show_tree, tr("t.tree"));
                    ui.separator();
                    if icon_button(ui, Icon::Folder, &format!("{}  (Ctrl+L)", tr("t.folder")), false, true).clicked() {
                        self.open_folder_dialog();
                    }
                    ui.separator();
                    let has_files = self.files.len() > 1;
                    if icon_button(ui, Icon::Prev, &tr("av.prev"), false, has_files).clicked() {
                        self.step(&ctx, -1, self.playing);
                    }
                    if icon_button(ui, Icon::Next, &tr("av.next"), false, has_files).clicked() {
                        self.step(&ctx, 1, self.playing);
                    }
                    ui.separator();
                    if ui.checkbox(&mut self.settings.looping, tr("av.loop")).on_hover_text(tr("av.loop_tip")).changed() {
                        self.sync_loop();
                    }
                    if ui.checkbox(&mut self.settings.auto_next, tr("av.auto_short")).on_hover_text(tr("av.auto_next")).changed() {
                        self.sync_loop();
                    }
                    ui.separator();
                    ui.label(tr("av.volume"));
                    ui.spacing_mut().slider_width = 90.0;
                    if ui.add(egui::Slider::new(&mut self.settings.volume, 0.0..=1.0).show_value(false)).changed() {
                        self.out.set_volume(self.settings.volume);
                    }
                    ui.separator();
                    ui.label(tr("av.speed"));
                    // Champ numérique (nombre seul) avec boutons - / + par pas de 0,1.
                    let mut sp = self.settings.speed;
                    let mut changed = false;
                    if ui.add(egui::Button::new("-").min_size(vec2(28.0, 28.0))).clicked() {
                        sp -= 0.1;
                        changed = true;
                    }
                    let field = ui.add(egui::DragValue::new(&mut sp).range(0.2..=5.0).speed(0.02).fixed_decimals(1));
                    changed |= field.changed();
                    field.on_hover_text(tr("av.speed_tip"));
                    if ui.add(egui::Button::new("+").min_size(vec2(28.0, 28.0))).clicked() {
                        sp += 0.1;
                        changed = true;
                    }
                    if changed {
                        self.settings.speed = ((sp.clamp(0.2, 5.0)) * 10.0).round() / 10.0;
                        self.out.set_speed(self.settings.speed);
                    }
                    ui.separator();
                    ui.checkbox(&mut self.settings.show_props, tr("properties"));
                });
                group_right = group.response.rect.right();
                let w = group.response.rect.width();
                if (w - self.toolbar_w).abs() > 0.5 {
                    self.toolbar_w = w;
                    ctx.request_repaint();
                }
            })
            .response
            .rect;
        // Informations à droite de la barre : rang du fichier dans le dossier.
        if let Some(i) = self.index {
            let info = format!("{} / {}", i + 1, self.files.len());
            let galley = ui.painter().layout_no_wrap(info, FontId::proportional(13.0), ui.visuals().weak_text_color());
            if bar.right() - 8.0 - galley.size().x > group_right + 12.0 {
                ui.painter().galley(pos2(bar.right() - 8.0 - galley.size().x, bar.center().y - galley.size().y / 2.0), galley, Color32::PLACEHOLDER);
            }
        }
        ui.add_space(3.0);
    }

    /// Commandes de lecture, en bas de la zone centrale : lecture / pause, arrêt, position / durée et barre de progression.
    pub(crate) fn transport(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let dur = self.duration();
        let enabled = self.loaded.is_some();
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().interact_size.y = 28.0;
            let (icon, tip) = if self.playing { (Icon::Pause, tr("av.pause")) } else { (Icon::Play, tr("av.play")) };
            if icon_button(ui, icon, &format!("{}  (Space)", tip), false, enabled).clicked() {
                self.toggle_play();
            }
            if icon_button(ui, Icon::Stop, &tr("av.stop"), false, enabled).clicked() {
                self.stop();
            }
            let time = format!("{} / {}", fmt_time(self.vt), fmt_time(dur));
            let time_w = ui.painter().layout_no_wrap(time.clone(), FontId::proportional(14.0), Color32::WHITE).size().x;
            // Barre de progression masquable pour l'audio (la vidéo la garde toujours).
            let show_bar = self.settings.show_transport || self.is_video();
            if show_bar {
                let mut t = self.vt;
                ui.spacing_mut().slider_width = (ui.available_width() - time_w - 28.0).max(60.0);
                let slider = ui.add_enabled(enabled && dur > 0.0, egui::Slider::new(&mut t, 0.0..=dur.max(0.001)).show_value(false).trailing_fill(true));
                if slider.changed() {
                    self.seek(t);
                    self.update_video(&ctx, false);
                }
            }
            if show_bar {
                ui.label(time);
            } else {
                // Sans la barre : le temps reste à droite, à la place qu'il a avec la barre.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(time));
            }
        });
        ui.add_space(3.0);
    }
}
