//! Design tab: build a creature from blocks.
//!
//! Drag a block from the palette onto the creature: it attaches at the
//! nearest point of the nearest segment (snapping to the back end, middle or
//! front end) and points towards the cursor. Click a block to edit its size,
//! position and motor; drag it to turn it around its joint. Try the design in
//! a race or a sumo bout, then save it to the creatures folder, where the Train
//! tab can use it as a template.

use super::{draw_creatures, draw_track, finished};
use cpg_arena::creature::{Brain, Creature, Rules, Segment};
use cpg_arena::game::{self, Entry};
use cpg_arena::sim::{rest_pose, Arena};
use eframe::egui::{self, Align2, Color32, CursorIcon, FontId, Key, Pos2, Rect, RichText, Sense, Shape, Stroke, Vec2};
use std::path::{Path, PathBuf};

const CANVAS: Color32 = Color32::from_rgb(28, 32, 40);
const CANVAS_TEXT: Color32 = Color32::from_gray(165);

/// The palette: (name, length, width) in metres.
const BLOCKS: [(&str, f64, f64); 4] = [("Leg", 0.45, 0.08), ("Short leg", 0.25, 0.08), ("Block", 0.35, 0.18), ("Plate", 0.6, 0.06)];

type Pose = ([f64; 2], f64);

/// What the Design tab asks the rest of the app to do.
pub enum Action {
    /// Open the Train tab with this saved creature as the template.
    Train(PathBuf),
}

/// The design being tried out in a race or a sumo bout.
struct Trial {
    arena: Arena,
    sumo: bool,
    /// Name of the sumo opponent.
    rival: String,
    accumulator: f64,
    /// The design it was started from; editing the design ends the trial.
    design: String,
}

pub struct Designer {
    creature: Creature,
    selected: usize,
    /// Index into BLOCKS of the block being dragged from the palette.
    dragging: Option<usize>,
    /// Turning the selected block by dragging it.
    turning: bool,
    /// Where this design was saved; saving again overwrites it.
    file: Option<PathBuf>,
    /// Last message, and whether it is an error.
    status: Option<(String, bool)>,
    trial: Option<Trial>,
    try_sumo: bool,
    /// Sumo trial opponent: a creature file, or None for the Rock.
    opponent: Option<PathBuf>,
    /// Centre x, centre y (m) and scale (px per m).
    camera: Option<[f32; 3]>,
}

fn blank() -> Creature {
    Creature {
        name: "My Creature".into(),
        author: String::new(),
        color: Some([230, 120, 40]),
        brain: Brain { frequency: 1.0, coupling: 4.0 },
        segments: vec![Segment { length: 0.5, width: 0.15, ..Default::default() }],
        meta: None,
    }
}

/// Degrees wrapped into [-180, 180).
fn wrap(deg: f64) -> f64 {
    (deg + 180.0).rem_euclid(360.0) - 180.0
}

/// Corner points of segment `s` placed at `pose`.
fn corners(s: &Segment, (p, a): Pose) -> [[f64; 2]; 4] {
    [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)].map(|(u, v)| {
        let (lx, ly) = (u * s.length / 2.0, v * s.width / 2.0);
        [p[0] + lx * a.cos() - ly * a.sin(), p[1] + lx * a.sin() + ly * a.cos()]
    })
}

/// Where segment `k` hangs on its parent.
fn joint(c: &Creature, pose: &[Pose], k: usize) -> [f64; 2] {
    let s = &c.segments[k];
    let (pc, pa) = pose[s.parent];
    let d = s.attach * c.segments[s.parent].length / 2.0;
    [pc[0] + d * pa.cos(), pc[1] + d * pa.sin()]
}

/// The topmost segment under `p` (widened by `pad` for easier clicking).
fn pick(c: &Creature, pose: &[Pose], p: [f64; 2], pad: f64) -> Option<usize> {
    (0..c.segments.len()).rev().find(|&i| {
        let (center, a) = pose[i];
        let (dx, dy) = (p[0] - center[0], p[1] - center[1]);
        let (lx, ly) = (dx * a.cos() + dy * a.sin(), -dx * a.sin() + dy * a.cos());
        lx.abs() <= c.segments[i].length / 2.0 + pad && ly.abs() <= c.segments[i].width / 2.0 + pad
    })
}

