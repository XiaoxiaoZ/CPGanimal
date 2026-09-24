//! The optimisation problem students solve with their own GA.
//!
//! A [`Problem`] is a black box: a genome is `dim()` numbers in [0, 1], and
//! `evaluate` returns one fitness per genome (higher is better). What the
//! numbers mean (`genes()`) is there for curiosity and debugging; a GA does
//! not need it.
//!
//! The same interface is exposed on the command line (`arena info`,
//! `arena batch`, `arena save`) so a GA can be written in any language.

use crate::creature::{Creature, GenomeSpec, Meta, Rules};
use crate::game::{self, Mode};
use rayon::prelude::*;
use serde::Serialize;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct Problem {
    pub template: Creature,
    pub rules: Rules,
    pub mode: Mode,
    pub spec: GenomeSpec,
    pub opponents: Vec<Creature>,
    evaluations: AtomicUsize,
}

/// Description of one gene, for `arena info`.
#[derive(Clone, Debug, Serialize)]
pub struct GeneInfo {
    pub name: String,
    /// Physical range the gene is mapped to (0 → lo, 1 → hi).
    pub lo: f64,
    pub hi: f64,
    /// Value in the template creature, normalised to [0, 1].
    pub start: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Info {
    pub creature: String,
    pub mode: &'static str,
    pub body: bool,
    pub dim: usize,
    pub opponents: Vec<String>,
    pub genes: Vec<GeneInfo>,
}

impl Problem {
    /// `opponents` only matters for sumo; with none, the creature fights the Rock.
    /// Opponents with the same name as the template are skipped (no fighting yourself).
    pub fn new(template: Creature, rules: Rules, mode: Mode, body: bool, opponents: Vec<Creature>) -> Result<Self, String> {
        template
            .validate(&rules)
            .map_err(|e| format!("{} breaks the rules:\n  {}", template.name, e.join("\n  ")))?;
        let opponents = opponents.into_iter().filter(|o| o.name != template.name).collect();
        Ok(Self { template, rules, mode, spec: GenomeSpec { body }, opponents, evaluations: AtomicUsize::new(0) })
    }

    /// Load the template from a file, the rules from `rules` (default:
    /// `arena.toml` next to the template, else built-in), and sumo opponents
    /// from every valid creature in `opponents_dir`.
    pub fn load(template: &Path, mode: Mode, body: bool, opponents_dir: Option<&Path>, rules: Option<&Path>) -> Result<Self, String> {
        let rules = match rules {
            Some(p) => Rules::load_file(p)?,
            None => Rules::load_dir(template.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new(".")))?,
        };
        let c = Creature::load(template)?;
        let ops = opponents_dir
            .map(|d| game::load_folder(d, &rules).into_iter().filter_map(|e| e.creature.ok()).collect())
            .unwrap_or_default();
        Self::new(c, rules, mode, body, ops)
    }

    /// Number of genes.
    pub fn dim(&self) -> usize {
        self.spec.genes(&self.template, &self.rules).len()
    }

    /// The template creature as a genome: a sensible starting point.
    pub fn start(&self) -> Vec<f64> {
        self.spec.encode(&self.template, &self.rules)
    }

    pub fn info(&self) -> Info {
        let start = self.start();
        let genes = self
            .spec
            .genes(&self.template, &self.rules)
            .into_iter()
            .zip(start)
            .map(|(g, start)| GeneInfo { name: g.name, lo: g.lo, hi: g.hi, start })
            .collect();
        Info {
            creature: self.template.name.clone(),
            mode: self.mode.name(),
            body: self.spec.body,
            dim: self.dim(),
            opponents: self.opponents.iter().map(|o| o.name.clone()).collect(),
            genes,
        }
    }

    fn check(&self, genome: &[f64]) -> Result<(), String> {
        if genome.len() != self.dim() {
            return Err(format!("genome has {} genes, expected {}", genome.len(), self.dim()));
        }
        if let Some(i) = genome.iter().position(|g| !g.is_finite()) {
            return Err(format!("gene {i} is {}", genome[i]));
        }
        Ok(())
    }

    /// Genome → creature. Genes outside [0, 1] are clamped.
    pub fn decode(&self, genome: &[f64]) -> Result<Creature, String> {
        self.check(genome)?;
        Ok(self.spec.decode(&self.template, &self.rules, genome))
    }

    /// Fitness of one genome (higher is better).
    pub fn evaluate(&self, genome: &[f64]) -> Result<f64, String> {
        let c = self.decode(genome)?;
        self.evaluations.fetch_add(1, Ordering::Relaxed);
        Ok(game::fitness(&c, self.mode, &self.rules, &self.opponents))
    }

    /// Fitness of a whole population, evaluated in parallel.
    pub fn evaluate_batch(&self, genomes: &[Vec<f64>]) -> Result<Vec<f64>, String> {
        for (k, g) in genomes.iter().enumerate() {
            self.check(g).map_err(|e| format!("genome {k}: {e}"))?;
        }
        genomes.par_iter().map(|g| self.evaluate(g)).collect()
    }

    /// How many genomes this problem has evaluated so far.
    pub fn evaluations(&self) -> usize {
        self.evaluations.load(Ordering::Relaxed)
    }

    /// Write the creature for `genome` to `path`, ready for the arena.
    pub fn save(&self, genome: &[f64], path: &Path, name: Option<&str>) -> Result<f64, String> {
        let mut c = self.decode(genome)?;
        let fitness = game::fitness(&c, self.mode, &self.rules, &self.opponents);
        if let Some(n) = name {
            c.name = n.to_string();
        }
        c.meta = Some(Meta { trained_for: self.mode.name().into(), fitness, evaluations: self.evaluations() });
        c.save(path)?;
        Ok(fitness)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worm() -> Problem {
        let c: Creature = toml::from_str(include_str!("../creatures/worm.toml")).unwrap();
        Problem::new(c, Rules::default(), Mode::Race, false, vec![]).unwrap()
    }

    #[test]
    fn start_genome_reproduces_template() {
        let p = worm();
        let direct = game::fitness(&p.template, Mode::Race, &p.rules, &[]);
        assert_eq!(p.evaluate(&p.start()).unwrap(), direct);
    }

    #[test]
    fn batch_matches_single_and_is_deterministic() {
        let p = worm();
        let pop: Vec<Vec<f64>> = (0..6).map(|k| vec![k as f64 / 6.0; p.dim()]).collect();
        let a = p.evaluate_batch(&pop).unwrap();
        let b: Vec<f64> = pop.iter().map(|g| p.evaluate(g).unwrap()).collect();
        assert_eq!(a, b);
        assert_eq!(p.evaluations(), 12);
    }

    #[test]
    fn bad_genomes_are_rejected() {
        let p = worm();
        assert!(p.evaluate(&[0.5]).is_err());
        let mut g = p.start();
        g[0] = f64::NAN;
        assert!(p.evaluate(&g).is_err());
    }
}
