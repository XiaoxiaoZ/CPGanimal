"""Your genetic algorithm: fill in the three operators.

    python3 python/my_ga.py creatures/worm.toml --out creatures/me.toml --name "My Worm"

It has the same structure, options and output as the default GA (python/ga.py);
only `select`, `crossover` and `mutate` are left for you. As they are, nothing
improves: parents are picked at random, a child is a copy of its parent and
mutation changes nothing. Compare your GA with python/random_search.py and
python/ga.py at the same --budget.

You may borrow single operators from ga.py (e.g. `from ga import tournament`)
and write only the rest, or change run() itself, e.g. the survivor selection.
Like ga.run, run() can be called from another script:

    import my_ga
    best, fitness = my_ga.run(Problem("creatures/worm.toml"), budget=1000)

The Train tab of arena-gui runs this file as "My GA"; a copy of it can be run
there as a custom script.
"""

import argparse
import random

from arena import Problem


# ---------------------------------------------------------------- operators
# TODO: write these three. A genome is a list of `dim` floats in [0, 1];
# run() calls them for every child it makes, so keep them fast and simple.

def select(population, fitness):
    """Pick one parent and return it (a genome from `population`).

    fitness[i] is the fitness of population[i]; higher is better. Fitter
    individuals should be picked more often, but not always, or the
    population loses its variety too fast.
    TODO: tournament selection, roulette wheel, rank selection, ...
    Placeholder: a random individual, whatever its fitness."""
    return random.choice(population)


def crossover(a, b):
    """Make a child from parents `a` and `b` and return it as a new list.

    The child should take something from both parents, gene by gene.
    TODO: uniform (each gene from a or b), one-point, blend (a value
    between the parents' values), ...
    Placeholder: a copy of the first parent."""
    return list(a)


def mutate(genome, sigma=0.1):
    """Change a child a little and return it as a new list.

    Small random changes let the population explore beyond what its parents
    already have. `sigma` is the mutation strength (the --sigma option).
    TODO: e.g. add Gaussian noise random.gauss(0, sigma) to each gene with a
    small probability such as 1/len(genome).
    Placeholder: no change at all."""
    return list(genome)


def clamp(genome):
    """Keep every gene in [0, 1]: crossover and mutation may step outside."""
    return [min(1.0, max(0.0, g)) for g in genome]


# ---------------------------------------------------------------- algorithm
# The same loop as ga.run: elitism, then new children from select / crossover / mutate.
# You may change it too, e.g. the survivor selection.

def run(problem, population=50, budget=1000, elites=2, crossover_rate=0.9,
        select=select, crossover=crossover, mutate=mutate,
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
    ap = argparse.ArgumentParser(description="Evolve a creature with your GA.")
    ap.add_argument("template", nargs="?", default="creatures/worm.toml",
                    help="creature file; its body plan is kept (default: creatures/worm.toml)")
    ap.add_argument("--out", default="creatures/my-creature.toml", help="where to write the champion")
    ap.add_argument("--name", default="My Creature", help="champion name shown in the arena")
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
    ap.add_argument("--sigma", type=float, default=0.1, help="mutation strength, passed to mutate()")
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--history", help="CSV file with best/mean fitness per generation")
    args = ap.parse_args()

    # The black box the GA optimises: genome (dim numbers in [0, 1]) -> fitness.
    problem = Problem(args.template, mode=args.mode, level=args.level, opponents=args.opponents, rules=args.rules,
                      trials=args.trials, env_seed=args.env_seed, friction_jitter=args.friction_jitter,
                      aggregate=args.aggregate)
    print(f"{problem.info['creature']}: {problem.dim} genes, level {args.level}, mode {args.mode}, budget {args.budget}")
    best, fitness = run(problem, population=args.population, budget=args.budget, seed=args.seed,
                        mutate=lambda g: mutate(g, sigma=args.sigma), history=args.history)
    # Decode the best genome into a creature file for the arena.
    problem.save(best, args.out, name=args.name)
    print(f"champion fitness {fitness:.3f} -> {args.out}")


if __name__ == "__main__":
    main()