/// Where a block dropped at `p` attaches: (parent, attach, angle in degrees).
fn attachment(c: &Creature, pose: &[Pose], p: [f64; 2]) -> (usize, f64, f64) {
    let mut best = (0, 0.0, f64::INFINITY);
    for (i, (s, &(center, a))) in c.segments.iter().zip(pose).enumerate() {
        let h = s.length / 2.0;
        let t = (((p[0] - center[0]) * a.cos() + (p[1] - center[1]) * a.sin()) / h).clamp(-1.0, 1.0);
        let q = [center[0] + t * h * a.cos(), center[1] + t * h * a.sin()];
        let d = (p[0] - q[0]).hypot(p[1] - q[1]);
        if d < best.2 {
            best = (i, t, d);
        }
    }
    let (parent, mut t, _) = best;
    for snap in [-1.0, 0.0, 1.0] {
        if (t - snap).abs() < 0.15 {
            t = snap;
        }
    }
    let t = (t * 100.0).round() / 100.0;
    let (center, a) = pose[parent];
    let h = c.segments[parent].length / 2.0;
    let j = [center[0] + t * h * a.cos(), center[1] + t * h * a.sin()];
    let (dx, dy) = (p[0] - j[0], p[1] - j[1]);
    // Point at the cursor, in steps of 5°; straight down when the cursor is on the joint.
    let angle = if dx.hypot(dy) < 0.03 { -90.0 } else { (wrap(dy.atan2(dx).to_degrees() - a.to_degrees()) / 5.0).round() * 5.0 };
    (parent, t, wrap(angle))
}

fn attach_name(t: f64) -> String {
    match t {
        t if t <= -1.0 => "back end".into(),
        t if t == 0.0 => "middle".into(),
        t if t >= 1.0 => "front end".into(),
        t => format!("{t:+.2} along it"),
    }
}

/// Remove segment `k` and everything hanging off it; returns the new index of its parent.
fn remove(c: &mut Creature, k: usize) -> usize {
    let n = c.segments.len();
    let (mut keep, mut index) = (vec![true; n], vec![0; n]);
    let mut next = 0;
    for i in 0..n {
        keep[i] = i != k && (i == 0 || keep[c.segments[i].parent]);
        if keep[i] {
            index[i] = next;
            next += 1;
        }
    }
    let parent = index[c.segments[k].parent];
    let old = std::mem::take(&mut c.segments);
    c.segments = old
        .into_iter()
        .enumerate()
        .filter(|(i, _)| keep[*i])
        .map(|(_, mut s)| {
            s.parent = index[s.parent];
            s
        })
        .collect();
    parent
}

/// File name from the creature's name: "Long Legs 2" → "long-legs-2".
fn slug(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let s = s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    match s.as_str() {
        "" => "my-creature".into(),
        "arena" => "arena-creature".into(), // arena.toml holds the rules
        _ => s,
    }
}

/// `dir/stem.toml`, or `dir/stem-2.toml`, ... when that is taken.
fn unique(dir: &Path, stem: &str) -> PathBuf {
    let mut path = dir.join(format!("{stem}.toml"));
    for k in 2.. {
        if !path.exists() {
            break;
        }
        path = dir.join(format!("{stem}-{k}.toml"));
    }
    path
}

fn slider(ui: &mut egui::Ui, label: &str, value: &mut f64, range: [f64; 2], suffix: &str, step: f64, hint: &str) {
    ui.label(label).on_hover_text(hint);
    ui.add(egui::Slider::new(value, range[0]..=range[1]).suffix(suffix).step_by(step)).on_hover_text(hint);
    ui.end_row();
}

impl Designer {
    pub fn new() -> Self {
        Self {
            creature: blank(),
            selected: 0,
            dragging: None,
            turning: false,
            file: None,
            status: None,
            trial: None,
            try_sumo: false,
            opponent: None,
            camera: None,
        }
    }

    fn start_from(&mut self, c: Creature) {
        *self = Self { creature: c, try_sumo: self.try_sumo, opponent: self.opponent.take(), ..Self::new() };
    }

