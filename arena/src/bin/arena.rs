//! Command-line tool. The optimisation interface for your own GA:
//!
//! ```text
//! arena info  worm.toml                 how many genes, what they mean
//! arena batch worm.toml < pop.txt       one genome per line in, one fitness per line out
//! arena eval  worm.toml --genes 0.1,…   fitness of one genome
//! arena save  worm.toml --genes 0.1,… --out me.toml --name "Me"
//! ```
//!
//! Plus `check`, `race` and `tournament` for folders of creatures.

use clap::{Args, Parser, Subcommand};
use cpg_arena::creature::{Creature, Level, Rules};
use cpg_arena::game::{self, Mode};
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
}

impl ProblemArgs {
    fn problem(&self) -> Problem {
        Problem::load(&self.template, self.mode, self.level, self.opponents.as_deref(), self.rules.as_deref()).unwrap_or_else(|e| die(&e))
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
        Cmd::Save { problem, genes, out, name } => {
            let p = problem.problem();
            let f = p.save(&genes, &out, name.as_deref()).unwrap_or_else(|e| die(&e));
            println!("fitness {f} -> {}", out.display());
        }
        Cmd::Judge { mode, opponents, rules } => {
            let rules = load_rules(rules.as_deref());
            let ops: Vec<Creature> = opponents
                .map(|d| game::load_folder(&d, &rules).into_iter().filter_map(|e| e.creature.ok()).collect())
                .unwrap_or_default();
            let mut cs = Vec::new();
            let mut bad = Vec::new();
            for line in std::io::stdin().lock().lines() {
                let line = line.unwrap_or_else(|e| die(&e.to_string()));
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<Creature>(&line) {
                    Ok(c) => cs.push(Some(c)),
                    Err(e) => {
                        bad.push(format!("creature {}: {e}", cs.len()));
                        cs.push(None);
                    }
                }
            }
            let valid: Vec<Creature> = cs.iter().flatten().cloned().collect();
            let mut results = game::judge(&valid, mode, &rules, &ops).into_iter();
            let mut out = BufWriter::new(std::io::stdout().lock());
            for (k, c) in cs.iter().enumerate() {
                let r = match c {
                    Some(_) => results.next().expect("one result per valid creature"),
                    None => Err(String::new()),
                };
                match r {
                    Ok(f) => writeln!(out, "{f}").ok(),
                    Err(e) => {
                        if !e.is_empty() {
                            bad.push(format!("creature {k}: {e}"));
                        }
                        writeln!(out, "-inf").ok()
                    }
                };
            }
            bad.sort();
            for b in bad {
                eprintln!("{b}");
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
