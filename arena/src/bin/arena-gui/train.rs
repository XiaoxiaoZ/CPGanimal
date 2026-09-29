//! Train tab: runs the Python training scripts in `python/` as child
//! processes, streams their output, and plots best / mean fitness against
//! evaluations for every run of the session, so algorithms can be compared
//! on one chart.
//!
//! A script is started as
//! `python -u <script> <template> --out <file> --name <name> --mode <m> --level <l>
//! --budget <n> --population <n> --seed <n> [--opponents <folder>] [--rules <file>]`
//! (the built-in scripts get the subset they accept). Every output line with
//! `evaluations N` (or `duels N`) and `best X`, optionally `mean Y`, becomes a
//! point on the chart.
//!
//! "Watch the population": python/arena.py appends every population it rates
//! to the file in ARENA_WATCH (one JSON line per `evaluate` call, or per round
//! of co-evolution `fight`s), so any GA works without changes. Each such generation can be replayed with
//! every individual drawn faintly and the best one highlighted.
//!
//! The champion is saved as `<template>-<algorithm>-<date>-<time>.toml` and
//! named `<name> (<algorithm> <time>)`, so runs never overwrite each other and
//! can be raced against each other afterwards.

use super::{draw_creatures, draw_track, finished, PALETTE};
use clap::ValueEnum;
use cpg_arena::creature::{Creature, Gene, GenomeSpec, Level, Rules};
use cpg_arena::game::{self, Entry, Mode};
use cpg_arena::problem::Problem;
use cpg_arena::sim::Arena;
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Shape, Stroke, Vec2};
use rayon::prelude::*;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::Arc;

/// Opacity of the rest of the population behind the highlighted best.
const GHOST: u8 = 45;

/// Secondary text on the dark canvases (the panels take their colours from the theme).
const CANVAS_TEXT: Color32 = Color32::from_gray(165);

/// Numbers the watch files of this process (one per run, unique across trainers).
static WATCH_FILES: AtomicUsize = AtomicUsize::new(0);

#[derive(PartialEq, Clone, Copy)]
enum Script {
    Ga,
    MyGa,
    Structure,
    Coevolve,
    Random,
    Custom,
}

impl Script {
    const ALL: [Script; 6] = [Script::Ga, Script::MyGa, Script::Structure, Script::Coevolve, Script::Random, Script::Custom];

    fn label(self) -> &'static str {
        match self {
            Script::Ga => "Default GA (ga.py)",
            Script::MyGa => "My GA (my_ga.py)",
            Script::Structure => "Structure evolution",
            Script::Coevolve => "Co-evolution (sumo)",
            Script::Random => "Random search",
            Script::Custom => "Custom script…",
        }
    }

    fn file(self) -> &'static str {
        match self {
            Script::Ga => "ga.py",
            Script::MyGa => "my_ga.py",
            Script::Structure => "evolve_structure.py",
            Script::Coevolve => "coevolve.py",
            Script::Random => "random_search.py",
            Script::Custom => "",
        }
    }

    /// Short name for run labels, creature names and file names.
    fn tag(self) -> &'static str {
        match self {
            Script::Ga => "ga",
            Script::MyGa => "my_ga",
            Script::Structure => "structure",
            Script::Coevolve => "coevolve",
            Script::Random => "random",
            Script::Custom => "custom",
        }
    }

    // evolve_structure.py mutates the creature itself: no gene level, (mu + lambda) instead of a population.
    fn has_level(self) -> bool {
        self != Script::Structure
    }

    fn has_population(self) -> bool {
        self != Script::Structure
    }
}

/// What the lower half of the Train view shows.
#[derive(PartialEq, Clone, Copy)]
enum Bottom {
    Fitness,
    Dna,
    Output,
}

/// What a gene is decoded into.
#[derive(PartialEq, Clone, Copy)]
enum GeneKind {
    Movement,
    Body,
    Topology,
}

impl GeneKind {
    const ALL: [GeneKind; 3] = [GeneKind::Movement, GeneKind::Body, GeneKind::Topology];

    fn of(gene: &Gene) -> Self {
        match gene.name.rsplit('.').next().unwrap_or("") {
            "length" | "width" | "attach" | "angle" => GeneKind::Body,
            "exists" | "parent" => GeneKind::Topology,
            _ => GeneKind::Movement,
        }
    }

    fn label(self) -> &'static str {
        match self {
            GeneKind::Movement => "movement (the CPG)",
            GeneKind::Body => "body shape",
            GeneKind::Topology => "structure (which segments exist, where they attach)",
        }
    }

    // Checked with the dataviz palette validator (all pairs, dark mode, colour-vision deficiencies).
    fn color(self) -> Color32 {
        match self {
            GeneKind::Movement => Color32::from_rgb(0x19, 0x9e, 0x70),
            GeneKind::Body => Color32::from_rgb(0xd9, 0x59, 0x26),
            GeneKind::Topology => Color32::from_rgb(0x90, 0x85, 0xe9),
        }
    }
}

#[derive(PartialEq)]
enum Status {
    Running,
    Done,
    Stopped,
    Failed(String),
}

/// One line of the ARENA_WATCH file, written by python/arena.py.
#[derive(Deserialize)]
struct Record {
    /// "genomes" (Problem.evaluate) or "creatures" (Judge.evaluate)
    kind: String,
    mode: String,
    template: Option<PathBuf>,
    level: Option<String>,
    opponents: Option<PathBuf>,
    rules: Option<PathBuf>,
    #[serde(default)]
    genomes: Vec<Vec<f64>>,
    #[serde(default)]
    creatures: Vec<serde_json::Value>,
    /// None for rule breakers (-inf).
    fitness: Vec<Option<f64>>,
    /// Evaluations (or duels) the script has used so far.
    evaluations: Option<usize>,
}

/// Where a recorded population is replayed, and what its genes mean.
struct Setting {
    rules: Rules,
    sumo: bool,
    /// Sumo: everyone fights the first opponent the script used (default the Rock).
    opponent: Creature,
    /// How many opponents the fitness is averaged over.
    opponents: usize,
    /// Decodes genomes; None when the script evaluated whole creatures.
    problem: Option<Problem>,
    level: Level,
    genes: Vec<Gene>,
    /// The template as a genome: where evolution starts.
    start: Option<Vec<f64>>,
}

impl Setting {
    fn new(r: &Record) -> Result<Self, String> {
        let mode = <Mode as ValueEnum>::from_str(&r.mode, true)?;
        let sumo = mode == Mode::Sumo;
        if r.kind == "genomes" {
            let level = <Level as ValueEnum>::from_str(r.level.as_deref().unwrap_or("brain"), true)?;
            let template = r.template.as_deref().ok_or("no template in the record")?;
            let p = Problem::load(template, mode, level, r.opponents.as_deref(), r.rules.as_deref())?;
            let opponent = p.opponents.first().cloned().unwrap_or_else(game::rock);
            let (genes, start, opponents) = (p.spec.genes(&p.template, &p.rules), Some(p.start()), p.opponents.len().max(1));
            Ok(Self { rules: p.rules.clone(), sumo, opponent, opponents, problem: Some(p), level, genes, start })
        } else {
            let rules = match &r.rules {
                Some(p) => Rules::load_file(p)?,
                None => Rules::default(),
            };
            let found: Vec<Creature> = r
                .opponents
                .as_deref()
                .map(|d| game::load_folder(d, &rules).into_iter().filter_map(|e| e.creature.ok()).collect())
                .unwrap_or_default();
            let opponents = found.len().max(1);
            let opponent = found.into_iter().next().unwrap_or_else(game::rock);
            // Whole creatures are shown as structure-level genomes, whose genes don't depend on the creature.
            let genes = GenomeSpec { level: Level::Structure }.genes(&opponent, &rules);
            Ok(Self { rules, sumo, opponent, opponents, problem: None, level: Level::Structure, genes, start: None })
        }
    }
}

/// One evaluated population, in the order the script evaluated it.
struct Generation {
    setting: Arc<Setting>,
    creatures: Vec<Creature>,
    /// The genome of each creature (for whole creatures: encoded at the structure level).
    genomes: Vec<Vec<f64>>,
    fitness: Vec<f64>,
    best: usize,
    /// Evaluations used up to and including this generation.
    evaluations: usize,
}

struct Run {
    label: String,
    color: Color32,
    out: PathBuf,
    /// (evaluations, best, mean); mean is NaN when the script prints none.
    points: Vec<[f64; 3]>,
    /// Output lines, flagged when they came from stderr.
    log: Vec<(String, bool)>,
    child: Option<Child>,
    lines: Option<Receiver<(String, bool)>>,
    status: Status,
    watch: Option<PathBuf>,
    watch_read: u64,
    watch_partial: Vec<u8>,
    settings: HashMap<String, Option<Arc<Setting>>>,
    evaluated: usize,
    generations: Vec<Generation>,
    /// Temporary folder with copies of the chosen sumo opponents.
    opponents: Option<PathBuf>,
}

impl Run {
    fn active(&self) -> bool {
        self.child.is_some() || self.lines.is_some()
    }

    fn push(&mut self, line: String, stderr: bool) {
        if let Some(p) = progress(&line) {
            self.points.push(p);
        }
        if self.log.len() >= 5000 {
            self.log.drain(..1000);
        }
        self.log.push((line, stderr));
    }

