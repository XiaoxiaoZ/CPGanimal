//! The Cup: the class competition, a program of its own (the students'
//! program, arena-gui, does not have it). Loads a folder of creature files;
//! the ticked creatures are drawn into a knockout bracket (see cup.rs).
//!
//! Usage: `arena-cup [folder] [--fullscreen]` (default folder `creatures`).
//! Another folder can be picked with Browse… in the top bar. Files are
//! reloaded automatically when they change. F11 fills the screen, Esc leaves it.

mod cup;
// The look and the environment seeds are arena-gui's; not all of them are used here.
#[allow(dead_code)]
#[path = "../arena-gui/environment.rs"]
mod environment;
#[allow(dead_code)]
#[path = "../arena-gui/scene.rs"]
mod scene;
mod sound;
mod trophy;

use cpg_arena::creature::Rules;
use cpg_arena::game::{self, Entry};
use eframe::egui::{self, Align2, Color32, RichText, Sense, Vec2};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

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
    app.full = args.iter().any(|a| a == "--fullscreen");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 760.0]).with_title("CPG Arena Cup").with_fullscreen(app.full),
        ..Default::default()
    };
    eframe::run_native(
        "CPG Arena Cup",
        options,
        Box::new(|cc| {
            scene::install_fonts(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
}

/// The `creatures` folder: in the current directory, next to this program,
/// or in the source tree it was built from.
fn creatures_dir() -> PathBuf {
    let next_to_exe = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|d| d.join("creatures")));
    let paths: Vec<PathBuf> = [Some(PathBuf::from("creatures")), next_to_exe, Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("creatures"))].into_iter().flatten().collect();
    let found = paths.iter().find(|d| d.is_dir()).unwrap_or(&paths[paths.len() - 1]);
    std::path::absolute(found).unwrap_or_else(|_| found.clone())
}

struct App {
    folder: PathBuf,
    folder_text: String,
    rules: Rules,
    rules_error: Option<String>,
    entries: Vec<Entry>,
    stamp: Vec<(PathBuf, Option<std::time::SystemTime>)>,
    last_poll: f64,
    selected: HashSet<PathBuf>,
    speed: f32,
    paused: bool,
    cup: cup::Cup,
    /// Full screen: only the arena, for the projector.
    full: bool,
}

impl App {
    fn new(folder: PathBuf) -> Self {
        let folder = std::path::absolute(&folder).unwrap_or(folder);
        let mut app = Self {
            folder_text: folder.display().to_string(),
            folder,
            rules: Rules::default(),
            rules_error: None,
            entries: Vec::new(),
            stamp: Vec::new(),
            last_poll: 0.0,
            selected: HashSet::new(),
            speed: 1.0,
            paused: false,
            cup: cup::Cup::new(),
            full: false,
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
        self.reload(true);
    }

    /// Native folder picker, starting from the current folder when it exists.
    fn browse(&mut self) {
        let mut dialog = rfd::FileDialog::new().set_title("Choose the folder with the competitors");
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
        for e in &self.entries {
            // New files join the selection automatically.
            if e.creature.is_ok() && (select_all || !known.contains(&e.path)) {
                self.selected.insert(e.path.clone());
            }
        }
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

    /// The ticked creatures, as competitors.
    fn entrants(&self) -> Vec<cup::Entrant> {
        self.valid()
            .into_iter()
            .filter(|&i| self.selected.contains(&self.entries[i].path))
            .map(|i| {
                let name = self.entries[i].creature.as_ref().expect("valid entry").name.clone();
                cup::Entrant { name, path: self.entries[i].path.clone(), color: self.color(i) }
            })
            .collect()
    }

    fn set_full(&mut self, ctx: &egui::Context, on: bool) {
        self.full = on;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
    }

    /// In full screen, when the mouse moves: how to leave, and the keys.
    fn full_screen_controls(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.pointer.time_since_last_movement()) > 2.5 {
            return;
        }
        let mut leave = false;
        egui::Area::new(egui::Id::new("full_screen_controls")).anchor(Align2::RIGHT_TOP, [-12.0, 12.0]).order(egui::Order::Foreground).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                leave = ui.button("Leave full screen (Esc)").clicked();
                ui.label(RichText::new("Space: next match, or back to the bracket\nS: skip to the result").small());
            });
        });
        if leave {
            self.set_full(ctx, false);
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            ui.heading("CPG Arena Cup");
            ui.separator();
            ui.label("Folder");
            let r = ui.add(egui::TextEdit::singleline(&mut self.folder_text).desired_width(260.0));
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
            if ui.button("Full screen").on_hover_text("Only the arena, for the projector (F11). Esc to leave.").clicked() {
                self.set_full(ctx, true);
            }
            let label = if self.paused { "▶ Play" } else { "⏸ Pause" };
            if ui.button(label).clicked() {
                self.paused = !self.paused;
            }
            ui.add(egui::Slider::new(&mut self.speed, 0.25..=8.0).logarithmic(true).text("speed"));
        });
    }

    /// The creatures of the folder, to tick the competitors. No training
    /// scores: seeing them, the class would compare and guess the winners,
    /// and the suspense would be gone.
    fn creature_list(&mut self, ui: &mut egui::Ui) {
        ui.heading("Competitors");
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
                        if !c.author.is_empty() {
                            ui.label(RichText::new(format!("by {}", c.author)).small().weak());
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
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        let dt = ctx.input(|i| i.stable_dt).min(0.1) as f64;

        // Pick up files added or changed in the folder.
        if now - self.last_poll > 1.0 {
            self.last_poll = now;
            if game::folder_stamp(&self.folder) != self.stamp {
                self.reload(false);
            }
        }
        let playing = self.cup.advance(dt, self.speed, self.paused);

        // Keys: F11 full screen (Esc leaves it); Space / Enter for the next
        // match (or back to the bracket) and S to skip to the result.
        let (f11, esc, next, skip) = ctx.input(|i| {
            (i.key_pressed(egui::Key::F11), i.key_pressed(egui::Key::Escape), i.key_pressed(egui::Key::Space) || i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::S))
        });
        if f11 || (esc && self.full) {
            self.set_full(&ctx, !self.full);
        }
        // Not while typing (or while a focused button would take the key).
        let typing = ctx.memory(|m| m.focused().is_some());
        if self.full || !typing {
            if next {
                self.cup.key_next();
            }
            if skip {
                self.cup.key_skip();
            }
        }

        if !self.full {
            egui::Panel::top("top").show(ui, |ui| self.top_bar(ui, &ctx));
            egui::Panel::left("creatures").resizable(true).default_size(260.0).show(ui, |ui| self.creature_list(ui));
            egui::Panel::right("cup").resizable(true).default_size(300.0).show(ui, |ui| {
                let entrants = self.entrants();
                self.cup.panel(ui, &self.folder, &self.rules, &entrants);
            });
        }
        let central = if self.full { egui::CentralPanel::no_frame() } else { egui::CentralPanel::default() };
        central.show(ui, |ui| self.cup.view(ui, &self.rules, self.full));
        if self.full {
            self.full_screen_controls(&ctx);
        }

        if playing {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }
}
