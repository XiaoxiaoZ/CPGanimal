//! Rust version of the default GA (python/ga.py): a real-coded GA (tournament selection,
//! BLX-α crossover, Gaussian mutation, elitism) on top of the same
//! [`Problem`] interface students use. Also an example of the Rust API.
//!
//! cargo run --release --example reference_ga -- creatures/worm.toml [race|sumo] [budget] [out.toml]

use cpg_arena::creature::Level;
use cpg_arena::game::Mode;
use cpg_arena::problem::Problem;
use std::path::Path;

/// SplitMix64: tiny, fast, reproducible.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed ^ 0x9E37_79B9_7F4A_7C15)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    pub fn f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.f64() * n as f64) as usize % n.max(1)
    }

    /// Standard normal (Box–Muller).
    pub fn normal(&mut self) -> f64 {
        let u = self.f64().max(1e-300);
        let v = self.f64();
        (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
    }
}

#[derive(Clone, Debug)]
pub struct GaConfig {
    pub population: usize,
    /// Best individuals copied unchanged into the next generation.
    pub elites: usize,
    pub tournament: usize,
    pub crossover_rate: f64,
    /// BLX-α blend crossover spread.
    pub blend_alpha: f64,
    /// Per-gene mutation probability; `None` means 1/n.
    pub mutation_rate: Option<f64>,
    pub mutation_sigma: f64,
    pub seed: u64,
}

impl Default for GaConfig {
    fn default() -> Self {
        Self {
            population: 48,
            elites: 2,
            tournament: 3,
            crossover_rate: 0.9,
            blend_alpha: 0.3,
            mutation_rate: None,
            mutation_sigma: 0.1,
            seed: 1,
        }
    }
}

pub struct Ga {
    pub cfg: GaConfig,
    rng: Rng,
    dim: usize,
    pop: Vec<Vec<f64>>,
    /// Fitness of `pop` once told.
    fit: Vec<f64>,
    pub generation: usize,
    best: Option<(Vec<f64>, f64)>,
}

impl Ga {
    /// `seeds` are placed into the first generation as-is (e.g. the student's
    /// hand-made creature); the rest is uniform random.
    pub fn new(dim: usize, cfg: GaConfig, seeds: &[Vec<f64>]) -> Self {
        let mut rng = Rng::new(cfg.seed);
        let mut pop: Vec<Vec<f64>> = seeds.iter().take(cfg.population).cloned().collect();
        while pop.len() < cfg.population {
            pop.push((0..dim).map(|_| rng.f64()).collect());
        }
        Self { cfg, rng, dim, pop, fit: Vec::new(), generation: 0, best: None }
    }

    /// Genomes to evaluate this generation.
    pub fn ask(&self) -> &[Vec<f64>] {
        &self.pop
    }

    /// Report fitness for the genomes returned by `ask`, then breed.
    pub fn tell(&mut self, fitness: Vec<f64>) {
        assert_eq!(fitness.len(), self.pop.len());
        // NaN / -inf never win.
        self.fit = fitness.into_iter().map(|f| if f.is_nan() { f64::NEG_INFINITY } else { f }).collect();
        let mut order: Vec<usize> = (0..self.pop.len()).collect();
        order.sort_by(|&a, &b| self.fit[b].total_cmp(&self.fit[a]));
        let top = order[0];
        if self.best.as_ref().is_none_or(|(_, f)| self.fit[top] > *f) {
            self.best = Some((self.pop[top].clone(), self.fit[top]));
        }

        let mut next: Vec<Vec<f64>> = order.iter().take(self.cfg.elites).map(|&i| self.pop[i].clone()).collect();
        let pm = self.cfg.mutation_rate.unwrap_or(1.0 / self.dim.max(1) as f64);
        while next.len() < self.cfg.population {
            let a = self.tournament();
            let b = self.tournament();
            let mut child = if self.rng.f64() < self.cfg.crossover_rate {
                self.blend(&self.pop[a].clone(), &self.pop[b].clone())
            } else {
                self.pop[a].clone()
            };
            for g in &mut child {
                if self.rng.f64() < pm {
                    *g += self.cfg.mutation_sigma * self.rng.normal();
                }
                *g = g.clamp(0.0, 1.0);
            }
            next.push(child);
        }
        self.pop = next;
        self.generation += 1;
    }

    pub fn best(&self) -> Option<(&[f64], f64)> {
        self.best.as_ref().map(|(g, f)| (g.as_slice(), *f))
    }

    fn tournament(&mut self) -> usize {
        let mut win = self.rng.below(self.pop.len());
        for _ in 1..self.cfg.tournament {
            let c = self.rng.below(self.pop.len());
            if self.fit[c] > self.fit[win] {
                win = c;
            }
        }
        win
    }

    fn blend(&mut self, a: &[f64], b: &[f64]) -> Vec<f64> {
        let al = self.cfg.blend_alpha;
        a.iter()
            .zip(b)
            .map(|(&x, &y)| {
                let (lo, hi) = (x.min(y), x.max(y));
                let d = hi - lo;
                lo - al * d + self.rng.f64() * (1.0 + 2.0 * al) * d
            })
            .collect()
    }
}


fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let template = args.first().map_or("creatures/worm.toml", String::as_str);
    let mode = if args.get(1).is_some_and(|m| m == "sumo") { Mode::Sumo } else { Mode::Race };
    let budget: usize = args.get(2).and_then(|b| b.parse().ok()).unwrap_or(1000);
    let opponents = (mode == Mode::Sumo).then(|| Path::new(template).parent().unwrap_or(Path::new(".")).to_path_buf());
    let p = Problem::load(Path::new(template), mode, Level::Brain, opponents.as_deref(), None).expect("problem");
    let cfg = GaConfig { population: 50, ..Default::default() };
    let mut ga = Ga::new(p.dim(), cfg, &[p.start()]);
    while p.evaluations() < budget {
        let fit = p.evaluate_batch(ga.ask()).expect("valid genomes");
        ga.tell(fit);
        println!("evaluations {:5}  best {:8.3}", p.evaluations(), ga.best().map_or(f64::NAN, |b| b.1));
    }
    if let (Some(out), Some((g, _))) = (args.get(3), ga.best()) {
        let f = p.save(g, Path::new(out), Some("Reference GA")).expect("save");
        println!("fitness {f:.3} -> {out}");
    }
}
