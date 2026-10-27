//! The look of the arena, shared by every view: an evening sky over a
//! stadium, textured ground with grass, shaded creatures with shadows, the
//! sumo platform (a clay ring edged with straw) and the crowd.
//! The textures are generated when first needed, so there are no image files.

use cpg_arena::creature::Rules;
use cpg_arena::sim::{Arena, Stage};
use eframe::egui::{self, epaint::Vertex, Align2, Color32, ColorImage, FontFamily, FontId, Mesh, Pos2, Rect, Shape, Stroke, TextureHandle, TextureOptions, Vec2};
use std::sync::atomic::{AtomicBool, Ordering};

/// Metres of ground per texture tile.
const TILE: f32 = 1.6;
const GRASS_TOP: Color32 = Color32::from_rgb(132, 196, 86);
const GRASS: Color32 = Color32::from_rgb(70, 128, 52);
const STRAW: Color32 = Color32::from_rgb(214, 178, 96);

/// Whether the display font (bold, for banners) was found.
static DISPLAY: AtomicBool = AtomicBool::new(false);

/// Add a bold font for big titles, if the computer has one (Windows: Segoe UI).
pub fn install_fonts(ctx: &egui::Context) {
    let candidates = ["C:\\Windows\\Fonts\\seguibl.ttf", "C:\\Windows\\Fonts\\segoeuib.ttf", "C:\\Windows\\Fonts\\arialbd.ttf", "/System/Library/Fonts/Supplemental/Arial Bold.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"];
    let Some(bytes) = candidates.iter().find_map(|p| std::fs::read(p).ok()) else { return };
    // The bold font first, then egui's own fonts for the glyphs it lacks (★, …).
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("display".into(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
    let mut family = vec!["display".to_string()];
    family.extend(fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default());
    fonts.families.insert(FontFamily::Name("display".into()), family);
    ctx.set_fonts(fonts);
    DISPLAY.store(true, Ordering::Relaxed);
}

/// A bold font for titles and banners (the normal one if none was found).
pub fn display(size: f32) -> FontId {
    if DISPLAY.load(Ordering::Relaxed) {
        FontId::new(size, FontFamily::Name("display".into()))
    } else {
        FontId::proportional(size)
    }
}

/// Text with a soft dark shadow, readable on any background.
pub fn shadow_text(painter: &egui::Painter, pos: Pos2, align: Align2, text: impl ToString, font: FontId, color: Color32) -> Rect {
    let text = text.to_string();
    let k = (font.size / 18.0).max(1.0);
    for (dx, dy, a) in [(0.0, 2.0, 140), (1.0, 3.0, 90), (0.0, 0.0, 0)] {
        if a > 0 {
            painter.text(pos + Vec2::new(dx, dy) * k, align, &text, font.clone(), Color32::from_black_alpha(a));
        }
    }
    painter.text(pos, align, text, font, color)
}

pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

fn shade(c: Color32, f: f32) -> Color32 {
    let s = |x: u8| (x as f32 * f).clamp(0.0, 255.0) as u8;
    Color32::from_rgba_unmultiplied(s(c.r()), s(c.g()), s(c.b()), c.a())
}

fn with_alpha(c: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (c.a() as u16 * alpha as u16 / 255) as u8)
}

/// Deterministic noise in [0, 1) from integers.
fn hash(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8DA6_B343) ^ (y as u32).wrapping_mul(0xD816_3841) ^ seed.wrapping_mul(0xCB1A_B31F);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5BD1_E995);
    h ^= h >> 15;
    (h & 0xFF_FFFF) as f32 / 0x100_0000 as f32
}

/// Smooth noise in [0, 1) at `x` (any scale).
fn noise1(x: f32, seed: u32) -> f32 {
    let i = x.floor();
    let t = x - i;
    let t = t * t * (3.0 - 2.0 * t);
    let (a, b) = (hash(i as i32, 0, seed), hash(i as i32 + 1, 0, seed));
    a + (b - a) * t
}

// ---------------------------------------------------------------- textures

#[derive(Clone)]
struct Textures {
    soil: TextureHandle,
    clay: TextureHandle,
}

fn textures(ctx: &egui::Context) -> Textures {
    let id = egui::Id::new("arena-scene-textures");
    if let Some(t) = ctx.data(|d| d.get_temp::<Textures>(id)) {
        return t;
    }
    let t = Textures {
        soil: ctx.load_texture("soil", soil_image(), TextureOptions::LINEAR_REPEAT),
        clay: ctx.load_texture("clay", clay_image(), TextureOptions::LINEAR_REPEAT),
    };
    ctx.data_mut(|d| d.insert_temp(id, t.clone()));
    t
}

const TEX: usize = 128;