    fn color(&self) -> Color32 {
        let c = self.creature.color.unwrap_or([230, 120, 40]);
        Color32::from_rgb(c[0], c[1], c[2])
    }

    /// Save to `dir` (the first time under a new file name); returns the file.
    fn save(&mut self, dir: &Path) -> Option<PathBuf> {
        let path = match &self.file {
            Some(p) if p.parent() == Some(dir) => p.clone(),
            _ => unique(dir, &slug(&self.creature.name)),
        };
        let saved = std::fs::create_dir_all(dir).map_err(|e| e.to_string()).and_then(|_| self.creature.save(&path));
        match saved {
            Ok(()) => {
                let file = path.file_name().map_or(String::new(), |f| f.to_string_lossy().into_owned());
                self.status = Some((format!("Saved as {file}"), false));
                self.file = Some(path.clone());
                Some(path)
            }
            Err(e) => {
                self.status = Some((format!("Could not save: {e}"), true));
                None
            }
        }
    }

    fn try_out(&mut self, rules: &Rules, entries: &[Entry]) {
        let c = &self.creature;
        let rival = self
            .opponent
            .as_ref()
            .and_then(|p| entries.iter().find(|e| &e.path == p))
            .and_then(|e| e.creature.as_ref().ok().cloned())
            .unwrap_or_else(game::rock);
        let arena = if self.try_sumo { Arena::sumo(c, &rival, rules) } else { Arena::race(c, rules) };
        self.trial = Some(Trial { arena, sumo: self.try_sumo, rival: rival.name, accumulator: 0.0, design: format!("{c:?}") });
        self.camera = None;
    }

    pub fn panel(&mut self, ui: &mut egui::Ui, rules: &Rules, entries: &[Entry], save_dir: &Path) -> Option<Action> {
        let mut action = None;
        // A palette drag that ended somewhere else.
        if self.dragging.is_some() && !ui.input(|i| i.pointer.any_down() || i.pointer.any_released()) {
            self.dragging = None;
        }
        let err = ui.visuals().error_fg_color;
        egui::ScrollArea::vertical().id_salt("design_panel").show(ui, |ui| {
            ui.heading("Design");
            ui.label("Drag a block onto the creature to attach it. Click a block to change it; drag it to turn it.");
            egui::Grid::new("design_form").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                ui.label("Name");
                ui.add(egui::TextEdit::singleline(&mut self.creature.name).desired_width(170.0));
                ui.end_row();
                ui.label("Author");
                ui.add(egui::TextEdit::singleline(&mut self.creature.author).desired_width(170.0).hint_text("your name"));
                ui.end_row();
                ui.label("Colour");
                let mut rgb = self.creature.color.unwrap_or([230, 120, 40]);
                if ui.color_edit_button_srgb(&mut rgb).changed() {
                    self.creature.color = Some(rgb);
                }
                ui.end_row();
                ui.label("Start from");
                let mut start = None;
                egui::ComboBox::from_id_salt("design_start").width(170.0).selected_text("choose…").show_ui(ui, |ui| {
                    if ui.selectable_label(false, "New: just a torso").clicked() {
                        start = Some(blank());
                    }
                    for c in entries.iter().filter_map(|e| e.creature.as_ref().ok()) {
                        if ui.selectable_label(false, format!("A copy of {}", c.name)).clicked() {
                            start = Some(Creature { name: format!("{} (edited)", c.name), meta: None, ..c.clone() });
                        }
                    }
                });
                if let Some(c) = start {
                    self.start_from(c);
                }
                ui.end_row();
            });

            ui.separator();
            ui.label(RichText::new("Blocks: drag one onto the creature").strong());
            let color = self.color();
            ui.horizontal_wrapped(|ui| {
                for (k, &(name, length, width)) in BLOCKS.iter().enumerate() {
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(66.0, 48.0), Sense::drag());
                    let (bg, frame, ink) = (ui.visuals().extreme_bg_color, ui.visuals().widgets.inactive.bg_stroke, ui.visuals().text_color());
                    let p = ui.painter();
                    p.rect_filled(rect, 4.0, bg);
                    p.rect_stroke(rect, 4.0, frame, egui::StrokeKind::Inside);
                    let block = Rect::from_center_size(rect.center() - Vec2::new(0.0, 7.0), Vec2::new(length as f32 * 50.0, width as f32 * 50.0));
                    p.rect_filled(block, 1.0, color);
                    p.text(rect.center_bottom() - Vec2::new(0.0, 3.0), Align2::CENTER_BOTTOM, name, FontId::proportional(11.0), ink);
                    let resp = resp.on_hover_text(format!("{length} × {width} m. Drag it onto the creature."));
                    if resp.hovered() {
                        ui.ctx().set_cursor_icon(CursorIcon::Grab);
                    }
                    if resp.drag_started() {
                        self.dragging = Some(k);
                        self.trial = None;
                    }
                }
            });

