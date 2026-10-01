"""Baseline: pure random search. Your GA should beat this with the same budget.

    python3 python/random_search.py creatures/worm.toml
    python3 python/random_search.py creatures/worm.toml --budget 2000 --out creatures/random.toml
"""

import argparse
import random

from arena import Problem

BUDGET = 1000          # total evaluations, keep it equal when comparing algorithms
BATCH = 50

ap = argparse.ArgumentParser()
ap.add_argument("template", nargs="?", default="creatures/worm.toml")
ap.add_argument("--out", help="save the best creature here")
ap.add_argument("--name", default="Random Search")
ap.add_argument("--mode", default="race", choices=["race", "sumo"])
ap.add_argument("--level", default="brain", choices=["brain", "body", "structure"])
ap.add_argument("--budget", type=int, default=BUDGET)
ap.add_argument("--population", type=int, default=BATCH, help="genomes per batch")
ap.add_argument("--seed", type=int, default=0)
ap.add_argument("--opponents")
ap.add_argument("--rules")
args = ap.parse_args()

p = Problem(args.template, mode=args.mode, level=args.level, opponents=args.opponents, rules=args.rules)
random.seed(args.seed)
# Start from the template, then try completely random genomes and keep the best.
# Nothing is learned from earlier tries: this is what a GA has to beat.
best, best_fit = p.start, p.evaluate([p.start])[0]
while p.evaluations < args.budget:
    # A batch of random genomes, evaluated in parallel. The last batch is cut
    # short: never use more than the budget, like the GAs.
    n = min(args.population, args.budget - p.evaluations)
    pop = [[random.random() for _ in range(p.dim)] for _ in range(n)]
    for g, f in zip(pop, p.evaluate(pop)):
        if f > best_fit:
            best, best_fit = g, f
    print(f"evaluations {p.evaluations:5d}  best {best_fit:8.3f}")
if args.out:
    p.save(best, args.out, name=args.name)
    print(f"best fitness {best_fit:.3f} -> {args.out}")