/// Value noise that tiles: `cells` cells across the texture, (u, v) in [0, 1).
fn tile_noise(u: f32, v: f32, cells: i32, seed: u32) -> f32 {
    let (fx, fy) = (u * cells as f32, v * cells as f32);
    let (ix, iy) = (fx.floor() as i32, fy.floor() as i32);
    let s = |t: f32| t * t * (3.0 - 2.0 * t);
    let (tx, ty) = (s(fx - ix as f32), s(fy - iy as f32));
    let h = |i: i32, j: i32| hash(i.rem_euclid(cells), j.rem_euclid(cells), seed);
    let top = h(ix, iy) + (h(ix + 1, iy) - h(ix, iy)) * tx;
    let bottom = h(ix, iy + 1) + (h(ix + 1, iy + 1) - h(ix, iy + 1)) * tx;
    top + (bottom - top) * ty
}

/// Earth: mottled brown with small stones.
pub fn soil_image() -> ColorImage {
    let dark = Color32::from_rgb(70, 49, 34);
    let light = Color32::from_rgb(132, 97, 64);
    let stone = Color32::from_rgb(158, 140, 116);
    let mut px = Vec::with_capacity(TEX * TEX);
    for y in 0..TEX {
        for x in 0..TEX {
            let (u, v) = (x as f32 / TEX as f32, y as f32 / TEX as f32);
            let n = 0.5 * tile_noise(u, v, 4, 1) + 0.3 * tile_noise(u, v, 12, 2) + 0.2 * tile_noise(u, v, 40, 3);
            let mut c = mix(dark, light, (n - 0.25) * 1.8);
            // Stones: at most one per 16 x 16 cell, wrapping around the edges.
            let (cx, cy) = (x as i32 / 16, y as i32 / 16);
            for (i, j) in (-1..=1).flat_map(|i| (-1..=1).map(move |j| (i, j))) {
                let (gx, gy) = ((cx + i).rem_euclid(8), (cy + j).rem_euclid(8));
                if hash(gx, gy, 9) > 0.35 {
                    continue;
                }
                let sx = (cx + i) as f32 * 16.0 + 3.0 + hash(gx, gy, 10) * 10.0;
                let sy = (cy + j) as f32 * 16.0 + 3.0 + hash(gx, gy, 11) * 10.0;
                let r = 1.6 + hash(gx, gy, 12) * 2.2;
                let d = ((x as f32 - sx).powi(2) + (y as f32 - sy).powi(2) * 1.6).sqrt();
                if d < r {
                    c = mix(stone, dark, (d / r).powi(3) * 0.8 + (y as f32 - sy + r) / (4.0 * r) * 0.4);
                }
            }
            px.push(c);
        }
    }
    ColorImage::new([TEX, TEX], px)
}

/// Packed clay of the sumo ring: sandy, fine-grained, faint layers.
pub fn clay_image() -> ColorImage {
    let dark = Color32::from_rgb(168, 128, 86);
    let light = Color32::from_rgb(214, 178, 128);
    let mut px = Vec::with_capacity(TEX * TEX);
    for y in 0..TEX {
        for x in 0..TEX {
            let (u, v) = (x as f32 / TEX as f32, y as f32 / TEX as f32);
            let layers = 0.5 + 0.5 * (v * std::f32::consts::TAU * 6.0 + 2.0 * tile_noise(u, v, 3, 5)).sin();
            let n = 0.45 * tile_noise(u, v, 6, 6) + 0.25 * layers + 0.3 * tile_noise(u, v, 48, 7);
            px.push(mix(dark, light, n));
        }
    }
    ColorImage::new([TEX, TEX], px)
}

/// The display font at `size`, smaller (down to half) if `text` would not fit in `max` pixels.
pub fn fitting_font(painter: &egui::Painter, text: &str, size: f32, max: f32) -> FontId {
    let mut s = size;
    while s > size / 2.0 && painter.layout_no_wrap(text.to_string(), display(s), Color32::WHITE).size().x > max {
        s -= 1.0;
    }
    display(s)
}

/// `text` shortened with "…" to fit in `max` pixels.
pub fn fit(painter: &egui::Painter, text: &str, font: &FontId, max: f32) -> String {
    let width = |s: &str| painter.layout_no_wrap(s.to_string(), font.clone(), Color32::WHITE).size().x;
    if width(text) <= max {
        return text.to_string();
    }
    let mut s: Vec<char> = text.chars().collect();
    while !s.is_empty() && width(&format!("{}…", s.iter().collect::<String>())) > max {
        s.pop();
    }
    format!("{}…", s.iter().collect::<String>())
}

// ---------------------------------------------------------------- shapes

/// Vertical gradient over `rect` through (position 0..1, colour) stops.
pub fn gradient(painter: &egui::Painter, rect: Rect, stops: &[(f32, Color32)]) {
    let mut mesh = Mesh::default();
    for (k, &(t, c)) in stops.iter().enumerate() {
        let y = rect.top() + rect.height() * t;
        mesh.colored_vertex(Pos2::new(rect.left(), y), c);
        mesh.colored_vertex(Pos2::new(rect.right(), y), c);
        if k > 0 {
            let i = 2 * k as u32;
            mesh.add_triangle(i - 2, i - 1, i);
            mesh.add_triangle(i - 1, i, i + 1);
        }
    }
    painter.add(Shape::mesh(mesh));
}

