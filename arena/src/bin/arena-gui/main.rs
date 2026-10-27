//! Arena viewer: loads a folder of creature files and shows races and sumo bouts.
//!
//! Usage: `arena-gui [folder] [--race | --tournament]` (default folder
//! `creatures`; the flags start that event right away). Another folder can be
//! picked with Browse… in the top bar. Files are reloaded automatically when
//! they change, so students can train in a terminal (or in the Train tab) and
//! watch the result appear. Creatures made in the Design and Train tabs are
//! saved to the creatures folder.

mod design;
mod environment;
mod python;
mod scene;
mod train;

use cpg_arena::creature::{Creature, Rules};
use cpg_arena::game::{self, Bout, Entry, Standing};
use cpg_arena::sim::{Arena, SumoResult};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Vec2};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
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
    let folder = args.iter().find(|a| !a.starts_with("--")).map(PathBuf::from).unwrap_or_else(creatures_dir);
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
    eframe::run_native(
        "CPG Arena",
        options,
        Box::new(|cc| {
            scene::install_fonts(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
}

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Race,
    Sumo,
    Design,
    Train,
}

/// Where to look for one of the arena's folders (`python`, `creatures`), in
/// order: the current directory, next to this program (the student package),
/// and the source tree it was built from.
fn arena_paths(name: &str) -> Vec<PathBuf> {
    let next_to_exe = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|d| d.join(name)));
    [Some(PathBuf::from(name)), next_to_exe, Some(Path::new(env!("CARGO_MANIFEST_DIR")).join(name))]
        .into_iter()
        .flatten()
        .collect()
}

/// The creatures folder: designed and trained creatures are saved here.
fn creatures_dir() -> PathBuf {
    let paths = arena_paths("creatures");
    let found = paths.iter().find(|d| d.is_dir()).unwrap_or(&paths[paths.len() - 1]);
    std::path::absolute(found).unwrap_or_else(|_| found.clone())
}

/// Whether two paths name the same existing folder.
fn same_dir(a: &Path, b: &Path) -> bool {
    matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
}

struct Racer {
    // Not an index into `entries`: the list is re-sorted when trained files appear.
    name: String,
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
    /// The rules the current race runs on, and a description of its track.
    race_rules: Rules,
    race_track: String,
    /// The track to race on, and whether each race gets a new environment seed.
    race_choice: environment::TrackChoice,
    /// The folder has an arena.toml: its own track is the class track, not flat.
    class_track: bool,
    race_new_each: bool,
    bout: Option<Bout1>,
    pick: [usize; 2],
    tour: Option<TourResult>,
    tour_pending: Option<Receiver<TourResult>>,
    camera: Option<(f32, f32)>,
    /// Where the Design and Train tabs save creatures.
    creatures: PathBuf,
    designer: design::Designer,
    trainer: train::Trainer,
}

