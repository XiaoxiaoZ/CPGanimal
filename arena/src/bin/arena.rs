//! Command-line tool. The optimisation interface for your own GA:
//!
//! ```text
//! arena info  worm.toml                 how many genes, what they mean
//! arena batch worm.toml < pop.txt       one genome per line in, one fitness per line out
//! arena eval  worm.toml --genes 0.1,…   fitness of one genome
//! arena save  worm.toml --genes 0.1,… --out me.toml --name "Me"
//! arena fight tailfin.toml < pairs.txt  "genesA | genesB" per line in, score of A per line out
//! ```
//!
//! Plus `check`, `race` and `tournament` for folders of creatures.

use clap::{Args, Parser, Subcommand};
use cpg_arena::creature::{Creature, Level, Rules};
use cpg_arena::game::{self, Aggregate, Environments, Mode};
use cpg_arena::problem::Problem;
use std::io::{BufRead, BufWriter, Write};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "arena", about = "CPG Arena: evaluate genomes for your own genetic algorithm, race and fight creatures")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

/// What is being optimised. Same flags for info / eval / batch / save.
#[derive(Args)]
struct ProblemArgs {
    /// Template creature: the starting point (and, below `structure`, the fixed body plan).
    template: PathBuf,
    #[arg(long, value_enum, default_value = "race")]
    mode: Mode,
    /// What the genes control: brain (CPG only), body (+ segment sizes and
    /// angles), structure (+ number of segments and who attaches to whom).
    #[arg(long, value_enum, default_value = "brain")]
    level: Level,
    /// Sumo only: fight every creature in this folder (default: the Rock).
    #[arg(long)]
    opponents: Option<PathBuf>,
    /// Rules file (default: arena.toml next to the template, else built-in rules).
    #[arg(long)]
    rules: Option<PathBuf>,
    #[command(flatten)]
    env: EnvArgs,
}

impl ProblemArgs {
    fn problem(&self) -> Problem {
        Problem::load(&self.template, self.mode, self.level, self.opponents.as_deref(), self.rules.as_deref())
            .unwrap_or_else(|e| die(&e))
            .with_environments(self.env.environments())
    }
}

/// Evaluate on several environments (terrain seeds / frictions) and combine.
#[derive(Args)]
struct EnvArgs {
    /// Number of environments per evaluation (1 = exactly the rules).
    #[arg(long, default_value_t = 1)]
    trials: usize,
    /// Terrain seed of the first environment; trial k uses seed + k
    /// (default: terrain_seed from the rules).
    #[arg(long)]
    env_seed: Option<u64>,
    /// Scale friction by a random factor in [1-j, 1+j] per environment.
    #[arg(long, default_value_t = 0.0)]
    friction_jitter: f64,
    /// Combine per-environment scores by mean or worst case.
    #[arg(long, value_enum, default_value = "mean")]
    aggregate: Aggregate,
}

impl EnvArgs {
    fn environments(&self) -> Environments {
        Environments { trials: self.trials, first_seed: self.env_seed, friction_jitter: self.friction_jitter, aggregate: self.aggregate }
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// Describe the genome: number of genes, names, ranges, template values.
    Info {
        #[command(flatten)]
        problem: ProblemArgs,
        /// Machine-readable output.
        #[arg(long)]
        json: bool,
    },
    /// Fitness of one genome (higher is better).
    Eval {
        #[command(flatten)]
        problem: ProblemArgs,
        /// Genes in [0,1], comma-separated. Default: the template itself.
        #[arg(long, value_delimiter = ',', allow_negative_numbers = true)]
        genes: Option<Vec<f64>>,
    },
    /// Fitness of a whole population, evaluated in parallel.
    /// Reads one genome per line from stdin (numbers separated by commas or
    /// spaces), writes one fitness per line to stdout, in the same order.
    Batch {
        #[command(flatten)]
        problem: ProblemArgs,
    },
    /// Sumo duels between genomes, for co-evolution. Reads one pair per line
    /// from stdin, "genesA | genesB"; writes the score of A against B per line
    /// (+1 win / 0 draw / -1 loss, plus ring control; B's score is the negative).
    Fight {
        #[command(flatten)]
        problem: ProblemArgs,
    },
    /// Turn a genome into a creature file for the arena.
    Save {
        #[command(flatten)]
        problem: ProblemArgs,
        #[arg(long, value_delimiter = ',', allow_negative_numbers = true)]
        genes: Vec<f64>,
        #[arg(long)]
        out: PathBuf,
        /// Name shown in the arena (default: the template's name).
        #[arg(long)]
        name: Option<String>,
    },
    /// Fitness of whole creatures instead of genomes, for evolving structure
    /// with your own operators. Reads one creature per line from stdin as JSON
    /// (same fields as the .toml files), writes one fitness per line; a
    /// creature that breaks the rules scores -inf and the reason goes to stderr.
    Judge {
        #[arg(long, value_enum, default_value = "race")]
        mode: Mode,
        /// Sumo duels instead: each line is a JSON array [creatureA, creatureB]
        /// and the output is A's score against B.
        #[arg(long)]
        pairs: bool,
        #[command(flatten)]
        env: EnvArgs,
        /// Sumo only: fight every creature in this folder (default: the Rock).
        #[arg(long)]
        opponents: Option<PathBuf>,
        /// Rules file (default: built-in rules).
        #[arg(long)]
        rules: Option<PathBuf>,
    },
    /// Print the rules in effect as JSON (limits for segments, sizes, angles, …).
    Rules {
        /// Rules file (default: built-in rules).
        rules: Option<PathBuf>,
    },
    /// Convert a creature between TOML and JSON. Use - for stdin/stdout (JSON).
    Convert { input: String, output: String },
    /// Validate every creature in a folder.
    Check { folder: PathBuf },
    /// Race every creature in a folder.
    Race { folder: PathBuf },
    /// Sumo round robin between every creature in a folder.
    Tournament { folder: PathBuf },
}

fn die(msg: &str) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1)
}