/// A soft round glow: `color` in the middle fading to nothing at `radius`.
pub fn glow(painter: &egui::Painter, center: Pos2, radius: Vec2, color: Color32) {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(center, color);
    let n = 32;
    for k in 0..=n {
        let a = k as f32 / n as f32 * std::f32::consts::TAU;
        mesh.colored_vertex(center + Vec2::new(a.cos() * radius.x, a.sin() * radius.y), Color32::TRANSPARENT);
        if k > 0 {
            mesh.add_triangle(0, k, k + 1);
        }
    }
    painter.add(Shape::mesh(mesh));
}

/// A filled convex polygon shaded per vertex by `color_at`.
fn shaded_polygon(painter: &egui::Painter, points: &[Pos2], center: Pos2, center_color: Color32, color_at: impl Fn(Pos2) -> Color32) {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(center, center_color);
    for (k, &p) in points.iter().enumerate() {
        mesh.colored_vertex(p, color_at(p));
        let next = (k + 1) % points.len();
        mesh.add_triangle(0, k as u32 + 1, next as u32 + 1);
    }
    painter.add(Shape::mesh(mesh));
}

// ---------------------------------------------------------------- sky

/// Evening sky down to `horizon`, with stars, distant hills and the
/// stadium's floodlights. `cam_x` (m) and `scale` (px per m) move the hills a
/// little as the camera moves, so the far background drifts slowly.
pub fn sky(painter: &egui::Painter, rect: Rect, horizon: f32, cam_x: f32, scale: f32) {
    let time = painter.ctx().input(|i| i.time) as f32;
    let h = (horizon - rect.top()).max(1.0);
    let sky = Rect::from_min_max(rect.min, Pos2::new(rect.right(), horizon.min(rect.bottom())));
    // Below the horizon: the dark floor of the arena (the ground usually covers it).
    if horizon < rect.bottom() {
        let floor = Rect::from_min_max(Pos2::new(rect.left(), horizon.max(rect.top())), rect.max);
        gradient(painter, floor, &[(0.0, Color32::from_rgb(38, 30, 56)), (1.0, Color32::from_rgb(10, 10, 20))]);
    }
    gradient(
        painter,
        sky,
        &[
            (0.0, Color32::from_rgb(9, 12, 30)),
            (0.45, Color32::from_rgb(24, 27, 64)),
            (0.8, Color32::from_rgb(66, 48, 94)),
            (1.0, Color32::from_rgb(150, 86, 92)),
        ],
    );
    // A thin strip (many race lanes) gets the colours only.
    if h < 90.0 {
        return;
    }
    // Stars, twinkling, in the upper part.
    let n = (rect.width() * h / 5000.0) as i32;
    for i in 0..n.min(120) {
        let x = rect.left() + hash(i, 1, 21) * rect.width();
        let y = rect.top() + hash(i, 2, 21).powf(1.6) * h * 0.6;
        let tw = 0.55 + 0.45 * (time * (1.0 + 2.0 * hash(i, 3, 21)) + 6.0 * hash(i, 4, 21)).sin();
        let r = 0.5 + 1.0 * hash(i, 5, 21);
        painter.circle_filled(Pos2::new(x, y), r, Color32::from_white_alpha((90.0 + 140.0 * tw * hash(i, 6, 21)) as u8));
    }
    // Floodlights at both sides.
    for side in [0.08f32, 0.92] {
        let pole_x = rect.left() + rect.width() * side;
        let top = rect.top() + h * 0.18;
        glow(painter, Pos2::new(pole_x, top), Vec2::new(rect.width() * 0.22, h * 0.45), Color32::from_rgba_unmultiplied(255, 236, 190, 40));
        painter.line_segment([Pos2::new(pole_x, top), Pos2::new(pole_x, horizon)], Stroke::new(3.0, Color32::from_rgb(20, 20, 34)));
        for k in 0..4 {
            for j in 0..2 {
                let p = Pos2::new(pole_x - 12.0 + 8.0 * k as f32, top - 6.0 + 7.0 * j as f32);
                painter.circle_filled(p, 2.6, Color32::from_rgb(255, 246, 220));
            }
        }
        glow(painter, Pos2::new(pole_x, top), Vec2::splat(26.0), Color32::from_rgba_unmultiplied(255, 240, 200, 120));
    }
    // Two layers of far hills, the nearer one moving a bit more.
    for (layer, (k, height, color)) in [(0.03f32, 0.22f32, Color32::from_rgb(40, 34, 70)), (0.08, 0.13, Color32::from_rgb(28, 26, 50))].into_iter().enumerate() {
        let shift = cam_x * scale * k;
        let mut mesh = Mesh::default();
        let steps = (rect.width() / 8.0).ceil() as usize + 1;
        for s in 0..=steps {
            let x = rect.left() + s as f32 * 8.0;
            let u = (x + shift) / 140.0;
            let y = horizon - h * height * (0.35 + 0.65 * (0.65 * noise1(u, 40 + layer as u32) + 0.35 * noise1(u * 3.0, 50 + layer as u32)));
            mesh.colored_vertex(Pos2::new(x, y), color);
            mesh.colored_vertex(Pos2::new(x, horizon + 1.0), color);
            if s > 0 {
                let i = 2 * s as u32;
                mesh.add_triangle(i - 2, i - 1, i);
                mesh.add_triangle(i - 1, i, i + 1);
            }
        }
        painter.add(Shape::mesh(mesh));
    }
}

