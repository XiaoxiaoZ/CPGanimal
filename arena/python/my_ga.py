"""Your genetic algorithm. Fill in the three TODOs.

    python3 python/my_ga.py

It runs as-is, but with the placeholder operators nothing improves.
Compare your result with python/random_search.py and the default GA
(python/ga.py) at the same BUDGET. You may also borrow single operators
from ga.py, e.g. `from ga import tournament`, and write only the rest.
"""

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
    p = Problem(TEMPLATE, mode=MODE)
    print(f"{p.dim} genes: {', '.join(p.genes)}")
    random.seed(1)
    population = [p.start] + [[random.random() for _ in range(p.dim)] for _ in range(POPULATION - 1)]
    fitness = p.evaluate(population)
    generation = 0
    while p.evaluations < BUDGET:
        children = [mutate(crossover(select(population, fitness), select(population, fitness))) for _ in range(POPULATION)]
        child_fitness = p.evaluate(children)
        # Survivor selection: keep the best POPULATION of parents + children.
        ranked = sorted(zip(fitness + child_fitness, population + children), key=lambda t: t[0], reverse=True)
        fitness = [f for f, _ in ranked[:POPULATION]]
        population = [g for _, g in ranked[:POPULATION]]
        generation += 1
        print(f"gen {generation:3d}  evaluations {p.evaluations:5d}  best {fitness[0]:8.3f}  mean {sum(fitness) / len(fitness):8.3f}")
    print(f"best fitness {p.save(population[0], OUT, name=NAME):.3f} -> {OUT}")


if __name__ == "__main__":
    main()
