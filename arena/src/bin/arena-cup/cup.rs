//! The Cup: a knockout competition for the class. The ticked creatures are
//! drawn at random into a bracket; every match is one against one (a race
//! side by side, or a sumo bout) on the environment of one seed, and the
//! winner goes on to the next round. A creature's file is read from the
//! folder when its match starts. Only the winner is announced: what decided
//! each match (distances, how a bout ended) is saved in `cup-results/` in the
//! folder, for the teacher.
//!
//! The physics are exactly those of training: the same simulator and the
//! same environment as that environment seed in the Train tab. Races last
//! a minute.

use super::environment::{self, Track};
use super::scene::{self, fit, fitting_font, shadow_text};
use super::sound::{Effect, Sound};
use super::trophy::trophy;
use cpg_arena::creature::{Creature, Rules};
use cpg_arena::sim::{hash01, Arena, SumoResult};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Shape, Stroke, StrokeKind, Vec2};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Seconds of countdown (3, 2, 1) before a match starts.
const COUNTDOWN: f64 = 3.0;
/// Seconds of drum roll before the winner is announced (the length of the roll).
const DRUM_ROLL: f64 = 2.1;
/// Seconds "GO!" stays on screen.
const GO: f64 = 0.8;
/// How long a race in the cup lasts (s).
const RACE_TIME: f64 = 60.0;
const GOLD: Color32 = Color32::from_rgb(244, 196, 64);

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Event {
    Race,
    Sumo,
}

/// A creature that can enter: a ticked, valid creature of the folder.
pub struct Entrant {
    pub name: String,
    pub path: PathBuf,
    pub color: Color32,
}

/// The rules of a cup match: the folder's rules on the environment of
/// `seed`, as in the Train tab; races last a minute.
pub fn cup_rules(folder: &Rules, event: Event, seed: u64) -> Rules {
    let mut r = Track::random(seed).apply(folder);
    if event == Event::Race {
        r.race_time = RACE_TIME;
    }
    r
}

/// What the field of a cup is like (`rules`: the cup's rules).
fn field_text(event: Event, seed: u64, rules: &Rules) -> String {
    let t = Track::random(seed);
    match event {
        Event::Race => format!("Environment {seed}: {} · races last {:.0} s", t.describe(), rules.race_time),
        Event::Sumo => format!("Environment {seed}: {} · bouts last up to {:.0} s", t.describe_ring(), rules.sumo_time),
    }
}

/// FNV-1a of a file's bytes: shows whether it changed after the draw.
fn fingerprint(bytes: &[u8]) -> String {
    let h = bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3));
    format!("{h:016x}")
}

#[derive(Clone, Serialize, Deserialize)]
struct Contestant {
    name: String,
    file: PathBuf,
    /// Of the file at the draw.
    fingerprint: String,
    color: [u8; 3],
}

impl Contestant {
    fn color(&self) -> Color32 {
        Color32::from_rgb(self.color[0], self.color[1], self.color[2])
    }
}

/// What decided a match: saved for the teacher, never shown.
#[derive(Clone, Serialize, Deserialize, Debug)]
struct Outcome {
    winner: String,
    decided_by: String,
    /// Race: metres each went (left, right); null when its physics broke.
    #[serde(skip_serializing_if = "Option::is_none")]
    distances_m: Option<[Option<f64>; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sumo: Option<SumoRecord>,
    played_at: String,
    /// Of the files as they were played (left, right).
    fingerprints: [String; 2],
}

#[derive(Clone, Serialize, Deserialize, Debug)]
struct SumoRecord {
    reason: String,
    time_s: f64,
    /// Ring control in [-1, 1]; positive: the left creature.
    margin: f64,
}

#[derive(Clone, Default)]
struct Match {
    /// Contestants (indices); None: not known yet, or the empty side of a bye.
    players: [Option<usize>; 2],
    bye: bool,
    winner: Option<usize>,
    outcome: Option<Outcome>,
    /// The two creature files as played (their text), to watch the match
    /// again, also from the saved results.
    played: Option<[String; 2]>,
}

struct Bracket {
    event: Event,
    seed: u64,
    rules: Rules,
    contestants: Vec<Contestant>,
    rounds: Vec<Vec<Match>>,
    /// The folder of the creatures: the results go to its `cup-results`.
    folder: PathBuf,
    /// The results file (`.json`, and the same name with `.csv`), named when
    /// the first match is saved: `cup-<event>-<time>.json`.
    results: Option<PathBuf>,
    drawn_at: String,
}

/// The saved results of a cup: everything the teacher may want to know,
/// and everything needed to open the cup again (to watch a match again, or
/// to go on after the program was closed).
#[derive(Serialize, Deserialize)]
struct Report {
    event: Event,
    environment_seed: u64,
    environment: String,
    drawn_at: String,
    champion: String,
    contestants: Vec<Contestant>,
    rounds: Vec<RoundReport>,
    rules: Rules,
    /// The same rules as an `arena.toml`: read back exactly (JSON numbers may not be).
    rules_toml: String,
}

#[derive(Serialize, Deserialize)]
struct RoundReport {
    name: String,
    matches: Vec<MatchReport>,
}

#[derive(Serialize, Deserialize)]
struct MatchReport {
    r#match: usize,
    left: String,
    right: String,
    bye: bool,
    winner: String,
    /// Indices into `contestants` (left, right; null: nobody) and of the winner.
    players: [Option<usize>; 2],
    winner_index: Option<usize>,
    outcome: Option<Outcome>,
    /// The two creature files as they were played.
    creature_files: Option<[String; 2]>,
}

/// Round names, counted from the final backwards.
fn round_name(r: usize, rounds: usize) -> String {
    match rounds - r {
        1 => "Final".into(),
        2 => "Semi-finals".into(),
        3 => "Quarter-finals".into(),
        _ => format!("Round {}", r + 1),
    }
}

/// The order of `n` things shuffled by `seed` (Fisher-Yates).
fn shuffled(n: usize, seed: u64) -> Vec<usize> {
    let mut v: Vec<usize> = (0..n).collect();
    for i in (1..n).rev() {
        let j = ((hash01(i as i64, seed) * (i + 1) as f64) as usize).min(i);
        v.swap(i, j);
    }
    v
}

impl Bracket {
    /// Draw the entrants at random into a knockout bracket. With a number of
    /// entrants that is not a power of two, some get a bye (a free pass)
    /// through the first round; never two byes in one match.
    fn draw(entrants: &[Entrant], event: Event, seed: u64, rules: &Rules, folder: &Path, draw_seed: u64) -> Self {
        let n = entrants.len();
        let size = n.next_power_of_two().max(2);
        let first = size / 2;
        let byes: HashSet<usize> = shuffled(first, draw_seed ^ 0xB7E5).into_iter().take(size - n).collect();
        let mut order = shuffled(n, draw_seed).into_iter();
        let round1: Vec<Match> = (0..first)
            .map(|m| {
                if byes.contains(&m) {
                    let p = order.next();
                    Match { players: [p, None], bye: true, winner: p, ..Default::default() }
                } else {
                    Match { players: [order.next(), order.next()], ..Default::default() }
                }
            })
            .collect();
        let mut rounds = vec![round1];
        while rounds.last().is_some_and(|r| r.len() > 1) {
            let k = rounds.last().map_or(0, Vec::len) / 2;
            rounds.push(vec![Match::default(); k]);
        }
        let now = chrono::Local::now();
        let mut b = Self {
            event,
            seed,
            rules: cup_rules(rules, event, seed),
            contestants: entrants
                .iter()
                .map(|e| Contestant {
                    name: e.name.clone(),
                    file: e.path.clone(),
                    fingerprint: fingerprint(&std::fs::read(&e.path).unwrap_or_default()),
                    color: [e.color.r(), e.color.g(), e.color.b()],
                })
                .collect(),
            rounds,
            folder: folder.to_path_buf(),
            results: None,
            drawn_at: now.format("%Y-%m-%d %H:%M:%S").to_string(),
        };
        b.advance_winners();
        b
    }

    /// Put every winner into its place in the next round.
    fn advance_winners(&mut self) {
        for r in 0..self.rounds.len() - 1 {
            for m in 0..self.rounds[r].len() {
                let w = self.rounds[r][m].winner;
                self.rounds[r + 1][m / 2].players[m % 2] = w;
            }
        }
    }

    fn started(&self) -> bool {
        self.rounds.iter().flatten().any(|m| m.outcome.is_some())
    }

    fn playable(m: &Match) -> bool {
        !m.bye && m.winner.is_none() && m.players.iter().all(Option::is_some)
    }

    /// The next match to play, round by round.
    fn next(&self) -> Option<(usize, usize)> {
        self.rounds.iter().enumerate().find_map(|(r, round)| round.iter().position(Self::playable).map(|m| (r, m)))
    }