// ---------------------------------------------------------------- ground

/// Height of a terrain polyline at `x` (0 outside it).
pub fn height_at(terrain: &[[f32; 2]], x: f32) -> f32 {
    let i = terrain.partition_point(|p| p[0] < x);
    match (i.checked_sub(1).map(|k| terrain[k]), terrain.get(i)) {
        (Some(a), Some(b)) if b[0] > a[0] => a[1] + (b[1] - a[1]) * (x - a[0]) / (b[0] - a[0]),
        _ => 0.0,
    }
}

/// Columns from `surface` (world points, left to right) down to the given
/// depths below it, as one mesh: vertex k*(depths) + j is point k at depth j.
fn columns(mut mesh: Mesh, textured: bool, surface: &[[f32; 2]], to_screen: &impl Fn([f32; 2]) -> Pos2, depths: &[(f32, Color32)]) -> Mesh {
    let d = depths.len() as u32;
    for (k, p) in surface.iter().enumerate() {
        for &(depth, color) in depths {
            let q = [p[0], p[1] - depth];
            let uv = if textured { Pos2::new(q[0] / TILE, -q[1] / TILE) } else { egui::epaint::WHITE_UV };
            mesh.vertices.push(Vertex { pos: to_screen(q), uv, color });
        }
        if k > 0 {
            for j in 0..d - 1 {
                let (a, b) = ((k as u32 - 1) * d + j, k as u32 * d + j);
                mesh.add_triangle(a, a + 1, b);
                mesh.add_triangle(a + 1, b, b + 1);
            }
        }
    }
    mesh
}

/// Textured earth under a surface (world points, left to right), darker
/// deeper down and reaching far below, with a grass edge on top. Works for a
/// tilted view too (`to_screen` may rotate).
pub fn earth(painter: &egui::Painter, surface: &[[f32; 2]], to_screen: &impl Fn([f32; 2]) -> Pos2, scale: f32, grass: bool) {
    if surface.len() < 2 {
        return;
    }
    let tex = textures(painter.ctx());
    let depths = [(0.0, Color32::WHITE), (0.7, Color32::from_gray(150)), (60.0, Color32::from_gray(110))];
    painter.add(Shape::mesh(columns(Mesh::with_texture(tex.soil.id()), true, surface, to_screen, &depths)));
    if !grass {
        return;
    }
    // Grass: a band along the surface, a bright rim, and tufts when close up.
    let band = (0.06 * scale).clamp(3.0, 9.0) / scale;
    painter.add(Shape::mesh(columns(Mesh::default(), false, surface, to_screen, &[(0.0, GRASS), (band, with_alpha(GRASS, 0))])));
    let rim: Vec<Pos2> = surface.iter().map(|p| to_screen(*p)).collect();
    painter.add(Shape::line(rim, Stroke::new((0.025 * scale).clamp(1.5, 3.5), GRASS_TOP)));
    if scale > 30.0 {
        let (x0, x1) = (surface[0][0], surface[surface.len() - 1][0]);
        let step = 0.07;
        let first = (x0 / step).floor() as i32;
        for k in first..=(x1 / step).ceil() as i32 {
            let x = k as f32 * step + 0.03 * hash(k, 0, 77);
            // Short blades only (a few cm): the ground the physics uses stays the visible line.
            let y = height_at(surface, x);
            let len = (0.012 + 0.025 * hash(k, 1, 77)) * scale.min(150.0);
            let lean = (hash(k, 2, 77) - 0.5) * len * 0.8;
            let base = to_screen([x, y]);
            painter.line_segment([base, base + Vec2::new(lean, -len)], Stroke::new(1.2, mix(GRASS, GRASS_TOP, hash(k, 3, 77))));
        }
    }
}

/// Distance marks along a race track.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Marks {
    /// None: a competition shows no distances.
    None,
    /// A stake every metre (or 5 / 10 m).
    Stakes,
    /// Stakes with their distance written under them.
    Labels,
}