    fn poll(&mut self) {
        let was_active = self.active();
        let mut got = Vec::new();
        if let Some(rx) = &self.lines {
            loop {
                match rx.try_recv() {
                    Ok(l) => got.push(l),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.lines = None;
                        break;
                    }
                }
            }
        }
        for (line, stderr) in got {
            self.push(line, stderr);
        }
        if let Some(child) = &mut self.child {
            if let Ok(Some(code)) = child.try_wait() {
                self.child = None;
                if self.status == Status::Running {
                    self.status = if code.success() {
                        Status::Done
                    } else {
                        Status::Failed(code.code().map_or("killed".into(), |c| format!("exit code {c}")))
                    };
                }
            }
        }
        if was_active {
            self.read_watch();
        }
    }

    /// Take in the populations the script has evaluated since the last call.
    fn read_watch(&mut self) {
        let Some(path) = &self.watch else { return };
        let Ok(mut f) = File::open(path) else { return };
        let mut new = Vec::new();
        if f.seek(SeekFrom::Start(self.watch_read)).is_err() || f.read_to_end(&mut new).is_err() {
            return;
        }
        self.watch_read += new.len() as u64;
        self.watch_partial.extend_from_slice(&new);
        while let Some(i) = self.watch_partial.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.watch_partial.drain(..=i).collect();
            match serde_json::from_slice::<Record>(&line) {
                Ok(r) => self.add_generation(r),
                Err(e) => self.push(format!("(watch) unreadable population: {e}"), true),
            }
        }
    }

    fn add_generation(&mut self, r: Record) {
        self.evaluated = r.evaluations.unwrap_or(self.evaluated + r.genomes.len().max(r.creatures.len()));
        let key = format!("{}|{}|{:?}|{:?}|{:?}|{:?}", r.kind, r.mode, r.template, r.level, r.opponents, r.rules);
        let setting = match self.settings.get(&key) {
            Some(s) => s.clone(),
            None => {
                let s = Setting::new(&r).map(Arc::new);
                if let Err(e) = &s {
                    self.push(format!("(watch) cannot replay this population: {e}"), true);
                }
                self.settings.insert(key, s.as_ref().ok().cloned());
                s.ok()
            }
        };
        let Some(setting) = setting else { return };
        let individuals: Vec<Option<(Creature, Vec<f64>)>> = match &setting.problem {
            Some(p) => r.genomes.into_iter().map(|g| Some((p.decode(&g).ok()?, g))).collect(),
            None => {
                let spec = GenomeSpec { level: Level::Structure };
                r.creatures
                    .into_iter()
                    .map(|v| {
                        let c = serde_json::from_value::<Creature>(v).ok().filter(|c| c.validate(&setting.rules).is_ok())?;
                        let g = spec.encode(&c, &setting.rules);
                        Some((c, g))
                    })
                    .collect()
            }
        };
        let (mut creatures, mut genomes, mut fitness) = (Vec::new(), Vec::new(), Vec::new());
        for (individual, f) in individuals.into_iter().zip(r.fitness) {
            if let (Some((c, g)), Some(f)) = (individual, f) {
                creatures.push(c);
                genomes.push(g);
                fitness.push(f);
            }
        }
        if !creatures.is_empty() {
            let best = (0..fitness.len()).max_by(|&a, &b| fitness[a].total_cmp(&fitness[b])).unwrap_or(0);
            self.generations.push(Generation { setting, creatures, genomes, fitness, best, evaluations: self.evaluated });
        }
    }

    fn stop(&mut self) {
        if let Some(c) = &mut self.child {
            c.kill().ok();
            self.status = Status::Stopped;
        }
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        if let Some(c) = &mut self.child {
            c.kill().ok();
            c.wait().ok();
        }
        if let Some(w) = &self.watch {
            std::fs::remove_file(w).ok();
        }
        if let Some(d) = &self.opponents {
            std::fs::remove_dir_all(d).ok();
        }
    }
}

/// A generation being played back, every individual in its own world.
struct Replay {
    run: usize,
    gen: usize,
    arenas: Vec<Arena>,
    accumulator: f64,
    /// Seconds left showing the final frame before the next playback.
    hold: Option<f64>,
}


/// Parse a progress line such as `gen  3  evaluations  150  best  2.345  mean  1.2`.
fn progress(line: &str) -> Option<[f64; 3]> {
    let words: Vec<&str> = line.split_whitespace().collect();
    let num = |key: &str| words.windows(2).find(|w| w[0] == key).and_then(|w| w[1].parse::<f64>().ok());
    let x = num("evaluations").or_else(|| num("duels"))?;
    Some([x, num("best")?, num("mean").unwrap_or(f64::NAN)])
}

/// Send a child's output to the UI line by line.
fn forward(stream: impl Read + Send + 'static, tx: Sender<(String, bool)>, stderr: bool) {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        let mut buf = Vec::new();
        while matches!(reader.read_until(b'\n', &mut buf), Ok(n) if n > 0) {
            let line = String::from_utf8_lossy(&buf).trim_end().to_string();
            if tx.send((line, stderr)).is_err() {
                break;
            }
            buf.clear();
        }
    });
}

fn absolute(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

/// `pattern` with every `0` standing for a digit.
fn matches_digits(s: &str, pattern: &str) -> bool {
    s.len() == pattern.len() && s.bytes().zip(pattern.bytes()).all(|(c, p)| if p == b'0' { c.is_ascii_digit() } else { c == p })
}

/// "Worm (ga 14:02:31)" → "Worm", so training a trained creature doesn't stack stamps.
fn strip_name_stamp(name: &str) -> &str {
    let Some(inner) = name.strip_suffix(')') else { return name };
    let Some(open) = inner.rfind(" (") else { return name };
    let time = inner.len().checked_sub(8).and_then(|i| inner.get(i..));
    if time.is_some_and(|t| matches_digits(t, "00:00:00")) { &name[..open] } else { name }
}

/// "worm-ga-20260929-140231" → "worm".
fn strip_file_stamp(stem: &str) -> &str {
    let Some(cut) = stem.len().checked_sub(16).filter(|&i| stem.get(i..).is_some_and(|t| matches_digits(t, "-00000000-000000"))) else {
        return stem;
    };
    let rest = &stem[..cut];
    rest.rfind('-').map_or(rest, |i| &rest[..i])
}

/// Sequential blue for a gene value in [0, 1]. 0 fades into the background
/// (dark on a dark theme, light on a light one), 1 stands out.
fn ramp(v: f64, dark: bool) -> Color32 {
    let v = if dark { v } else { 1.0 - v };
    const STEPS: [[u8; 3]; 13] = [
        [13, 54, 107],
        [16, 66, 129],
        [24, 79, 149],
        [28, 92, 171],
        [37, 106, 191],
        [42, 120, 214],
        [57, 135, 229],
        [85, 152, 231],
        [109, 167, 236],
        [134, 182, 239],
        [158, 197, 244],
        [183, 211, 246],
        [205, 226, 251],
    ];
    let x = v.clamp(0.0, 1.0) * 12.0;
    let i = (x.floor() as usize).min(11);
    let t = x - i as f64;
    let mix = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * t).round() as u8;
    let (a, b) = (STEPS[i], STEPS[i + 1]);
    Color32::from_rgb(mix(a[0], b[0]), mix(a[1], b[1]), mix(a[2], b[2]))
}

/// A gene value as the quantity it is decoded into, e.g. "1.20 Hz", "35°", "0.40 m".
fn physical(gene: &Gene, x: f64) -> String {
    let v = gene.lo + x.clamp(0.0, 1.0) * (gene.hi - gene.lo);
    match gene.name.rsplit('.').next().unwrap_or("") {
        "frequency" => format!("{v:.2} Hz"),
        "amplitude" | "offset" | "phase" | "angle" => format!("{v:.0}°"),
        "length" | "width" => format!("{v:.2} m"),
        "exists" => (if x > 0.5 { "yes" } else { "no" }).to_string(),
        _ => format!("{v:.2}"),
    }
}

/// A creature as the dict a Python script holds: one segment per line, numbers rounded for reading.
fn creature_text(c: &Creature) -> String {
    fn round(v: &mut serde_json::Value) {
        match v {
            serde_json::Value::Number(n) if n.is_f64() => {
                if let Some(x) = n.as_f64() {
                    *v = serde_json::json!((x * 1000.0).round() / 1000.0);
                }
            }
            serde_json::Value::Array(a) => a.iter_mut().for_each(round),
            serde_json::Value::Object(o) => o.values_mut().for_each(round),
            _ => {}
        }
    }
    let Ok(mut v) = serde_json::to_value(c) else { return String::new() };
    round(&mut v);
    let Some(map) = v.as_object() else { return v.to_string() };
    let entries: Vec<String> = map
        .iter()
        .map(|(k, val)| match val.as_array() {
            Some(items) if k == "segment" => format!("  \"{k}\": [\n{}\n  ]", items.iter().map(|s| format!("    {s}")).collect::<Vec<_>>().join(",\n")),
            _ => format!("  \"{k}\": {val}"),
        })
        .collect();
    format!("{{\n{}\n}}", entries.join(",\n"))
}

/// A genome as the list a Python GA holds.
fn genome_text(g: &[f64]) -> String {
    format!("[{}]", g.iter().map(|x| format!("{x:.3}")).collect::<Vec<_>>().join(", "))
}

/// Standard deviation of gene `i` across a population.
fn spread(genomes: &[Vec<f64>], i: usize) -> f64 {
    let n = genomes.len() as f64;
    let mean = genomes.iter().map(|g| g[i].clamp(0.0, 1.0)).sum::<f64>() / n;
    (genomes.iter().map(|g| (g[i].clamp(0.0, 1.0) - mean).powi(2)).sum::<f64>() / n).sqrt()
}

