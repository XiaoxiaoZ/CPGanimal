"""Baseline: pure random search. Your GA should beat this with the same budget.

    python3 python/random_search.py creatures/worm.toml
"""

import random
import sys

from arena import Problem

BUDGET = 1000          # total evaluations, keep it equal when comparing algorithms
BATCH = 50

p = Problem(sys.argv[1] if len(sys.argv) > 1 else "creatures/worm.toml", mode="race")
random.seed(0)
best, best_fit = p.start, p.evaluate([p.start])[0]
while p.evaluations < BUDGET:
    pop = [[random.random() for _ in range(p.dim)] for _ in range(BATCH)]
    for g, f in zip(pop, p.evaluate(pop)):
        if f > best_fit:
            best, best_fit = g, f
    print(f"evaluations {p.evaluations:5d}  best {best_fit:8.3f}")