/// A race track seen from the side: sky, hills of earth with grass, distance
/// marks and the start flag. `ground_y` is the screen y of height 0.
#[allow(clippy::too_many_arguments)]
pub fn track(painter: &egui::Painter, lane: Rect, ground_y: f32, to_screen: &impl Fn([f32; 2]) -> Pos2, (x0, x1): (f32, f32), scale: f32, terrain: &[[f32; 2]], marks: Marks) {
    let painter = &painter.with_clip_rect(lane.intersect(painter.clip_rect()));
    let highest = terrain.iter().filter(|p| p[0] >= x0 && p[0] <= x1).map(|p| p[1]).fold(0.0f32, f32::max);
    sky(painter, lane, ground_y - highest * scale * 0.5, (x0 + x1) / 2.0, scale);
    // The surface, sampled every few pixels.
    let step = (3.0 / scale).max(0.02);
    let n = ((x1 - x0) / step).ceil() as usize + 3;
    let surface: Vec<[f32; 2]> = (0..n)
        .map(|i| {
            let x = x0 - step + i as f32 * step;
            [x, height_at(terrain, x)]
        })
        .collect();
    earth(painter, &surface, to_screen, scale, true);
    // Distance marks: a small stake every metre (or 5 / 10 m), labelled.
    let label_every = if scale > 40.0 { 1 } else if scale > 15.0 { 5 } else { 10 };
    for m in (x0.floor() as i32)..=(x1.ceil() as i32) {
        if marks == Marks::None || m == 0 || m % label_every != 0 {
            continue;
        }
        let top = to_screen([m as f32, height_at(terrain, m as f32)]);
        painter.line_segment([top + Vec2::new(0.0, 2.0), top + Vec2::new(0.0, 9.0)], Stroke::new(1.5, Color32::from_white_alpha(150)));
        if marks == Marks::Labels {
            shadow_text(painter, top + Vec2::new(0.0, 11.0), Align2::CENTER_TOP, format!("{m} m"), FontId::proportional(11.0), Color32::from_gray(225));
        }
    }
    start_flag(painter, to_screen, scale);
}

/// A chequered flag on a pole at the start line (x = 0).
fn start_flag(painter: &egui::Painter, to_screen: &impl Fn([f32; 2]) -> Pos2, scale: f32) {
    let foot = to_screen([0.0, 0.0]);
    let pole = (0.9 * scale).clamp(24.0, 70.0);
    let top = foot - Vec2::new(0.0, pole);
    painter.line_segment([foot, top], Stroke::new(2.0, Color32::from_gray(230)));
    let cell = (pole * 0.12).max(3.0);
    let wave = painter.ctx().input(|i| i.time) as f32 * 4.0;
    for i in 0..4 {
        for j in 0..3 {
            let dy = (wave + i as f32 * 0.9).sin() * cell * 0.25 * i as f32 / 3.0;
            let r = Rect::from_min_size(top + Vec2::new(1.0 + i as f32 * cell, j as f32 * cell + dy), Vec2::splat(cell));
            painter.rect_filled(r, 0.0, if (i + j) % 2 == 0 { Color32::WHITE } else { Color32::from_gray(20) });
        }
    }
    // The start line on the ground.
    for j in 0..3 {
        let r = Rect::from_min_size(foot + Vec2::new(-cell * 0.5, cell * 0.4 * j as f32), Vec2::new(cell, cell * 0.4));
        painter.rect_filled(r, 0.0, if j % 2 == 0 { Color32::from_white_alpha(200) } else { Color32::from_black_alpha(150) });
    }
}

// ---------------------------------------------------------------- sumo ring