/// Smoothly move a camera towards (centre x in m, pixels per m).
fn follow(camera: &mut Option<(f32, f32)>, target: (f32, f32)) -> (f32, f32) {
    let cam = match *camera {
        Some((x, s)) => (x + (target.0 - x) * 0.1, s + (target.1 - s) * 0.1),
        None => target,
    };
    *camera = Some(cam);
    cam
}

pub struct Trainer {
    python: String,
    /// The `python/` folder with arena.py, ga.py, my_ga.py, ...
    scripts: Option<PathBuf>,
    /// The `arena` binary next to this one, so Python uses the same build.
    arena_bin: Option<PathBuf>,
    script: Script,
    custom: String,
    template: Option<PathBuf>,
    mode: &'static str,
    level: &'static str,
    budget: u32,
    generations: u32,
    population: u32,
    seed: u32,
    /// Sumo: fight the Rock, or the creatures ticked in `chosen`.
    vs_rock: bool,
    chosen: HashSet<PathBuf>,
    /// Where champions are saved: the creatures folder.
    save_dir: PathBuf,
    /// Champion name without the time stamp.
    name: String,
    extra: String,
    runs: Vec<Run>,
    shown: Option<usize>,
    started: usize,
    /// Replay the recorded populations.
    watch: bool,
    /// Jump to the newest generation after each playback.
    follow: bool,
    pick: usize,
    replay: Option<Replay>,
    camera: Option<(f32, f32)>,
    bottom: Bottom,
}

impl Trainer {
    pub fn new(save_dir: PathBuf) -> Self {
        let scripts = [PathBuf::from("python"), Path::new(env!("CARGO_MANIFEST_DIR")).join("python")]
            .into_iter()
            .find(|d| d.join("arena.py").is_file())
            .map(|d| absolute(&d));
        let arena_bin = std::env::current_exe()
            .ok()
            .map(|exe| exe.with_file_name(format!("arena{}", std::env::consts::EXE_SUFFIX)))
            .filter(|p| p.is_file());
        Self {
            python: if cfg!(windows) { "python" } else { "python3" }.into(),
            scripts,
            arena_bin,
            script: Script::Ga,
            custom: String::new(),
            template: None,
            mode: "race",
            level: "brain",
            budget: 1000,
            generations: 30,
            population: 50,
            seed: 1,
            vs_rock: true,
            chosen: HashSet::new(),
            save_dir,
            name: String::new(),
            extra: String::new(),
            runs: Vec::new(),
            shown: None,
            started: 0,
            watch: true,
            follow: true,
            pick: 0,
            replay: None,
            camera: None,
            bottom: Bottom::Fitness,
        }
    }

    /// Drain the output of running scripts; true while any is still going.
    pub fn poll(&mut self) -> bool {
        for r in &mut self.runs {
            let failed = matches!(r.status, Status::Failed(_));
            r.poll();
            if !failed && matches!(r.status, Status::Failed(_)) {
                // Show the Python error (e.g. in a student's own GA).
                self.bottom = Bottom::Output;
            }
        }
        self.runs.iter().any(Run::active)
    }

    fn busy(&self) -> bool {
        self.runs.iter().any(|r| r.child.is_some())
    }

    fn sumo(&self) -> bool {
        self.script == Script::Coevolve || self.mode == "sumo"
    }

    fn script_path(&self) -> Result<PathBuf, String> {
        if self.script == Script::Custom {
            let p = PathBuf::from(self.custom.trim());
            return if p.is_file() { Ok(absolute(&p)) } else { Err("Pick your .py script with Browse….".into()) };
        }
        let dir = self.scripts.as_ref().ok_or("python/ folder not found: start arena-gui from the arena folder.")?;
        Ok(dir.join(self.script.file()))
    }

    fn tag(&self) -> String {
        match self.script {
            Script::Custom => Path::new(self.custom.trim()).file_stem().map_or("custom".into(), |s| s.to_string_lossy().into_owned()),
            s => s.tag().into(),
        }
    }

    /// Default champion name: the template's, without an earlier time stamp.
    fn reset_name(&mut self, entries: &[Entry]) {
        let Some(t) = &self.template else { return };
        if let Some(c) = entries.iter().find(|e| &e.path == t).and_then(|e| e.creature.as_ref().ok()) {
            self.name = strip_name_stamp(&c.name).to_string();
        }
    }

    /// Train this creature next, e.g. one just made in the Design tab.
    pub fn use_template(&mut self, path: &Path, entries: &[Entry]) {
        if let Some(e) = entries.iter().find(|e| e.path.file_name() == path.file_name()) {
            self.template = Some(e.path.clone());
            self.reset_name(entries);
        }
    }

