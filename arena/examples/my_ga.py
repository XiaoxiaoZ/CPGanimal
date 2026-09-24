"""Write your own genetic algorithm: this file is a starting point.

The simulator is a black box you call with a list of genes in [0, 1]:

    arena eval creatures/worm.toml --genes 0.1,0.5,... --mode race
    -> {"fitness": 12.3, ...}

`arena genes creatures/worm.toml` lists what each gene means.
Everything below (selection, crossover, mutation) is yours to change.

Run from the arena/ folder after `cargo build --release`:
    python3 examples/my_ga.py creatures/worm.toml
"""

import json
import random
import subprocess
import sys

ARENA = "./target/release/arena"
TEMPLATE = sys.argv[1] if len(sys.argv) > 1 else "creatures/worm.toml"
MODE = "race"          # or "sumo"
POP, GENERATIONS = 20, 10


def n_genes():
    out = subprocess.run([ARENA, "genes", TEMPLATE], capture_output=True, text=True, check=True).stdout
    return len(out.strip().splitlines())


def fitness(genes, out=None):
    cmd = [ARENA, "eval", TEMPLATE, "--mode", MODE, "--genes", ",".join(f"{g:.6f}" for g in genes)]
    if out:
        cmd += ["--out", out, "--name", "My Champion"]
    res = subprocess.run(cmd, capture_output=True, text=True, check=True).stdout
    return json.loads(res)["fitness"]


def mutate(genes, rate=0.2, sigma=0.1):
    return [min(1.0, max(0.0, g + random.gauss(0, sigma))) if random.random() < rate else g for g in genes]


def crossover(a, b):
    return [random.choice(pair) for pair in zip(a, b)]


def main():
    random.seed(1)
    n = n_genes()
    pop = [[random.random() for _ in range(n)] for _ in range(POP)]
    for gen in range(GENERATIONS):
        scored = sorted(((fitness(g), g) for g in pop), key=lambda t: t[0], reverse=True)
        print(f"gen {gen:2d}  best {scored[0][0]:8.3f}")
        parents = [g for _, g in scored[: POP // 4]]           # truncation selection
        pop = [scored[0][1]] + [mutate(crossover(*random.sample(parents, 2))) for _ in range(POP - 1)]
    best = max(pop, key=fitness)
    print("best fitness", fitness(best, out="creatures/my-champion.toml"), "-> creatures/my-champion.toml")


if __name__ == "__main__":
    main()
