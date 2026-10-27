//! Race track environments. An environment comes from one number, its seed:
//! the seed decides the hills, the terrain, the slope and the friction, and the
//! same seed always gives the same track. The Race and Train tabs choose
//! between the folder's own track (its arena.toml, else flat) and an
//! environment seed, and show the track they will use.

use super::scene;
use cpg_arena::creature::Rules;
use cpg_arena::sim::{hash01, ring_surface, terrain_height};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Stroke};

/// How much of the track a preview shows (m).
const PREVIEW: (f64, f64) = (-1.0, 20.0);

/// The track settings of the rules: hills, which terrain, slope, friction.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Track {
    roughness: f64,
    seed: u64,
    slope: f64,
    friction: f64,
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

impl Track {
    pub fn of(r: &Rules) -> Self {
        Self { roughness: r.terrain_roughness, seed: r.terrain_seed, slope: r.slope, friction: r.friction }
    }

    /// `r` with this track.
    pub fn apply(self, r: &Rules) -> Rules {
        Rules { terrain_roughness: self.roughness, terrain_seed: self.seed, slope: self.slope, friction: self.friction, ..r.clone() }
    }

    /// The environment generated from one number. The same seed always gives
    /// the same track, so a whole class can race on "environment 4711".
    pub fn random(seed: u64) -> Self {
        let u = |k: i64| hash01(k, seed ^ 0xE4F1_2A0D);
        let snap = |v: f64, step: f64| round3((v / step).round() * step);
        Self {
            roughness: snap(0.1 + 0.5 * u(1), 0.05), // 0.1 - 0.6 m of hills
            seed,                                    // which terrain
            slope: snap(-5.0 + 13.0 * u(2), 0.5),    // -5 to +8 degrees, mostly uphill
            friction: snap(0.5 + 0.7 * u(3), 0.05),  // 0.5 (slippery) - 1.2 (grippy)
        }
    }

    pub fn describe(self) -> String {
        let slope = match self.slope {
            s if s > 0.0 => format!("slope {s:+.1}° (uphill)"),
            s if s < 0.0 => format!("slope {s:+.1}° (downhill)"),
            _ => "no slope".into(),
        };
        let hills = if self.roughness > 0.0 { format!("hills {:.2} m (terrain {})", self.roughness, self.seed) } else { "no hills".into() };
        format!("{hills} · {slope} · friction {:.2}", self.friction)
    }

    /// The same environment as a sumo platform: the hills, mirrored so both
    /// sides are equal, and the friction (no slope: it would favour one side).
    pub fn describe_ring(self) -> String {
        let hills = if self.roughness > 0.0 {
            format!("platform hills {:.2} m (terrain {}, the same on both sides)", self.roughness, self.seed)
        } else {
            "a flat platform".into()
        };
        format!("{hills} · friction {:.2}", self.friction)
    }
}

/// A fresh environment seed, 0 to 9999, from the clock.
pub fn new_seed() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos() as u64 ^ d.as_secs()) % 10_000
}

/// Which track to use: the folder's own, or the environment from a seed. The
/// folder's own is the class track when the folder has an arena.toml (the
/// teacher's rules), and flat ground otherwise.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TrackChoice {
    pub seeded: bool,
    pub seed: u64,
}

impl TrackChoice {
    pub fn new() -> Self {
        Self { seeded: false, seed: new_seed() }
    }

    /// The folder's rules with the chosen track.
    pub fn rules(&self, folder: &Rules) -> Rules {
        if self.seeded { Track::random(self.seed).apply(folder) } else { folder.clone() }
    }

    /// `class`: the folder has an arena.toml.
    pub fn describe(&self, folder: &Rules, class: bool) -> String {
        match (self.seeded, class) {
            (true, _) => format!("Environment {}: {}", self.seed, Track::random(self.seed).describe()),
            (false, true) => format!("Class track (arena.toml): {}", Track::of(folder).describe()),
            (false, false) => format!("Flat track: {}", Track::of(folder).describe()),
        }
    }

    /// Like [`Self::describe`], for sumo.
    pub fn describe_sumo(&self, folder: &Rules, class: bool) -> String {
        match (self.seeded, class) {
            (true, _) => format!("Environment {}: {}", self.seed, Track::random(self.seed).describe_ring()),
            (false, true) => format!("Class arena (arena.toml): {}", Track::of(folder).describe_ring()),
            (false, false) => format!("Sumo: {}", Track::of(folder).describe_ring()),
        }
    }

    /// The track selector: flat or the class track (the folder's own), or an environment seed.
    pub fn ui(&mut self, ui: &mut egui::Ui, class: bool) {
        ui.horizontal(|ui| {
            ui.label("Track");
            let (own, hint) = if class {
                ("class track", "The track in this folder's arena.toml, set by the teacher")
            } else {
                ("flat", "Flat ground: no hills, no slope")
            };
            ui.selectable_value(&mut self.seeded, false, own).on_hover_text(hint);
            ui.selectable_value(&mut self.seeded, true, "environment seed")
                .on_hover_text("Hills, terrain, slope and friction from one number: the same seed always gives the same track");
            if self.seeded {
                ui.add(egui::DragValue::new(&mut self.seed).range(0..=9999));
                if ui.small_button("new").on_hover_text("A new random environment").clicked() {
                    self.seed = new_seed();
                }
            }
        });
    }
}

