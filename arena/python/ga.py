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
    """Tournament selection: pick k individuals at random and return the
    fittest of them. fitness[i] belongs to population[i]; higher is better.
    A larger k means stronger selection pressure (the best are picked more
    often); k = 1 would be a purely random pick."""
    best = random.randrange(len(population))      # the first contender
    for _ in range(k - 1):
        c = random.randrange(len(population))     # another random contender
        if fitness[c] > fitness[best]:
            best = c
    return population[best]


def blend_crossover(a, b, alpha=0.3):
    """BLX-alpha crossover for real-valued genes. For each gene the child's
    value is drawn uniformly from the interval between the two parents'
    values, widened by alpha times its length on both sides: alpha = 0 stays
    strictly between the parents, alpha > 0 also explores a little beyond."""
    child = []
    for x, y in zip(a, b):
        lo, hi = min(x, y), max(x, y)
        d = hi - lo                               # how far apart the parents are on this gene
        child.append(random.uniform(lo - alpha * d, hi + alpha * d))
    return child


def gaussian_mutation(genome, sigma=0.1, rate=None):
    """Gaussian mutation: each gene, with probability `rate`, gets noise from
    a normal distribution N(0, sigma) added to it. The default rate 1/dim
    changes about one gene per child; sigma is the typical step size."""
    rate = 1.0 / len(genome) if rate is None else rate
    return [g + random.gauss(0.0, sigma) if random.random() < rate else g for g in genome]


def clamp(genome):
    """Keep every gene in [0, 1]: crossover and mutation may step outside."""
    return [min(1.0, max(0.0, g)) for g in genome]


# ---------------------------------------------------------------- algorithm

def run(problem, population=50, budget=1000, elites=2, crossover_rate=0.9,
        select=tournament, crossover=blend_crossover, mutate=gaussian_mutation,
        seed=1, log=print, history=None):
    """Evolve until `budget` evaluations are used. Returns (best genome, best fitness).

    population:      individuals per generation
    budget:          the most evaluations (simulations) to use in total
    elites:          how many of the best go on to the next generation unchanged
    crossover_rate:  chance that a child comes from crossover; otherwise it copies one parent
    select, crossover, mutate: the operators above (or your own)
    seed:            random seed, so that a run can be repeated exactly
    log:             prints one progress line per generation; None for silence
    history:         optional path of a CSV file with one row per generation
    """
    random.seed(seed)
    # Generation 0: the template creature (a sensible start) plus random genomes.
    pop = [problem.start] + [[random.random() for _ in range(problem.dim)] for _ in range(population - 1)]
    fit = problem.evaluate(pop)  # one simulation per individual, all run in parallel
    # The best individual seen so far: the answer, whatever happens later.
    best_i = max(range(len(pop)), key=lambda i: fit[i])
    best, best_fit = pop[best_i], fit[best_i]
    rows = []
    generation = 0

    def report():
        """Print and remember one line of progress for the current generation."""
        mean = sum(fit) / len(fit)
        rows.append((generation, problem.evaluations, max(fit), mean, best_fit))
        if log:
            log(f"gen {generation:3d}  evaluations {problem.evaluations:5d}  "
                f"best {max(fit):8.3f}  mean {mean:8.3f}  best ever {best_fit:8.3f}")

    report()
    # Each generation evaluates `population - elites` new children. Stop before
    # that would exceed the budget, so different algorithms compare fairly.
    while problem.evaluations + population - elites <= budget:
        # 1. Elitism: the `elites` best individuals survive unchanged.
        order = sorted(range(len(pop)), key=lambda i: fit[i], reverse=True)
        next_pop = [pop[i] for i in order[:elites]]
        next_fit = [fit[i] for i in order[:elites]]
        # 2. Children fill the rest of the new generation.
        children = []
        while len(next_pop) + len(children) < population:
            a = select(pop, fit)                          # first parent
            if random.random() < crossover_rate:
                child = crossover(a, select(pop, fit))    # mix it with a second parent
            else:
                child = list(a)                           # or copy it
            children.append(clamp(mutate(child)))         # then mutate, keeping genes in [0, 1]
        # 3. Evaluate the children only. The elites keep their fitness: the
        #    simulation is deterministic, so the same genome always scores the same.
        pop = next_pop + children
        fit = next_fit + problem.evaluate(children)
        generation += 1
        # 4. Remember the best individual ever seen (with elites > 0 it is never
        #    lost anyway; this also covers elites = 0).
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
    ap.add_argument("--level", default="brain", choices=["brain", "body", "structure"],
                    help="brain: CPG only; body: + segment sizes and angles; structure: + number of segments and topology")
    ap.add_argument("--opponents", help="sumo: folder of creatures to fight (default: the Rock)")
    ap.add_argument("--rules", help="rules file, e.g. the class arena.toml")
    ap.add_argument("--budget", type=int, default=1000, help="total evaluations (default 1000)")
    ap.add_argument("--trials", type=int, default=1, help="environments per evaluation (terrain seeds)")
    ap.add_argument("--env-seed", type=int, help="terrain seed of the first environment")
    ap.add_argument("--friction-jitter", type=float, default=0.0, help="random friction scale +-j per environment")
    ap.add_argument("--aggregate", default="mean", choices=["mean", "min"], help="combine environments by mean or worst case")
    ap.add_argument("--population", type=int, default=50)
    ap.add_argument("--sigma", type=float, default=0.1, help="mutation strength")
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--history", help="CSV file with best/mean fitness per generation")
    args = ap.parse_args()

    # The black box the GA optimises: genome (dim numbers in [0, 1]) -> fitness.
    problem = Problem(args.template, mode=args.mode, level=args.level, opponents=args.opponents, rules=args.rules,
                      trials=args.trials, env_seed=args.env_seed, friction_jitter=args.friction_jitter,
                      aggregate=args.aggregate)
    print(f"{problem.info['creature']}: {problem.dim} genes, level {args.level}, mode {args.mode}, budget {args.budget}")
    best, fitness = run(problem, population=args.population, budget=args.budget, seed=args.seed,
                        mutate=lambda g: gaussian_mutation(g, sigma=args.sigma), history=args.history)
    # Decode the best genome into a creature file for the arena.
    problem.save(best, args.out, name=args.name)
    print(f"champion fitness {fitness:.3f} -> {args.out}")


if __name__ == "__main__":
    main()
