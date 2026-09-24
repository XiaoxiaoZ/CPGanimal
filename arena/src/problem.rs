//! The optimisation problem students solve with their own GA.
//!
//! A [`Problem`] is a black box: a genome is `dim()` numbers in [0, 1], and
//! `evaluate` returns one fitness per genome (higher is better). What the
//! numbers mean (`genes()`) is there for curiosity and debugging; a GA does
//! not need it.
//!
//! Two extras:
//! - [`Environments`]: evaluate each genome on several terrains / frictions
//!   and combine the scores, so evolution generalises instead of overfitting.
//! - [`Problem::fight`]: sumo duels between two genomes, for co-evolution.
//!
//! The same interface is exposed on the command line (`arena info`,
//! `arena batch`, `arena fight`, `arena save`) so a GA can be written in any language.

use crate::creature::{Creature, GenomeSpec, Level, Meta, Rules};
use crate::game::{self, Environments, Mode};
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
    pub env: Environments,
    variants: Vec<Rules>,
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
    pub level: Level,
    pub dim: usize,
    pub opponents: Vec<String>,
    pub environments: Environments,
    pub genes: Vec<GeneInfo>,
}

impl Problem {
    /// `opponents` only matters for sumo; with none, the creature fights the Rock.
    /// Opponents with the same name as the template are skipped (no fighting yourself).
    pub fn new(template: Creature, rules: Rules, mode: Mode, level: Level, opponents: Vec<Creature>) -> Result<Self, String> {
        template
            .validate(&rules)
            .map_err(|e| format!("{} breaks the rules:\n  {}", template.name, e.join("\n  ")))?;
        let opponents = opponents.into_iter().filter(|o| o.name != template.name).collect();
        let env = Environments::default();
        let variants = env.variants(&rules);
        Ok(Self { template, rules, mode, spec: GenomeSpec { level }, opponents, env, variants, evaluations: AtomicUsize::new(0) })
    }

    /// Evaluate on these environments instead of the rules' single one.
    pub fn with_environments(mut self, env: Environments) -> Self {
        self.variants = env.variants(&self.rules);
        self.env = env;
        self
    }

    /// Load the template from a file, the rules from `rules` (default:
    /// `arena.toml` next to the template, else built-in), and sumo opponents
    /// from every valid creature in `opponents_dir`.
    pub fn load(template: &Path, mode: Mode, level: Level, opponents_dir: Option<&Path>, rules: Option<&Path>) -> Result<Self, String> {
        let rules = match rules {
            Some(p) => Rules::load_file(p)?,
            None => Rules::load_dir(template.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new(".")))?,
        };
        let c = Creature::load(template)?;
        let ops = opponents_dir
            .map(|d| game::load_folder(d, &rules).into_iter().filter_map(|e| e.creature.ok()).collect())
            .unwrap_or_default();
        Self::new(c, rules, mode, level, ops)
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
            level: self.spec.level,
            dim: self.dim(),
            opponents: self.opponents.iter().map(|o| o.name.clone()).collect(),
            environments: self.env.clone(),
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
        Ok(self.score(&c))
    }

    fn score(&self, c: &Creature) -> f64 {
        game::fitness_in(c, self.mode, self.env.aggregate, &self.variants, &self.opponents)
    }

    /// Sumo duels for co-evolution: for each pair (a, b), the score of a
    /// against b (+1 win / 0 draw / −1 loss, plus ring control). The score is
    /// antisymmetric, so b's score is the negative: one bout rates both.
    /// Each pair counts as one evaluation.
    pub fn fight(&self, pairs: &[(Vec<f64>, Vec<f64>)]) -> Result<Vec<f64>, String> {
        let decoded: Vec<(Creature, Creature)> = pairs
            .iter()
            .enumerate()
            .map(|(k, (a, b))| {
                let a = self.decode(a).map_err(|e| format!("pair {k}, first: {e}"))?;
                let b = self.decode(b).map_err(|e| format!("pair {k}, second: {e}"))?;
                Ok((a, b))
            })
            .collect::<Result<_, String>>()?;
        self.evaluations.fetch_add(pairs.len(), Ordering::Relaxed);
        Ok(decoded.par_iter().map(|(a, b)| game::duel(a, b, self.env.aggregate, &self.variants)).collect())
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
        let fitness = self.score(&c);
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
        Problem::new(c, Rules::default(), Mode::Race, Level::Brain, vec![]).unwrap()
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
    fn environments_vary_terrain_and_one_trial_is_the_rules() {
        let c: Creature = toml::from_str(include_str!("../creatures/worm.toml")).unwrap();
        let rules = Rules { terrain_roughness: 0.3, ..Rules::default() };
        let one = Problem::new(c.clone(), rules.clone(), Mode::Race, Level::Brain, vec![]).unwrap();
        let direct = game::fitness(&c, Mode::Race, &rules, &[]);
        assert_eq!(one.evaluate(&one.start()).unwrap(), direct);
        let env = Environments { trials: 3, first_seed: Some(100), ..Default::default() };
        let many = Problem::new(c, rules, Mode::Race, Level::Brain, vec![]).unwrap().with_environments(env);
        let seeds: Vec<u64> = many.variants.iter().map(|r| r.terrain_seed).collect();
        assert_eq!(seeds, vec![100, 101, 102]);
        assert!(many.evaluate(&many.start()).unwrap().is_finite());
    }

    #[test]
    fn fight_is_roughly_antisymmetric() {
        let c: Creature = toml::from_str(include_str!("../creatures/tailfin.toml")).unwrap();
        let p = Problem::new(c, Rules::default(), Mode::Sumo, Level::Brain, vec![]).unwrap();
        let a = p.start();
        let b = vec![0.8; p.dim()];
        let s = p.fight(&[(a.clone(), b.clone()), (b, a)]).unwrap();
        assert!(s[0] * s[1] <= 0.0, "one side should not beat the other from both sides: {s:?}");
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
