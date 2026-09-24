//! Command-line tool: check, evaluate, train, race, and run tournaments.

use clap::{Parser, Subcommand};
use cpg_arena::creature::{Creature, GenomeSpec, Rules};
use cpg_arena::ga::GaConfig;
use cpg_arena::game::{self, Mode, TrainOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "arena", about = "Evolve CPG creatures with a genetic algorithm")]
struct Cli {
    /// Rules file (defaults to <folder>/arena.toml if present, else built-in rules).
    #[arg(long, global = true)]
    rules: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Validate every creature in a folder.
    Check { folder: PathBuf },
    /// List the genes of a creature (what the GA can change).
    Genes {
        creature: PathBuf,
        /// Include body genes (sizes, attach points, angles).
        #[arg(long)]
        body: bool,
    },
    /// Print the fitness of a creature as JSON. With --genes, the genes are
    /// first written into the creature (for your own GA in any language).
    Eval {
        creature: PathBuf,
        #[arg(long, value_enum, default_value = "race")]
        mode: Mode,
        /// Folder of sumo opponents (default: the Rock).
        #[arg(long)]
        opponents: Option<PathBuf>,
        /// Comma-separated genes in [0,1], see `arena genes`.
        #[arg(long, value_delimiter = ',')]
        genes: Option<Vec<f64>>,
        #[arg(long)]
        body: bool,
        /// Write the decoded creature here.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Rename the creature written with --out.
        #[arg(long)]
        name: Option<String>,
    },
    /// Evolve a creature with the built-in genetic algorithm.
    Train {
        /// Starting creature; its body plan (topology) is kept.
        template: PathBuf,
        #[arg(long, value_enum, default_value = "race")]
        mode: Mode,
        /// Where to write the champion.
        #[arg(long)]
        out: PathBuf,
        /// Name of the champion (default: "<template name> (<mode>)").
        #[arg(long)]
        name: Option<String>,
        /// Also evolve the body (sizes, attach points, angles).
        #[arg(long)]
        body: bool,
        #[arg(long, default_value_t = 30)]
        generations: usize,
        #[arg(long, default_value_t = 48)]
        population: usize,
        #[arg(long, default_value_t = 0.1)]
        sigma: f64,
        #[arg(long, default_value_t = 1)]
        seed: u64,
        /// Folder of sumo sparring partners (default: the Rock).
        #[arg(long)]
        opponents: Option<PathBuf>,
    },
    /// Race every creature in a folder.
    Race { folder: PathBuf },
    /// Sumo round robin between every creature in a folder.
    Tournament { folder: PathBuf },
}

fn rules_for(cli: &Option<PathBuf>, dir: &Path) -> Rules {
    let r = match cli {
        Some(p) => std::fs::read_to_string(p)
            .map_err(|e| e.to_string())
            .and_then(|t| toml::from_str(&t).map_err(|e| e.to_string())),
        None => Rules::load_dir(dir),
    };
    r.unwrap_or_else(|e| die(&e))
}

fn die(msg: &str) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1)
}

fn parent(p: &Path) -> PathBuf {
    p.parent().map(Path::to_path_buf).unwrap_or_else(|| ".".into())
}

fn load_valid(p: &Path, rules: &Rules) -> Creature {
    let c = Creature::load(p).unwrap_or_else(|e| die(&e));
    if let Err(errs) = c.validate(rules) {
        die(&format!("{} breaks the rules:\n  {}", p.display(), errs.join("\n  ")));
    }
    c
}

fn valid_creatures(dir: &Path, rules: &Rules) -> Vec<Creature> {
    game::load_folder(dir, rules).into_iter().filter_map(|e| e.creature.ok()).collect()
}

