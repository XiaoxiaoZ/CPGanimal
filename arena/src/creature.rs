//! Creature files: body plan + CPG "brain", plus the game rules they must obey.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// A creature as written in a `.toml` file.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Creature {
    pub name: String,
    #[serde(default)]
    pub author: String,
    /// Optional RGB colour for the UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[u8; 3]>,
    pub brain: Brain,
    /// Segment 0 is the torso. Every other segment hangs off an earlier one
    /// through a motorised joint driven by one CPG oscillator.
    #[serde(rename = "segment")]
    pub segments: Vec<Segment>,
    /// Written by the trainer, ignored by the simulator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Brain {
    /// Oscillation frequency shared by all joints (Hz).
    pub frequency: f64,
    /// How strongly neighbouring oscillators pull each other into step.
    #[serde(default = "default_coupling")]
    pub coupling: f64,
}

fn default_coupling() -> f64 {
    4.0
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Segment {
    /// Index of the segment this one is attached to (ignored for segment 0).
    pub parent: usize,
    /// Where on the parent: -1 = back end, 0 = middle, 1 = front end.
    pub attach: f64,
    /// Rest angle relative to the parent (deg).
    pub angle: f64,
    /// Size (m).
    pub length: f64,
    pub width: f64,
    /// Joint swing amplitude (deg).
    pub amplitude: f64,
    /// Shift of the swing centre (deg).
    pub offset: f64,
    /// Phase of this joint's oscillation (deg). Only differences matter.
    pub phase: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Meta {
    pub trained_for: String,
    pub fitness: f64,
    /// Genomes evaluated by the GA that produced this creature.
    pub evaluations: usize,
}

/// Game rules. Defaults are built in; a teacher can override any of them
/// with an `arena.toml` in the creatures folder.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Rules {
    pub max_segments: usize,
    pub length: [f64; 2],
    pub width: [f64; 2],
    /// Total body area budget (m²). Bigger bodies are heavier, not stronger.
    pub max_area: f64,
    pub density: f64,
    pub frequency: [f64; 2],
    pub coupling: [f64; 2],
    pub amplitude: [f64; 2],
    pub offset: [f64; 2],
    pub angle: [f64; 2],
    /// Joints may bend this far (deg) away from their rest angle.
    pub joint_range: f64,
    /// Same motor for every joint of every creature.
    pub motor_torque: f64,
    pub motor_stiffness: f64,
    pub motor_damping: f64,
    pub friction: f64,
    pub dt: f64,
    pub race_time: f64,
    pub sumo_time: f64,
    /// Width of the sumo platform (m).
    pub ring_width: f64,
    /// Start distance of each wrestler from the ring centre (m).
    pub sumo_start: f64,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            max_segments: 8,
            length: [0.1, 1.0],
            width: [0.05, 0.3],
            max_area: 0.6,
            density: 10.0,
            frequency: [0.2, 3.0],
            coupling: [0.5, 10.0],
            amplitude: [0.0, 60.0],
            offset: [-45.0, 45.0],
            angle: [-180.0, 180.0],
            joint_range: 110.0,
            motor_torque: 5.0,
            motor_stiffness: 40.0,
            motor_damping: 2.0,
            friction: 0.9,
            dt: 1.0 / 60.0,
            race_time: 15.0,
            sumo_time: 20.0,
            ring_width: 8.0,
            sumo_start: 1.8,
        }
    }
}

impl Rules {
    /// Load `arena.toml` from `dir` if present, otherwise defaults.
    pub fn load_dir(dir: &Path) -> Result<Self, String> {
        let p = dir.join("arena.toml");
        if p.exists() { Self::load_file(&p) } else { Ok(Self::default()) }
    }

    pub fn load_file(p: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
        toml::from_str(&text).map_err(|e| format!("{}: {e}", p.display()))
    }
}

impl Creature {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn joints(&self) -> usize {
        self.segments.len().saturating_sub(1)
    }

    pub fn area(&self) -> f64 {
        self.segments.iter().map(|s| s.length * s.width).sum()
    }

    /// Check the creature against the rules. Returns every problem found.
    pub fn validate(&self, rules: &Rules) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();
        let within = |v: f64, r: [f64; 2]| v.is_finite() && v >= r[0] - 1e-9 && v <= r[1] + 1e-9;
        if self.segments.is_empty() {
            errs.push("needs at least one [[segment]]".into());
        }
        if self.segments.len() > rules.max_segments {
            errs.push(format!("{} segments, max is {}", self.segments.len(), rules.max_segments));
        }
        if !within(self.brain.frequency, rules.frequency) {
            errs.push(format!("brain.frequency {} not in {:?}", self.brain.frequency, rules.frequency));
        }
        if !within(self.brain.coupling, rules.coupling) {
            errs.push(format!("brain.coupling {} not in {:?}", self.brain.coupling, rules.coupling));
        }
        for (i, s) in self.segments.iter().enumerate() {
            let mut check = |what: &str, v: f64, r: [f64; 2]| {
                if !within(v, r) {
                    errs.push(format!("segment {i}: {what} {v} not in {r:?}"));
                }
            };
            check("length", s.length, rules.length);
            check("width", s.width, rules.width);
            if i > 0 {
                check("attach", s.attach, [-1.0, 1.0]);
                check("angle", s.angle, rules.angle);
                check("amplitude", s.amplitude, rules.amplitude);
                check("offset", s.offset, rules.offset);
                check("phase", s.phase, [-360.0, 360.0]);
                if s.parent >= i {
                    errs.push(format!("segment {i}: parent {} must be an earlier segment", s.parent));
                }
            }
        }
        if self.area() > rules.max_area + 1e-9 {
            errs.push(format!("body area {:.3} m² exceeds budget {:.3} m²", self.area(), rules.max_area));
        }
        if errs.is_empty() { Ok(()) } else { Err(errs) }
    }
}