impl App {
    fn new(folder: PathBuf) -> Self {
        let folder = std::path::absolute(&folder).unwrap_or(folder);
        let creatures = creatures_dir();
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
            race_rules: Rules::default(),
            race_track: String::new(),
            race_choice: environment::TrackChoice::new(),
            class_track: false,
            race_new_each: false,
            bout: None,
            pick: [0, 1],
            tour: None,
            tour_pending: None,
            camera: None,
            trainer: train::Trainer::new(creatures.clone()),
            designer: design::Designer::new(),
            creatures,
        };
        app.reload(true);
        app
    }

    fn open_folder(&mut self, folder: PathBuf) {
        let folder = std::path::absolute(&folder).unwrap_or(folder);
        self.folder_text = folder.display().to_string();
        self.folder = folder;
        self.selected.clear();
        self.entries.clear();
        self.race.clear();
        self.reload(true);
    }

    /// Show the creatures folder, where Design and Train save.
    fn open_creatures(&mut self) {
        if same_dir(&self.folder, &self.creatures) {
            self.reload(false);
        } else {
            self.open_folder(self.creatures.clone());
        }
    }

    fn design_action(&mut self, action: design::Action) {
        match action {
            design::Action::Train(path) => {
                self.open_creatures();
                self.trainer.use_template(&path, &self.entries);
                self.tab = Tab::Train;
            }
        }
    }

    /// Native folder picker, starting from the current folder when it exists.
    fn browse(&mut self) {
        let mut dialog = rfd::FileDialog::new().set_title("Choose a creature folder");
        if let Some(dir) = std::path::absolute(&self.folder).ok().filter(|p| p.is_dir()) {
            dialog = dialog.set_directory(dir);
        }
        if let Some(dir) = dialog.pick_folder() {
            self.open_folder(dir);
        }
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
        self.class_track = self.folder.join("arena.toml").is_file();
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
        if self.race_choice.seeded && self.race_new_each {
            self.race_choice.seed = environment::new_seed();
        }
        let (rules, track) = (self.race_choice.rules(&self.rules), self.race_choice.describe(&self.rules, self.class_track));
        self.race = self
            .checked()
            .into_iter()
            .map(|i| Racer { name: self.creature(i).name.clone(), arena: Arena::race(self.creature(i), &rules), color: self.color(i) })
            .collect();
        (self.race_rules, self.race_track) = (rules, track);
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
        let race_on = self.race.iter().any(|r| r.arena.time < r.arena.rules.race_time);
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
                if r.arena.time < r.arena.rules.race_time && !r.arena.is_broken() {
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
        let training = self.trainer.poll();
        let replaying = self.tab == Tab::Train && self.trainer.advance(dt, self.speed, self.paused);
        let designing = self.tab == Tab::Design && self.designer.advance(dt, self.speed, self.paused);

        egui::Panel::top("top").show(ui, |ui| self.top_bar(ui));
        egui::Panel::left("creatures").resizable(true).default_size(260.0).show(ui, |ui| self.creature_list(ui));
        egui::Panel::right("results").resizable(true).default_size(300.0).show(ui, |ui| match self.tab {
            Tab::Race => self.race_panel(ui),
            Tab::Sumo => self.sumo_panel(ui),
            Tab::Design => {
                if let Some(a) = self.designer.panel(ui, &self.rules, &self.entries, &self.creatures) {
                    self.design_action(a);
                }
            }
            Tab::Train => {
                if self.trainer.panel(ui, &self.folder, &self.entries, &self.rules) {
                    self.open_creatures();
                }
            }
        });
        egui::CentralPanel::default().show(ui, |ui| match self.tab {
            Tab::Design => self.designer.view(ui, &self.rules),
            Tab::Train => self.trainer.view(ui, &self.rules),
            _ => self.canvas(ui),
        });

        if self.running() || self.tour_pending.is_some() || replaying || designing {
            ctx.request_repaint();
        } else if training {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
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
                self.open_folder(PathBuf::from(&self.folder_text));
            }
            if ui.button("Browse…").clicked() {
                self.browse();
            }
            if ui.button("⟳ Reload").clicked() {
                self.reload(false);
            }
            ui.separator();
            ui.selectable_value(&mut self.tab, Tab::Race, "Race");
            ui.selectable_value(&mut self.tab, Tab::Sumo, "Sumo");
            ui.selectable_value(&mut self.tab, Tab::Design, "Design");
            ui.selectable_value(&mut self.tab, Tab::Train, "Train");
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
        if !self.folder.is_dir() {
            ui.colored_label(Color32::LIGHT_RED, "Folder not found: pick one with Browse…");
        }
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
                                RichText::new(format!("trained: {} · fitness {:.2} · {} evals", m.trained_for, m.fitness, m.evaluations))
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
        let before = self.race_choice;
        self.race_choice.ui(ui, self.class_track);
        if self.race_choice != before {
            // Show the newly chosen track instead of the last race.
            self.race.clear();
        }
        if self.race_choice.seeded {
            ui.checkbox(&mut self.race_new_each, "a new environment for every race");
        }
        let next = if self.race_choice.seeded && self.race_new_each {
            "Each race gets a new environment seed.".to_string()
        } else {
            self.race_choice.describe(&self.rules, self.class_track)
        };
        ui.label(RichText::new(next).small().weak());
        if ui.add_enabled(!self.checked().is_empty(), egui::Button::new("▶ Start race")).clicked() {
            self.start_race();
        }
        ui.separator();
        if self.race.is_empty() {
            ui.label("Tick creatures on the left, then start.");
            return;
        }
        ui.label(RichText::new(&self.race_track).small());
        let t = self.race.iter().map(|r| r.arena.time).fold(0.0, f64::max);
        ui.label(format!("t = {t:.1} / {:.0} s", self.race_rules.race_time));
        let mut order: Vec<(usize, f64)> = self.race.iter().enumerate().map(|(k, r)| (k, r.arena.progress(0))).collect();
        order.sort_by(|a, b| b.1.total_cmp(&a.1));
        let done = t >= self.race_rules.race_time - 1e-9;
        egui::Grid::new("race_table").striped(true).show(ui, |ui| {
            for (rank, (k, d)) in order.iter().enumerate() {
                let r = &self.race[*k];
                let place = format!("{}", rank + 1);
                ui.label(if done && rank == 0 { RichText::new(place + " ★").strong() } else { RichText::new(place) });
                ui.colored_label(r.color, &r.name);
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
        } else if self.tab == Tab::Race {
            // The track the next race will run on.
            environment::draw_terrain(&painter, rect.shrink2(Vec2::new(30.0, 70.0)), &self.race_choice.rules(&self.rules));
            let head = if self.race_choice.seeded && self.race_new_each {
                "A new environment for every race; this is the next one.".to_string()
            } else {
                self.race_choice.describe(&self.rules, self.class_track)
            };
            scene::shadow_text(&painter, rect.left_top() + Vec2::new(10.0, 8.0), Align2::LEFT_TOP, head, FontId::proportional(15.0), Color32::WHITE);
            let hint = "The first 20 m of the track. Tick creatures on the left, then ▶ Start race.";
            scene::shadow_text(&painter, rect.left_top() + Vec2::new(10.0, 28.0), Align2::LEFT_TOP, hint, FontId::proportional(13.0), Color32::from_gray(200));
        } else {
            // The platform the next bout will be on.
            environment::draw_ring(&painter, rect, &self.rules);
            let head = environment::TrackChoice { seeded: false, seed: 0 }.describe_sumo(&self.rules, self.class_track);
            scene::shadow_text(&painter, rect.left_top() + Vec2::new(10.0, 8.0), Align2::LEFT_TOP, head, FontId::proportional(15.0), Color32::WHITE);
            scene::shadow_text(&painter, rect.center_top() + Vec2::new(0.0, 40.0), Align2::CENTER_TOP, "Pick two creatures, then Fight!", FontId::proportional(18.0), Color32::from_gray(220));
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

        for (k, r) in self.race.iter().enumerate() {
            let top = rect.top() + k as f32 * lane_h;
            let lane = Rect::from_min_max(Pos2::new(rect.left(), top), Pos2::new(rect.right(), top + lane_h));
            let ground_y = top + lane_h * 0.82;
            let to_screen = |p: [f32; 2]| Pos2::new(rect.center().x + (p[0] - cx) * scale, ground_y - p[1] * scale);
            if k % 2 == 1 {
                painter.rect_filled(lane, 0.0, Color32::from_rgb(33, 38, 48));
            }
            draw_track(painter, lane, ground_y, &to_screen, (x0, x1), scale, &r.arena.terrain, k == n - 1 || lane_h > 60.0);
            draw_creatures(&painter.with_clip_rect(lane.intersect(rect)), &r.arena, &[r.color], &to_screen, scale, 255);
            let d = r.arena.progress(0);
            // On a dark tag, so it stays readable in thin lanes.
            let font = FontId::proportional(if lane_h < 40.0 { 11.0 } else { 14.0 });
            let at = Pos2::new(rect.left() + 10.0, top + (lane_h * 0.12).min(6.0));
            let label = painter.layout_no_wrap(format!("{}  {:.2} m", r.name, d), font, r.color);
            painter.rect_filled(Rect::from_min_size(at, label.size()).expand2(Vec2::new(5.0, 1.0)), 4.0, Color32::from_black_alpha(170));
            painter.galley(at, label, r.color);
        }
        let t = self.race.iter().map(|r| r.arena.time).fold(0.0, f64::max);
        let r = &self.race_rules;
        let mut hud = format!("t = {t:.1} / {:.0} s", r.race_time);
        if r.slope != 0.0 {
            hud += &format!("   slope {:+.1}° ({})", r.slope, if r.slope > 0.0 { "uphill" } else { "downhill" });
        }
        scene::shadow_text(painter, rect.right_top() + Vec2::new(-12.0, 6.0), Align2::RIGHT_TOP, hud, FontId::proportional(18.0), Color32::WHITE);
    }

    fn draw_sumo(&mut self, painter: &egui::Painter, rect: Rect) {
        let Some(b) = self.bout.as_ref() else { return };
        scene::sumo_view(painter, rect, &b.arena, &b.colors, &b.names, 0.4);
        let status = match &b.result {
            None => format!("t = {:.1} / {:.0} s", b.arena.time, b.arena.rules.sumo_time),
            Some(r) => match r.winner {
                Some(w) => format!("Winner: {} ({}, {:.1} s)", b.names[w], r.reason, r.time),
                None => format!("Draw ({})", r.reason),
            },
        };
        scene::shadow_text(painter, rect.left_top() + Vec2::new(12.0, 10.0), Align2::LEFT_TOP, status, FontId::proportional(20.0), Color32::WHITE);
    }
}

/// Whether a race is over (time up or physics broke) or a sumo bout is decided.
fn finished(a: &Arena, sumo: bool) -> bool {
    if sumo {
        a.sumo_status().is_some()
    } else {
        a.time >= a.rules.race_time || a.is_broken()
    }
}

/// One race lane: sky, the ground with its hills, distance marks and the
/// start flag (see scene.rs; only the look, the physics are the arena's).
#[allow(clippy::too_many_arguments)]
fn draw_track(painter: &egui::Painter, lane: Rect, ground_y: f32, to_screen: &impl Fn([f32; 2]) -> Pos2, x_range: (f32, f32), scale: f32, terrain: &[[f32; 2]], labels: bool) {
    let marks = if labels { scene::Marks::Labels } else { scene::Marks::Stakes };
    scene::track(painter, lane, ground_y, to_screen, x_range, scale, terrain, marks);
}

/// The sumo platform of an arena, with its hills.
fn draw_ring(painter: &egui::Painter, arena: &Arena, to_screen: &impl Fn([f32; 2]) -> Pos2, scale: f32) {
    scene::ring(painter, &arena.rules, &arena.terrain, to_screen, scale);
}

/// `alpha` < 255 draws faint ghosts (outline and eye fade too).
fn draw_creatures(painter: &egui::Painter, arena: &Arena, colors: &[Color32], to_screen: &impl Fn([f32; 2]) -> Pos2, scale: f32, alpha: u8) {
    scene::creatures(painter, arena, colors, to_screen, scale, alpha);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_race_runs_on_the_chosen_track() {
        let dir = std::env::temp_dir().join("arena-gui-race-test");
        std::fs::create_dir_all(&dir).unwrap();
        for c in ["worm", "walker"] {
            std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("creatures/{c}.toml")), dir.join(format!("{c}.toml"))).unwrap();
        }
        std::fs::write(dir.join("arena.toml"), "terrain_roughness = 0.2\nrace_time = 12.0\n").unwrap();
        let mut app = App::new(dir);
        // A random environment: every racer gets the track generated from the seed, the folder's other rules stay.
        app.race_choice = environment::TrackChoice { seeded: true, seed: 4711 };
        app.start_race();
        let track = environment::Track::random(4711);
        assert_eq!(app.race.len(), 2);
        assert!(app.race.iter().all(|r| environment::Track::of(&r.arena.rules) == track && r.arena.rules.race_time == 12.0));
        assert!(app.race_track.contains("4711"), "{}", app.race_track);
        // "New for every race" picks another seed when the race starts.
        app.race_new_each = true;
        app.start_race();
        assert_eq!(environment::Track::of(&app.race_rules), environment::Track::random(app.race_choice.seed));
        // Back to the folder's track.
        app.race_choice.seeded = false;
        app.start_race();
        assert_eq!(app.race[0].arena.rules.terrain_roughness, 0.2);
    }
}