fn main() {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Check { folder } => {
            let rules = rules_for(&cli.rules, &folder);
            let mut bad = 0;
            for e in game::load_folder(&folder, &rules) {
                match &e.creature {
                    Ok(c) => println!("ok    {:<24} {} segments, area {:.3} m²", c.name, c.segments.len(), c.area()),
                    Err(errs) => {
                        bad += 1;
                        println!("FAIL  {}", e.path.display());
                        for m in errs {
                            println!("        {m}");
                        }
                    }
                }
            }
            if bad > 0 {
                std::process::exit(1);
            }
        }
        Cmd::Genes { creature, body } => {
            let rules = rules_for(&cli.rules, &parent(&creature));
            let c = load_valid(&creature, &rules);
            let spec = GenomeSpec { body };
            let values = spec.encode(&c, &rules);
            for (g, v) in spec.genes(&c, &rules).iter().zip(values) {
                println!("{:<24} [{:>7.2}, {:>7.2}]  now {:.4}", g.name, g.lo, g.hi, v);
            }
        }
        Cmd::Eval { creature, mode, opponents, genes, body, out, name } => {
            let rules = rules_for(&cli.rules, &parent(&creature));
            let mut c = load_valid(&creature, &rules);
            if let Some(g) = genes {
                let spec = GenomeSpec { body };
                let n = spec.genes(&c, &rules).len();
                if g.len() != n {
                    die(&format!("expected {n} genes, got {}", g.len()));
                }
                c = spec.decode(&c, &rules, &g);
            }
            let ops = opponents.map(|d| valid_creatures(&d, &rules)).unwrap_or_default();
            let f = game::fitness(&c, mode, &rules, &ops);
            if let Some(p) = out {
                if let Some(n) = name {
                    c.name = n;
                }
                c.save(&p).unwrap_or_else(|e| die(&e));
            }
            println!("{}", serde_json::json!({ "name": c.name, "mode": mode.name(), "fitness": f }));
        }
        Cmd::Train { template, mode, out, name, body, generations, population, sigma, seed, opponents } => {
            let rules = rules_for(&cli.rules, &parent(&template));
            let c = load_valid(&template, &rules);
            let ops: Vec<Creature> = opponents
                .map(|d| valid_creatures(&d, &rules))
                .unwrap_or_default()
                .into_iter()
                .filter(|o| o.name != c.name)
                .collect();
            let opts = TrainOptions {
                mode,
                generations,
                spec: GenomeSpec { body },
                ga: GaConfig { population, mutation_sigma: sigma, seed, ..Default::default() },
            };
            let dim = opts.spec.genes(&c, &rules).len();
            println!("training '{}' for {} ({dim} genes, population {population})", c.name, mode.name());
            let hist_path = out.with_extension("history.csv");
            let mut hist = std::fs::File::create(&hist_path).unwrap_or_else(|e| die(&e.to_string()));
            writeln!(hist, "generation,best,mean,best_ever,evaluations").ok();
            let t0 = std::time::Instant::now();
            let (best, f) = game::train(&c, &rules, &ops, &opts, |s| {
                println!("gen {:>3}  best {:>8.3}  mean {:>8.3}  best ever {:>8.3}", s.generation, s.best, s.mean, s.best_ever);
                writeln!(hist, "{},{},{},{},{}", s.generation, s.best, s.mean, s.best_ever, s.evaluations).ok();
            });
            let mut best = best;
            best.name = name.unwrap_or_else(|| format!("{} ({})", c.name, mode.name()));
            best.save(&out).unwrap_or_else(|e| die(&e));
            println!("champion fitness {f:.3} -> {} ({:.1}s)", out.display(), t0.elapsed().as_secs_f64());
        }
        Cmd::Race { folder } => {
            let rules = rules_for(&cli.rules, &folder);
            let cs = valid_creatures(&folder, &rules);
            for (rank, (i, d)) in game::race_all(&cs, &rules).into_iter().enumerate() {
                println!("{:>2}. {:<24} {:>7.2} m", rank + 1, cs[i].name, d);
            }
        }
        Cmd::Tournament { folder } => {
            let rules = rules_for(&cli.rules, &folder);
            let cs = valid_creatures(&folder, &rules);
            let (table, _) = game::tournament(&cs, &rules);
            println!("    {:<24} {:>3} {:>3} {:>3} {:>4}", "", "W", "D", "L", "Pts");
            for (rank, s) in table.iter().enumerate() {
                println!("{:>2}. {:<24} {:>3} {:>3} {:>3} {:>4}", rank + 1, cs[s.index].name, s.wins, s.draws, s.losses, s.points);
            }
        }
    }
}