/// Which parts of a creature the genetic algorithm may change.
/// The body *topology* (number of segments, who attaches to whom) is always
/// the student's design; the GA tunes the numbers.
#[derive(Clone, Copy, Debug, Default)]
pub struct GenomeSpec {
    /// Also evolve segment sizes, attach points and rest angles.
    pub body: bool,
}

/// One gene: a named, bounded real value, stored in [0, 1].
#[derive(Clone, Debug)]
pub struct Gene {
    pub name: String,
    pub lo: f64,
    pub hi: f64,
}

impl GenomeSpec {
    pub fn genes(&self, c: &Creature, r: &Rules) -> Vec<Gene> {
        let g = |name: String, b: [f64; 2]| Gene { name, lo: b[0], hi: b[1] };
        let mut out = vec![
            g("brain.frequency".into(), r.frequency),
            g("brain.coupling".into(), r.coupling),
        ];
        for i in 0..c.segments.len() {
            if i > 0 {
                out.push(g(format!("segment[{i}].amplitude"), r.amplitude));
                out.push(g(format!("segment[{i}].offset"), r.offset));
                out.push(g(format!("segment[{i}].phase"), [-180.0, 180.0]));
            }
            if self.body {
                out.push(g(format!("segment[{i}].length"), r.length));
                out.push(g(format!("segment[{i}].width"), r.width));
                if i > 0 {
                    out.push(g(format!("segment[{i}].attach"), [-1.0, 1.0]));
                    out.push(g(format!("segment[{i}].angle"), r.angle));
                }
            }
        }
        out
    }

    /// Read the tunable values out of a creature, normalised to [0, 1].
    pub fn encode(&self, c: &Creature, r: &Rules) -> Vec<f64> {
        let genes = self.genes(c, r);
        self.values(c)
            .iter()
            .zip(&genes)
            .map(|(v, g)| ((v - g.lo) / (g.hi - g.lo)).clamp(0.0, 1.0))
            .collect()
    }

    /// Write normalised genes into a copy of `template`. The result always
    /// satisfies the rules (the body is shrunk to fit the area budget).
    pub fn decode(&self, template: &Creature, r: &Rules, genome: &[f64]) -> Creature {
        let genes = self.genes(template, r);
        assert_eq!(genes.len(), genome.len(), "genome length does not match template");
        let v: Vec<f64> = genes
            .iter()
            .zip(genome)
            .map(|(g, x)| g.lo + x.clamp(0.0, 1.0) * (g.hi - g.lo))
            .collect();
        let mut c = template.clone();
        let mut k = 0;
        let mut next = || {
            k += 1;
            v[k - 1]
        };
        c.brain.frequency = next();
        c.brain.coupling = next();
        for i in 0..c.segments.len() {
            let s = &mut c.segments[i];
            if i > 0 {
                s.amplitude = next();
                s.offset = next();
                s.phase = next();
            }
            if self.body {
                s.length = next();
                s.width = next();
                if i > 0 {
                    s.attach = next();
                    s.angle = next();
                }
            }
        }
        fit_area(&mut c, r);
        c.meta = None;
        c
    }

    fn values(&self, c: &Creature) -> Vec<f64> {
        let mut out = vec![c.brain.frequency, c.brain.coupling];
        for (i, s) in c.segments.iter().enumerate() {
            if i > 0 {
                // Phase is periodic: wrap into the gene range instead of clamping.
                let phase = (s.phase + 180.0).rem_euclid(360.0) - 180.0;
                out.extend([s.amplitude, s.offset, phase]);
            }
            if self.body {
                out.extend([s.length, s.width]);
                if i > 0 {
                    out.extend([s.attach, s.angle]);
                }
            }
        }
        out
    }
}

/// Shrink widths (then lengths) until the body fits the area budget.
fn fit_area(c: &mut Creature, r: &Rules) {
    for _ in 0..4 {
        let area = c.area();
        if area <= r.max_area {
            return;
        }
        let k = (r.max_area / area).sqrt();
        for s in &mut c.segments {
            s.width = (s.width * k).max(r.width[0]);
            s.length = (s.length * k).max(r.length[0]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worm() -> Creature {
        toml::from_str(
            r#"
            name = "worm"
            [brain]
            frequency = 1.0
            [[segment]]
            length = 0.5
            width = 0.1
            [[segment]]
            parent = 0
            attach = 1.0
            length = 0.5
            width = 0.1
            amplitude = 30
            "#,
        )
        .unwrap()
    }

    #[test]
    fn encode_decode_roundtrip() {
        let r = Rules::default();
        let c = worm();
        for body in [false, true] {
            let spec = GenomeSpec { body };
            let g = spec.encode(&c, &r);
            assert_eq!(g.len(), spec.genes(&c, &r).len());
            let d = spec.decode(&c, &r, &g);
            assert!((d.segments[1].amplitude - 30.0).abs() < 1e-9);
            assert!((d.brain.frequency - 1.0).abs() < 1e-9);
        }
    }

    #[test]
    fn decoded_creatures_are_always_legal() {
        let r = Rules::default();
        let spec = GenomeSpec { body: true };
        let c = worm();
        let n = spec.genes(&c, &r).len();
        for fill in [0.0, 0.5, 1.0] {
            let d = spec.decode(&c, &r, &vec![fill; n]);
            d.validate(&r).unwrap();
        }
    }
}
