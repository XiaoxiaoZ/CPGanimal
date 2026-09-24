//! Arena viewer: loads a folder of creature files and shows races and sumo bouts.
//!
//! Usage: `arena-gui [folder] [--race | --tournament]` (default folder
//! `creatures`; the flags start that event right away). Files are reloaded
//! automatically when they change, so students can train in a terminal and
//! watch the result appear.

use cpg_arena::creature::{Creature, Rules};
use cpg_arena::game::{self, Bout, Entry, Standing};
use cpg_arena::sim::{Arena, SumoResult};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Shape, Stroke, Vec2};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};

const PALETTE: [[u8; 3]; 8] = [
    [230, 120, 40],
    [60, 150, 230],
    [90, 190, 90],
    [210, 80, 160],
    [240, 200, 50],
    [150, 110, 220],
    [60, 200, 190],
    [220, 70, 70],
];

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let folder = args.iter().find(|a| !a.starts_with("--")).map(PathBuf::from).unwrap_or_else(|| "creatures".into());
    let mut app = App::new(folder);
    if args.iter().any(|a| a == "--race") {
        app.start_race();
    }
    if args.iter().any(|a| a == "--tournament") {
        app.tab = Tab::Sumo;
        app.start_tournament();
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 760.0]).with_title("CPG Arena"),
        ..Default::default()
    };
    eframe::run_native("CPG Arena", options, Box::new(|_cc| Ok(Box::new(app))))
}

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Race,
    Sumo,
}

struct Racer {
    entry: usize,
    arena: Arena,
    color: Color32,
}

struct Bout1 {
    arena: Arena,
    names: [String; 2],
    colors: [Color32; 2],
    result: Option<SumoResult>,
}

type TourResult = (Vec<Creature>, Vec<Standing>, Vec<Bout>);

struct App {
    folder: PathBuf,
    folder_text: String,
    rules: Rules,
    rules_error: Option<String>,
    entries: Vec<Entry>,
    stamp: Vec<(PathBuf, Option<std::time::SystemTime>)>,
    last_poll: f64,
    selected: HashSet<PathBuf>,
    tab: Tab,
    speed: f32,
    paused: bool,
    accumulator: f64,
    race: Vec<Racer>,
    bout: Option<Bout1>,
    pick: [usize; 2],
    tour: Option<TourResult>,
    tour_pending: Option<Receiver<TourResult>>,
    camera: Option<(f32, f32)>,
}

impl App {
    fn new(folder: PathBuf) -> Self {
        let mut app = Self {
            folder_text: folder.display().to_string(),
            folder,
            rules: Rules::default(),
            rules_error: None,
            entries: Vec::new(),
            stamp: Vec::new(),
            last_poll: 0.0,
            selected: HashSet::new(),
            tab: Tab::Race,
            speed: 1.0,
            paused: false,
            accumulator: 0.0,
            race: Vec::new(),
            bout: None,
            pick: [0, 1],
            tour: None,
            tour_pending: None,
            camera: None,
        };
        app.reload(true);
        app
    }

    fn reload(&mut self, select_all: bool) {
        match Rules::load_dir(&self.folder) {
            Ok(r) => {
                self.rules = r;
                self.rules_error = None;
            }
            Err(e) => self.rules_error = Some(e),
        }
        let known: HashSet<PathBuf> = self.entries.iter().map(|e| e.path.clone()).collect();
        self.entries = game::load_folder(&self.folder, &self.rules);
        self.stamp = game::folder_stamp(&self.folder);
        for e in &self.entries {
            // New files join the selection automatically.
            if e.creature.is_ok() && (select_all || !known.contains(&e.path)) {
                self.selected.insert(e.path.clone());
            }
        }
        let n = self.valid().len();
        self.pick = [self.pick[0].min(n.saturating_sub(1)), self.pick[1].min(n.saturating_sub(1))];
    }

