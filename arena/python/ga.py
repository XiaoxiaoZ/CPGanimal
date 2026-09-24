"""A ready-to-use genetic algorithm for CPG Arena.

Use it as a tool:

    python3 python/ga.py creatures/worm.toml --out creatures/me.toml --name "My Worm"
    python3 python/ga.py creatures/tailfin.toml --mode sumo --opponents creatures --budget 2000 --out ...

or as a library, swapping in your own operators one at a time:

    from arena import Problem
    import ga

    def my_mutation(genome):
        ...

    best, fitness = ga.run(Problem("creatures/worm.toml"), mutate=my_mutation)

Algorithm: generational, real-coded, genes in [0, 1].
  selection  tournament of size 3
  crossover  BLX-alpha blend (alpha = 0.3), with probability 0.9
  mutation   Gaussian noise (sigma = 0.1) on each gene with probability 1/dim
  elitism    the best 2 individuals survive unchanged
"""

import argparse
import random

from arena import Problem


# ---------------------------------------------------------------- operators
# Each operator is a plain function; replace any of them in run(...).

def tournament(population, fitness, k=3):
    """Pick the fittest of k random individuals."""
    best = random.randrange(len(population))
    for _ in range(k - 1):
        c = random.randrange(len(population))
        if fitness[c] > fitness[best]:
            best = c
    return population[best]


def blend_crossover(a, b, alpha=0.3):
    """BLX-alpha: each child gene is uniform in the parents' range, widened by alpha."""
    child = []
    for x, y in zip(a, b):
        lo, hi = min(x, y), max(x, y)
        d = hi - lo
        child.append(random.uniform(lo - alpha * d, hi + alpha * d))
    return child


def gaussian_mutation(genome, sigma=0.1, rate=None):
    """Add N(0, sigma) noise to each gene with probability `rate` (default 1/dim)."""
    rate = 1.0 / len(genome) if rate is None else rate
    return [g + random.gauss(0.0, sigma) if random.random() < rate else g for g in genome]


def clamp(genome):
    return [min(1.0, max(0.0, g)) for g in genome]


# ---------------------------------------------------------------- algorithm

def run(problem, population=50, budget=1000, elites=2, crossover_rate=0.9,
        select=tournament, crossover=blend_crossover, mutate=gaussian_mutation,
        seed=1, log=print, history=None):
    """Evolve until `budget` evaluations are used. Returns (best genome, best fitness).

    history: optional path of a CSV file with one row per generation.
    """
    random.seed(seed)
    pop = [problem.start] + [[random.random() for _ in range(problem.dim)] for _ in range(population - 1)]
    fit = problem.evaluate(pop)
    best_i = max(range(len(pop)), key=lambda i: fit[i])
    best, best_fit = pop[best_i], fit[best_i]
    rows = []
    generation = 0

    def report():
        mean = sum(fit) / len(fit)
        rows.append((generation, problem.evaluations, max(fit), mean, best_fit))
        if log:
            log(f"gen {generation:3d}  evaluations {problem.evaluations:5d}  "
                f"best {max(fit):8.3f}  mean {mean:8.3f}  best ever {best_fit:8.3f}")

    report()
    while problem.evaluations + population - elites <= budget:
        order = sorted(range(len(pop)), key=lambda i: fit[i], reverse=True)
        next_pop = [pop[i] for i in order[:elites]]
        next_fit = [fit[i] for i in order[:elites]]
        children = []
        while len(next_pop) + len(children) < population:
            a = select(pop, fit)
            child = crossover(a, select(pop, fit)) if random.random() < crossover_rate else list(a)
            children.append(clamp(mutate(child)))
        # Elites are not re-evaluated: fitness is deterministic.
        pop = next_pop + children
        fit = next_fit + problem.evaluate(children)
        generation += 1
        i = max(range(len(pop)), key=lambda i: fit[i])
        if fit[i] > best_fit:
            best, best_fit = pop[i], fit[i]
        report()

    if history:
        with open(history, "w") as f:
            f.write("generation,evaluations,best,mean,best_ever\n")
            for r in rows:
                f.write(",".join(str(x) for x in r) + "\n")
    return best, best_fit


def main():
    ap = argparse.ArgumentParser(description="Evolve a creature with the default GA.")
    ap.add_argument("template", help="creature file; its body plan is kept")
    ap.add_argument("--out", required=True, help="where to write the champion")
    ap.add_argument("--name", help="champion name shown in the arena")
    ap.add_argument("--mode", default="race", choices=["race", "sumo"])
    ap.add_argument("--body", action="store_true", help="also evolve segment sizes, attach points and angles")
    ap.add_argument("--opponents", help="sumo: folder of creatures to fight (default: the Rock)")
    ap.add_argument("--rules", help="rules file, e.g. the class arena.toml")
    ap.add_argument("--budget", type=int, default=1000, help="total evaluations (default 1000)")
    ap.add_argument("--population", type=int, default=50)
    ap.add_argument("--sigma", type=float, default=0.1, help="mutation strength")
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--history", help="CSV file with best/mean fitness per generation")
    args = ap.parse_args()

    problem = Problem(args.template, mode=args.mode, body=args.body, opponents=args.opponents, rules=args.rules)
    print(f"{problem.info['creature']}: {problem.dim} genes, mode {args.mode}, budget {args.budget}")
    best, fitness = run(problem, population=args.population, budget=args.budget, seed=args.seed,
                        mutate=lambda g: gaussian_mutation(g, sigma=args.sigma), history=args.history)
    problem.save(best, args.out, name=args.name)
    print(f"champion fitness {fitness:.3f} -> {args.out}")


if __name__ == "__main__":
    main()
