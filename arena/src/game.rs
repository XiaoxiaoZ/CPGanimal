//! Game modes, fitness functions, the creatures folder, and tournaments.

use crate::creature::{Brain, Creature, Rules, Segment};
use crate::sim::{run_race, run_sumo};
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Mode {
    /// Go as far right as possible in `race_time` seconds.
    Race,
    /// Push the opponent off the platform.
    Sumo,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Race => "race",
            Mode::Sumo => "sumo",
        }
    }
}

/// A motionless block: the default sparring partner for sumo training.
pub fn rock() -> Creature {
    Creature {
        name: "Rock".into(),
        author: "arena".into(),
        color: Some([120, 120, 120]),
        brain: Brain { frequency: 1.0, coupling: 4.0 },
        segments: vec![Segment { length: 0.7, width: 0.3, ..Default::default() }],
        meta: None,
    }
}

/// Sumo score of `c` against one opponent: +1 win / 0 draw / −1 loss, plus
/// the ring-control margin so the GA gets a smooth signal.
pub fn sumo_score(c: &Creature, opponent: &Creature, rules: &Rules) -> f64 {
    let r = run_sumo(c, opponent, rules);
    let outcome = match r.winner {
        Some(0) => 1.0,
        Some(_) => -1.0,
        None => 0.0,
    };
    outcome + r.margin
}

/// Fitness of a creature in a mode (higher is better).
pub fn fitness(c: &Creature, mode: Mode, rules: &Rules, opponents: &[Creature]) -> f64 {
    match mode {
        Mode::Race => run_race(c, rules),
        Mode::Sumo => {
            let own = rock();
            let ops: &[Creature] = if opponents.is_empty() { std::slice::from_ref(&own) } else { opponents };
            ops.iter().map(|o| sumo_score(c, o, rules)).sum::<f64>() / ops.len() as f64
        }
    }
}

/// One file in the creatures folder.
#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub creature: Result<Creature, Vec<String>>,
    pub modified: Option<SystemTime>,
}

impl Entry {
    pub fn label(&self) -> String {
        match &self.creature {
            Ok(c) => c.name.clone(),
            Err(_) => self.path.file_name().unwrap_or_default().to_string_lossy().into_owned(),
        }
    }
}

/// Every `*.toml` in `dir` except `arena.toml`, sorted by file name.
pub fn load_folder(dir: &Path, rules: &Rules) -> Vec<Entry> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    paths.retain(|p| p.extension().is_some_and(|e| e == "toml") && p.file_name().is_some_and(|n| n != "arena.toml"));
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
            let creature = Creature::load(&path)
                .map_err(|e| vec![e])
                .and_then(|c| c.validate(rules).map(|_| c));
            Entry { path, creature, modified }
        })
        .collect()
}

/// Fingerprint of a folder, used to notice edits.
pub fn folder_stamp(dir: &Path) -> Vec<(PathBuf, Option<SystemTime>)> {
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| (e.path(), e.metadata().and_then(|m| m.modified()).ok()))
                .filter(|(p, _)| p.extension().is_some_and(|e| e == "toml"))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// Race every creature; returns (index, distance) best first.
pub fn race_all(creatures: &[Creature], rules: &Rules) -> Vec<(usize, f64)> {
    let mut v: Vec<(usize, f64)> = creatures.par_iter().enumerate().map(|(i, c)| (i, run_race(c, rules))).collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1));
    v
}

#[derive(Clone, Debug, Default)]
pub struct Standing {
    pub index: usize,
    pub wins: usize,
    pub draws: usize,
    pub losses: usize,
    /// 3 per win, 1 per draw.
    pub points: usize,
}

#[derive(Clone, Debug)]
pub struct Bout {
    pub left: usize,
    pub right: usize,
    /// Some(left) / Some(right) / None.
    pub winner: Option<usize>,
    pub reason: String,
    pub time: f64,
}

/// Round robin: every pair fights twice, once from each side.
pub fn tournament(creatures: &[Creature], rules: &Rules) -> (Vec<Standing>, Vec<Bout>) {
    let n = creatures.len();
    let pairs: Vec<(usize, usize)> =
        (0..n).flat_map(|i| (0..n).filter(move |&j| j != i).map(move |j| (i, j))).collect();
    let bouts: Vec<Bout> = pairs
        .par_iter()
        .map(|&(l, r)| {
            let res = run_sumo(&creatures[l], &creatures[r], rules);
            Bout { left: l, right: r, winner: res.winner.map(|w| if w == 0 { l } else { r }), reason: res.reason, time: res.time }
        })
        .collect();
    let mut table: Vec<Standing> = (0..n).map(|index| Standing { index, ..Default::default() }).collect();
    for b in &bouts {
        match b.winner {
            Some(w) => {
                let l = if w == b.left { b.right } else { b.left };
                table[w].wins += 1;
                table[w].points += 3;
                table[l].losses += 1;
            }
            None => {
                for k in [b.left, b.right] {
                    table[k].draws += 1;
                    table[k].points += 1;
                }
            }
        }
    }
    table.sort_by(|a, b| b.points.cmp(&a.points).then(b.wins.cmp(&a.wins)));
    (table, bouts)
}

/// Fitness of creatures given directly (not as genomes), in parallel.
/// Creatures that break the rules get an `Err` with the reasons.
pub fn judge(creatures: &[Creature], mode: Mode, rules: &Rules, opponents: &[Creature]) -> Vec<Result<f64, String>> {
    creatures
        .par_iter()
        .map(|c| {
            c.validate(rules).map_err(|e| e.join("; "))?;
            let ops: Vec<Creature> = opponents.iter().filter(|o| o.name != c.name).cloned().collect();
            Ok(fitness(c, mode, rules, &ops))
        })
        .collect()
}