    fn color(&self, idx: usize) -> Color32 {
        let c = match &self.entries[idx].creature {
            Ok(c) => c.color.unwrap_or(PALETTE[idx % PALETTE.len()]),
            Err(_) => [128, 128, 128],
        };
        Color32::from_rgb(c[0], c[1], c[2])
    }

    /// Indices of valid creatures.
    fn valid(&self) -> Vec<usize> {
        (0..self.entries.len()).filter(|&i| self.entries[i].creature.is_ok()).collect()
    }

    fn checked(&self) -> Vec<usize> {
        self.valid().into_iter().filter(|&i| self.selected.contains(&self.entries[i].path)).collect()
    }

    fn creature(&self, i: usize) -> &Creature {
        self.entries[i].creature.as_ref().expect("valid entry")
    }

    fn start_race(&mut self) {
        self.bout = None;
        self.camera = None;
        self.accumulator = 0.0;
        self.paused = false;
        self.race = self
            .checked()
            .into_iter()
            .map(|i| Racer { entry: i, arena: Arena::race(self.creature(i), &self.rules), color: self.color(i) })
            .collect();
    }

    fn start_bout(&mut self, a: &Creature, b: &Creature, colors: [Color32; 2]) {
        self.race.clear();
        self.accumulator = 0.0;
        self.paused = false;
        self.bout = Some(Bout1 {
            arena: Arena::sumo(a, b, &self.rules),
            names: [a.name.clone(), b.name.clone()],
            colors,
            result: None,
        });
    }