    fn champion(&self) -> Option<usize> {
        self.rounds.last().and_then(|r| r[0].winner)
    }

    fn name(&self, p: Option<usize>) -> String {
        p.map_or(String::new(), |i| self.contestants[i].name.clone())
    }

    fn environment(&self) -> String {
        field_text(self.event, self.seed, &self.rules)
    }

    fn report(&self) -> Report {
        let n = self.rounds.len();
        let rounds = self
            .rounds
            .iter()
            .enumerate()
            .map(|(r, round)| RoundReport {
                name: round_name(r, n),
                matches: round
                    .iter()
                    .enumerate()
                    .map(|(m, x)| MatchReport {
                        r#match: m + 1,
                        left: self.name(x.players[0]),
                        right: self.name(x.players[1]),
                        bye: x.bye,
                        winner: self.name(x.winner),
                        players: x.players,
                        winner_index: x.winner,
                        outcome: x.outcome.clone(),
                        creature_files: x.played.clone(),
                    })
                    .collect(),
            })
            .collect();
        Report {
            event: self.event,
            environment_seed: self.seed,
            environment: self.environment(),
            drawn_at: self.drawn_at.clone(),
            champion: self.name(self.champion()),
            contestants: self.contestants.clone(),
            rounds,
            rules: self.rules.clone(),
            rules_toml: toml::to_string(&self.rules).unwrap_or_default(),
        }
    }

    /// Open a cup saved by `save`: to watch its matches again, or to go on
    /// with it. Its results keep going to the same file.
    fn open(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let report: Report = serde_json::from_str(&text).map_err(|e| format!("{} is not a saved cup that this program can open: {e}", path.display()))?;
        let rules: Rules = toml::from_str(&report.rules_toml).map_err(|e| format!("{}: {e}", path.display()))?;
        let rounds: Vec<Vec<Match>> = report
            .rounds
            .into_iter()
            .map(|round| {
                round
                    .matches
                    .into_iter()
                    .map(|m| Match { players: m.players, bye: m.bye, winner: m.winner_index, outcome: m.outcome, played: m.creature_files })
                    .collect()
            })
            .collect();
        // A knockout (every round half the one before, down to the final)
        // between the contestants of the file.
        let n = report.contestants.len();
        let shape = rounds.last().is_some_and(|r| r.len() == 1) && rounds.windows(2).all(|w| w[0].len() == 2 * w[1].len());
        let players = rounds.iter().flatten().all(|m| {
            m.players.iter().flatten().all(|&i| i < n) && m.winner.is_none_or(|w| m.players.contains(&Some(w))) && (m.outcome.is_none() || m.played.is_some())
        });
        if !shape || !players {
            return Err(format!("{}: the bracket in this file is broken", path.display()));
        }
        let mut b = Self {
            event: report.event,
            seed: report.environment_seed,
            rules,
            contestants: report.contestants,
            rounds,
            folder: path.parent().and_then(Path::parent).map_or_else(PathBuf::new, Path::to_path_buf),
            results: Some(path.to_path_buf()),
            drawn_at: report.drawn_at,
        };
        b.advance_winners();
        Ok(b)
    }