/// The sumo platform: a block of clay with its hills on top (`terrain`, the
/// arena's), its edge bound with straw, floating above the dark. Call after the sky.
pub fn ring(painter: &egui::Painter, rules: &Rules, terrain: &[[f32; 2]], to_screen: &impl Fn([f32; 2]) -> Pos2, scale: f32) {
    let half = rules.ring_width as f32 / 2.0;
    let tex = textures(painter.ctx());
    let step = (3.0 / scale).max(0.02);
    let n = ((2.0 * half) / step).ceil() as usize;
    let surface: Vec<[f32; 2]> = (0..=n)
        .map(|i| {
            let x = -half + 2.0 * half * i as f32 / n as f32;
            [x, height_at(terrain, x)]
        })
        .collect();
    // The block: clay, darker towards the bottom, its underside in shadow.
    let bottom = to_screen([0.0, -1.0]).y;
    let mut mesh = Mesh::with_texture(tex.clay.id());
    for (k, p) in surface.iter().enumerate() {
        let top = to_screen(*p);
        mesh.vertices.push(Vertex { pos: top, uv: Pos2::new(p[0] / TILE, -p[1] / TILE), color: Color32::from_gray(255) });
        mesh.vertices.push(Vertex { pos: Pos2::new(top.x, bottom), uv: Pos2::new(p[0] / TILE, 1.0 / TILE), color: Color32::from_gray(120) });
        if k > 0 {
            let i = 2 * k as u32;
            mesh.add_triangle(i - 2, i - 1, i);
            mesh.add_triangle(i - 1, i, i + 1);
        }
    }
    painter.add(Shape::mesh(mesh));
    // Straw rope just under the top surface, with twisted strands.
    let rope = (0.09 * scale).max(4.0);
    let rim: Vec<Pos2> = surface.iter().map(|p| to_screen(*p) + Vec2::new(0.0, rope * 0.6)).collect();
    painter.add(Shape::line(rim.clone(), Stroke::new(rope, STRAW)));
    painter.add(Shape::line(rim.iter().map(|p| *p - Vec2::new(0.0, rope * 0.3)).collect(), Stroke::new(rope * 0.25, Color32::from_rgb(240, 214, 150))));
    let strands = ((2.0 * half * scale) / (rope * 0.9)) as usize;
    for k in 0..strands {
        let x = -half + (k as f32 + 0.5) * 2.0 * half / strands as f32;
        let p = to_screen([x, height_at(terrain, x)]) + Vec2::new(0.0, rope * 0.6);
        painter.line_segment([p + Vec2::new(-rope * 0.25, -rope * 0.45), p + Vec2::new(rope * 0.25, rope * 0.45)], Stroke::new(1.0, Color32::from_rgb(150, 116, 52)));
    }
    // The straw bales at both edges, set into the clay (nothing sticks up
    // above the surface the physics uses).
    for side in [-1.0f32, 1.0] {
        let edge = to_screen([side * half, height_at(terrain, side * half)]);
        let r = (0.13 * scale).max(5.0);
        let c = edge + Vec2::new(-side * r * 0.6, r);
        painter.add(Shape::ellipse_filled(c, Vec2::new(r * 0.75, r), shade(STRAW, 0.9)));
        painter.add(Shape::ellipse_stroke(c, Vec2::new(r * 0.75, r), Stroke::new(1.2, Color32::from_rgb(140, 104, 46))));
        painter.line_segment([c - Vec2::new(0.0, r), c + Vec2::new(0.0, r)], Stroke::new(1.2, Color32::from_rgb(140, 104, 46)));
    }
    // The top surface: sand-coloured rim light, and the start marks.
    let lit: Vec<Pos2> = surface.iter().map(|p| to_screen(*p)).collect();
    painter.add(Shape::line(lit, Stroke::new(1.6, Color32::from_rgb(240, 214, 168))));
    for x in [-(rules.sumo_start as f32) + 0.9, rules.sumo_start as f32 - 0.9] {
        let p = to_screen([x, height_at(terrain, x)]);
        painter.line_segment([p, p + Vec2::new(0.0, 4.0)], Stroke::new(3.0, Color32::from_white_alpha(220)));
    }
    // Shadow under the platform.
    let under = Rect::from_min_max(to_screen([-half, -1.0]), to_screen([half, -1.0]) + Vec2::new(0.0, (0.25 * scale).max(6.0)));
    gradient(painter, under, &[(0.0, Color32::from_black_alpha(120)), (1.0, Color32::TRANSPARENT)]);
}

/// A sumo bout as the audience sees it: the stands behind, the platform in a
/// spotlight on the arena floor, the creatures, a marker over each one and
/// the names at the sides (above the creatures they would overlap).
pub fn sumo_view(painter: &egui::Painter, rect: Rect, arena: &Arena, colors: &[Color32; 2], names: &[String; 2], excitement: f32) {
    let rules = &arena.rules;
    let ring = rules.ring_width as f32;
    let scale = (rect.width() / (ring + 3.0)).min(rect.height() / 3.4);
    let ground_y = rect.top() + rect.height() * 0.7;
    let to_screen = |p: [f32; 2]| Pos2::new(rect.center().x + p[0] * scale, ground_y - p[1] * scale);
    let stands = Rect::from_min_max(Pos2::new(rect.left(), rect.top() + rect.height() * 0.2), Pos2::new(rect.right(), rect.top() + rect.height() * 0.5));
    sky(painter, rect, stands.bottom(), 0.0, scale);
    crowd(painter, stands, excitement);
    glow(painter, to_screen([0.0, 0.5]), Vec2::new(ring * 0.6 * scale, 1.6 * scale), Color32::from_rgba_unmultiplied(255, 240, 200, 28));
    self::ring(painter, rules, &arena.terrain, &to_screen, scale);
    creatures(painter, arena, colors, &to_screen, scale, 255);
    for k in 0..2 {
        let c = arena.com(k);
        if c[0].is_finite() {
            let tip = to_screen([c[0] as f32, c[1] as f32 + 0.65]);
            let s = 7.0;
            painter.add(Shape::convex_polygon(vec![tip, tip + Vec2::new(-s, -s * 1.4), tip + Vec2::new(s, -s * 1.4)], colors[k], Stroke::new(1.0, Color32::from_black_alpha(160))));
        }
        let room = rect.width() / 2.0 - 60.0;
        let font = fitting_font(painter, &names[k], 20.0, room);
        let y = stands.bottom() + 14.0;
        let (chip, align, text_at) = if k == 0 {
            let chip = Rect::from_min_size(Pos2::new(rect.left() + 16.0, y), Vec2::splat(16.0));
            (chip, Align2::LEFT_CENTER, chip.right_center() + Vec2::new(8.0, 0.0))
        } else {
            let chip = Rect::from_min_size(Pos2::new(rect.right() - 32.0, y), Vec2::splat(16.0));
            (chip, Align2::RIGHT_CENTER, chip.left_center() - Vec2::new(8.0, 0.0))
        };
        painter.rect_filled(chip, 4.0, colors[k]);
        let name = fit(painter, &names[k], &font, room);
        shadow_text(painter, text_at, align, name, font, Color32::WHITE);
    }
}

// ---------------------------------------------------------------- creatures