    fn start_tournament(&mut self) {
        let cs: Vec<Creature> = self.checked().into_iter().map(|i| self.creature(i).clone()).collect();
        let rules = self.rules.clone();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let (table, bouts) = game::tournament(&cs, &rules);
            tx.send((cs, table, bouts)).ok();
        });
        self.tour_pending = Some(rx);
    }

    /// Replay the bout between the top two of the tournament.
    fn play_final(&mut self) {
        let Some((cs, table, _)) = &self.tour else { return };
        if table.len() < 2 {
            return;
        }
        let (a, b) = (cs[table[0].index].clone(), cs[table[1].index].clone());
        let colors = [self.color_of(&a), self.color_of(&b)];
        self.start_bout(&a, &b, colors);
    }

    fn color_of(&self, c: &Creature) -> Color32 {
        let i = self.entries.iter().position(|e| e.creature.as_ref().is_ok_and(|x| x.name == c.name));
        i.map_or(Color32::GRAY, |i| self.color(i))
    }

    fn running(&self) -> bool {
        let race_on = self.race.iter().any(|r| r.arena.time < self.rules.race_time);
        let bout_on = self.bout.as_ref().is_some_and(|b| b.result.is_none());
        !self.paused && (race_on || bout_on)
    }

    fn advance(&mut self, dt: f64) {
        if !self.running() {
            self.accumulator = 0.0;
            return;
        }
        self.accumulator += dt * self.speed as f64;
        let step = self.rules.dt;
        let mut budget = 240; // never freeze the UI
        while self.accumulator >= step && budget > 0 {
            self.accumulator -= step;
            budget -= 1;
            for r in &mut self.race {
                if r.arena.time < self.rules.race_time && !r.arena.is_broken() {
                    r.arena.step();
                }
            }
            if let Some(b) = &mut self.bout {
                if b.result.is_none() {
                    b.arena.step();
                    b.result = b.arena.sumo_status();
                }
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        let dt = ctx.input(|i| i.stable_dt).min(0.1) as f64;

        // Pick up edited / newly trained files.
        if now - self.last_poll > 1.0 {
            self.last_poll = now;
            if game::folder_stamp(&self.folder) != self.stamp {
                self.reload(false);
            }
        }
        if let Some(rx) = &self.tour_pending {
            if let Ok(t) = rx.try_recv() {
                self.tour = Some(t);
                self.tour_pending = None;
                self.play_final();
            }
        }
        self.advance(dt);

        egui::Panel::top("top").show(ui, |ui| self.top_bar(ui));
        egui::Panel::left("creatures").resizable(true).default_size(260.0).show(ui, |ui| self.creature_list(ui));
        egui::Panel::right("results").resizable(true).default_size(300.0).show(ui, |ui| match self.tab {
            Tab::Race => self.race_panel(ui),
            Tab::Sumo => self.sumo_panel(ui),
        });
        egui::CentralPanel::default().show(ui, |ui| self.canvas(ui));

        if self.running() || self.tour_pending.is_some() {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }
}

impl App {
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("CPG Arena");
            ui.separator();
            ui.label("Folder");
            let r = ui.add(egui::TextEdit::singleline(&mut self.folder_text).desired_width(220.0));
            if ui.button("Load").clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                self.folder = PathBuf::from(&self.folder_text);
                self.selected.clear();
                self.entries.clear();
                self.reload(true);
            }
            if ui.button("⟳ Reload").clicked() {
                self.reload(false);
            }
            ui.separator();
            ui.selectable_value(&mut self.tab, Tab::Race, "Race");
            ui.selectable_value(&mut self.tab, Tab::Sumo, "Sumo");
            ui.separator();
            let label = if self.paused { "▶ Play" } else { "⏸ Pause" };
            if ui.button(label).clicked() {
                self.paused = !self.paused;
            }
            ui.add(egui::Slider::new(&mut self.speed, 0.25..=8.0).logarithmic(true).text("speed"));
        });
    }

    fn creature_list(&mut self, ui: &mut egui::Ui) {
        ui.heading("Creatures");
        ui.label(RichText::new(format!("{}", self.folder.display())).small().weak());
        if let Some(e) = &self.rules_error {
            ui.colored_label(Color32::LIGHT_RED, format!("arena.toml: {e}"));
        }
        ui.horizontal(|ui| {
            if ui.small_button("all").clicked() {
                for i in self.valid() {
                    self.selected.insert(self.entries[i].path.clone());
                }
            }
            if ui.small_button("none").clicked() {
                self.selected.clear();
            }
        });
        ui.separator();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for i in 0..self.entries.len() {
                let color = self.color(i);
                let e = &self.entries[i];
                let path = e.path.clone();
                match &e.creature {
                    Ok(c) => {
                        let mut on = self.selected.contains(&path);
                        ui.horizontal(|ui| {
                            let (r, p) = ui.allocate_painter(Vec2::splat(14.0), Sense::hover());
                            p.rect_filled(r.rect.shrink(1.0), 3.0, color);
                            if ui.checkbox(&mut on, RichText::new(&c.name).strong()).changed() {
                                if on {
                                    self.selected.insert(path.clone());
                                } else {
                                    self.selected.remove(&path);
                                }
                            }
                        });
                        let mut info = format!("{} segs · {:.2} m²", c.segments.len(), c.area());
                        if !c.author.is_empty() {
                            info = format!("by {} · {info}", c.author);
                        }
                        ui.label(RichText::new(info).small().weak());
                        if let Some(m) = &c.meta {
                            ui.label(
                                RichText::new(format!("trained: {} · fitness {:.2} · {} gens", m.trained_for, m.fitness, m.generations))
                                    .small()
                                    .color(Color32::from_rgb(120, 180, 120)),
                            );
                        }
                    }
                    Err(errs) => {
                        ui.label(RichText::new(format!("⚠ {}", e.label())).color(Color32::LIGHT_RED));
                        for m in errs {
                            ui.label(RichText::new(m).small().color(Color32::LIGHT_RED));
                        }
                    }
                }
                ui.add_space(4.0);
            }
        });
    }

    fn race_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Race");
        ui.label(format!("Go right as far as possible in {:.0} s.", self.rules.race_time));
        if ui.add_enabled(!self.checked().is_empty(), egui::Button::new("▶ Start race")).clicked() {
            self.start_race();
        }
        ui.separator();
        if self.race.is_empty() {
            ui.label("Tick creatures on the left, then start.");
            return;
        }
        let t = self.race.iter().map(|r| r.arena.time).fold(0.0, f64::max);
        ui.label(format!("t = {t:.1} / {:.0} s", self.rules.race_time));
        let mut order: Vec<(usize, f64)> = self.race.iter().enumerate().map(|(k, r)| (k, r.arena.progress(0))).collect();
        order.sort_by(|a, b| b.1.total_cmp(&a.1));
        let done = t >= self.rules.race_time - 1e-9;
        egui::Grid::new("race_table").striped(true).show(ui, |ui| {
            for (rank, (k, d)) in order.iter().enumerate() {
                let r = &self.race[*k];
                let place = format!("{}", rank + 1);
                ui.label(if done && rank == 0 { RichText::new(place + " ★").strong() } else { RichText::new(place) });
                ui.colored_label(r.color, &self.creature(r.entry).name);
                ui.label(if r.arena.is_broken() { "broke".to_string() } else { format!("{d:6.2} m") });
                ui.end_row();
            }
        });
    }

    fn sumo_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Sumo");
        ui.label("Push the other creature off the platform. At time-up, whoever is nearer the centre wins.");
        let valid = self.valid();
        if valid.len() >= 2 {
            ui.horizontal(|ui| {
                for (side, label) in ["Left", "Right"].iter().enumerate() {
                    let cur = valid.get(self.pick[side]).map_or("-".to_string(), |&i| self.creature(i).name.clone());
                    egui::ComboBox::from_id_salt(label).selected_text(cur).show_ui(ui, |ui| {
                        for (k, &i) in valid.iter().enumerate() {
                            let name = self.creature(i).name.clone();
                            ui.selectable_value(&mut self.pick[side], k, name);
                        }
                    });
                }
            });
            if ui.button("▶ Fight!").clicked() {
                let (a, b) = (valid[self.pick[0]], valid[self.pick[1]]);
                let (ca, cb) = (self.creature(a).clone(), self.creature(b).clone());
                self.start_bout(&ca, &cb, [self.color(a), self.color(b)]);
            }
        } else {
            ui.label("Need at least two valid creatures.");
        }
        ui.separator();
        let n = self.checked().len();
        let busy = self.tour_pending.is_some();
        if ui.add_enabled(n >= 2 && !busy, egui::Button::new(format!("Tournament ({n} ticked)"))).clicked() {
            self.start_tournament();
        }
        if busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("fighting…");
            });
        }
        let mut replay = None;
        let mut replay_final = false;
        if let Some((cs, table, bouts)) = &self.tour {
            ui.label(RichText::new("Standings (3 per win, 1 per draw)").strong());
            egui::Grid::new("standings").striped(true).show(ui, |ui| {
                for h in ["#", "Creature", "W", "D", "L", "Pts"] {
                    ui.label(RichText::new(h).weak());
                }
                ui.end_row();
                for (rank, s) in table.iter().enumerate() {
                    ui.label(format!("{}", rank + 1));
                    ui.label(&cs[s.index].name);
                    ui.label(s.wins.to_string());
                    ui.label(s.draws.to_string());
                    ui.label(s.losses.to_string());
                    ui.label(RichText::new(s.points.to_string()).strong());
                    ui.end_row();
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(RichText::new("Bouts (click to replay)").strong());
                if ui.small_button("▶ Final").clicked() {
                    replay_final = true;
                }
            });
            egui::ScrollArea::vertical().show(ui, |ui| {
                for b in bouts {
                    let res = match b.winner {
                        Some(w) => format!("winner {} ({})", cs[w].name, b.reason),
                        None => format!("draw ({})", b.reason),
                    };
                    let text = format!("{} vs {}: {}", cs[b.left].name, cs[b.right].name, res);
                    if ui.selectable_label(false, RichText::new(text).small()).clicked() {
                        replay = Some((cs[b.left].clone(), cs[b.right].clone()));
                    }
                }
            });
        }
        if replay_final {
            self.play_final();
        }
        if let Some((a, b)) = replay {
            let colors = [self.color_of(&a), self.color_of(&b)];
            self.start_bout(&a, &b, colors);
        }
    }

    fn canvas(&mut self, ui: &mut egui::Ui) {
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::hover());
        let rect = resp.rect;
        let painter = painter.with_clip_rect(rect);
        painter.rect_filled(rect, 0.0, Color32::from_rgb(28, 32, 40));
        if self.bout.is_some() {
            self.draw_sumo(&painter, rect);
        } else if !self.race.is_empty() {
            self.draw_race(&painter, rect);
        } else {
            painter.text(rect.center(), Align2::CENTER_CENTER, "Pick creatures, then Start race or Fight!", FontId::proportional(18.0), Color32::from_gray(160));
        }
    }

    /// Smoothly move the camera towards (centre x in m, pixels per m).
    fn follow(&mut self, target: (f32, f32)) -> (f32, f32) {
        let cam = match self.camera {
            Some((x, s)) => (x + (target.0 - x) * 0.1, s + (target.1 - s) * 0.1),
            None => target,
        };
        self.camera = Some(cam);
        cam
    }

    /// One lane per racer, all sharing the same horizontal camera.
    fn draw_race(&mut self, painter: &egui::Painter, rect: Rect) {
        let n = self.race.len();
        let lane_h = rect.height() / n as f32;
        let xs: Vec<f32> = self.race.iter().map(|r| r.arena.com(0)[0] as f32).filter(|x| x.is_finite()).collect();
        let (lo, hi) = xs.iter().fold((f32::MAX, f32::MIN), |(l, h), &x| (l.min(x), h.max(x)));
        let span = (hi - lo + 4.0).max(6.0);
        let fit = (rect.width() / span).min(lane_h / 1.4).clamp(8.0, 160.0);
        let (cx, scale) = self.follow(((lo + hi) / 2.0, fit));
        let x0 = cx - rect.width() / 2.0 / scale;
        let x1 = cx + rect.width() / 2.0 / scale;
        let label_every = if scale > 40.0 { 1 } else if scale > 15.0 { 5 } else { 10 };

        for (k, r) in self.race.iter().enumerate() {
            let top = rect.top() + k as f32 * lane_h;
            let ground_y = top + lane_h * 0.82;
            let to_screen = |p: [f32; 2]| Pos2::new(rect.center().x + (p[0] - cx) * scale, ground_y - p[1] * scale);
            if k % 2 == 1 {
                painter.rect_filled(Rect::from_min_max(Pos2::new(rect.left(), top), Pos2::new(rect.right(), top + lane_h)), 0.0, Color32::from_rgb(33, 38, 48));
            }
            painter.rect_filled(Rect::from_min_max(Pos2::new(rect.left(), ground_y), Pos2::new(rect.right(), ground_y + lane_h * 0.06 + 2.0)), 0.0, Color32::from_rgb(110, 90, 65));
            for m in (x0.floor() as i32)..=(x1.ceil() as i32) {
                if m % label_every != 0 {
                    continue;
                }
                let x = to_screen([m as f32, 0.0]).x;
                let tick = if m == 0 { Color32::WHITE } else { Color32::from_gray(150) };
                painter.line_segment([Pos2::new(x, ground_y), Pos2::new(x, ground_y + 5.0)], Stroke::new(1.0, tick));
                if k == n - 1 || lane_h > 60.0 {
                    painter.text(Pos2::new(x, ground_y + 6.0), Align2::CENTER_TOP, format!("{m} m"), FontId::proportional(10.0), Color32::from_gray(170));
                }
            }
            let start = to_screen([0.0, 0.0]);
            painter.line_segment([Pos2::new(start.x, top + 4.0), Pos2::new(start.x, ground_y)], Stroke::new(1.0, Color32::from_white_alpha(40)));
            draw_creatures(painter, &r.arena, &[r.color], &to_screen, scale, 255);
            let d = r.arena.progress(0);
            painter.text(Pos2::new(rect.left() + 10.0, top + 6.0), Align2::LEFT_TOP, format!("{}  {:.2} m", self.creature(r.entry).name, d), FontId::proportional(14.0), r.color);
        }
        let t = self.race.iter().map(|r| r.arena.time).fold(0.0, f64::max);
        painter.text(rect.right_top() + Vec2::new(-12.0, 6.0), Align2::RIGHT_TOP, format!("t = {t:.1} / {:.0} s", self.rules.race_time), FontId::proportional(18.0), Color32::WHITE);
    }

    fn draw_sumo(&mut self, painter: &egui::Painter, rect: Rect) {
        let Some(ring) = self.bout.as_ref().map(|b| b.arena.rules.ring_width as f32) else { return };
        let (cx, scale) = self.follow((0.0, rect.width() / (ring + 3.0)));
        let b = self.bout.as_ref().expect("bout");
        let ground_y = rect.top() + rect.height() * 0.65;
        let to_screen = |p: [f32; 2]| Pos2::new(rect.center().x + (p[0] - cx) * scale, ground_y - p[1] * scale);
        let half = ring / 2.0;
        painter.rect_filled(Rect::from_two_pos(to_screen([-half, 0.0]), to_screen([half, -1.0])), 2.0, Color32::from_rgb(170, 140, 100));
        painter.line_segment([to_screen([0.0, 0.0]), to_screen([0.0, -0.15])], Stroke::new(2.0, Color32::WHITE));
        draw_creatures(painter, &b.arena, &b.colors, &to_screen, scale, 255);
        for k in 0..2 {
            let c = b.arena.com(k);
            painter.text(to_screen([c[0] as f32, c[1] as f32 + 0.8]), Align2::CENTER_BOTTOM, &b.names[k], FontId::proportional(15.0), b.colors[k]);
        }
        let status = match &b.result {
            None => format!("t = {:.1} / {:.0} s", b.arena.time, b.arena.rules.sumo_time),
            Some(r) => match r.winner {
                Some(w) => format!("Winner: {} ({}, {:.1} s)", b.names[w], r.reason, r.time),
                None => format!("Draw ({})", r.reason),
            },
        };
        painter.text(rect.left_top() + Vec2::new(12.0, 10.0), Align2::LEFT_TOP, status, FontId::proportional(20.0), Color32::WHITE);
    }
}