    /// The Train form and the list of runs. Returns true when the user asks to open the creatures folder.
    pub fn panel(&mut self, ui: &mut egui::Ui, folder: &Path, entries: &[Entry]) -> bool {
        let mut open_creatures = false;
        ui.heading("Train");
        ui.label("Evolve a creature with a Python script. Each champion is saved as a new creature in the creatures folder, with the time in its name.");
        let valid: Vec<&Entry> = entries.iter().filter(|e| e.creature.is_ok()).collect();
        let name_of = |e: &Entry| e.creature.as_ref().map_or(String::new(), |c| c.name.clone());
        if !valid.iter().any(|e| Some(&e.path) == self.template.as_ref()) {
            self.template = valid.first().map(|e| e.path.clone());
            self.reset_name(entries);
        }

        let mut new_template = false;
        egui::Grid::new("train_form").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            ui.label("Algorithm");
            egui::ComboBox::from_id_salt("script").width(170.0).selected_text(self.script.label()).show_ui(ui, |ui| {
                for s in Script::ALL {
                    ui.selectable_value(&mut self.script, s, s.label());
                }
            });
            ui.end_row();
            if self.script == Script::Custom {
                ui.label("Script");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.custom).desired_width(110.0));
                    if ui.button("Browse…").clicked() {
                        let mut dialog = rfd::FileDialog::new().set_title("Choose your GA script").add_filter("Python", &["py"]);
                        if let Some(dir) = &self.scripts {
                            dialog = dialog.set_directory(dir);
                        }
                        if let Some(p) = dialog.pick_file() {
                            self.custom = p.display().to_string();
                        }
                    }
                });
                ui.end_row();
            }
            ui.label("Template");
            let current = valid.iter().find(|e| Some(&e.path) == self.template.as_ref()).map_or("-".into(), |e| name_of(e));
            egui::ComboBox::from_id_salt("template").width(170.0).selected_text(current).show_ui(ui, |ui| {
                for e in &valid {
                    new_template |= ui.selectable_value(&mut self.template, Some(e.path.clone()), name_of(e)).changed();
                }
            });
            ui.end_row();
            ui.label("Mode");
            ui.horizontal(|ui| {
                if self.script == Script::Coevolve {
                    ui.label("sumo");
                } else {
                    ui.selectable_value(&mut self.mode, "race", "race");
                    ui.selectable_value(&mut self.mode, "sumo", "sumo");
                }
            });
            ui.end_row();
            if self.script.has_level() {
                ui.label("Level");
                ui.horizontal(|ui| {
                    for l in ["brain", "body", "structure"] {
                        ui.selectable_value(&mut self.level, l, l);
                    }
                });
                ui.end_row();
            }
            if self.script == Script::Coevolve {
                ui.label("Generations");
                ui.add(egui::DragValue::new(&mut self.generations).range(1..=1000));
            } else {
                ui.label("Budget");
                ui.add(egui::DragValue::new(&mut self.budget).range(50..=200_000).speed(10).suffix(" evaluations"));
            }
            ui.end_row();
            if self.script.has_population() {
                ui.label("Population");
                ui.add(egui::DragValue::new(&mut self.population).range(4..=2000));
                ui.end_row();
            }
            ui.label("Seed");
            ui.add(egui::DragValue::new(&mut self.seed));
            ui.end_row();
            if self.sumo() {
                ui.label(if self.script == Script::Coevolve { "Benchmark" } else { "Opponents" })
                    .on_hover_text("Who the creatures fight (co-evolution: who measures real progress)");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.vs_rock, true, "the Rock");
                    ui.selectable_value(&mut self.vs_rock, false, "choose…");
                });
                ui.end_row();
            }
            ui.label("Name");
            ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(170.0)).on_hover_text("The algorithm and time are added, e.g. \"Worm (ga 14:02:31)\".");
            ui.end_row();
            ui.label("Extra args");
            ui.add(egui::TextEdit::singleline(&mut self.extra).desired_width(170.0).hint_text("e.g. --sigma 0.2"));
            ui.end_row();
            ui.label("Python");
            ui.add(egui::TextEdit::singleline(&mut self.python).desired_width(170.0));
            ui.end_row();
        });
        if new_template {
            self.reset_name(entries);
        }
        if self.script.has_level() && self.level == "structure" {
            ui.label(
                RichText::new(
                    "At the structure level the template is 1 of the starting population and is soon forgotten: \
                     with the same seed, every template ends up as the same creature. To evolve the structure \
                     of your own design, use Structure evolution.",
                )
                .small()
                .weak(),
            );
            if ui.small_button("Use Structure evolution").clicked() {
                self.script = Script::Structure;
            }
        }
        let choosing = self.sumo() && !self.vs_rock;
        if choosing {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Tick who to fight (the fitness is the average):").small());
                if ui.small_button("all").clicked() {
                    self.chosen = valid.iter().map(|e| e.path.clone()).filter(|p| Some(p) != self.template.as_ref()).collect();
                }
                if ui.small_button("none").clicked() {
                    self.chosen.clear();
                }
            });
            egui::ScrollArea::vertical().id_salt("opponents").max_height(160.0).show(ui, |ui| {
                for e in &valid {
                    let itself = Some(&e.path) == self.template.as_ref();
                    let mut on = self.chosen.contains(&e.path) && !itself;
                    let label = if itself { format!("{} (itself)", name_of(e)) } else { name_of(e) };
                    if ui.add_enabled(!itself, egui::Checkbox::new(&mut on, label)).changed() {
                        if on {
                            self.chosen.insert(e.path.clone());
                        } else {
                            self.chosen.remove(&e.path);
                        }
                    }
                }
            });
        }
        let ticked = valid.iter().filter(|e| self.chosen.contains(&e.path) && Some(&e.path) != self.template.as_ref()).count();
        ui.checkbox(&mut self.watch, "Watch the population while training")
            .on_hover_text("Replay each generation: every individual drawn faintly, the best one highlighted.");

        let rules = folder.join("arena.toml").is_file();
        ui.label(RichText::new(if rules { "Rules: arena.toml in this folder" } else { "Rules: defaults (no arena.toml in this folder)" }).small().weak());
        let problem = if valid.is_empty() {
            Some("No valid creature in this folder to start from.".to_string())
        } else if choosing && ticked == 0 {
            Some("Tick at least one opponent.".to_string())
        } else {
            self.script_path().err()
        };
        ui.horizontal(|ui| {
            let busy = self.busy();
            if ui.add_enabled(problem.is_none() && !busy, egui::Button::new("▶ Train")).clicked() {
                self.start(folder);
            }
            if ui.add_enabled(busy, egui::Button::new("■ Stop")).clicked() {
                for r in &mut self.runs {
                    r.stop();
                }
            }
        });
        if let Some(p) = problem {
            ui.colored_label(Color32::LIGHT_RED, p);
        }
        if !super::same_dir(folder, &self.save_dir) {
            ui.label(RichText::new("You are looking at another folder: new champions go to the creatures folder.").small().weak());
            open_creatures = ui.small_button("Open the creatures folder").clicked();
        }

        ui.separator();
        ui.horizontal(|ui| {
            ui.label(RichText::new("Runs").strong());
            if ui.small_button("clear finished").clicked() {
                self.runs.retain(Run::active);
                self.shown = self.runs.len().checked_sub(1);
                self.replay = None;
            }
        });
        let mut remove = None;
        egui::ScrollArea::vertical().id_salt("runs").show(ui, |ui| {
            for (k, r) in self.runs.iter().enumerate().rev() {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                    ui.painter().rect_filled(rect.shrink(1.0), 2.0, r.color);
                    let file = r.out.file_name().map_or(String::new(), |f| f.to_string_lossy().into_owned());
                    if ui.selectable_label(self.shown == Some(k), RichText::new(&r.label).small()).on_hover_text(format!("saves {file}")).clicked() {
                        self.shown = Some(k);
                    }
                    if ui.small_button("×").on_hover_text("remove (stops it if running)").clicked() {
                        remove = Some(k);
                    }
                });
                let best = r.points.last().map_or(String::new(), |p| format!(" · best {:.3}", p[1]));
                ui.horizontal(|ui| {
                    ui.add_space(18.0);
                    match &r.status {
                        Status::Running => {
                            ui.spinner();
                            ui.label(RichText::new(format!("running{best}")).small());
                        }
                        Status::Done => {
                            ui.label(RichText::new(format!("done{best}")).small().color(Color32::from_rgb(120, 180, 120)));
                        }
                        Status::Stopped => {
                            ui.label(RichText::new(format!("stopped{best}")).small().weak());
                        }
                        Status::Failed(m) => {
                            ui.label(RichText::new(format!("failed: {m}")).small().color(Color32::LIGHT_RED));
                        }
                    }
                });
            }
        });
        if let Some(k) = remove {
            self.runs.remove(k);
            self.shown = self.runs.len().checked_sub(1);
            self.replay = None;
        }
        open_creatures
    }

    fn start(&mut self, folder: &Path) {
        let (Some(template), Ok(script)) = (self.template.clone(), self.script_path()) else { return };
        let folder = absolute(folder);
        let now = chrono::Local::now();
        let tag = self.tag();
        let stem = template.file_stem().map_or(String::new(), |s| s.to_string_lossy().into_owned());
        let stem = strip_file_stamp(&stem);
        let stamp = now.format("%Y%m%d-%H%M%S");
        std::fs::create_dir_all(&self.save_dir).ok();
        let mut out = self.save_dir.join(format!("{stem}-{tag}-{stamp}.toml"));
        for k in 2.. {
            if !out.exists() {
                break;
            }
            out = self.save_dir.join(format!("{stem}-{tag}-{stamp}-{k}.toml"));
        }
        // Chosen sumo opponents: copies in a folder of their own, the form the scripts take.
        let opponents = (self.sumo() && !self.vs_rock).then(|| {
            let dir = std::env::temp_dir().join(format!("arena-gui-opponents-{}-{}", std::process::id(), WATCH_FILES.fetch_add(1, Ordering::Relaxed)));
            std::fs::remove_dir_all(&dir).ok();
            std::fs::create_dir_all(&dir).ok();
            for p in self.chosen.iter().filter(|p| **p != template) {
                if let Some(file) = p.file_name() {
                    std::fs::copy(p, dir.join(file)).ok();
                }
            }
            dir
        });
        let name = format!("{} ({tag} {})", self.name.trim(), now.format("%H:%M:%S"));

        let mut args: Vec<OsString> =
            vec!["-u".into(), script.clone().into(), absolute(&template).into(), "--out".into(), out.clone().into(), "--name".into(), name.into()];
        let mut flag = |key: &str, value: OsString| {
            args.push(key.into());
            args.push(value);
        };
        if self.script == Script::Coevolve {
            flag("--generations", self.generations.to_string().into());
        } else {
            flag("--mode", self.mode.into());
            flag("--budget", self.budget.to_string().into());
        }
        if self.script.has_level() {
            flag("--level", self.level.into());
        }
        if self.script.has_population() {
            flag("--population", self.population.to_string().into());
        }
        flag("--seed", self.seed.to_string().into());
        if let Some(dir) = &opponents {
            flag(if self.script == Script::Coevolve { "--benchmark" } else { "--opponents" }, dir.clone().into());
        }
        let rules = folder.join("arena.toml");
        if rules.is_file() {
            flag("--rules", rules.into());
        }
        args.extend(self.extra.split_whitespace().map(OsString::from));

        // Populations are always recorded, so Watch can also be switched on afterwards.
        let watch = std::env::temp_dir().join(format!("arena-gui-{}-{}.jsonl", std::process::id(), WATCH_FILES.fetch_add(1, Ordering::Relaxed)));
        std::fs::remove_file(&watch).ok();
        let mut cmd = Command::new(self.python.trim());
        cmd.args(&args)
            .env("PYTHONUTF8", "1")
            .env("ARENA_WATCH", &watch)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Run from the arena folder like the README commands, and let a script
        // anywhere `from arena import Problem` or `import ga`.
        if let Some(dir) = &self.scripts {
            if let Some(root) = dir.parent() {
                cmd.current_dir(root);
            }
            let old = std::env::var_os("PYTHONPATH").unwrap_or_default();
            let paths = std::iter::once(dir.clone()).chain(std::env::split_paths(&old).filter(|p| !p.as_os_str().is_empty()));
            if let Ok(p) = std::env::join_paths(paths) {
                cmd.env("PYTHONPATH", p);
            }
        } else if let Some(dir) = script.parent() {
            cmd.current_dir(dir);
        }
        if let Some(bin) = &self.arena_bin {
            cmd.env("ARENA_BIN", bin);
        }

        let mut label = format!("{tag} · {stem} · {}", if self.sumo() { "sumo" } else { "race" });
        if let Some(n) = opponents.as_ref().and_then(|d| std::fs::read_dir(d).ok()).map(|d| d.count()) {
            label += &format!(" vs {n}");
        }
        if self.script.has_level() && self.level != "brain" {
            label += &format!(" · {}", self.level);
        }
        label += &format!(" · seed {} · {}", self.seed, now.format("%H:%M:%S"));
        let c = PALETTE[self.started % PALETTE.len()];
        self.started += 1;
        let mut run = Run {
            label,
            color: Color32::from_rgb(c[0], c[1], c[2]),
            out,
            points: Vec::new(),
            log: Vec::new(),
            child: None,
            lines: None,
            status: Status::Running,
            watch: Some(watch),
            watch_read: 0,
            watch_partial: Vec::new(),
            settings: HashMap::new(),
            evaluated: 0,
            generations: Vec::new(),
            opponents,
        };
        let shown: Vec<String> = args
            .iter()
            .map(|a| {
                let a = a.to_string_lossy();
                if a.contains(' ') { format!("\"{a}\"") } else { a.into_owned() }
            })
            .collect();
        run.push(format!("> {} {}", self.python.trim(), shown.join(" ")), false);
        match cmd.spawn() {
            Ok(mut child) => {
                let (tx, rx) = channel();
                if let Some(s) = child.stdout.take() {
                    forward(s, tx.clone(), false);
                }
                if let Some(s) = child.stderr.take() {
                    forward(s, tx, true);
                }
                run.child = Some(child);
                run.lines = Some(rx);
            }
            Err(e) => {
                run.push(format!("could not start `{}`: {e}", self.python.trim()), true);
                run.push("Is Python installed? Set the Python command in the form (python, python3 or py).".into(), true);
                run.status = Status::Failed("could not start Python".into());
                self.bottom = Bottom::Output;
            }
        }
        self.runs.push(run);
        self.shown = Some(self.runs.len() - 1);
        self.follow = true;
        self.replay = None;
    }

    fn play(&mut self, run: usize, gen: usize) {
        let g = &self.runs[run].generations[gen];
        let s = &g.setting;
        let arenas = g
            .creatures
            .par_iter()
            .map(|c| if s.sumo { Arena::sumo(c, &s.opponent, &s.rules) } else { Arena::race(c, &s.rules) })
            .collect();
        self.replay = Some(Replay { run, gen, arenas, accumulator: 0.0, hold: None });
        self.camera = None;
    }

    /// Step the population playback (Watch); true while there is one to show.
    pub fn advance(&mut self, dt: f64, speed: f32, paused: bool) -> bool {
        let n = self.shown.and_then(|k| self.runs.get(k)).map_or(0, |r| r.generations.len());
        let (Some(k), true, true) = (self.shown, self.watch, n > 0) else {
            self.replay = None;
            return false;
        };
        let target = if self.follow { n - 1 } else { self.pick.min(n - 1) };
        if self.replay.as_ref().is_none_or(|r| r.run != k || (!self.follow && r.gen != target)) {
            self.play(k, target);
        }
        let replay = self.replay.as_mut().expect("playback was just started");
        if let Some(hold) = &mut replay.hold {
            *hold -= dt;
            if *hold <= 0.0 {
                let gen = if self.follow { n - 1 } else { replay.gen };
                self.play(k, gen);
            }
            return true;
        }
        if paused {
            return true;
        }
        let sumo = self.runs[k].generations[replay.gen].setting.sumo;
        let step = replay.arenas.first().map_or(0.01, |a| a.rules.dt);
        replay.accumulator += dt * speed as f64;
        // At most 60 physics steps a frame; beyond that the playback slows down instead of freezing the UI.
        let steps = ((replay.accumulator / step) as usize).min(60);
        replay.accumulator = if steps == 60 { 0.0 } else { replay.accumulator - steps as f64 * step };
        replay.arenas.par_iter_mut().for_each(|a| {
            for _ in 0..steps {
                if finished(a, sumo) {
                    break;
                }
                a.step();
            }
        });
        if replay.arenas.iter().all(|a| finished(a, sumo)) {
            replay.hold = Some(1.5);
        }
        true
    }

    /// Population playback (when watching), then the Fitness / DNA / Output views.
    pub fn view(&mut self, ui: &mut egui::Ui) {
        if self.watch {
            // Smaller while the DNA table needs the room.
            let share = if self.bottom == Bottom::Dna { 0.3 } else { 0.42 };
            let size = Vec2::new(ui.available_width(), (ui.available_height() * share).max(160.0));
            let (resp, painter) = ui.allocate_painter(size, Sense::hover());
            self.draw_population(&painter, resp.rect);
        }
        if self.watch || self.bottom == Bottom::Dna {
            self.generation_bar(ui);
        }
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.bottom, Bottom::Fitness, "Fitness");
            ui.selectable_value(&mut self.bottom, Bottom::Dna, "DNA");
            ui.selectable_value(&mut self.bottom, Bottom::Output, "Output");
        });
        match self.bottom {
            Bottom::Fitness => {
                let (resp, painter) = ui.allocate_painter(ui.available_size().max(Vec2::new(0.0, 140.0)), Sense::hover());
                self.chart(&painter, resp.rect);
            }
            Bottom::Dna => self.dna(ui),
            Bottom::Output => self.output(ui),
        }
    }

    fn output(&self, ui: &mut egui::Ui) {
        let Some(run) = self.shown.and_then(|k| self.runs.get(k)) else {
            ui.label(RichText::new("The output of the selected run appears here.").weak());
            return;
        };
        ui.label(RichText::new(&run.label).color(run.color).strong());
        let row = ui.text_style_height(&egui::TextStyle::Monospace);
        egui::ScrollArea::both().id_salt("log").auto_shrink(false).stick_to_bottom(true).show_rows(ui, row, run.log.len(), |ui, range| {
            for (line, stderr) in &run.log[range] {
                let color = if *stderr { Color32::LIGHT_RED } else { Color32::from_gray(200) };
                ui.add(egui::Label::new(RichText::new(line).monospace().color(color)).extend());
            }
        });
    }

    /// How the numbers are decoded, the raw genomes, and every gene across the population and over time.
    fn dna(&self, ui: &mut egui::Ui) {
        let Some((k, gen)) = self.selected() else {
            ui.label(RichText::new("Train something to see its DNA: the list of numbers the GA changes.").weak());
            return;
        };
        let run = &self.runs[k];
        egui::ScrollArea::vertical().id_salt("dna").auto_shrink(false).show(ui, |ui| {
            decoding(ui, &run.generations[gen].setting);
            ui.add_space(8.0);
            raw_data(ui, run, gen);
            ui.add_space(8.0);
            gene_table(ui, run, gen);
        });
    }

    /// The run and generation shown by the playback and the DNA view.
    fn selected(&self) -> Option<(usize, usize)> {
        let k = self.shown?;
        let n = self.runs.get(k)?.generations.len();
        let gen = match &self.replay {
            _ if n == 0 => return None,
            Some(r) if r.run == k => r.gen,
            _ if self.follow => n - 1,
            _ => self.pick.min(n - 1),
        };
        Some((k, gen))
    }

    fn generation_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Generation");
            let Some((k, mut g)) = self.selected() else {
                ui.label(RichText::new("none recorded yet").weak());
                return;
            };
            let n = self.runs[k].generations.len();
            if ui.add(egui::Slider::new(&mut g, 0..=n - 1)).changed() {
                self.pick = g;
                self.follow = false;
            }
            let was = self.follow;
            ui.checkbox(&mut self.follow, "follow the latest");
            if was && !self.follow {
                self.pick = g;
            }
            if self.watch && ui.button("⟳ Restart").clicked() {
                self.replay = None;
            }
        });
    }

    fn draw_population(&mut self, painter: &egui::Painter, rect: Rect) {
        let painter = painter.with_clip_rect(rect);
        painter.rect_filled(rect, 0.0, Color32::from_rgb(28, 32, 40));
        let Some(r) = &self.replay else {
            let hint = if self.shown.is_some() { "No population recorded yet." } else { "Press ▶ Train to watch the population evolve." };
            painter.text(rect.center(), Align2::CENTER_CENTER, hint, FontId::proportional(16.0), Color32::from_gray(160));
            return;
        };
        let run = &self.runs[r.run];
        let g = &run.generations[r.gen];
        let s = &g.setting;
        let best = g.best;
        let top = &r.arenas[best];
        let label = FontId::proportional(14.0);
        if s.sumo {
            let ring = s.rules.ring_width as f32;
            let scale = (rect.width() / (ring + 3.0)).min(rect.height() / 3.0);
            let ground_y = rect.top() + rect.height() * 0.62;
            let to_screen = |p: [f32; 2]| Pos2::new(rect.center().x + p[0] * scale, ground_y - p[1] * scale);
            let half = ring / 2.0;
            painter.rect_filled(Rect::from_two_pos(to_screen([-half, 0.0]), to_screen([half, -1.0])), 2.0, Color32::from_rgb(170, 140, 100));
            painter.line_segment([to_screen([0.0, 0.0]), to_screen([0.0, -0.15])], Stroke::new(2.0, Color32::WHITE));
            let colors = [run.color, Color32::from_gray(150)];
            for (i, a) in r.arenas.iter().enumerate() {
                if i != best {
                    draw_creatures(&painter, a, &colors, &to_screen, scale, GHOST);
                }
            }
            draw_creatures(&painter, top, &colors, &to_screen, scale, 255);
            let c = top.com(0);
            painter.text(to_screen([c[0] as f32, c[1] as f32 + 0.8]), Align2::CENTER_BOTTOM, "best", label.clone(), Color32::WHITE);
        } else {
            let xs: Vec<f32> = r.arenas.iter().map(|a| a.com(0)[0] as f32).filter(|x| x.is_finite()).collect();
            let (lo, hi) = xs.iter().fold((f32::MAX, f32::MIN), |(l, h), &x| (l.min(x), h.max(x)));
            let span = (hi - lo + 4.0).max(6.0);
            let fit = (rect.width() / span).min(rect.height() / 2.5).clamp(8.0, 160.0);
            let (cx, scale) = follow(&mut self.camera, ((lo + hi) / 2.0, fit));
            let ground_y = rect.top() + rect.height() * 0.8;
            let to_screen = |p: [f32; 2]| Pos2::new(rect.center().x + (p[0] - cx) * scale, ground_y - p[1] * scale);
            let x_range = (cx - rect.width() / 2.0 / scale, cx + rect.width() / 2.0 / scale);
            draw_track(&painter, rect, ground_y, &to_screen, x_range, scale, &top.terrain, true);
            for (i, a) in r.arenas.iter().enumerate() {
                if i != best {
                    draw_creatures(&painter, a, &[run.color], &to_screen, scale, GHOST);
                }
            }
            draw_creatures(&painter, top, &[run.color], &to_screen, scale, 255);
            let c = top.com(0);
            painter.text(to_screen([c[0] as f32, c[1] as f32 + 0.6]), Align2::CENTER_BOTTOM, format!("best  {:.2} m", top.progress(0)), label.clone(), Color32::WHITE);
        }
        let n = run.generations.len();
        let time = r.arenas.iter().map(|a| a.time).fold(0.0, f64::max);
        let limit = if s.sumo { s.rules.sumo_time } else { s.rules.race_time };
        let head = format!("Generation {} of {} · {} individuals · {} evaluations so far", r.gen, n - 1, g.creatures.len(), g.evaluations);
        painter.text(rect.left_top() + Vec2::new(10.0, 8.0), Align2::LEFT_TOP, head, FontId::proportional(15.0), Color32::WHITE);
        let mut sub = format!("Highlighted: the best of this generation (fitness {:.2}); faint: the rest", g.fitness[best]);
        if s.sumo {
            sub += &format!(" · all fight {}", s.opponent.name);
            if s.opponents > 1 {
                sub += &format!(" (the first of {} opponents)", s.opponents);
            }
        }
        painter.text(rect.left_top() + Vec2::new(10.0, 28.0), Align2::LEFT_TOP, sub, label.clone(), CANVAS_TEXT);
        painter.text(rect.right_top() + Vec2::new(-10.0, 8.0), Align2::RIGHT_TOP, format!("t = {time:.1} / {limit:.0} s"), FontId::proportional(15.0), Color32::WHITE);
    }

    fn chart(&self, painter: &egui::Painter, rect: Rect) {
        painter.rect_filled(rect, 0.0, Color32::from_rgb(28, 32, 40));
        let points = || self.runs.iter().flat_map(|r| r.points.iter());
        let x_max = points().map(|p| p[0]).fold(0.0, f64::max);
        let (lo, hi) = points().flat_map(|p| [p[1], p[2]]).filter(|y| y.is_finite()).fold((f64::MAX, f64::MIN), |(l, h), y| (l.min(y), h.max(y)));
        if x_max <= 0.0 || lo > hi {
            let hint = "Press ▶ Train. Best (solid) and mean (dashed) fitness are plotted against evaluations.";
            painter.text(rect.center(), Align2::CENTER_CENTER, hint, FontId::proportional(16.0), Color32::from_gray(160));
            return;
        }
        let pad = ((hi - lo) * 0.08).max(0.1);
        let (lo, hi) = (lo - pad, hi + pad);
        let plot = Rect::from_min_max(rect.min + Vec2::new(60.0, 26.0), rect.max - Vec2::new(18.0, 34.0));
        let to_screen = |x: f64, y: f64| {
            Pos2::new(plot.left() + (x / x_max) as f32 * plot.width(), plot.bottom() - ((y - lo) / (hi - lo)) as f32 * plot.height())
        };
        let grid = Stroke::new(1.0, Color32::from_gray(55));
        let (font, text) = (FontId::proportional(12.0), Color32::from_gray(170));
        for y in ticks(lo, hi, 6) {
            let p = to_screen(0.0, y);
            painter.line_segment([p, Pos2::new(plot.right(), p.y)], grid);
            painter.text(p - Vec2::new(6.0, 0.0), Align2::RIGHT_CENTER, round(y), font.clone(), text);
        }
        for x in ticks(0.0, x_max, 8) {
            let p = to_screen(x, lo);
            painter.line_segment([p, Pos2::new(p.x, plot.top())], grid);
            painter.text(p + Vec2::new(0.0, 4.0), Align2::CENTER_TOP, round(x), font.clone(), text);
        }
        painter.text(Pos2::new(plot.right(), rect.bottom() - 2.0), Align2::RIGHT_BOTTOM, "evaluations", font.clone(), text);
        painter.text(rect.min + Vec2::new(8.0, 6.0), Align2::LEFT_TOP, "fitness", font.clone(), text);
        // The generation being replayed / shown in the DNA view.
        if let Some((k, gen)) = self.selected() {
            let x = to_screen(self.runs[k].generations[gen].evaluations as f64, lo).x;
            if x <= plot.right() + 1.0 {
                painter.line_segment([Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())], Stroke::new(1.0, self.runs[k].color.gamma_multiply(0.6)));
            }
        }

        for r in &self.runs {
            let line = |k: usize| -> Vec<Pos2> { r.points.iter().filter(|p| p[k].is_finite()).map(|p| to_screen(p[0], p[k])).collect() };
            let (best, mean) = (line(1), line(2));
            if mean.len() >= 2 {
                painter.extend(Shape::dashed_line(&mean, Stroke::new(1.0, r.color.gamma_multiply(0.7)), 6.0, 4.0));
            }
            match best.len() {
                0 => {}
                1 => {
                    painter.circle_filled(best[0], 3.0, r.color);
                }
                _ => {
                    painter.add(Shape::line(best, Stroke::new(2.0, r.color)));
                }
            }
        }
        for (k, r) in self.runs.iter().enumerate() {
            let at = Pos2::new(plot.left() + 10.0, plot.top() + 8.0 + 16.0 * k as f32);
            painter.line_segment([at, at + Vec2::new(16.0, 0.0)], Stroke::new(2.0, r.color));
            painter.text(at + Vec2::new(22.0, 0.0), Align2::LEFT_CENTER, &r.label, font.clone(), Color32::from_gray(220));
        }
    }
}