/// The first 20 m of a race track as the simulator builds it (hills from
/// `rules`' terrain), tilted by the slope and fitted into `inner`. The ground
/// reaches past `inner`; the painter's clip rect cuts it off.
pub fn draw_terrain(painter: &egui::Painter, inner: Rect, rules: &Rules) {
    let t = Track::of(rules);
    let (sin, cos) = t.slope.to_radians().sin_cos();
    let tilt = |x: f64, y: f64| [x * cos - y * sin, x * sin + y * cos];
    // The surface every 10 cm; drawn well past both ends so the tilted ground fills the canvas.
    let (x0, x1) = (PREVIEW.0 - 30.0, PREVIEW.1 + 30.0);
    let surface: Vec<[f64; 2]> = (0..=((x1 - x0) * 10.0) as usize)
        .map(|i| {
            let x = x0 + i as f64 * 0.1;
            [x, terrain_height(x, t.roughness, t.seed)]
        })
        .collect();
    // Fit the previewed stretch of the tilted surface (plus room above it for creatures) into `inner`.
    let shown = surface.iter().filter(|p| p[0] >= PREVIEW.0 && p[0] <= PREVIEW.1);
    let pts: Vec<[f64; 2]> = shown.flat_map(|p| [tilt(p[0], p[1]), tilt(p[0], p[1] + 1.5)]).collect();
    let (lo, hi) = pts.iter().fold(([f64::MAX; 2], [f64::MIN; 2]), |(l, h), p| ([l[0].min(p[0]), l[1].min(p[1])], [h[0].max(p[0]), h[1].max(p[1])]));
    let scale = (inner.width() as f64 / (hi[0] - lo[0])).min(inner.height() as f64 / (hi[1] - lo[1]));
    let mid = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
    let to_screen = |x: f64, y: f64| {
        let p = tilt(x, y);
        Pos2::new(inner.center().x + ((p[0] - mid[0]) * scale) as f32, inner.center().y - ((p[1] - mid[1]) * scale) as f32)
    };
    // The sky behind, then the earth: the surface reaching well below.
    let horizon = to_screen((PREVIEW.0 + PREVIEW.1) / 2.0, 0.0).y;
    scene::sky(painter, painter.clip_rect(), horizon, 0.0, scale as f32);
    let ground: Vec<[f32; 2]> = surface.iter().map(|p| [p[0] as f32, p[1] as f32]).collect();
    scene::earth(painter, &ground, &|p: [f32; 2]| to_screen(p[0] as f64, p[1] as f64), scale as f32, true);
    for m in (0..=PREVIEW.1 as i32).step_by(5) {
        let x = m as f64;
        let top = to_screen(x, terrain_height(x, t.roughness, t.seed));
        painter.line_segment([top, to_screen(x, -0.25)], Stroke::new(1.5, if m == 0 { Color32::WHITE } else { Color32::from_gray(200) }));
        let label = if m == 0 { "start".to_string() } else { format!("{m} m") };
        scene::shadow_text(painter, to_screen(x, -0.3), Align2::CENTER_TOP, label, FontId::proportional(12.0), Color32::from_gray(230));
    }
}

/// The sumo platform of `rules` (with its hills, exactly as the simulator
/// builds them) in front of the stands, fitted into `rect`.
pub fn draw_ring(painter: &egui::Painter, rect: Rect, rules: &Rules) {
    let half = rules.ring_width as f32 / 2.0;
    let scale = (rect.width() / (2.0 * half + 2.0)).min(rect.height() / 2.6);
    let ground_y = rect.top() + rect.height() * 0.62;
    let to_screen = |p: [f32; 2]| Pos2::new(rect.center().x + p[0] * scale, ground_y - p[1] * scale);
    scene::sky(painter, painter.clip_rect(), ground_y, 0.0, scale);
    scene::ring(painter, rules, &ring_surface(rules), &to_screen, scale);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seed_always_gives_the_same_environment() {
        assert_eq!(Track::random(4711), Track::random(4711));
        assert_ne!(Track::random(4711), Track::random(4712));
        for seed in 0..500 {
            let t = Track::random(seed);
            assert_eq!(t.seed, seed);
            assert!((0.1..=0.6).contains(&t.roughness) && (-5.0..=8.0).contains(&t.slope) && (0.5..=1.2).contains(&t.friction), "{t:?}");
        }
        assert!(new_seed() < 10_000);
    }

    #[test]
    fn a_choice_keeps_the_folders_other_rules() {
        let folder = Rules { race_time: 12.0, terrain_roughness: 0.2, ..Rules::default() };
        let mut choice = TrackChoice { seeded: false, seed: 4711 };
        assert_eq!(Track::of(&choice.rules(&folder)), Track::of(&folder));
        choice.seeded = true;
        let r = choice.rules(&folder);
        assert_eq!((Track::of(&r), r.race_time), (Track::random(4711), 12.0));
    }
}
