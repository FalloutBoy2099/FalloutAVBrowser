//! Boutons à icônes dessinées au pinceau (indépendants des polices).

use eframe::egui::{self, Color32, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2, pos2, vec2};

#[derive(Clone, Copy)]
pub enum Icon {
    Prev,
    Next,
    ZoomIn,
    ZoomOut,
    Actual,
    Fit,
    Play,
    Pause,
    Stop,
    Desktop,
    Documents,
    Downloads,
    Star,
    Sha,
    Folder,
    CopyText,
    CopyJson,
    CopyXml,
    Close,
    CopyImage,
    CopyPath,
    CopyTo,
    MoveTo,
    Rename,
    Convert,
    Palette,
}

pub fn icon_button(ui: &mut Ui, icon: Icon, tip: &str, selected: bool, enabled: bool) -> Response {
    let size = vec2(32.0, 28.0);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let visuals = ui.visuals();
    let style = if !enabled {
        &visuals.widgets.noninteractive
    } else if resp.is_pointer_button_down_on() {
        &visuals.widgets.active
    } else if resp.hovered() {
        &visuals.widgets.hovered
    } else {
        &visuals.widgets.inactive
    };
    let mut fg = visuals.widgets.inactive.fg_stroke.color;
    if !enabled {
        fg = visuals.widgets.noninteractive.fg_stroke.color.gamma_multiply(0.45);
    }
    if selected || resp.hovered() || resp.is_pointer_button_down_on() {
        let fill = if selected { visuals.selection.bg_fill } else { style.bg_fill };
        ui.painter().rect_filled(rect, 4.0, fill);
    }
    if selected {
        fg = visuals.selection.stroke.color;
    }
    paint_icon(ui, rect.shrink2(vec2(6.0, 4.0)), icon, fg);
    resp.on_hover_text(tip)
}