    /// Write the results (all of them, after every match) for the teacher.
    fn save(&mut self) -> Result<PathBuf, String> {
        let report = self.report();
        let event = if self.event == Event::Race { "race" } else { "sumo" };
        let name = format!("cup-{event}-{}.json", chrono::Local::now().format("%Y%m%d-%H%M%S"));
        let json = self.results.get_or_insert_with(|| self.folder.join("cup-results").join(name)).clone();
        if let Some(dir) = json.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let text = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
        std::fs::write(&json, text).map_err(|e| format!("{}: {e}", json.display()))?;
        let n = self.rounds.len();
        // The same as a table, for a spreadsheet.
        let q = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
        let num = |v: Option<f64>| v.map_or(String::new(), |v| format!("{v:.4}"));
        let mut csv = String::from("round,match,left,right,winner,decided_by,left_distance_m,right_distance_m,sumo_end,sumo_time_s,sumo_margin,played_at\n");
        for (r, round) in self.rounds.iter().enumerate() {
            for (m, x) in round.iter().enumerate() {
                let Some(o) = &x.outcome else { continue };
                let d = o.distances_m.unwrap_or([None, None]);
                let s = o.sumo.as_ref();
                csv += &format!(
                    "{},{},{},{},{},{},{},{},{},{},{},{}\n",
                    q(&round_name(r, n)),
                    m + 1,
                    q(&self.name(x.players[0])),
                    q(&self.name(x.players[1])),
                    q(&o.winner),
                    q(&o.decided_by),
                    num(d[0]),
                    num(d[1]),
                    q(s.map_or("", |s| s.reason.as_str())),
                    num(s.map(|s| s.time_s)),
                    num(s.map(|s| s.margin)),
                    q(&o.played_at)
                );
            }
        }
        let csv_path = json.with_extension("csv");
        std::fs::write(&csv_path, csv).map_err(|e| format!("{}: {e}", csv_path.display()))?;
        Ok(json)
    }
}

/// Who won a race (side 0 or 1) and why: the one that went farther; a
/// creature whose physics broke loses; a dead heat goes to `coin`.
fn race_winner(d: [Option<f64>; 2], coin: usize) -> (usize, String) {
    match (d[0], d[1]) {
        (Some(a), Some(b)) if a > b => (0, "went farther".into()),
        (Some(a), Some(b)) if b > a => (1, "went farther".into()),
        (Some(_), None) => (0, "the other creature's physics broke".into()),
        (None, Some(_)) => (1, "the other creature's physics broke".into()),
        _ => (coin, "dead heat: decided by a coin toss".into()),
    }
}

/// Who won a sumo bout: a knockout needs a winner, so a draw goes to the
/// one in control of the ring (nearer the centre), and else to `coin`.
fn sumo_winner(r: &SumoResult, coin: usize) -> (usize, String) {
    match r.winner {
        Some(w) => (w, r.reason.clone()),
        None if r.margin > 0.0 => (0, format!("{}; draw decided by ring control", r.reason)),
        None if r.margin < 0.0 => (1, format!("{}; draw decided by ring control", r.reason)),
        None => (coin, format!("{}; draw decided by a coin toss", r.reason)),
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Phase {
    Countdown,
    Running,
    DrumRoll,
    Reveal,
}

/// A bit of confetti, in fractions of the canvas.
struct Bit {
    pos: Vec2,
    vel: Vec2,
    angle: f32,
    spin: f32,
    size: f32,
    color: Color32,
}

/// A match on screen.
struct Show {
    round: usize,
    index: usize,
    event: Event,
    names: [String; 2],
    colors: [Color32; 2],
    /// The two creature files as played.
    files: [String; 2],
    arenas: Vec<Arena>,
    phase: Phase,
    /// Real seconds in this phase (simulated seconds while running).
    clock: f64,
    accumulator: f64,
    /// Watching a match again: its winner is already known.
    replay: Option<usize>,
    winner: Option<usize>,
    last_match: bool,
    camera: Option<(f32, f32)>,
    confetti: Vec<Bit>,
    excitement: f32,
}

/// Who won the match on screen (side 0 or 1), why, and what decided it:
/// the distances of a race, or how a bout ended. A tie goes to a coin toss
/// that depends only on the cup's seed and the match.
fn decide(show: &Show, seed: u64) -> (usize, String, Option<[Option<f64>; 2]>, Option<SumoRecord>) {
    let coin = usize::from(hash01((show.round * 1024 + show.index) as i64, seed ^ 0xC0FFEE) >= 0.5);
    match show.event {
        Event::Race => {
            let d: Vec<Option<f64>> = show.arenas.iter().map(|a| (!a.is_broken()).then(|| a.progress(0))).collect();
            let (w, why) = race_winner([d[0], d[1]], coin);
            (w, why, Some([d[0], d[1]]), None)
        }
        Event::Sumo => {
            let a = &show.arenas[0];
            let r = a.sumo_status().unwrap_or(SumoResult { winner: None, time: a.time, reason: "time up".into(), margin: 0.0 });
            let (w, why) = sumo_winner(&r, coin);
            (w, why, None, Some(SumoRecord { reason: r.reason.clone(), time_s: r.time, margin: r.margin }))
        }
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

impl Show {
    fn done(&self) -> bool {
        self.arenas.iter().all(|a| finished(a, self.event == Event::Sumo))
    }

    fn step_all(&mut self) {
        let sumo = self.event == Event::Sumo;
        for a in &mut self.arenas {
            if !finished(a, sumo) {
                a.step();
            }
        }
    }

    /// Seconds left in the match.
    fn left(&self) -> f64 {
        let a = &self.arenas[0];
        let limit = if self.event == Event::Sumo { a.rules.sumo_time } else { a.rules.race_time };
        (limit - self.arenas.iter().map(|a| a.time).fold(0.0, f64::max)).max(0.0)
    }

    /// How close the match is, for the crowd: racers neck and neck, or a
    /// wrestler near the edge.
    fn tension(&self) -> f32 {
        match self.event {
            Event::Race => {
                let d: Vec<f64> = self.arenas.iter().map(|a| a.progress(0)).collect();
                (1.0 - ((d[0] - d[1]).abs() / 1.5).min(1.0)) as f32
            }
            Event::Sumo => {
                let a = &self.arenas[0];
                let half = a.rules.ring_width / 2.0;
                (0..2).map(|k| (a.com(k)[0].abs() / half).min(1.0)).fold(0.0, f64::max).powi(2) as f32
            }
        }
    }

    fn throw_confetti(&mut self, n: usize) {
        let colors = [self.colors[self.winner.unwrap_or(0)], GOLD, Color32::WHITE, Color32::from_rgb(255, 110, 160), Color32::from_rgb(90, 200, 255), Color32::from_rgb(140, 230, 110)];
        let base = self.confetti.len() as i64;
        for i in 0..n as i64 {
            let u = |k: i64| hash01((base + i) * 8 + k, 0xC0FE) as f32;
            self.confetti.push(Bit {
                pos: Vec2::new(u(0), -0.05 - 0.5 * u(1)),
                vel: Vec2::new((u(2) - 0.5) * 0.25, 0.08 + 0.2 * u(3)),
                angle: u(4) * 6.3,
                spin: (u(5) - 0.5) * 14.0,
                size: 5.0 + 6.0 * u(6),
                color: colors[(u(7) * colors.len() as f32) as usize % colors.len()],
            });
        }
    }
}

pub struct Cup {
    event: Event,
    seed: u64,
    bracket: Option<Bracket>,
    show: Option<Show>,
    sound: Sound,
    /// The last error or note, and whether it is an error.
    message: Option<(String, bool)>,
}

impl Cup {
    pub fn new() -> Self {
        Self { event: Event::Race, seed: environment::new_seed(), bracket: None, show: None, sound: Sound::new(), message: None }
    }

    /// Once a match is played, the draw is fixed until the cup has its champion.
    fn draw_fixed(&self) -> bool {
        self.bracket.as_ref().is_some_and(|b| b.started() && b.champion().is_none())
    }

    /// Pick a saved cup (in the folder's cup-results) and open it.
    fn open_saved(&mut self, folder: &Path) {
        let mut dialog = rfd::FileDialog::new().set_title("Open a saved cup").add_filter("saved cup", &["json"]);
        let dir = folder.join("cup-results");
        if dir.is_dir() {
            dialog = dialog.set_directory(dir);
        }
        if let Some(path) = dialog.pick_file() {
            self.open(&path);
        }
    }

    fn open(&mut self, path: &Path) {
        match Bracket::open(path) {
            Ok(b) => {
                let played = b.rounds.iter().flatten().filter(|m| m.outcome.is_some()).count();
                let matches = b.rounds.iter().flatten().filter(|m| !m.bye).count();
                let name = path.file_name().map_or_else(String::new, |f| f.to_string_lossy().into_owned());
                self.message = Some((format!("Opened {name}: {played} of {matches} matches played."), false));
                (self.event, self.seed) = (b.event, b.seed);
                self.bracket = Some(b);
                self.show = None;
                self.sound.stop();
                self.sound.prepare();
            }
            Err(e) => self.message = Some((e, true)),
        }
    }

    fn new_draw(&mut self, entrants: &[Entrant], folder: &Path, rules: &Rules) {
        let draw_seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
        self.bracket = Some(Bracket::draw(entrants, self.event, self.seed, rules, folder, draw_seed));
        self.show = None;
        self.message = None;
        self.sound.stop();
        self.sound.prepare();
    }

    /// Space / Enter: play the next match, or once the winner is announced,
    /// go back to the bracket.
    pub fn key_next(&mut self) {
        match &self.show {
            None => {
                if let Some((r, m)) = self.bracket.as_ref().and_then(Bracket::next) {
                    self.start(r, m);
                }
            }
            Some(s) if s.phase == Phase::Reveal => self.back(),
            Some(_) => {}
        }
    }

    /// S: skip to the result (then past the drum roll).
    pub fn key_skip(&mut self) {
        if self.show.is_some() {
            self.skip();
        }
    }

    /// Start match `m` of round `r`, reading the creature files now; or watch
    /// a played match again, with the creatures as they were played.
    fn start(&mut self, r: usize, m: usize) {
        let Some(b) = &self.bracket else { return };
        let x = &b.rounds[r][m];
        let [Some(p0), Some(p1)] = x.players else { return };
        let read = |c: &Contestant| std::fs::read_to_string(&c.file).map_err(|e| format!("{}: {e}", c.name));
        let files = match (&x.played, x.outcome.is_some()) {
            (Some(f), true) => Ok(f.clone()),
            _ => read(&b.contestants[p0]).and_then(|a| Ok([a, read(&b.contestants[p1])?])),
        };
        let replay = x.outcome.is_some().then(|| usize::from(x.winner != Some(p0)));
        let parse = |c: &Contestant, text: &str| -> Result<Creature, String> {
            let creature: Creature = toml::from_str(text).map_err(|e| format!("{}: {e}", c.name))?;
            creature.validate(&b.rules).map_err(|e| format!("{} breaks the rules: {}", c.name, e.join("; ")))?;
            Ok(creature)
        };
        let loaded = files.and_then(|f| Ok(([parse(&b.contestants[p0], &f[0])?, parse(&b.contestants[p1], &f[1])?], f)));
        let (creatures, files) = match loaded {
            Ok(l) => l,
            Err(e) => {
                self.message = Some((format!("Could not load the match: {e}"), true));
                return;
            }
        };
        let rules = &b.rules;
        let arenas = match b.event {
            Event::Race => vec![Arena::race(&creatures[0], rules), Arena::race(&creatures[1], rules)],
            Event::Sumo => vec![Arena::sumo(&creatures[0], &creatures[1], rules)],
        };
        let mut colors = [b.contestants[p0].color(), b.contestants[p1].color()];
        if colors[0] == colors[1] {
            colors[1] = Color32::from_rgb(255 - colors[0].r() / 2, 255 - colors[0].g() / 2, 255 - colors[0].b() / 2);
        }
        let changed: Vec<&str> = [p0, p1]
            .iter()
            .zip(&files)
            .filter(|(p, f)| b.contestants[**p].fingerprint != fingerprint(f.as_bytes()))
            .map(|(p, _)| b.contestants[*p].name.as_str())
            .collect();
        self.message = (!changed.is_empty() && replay.is_none()).then(|| (format!("Changed since the draw: {} (the file as it is now is used)", changed.join(", ")), false));
        self.show = Some(Show {
            round: r,
            index: m,
            event: b.event,
            names: [b.contestants[p0].name.clone(), b.contestants[p1].name.clone()],
            colors,
            files,
            arenas,
            phase: Phase::Countdown,
            clock: 0.0,
            accumulator: 0.0,
            replay,
            winner: None,
            last_match: r == b.rounds.len() - 1,
            camera: None,
            confetti: Vec::new(),
            excitement: 0.2,
        });
        self.sound.play(Effect::Beep);
        self.sound.crowd(true);
    }

    /// The match is over: decide (unless it is a replay), record, save, drum roll.
    fn finish(&mut self) {
        let Some(show) = &mut self.show else { return };
        let side = match show.replay {
            Some(w) => w,
            None => {
                let Some(b) = &mut self.bracket else { return };
                let (side, decided_by, distances, sumo) = decide(show, b.seed);
                let x = &mut b.rounds[show.round][show.index];
                x.winner = x.players[side];
                x.outcome = Some(Outcome {
                    winner: show.names[side].clone(),
                    decided_by,
                    distances_m: distances,
                    sumo,
                    played_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
                    fingerprints: [fingerprint(show.files[0].as_bytes()), fingerprint(show.files[1].as_bytes())],
                });
                x.played = Some(show.files.clone());
                b.advance_winners();
                self.message = match b.save() {
                    Ok(_) => None,
                    Err(e) => Some((format!("Could not save the results: {e}"), true)),
                };
                side
            }
        };
        show.winner = Some(side);
        show.phase = Phase::DrumRoll;
        show.clock = 0.0;
        self.sound.crowd(false);
        self.sound.play(Effect::DrumRoll);
    }

    fn reveal(&mut self) {
        let Some(show) = &mut self.show else { return };
        show.phase = Phase::Reveal;
        show.clock = 0.0;
        let last = show.last_match;
        show.throw_confetti(if last { 420 } else { 220 });
        self.sound.play(if last { Effect::Champion } else { Effect::Fanfare });
        self.sound.play(Effect::Cheer);
    }

    /// Skip to the end of the match (and then past the drum roll).
    fn skip(&mut self) {
        let Some(show) = &mut self.show else { return };
        match show.phase {
            Phase::Countdown | Phase::Running => {
                // Every step the match would have taken, at once: the same result.
                let mut guard = 100_000;
                while !show.done() && guard > 0 {
                    show.step_all();
                    guard -= 1;
                }
                self.finish();
            }
            Phase::DrumRoll => self.reveal(),
            Phase::Reveal => {}
        }
    }

    fn back(&mut self) {
        self.show = None;
        self.sound.stop();
    }

    /// Advance the match on screen; true while the Cup needs redrawing.
    pub fn advance(&mut self, dt: f64, speed: f32, paused: bool) -> bool {
        let Some(show) = &mut self.show else { return self.bracket.is_some() };
        match show.phase {
            Phase::Countdown => {
                let before = show.clock;
                show.clock += dt;
                if before.floor() != show.clock.floor() && show.clock < COUNTDOWN {
                    self.sound.play(Effect::Beep);
                }
                if show.clock >= COUNTDOWN {
                    show.phase = Phase::Running;
                    show.clock = 0.0;
                    show.excitement = 1.0;
                    self.sound.play(Effect::Go);
                    self.sound.play(Effect::Cheer);
                }
            }
            Phase::Running => {
                show.clock += dt;
                if !paused {
                    show.accumulator += dt * speed as f64;
                    let step = show.arenas[0].rules.dt;
                    let mut budget = 240;
                    while show.accumulator >= step && budget > 0 && !show.done() {
                        show.accumulator -= step;
                        budget -= 1;
                        show.step_all();
                    }
                }
                let target = 0.25 + 0.75 * show.tension();
                show.excitement += (target - show.excitement) * (dt as f32 * 1.5).min(1.0);
                self.sound.crowd(true);
                if show.done() {
                    self.finish();
                }
            }
            Phase::DrumRoll => {
                show.clock += dt;
                show.excitement = 0.6;
                if show.clock >= DRUM_ROLL {
                    self.reveal();
                }
            }
            Phase::Reveal => {
                show.clock += dt;
                show.excitement = 1.0;
                let dt = dt as f32;
                for b in &mut show.confetti {
                    b.vel.y = (b.vel.y + 0.25 * dt).min(0.22);
                    b.vel.x *= 1.0 - 0.8 * dt;
                    b.pos += b.vel * dt + Vec2::new((b.angle * 0.7).sin() * 0.03 * dt, 0.0);
                    b.angle += b.spin * dt;
                }
                show.confetti.retain(|b| b.pos.y < 1.1);
                // The champion gets a second shower.
                if show.last_match && show.clock < 2.5 && show.confetti.len() < 200 {
                    show.throw_confetti(60);
                }
            }
        }
        true
    }

    // ------------------------------------------------------------ panel

    /// The Cup form: event, field, draw, next match, sound.
    pub fn panel(&mut self, ui: &mut egui::Ui, folder: &Path, rules: &Rules, entrants: &[Entrant]) {
        ui.heading("Cup");
        ui.label("A knockout: one against one, and the winner goes on to the next round. Only the winner of each match is announced.");
        if self.show.is_some() {
            self.match_controls(ui);
            ui.separator();
        }
        let locked = self.bracket.as_ref().is_some_and(Bracket::started);
        if let Some(b) = self.bracket.as_ref().filter(|_| locked) {
            (self.event, self.seed) = (b.event, b.seed);
        }
        ui.add_enabled_ui(!locked && self.show.is_none(), |ui| {
            ui.horizontal(|ui| {
                ui.label("Event");
                ui.selectable_value(&mut self.event, Event::Race, "race").on_hover_text("Two creatures race side by side on the same track: the one that gets farther wins");
                ui.selectable_value(&mut self.event, Event::Sumo, "sumo").on_hover_text("Push the other creature off the platform");
            });
            ui.horizontal(|ui| {
                ui.label("Field");
                ui.label(RichText::new(format!("environment {}", self.seed)).strong());
                if ui.button("New field").on_hover_text("A random environment seed: new hills, terrain, slope and friction").clicked() {
                    self.seed = environment::new_seed();
                }
            });
        });
        // An unplayed draw follows the settings (and the folder's rules).
        if let Some(b) = self.bracket.as_mut().filter(|_| !locked) {
            (b.event, b.seed, b.rules) = (self.event, self.seed, cup_rules(rules, self.event, self.seed));
        }
        let field = field_text(self.event, self.seed, &cup_rules(rules, self.event, self.seed));
        ui.label(RichText::new(field).small());
        if self.event == Event::Race {
            ui.label(RichText::new(format!("Races in the cup last a minute ({RACE_TIME:.0} s instead of the usual {:.0} s).", rules.race_time)).small().weak());
        }
        ui.label(
            RichText::new("The physics are those of training: with this environment seed in the Train tab, students train on exactly this field.")
                .small()
                .weak(),
        );
        if locked {
            ui.label(RichText::new("The cup has started: its event and field are fixed.").small().weak());
        }
        ui.separator();

        ui.label(format!("Competitors: the {} creatures ticked on the left.", entrants.len()));
        let fixed = self.draw_fixed();
        let can_draw = entrants.len() >= 2 && self.show.is_none() && !fixed;
        if ui.add_enabled(can_draw, egui::Button::new("Random draw")).on_hover_text("Draw the ticked creatures into a new bracket, at random").clicked() {
            self.new_draw(entrants, folder, rules);
        }
        if fixed {
            ui.label(RichText::new("Matches have been played: the draw is fixed until the cup has its champion.").small().weak());
        } else if entrants.len() < 2 {
            ui.label(RichText::new("Tick at least two creatures.").small().weak());
        }
        if ui.add_enabled(self.show.is_none(), egui::Button::new("Open a saved cup…")).on_hover_text("A cup from cup-results: watch its matches again, or go on with it").clicked() {
            self.open_saved(folder);
        }

        let mut play = None;
        // Until the winner is announced, the panel must not give it away.
        let suspense = self.show.as_ref().is_some_and(|s| s.phase != Phase::Reveal);
        if let Some(b) = self.bracket.as_ref().filter(|_| !suspense) {
            ui.add_space(6.0);
            match (b.next(), b.champion()) {
                (Some((r, m)), _) => {
                    let x = &b.rounds[r][m];
                    ui.label(format!("Next: {}, match {}", round_name(r, b.rounds.len()), m + 1));
                    ui.label(RichText::new(format!("{}  vs  {}", b.name(x.players[0]), b.name(x.players[1]))).strong());
                    if ui.add_enabled(self.show.is_none(), egui::Button::new("▶ Play the next match")).clicked() {
                        play = Some((r, m));
                    }
                    ui.label(RichText::new("Or click a match in the bracket. Click a played match to watch it again.").small().weak());
                }
                (None, Some(c)) => {
                    ui.label(RichText::new(format!("Champion: {}", b.contestants[c].name)).strong().color(GOLD));
                    ui.label(RichText::new("Click a match in the bracket to watch it again.").small().weak());
                }
                (None, None) => {}
            }
            if let Some(file) = b.results.as_ref().and_then(|p| p.file_name()) {
                let text = format!(
                    "Every match is saved when it ends, in cup-results/{} with the creatures as they were played: the detailed results for the teacher (never shown here), and the matches to watch again, also after closing the program (Open a saved cup…).",
                    file.to_string_lossy()
                );
                ui.label(RichText::new(text).small().weak());
            }
        }
        if let Some((r, m)) = play {
            self.start(r, m);
        }
        ui.separator();
        if ui.checkbox(&mut self.sound.on, "Sound").changed() && !self.sound.on {
            self.sound.stop();
        }
        if let Some((m, error)) = &self.message {
            let color = if *error { Color32::LIGHT_RED } else { ui.visuals().warn_fg_color };
            ui.colored_label(color, m);
        }
    }


    fn match_controls(&mut self, ui: &mut egui::Ui) {
        let Some(show) = &self.show else { return };
        let rounds = self.bracket.as_ref().map_or(1, |b| b.rounds.len());
        ui.label(RichText::new(format!("{}, match {}{}", round_name(show.round, rounds), show.index + 1, if show.replay.is_some() { " (again)" } else { "" })).strong());
        ui.label(format!("{}  vs  {}", show.names[0], show.names[1]));
        let phase = show.phase;
        ui.horizontal(|ui| {
            if phase != Phase::Reveal && ui.button("Skip ▶▶").on_hover_text("Jump to the result (the match is still fully simulated)").clicked() {
                self.skip();
            }
            if ui.button(if phase == Phase::Reveal { "Back to the bracket" } else { "Stop" }).clicked() {
                if phase == Phase::Reveal || phase == Phase::DrumRoll {
                    self.back();
                } else {
                    // Stopping a match half-way does not decide it.
                    self.show = None;
                    self.sound.stop();
                }
            }
        });
    }

    // ------------------------------------------------------------ canvas

    /// The bracket or the match. `full`: full screen (the side panels are hidden).
    pub fn view(&mut self, ui: &mut egui::Ui, rules: &Rules, full: bool) {
        if self.show.is_some() {
            self.draw_show(ui);
        } else {
            self.draw_bracket(ui, rules, full);
        }
    }

    fn draw_bracket(&mut self, ui: &mut egui::Ui, rules: &Rules, full: bool) {
        let avail = ui.available_size();
        let first = self.bracket.as_ref().map_or(0, |b| b.rounds[0].len());
        let card = Vec2::new(0.0, 54.0);
        let preview_h = 150.0;
        let needed = 116.0 + first as f32 * (card.y + 14.0) + preview_h + 20.0;
        let mut clicked = None;
        egui::ScrollArea::vertical().id_salt("bracket").show(ui, |ui| {
            let (resp, painter) = ui.allocate_painter(Vec2::new(avail.x, needed.max(avail.y)), Sense::click());
            let rect = resp.rect;
            let painter = painter.with_clip_rect(rect.intersect(ui.clip_rect()));
            scene::gradient(&painter, rect, &[(0.0, Color32::from_rgb(12, 16, 36)), (0.6, Color32::from_rgb(26, 26, 58)), (1.0, Color32::from_rgb(44, 32, 66))]);
            let event = self.event;
            let title = match event {
                Event::Race => "CPG ARENA CUP · RACE",
                Event::Sumo => "CPG ARENA CUP · SUMO",
            };
            shadow_text(&painter, rect.center_top() + Vec2::new(0.0, 14.0), Align2::CENTER_TOP, title, scene::display(30.0), GOLD);
            let field = Rect::from_min_max(Pos2::new(rect.left() + 16.0, rect.bottom() - preview_h - 10.0), Pos2::new(rect.right() - 16.0, rect.bottom() - 10.0));
            let field_rules = self.bracket.as_ref().map_or_else(|| cup_rules(rules, self.event, self.seed), |b| b.rules.clone());
            self.draw_field(&painter, field, &field_rules);
            let Some(b) = &self.bracket else {
                shadow_text(
                    &painter,
                    rect.center() - Vec2::new(0.0, 40.0),
                    Align2::CENTER_CENTER,
                    if full { "No draw yet: press Esc, tick the competitors on the left, then Random draw." } else { "Tick the competitors on the left, then Random draw." },
                    FontId::proportional(18.0),
                    Color32::from_gray(220),
                );
                return;
            };
            let pointer = resp.hover_pos();
            let time = ui.input(|i| i.time) as f32;
            let area = Rect::from_min_max(Pos2::new(rect.left() + 24.0, rect.top() + 92.0), Pos2::new(rect.right() - 160.0, field.top() - 16.0));
            let n = b.rounds.len();
            let col = area.width() / n as f32;
            let w = (col - 26.0).clamp(90.0, 300.0);
            let slot = area.height() / first as f32;
            let pos = |r: usize, m: usize| -> Rect {
                let y = area.top() + slot * (1 << r) as f32 * (m as f32 + 0.5);
                Rect::from_center_size(Pos2::new(area.left() + col * r as f32 + w / 2.0, y), Vec2::new(w, card.y))
            };
            let next = b.next();
            for (r, round) in b.rounds.iter().enumerate() {
                shadow_text(&painter, Pos2::new(area.left() + col * r as f32 + w / 2.0, area.top() - 8.0), Align2::CENTER_BOTTOM, round_name(r, n).to_uppercase(), FontId::proportional(12.0), Color32::from_gray(170));
                for (m, x) in round.iter().enumerate() {
                    let rc = pos(r, m);
                    // The line to the next round, gold once decided.
                    if r + 1 < n {
                        let to = pos(r + 1, m / 2);
                        let color = if x.winner.is_some() { GOLD.gamma_multiply(0.8) } else { Color32::from_rgb(80, 88, 124) };
                        let mid = rc.right() + (to.left() - rc.right()) / 2.0;
                        let pts = vec![rc.right_center(), Pos2::new(mid, rc.center().y), Pos2::new(mid, to.center().y), to.left_center()];
                        painter.add(Shape::line(pts, Stroke::new(2.0, color)));
                    }
                    let hover = pointer.is_some_and(|p| rc.contains(p));
                    let clickable = Bracket::playable(x) || x.outcome.is_some();
                    if hover && clickable {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        if resp.clicked() {
                            clicked = Some((r, m));
                        }
                    }
                    card_shape(&painter, rc, b, x, next == Some((r, m)), hover && clickable, time);
                }
            }
            // The trophy, and the champion under it.
            let fin = pos(n - 1, 0);
            let tc = Pos2::new(area.right() + 84.0, fin.center().y - 30.0);
            trophy(&painter, tc, 80.0);
            let champ = b.champion().map_or("?".to_string(), |c| b.contestants[c].name.clone());
            shadow_text(&painter, tc + Vec2::new(0.0, 62.0), Align2::CENTER_TOP, champ, scene::display(18.0), if b.champion().is_some() { GOLD } else { Color32::from_gray(150) });
        });
        if let Some((r, m)) = clicked {
            self.start(r, m);
        }
    }

    /// The field of the cup, small, at the bottom of the bracket.
    fn draw_field(&self, painter: &egui::Painter, rect: Rect, rules: &Rules) {
        let p = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
        p.rect_filled(rect, 10.0, Color32::from_rgb(20, 22, 40));
        match self.event {
            Event::Race => environment::draw_terrain(&p, rect.shrink2(Vec2::new(30.0, 26.0)), rules, scene::Backdrop::Stadium),
            Event::Sumo => environment::draw_ring(&p, rect, rules),
        }
        p.rect_stroke(rect, 10.0, Stroke::new(1.5, Color32::from_rgb(90, 96, 140)), StrokeKind::Inside);
        let label = format!("THE FIELD · ENVIRONMENT {}", self.seed);
        let at = rect.center_top() + Vec2::new(0.0, 8.0);
        let size = p.layout_no_wrap(label.clone(), FontId::proportional(13.0), Color32::WHITE).size();
        p.rect_filled(Rect::from_center_size(at + Vec2::new(0.0, size.y / 2.0), size + Vec2::new(20.0, 8.0)), 8.0, Color32::from_black_alpha(170));
        p.text(at, Align2::CENTER_TOP, label, FontId::proportional(13.0), Color32::WHITE);
    }

    fn draw_show(&mut self, ui: &mut egui::Ui) {
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click());
        let rect = resp.rect;
        let painter = painter.with_clip_rect(rect);
        let rounds = self.bracket.as_ref().map_or(1, |b| b.rounds.len());
        let Some(show) = &mut self.show else { return };
        match show.event {
            Event::Race => draw_race(&painter, rect, show),
            Event::Sumo => draw_sumo(&painter, rect, show),
        }
        // Header: the round, the two names, the time left.
        let head = format!("{} · MATCH {}", round_name(show.round, rounds).to_uppercase(), show.index + 1);
        shadow_text(&painter, rect.center_top() + Vec2::new(0.0, 8.0), Align2::CENTER_TOP, head, FontId::proportional(13.0), Color32::from_gray(220));
        let vs = rect.center_top() + Vec2::new(0.0, 30.0);
        shadow_text(&painter, vs, Align2::CENTER_TOP, "vs", scene::display(18.0), Color32::from_gray(220));
        let room = rect.width() / 2.0 - 40.0;
        for (k, (align, dx)) in [(Align2::RIGHT_TOP, -26.0), (Align2::LEFT_TOP, 26.0)].into_iter().enumerate() {
            let font = fitting_font(&painter, &show.names[k], 24.0, room);
            let name = fit(&painter, &show.names[k], &font, room);
            shadow_text(&painter, vs + Vec2::new(dx, 0.0), align, name, font, show.colors[k]);
        }
        if matches!(show.phase, Phase::Running | Phase::Countdown) {
            let clock = format!("{:.0}", show.left().ceil());
            let c = rect.center_top() + Vec2::new(0.0, 94.0);
            painter.circle_filled(c, 24.0, Color32::from_black_alpha(150));
            painter.circle_stroke(c, 24.0, Stroke::new(2.0, GOLD));
            painter.text(c, Align2::CENTER_CENTER, clock, scene::display(21.0), Color32::WHITE);
        }
        let center = rect.center();
        match show.phase {
            Phase::Countdown => {
                painter.rect_filled(rect, 0.0, Color32::from_black_alpha(90));
                let k = show.clock.floor();
                let frac = (show.clock - k) as f32;
                let size = 170.0 * (1.35 - 0.35 * (1.0 - (1.0 - frac).powi(3)));
                let alpha = (255.0 * (1.0 - frac.powi(4))) as u8;
                let digit = format!("{}", (COUNTDOWN - k) as i32);
                shadow_text(&painter, center, Align2::CENTER_CENTER, digit, scene::display(size), Color32::from_rgba_unmultiplied(255, 255, 255, alpha));
            }
            Phase::Running if show.clock < GO => {
                let u = (show.clock / GO) as f32;
                let alpha = (255.0 * (1.0 - u * u)) as u8;
                shadow_text(&painter, center, Align2::CENTER_CENTER, "GO!", scene::display(150.0 + 60.0 * u), Color32::from_rgba_unmultiplied(GOLD.r(), GOLD.g(), GOLD.b(), alpha));
            }
            Phase::Running => {}
            Phase::DrumRoll => {
                painter.rect_filled(rect, 0.0, Color32::from_black_alpha(130));
                let dots = ".".repeat((show.clock * 3.0) as usize % 4);
                let shake = (show.clock as f32 * 40.0).sin() * 2.0;
                shadow_text(&painter, center + Vec2::new(shake, 0.0), Align2::CENTER_CENTER, format!("And the winner is{dots}"), scene::display(44.0), Color32::WHITE);
            }
            Phase::Reveal => {
                painter.rect_filled(rect, 0.0, Color32::from_black_alpha(120));
                let w = show.winner.unwrap_or(0);
                let pop = (show.clock as f32 / 0.35).min(1.0);
                let ease = 1.0 - (1.0 - pop).powi(3);
                if show.last_match {
                    trophy(&painter, center - Vec2::new(0.0, 120.0 * ease), 120.0 * ease.max(0.01));
                    shadow_text(&painter, center + Vec2::new(0.0, 4.0), Align2::CENTER_BOTTOM, "CHAMPION OF THE CUP", scene::display(26.0), GOLD);
                }
                banner(&painter, center + Vec2::new(0.0, 60.0), (rect.width() * 0.62).min(680.0) * ease, &show.names[w], show.colors[w], !show.last_match);
                for b in &show.confetti {
                    let p = rect.min + Vec2::new(b.pos.x * rect.width(), b.pos.y * rect.height());
                    // Flutter: the bit turns, so its height changes.
                    let (s, c) = b.angle.sin_cos();
                    let h = b.size * 0.5 * (b.angle * 1.7).cos().abs().max(0.15);
                    let corner = |u: f32, v: f32| p + Vec2::new(u * c - v * s, u * s + v * c);
                    let pts = vec![corner(b.size / 2.0, h / 2.0), corner(-b.size / 2.0, h / 2.0), corner(-b.size / 2.0, -h / 2.0), corner(b.size / 2.0, -h / 2.0)];
                    painter.add(Shape::convex_polygon(pts, b.color, Stroke::NONE));
                }
                if show.clock > 1.5 {
                    shadow_text(&painter, rect.center_bottom() - Vec2::new(0.0, 16.0), Align2::CENTER_BOTTOM, "Click to go back to the bracket", FontId::proportional(14.0), Color32::from_gray(210));
                }
            }
        }
        if show.phase == Phase::Reveal && resp.clicked() {
            self.back();
        }
    }
}

/// `text` shortened in the middle with "…" to fit in `max` pixels: the end
/// often tells similar names apart ("My Worm (ga 12:20:01)").
fn fit_middle(painter: &egui::Painter, text: &str, font: &FontId, max: f32) -> String {
    let width = |s: &str| painter.layout_no_wrap(s.to_string(), font.clone(), Color32::WHITE).size().x;
    if width(text) <= max {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut head, mut tail) = ((chars.len() + 1) / 2, chars.len() / 2);
    loop {
        let s: String = chars[..head].iter().chain(['…'].iter()).chain(chars[chars.len() - tail..].iter()).collect();
        if width(&s) <= max || head + tail == 0 {
            return s;
        }
        if head > tail { head -= 1 } else { tail -= 1 }
    }
}

/// One match in the bracket: two rows (a colour chip and a name each), the
/// winner in bold with a star.
fn card_shape(painter: &egui::Painter, rc: Rect, b: &Bracket, x: &Match, next: bool, hover: bool, time: f32) {
    painter.rect_filled(rc.translate(Vec2::new(3.0, 4.0)), 9.0, Color32::from_black_alpha(110));
    let fill = if hover { Color32::from_rgb(48, 56, 92) } else { Color32::from_rgb(32, 38, 66) };
    painter.rect_filled(rc, 9.0, fill);
    let top = Rect::from_min_max(rc.min, Pos2::new(rc.right(), rc.center().y));
    painter.rect_filled(top.shrink(1.0), 8.0, Color32::from_white_alpha(6));
    let border = if next {
        let pulse = 0.5 + 0.5 * (time * 4.0).sin();
        Stroke::new(2.0 + pulse, scene::mix(GOLD, Color32::WHITE, pulse * 0.4))
    } else if Bracket::playable(x) {
        Stroke::new(1.5, Color32::from_rgb(130, 150, 210))
    } else {
        Stroke::new(1.0, Color32::from_rgb(76, 84, 120))
    };
    painter.rect_stroke(rc, 9.0, border, StrokeKind::Inside);
    painter.line_segment([Pos2::new(rc.left() + 8.0, rc.center().y), Pos2::new(rc.right() - 8.0, rc.center().y)], Stroke::new(1.0, Color32::from_white_alpha(18)));
    for side in 0..2 {
        let row = Rect::from_min_size(rc.min + Vec2::new(0.0, side as f32 * rc.height() / 2.0), Vec2::new(rc.width(), rc.height() / 2.0));
        let y = row.center().y;
        match x.players[side] {
            Some(p) => {
                let c = &b.contestants[p];
                let won = x.winner == Some(p) && !x.bye;
                let lost = x.winner.is_some() && x.winner != Some(p);
                let chip = Rect::from_center_size(Pos2::new(row.left() + 14.0, y), Vec2::splat(12.0));
                painter.rect_filled(chip, 3.0, if lost { c.color().gamma_multiply(0.35) } else { c.color() });
                let font = if won { scene::display(13.0) } else { FontId::proportional(13.0) };
                let color = if won { Color32::WHITE } else if lost { Color32::from_gray(110) } else { Color32::from_gray(225) };
                let name = fit_middle(painter, &c.name, &font, row.width() - 48.0);
                painter.text(Pos2::new(row.left() + 26.0, y), Align2::LEFT_CENTER, name, font, color);
                if won {
                    painter.text(Pos2::new(row.right() - 10.0, y), Align2::RIGHT_CENTER, "★", FontId::proportional(15.0), GOLD);
                }
            }
            None => {
                let text = if x.bye { "bye" } else { "—" };
                painter.text(Pos2::new(row.left() + 26.0, y), Align2::LEFT_CENTER, text, FontId::proportional(13.0), Color32::from_gray(120));
            }
        }
    }
}

/// The winner's ribbon: a band in their colour with folded ends.
fn banner(painter: &egui::Painter, center: Pos2, width: f32, name: &str, color: Color32, label: bool) {
    if width < 10.0 {
        return;
    }
    let h = 96.0;
    let band = Rect::from_center_size(center, Vec2::new(width, h));
    let dark = scene::mix(color, Color32::BLACK, 0.55);
    for side in [-1.0f32, 1.0] {
        let x = if side < 0.0 { band.left() } else { band.right() };
        let tail = [
            Pos2::new(x - side * 10.0, band.top() + 18.0),
            Pos2::new(x + side * 46.0, band.top() + 18.0),
            Pos2::new(x + side * 30.0, band.center().y + 18.0),
            Pos2::new(x + side * 46.0, band.bottom() + 18.0),
            Pos2::new(x - side * 10.0, band.bottom() + 18.0),
        ];
        painter.add(Shape::convex_polygon(vec![tail[0], tail[1], tail[2], tail[4]], dark, Stroke::NONE));
        painter.add(Shape::convex_polygon(vec![tail[2], tail[3], tail[4]], dark, Stroke::NONE));
    }
    painter.rect_filled(band.translate(Vec2::new(0.0, 6.0)), 6.0, Color32::from_black_alpha(90));
    scene::gradient(painter, band, &[(0.0, scene::mix(color, Color32::WHITE, 0.25)), (1.0, scene::mix(color, Color32::BLACK, 0.25))]);
    painter.rect_stroke(band, 4.0, Stroke::new(3.0, GOLD), StrokeKind::Inside);
    if label {
        shadow_text(painter, band.center_top() + Vec2::new(0.0, 8.0), Align2::CENTER_TOP, "WINNER", FontId::proportional(14.0), Color32::WHITE);
    }
    let font = fitting_font(painter, name, 44.0, width - 40.0);
    let name = fit(painter, name, &font, width - 40.0);
    shadow_text(painter, band.center() + Vec2::new(0.0, if label { 8.0 } else { 0.0 }), Align2::CENTER_CENTER, name, font, Color32::WHITE);
}

/// Smoothly move a camera towards (centre x in m, pixels per m).
fn follow(camera: &mut Option<(f32, f32)>, target: (f32, f32)) -> (f32, f32) {
    let cam = match *camera {
        Some((x, s)) => (x + (target.0 - x) * 0.08, s + (target.1 - s) * 0.08),
        None => target,
    };
    *camera = Some(cam);
    cam
}

/// A race: the stands on top, then one lane per creature on the same track.
fn draw_race(painter: &egui::Painter, rect: Rect, show: &mut Show) {
    let stands = Rect::from_min_max(rect.min, Pos2::new(rect.right(), rect.top() + rect.height() * 0.22));
    scene::crowd(painter, stands, show.excitement);
    let lanes = Rect::from_min_max(Pos2::new(rect.left(), stands.bottom()), rect.max);
    let lane_h = lanes.height() / 2.0;
    let xs: Vec<f32> = show.arenas.iter().map(|a| a.com(0)[0] as f32).filter(|x| x.is_finite()).collect();
    let (lo, hi) = xs.iter().fold((f32::MAX, f32::MIN), |(l, h), &x| (l.min(x), h.max(x)));
    let span = (hi - lo + 4.0).max(6.0);
    let fit = (rect.width() / span).min(lane_h / 1.5).clamp(8.0, 150.0);
    let (cx, scale) = follow(&mut show.camera, ((lo + hi) / 2.0, fit));
    let (x0, x1) = (cx - rect.width() / 2.0 / scale, cx + rect.width() / 2.0 / scale);
    for (k, a) in show.arenas.iter().enumerate() {
        let lane = Rect::from_min_size(Pos2::new(rect.left(), lanes.top() + k as f32 * lane_h), Vec2::new(rect.width(), lane_h));
        let ground_y = lane.top() + lane_h * 0.78;
        let to_screen = |p: [f32; 2]| Pos2::new(rect.center().x + (p[0] - cx) * scale, ground_y - p[1] * scale);
        scene::track(painter, lane, ground_y, &to_screen, (x0, x1), scale, &a.terrain, scene::Marks::None, scene::Backdrop::Stadium);
        let lp = painter.with_clip_rect(lane.intersect(painter.clip_rect()));
        scene::creatures(&lp, a, &[show.colors[k]], &to_screen, scale, 255);
        // Name tag in the lane.
        let tag = Rect::from_min_size(lane.left_top() + Vec2::new(12.0, 10.0), Vec2::new(16.0, 16.0));
        painter.rect_filled(tag, 4.0, show.colors[k]);
        shadow_text(painter, tag.right_center() + Vec2::new(8.0, 0.0), Align2::LEFT_CENTER, &show.names[k], scene::display(18.0), Color32::WHITE);
        painter.line_segment([lane.left_top(), lane.right_top()], Stroke::new(2.0, Color32::from_black_alpha(160)));
    }
    let slope = show.arenas[0].rules.slope;
    if slope != 0.0 {
        let text = format!("slope {slope:+.1}° ({})", if slope > 0.0 { "uphill" } else { "downhill" });
        shadow_text(painter, Pos2::new(rect.right() - 12.0, lanes.top() + 12.0), Align2::RIGHT_TOP, text, FontId::proportional(13.0), Color32::from_gray(230));
    }
}

/// A sumo bout: the stands behind, the platform on the arena floor.
fn draw_sumo(painter: &egui::Painter, rect: Rect, show: &mut Show) {
    scene::sumo_view(painter, rect, &show.arenas[0], &show.colors, &show.names, show.excitement);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entrants(n: usize) -> Vec<Entrant> {
        (0..n).map(|i| Entrant { name: format!("C{i}"), path: PathBuf::from(format!("c{i}.toml")), color: Color32::RED }).collect()
    }

    #[test]
    fn the_draw_puts_everyone_in_once_and_spreads_the_byes() {
        for n in 2..=17 {
            let b = Bracket::draw(&entrants(n), Event::Race, 1, &Rules::default(), Path::new("."), 42 + n as u64);
            let size = n.next_power_of_two();
            assert_eq!(b.rounds.len(), size.trailing_zeros() as usize, "n = {n}");
            assert_eq!(b.rounds.last().unwrap().len(), 1);
            let mut seen: Vec<usize> = b.rounds[0].iter().flat_map(|m| m.players.iter().flatten().copied()).collect();
            seen.sort();
            assert_eq!(seen, (0..n).collect::<Vec<_>>(), "n = {n}");
            let byes: Vec<&Match> = b.rounds[0].iter().filter(|m| m.bye).collect();
            assert_eq!(byes.len(), size - n);
            // A bye has one player, who goes straight through.
            assert!(byes.iter().all(|m| m.players[0].is_some() && m.players[1].is_none() && m.winner == m.players[0]));
            if b.rounds.len() > 1 {
                let through: usize = b.rounds[1].iter().map(|m| m.players.iter().flatten().count()).sum();
                assert_eq!(through, size - n);
            }
            assert!(b.next().is_some() && b.champion().is_none());
        }
        // Another draw, another order.
        let a = Bracket::draw(&entrants(8), Event::Race, 1, &Rules::default(), Path::new("."), 1);
        let b = Bracket::draw(&entrants(8), Event::Race, 1, &Rules::default(), Path::new("."), 2);
        assert_ne!(a.rounds[0].iter().map(|m| m.players).collect::<Vec<_>>(), b.rounds[0].iter().map(|m| m.players).collect::<Vec<_>>());
    }

    #[test]
    fn every_match_has_a_winner() {
        assert_eq!(race_winner([Some(3.0), Some(2.0)], 1).0, 0);
        assert_eq!(race_winner([Some(-1.0), Some(2.0)], 0).0, 1);
        assert_eq!(race_winner([None, Some(-5.0)], 0).0, 1);
        assert_eq!(race_winner([Some(1.0), Some(1.0)], 1), (1, "dead heat: decided by a coin toss".into()));
        assert_eq!(race_winner([None, None], 0).0, 0);
        let r = |winner, margin| SumoResult { winner, time: 20.0, reason: "time up, ring control".into(), margin };
        assert_eq!(sumo_winner(&r(Some(1), -1.0), 0).0, 1);
        assert_eq!(sumo_winner(&r(None, 0.03), 1).0, 0);
        assert_eq!(sumo_winner(&r(None, -0.02), 0).0, 1);
        assert_eq!(sumo_winner(&r(None, 0.0), 1).0, 1);
    }

    #[test]
    fn the_field_is_the_training_environment() {
        let folder = Rules { race_time: 12.0, ..Rules::default() };
        let choice = environment::TrackChoice { seeded: true, seed: 4711 };
        // Exactly the Train tab's environment 4711; only the race is longer.
        let race = cup_rules(&folder, Event::Race, 4711);
        assert_eq!(race.race_time, 60.0);
        assert_eq!(toml::to_string(&Rules { race_time: 12.0, ..race }).unwrap(), toml::to_string(&choice.rules(&folder)).unwrap());
        let sumo = cup_rules(&folder, Event::Sumo, 4711);
        assert_eq!(toml::to_string(&sumo).unwrap(), toml::to_string(&choice.rules(&folder)).unwrap());
    }

    /// A drawn cup of worm, walker and tailfin (copied to a new folder), not played yet.
    fn draw_cup(event: Event, dir_name: &str) -> (Cup, PathBuf) {
        let dir = std::env::temp_dir().join(dir_name);
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let mut list = Vec::new();
        for c in ["worm", "walker", "tailfin"] {
            let path = dir.join(format!("{c}.toml"));
            std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("creatures/{c}.toml")), &path).unwrap();
            list.push(Entrant { name: Creature::load(&path).unwrap().name, path, color: Color32::BLUE });
        }
        let mut cup = Cup::new();
        cup.sound.on = false;
        (cup.event, cup.seed) = (event, 4711);
        let rules = Rules { race_time: 5.0, sumo_time: 5.0, ..Rules::default() };
        cup.new_draw(&list, &dir, &rules);
        (cup, dir)
    }

    /// Play up to `n` matches without the screen, each skipped to its result.
    fn play(cup: &mut Cup, n: usize) {
        for _ in 0..n {
            let Some((r, m)) = cup.bracket.as_ref().unwrap().next() else { return };
            cup.start(r, m);
            assert!(cup.show.is_some(), "{:?}", cup.message);
            cup.skip();
            assert_eq!(cup.show.as_ref().unwrap().phase, Phase::DrumRoll);
            cup.skip();
            assert_eq!(cup.show.as_ref().unwrap().phase, Phase::Reveal);
            cup.back();
        }
    }

    /// Play a whole cup.
    fn play_cup(event: Event, dir_name: &str) -> (Cup, PathBuf) {
        let (mut cup, dir) = draw_cup(event, dir_name);
        play(&mut cup, usize::MAX);
        (cup, dir)
    }

    fn results(cup: &Cup) -> PathBuf {
        cup.bracket.as_ref().unwrap().results.clone().expect("saved")
    }

    #[test]
    fn the_keys_play_a_cup() {
        let dir = std::env::temp_dir().join("arena-gui-cup-keys");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let list: Vec<Entrant> = ["worm", "walker"]
            .iter()
            .map(|c| {
                let path = dir.join(format!("{c}.toml"));
                std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("creatures/{c}.toml")), &path).unwrap();
                Entrant { name: c.to_string(), path, color: Color32::RED }
            })
            .collect();
        let mut cup = Cup::new();
        cup.sound.on = false;
        cup.key_next(); // no draw yet: nothing happens
        assert!(cup.show.is_none());
        cup.new_draw(&list, &dir, &Rules { race_time: 5.0, ..Rules::default() });
        cup.key_next(); // Space: the next match starts with the countdown
        assert_eq!(cup.show.as_ref().unwrap().phase, Phase::Countdown);
        cup.key_next(); // Space during a match does nothing
        assert_eq!(cup.show.as_ref().unwrap().phase, Phase::Countdown);
        cup.key_skip(); // S: to the drum roll ...
        assert_eq!(cup.show.as_ref().unwrap().phase, Phase::DrumRoll);
        cup.key_skip(); // ... and to the winner
        assert_eq!(cup.show.as_ref().unwrap().phase, Phase::Reveal);
        cup.key_next(); // Space: back to the bracket, which has its champion
        assert!(cup.show.is_none() && cup.bracket.as_ref().unwrap().champion().is_some());
    }