            ui.separator();
            let n = self.creature.segments.len();
            let k = self.selected.min(n - 1);
            self.selected = k;
            let parent = self.creature.segments[k].parent;
            let title = if k == 0 { "Torso (segment 0)".to_string() } else { format!("Segment {k}, hanging on segment {parent}") };
            ui.label(RichText::new(title).strong());
            let s = &mut self.creature.segments[k];
            egui::Grid::new("design_segment").num_columns(2).show(ui, |ui| {
                slider(ui, "Length", &mut s.length, rules.length, " m", 0.01, "Size along the block");
                slider(ui, "Width", &mut s.width, rules.width, " m", 0.01, "Thickness of the block");
                if k > 0 {
                    slider(ui, "Attach", &mut s.attach, [-1.0, 1.0], "", 0.05, "Where on the parent: -1 back end, 0 middle, 1 front end");
                    slider(ui, "Angle", &mut s.angle, rules.angle, "°", 1.0, "Rest angle relative to the parent (you can also drag the block)");
                }
            });
            if k > 0 {
                ui.label(RichText::new("Motor").strong());
                egui::Grid::new("design_motor").num_columns(2).show(ui, |ui| {
                    slider(ui, "Amplitude", &mut s.amplitude, rules.amplitude, "°", 1.0, "How far the joint swings");
                    slider(ui, "Offset", &mut s.offset, rules.offset, "°", 1.0, "Shift of the swing centre");
                    slider(ui, "Phase", &mut s.phase, [-180.0, 180.0], "°", 5.0, "When in the cycle this joint swings; phase differences between joints make the gait");
                });
                ui.label(RichText::new("Joint angle = angle + offset + amplitude · cos(CPG phase)").small().weak());
                if ui.button("Delete this block (and what hangs on it)").clicked() {
                    self.selected = remove(&mut self.creature, k);
                }
            }
            ui.label(RichText::new("Brain (shared by all joints)").strong());
            egui::Grid::new("design_brain").num_columns(2).show(ui, |ui| {
                let b = &mut self.creature.brain;
                slider(ui, "Frequency", &mut b.frequency, rules.frequency, " Hz", 0.05, "Swings per second");
                slider(ui, "Coupling", &mut b.coupling, rules.coupling, "", 0.1, "How strongly neighbouring joints keep in step");
            });