fn load_rules(file: Option<&Path>) -> Rules {
    file.map_or_else(|| Ok(Rules::default()), Rules::load_file).unwrap_or_else(|e| die(&e))
}

fn folder_rules(dir: &Path) -> Rules {
    Rules::load_dir(dir).unwrap_or_else(|e| die(&e))
}

fn parse_genome(line: &str) -> Result<Vec<f64>, String> {
    line.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|t| !t.is_empty())
        .map(|t| t.parse::<f64>().map_err(|e| format!("'{t}': {e}")))
        .collect()
}

fn main() {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Info { problem, json } => {
            let p = problem.problem();
            let info = p.info();
            if json {
                println!("{}", serde_json::to_string_pretty(&info).expect("serialisable"));
                return;
            }
            println!("creature  {}\nmode      {}\nlevel     {}\ndim       {}", info.creature, info.mode, info.level.name(), info.dim);
            let e = &info.environments;
            if e.trials > 1 || e.friction_jitter > 0.0 {
                let seed = e.first_seed.unwrap_or(p.rules.terrain_seed);
                println!("envs      {} trials, terrain seeds {}..{}, friction ±{:.0}%, {:?}", e.trials, seed, seed + e.trials as u64 - 1, e.friction_jitter * 100.0, e.aggregate);
            }
            if p.mode == Mode::Sumo {
                let ops = if info.opponents.is_empty() { "Rock".to_string() } else { info.opponents.join(", ") };
                println!("opponents {ops}");
            }
            println!("\n  #  {:<22} {:>16}  {:>6}", "gene", "0 → 1 maps to", "start");
            for (i, g) in info.genes.iter().enumerate() {
                println!("{i:>3}  {:<22} [{:>6.2}, {:>6.2}]  {:>6.3}", g.name, g.lo, g.hi, g.start);
            }
        }
        Cmd::Eval { problem, genes } => {
            let p = problem.problem();
            let g = genes.unwrap_or_else(|| p.start());
            println!("{}", p.evaluate(&g).unwrap_or_else(|e| die(&e)));
        }
        Cmd::Batch { problem } => {
            let p = problem.problem();
            let mut pop = Vec::new();
            for (n, line) in std::io::stdin().lock().lines().enumerate() {
                let line = line.unwrap_or_else(|e| die(&e.to_string()));
                if line.trim().is_empty() || line.trim_start().starts_with('#') {
                    continue;
                }
                pop.push(parse_genome(&line).unwrap_or_else(|e| die(&format!("line {}: {e}", n + 1))));
            }
            let fit = p.evaluate_batch(&pop).unwrap_or_else(|e| die(&e));
            let mut out = BufWriter::new(std::io::stdout().lock());
            for f in fit {
                writeln!(out, "{f}").ok();
            }
        }
        Cmd::Fight { problem } => {
            let p = problem.problem();
            let mut pairs = Vec::new();
            for (n, line) in std::io::stdin().lock().lines().enumerate() {
                let line = line.unwrap_or_else(|e| die(&e.to_string()));
                if line.trim().is_empty() || line.trim_start().starts_with('#') {
                    continue;
                }
                let (a, b) = line.split_once('|').unwrap_or_else(|| die(&format!("line {}: expected \"genesA | genesB\"", n + 1)));
                let parse = |t: &str| parse_genome(t).unwrap_or_else(|e| die(&format!("line {}: {e}", n + 1)));
                pairs.push((parse(a), parse(b)));
            }
            let scores = p.fight(&pairs).unwrap_or_else(|e| die(&e));
            let mut out = BufWriter::new(std::io::stdout().lock());
            for f in scores {
                writeln!(out, "{f}").ok();
            }
        }
        Cmd::Save { problem, genes, out, name } => {
            let p = problem.problem();
            let f = p.save(&genes, &out, name.as_deref()).unwrap_or_else(|e| die(&e));
            println!("fitness {f} -> {}", out.display());
        }
        Cmd::Judge { mode, pairs, env, opponents, rules } => {
            let rules = load_rules(rules.as_deref());
            let env = env.environments();
            // Parse every line; unparsable lines score -inf.
            let mut parsed: Vec<Result<Vec<Creature>, String>> = Vec::new();
            for line in std::io::stdin().lock().lines() {
                let line = line.unwrap_or_else(|e| die(&e.to_string()));
                if line.trim().is_empty() {
                    continue;
                }
                let item = if pairs {
                    serde_json::from_str::<[Creature; 2]>(&line).map(Vec::from)
                } else {
                    serde_json::from_str::<Creature>(&line).map(|c| vec![c])
                };
                parsed.push(item.map_err(|e| e.to_string()));
            }
            let ok: Vec<Vec<Creature>> = parsed.iter().filter_map(|p| p.as_ref().ok().cloned()).collect();
            let results = if pairs {
                let ps: Vec<(Creature, Creature)> = ok.into_iter().map(|v| (v[0].clone(), v[1].clone())).collect();
                game::judge_pairs(&ps, &env, &rules)
            } else {
                let ops: Vec<Creature> = opponents
                    .map(|d| game::load_folder(&d, &rules).into_iter().filter_map(|e| e.creature.ok()).collect())
                    .unwrap_or_default();
                let cs: Vec<Creature> = ok.into_iter().map(|mut v| v.remove(0)).collect();
                game::judge(&cs, mode, &env, &rules, &ops)
            };
            let mut results = results.into_iter();
            let mut out = BufWriter::new(std::io::stdout().lock());
            let what = if pairs { "pair" } else { "creature" };
            for (k, p) in parsed.iter().enumerate() {
                let r = match p {
                    Ok(_) => results.next().expect("one result per parsed line"),
                    Err(e) => Err(e.clone()),
                };
                match r {
                    Ok(f) => writeln!(out, "{f}").ok(),
                    Err(e) => {
                        eprintln!("{what} {k}: {e}");
                        writeln!(out, "-inf").ok()
                    }
                };
            }
        }
        Cmd::Rules { rules } => {
            println!("{}", serde_json::to_string_pretty(&load_rules(rules.as_deref())).expect("serialisable"));
        }
        Cmd::Convert { input, output } => {
            let c: Creature = if input == "-" {
                let mut text = String::new();
                std::io::Read::read_to_string(&mut std::io::stdin(), &mut text).unwrap_or_else(|e| die(&e.to_string()));
                serde_json::from_str(&text).unwrap_or_else(|e| die(&e.to_string()))
            } else if input.ends_with(".json") {
                let text = std::fs::read_to_string(&input).unwrap_or_else(|e| die(&e.to_string()));
                serde_json::from_str(&text).unwrap_or_else(|e| die(&e.to_string()))
            } else {
                Creature::load(Path::new(&input)).unwrap_or_else(|e| die(&e))
            };
            if output == "-" {
                println!("{}", serde_json::to_string(&c).expect("serialisable"));
            } else if output.ends_with(".json") {
                std::fs::write(&output, serde_json::to_string_pretty(&c).expect("serialisable")).unwrap_or_else(|e| die(&e.to_string()));
            } else {
                c.save(Path::new(&output)).unwrap_or_else(|e| die(&e));
            }
        }
        Cmd::Check { folder } => {
            let rules = folder_rules(&folder);
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
        Cmd::Race { folder } => {
            let rules = folder_rules(&folder);
            let cs: Vec<_> = game::load_folder(&folder, &rules).into_iter().filter_map(|e| e.creature.ok()).collect();
            for (rank, (i, d)) in game::race_all(&cs, &rules).into_iter().enumerate() {
                println!("{:>2}. {:<24} {:>7.2} m", rank + 1, cs[i].name, d);
            }
        }
        Cmd::Tournament { folder } => {
            let rules = folder_rules(&folder);
            let cs: Vec<_> = game::load_folder(&folder, &rules).into_iter().filter_map(|e| e.creature.ok()).collect();
            let (table, _) = game::tournament(&cs, &rules);
            println!("    {:<24} {:>3} {:>3} {:>3} {:>4}", "", "W", "D", "L", "Pts");
            for (rank, s) in table.iter().enumerate() {
                println!("{:>2}. {:<24} {:>3} {:>3} {:>3} {:>4}", rank + 1, cs[s.index].name, s.wins, s.draws, s.losses, s.points);
            }
        }
    }
}