/// Ground height under `x` for a creature's shadow (None: no ground there).
fn ground_under(arena: &Arena, x: f32) -> Option<f32> {
    match arena.stage {
        Stage::Race => Some(height_at(&arena.terrain, x)),
        Stage::Sumo => (x.abs() <= arena.rules.ring_width as f32 / 2.0).then(|| height_at(&arena.terrain, x)),
    }
}

/// The creatures of an arena: rounded, lit from above, with an outline, a
/// shadow on the ground and an eye on the torso. `alpha` < 255 draws faint
/// ghosts (without shadows).
pub fn creatures(painter: &egui::Painter, arena: &Arena, colors: &[Color32], to_screen: &impl Fn([f32; 2]) -> Pos2, scale: f32, alpha: u8) {
    let segs = arena.segments();
    if alpha == 255 {
        for k in 0..arena.fighters.len() {
            let mine: Vec<_> = segs.iter().filter(|s| s.fighter == k).collect();
            let (lo, hi) = mine.iter().fold((f32::MAX, f32::MIN), |(l, h), s| (l.min(s.center[0] - s.half[0]), h.max(s.center[0] + s.half[0])));
            let c = arena.com(k);
            let (cx, cy) = (c[0] as f32, c[1] as f32);
            if !cx.is_finite() {
                continue;
            }
            if let Some(g) = ground_under(arena, cx) {
                let above = (cy - g).max(0.0);
                let fade = (1.0 - above / 2.5).clamp(0.0, 1.0);
                if fade > 0.0 {
                    let w = ((hi - lo) * 0.55 + 0.1) * scale * (1.0 + above * 0.3);
                    glow(painter, to_screen([cx, g]), Vec2::new(w, (0.07 * scale).max(3.0)), Color32::from_black_alpha((150.0 * fade) as u8));
                }
            }
        }
    }
    let mut first_of = vec![true; arena.fighters.len()];
    for s in &segs {
        let base = with_alpha(colors[s.fighter.min(colors.len() - 1)], alpha);
        let (sin, cos) = s.angle.sin_cos();
        let world = |u: f32, v: f32| [s.center[0] + u * cos - v * sin, s.center[1] + u * sin + v * cos];
        // A rounded box: three points per corner arc.
        let (hx, hy) = (s.half[0], s.half[1]);
        let r = hx.min(hy) * 0.55;
        let mut outline = Vec::with_capacity(16);
        for (k, (sx, sy)) in [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)].into_iter().enumerate() {
            let (cx, cy) = (sx * (hx - r), sy * (hy - r));
            for j in 0..4 {
                let a = (k as f32 + j as f32 / 3.0) * std::f32::consts::FRAC_PI_2;
                outline.push(world(cx + r * a.cos(), cy + r * a.sin()));
            }
        }
        let pts: Vec<Pos2> = outline.iter().map(|p| to_screen(*p)).collect();
        let reach = hx.max(hy);
        let top_lit = shade(base, 1.3);
        let bottom_dark = shade(base, 0.62);
        shaded_polygon(painter, &pts, to_screen(s.center), shade(base, 1.08), |p| {
            // Screen y grows downwards: higher points are lighter.
            let up = (to_screen(s.center).y - p.y) / (reach * scale).max(1.0);
            mix(bottom_dark, top_lit, 0.5 + up * 0.6)
        });
        painter.add(Shape::closed_line(pts, Stroke::new(1.2, with_alpha(shade(base, 0.4), alpha))));
        // An eye on the torso, looking forward.
        if std::mem::take(&mut first_of[s.fighter]) {
            let rr = (hy * 0.5 * scale).max(2.5);
            let eye = to_screen(world(hx * 0.55, hy * 0.15));
            let forward = (to_screen(world(hx, hy * 0.15)) - eye).normalized();
            painter.circle_filled(eye, rr * 1.12, with_alpha(Color32::from_gray(30), alpha));
            painter.circle_filled(eye, rr, with_alpha(Color32::WHITE, alpha));
            painter.circle_filled(eye + forward * rr * 0.3, rr * 0.55, with_alpha(Color32::from_gray(15), alpha));
            painter.circle_filled(eye + forward * rr * 0.3 + Vec2::new(-rr * 0.2, -rr * 0.25), rr * 0.18, with_alpha(Color32::WHITE, alpha));
        }
    }
}

// ---------------------------------------------------------------- stadium