/// "Same GA, different decoding": the genome layout at each level, this run's highlighted.
fn decoding(ui: &mut egui::Ui, s: &Setting) {
    let Some(p) = &s.problem else {
        ui.label(RichText::new("Different algorithm, no fixed genome").strong());
        ui.label(
            "This script evolves the creature itself: its mutations (add a limb, remove a limb, move a limb, nudge a value) \
             work on a dict with the same fields as the .toml file. Here it is the algorithm that changes, not the decoding. \
             To compare generations, each creature is shown below as a structure-level genome.",
        );
        kind_legend(ui);
        return;
    };
    ui.label(RichText::new("Same GA, different decoding").strong());
    ui.label(
        "The GA is the same at every level: it only ever sees a list of numbers between 0 and 1, and returns new lists. \
         What changes is how the arena decodes the list. At the brain level the DNA becomes movement; at the body and \
         structure levels the same kind of list also becomes the body and its structure.",
    );
    let levels = [Level::Brain, Level::Body, Level::Structure];
    let layouts: Vec<Vec<GeneKind>> = levels.iter().map(|&level| GenomeSpec { level }.genes(&p.template, &p.rules).iter().map(GeneKind::of).collect()).collect();
    let longest = layouts.iter().map(Vec::len).max().unwrap_or(1);
    let v = ui.visuals().clone();
    let font = FontId::proportional(13.0);
    let texts: Vec<String> = levels
        .iter()
        .zip(&layouts)
        .map(|(level, kinds)| {
            let what = match level {
                Level::Brain => "movement",
                Level::Body => "movement + body shape",
                Level::Structure => "movement + body shape + structure",
            };
            format!("{} genes: {what}", kinds.len())
        })
        .collect();
    let text_w = texts.iter().map(|t| ui.painter().layout_no_wrap(t.clone(), font.clone(), v.text_color()).size().x).fold(0.0, f32::max);
    for ((level, kinds), text) in levels.iter().zip(&layouts).zip(texts) {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), Sense::hover());
        let painter = ui.painter_at(rect);
        let current = *level == s.level;
        let ink = if current { v.strong_text_color() } else { v.text_color() };
        let name = if current { format!("▶ {}", level.name()) } else { format!("   {}", level.name()) };
        painter.text(rect.left_center(), Align2::LEFT_CENTER, name, font.clone(), ink);
        let x0 = rect.left() + 100.0;
        let cell = ((rect.width() - 100.0 - text_w - 16.0).max(60.0) / longest as f32).min(10.0);
        let gap = if cell >= 4.0 { 1.0 } else { 0.0 };
        for (i, kind) in kinds.iter().enumerate() {
            let r = Rect::from_min_size(Pos2::new(x0 + i as f32 * cell, rect.top() + 5.0), Vec2::new(cell - gap, rect.height() - 10.0));
            painter.rect_filled(r, 1.0, kind.color());
        }
        if current {
            let outline = Rect::from_min_size(Pos2::new(x0 - 2.0, rect.top() + 2.0), Vec2::new(kinds.len() as f32 * cell + 3.0, rect.height() - 4.0));
            painter.rect_stroke(outline, 3.0, Stroke::new(1.5, v.strong_text_color()), egui::StrokeKind::Outside);
        }
        painter.text(Pos2::new(x0 + longest as f32 * cell + 12.0, rect.center().y), Align2::LEFT_CENTER, text, font.clone(), ink);
    }
    kind_legend(ui);
}

