//! Zone centrale : image de la vidéo, ou forme d'onde de l'audio.

use super::*;
use crate::audio::Pcm;

/// Ce que l'utilisateur a demandé sur la zone centrale pendant cette image ; appliqué une fois le dessin terminé.
#[derive(Default)]
struct CanvasEvents {
    /// Facteur de zoom demandé (molette), 1.0 = aucun.
    zoom: Option<f32>,
    /// Double-clic : passer de « ajusté » à 100 %.
    actual_size: bool,
    /// Clic simple sur la vidéo : lecture / pause.
    toggle_play: bool,
    /// Position demandée (secondes) en cliquant sur la forme d'onde.
    seek_to: Option<f64>,
}

impl App {
    pub(crate) fn canvas(&mut self, ui: &mut Ui) {
        let rect = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        ui.painter().rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);
        if let Some(e) = &self.error {
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, e, FontId::proportional(15.0), ui.visuals().error_fg_color);
            return;
        }
        if self.loaded.is_none() {
            let msg = if self.loading { tr("av.loading") } else { tr("av.hint") };
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, msg, FontId::proportional(16.0), ui.visuals().weak_text_color());
            return;
        }
        if self.settings.canvas_menu {
            resp.context_menu(|ui| {
                if ui.add(egui::Button::new(tr("av.export")).shortcut_text("Ctrl+E")).clicked() {
                    self.open_convert();
                    ui.close();
                }
            });
        }
        let events = if self.tex.is_some() && self.is_video() { self.draw_video(ui, rect, &resp) } else { self.draw_waveform(ui, rect, &resp) };
        if let Some(f) = events.zoom {
            self.zoom_by(f);
        }
        if events.actual_size {
            self.zoom_actual();
        }
        if events.toggle_play {
            self.toggle_play();
        }
        if let Some(t) = events.seek_to {
            self.seek(t);
        }
    }

    /// Image de la vidéo, ajustée à la zone (ou zoomée / déplacée à la main).
    fn draw_video(&mut self, ui: &mut Ui, rect: Rect, resp: &egui::Response) -> CanvasEvents {
        let mut ev = CanvasEvents::default();
        let (Some(tex), Some(video)) = (self.tex.clone(), self.loaded.as_ref().and_then(|l| l.video.clone())) else { return ev };
        ui.painter().rect_filled(rect, 0.0, Color32::BLACK);
        let (w, h) = (video.width() as f32, video.height() as f32);
        let fit_scale = (rect.width() / w).min(rect.height() / h).max(0.01);
        // Molette : zoom ; glisser : déplacement ; double-clic : ajuster / 100 % ; clic : lecture / pause.
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                ev.zoom = Some(if scroll > 0.0 { 1.1 } else { 1.0 / 1.1 });
            }
        }
        if resp.double_clicked() {
            if self.fit {
                ev.actual_size = true;
            } else {
                self.fit = true;
                self.pan = Vec2::ZERO;
            }
        } else if resp.clicked() {
            ev.toggle_play = true;
        }
        if resp.dragged() && !self.fit {
            self.pan += resp.drag_delta();
        }
        let scale = if self.fit { fit_scale } else { self.zoom };
        self.cur_scale = scale;
        let offset = if self.fit { Vec2::ZERO } else { self.pan };
        let image_rect = Rect::from_center_size(rect.center() + offset, vec2(w * scale, h * scale));
        ui.painter_at(rect).image(tex.id(), image_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        ev
    }

    /// Forme d'onde d'un fichier audio : la partie jouée est pleine, le reste atténué ; un clic ou un glisser déplace la lecture.
    fn draw_waveform(&self, ui: &mut Ui, rect: Rect, resp: &egui::Response) -> CanvasEvents {
        let mut ev = CanvasEvents::default();
        if self.peaks.is_empty() {
            return ev;
        }
        let dur = self.duration();
        let wave = Rect::from_min_max(pos2(rect.left() + 24.0, rect.top() + 56.0), pos2(rect.right() - 24.0, rect.bottom() - 24.0));
        let mid = wave.center().y;
        let n = self.peaks.len();
        let [wr, wg, wb] = self.settings.wave_color;
        let col = Color32::from_rgb(wr, wg, wb);
        let played = if dur > 0.0 { (self.vt / dur).clamp(0.0, 1.0) as f32 } else { 0.0 };
        let thick = (wave.width() / n as f32).max(1.0);
        for (i, (lo, hi)) in self.peaks.iter().enumerate() {
            let x = wave.left() + wave.width() * (i as f32 + 0.5) / n as f32;
            let c = if (i as f32 / n as f32) < played { col } else { col.gamma_multiply(0.45) };
            let (y0, y1) = (mid - hi.min(1.0) * wave.height() * 0.5, mid - lo.max(-1.0) * wave.height() * 0.5);
            ui.painter().line_segment([pos2(x, y0), pos2(x, y1.max(y0 + 1.0))], Stroke::new(thick, c));
        }
        // Trait de lecture : sa propre couleur.
        let [lr, lg, lb] = self.settings.line_color;
        let px = wave.left() + wave.width() * played;
        ui.painter().line_segment([pos2(px, wave.top()), pos2(px, wave.bottom())], Stroke::new(1.5, Color32::from_rgb(lr, lg, lb)));
        if (resp.clicked() || resp.dragged())
            && let Some(p) = resp.interact_pointer_pos()
        {
            ev.seek_to = Some(((p.x - wave.left()) / wave.width()).clamp(0.0, 1.0) as f64 * dur);
        }
        if let Some(name) = self.wanted.as_ref().and_then(|p| p.file_name()) {
            ui.painter().text(pos2(rect.left() + 24.0, rect.top() + 16.0), Align2::LEFT_TOP, name.to_string_lossy(), FontId::proportional(17.0), ui.visuals().strong_text_color());
        }
        ev
    }
}

/// Enveloppe (min, max) de la forme d'onde, canaux moyennés.
pub(crate) fn peaks(p: &Pcm, n: usize) -> Vec<(f32, f32)> {
    let ch = p.channels.max(1) as usize;
    let frames = p.frames();
    if frames == 0 {
        return Vec::new();
    }
    (0..n)
        .map(|b| {
            let (a, z) = (b * frames / n, ((b + 1) * frames / n).max(b * frames / n + 1).min(frames));
            let (mut lo, mut hi) = (0f32, 0f32);
            let step = ((z - a) / 64).max(1);
            let mut f = a;
            while f < z {
                let v = (0..ch).map(|c| p.samples[f * ch + c] as f32).sum::<f32>() / ch as f32 / 32768.0;
                lo = lo.min(v);
                hi = hi.max(v);
                f += step;
            }
            (lo, hi)
        })
        .collect()
}
