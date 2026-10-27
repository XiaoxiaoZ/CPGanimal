//! The champion's trophy (drawing only).

use super::scene::{glow, mix, shaded_polygon};
use eframe::egui::{self, Color32, Pos2, Rect, Shape, Stroke, Vec2};

/// A four-pointed sparkle.
fn sparkle(painter: &egui::Painter, c: Pos2, r: f32, color: Color32) {
    let pts: Vec<Pos2> = (0..8)
        .map(|k| {
            let a = k as f32 * std::f32::consts::FRAC_PI_4;
            let len = if k % 2 == 0 { r } else { r * 0.28 };
            c + Vec2::new(a.cos(), a.sin()) * len
        })
        .collect();
    shaded_polygon(painter, &pts, c, Color32::WHITE, |_| color);
}

/// A gold cup, gleaming.
pub fn trophy(painter: &egui::Painter, center: Pos2, size: f32) {
    let time = painter.ctx().input(|i| i.time) as f32;
    let gold_light = Color32::from_rgb(255, 230, 130);
    let gold = Color32::from_rgb(222, 170, 40);
    let gold_dark = Color32::from_rgb(150, 98, 20);
    let s = size;
    glow(painter, center, Vec2::splat(s * 1.3), Color32::from_rgba_unmultiplied(255, 210, 90, 60));
    // Bowl: a half ellipse, lit from the left.
    let top = center.y - s * 0.55;
    let bowl: Vec<Pos2> = (0..=20)
        .map(|k| {
            let a = std::f32::consts::PI * k as f32 / 20.0;
            Pos2::new(center.x + s * 0.42 * a.cos(), top + s * 0.55 * a.sin())
        })
        .collect();
    shaded_polygon(painter, &bowl, Pos2::new(center.x - s * 0.1, top + s * 0.2), gold_light, |p| mix(gold, gold_dark, (p.x - center.x) / (s * 0.42) * 0.5 + 0.4));
    // Handles.
    for side in [-1.0f32, 1.0] {
        let pts: Vec<Pos2> = (0..=12)
            .map(|k| {
                let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * k as f32 / 12.0;
                Pos2::new(center.x + side * (s * 0.42 + s * 0.16 * a.cos()), top + s * 0.18 + s * 0.17 * a.sin())
            })
            .collect();
        painter.add(Shape::line(pts, Stroke::new(s * 0.06, gold)));
    }
    // Stem and base.
    let stem = Rect::from_center_size(Pos2::new(center.x, top + s * 0.68), Vec2::new(s * 0.1, s * 0.28));
    painter.rect_filled(stem, 0.0, gold);
    let knob = Pos2::new(center.x, top + s * 0.6);
    painter.add(Shape::ellipse_filled(knob, Vec2::new(s * 0.1, s * 0.05), gold_light));
    let base = Rect::from_center_size(Pos2::new(center.x, top + s * 0.88), Vec2::new(s * 0.5, s * 0.14));
    painter.rect_filled(base, s * 0.03, gold_dark);
    painter.rect_filled(Rect::from_center_size(base.center() - Vec2::new(0.0, s * 0.1), Vec2::new(s * 0.34, s * 0.07)), s * 0.02, gold);
    // Rim and shine.
    painter.add(Shape::ellipse_filled(Pos2::new(center.x, top), Vec2::new(s * 0.42, s * 0.06), gold_light));
    painter.add(Shape::ellipse_filled(Pos2::new(center.x, top), Vec2::new(s * 0.36, s * 0.035), gold_dark));
    painter.line_segment([Pos2::new(center.x - s * 0.26, top + s * 0.1), Pos2::new(center.x - s * 0.2, top + s * 0.38)], Stroke::new(s * 0.04, Color32::from_white_alpha(170)));
    for k in 0..4 {
        let a = time * 0.7 + k as f32 * 1.7;
        let p = center + Vec2::new(a.cos() * s * 0.7, (a * 1.3).sin() * s * 0.6);
        let tw = (time * 3.0 + k as f32 * 2.1).sin().max(0.0);
        sparkle(painter, p, s * 0.09 * tw, Color32::from_rgba_unmultiplied(255, 240, 180, 200));
    }
}