fn kind_legend(ui: &mut egui::Ui) {
    ui.horizontal_wrapped(|ui| {
        for kind in GeneKind::ALL {
            let (r, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
            ui.painter().rect_filled(r, 2.0, kind.color());
            ui.label(RichText::new(kind.label()).small());
            ui.add_space(8.0);
        }
    });
}

/// What the GA actually holds: plain lists of numbers (or, for whole creatures, a dict).
fn raw_data(ui: &mut egui::Ui, run: &Run, gen: usize) {
    let g = &run.generations[gen];
    let s = &g.setting;
    ui.label(RichText::new("Raw data: what the GA reads and writes").strong());
    let line = |ui: &mut egui::Ui, label: String, shown: String, copy: String| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(label).small().weak());
            if ui.small_button("copy").on_hover_text("Copy, e.g. to paste into Python").clicked() {
                ui.ctx().copy_text(copy);
            }
        });
        ui.label(RichText::new(shown).monospace().small());
    };
    if let Some(start) = &s.start {
        line(ui, format!("template (the start), a list of {} floats:", start.len()), genome_text(start), genome_text(start));
    }
    let label = format!("best of generation {gen} (fitness {:.2}),", g.fitness[g.best]);
    match s.problem {
        Some(_) => {
            let text = genome_text(&g.genomes[g.best]);
            line(ui, format!("{label} a list of {} floats:", g.genomes[g.best].len()), text.clone(), text);
        }
        None => {
            let c = &g.creatures[g.best];
            line(ui, format!("{label} a creature dict:"), creature_text(c), serde_json::to_string(c).unwrap_or_default());
        }
    }
    let diversity = (0..s.genes.len()).map(|i| spread(&g.genomes, i)).sum::<f64>() / s.genes.len().max(1) as f64;
    let note = format!(
        "{} individuals · diversity {diversity:.3} (the average spread of each gene across the population; it shrinks as the population converges)",
        g.genomes.len()
    );
    ui.label(RichText::new(note).small().weak());
}