fn draw_creatures(painter: &egui::Painter, arena: &Arena, colors: &[Color32], to_screen: &impl Fn([f32; 2]) -> Pos2, scale: f32, alpha: u8) {
    let mut first_of = vec![true; arena.fighters.len()];
    for s in arena.segments() {
        let base = colors[s.fighter.min(colors.len() - 1)];
        let fill = Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), alpha);
        let (sin, cos) = s.angle.sin_cos();
        let corner = |u: f32, v: f32| {
            let (lx, ly) = (u * s.half[0], v * s.half[1]);
            to_screen([s.center[0] + lx * cos - ly * sin, s.center[1] + lx * sin + ly * cos])
        };
        let pts = vec![corner(1.0, 1.0), corner(-1.0, 1.0), corner(-1.0, -1.0), corner(1.0, -1.0)];
        painter.add(Shape::convex_polygon(pts, fill, Stroke::new(1.0, Color32::from_black_alpha(160))));
        // An eye on the torso so you can tell which way is forward.
        if std::mem::take(&mut first_of[s.fighter]) {
            let r = (s.half[1] * 0.45 * scale).max(2.0);
            let eye = corner(0.6, 0.0);
            painter.circle_filled(eye, r, Color32::WHITE);
            painter.circle_filled(eye, r * 0.5, Color32::BLACK);
        }
    }
}