            ui.separator();
            let area = self.creature.area();
            let bar = egui::ProgressBar::new((area / rules.max_area) as f32).text(format!("body area {area:.2} of {:.2} m²", rules.max_area));
            ui.add(if area > rules.max_area { bar.fill(err) } else { bar });
            ui.label(format!("{n} of at most {} segments · bigger bodies are heavier, every joint has the same motor", rules.max_segments));
            let problems = self.creature.validate(rules).err().unwrap_or_default();
            for p in &problems {
                ui.colored_label(err, p);
            }

            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Try it:");
                ui.selectable_value(&mut self.try_sumo, false, "race");
                ui.selectable_value(&mut self.try_sumo, true, "sumo");
                if self.try_sumo {
                    let current = self
                        .opponent
                        .as_ref()
                        .and_then(|p| entries.iter().find(|e| &e.path == p))
                        .and_then(|e| e.creature.as_ref().ok())
                        .map_or("the Rock".to_string(), |c| c.name.clone());
                    egui::ComboBox::from_id_salt("design_opponent").selected_text(format!("vs {current}")).show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.opponent, None, "the Rock");
                        for e in entries {
                            if let Ok(c) = &e.creature {
                                ui.selectable_value(&mut self.opponent, Some(e.path.clone()), c.name.clone());
                            }
                        }
                    });
                }
            });
            ui.horizontal(|ui| {
                let label = if self.trial.is_some() { "■ Stop" } else { "▶ Try it" };
                if ui.add_enabled(problems.is_empty(), egui::Button::new(label)).on_disabled_hover_text("Fix the problems above first").clicked() {
                    if self.trial.is_some() {
                        self.trial = None;
                    } else {
                        self.try_out(rules, entries);
                    }
                }
                let named = !self.creature.name.trim().is_empty();
                if ui.add_enabled(named, egui::Button::new("Save")).on_disabled_hover_text("Give it a name first").clicked() {
                    self.save(save_dir);
                }
                let train = ui.add_enabled(named && problems.is_empty(), egui::Button::new("Train it"));
                if train.on_hover_text("Save it, then evolve it in the Train tab").clicked() {
                    action = self.save(save_dir).map(Action::Train);
                }
            });
            ui.label(RichText::new(format!("Saved in {}", save_dir.display())).small().weak());
            if let Some((msg, is_err)) = &self.status {
                if *is_err {
                    ui.colored_label(err, msg);
                } else {
                    ui.label(msg);
                }
            }
        });
        action
    }

    /// Step a trial run; true while the canvas needs redrawing.
    pub fn advance(&mut self, dt: f64, speed: f32, paused: bool) -> bool {
        let editing = self.dragging.is_some() || self.turning;
        let Some(t) = &mut self.trial else { return editing };
        if t.design != format!("{:?}", self.creature) {
            self.trial = None;
            return editing;
        }
        if paused || finished(&t.arena, t.sumo) {
            return editing;
        }
        t.accumulator += dt * speed as f64;
        let step = t.arena.rules.dt;
        let mut budget = 240; // never freeze the UI
        while t.accumulator >= step && budget > 0 && !finished(&t.arena, t.sumo) {
            t.accumulator -= step;
            budget -= 1;
            t.arena.step();
        }
        true
    }

    pub fn view(&mut self, ui: &mut egui::Ui, rules: &Rules) {
        if self.trial.is_some() {
            self.draw_trial(ui);
            return;
        }
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = resp.rect;
        let painter = painter.with_clip_rect(rect);
        painter.rect_filled(rect, 0.0, CANVAS);

        // Camera: fit the creature, but hold still while something is being dragged.
        let pose = rest_pose(&self.creature);
        let pts: Vec<[f64; 2]> = self.creature.segments.iter().zip(&pose).flat_map(|(s, &p)| corners(s, p)).collect();
        let lo = pts.iter().fold([f64::MAX; 2], |m, p| [m[0].min(p[0]), m[1].min(p[1])]);
        let hi = pts.iter().fold([f64::MIN; 2], |m, p| [m[0].max(p[0]), m[1].max(p[1])]);
        let span = [(hi[0] - lo[0] + 1.6).max(3.0), (hi[1] - lo[1] + 1.2).max(1.8)];
        let fit = ((rect.width() as f64 / span[0]).min(rect.height() as f64 / span[1])).min(400.0) as f32;
        let target = [((lo[0] + hi[0]) / 2.0) as f32, ((lo[1] + hi[1]) / 2.0) as f32, fit];
        let cam = match self.camera {
            Some(c) if self.dragging.is_some() || self.turning => c,
            Some(c) => [c[0] + (target[0] - c[0]) * 0.15, c[1] + (target[1] - c[1]) * 0.15, c[2] + (target[2] - c[2]) * 0.15],
            None => target,
        };
        self.camera = Some(cam);
        let center = rect.center();
        let to_screen = |p: [f64; 2]| Pos2::new(center.x + (p[0] as f32 - cam[0]) * cam[2], center.y - (p[1] as f32 - cam[1]) * cam[2]);
        let to_world = |q: Pos2| [((q.x - center.x) / cam[2] + cam[0]) as f64, (-(q.y - center.y) / cam[2] + cam[1]) as f64];
        let pad = 4.0 / cam[2] as f64;

        // Select, turn, delete.
        if resp.clicked() {
            if let Some(k) = resp.interact_pointer_pos().and_then(|p| pick(&self.creature, &pose, to_world(p), pad)) {
                self.selected = k;
            }
        }
        if resp.drag_started() {
            if let Some(k) = ui.input(|i| i.pointer.press_origin()).and_then(|p| pick(&self.creature, &pose, to_world(p), pad)) {
                self.selected = k;
                self.turning = k > 0;
            }
        }
        if self.turning {
            if let Some(p) = resp.interact_pointer_pos() {
                let k = self.selected;
                let j = joint(&self.creature, &pose, k);
                let w = to_world(p);
                let parent_angle = pose[self.creature.segments[k].parent].1;
                self.creature.segments[k].angle = wrap((w[1] - j[1]).atan2(w[0] - j[0]).to_degrees() - parent_angle.to_degrees()).round();
            }
            if !ui.input(|i| i.pointer.any_down()) {
                self.turning = false;
            }
        }
        if resp.hovered() && self.selected > 0 && !ui.ctx().egui_wants_keyboard_input() && ui.input(|i| i.key_pressed(Key::Delete)) {
            self.selected = remove(&mut self.creature, self.selected);
        }

        // Drop a block from the palette.
        let mut ghost = None;
        if let Some(b) = self.dragging {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            if let Some(p) = ui.input(|i| i.pointer.latest_pos()).filter(|p| rect.contains(*p)) {
                let (parent, attach, angle) = attachment(&self.creature, &pose, to_world(p));
                let (_, length, width) = BLOCKS[b];
                ghost = Some(Segment {
                    parent,
                    attach,
                    angle,
                    length: length.clamp(rules.length[0], rules.length[1]),
                    width: width.clamp(rules.width[0], rules.width[1]),
                    amplitude: 30.0f64.clamp(rules.amplitude[0], rules.amplitude[1]),
                    offset: 0.0,
                    // A quarter cycle behind its parent: a travelling wave to start from.
                    phase: wrap(self.creature.segments[parent].phase + 90.0),
                });
            }
            if ui.input(|i| i.pointer.any_released()) {
                if let Some(s) = ghost.take() {
                    if self.creature.segments.len() < rules.max_segments {
                        self.creature.segments.push(s);
                        self.selected = self.creature.segments.len() - 1;
                    } else {
                        self.status = Some((format!("At most {} segments: delete one first", rules.max_segments), true));
                    }
                }
                self.dragging = None;
            }
        }

        // Grid (10 cm) and ground, under the creature as it would stand in the arena.
        let c = &self.creature;
        let pose = rest_pose(c);
        let (w0, w1) = (to_world(rect.left_bottom()), to_world(rect.right_top()));
        for i in (w0[0] / 0.1).floor() as i64..=(w1[0] / 0.1).ceil() as i64 {
            let x = to_screen([i as f64 * 0.1, 0.0]).x;
            let g = if i % 5 == 0 { 52 } else { 38 };
            painter.line_segment([Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())], Stroke::new(1.0, Color32::from_gray(g)));
        }
        for i in (w0[1] / 0.1).floor() as i64..=(w1[1] / 0.1).ceil() as i64 {
            let y = to_screen([0.0, i as f64 * 0.1]).y;
            let g = if i % 5 == 0 { 52 } else { 38 };
            painter.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)], Stroke::new(1.0, Color32::from_gray(g)));
        }
        let min_y = c.segments.iter().zip(&pose).flat_map(|(s, &p)| corners(s, p)).map(|q| q[1]).fold(f64::MAX, f64::min);
        let ground = to_screen([0.0, min_y - 0.02]).y;
        painter.rect_filled(Rect::from_min_max(Pos2::new(rect.left(), ground), rect.right_bottom()), 0.0, Color32::from_rgba_unmultiplied(110, 90, 65, 170));

        // The creature, the selected block outlined, joints as dots, numbers as in the panel.
        let color = self.color();
        for (i, (s, &p)) in c.segments.iter().zip(&pose).enumerate() {
            let outline = if i == self.selected { Stroke::new(2.5, Color32::WHITE) } else { Stroke::new(1.0, Color32::from_black_alpha(160)) };
            painter.add(Shape::convex_polygon(corners(s, p).iter().map(|&q| to_screen(q)).collect(), color, outline));
            painter.text(to_screen(p.0), Align2::CENTER_CENTER, i.to_string(), FontId::proportional(11.0), Color32::from_black_alpha(200));
        }
        let (tc, ta) = pose[0];
        let eye = to_screen([tc[0] + 0.3 * c.segments[0].length * ta.cos(), tc[1] + 0.3 * c.segments[0].length * ta.sin()]);
        let r = (c.segments[0].width as f32 * 0.2 * cam[2]).max(2.0);
        painter.circle_filled(eye, r, Color32::WHITE);
        painter.circle_filled(eye, r * 0.5, Color32::BLACK);
        for i in 1..c.segments.len() {
            painter.circle(to_screen(joint(c, &pose, i)), 3.0, Color32::from_gray(40), Stroke::new(1.5, Color32::WHITE));
        }
        if self.turning {
            let k = self.selected;
            let text = format!("{:.0}°", c.segments[k].angle);
            painter.text(to_screen(joint(c, &pose, k)) + Vec2::new(8.0, -8.0), Align2::LEFT_BOTTOM, text, FontId::proportional(14.0), Color32::WHITE);
        }
        if let Some(s) = &ghost {
            let (pc, pa) = pose[s.parent];
            let d = s.attach * c.segments[s.parent].length / 2.0;
            let a = pa + s.angle.to_radians();
            let p = [pc[0] + d * pa.cos() + s.length / 2.0 * a.cos(), pc[1] + d * pa.sin() + s.length / 2.0 * a.sin()];
            let fill = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 110);
            painter.add(Shape::convex_polygon(corners(s, (p, a)).iter().map(|&q| to_screen(q)).collect(), fill, Stroke::new(1.5, Color32::WHITE)));
            let text = format!("attach to segment {} ({}), angle {:.0}°", s.parent, attach_name(s.attach), s.angle);
            painter.text(to_screen(p) + Vec2::new(0.0, -14.0), Align2::CENTER_BOTTOM, text, FontId::proportional(13.0), Color32::WHITE);
        }

        let hints = [
            "Drag a block from the right onto the creature to attach it.",
            "Click a block to select it, drag it to turn it; Delete removes the selected block.",
            "The eye marks the front: races go to the right. Grid: 10 cm.",
        ];
        for (i, hint) in hints.iter().enumerate() {
            painter.text(rect.left_top() + Vec2::new(10.0, 8.0 + 18.0 * i as f32), Align2::LEFT_TOP, *hint, FontId::proportional(13.0), CANVAS_TEXT);
        }
    }

    fn draw_trial(&mut self, ui: &mut egui::Ui) {
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::hover());
        let rect = resp.rect;
        let painter = painter.with_clip_rect(rect);
        painter.rect_filled(rect, 0.0, CANVAS);
        let color = self.color();
        let Some(t) = &self.trial else { return };
        let a = &t.arena;
        let status = if t.sumo {
            let ring = a.rules.ring_width as f32;
            let scale = (rect.width() / (ring + 3.0)).min(rect.height() / 3.0);
            let ground_y = rect.top() + rect.height() * 0.62;
            let to_screen = |p: [f32; 2]| Pos2::new(rect.center().x + p[0] * scale, ground_y - p[1] * scale);
            let half = ring / 2.0;
            painter.rect_filled(Rect::from_two_pos(to_screen([-half, 0.0]), to_screen([half, -1.0])), 2.0, Color32::from_rgb(170, 140, 100));
            draw_creatures(&painter, a, &[color, Color32::from_gray(150)], &to_screen, scale, 255);
            match a.sumo_status() {
                None => format!("t = {:.1} / {:.0} s", a.time, a.rules.sumo_time),
                Some(r) => match r.winner {
                    Some(0) => format!("you win ({}, {:.1} s)", r.reason, r.time),
                    Some(_) => format!("{} wins ({}, {:.1} s)", t.rival, r.reason, r.time),
                    None => format!("Draw ({})", r.reason),
                },
            }
        } else {
            let x = a.com(0)[0] as f32;
            let fit = (rect.height() / 3.0).min(rect.width() / 6.0);
            let cam = match self.camera {
                Some(c) => [c[0] + (x - c[0]) * 0.1, 0.0, fit],
                None => [x, 0.0, fit],
            };
            self.camera = Some(cam);
            let ground_y = rect.top() + rect.height() * 0.7;
            let to_screen = |p: [f32; 2]| Pos2::new(rect.center().x + (p[0] - cam[0]) * fit, ground_y - p[1] * fit);
            let x_range = (cam[0] - rect.width() / 2.0 / fit, cam[0] + rect.width() / 2.0 / fit);
            draw_track(&painter, rect, ground_y, &to_screen, x_range, fit, &a.terrain, true);
            draw_creatures(&painter, a, &[color], &to_screen, fit, 255);
            format!("{:.2} m in {:.1} / {:.0} s", a.progress(0), a.time, a.rules.race_time)
        };
        painter.text(rect.left_top() + Vec2::new(10.0, 8.0), Align2::LEFT_TOP, format!("Trying {}: {status}", self.creature.name), FontId::proportional(16.0), Color32::WHITE);
        let hint = "Change anything, or press ■ Stop, to go back to the design.";
        painter.text(rect.left_top() + Vec2::new(10.0, 30.0), Align2::LEFT_TOP, hint, FontId::proportional(13.0), CANVAS_TEXT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worm() -> Creature {
        toml::from_str(include_str!("../../../creatures/worm.toml")).unwrap()
    }

    #[test]
    fn a_dropped_block_hangs_where_it_was_dropped() {
        let c = blank();
        let pose = rest_pose(&c);
        // Below the front end of the torso (0.5 m long, pointing right): front end, pointing down.
        assert_eq!(attachment(&c, &pose, [0.26, -0.4]), (0, 1.0, -90.0));
        // Behind it: back end, pointing back.
        assert_eq!(attachment(&c, &pose, [-0.6, 0.0]), (0, -1.0, -180.0));
        // Above the middle.
        assert_eq!(attachment(&c, &pose, [0.01, 0.3]), (0, 0.0, 90.0));
    }

    #[test]
    fn removing_a_block_removes_what_hangs_on_it() {
        // worm: 0 ← 1 ← 2 ← 3 ← 4, a chain
        let mut c = worm();
        assert_eq!(remove(&mut c, 2), 1);
        assert_eq!(c.segments.len(), 2);
        let mut c = worm();
        c.segments[3].parent = 1; // 0 ← 1 ← {2, 3 ← 4}
        assert_eq!(remove(&mut c, 2), 1);
        assert_eq!(c.segments.iter().map(|s| s.parent).collect::<Vec<_>>(), vec![0, 0, 1, 2]);
        assert!(c.validate(&Rules::default()).is_ok());
    }

    #[test]
    fn file_names_come_from_the_creature_name() {
        assert_eq!(slug("Long Legs 2"), "long-legs-2");
        assert_eq!(slug("Wörm!"), "w-rm");
        assert_eq!(slug("  "), "my-creature");
        assert_eq!(slug("Arena"), "arena-creature");
        let dir = std::env::temp_dir().join("arena-gui-design-test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x.toml"), "").unwrap();
        assert_eq!(unique(&dir, "x"), dir.join("x-2.toml"));
    }

    #[test]
    fn the_design_view_matches_the_arena() {
        // The canvas draws from rest_pose, the same kinematics the arena spawns with.
        let c = worm();
        let pose = rest_pose(&c);
        let arena = Arena::race(&c, &Rules::default());
        let segs = arena.segments();
        let (dx, dy) = (segs[0].center[0] as f64 - pose[0].0[0], segs[0].center[1] as f64 - pose[0].0[1]);
        for (s, (p, a)) in segs.iter().zip(&pose) {
            assert!((s.center[0] as f64 - p[0] - dx).abs() < 1e-4 && (s.center[1] as f64 - p[1] - dy).abs() < 1e-4);
            assert!((s.angle as f64 - wrap(a.to_degrees()).to_radians()).abs() < 1e-4);
        }
    }
}