/// A full-width legend row: its painter, the centre line and the left edge.
fn legend_row(ui: &mut egui::Ui) -> (egui::Painter, f32, f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 18.0), Sense::hover());
    (ui.painter_at(rect), rect.center().y, rect.left())
}

/// A legend label at `x`; returns where the next item starts.
fn legend_label(painter: &egui::Painter, x: f32, y: f32, text: &str, color: Color32) -> f32 {
    painter.text(Pos2::new(x, y), Align2::LEFT_CENTER, text, FontId::proportional(11.0), color).right() + 16.0
}

fn diamond(painter: &egui::Painter, at: Pos2, size: f32, fill: Color32, ring: Color32) {
    let points = vec![at - Vec2::new(0.0, size), at + Vec2::new(size, 0.0), at + Vec2::new(0.0, size), at - Vec2::new(size, 0.0)];
    painter.add(Shape::convex_polygon(points, fill, Stroke::new(1.5, ring)));
}

/// One row per gene: this generation's population on the left, the best genome of every generation on the right.
fn gene_table(ui: &mut egui::Ui, run: &Run, gen: usize) {
    const NAME_W: f32 = 170.0;
    const VALUE_W: f32 = 70.0;
    const ROW_H: f32 = 18.0;
    let g = &run.generations[gen];
    let genes = &g.setting.genes;
    let n = run.generations.len();
    let v = ui.visuals().clone();
    // The best is outlined in ink so it stands out from the population dots of the same colour.
    let (ink, weak, strong) = (v.text_color(), v.weak_text_color(), v.strong_text_color());
    let ring = strong;
    let dot = run.color.gamma_multiply(0.3);
    let small = FontId::proportional(11.0);
    let columns = |rect: Rect| {
        let rest = (rect.width() - NAME_W - VALUE_W - 12.0).max(120.0);
        let strip = Rect::from_min_size(Pos2::new(rect.left() + NAME_W, rect.top()), Vec2::new(rest * 0.45, rect.height()));
        let heat = Rect::from_min_size(Pos2::new(strip.right() + 12.0, rect.top()), Vec2::new(rest * 0.55, rect.height()));
        (strip, heat)
    };

    // Legend, drawn with the same marks as the table (the font has no symbols for them).
    let (p, y, x) = legend_row(ui);
    p.circle_filled(Pos2::new(x + 4.0, y), 4.0, dot);
    let x = legend_label(&p, x + 12.0, y, &format!("each individual of generation {gen}"), weak);
    diamond(&p, Pos2::new(x + 6.0, y), 6.0, run.color, ring);
    let x = legend_label(&p, x + 16.0, y, "the best", weak);
    if g.setting.start.is_some() {
        p.line_segment([Pos2::new(x + 1.0, y - 7.0), Pos2::new(x + 1.0, y + 7.0)], Stroke::new(2.0, ink));
        legend_label(&p, x + 8.0, y, "the template (where evolution started)", weak);
    }
    let dark = v.dark_mode;
    let (p, y, x) = legend_row(ui);
    for j in 0..20 {
        p.rect_filled(Rect::from_min_size(Pos2::new(x + j as f32 * 2.0, y - 4.0), Vec2::new(2.2, 8.0)), 0.0, ramp(j as f64 / 19.0, dark));
    }
    let x = legend_label(&p, x + 46.0, y, "the best genome of each generation, gene value 0 to 1", weak);
    p.rect_stroke(Rect::from_center_size(Pos2::new(x + 4.0, y), Vec2::new(6.0, 12.0)), 0.0, Stroke::new(1.5, strong), egui::StrokeKind::Outside);
    legend_label(&p, x + 14.0, y, &format!("generation {gen}"), weak);

    // Column heads.
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 16.0), Sense::hover());
    let painter = ui.painter_at(rect);
    let (strip, heat) = columns(rect);
    let y = rect.center().y;
    painter.text(Pos2::new(rect.left(), y), Align2::LEFT_CENTER, "gene", small.clone(), weak);
    painter.text(Pos2::new(strip.left() + 6.0, y), Align2::CENTER_CENTER, "0", small.clone(), weak);
    painter.text(Pos2::new(strip.center().x, y), Align2::CENTER_CENTER, format!("generation {gen}"), small.clone(), weak);
    painter.text(Pos2::new(strip.right() - 6.0, y), Align2::CENTER_CENTER, "1", small.clone(), weak);
    painter.text(Pos2::new(heat.left(), y), Align2::LEFT_CENTER, "generation 0", small.clone(), weak);
    painter.text(Pos2::new(heat.right(), y), Align2::RIGHT_CENTER, format!("{}", n - 1), small.clone(), weak);
    painter.text(Pos2::new(rect.right(), y), Align2::RIGHT_CENTER, "best value", small.clone(), weak);

    for (i, gene) in genes.iter().enumerate() {
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::hover());
        if !ui.is_rect_visible(rect) {
            continue;
        }
        let painter = ui.painter_at(rect);
        let (strip, heat) = columns(rect);
        let kind = GeneKind::of(gene);
        if i % 2 == 1 {
            painter.rect_filled(rect, 0.0, v.faint_bg_color);
        }
        painter.rect_filled(Rect::from_center_size(Pos2::new(rect.left() + 5.0, rect.center().y), Vec2::splat(8.0)), 2.0, kind.color());
        painter.text(Pos2::new(rect.left() + 14.0, rect.center().y), Align2::LEFT_CENTER, &gene.name, FontId::monospace(11.0), ink);

        // This generation: every individual, the template, the best.
        let y = strip.center().y;
        let x = |v: f64| strip.left() + 6.0 + v.clamp(0.0, 1.0) as f32 * (strip.width() - 12.0);
        painter.line_segment([Pos2::new(x(0.0), y), Pos2::new(x(1.0), y)], Stroke::new(1.0, weak.gamma_multiply(0.5)));
        for genome in &g.genomes {
            painter.circle_filled(Pos2::new(x(genome[i]), y), 4.0, dot);
        }
        if let Some(start) = &g.setting.start {
            let tx = x(start[i]);
            painter.line_segment([Pos2::new(tx, rect.top() + 2.0), Pos2::new(tx, rect.bottom() - 2.0)], Stroke::new(2.0, ink));
        }
        diamond(&painter, Pos2::new(x(g.genomes[g.best][i]), y), 6.0, run.color, ring);

        // Over time: the best genome of every generation (sampled when there are more generations than pixels).
        let cells = n.min((heat.width() / 2.0) as usize).max(1);
        let w = heat.width() / cells as f32;
        let gap = if w >= 6.0 { 2.0 } else { 0.0 };
        for j in 0..cells {
            let gg = &run.generations[j * n / cells];
            let r = Rect::from_min_size(Pos2::new(heat.left() + j as f32 * w, rect.top() + 2.0), Vec2::new(w - gap + 0.3, ROW_H - 4.0));
            painter.rect_filled(r, 0.0, ramp(gg.genomes[gg.best][i], dark));
        }
        let now = Rect::from_min_size(Pos2::new(heat.left() + (gen * cells / n) as f32 * w, rect.top() + 1.0), Vec2::new((w - gap).max(2.0), ROW_H - 2.0));
        painter.rect_stroke(now, 0.0, Stroke::new(1.5, strong), egui::StrokeKind::Outside);

        painter.text(rect.right_center(), Align2::RIGHT_CENTER, physical(gene, g.genomes[g.best][i]), FontId::proportional(12.0), ink);

        let tip = match resp.hover_pos() {
            Some(p) if heat.contains(p) => {
                let gi = (((p.x - heat.left()) / w) as usize).min(cells - 1) * n / cells;
                let v = run.generations[gi].genomes[run.generations[gi].best][i];
                format!("{}\ngeneration {gi}, best individual: {v:.3} ({})", gene.name, physical(gene, v))
            }
            _ => {
                let v = g.genomes[g.best][i];
                let mut t = format!(
                    "{}: {}\nrange {} to {}\nbest of generation {gen}: {v:.3} ({})",
                    gene.name,
                    kind.label(),
                    physical(gene, 0.0),
                    physical(gene, 1.0),
                    physical(gene, v)
                );
                if let Some(start) = &g.setting.start {
                    t += &format!("\ntemplate: {:.3} ({})", start[i], physical(gene, start[i]));
                }
                t + &format!("\nspread in this generation: {:.3}", spread(&g.genomes, i))
            }
        };
        resp.on_hover_text(tip);
    }
}