    #[test]
    fn a_race_cup_is_played_to_the_end_and_the_data_is_saved() {
        let (cup, dir) = play_cup(Event::Race, "arena-cup-race");
        let b = cup.bracket.as_ref().unwrap();
        let champion = b.champion().expect("a champion");
        // 3 creatures: one bye, then two matches.
        let played: Vec<&Match> = b.rounds.iter().flatten().filter(|m| m.outcome.is_some()).collect();
        assert_eq!(played.len(), 2);
        let saved = results(&cup);
        assert_eq!(saved.parent().unwrap(), dir.join("cup-results"));
        assert!(saved.file_name().unwrap().to_string_lossy().starts_with("cup-race-"));
        let json = std::fs::read_to_string(&saved).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["champion"], b.contestants[champion].name);
        assert_eq!(v["environment_seed"], 4711);
        assert_eq!(v["rules"]["race_time"], 60.0);
        let o = &v["rounds"][1]["matches"][0]["outcome"];
        assert!(o["distances_m"][0].is_number() && o["decided_by"].is_string(), "{o}");
        // The winner went farther.
        let d = [o["distances_m"][0].as_f64().unwrap(), o["distances_m"][1].as_f64().unwrap()];
        let final_match = &b.rounds[1][0];
        assert_eq!(final_match.winner, final_match.players[if d[0] > d[1] { 0 } else { 1 }]);
        // The creature files as played, for watching the match again.
        let files = &v["rounds"][1]["matches"][0]["creature_files"];
        assert!(files[0].as_str().unwrap().contains("[[segment]]"), "{files}");
        let csv = std::fs::read_to_string(saved.with_extension("csv")).unwrap();
        assert_eq!(csv.lines().count(), 3, "{csv}");
        assert!(csv.lines().nth(2).unwrap().starts_with("\"Final\",1,"));
    }

    #[test]
    fn a_sumo_cup_on_hills_is_played_to_the_end() {
        let (cup, _) = play_cup(Event::Sumo, "arena-cup-sumo");
        let b = cup.bracket.as_ref().unwrap();
        assert!(b.champion().is_some());
        assert!(b.rules.terrain_roughness > 0.0);
        let json = std::fs::read_to_string(results(&cup)).unwrap();
        assert!(json.contains("\"sumo\"") && json.contains("\"margin\""), "{json}");
        // Watching a match again shows the same winner and changes nothing.
        let mut cup = cup;
        let (r, m) = (1, 0);
        let before = cup.bracket.as_ref().unwrap().rounds[r][m].winner;
        cup.start(r, m);
        assert!(cup.show.as_ref().unwrap().replay.is_some());
        cup.skip();
        let side = cup.show.as_ref().unwrap().winner.unwrap();
        assert_eq!(cup.bracket.as_ref().unwrap().rounds[r][m].players[side], before);
    }

    #[test]
    fn the_draw_is_fixed_once_a_match_is_played() {
        let (mut cup, _) = draw_cup(Event::Race, "arena-cup-fixed");
        assert!(!cup.draw_fixed(), "a new draw may be redrawn");
        play(&mut cup, 1);
        assert!(cup.draw_fixed(), "after the first match");
        play(&mut cup, 1);
        assert!(cup.bracket.as_ref().unwrap().champion().is_some());
        assert!(!cup.draw_fixed(), "the cup is over: a new cup may be drawn");
    }

    #[test]
    fn a_saved_cup_opens_again_and_its_matches_replay_the_same() {
        for (event, dir_name) in [(Event::Race, "arena-cup-reopen-race"), (Event::Sumo, "arena-cup-reopen-sumo")] {
            let (cup, dir) = play_cup(event, dir_name);
            // The students' files are gone after the cup: the saved files are used.
            for c in ["worm", "walker", "tailfin"] {
                std::fs::remove_file(dir.join(format!("{c}.toml"))).unwrap();
            }
            let mut again = Cup::new();
            again.sound.on = false;
            again.open(&results(&cup));
            let (a, b) = (cup.bracket.as_ref().unwrap(), again.bracket.as_ref().expect("opened"));
            assert_eq!((b.event, b.seed, b.champion()), (a.event, a.seed, a.champion()));
            assert_eq!(toml::to_string(&b.rules).unwrap(), toml::to_string(&a.rules).unwrap());
            assert!(!again.draw_fixed() && b.next().is_none());
            // Every played match, simulated again from the saved files, has the same winner.
            let played: Vec<(usize, usize, Option<usize>)> =
                b.rounds.iter().enumerate().flat_map(|(r, round)| round.iter().enumerate().filter(|(_, x)| x.outcome.is_some()).map(move |(m, x)| (r, m, x.winner))).collect();
            assert_eq!(played.len(), 2);
            for (r, m, winner) in played {
                again.start(r, m);
                assert!(again.show.is_some(), "{:?}", again.message);
                let show = again.show.as_mut().unwrap();
                while !show.done() {
                    show.step_all();
                }
                let side = decide(show, again.bracket.as_ref().unwrap().seed).0;
                assert_eq!(again.bracket.as_ref().unwrap().rounds[r][m].players[side], winner, "{event:?}, round {r} match {m}");
                again.back();
            }
        }
    }

    #[test]
    fn an_unfinished_cup_goes_on_where_it_stopped() {
        let (mut cup, _) = draw_cup(Event::Race, "arena-cup-resume");
        play(&mut cup, 1);
        let saved = results(&cup);
        // The program is closed and opened again.
        let mut again = Cup::new();
        again.sound.on = false;
        again.open(&saved);
        assert!(again.draw_fixed(), "{:?}", again.message);
        assert_eq!(again.bracket.as_ref().unwrap().next(), cup.bracket.as_ref().unwrap().next());
        play(&mut again, 1);
        // The final goes to the same file.
        assert_eq!(results(&again), saved);
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&saved).unwrap()).unwrap();
        let champion = again.bracket.as_ref().unwrap().champion().expect("a champion");
        assert_eq!(v["champion"], again.bracket.as_ref().unwrap().contestants[champion].name);
        assert!(again.message.is_none(), "{:?}", again.message);
    }

    #[test]
    fn a_file_that_is_not_a_saved_cup_is_refused() {
        let dir = std::env::temp_dir().join("arena-cup-not-a-cup");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("other.json");
        std::fs::write(&path, "{\"event\": \"race\"}").unwrap();
        let mut cup = Cup::new();
        cup.open(&path);
        assert!(cup.bracket.is_none());
        assert!(cup.message.as_ref().is_some_and(|(m, error)| *error && m.contains("not a saved cup")), "{:?}", cup.message);
    }
}