/// Rows of spectators in the stands, bobbing; more and higher when
/// `excitement` (0..1) is up, and cheering with their arms up near 1.
pub fn crowd(painter: &egui::Painter, area: Rect, excitement: f32) {
    let time = painter.ctx().input(|i| i.time) as f32;
    gradient(painter, area, &[(0.0, Color32::from_rgb(22, 22, 40)), (1.0, Color32::from_rgb(40, 36, 62))]);
    let rows = 4;
    let row_h = area.height() / rows as f32;
    let shirts = [
        Color32::from_rgb(200, 70, 70),
        Color32::from_rgb(70, 120, 210),
        Color32::from_rgb(230, 190, 70),
        Color32::from_rgb(80, 170, 110),
        Color32::from_rgb(170, 90, 190),
        Color32::from_rgb(230, 130, 60),
        Color32::from_rgb(210, 210, 220),
    ];
    let skins = [Color32::from_rgb(236, 196, 160), Color32::from_rgb(198, 150, 110), Color32::from_rgb(140, 98, 70), Color32::from_rgb(96, 66, 50)];
    for row in 0..rows {
        // Back rows are smaller and darker (further away and less lit).
        let depth = row as f32 / (rows - 1) as f32;
        let size = row_h * (0.42 + 0.18 * depth);
        let light = 0.45 + 0.4 * depth;
        let y = area.top() + row_h * (row as f32 + 0.85);
        let spacing = size * 1.05;
        let count = (area.width() / spacing) as i32 + 2;
        for i in 0..count {
            let seed = (row * 1000 + i) as i32;
            let x = area.left() + (i as f32 + 0.5 * (row % 2) as f32) * spacing + (hash(seed, 1, 5) - 0.5) * spacing * 0.3;
            let jump = (time * (3.0 + 2.5 * hash(seed, 2, 5)) + 6.3 * hash(seed, 3, 5)).sin().max(0.0);
            let lift = jump * size * (0.04 + 0.3 * excitement * hash(seed, 4, 5));
            let shirt = shade(shirts[(hash(seed, 5, 5) * shirts.len() as f32) as usize % shirts.len()], light);
            let skin = shade(skins[(hash(seed, 6, 5) * skins.len() as f32) as usize % skins.len()], light);
            let body = Rect::from_center_size(Pos2::new(x, y - size * 0.25 - lift), Vec2::new(size * 0.62, size * 0.62));
            if excitement > 0.55 && hash(seed, 7, 5) < excitement - 0.3 {
                let wave = (time * 6.0 + seed as f32).sin() * size * 0.08;
                for side in [-1.0f32, 1.0] {
                    let shoulder = body.center_top() + Vec2::new(side * size * 0.22, size * 0.08);
                    painter.line_segment([shoulder, shoulder + Vec2::new(side * size * 0.18 + wave, -size * 0.5)], Stroke::new(size * 0.13, shirt));
                }
            }
            painter.rect_filled(body, size * 0.25, shirt);
            painter.circle_filled(body.center_top() - Vec2::new(0.0, size * 0.2), size * 0.22, skin);
        }
    }
    // The barrier in front of the stands, with banners.
    let rail = Rect::from_min_max(Pos2::new(area.left(), area.bottom() - row_h * 0.35), area.max);
    gradient(painter, rail, &[(0.0, Color32::from_rgb(70, 70, 96)), (1.0, Color32::from_rgb(34, 34, 52))]);
    painter.line_segment([rail.left_top(), rail.right_top()], Stroke::new(2.0, Color32::from_rgb(150, 150, 190)));
    let banners = [Color32::from_rgb(200, 60, 60), Color32::from_rgb(60, 110, 200), Color32::from_rgb(220, 180, 60)];
    let n = (area.width() / 240.0).ceil() as i32;
    for k in 0..n {
        let x = area.left() + (k as f32 + 0.5) * area.width() / n as f32;
        let b = Rect::from_center_size(Pos2::new(x, rail.center().y), Vec2::new(90.0f32.min(area.width() / n as f32 * 0.6), rail.height() * 0.8));
        painter.rect_filled(b, 2.0, banners[k as usize % banners.len()]);
        painter.text(b.center(), Align2::CENTER_CENTER, "CPG ARENA", display(b.height() * 0.6), Color32::WHITE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn textures_tile_without_seams() {
        for img in [soil_image(), clay_image()] {
            assert_eq!(img.size, [TEX, TEX]);
            // Left and right edges (and top and bottom) continue into each other.
            let at = |x: usize, y: usize| img.pixels[y * TEX + x];
            let diff = |a: Color32, b: Color32| (a.r() as i32 - b.r() as i32).abs() + (a.g() as i32 - b.g() as i32).abs();
            let seam: i32 = (0..TEX).map(|y| diff(at(0, y), at(TEX - 1, y))).sum::<i32>() + (0..TEX).map(|x| diff(at(x, 0), at(x, TEX - 1))).sum::<i32>();
            let inside: i32 = (0..TEX).map(|y| diff(at(60, y), at(61, y))).sum::<i32>() + (0..TEX).map(|x| diff(at(x, 60), at(x, 61))).sum::<i32>();
            assert!(seam < inside * 3 + 600, "seam {seam} vs inside {inside}");
        }
    }

    #[test]
    fn terrain_height_is_interpolated() {
        let t = [[0.0, 0.0], [1.0, 1.0], [2.0, 0.0]];
        assert_eq!(height_at(&t, 0.5), 0.5);
        assert_eq!(height_at(&t, 1.5), 0.5);
        assert_eq!(height_at(&t, -3.0), 0.0);
        assert_eq!(height_at(&t, 5.0), 0.0);
        assert_eq!(height_at(&[], 1.0), 0.0);
    }
}