/// Round numbers (steps of 1, 2 or 5 times a power of ten) between lo and hi.
fn ticks(lo: f64, hi: f64, n: usize) -> Vec<f64> {
    let raw = (hi - lo) / n as f64;
    let mag = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 5.0, 10.0].into_iter().map(|m| m * mag).find(|&s| s >= raw).unwrap_or(10.0 * mag);
    ((lo / step).ceil() as i64..=(hi / step).floor() as i64).map(|k| k as f64 * step).collect()
}

fn round(v: f64) -> String {
    format!("{}", (v * 1000.0).round() / 1000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_progress_lines_of_every_script() {
        // ga.py, my_ga.py, evolve_structure.py, random_search.py, coevolve.py
        let ga = progress("gen   3  evaluations   194  best    4.250  mean    1.500  best ever    4.300").unwrap();
        assert_eq!((ga[0], ga[1], ga[2]), (194.0, 4.25, 1.5));
        assert_eq!(progress("gen   1  evaluations   100  best    2.000  mean    0.500").unwrap()[1], 2.0);
        let s = progress("gen   2  evaluations    90  best     -inf  segments best 3  range 2-5").unwrap();
        assert_eq!(s[0], 90.0);
        assert!(s[1].is_infinite() && s[2].is_nan());
        assert_eq!(progress("evaluations   101  best    3.100").unwrap()[1], 3.1);
        assert_eq!(progress("gen   5  duels   600  best  0.750  mean  0.010  | champion vs benchmark  0.400").unwrap()[0], 600.0);
        assert!(progress("14 genes: brain.frequency, brain.coupling").is_none());
        assert!(progress("champion fitness 4.300 -> creatures/me.toml").is_none());
    }

    #[test]
    fn ticks_are_round_and_inside_the_range() {
        assert_eq!(ticks(0.0, 1000.0, 8), vec![0.0, 200.0, 400.0, 600.0, 800.0, 1000.0]);
        assert_eq!(ticks(-0.4, 1.3, 6), vec![0.0, 0.5, 1.0]);
        assert_eq!(ticks(2.5, 9.7, 6), vec![4.0, 6.0, 8.0]);
    }

    #[test]
    fn earlier_time_stamps_are_stripped() {
        assert_eq!(strip_name_stamp("Worm (ga 14:02:31)"), "Worm");
        assert_eq!(strip_name_stamp("Tailfin Sumo (my_ga 09:00:05)"), "Tailfin Sumo");
        assert_eq!(strip_name_stamp("Worm (ga)"), "Worm (ga)");
        assert_eq!(strip_name_stamp("Wörm (ga 14:02:31)"), "Wörm");
        assert_eq!(strip_name_stamp("ö)"), "ö)");
        assert_eq!(strip_file_stamp("worm-ga-20260929-140231"), "worm");
        assert_eq!(strip_file_stamp("tailfin-sumo-structure-20260929-140231"), "tailfin-sumo");
        assert_eq!(strip_file_stamp("walker-racer"), "walker-racer");
    }

    /// Train `creature` from a fresh copy in its own temp folder and wait for the script to finish.
    fn train(folder: &str, creature: &str, setup: impl FnOnce(&mut Trainer)) -> (Trainer, PathBuf) {
        let dir = std::env::temp_dir().join(folder);
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let file = format!("{creature}.toml");
        std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("creatures").join(&file), dir.join(&file)).unwrap();
        let mut t = Trainer::new(dir.clone());
        t.template = Some(dir.join(&file));
        setup(&mut t);
        t.start(&dir);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        while t.poll() {
            assert!(std::time::Instant::now() < deadline, "timed out");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        (t, dir)
    }

    #[test]
    #[ignore = "needs Python and a release build of the arena binary"]
    fn structure_evolution_records_whole_creatures_and_replays_sumo() {
        let (mut t, _) = train("arena-gui-train-structure", "tailfin", |t| {
            (t.script, t.mode, t.budget, t.name) = (Script::Structure, "sumo", 130, "Tailfin".into());
        });
        let run = &t.runs[0];
        assert!(run.status == Status::Done, "{:#?}", run.log);
        assert!(run.generations.len() >= 2, "{:#?}", run.log);
        let g = &run.generations[0];
        assert!(g.setting.sumo && g.setting.problem.is_none() && g.setting.opponent.name == game::rock().name);
        // DNA view: whole creatures are shown as structure-level genomes.
        assert!(g.setting.level == Level::Structure && g.setting.start.is_none());
        assert!(g.genomes.len() == g.creatures.len() && g.genomes.iter().all(|x| x.len() == g.setting.genes.len()));
        assert!(t.advance(0.1, 8.0, false));
        assert!(t.replay.as_ref().unwrap().arenas.iter().all(|a| a.fighters.len() == 2 && a.time > 0.0));
    }

    #[test]
    #[ignore = "needs Python and a release build of the arena binary"]
    fn co_evolution_records_every_generation() {
        let (t, _) = train("arena-gui-train-coevolve", "tailfin", |t| {
            (t.script, t.generations, t.population, t.name) = (Script::Coevolve, 6, 8, "Tailfin".into());
        });
        let run = &t.runs[0];
        assert!(run.status == Status::Done, "{:#?}", run.log);
        // One population per generation from the duels; the champion's benchmark checks are not generations.
        assert_eq!(run.generations.len(), 6);
        assert!(run.generations.iter().all(|g| g.creatures.len() == 8));
        // Evaluations count duels, like the chart's x axis.
        assert_eq!(run.generations.last().unwrap().evaluations, run.points.last().unwrap()[0] as usize);
    }

    #[test]
    #[ignore = "needs Python and a release build of the arena binary"]
    fn sumo_against_chosen_opponents() {
        let (t, _) = train("arena-gui-train-opponents", "tailfin", |t| {
            let dir = std::env::temp_dir().join("arena-gui-train-opponents");
            for c in ["worm", "walker"] {
                std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("creatures/{c}.toml")), dir.join(format!("{c}.toml"))).unwrap();
                t.chosen.insert(dir.join(format!("{c}.toml")));
            }
            (t.mode, t.vs_rock, t.budget, t.population, t.name) = ("sumo", false, 60, 20, "Tailfin".into());
        });
        let run = &t.runs[0];
        assert!(run.status == Status::Done, "{:#?}", run.log);
        assert!(run.label.contains("vs 2"), "{}", run.label);
        let s = &run.generations[0].setting;
        assert_eq!(s.opponents, 2);
        assert!(["Worm", "Walker"].contains(&s.opponent.name.as_str()), "{}", s.opponent.name);
        let copies = run.opponents.clone().unwrap();
        drop(t);
        assert!(!copies.exists(), "the copies of the opponents are cleaned up");
    }

    #[test]
    #[ignore = "needs Python and a release build of the arena binary"]
    fn trains_with_the_default_ga_records_and_replays_the_population() {
        let (mut t, dir) = train("arena-gui-train-test", "worm", |t| (t.budget, t.population, t.name) = (100, 20, "Worm".into()));
        let run = &t.runs[0];
        assert!(run.status == Status::Done, "{:#?}", run.log);
        assert!(run.points.len() >= 5 && run.points.iter().all(|p| p[1].is_finite() && p[2].is_finite()));
        // Saved under a new, time-stamped name; the template is untouched.
        let file = run.out.file_name().unwrap().to_string_lossy().into_owned();
        assert!(file.starts_with("worm-ga-") && run.out.is_file(), "{file}");
        let saved = Creature::load(&run.out).unwrap();
        assert!(saved.name.starts_with("Worm (ga ") && strip_name_stamp(&saved.name) == "Worm", "{}", saved.name);
        assert_eq!(Creature::load(&dir.join("worm.toml")).unwrap().name, "Worm");
        // One recorded generation per evaluate call: the initial population, then the children.
        assert_eq!(run.generations[0].creatures.len(), 20);
        assert!(run.generations.len() >= 5);
        // DNA view: the genomes the GA sent, one value per gene, and the template's genome (brain level, 14 genes for the worm).
        let g0 = &run.generations[0];
        assert_eq!((g0.setting.genes.len(), g0.setting.start.as_ref().map(Vec::len)), (14, Some(14)));
        assert!(g0.genomes.iter().all(|x| x.len() == 14));
        // ga.py puts the template in the first population.
        assert!(g0.genomes[0].iter().zip(g0.setting.start.as_ref().unwrap()).all(|(a, b)| (a - b).abs() < 1e-9));
        assert!(g0.fitness[g0.best] >= g0.fitness.iter().copied().fold(f64::MIN, f64::max));
        assert_eq!(run.generations.last().unwrap().evaluations, run.points.last().unwrap()[0] as usize);
        // Watching replays the newest generation, every individual in its own world.
        let newest = run.generations.len() - 1;
        let size = run.generations[newest].creatures.len();
        assert!(t.advance(0.1, 8.0, false));
        let replay = t.replay.as_ref().unwrap();
        assert_eq!((replay.gen, replay.arenas.len()), (newest, size));
        assert!(replay.arenas.iter().all(|a| a.time > 0.0));
    }
}