fn paint_icon(ui: &Ui, r: Rect, icon: Icon, c: Color32) {
    let p = ui.painter();
    let st = Stroke::new(1.6, c);
    let cen = r.center();
    match icon {
        Icon::Prev => tri(p, cen, 7.0, true, c),
        Icon::Next => tri(p, cen, 7.0, false, c),
        Icon::ZoomIn | Icon::ZoomOut => {
            let lc = cen - vec2(2.0, 2.0);
            p.circle_stroke(lc, 6.0, st);
            p.line_segment([lc + vec2(4.5, 4.5), lc + vec2(9.0, 9.0)], Stroke::new(2.2, c));
            p.line_segment([lc - vec2(3.0, 0.0), lc + vec2(3.0, 0.0)], st);
            if matches!(icon, Icon::ZoomIn) {
                p.line_segment([lc - vec2(0.0, 3.0), lc + vec2(0.0, 3.0)], st);
            }
        }
        Icon::Actual => {
            p.text(cen, egui::Align2::CENTER_CENTER, "1:1", egui::FontId::proportional(13.0), c);
        }
        Icon::Fit => {
            let q = r.shrink(2.0);
            for (a, b, d1, d2) in [
                (q.left_top(), q.left_top(), vec2(5.0, 0.0), vec2(0.0, 5.0)),
                (q.right_top(), q.right_top(), vec2(-5.0, 0.0), vec2(0.0, 5.0)),
                (q.left_bottom(), q.left_bottom(), vec2(5.0, 0.0), vec2(0.0, -5.0)),
                (q.right_bottom(), q.right_bottom(), vec2(-5.0, 0.0), vec2(0.0, -5.0)),
            ] {
                p.line_segment([a, a + d1], st);
                p.line_segment([b, b + d2], st);
            }
        }
        Icon::Play => tri(p, cen, 7.0, false, c),
        Icon::CopyText => {
            // Page de texte : feuille avec quelques lignes.
            let q = Rect::from_center_size(cen, vec2(13.0, 16.0));
            p.rect_stroke(q, 1.5, st, egui::StrokeKind::Inside);
            for i in 0..3 {
                let y = q.top() + 4.5 + i as f32 * 3.6;
                p.line_segment([pos2(q.left() + 3.0, y), pos2(q.right() - 3.0, y)], Stroke::new(1.2, c));
            }
        }
        Icon::CopyJson => {
            p.text(cen, egui::Align2::CENTER_CENTER, "{ }", egui::FontId::monospace(14.0), c);
        }
        Icon::CopyXml => {
            p.text(cen, egui::Align2::CENTER_CENTER, "</>", egui::FontId::monospace(12.0), c);
        }
        Icon::Close => {
            let d = 5.5;
            p.line_segment([cen + vec2(-d, -d), cen + vec2(d, d)], Stroke::new(1.8, c));
            p.line_segment([cen + vec2(-d, d), cen + vec2(d, -d)], Stroke::new(1.8, c));
        }
        Icon::CopyImage => {
            // Deux cadres superposés ; le premier contient un petit paysage.
            let back = Rect::from_center_size(cen + vec2(3.0, -3.0), vec2(12.0, 10.0));
            let front = Rect::from_center_size(cen + vec2(-2.0, 2.5), vec2(12.0, 10.0));
            p.rect_stroke(back, 1.5, Stroke::new(1.2, c), egui::StrokeKind::Inside);
            p.rect_filled(front, 1.5, ui.visuals().panel_fill);
            p.rect_stroke(front, 1.5, st, egui::StrokeKind::Inside);
            p.add(Shape::convex_polygon(
                vec![front.left_bottom() + vec2(2.0, -2.0), front.center() + vec2(-1.0, -1.0), front.center() + vec2(1.5, 2.5), front.right_bottom() + vec2(-2.0, -2.0)],
                c,
                Stroke::NONE,
            ));
        }
        Icon::CopyPath => {
            p.text(cen, egui::Align2::CENTER_CENTER, "a/b", egui::FontId::monospace(12.0), c);
        }
        Icon::CopyTo | Icon::MoveTo => {
            let q = Rect::from_center_size(cen + vec2(-2.5, -1.5), vec2(12.0, 9.0));
            p.rect_stroke(q, 1.2, Stroke::new(1.2, c), egui::StrokeKind::Inside);
            p.rect_filled(Rect::from_min_size(q.left_top() - vec2(0.0, 2.0), vec2(5.0, 2.5)), 1.0, c);
            let y = cen.y + 6.0;
            if matches!(icon, Icon::MoveTo) {
                p.line_segment([pos2(cen.x - 1.0, y - 2.5), pos2(cen.x - 1.0, y + 2.5)], Stroke::new(1.4, c));
            }
            arrow(p, pos2(cen.x + 1.0, y), pos2(cen.x + 8.0, y), Stroke::new(1.4, c));
        }
        Icon::Rename => {
            // Crayon incliné.
            let (a, b) = (cen + vec2(-6.0, 6.0), cen + vec2(4.0, -4.0));
            p.line_segment([a + vec2(2.0, -2.0), b], Stroke::new(3.5, c));
            p.add(Shape::convex_polygon(vec![a, a + vec2(1.0, -3.5), a + vec2(3.5, -1.0)], c, Stroke::NONE));
            p.line_segment([cen + vec2(1.0, 7.0), cen + vec2(8.0, 7.0)], Stroke::new(1.2, c));
        }
        Icon::Convert => {
            arrow(p, pos2(cen.x - 7.0, cen.y - 3.5), pos2(cen.x + 7.0, cen.y - 3.5), st);
            arrow(p, pos2(cen.x + 7.0, cen.y + 3.5), pos2(cen.x - 7.0, cen.y + 3.5), st);
        }
        Icon::Palette => {
            let colors = [Color32::from_rgb(220, 70, 60), Color32::from_rgb(90, 180, 80), Color32::from_rgb(70, 120, 220), Color32::from_rgb(235, 190, 50)];
            for (i, col) in colors.iter().enumerate() {
                let (x, y) = ((i % 2) as f32, (i / 2) as f32);
                p.rect_filled(Rect::from_min_size(cen + vec2(-8.0 + x * 8.5, -8.0 + y * 8.5), vec2(7.5, 7.5)), 1.5, *col);
            }
        }
        Icon::Folder => {
            let q = Rect::from_center_size(cen, vec2(18.0, 13.0));
            let tab = Rect::from_min_size(q.left_top() - vec2(0.0, 2.0), vec2(8.0, 3.0));
            p.rect_stroke(q, 1.5, st, egui::StrokeKind::Inside);
            p.rect_filled(tab, 1.0, c);
        }
        Icon::Desktop => {
            // Écran sur son pied.
            let q = Rect::from_center_size(cen - vec2(0.0, 1.5), vec2(18.0, 12.0));
            p.rect_stroke(q, 1.5, st, egui::StrokeKind::Inside);
            p.line_segment([pos2(cen.x, q.bottom()), pos2(cen.x, q.bottom() + 3.0)], st);
            p.line_segment([pos2(cen.x - 4.0, q.bottom() + 3.5), pos2(cen.x + 4.0, q.bottom() + 3.5)], st);
        }
        Icon::Documents => {
            // Feuille avec des lignes de texte.
            let q = Rect::from_center_size(cen, vec2(13.0, 16.0));
            p.rect_stroke(q, 1.5, st, egui::StrokeKind::Inside);
            for i in 0..3 {
                let y = q.top() + 5.0 + i as f32 * 3.4;
                p.line_segment([pos2(q.left() + 3.0, y), pos2(q.right() - 3.0, y)], Stroke::new(1.2, c));
            }
        }
        Icon::Downloads => {
            // Flèche vers un plateau.
            arrow(p, cen - vec2(0.0, 7.0), cen + vec2(0.0, 3.0), st);
            p.line_segment([cen + vec2(-8.0, 7.0), cen + vec2(8.0, 7.0)], st);
            p.line_segment([cen + vec2(-8.0, 3.5), cen + vec2(-8.0, 7.0)], st);
            p.line_segment([cen + vec2(8.0, 3.5), cen + vec2(8.0, 7.0)], st);
        }
        Icon::Star => {
            let pts: Vec<Pos2> = (0..10)
                .map(|i| {
                    let r = if i % 2 == 0 { 9.0 } else { 3.8 };
                    let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
                    cen + vec2(a.cos() * r, a.sin() * r)
                })
                .collect();
            p.add(Shape::convex_polygon(pts, c, Stroke::NONE));
        }
        Icon::Sha => {
            // « # » : empreinte.
            for dx in [-2.5, 2.5] {
                p.line_segment([cen + vec2(dx, -7.0), cen + vec2(dx, 7.0)], st);
            }
            for dy in [-2.8, 2.8] {
                p.line_segment([cen + vec2(-7.0, dy), cen + vec2(7.0, dy)], st);
            }
        }
        Icon::Stop => {
            p.rect_filled(Rect::from_center_size(cen, vec2(12.0, 12.0)), 1.5, c);
        }
        Icon::Pause => {
            p.rect_filled(Rect::from_center_size(cen - vec2(3.5, 0.0), vec2(4.0, 13.0)), 1.0, c);
            p.rect_filled(Rect::from_center_size(cen + vec2(3.5, 0.0), vec2(4.0, 13.0)), 1.0, c);
        }
    }
}

fn tri(p: &egui::Painter, cen: Pos2, s: f32, left: bool, c: Color32) {
    let d = if left { -1.0 } else { 1.0 };
    let pts = vec![pos2(cen.x + d * s * 0.6, cen.y), pos2(cen.x - d * s * 0.5, cen.y - s), pos2(cen.x - d * s * 0.5, cen.y + s)];
    p.add(Shape::convex_polygon(pts, c, Stroke::NONE));
    let _ = Vec2::ZERO;
}

/// Flèche de `a` vers `b` (tige et deux ailes).
fn arrow(p: &egui::Painter, a: Pos2, b: Pos2, st: Stroke) {
    p.line_segment([a, b], st);
    let d = (b - a).normalized();
    let n = vec2(-d.y, d.x);
    for side in [1.0f32, -1.0] {
        p.line_segment([b, b - d * 4.5 + n * 3.5 * side], st);
    }
}
