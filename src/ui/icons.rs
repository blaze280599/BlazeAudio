//! Small line icons painted with egui shapes, so nothing has to be bundled.

use egui::{Color32, CornerRadius, Pos2, Rect, Stroke, StrokeKind, Vec2};

#[derive(Clone, Copy)]
pub(super) enum Icon {
    Logo,
    Visuals,
    Audio,
    Palette,
    General,
    Keys,
    Collapse,
    Expand,
    Close,
    HideSidebar,
    Reset,
    Fullscreen,
    Overlay,
    Speaker,
    Muted,
    Play,
    Pause,
    Previous,
    Next,
}

/// Small line icons, drawn in a ~16 pt box centred on `c`.
pub(super) fn paint_icon(p: &egui::Painter, icon: Icon, c: Pos2, col: Color32) {
    let s = Stroke::new(1.5_f32, col);
    let v = egui::vec2;
    let line = |a: Vec2, b: Vec2| p.line_segment([c + a, c + b], s);
    let chevron = |dx: f32, dir: f32| {
        line(v(dx + 2.5 * dir, -5.0), v(dx - 2.5 * dir, 0.0));
        line(v(dx - 2.5 * dir, 0.0), v(dx + 2.5 * dir, 5.0));
    };
    match icon {
        Icon::Logo => {
            p.circle_filled(c, 3.5, col);
            for i in 0..12 {
                let a = i as f32 / 12.0 * std::f32::consts::TAU;
                let d = v(a.cos(), a.sin());
                let len = if i % 2 == 0 { 4.0 } else { 2.5 };
                p.line_segment([c + d * 6.0, c + d * (6.0 + len)], Stroke::new(1.8_f32, col));
            }
        }
        Icon::Visuals => {
            for (y, k) in [(-5.0, -3.0), (0.0, 3.5), (5.0, -0.5)] {
                line(v(-7.0, y), v(7.0, y));
                p.circle_filled(c + v(k, y), 2.4, col);
            }
        }
        Icon::Audio => {
            for (i, h) in [4.0, 9.0, 14.0, 8.0, 5.0].iter().enumerate() {
                let x = -6.0 + i as f32 * 3.0;
                line(v(x, -h / 2.0), v(x, h / 2.0));
            }
        }
        Icon::Palette => {
            p.circle_stroke(c, 7.0, s);
            for (x, y) in [(-3.0, -2.5), (2.5, -3.0), (3.5, 2.0)] {
                p.circle_filled(c + v(x, y), 1.5, col);
            }
        }
        Icon::General => {
            // Gear: a ring with eight teeth around a hub.
            p.circle_stroke(c, 5.0, s);
            p.circle_stroke(c, 2.0, s);
            for i in 0..8 {
                let a = i as f32 / 8.0 * std::f32::consts::TAU;
                let d = v(a.cos(), a.sin());
                p.line_segment([c + d * 5.0, c + d * 7.5], Stroke::new(2.4_f32, col));
            }
        }
        Icon::Keys => {
            p.rect_stroke(Rect::from_center_size(c, v(16.0, 11.0)), CornerRadius::same(2), s, StrokeKind::Middle);
            for x in [-4.5, -1.5, 1.5, 4.5] {
                p.circle_filled(c + v(x, -1.8), 0.9, col);
            }
            line(v(-3.5, 2.2), v(3.5, 2.2));
        }
        Icon::Collapse => {
            chevron(-2.5, 1.0);
            chevron(2.5, 1.0);
        }
        Icon::Expand => {
            chevron(-2.5, -1.0);
            chevron(2.5, -1.0);
        }
        Icon::Close => {
            line(v(-4.5, -4.5), v(4.5, 4.5));
            line(v(-4.5, 4.5), v(4.5, -4.5));
        }
        Icon::HideSidebar => {
            // A panel with its left pane folding away.
            p.rect_stroke(Rect::from_center_size(c, v(16.0, 13.0)), CornerRadius::same(2), s, StrokeKind::Middle);
            line(v(-3.0, -6.5), v(-3.0, 6.5));
            line(v(3.5, -3.0), v(1.0, 0.0));
            line(v(1.0, 0.0), v(3.5, 3.0));
        }
        Icon::Reset => {
            // Circular arrow, most of a turn, head at the open end.
            let r = 4.5;
            let (a0, a1) = (-40f32.to_radians(), 230f32.to_radians());
            let at = |a: f32| v(a.cos(), a.sin());
            let pts: Vec<Pos2> = (0..=16).map(|i| c + at(a0 + (a1 - a0) * i as f32 / 16.0) * r).collect();
            p.add(egui::Shape::line(pts, s));
            let (end, out) = (c + at(a1) * r, at(a1));
            let tan = v(-out.y, out.x);
            let head = vec![end + tan * 2.5, end - tan * 1.0 + out * 2.5, end - tan * 1.0 - out * 2.5];
            p.add(egui::Shape::convex_polygon(head, col, Stroke::NONE));
        }
        Icon::Fullscreen => {
            for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                line(v(6.0 * sx, 6.0 * sy), v(2.0 * sx, 6.0 * sy));
                line(v(6.0 * sx, 6.0 * sy), v(6.0 * sx, 2.0 * sy));
            }
        }
        Icon::Speaker | Icon::Muted => {
            let body = [v(-7.0, -2.5), v(-4.0, -2.5), v(0.0, -6.0), v(0.0, 6.0), v(-4.0, 2.5), v(-7.0, 2.5)];
            p.add(egui::Shape::convex_polygon(body.iter().map(|o| c + *o).collect(), col, Stroke::NONE));
            if matches!(icon, Icon::Muted) {
                line(v(3.0, -3.0), v(8.0, 3.0));
                line(v(3.0, 3.0), v(8.0, -3.0));
            } else {
                for r in [3.5, 7.0] {
                    let pts: Vec<Pos2> = (0..=8)
                        .map(|i| {
                            let a = (-45.0 + i as f32 * 11.25_f32).to_radians();
                            c + v(a.cos(), a.sin()) * r
                        })
                        .collect();
                    p.add(egui::Shape::line(pts, s));
                }
            }
        }
        Icon::Overlay => {
            p.rect_stroke(Rect::from_center_size(c + v(-2.0, -2.0), v(10.0, 8.0)), CornerRadius::same(1), s, StrokeKind::Middle);
            p.rect_stroke(Rect::from_center_size(c + v(2.5, 2.5), v(10.0, 8.0)), CornerRadius::same(1), s, StrokeKind::Middle);
        }
        Icon::Play => {
            // Nudged right so the triangle looks centred.
            let pts = [v(-3.0, -5.5), v(5.5, 0.0), v(-3.0, 5.5)];
            p.add(egui::Shape::convex_polygon(pts.iter().map(|o| c + *o).collect(), col, Stroke::NONE));
        }
        Icon::Pause => {
            for x in [-3.0, 3.0] {
                p.rect_filled(Rect::from_center_size(c + v(x, 0.0), v(3.0, 11.0)), CornerRadius::same(1), col);
            }
        }
        Icon::Previous | Icon::Next => {
            let d = if matches!(icon, Icon::Next) { 1.0 } else { -1.0 };
            let pts = [v(-4.0 * d, -5.0), v(3.0 * d, 0.0), v(-4.0 * d, 5.0)];
            p.add(egui::Shape::convex_polygon(pts.iter().map(|o| c + *o).collect(), col, Stroke::NONE));
            p.rect_filled(Rect::from_center_size(c + v(4.5 * d, 0.0), v(2.0, 10.0)), CornerRadius::ZERO, col);
        }
    }
}
