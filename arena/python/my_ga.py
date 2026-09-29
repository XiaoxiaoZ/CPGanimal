"""Your genetic algorithm. Fill in the three TODOs.

    python3 python/my_ga.py

It runs as-is, but with the placeholder operators nothing improves.
Compare your result with python/random_search.py and the default GA
(python/ga.py) at the same BUDGET. You may also borrow single operators
from ga.py, e.g. `from ga import tournament`, and write only the rest.

The constants below are the defaults; the Train tab of arena-gui overrides
them on the command line (template, --out, --name, --mode, --level, --budget,
--population, --seed, --opponents, --rules). A copy of this file with the
same main() can be run from the Train tab as a custom script.
"""

import argparse
import random

from arena import Problem

TEMPLATE = "creatures/worm.toml"
MODE = "race"          # "race" or "sumo"
POPULATION = 50
BUDGET = 1000          # total number of evaluations
OUT = "creatures/my-creature.toml"
NAME = "My Creature"


def select(population, fitness):
    """Pick one parent. TODO: tournament selection, roulette wheel, ..."""
    return random.choice(population)


def crossover(a, b):
    """Make a child from two parents. TODO: uniform, one-point, blend, ..."""
    return list(a)


def mutate(genome):
    """Change a child a little. TODO: e.g. add Gaussian noise to some genes.
    Keep genes in [0, 1] (values outside are clamped anyway)."""
    return genome


def main():
    ap = argparse.ArgumentParser(description="Your GA. Without arguments it uses the constants at the top.")
    ap.add_argument("template", nargs="?", default=TEMPLATE)
    ap.add_argument("--out", default=OUT)
    ap.add_argument("--name", default=NAME)
    ap.add_argument("--mode", default=MODE, choices=["race", "sumo"])
    ap.add_argument("--level", default="brain", choices=["brain", "body", "structure"])
    ap.add_argument("--budget", type=int, default=BUDGET)
    ap.add_argument("--population", type=int, default=POPULATION)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--opponents")
    ap.add_argument("--rules")
    args = ap.parse_args()
    n = args.population

    p = Problem(args.template, mode=args.mode, level=args.level, opponents=args.opponents, rules=args.rules)
    print(f"{p.dim} genes: {', '.join(p.genes)}")
    random.seed(args.seed)
    population = [p.start] + [[random.random() for _ in range(p.dim)] for _ in range(n - 1)]
    fitness = p.evaluate(population)
    generation = 0
    while p.evaluations < args.budget:
        children = [mutate(crossover(select(population, fitness), select(population, fitness))) for _ in range(n)]
        child_fitness = p.evaluate(children)
        # Survivor selection: keep the best n of parents + children.
        ranked = sorted(zip(fitness + child_fitness, population + children), key=lambda t: t[0], reverse=True)
        fitness = [f for f, _ in ranked[:n]]
        population = [g for _, g in ranked[:n]]
        generation += 1
        print(f"gen {generation:3d}  evaluations {p.evaluations:5d}  best {fitness[0]:8.3f}  mean {sum(fitness) / len(fitness):8.3f}")
    print(f"best fitness {p.save(population[0], args.out, name=args.name):.3f} -> {args.out}")


if __name__ == "__main__":
    main()
