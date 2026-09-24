"""Co-evolution: sumo wrestlers that train against each other.

Training against fixed opponents only teaches a creature to beat those few.
Here the opponents are the population itself, so every improvement raises
the bar for everyone: an arms race.

Two classic problems, and what this script does about them:

- Cycling (rock-paper-scissors): A beats B, B beats C, C beats A, and the
  population goes round in circles. A *hall of fame* of past champions stays
  in the opponent pool, so a creature must also keep beating old tricks.
- No absolute progress signal: fitness is relative to a moving population,
  so "best fitness" can stay flat while everyone gets better. Every few
  generations the champion is measured against a fixed benchmark folder.

    python3 python/coevolve.py creatures/tailfin.toml --benchmark creatures --out creatures/me.toml

Operators (selection, crossover, mutation) come from ga.py; swap in your own.
"""

import argparse
import random

import ga
from arena import Problem


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("template")
    ap.add_argument("--out", required=True)
    ap.add_argument("--name", default="Coevolved")
    ap.add_argument("--level", default="brain", choices=["brain", "body", "structure"])
    ap.add_argument("--rules")
    ap.add_argument("--population", type=int, default=30)
    ap.add_argument("--generations", type=int, default=30)
    ap.add_argument("--bouts", type=int, default=4, help="opponents each individual fights per generation")
    ap.add_argument("--hall", type=int, default=10, help="past champions kept as opponents (0 = no hall of fame)")
    ap.add_argument("--benchmark", help="folder of fixed opponents to measure real progress (default: the Rock)")
    ap.add_argument("--seed", type=int, default=1)
    args = ap.parse_args()

    random.seed(args.seed)
    arena = Problem(args.template, mode="sumo", level=args.level, rules=args.rules)
    bench = Problem(args.template, mode="sumo", level=args.level, rules=args.rules, opponents=args.benchmark)
    n = args.population
    pop = [arena.start] + [[random.random() for _ in range(arena.dim)] for _ in range(n - 1)]
    hall = []

    for gen in range(1, args.generations + 1):
        # Everyone fights `bouts` opponents drawn from the population and the hall of fame.
        pairs, who = [], []
        for i in range(n):
            for _ in range(args.bouts):
                if hall and random.random() < 0.5:
                    pairs.append((pop[i], random.choice(hall)))
                    who.append((i, None))
                else:
                    j = random.choice([j for j in range(n) if j != i])
                    pairs.append((pop[i], pop[j]))
                    who.append((i, j))
        scores = arena.fight(pairs)
        # One bout rates both sides: b's score is the negative of a's.
        total, count = [0.0] * n, [0] * n
        for (i, j), s in zip(who, scores):
            total[i] += s
            count[i] += 1
            if j is not None:
                total[j] -= s
                count[j] += 1
        fit = [t / c for t, c in zip(total, count)]

        champ = pop[max(range(n), key=lambda i: fit[i])]
        if args.hall:
            hall = (hall + [champ])[-args.hall:]
        line = f"gen {gen:3d}  duels {arena.evaluations:5d}  best {max(fit):6.3f}  mean {sum(fit) / n:6.3f}"
        if gen % 5 == 0 or gen == args.generations:
            line += f"  | champion vs benchmark {bench.evaluate([champ])[0]:6.3f}"
        print(line)

        # Next generation: keep the 2 best, breed the rest with the ga.py operators.
        order = sorted(range(n), key=lambda i: fit[i], reverse=True)
        nxt = [pop[order[0]], pop[order[1]]]
        while len(nxt) < n:
            a, b = ga.tournament(pop, fit), ga.tournament(pop, fit)
            nxt.append(ga.clamp(ga.gaussian_mutation(ga.blend_crossover(a, b))))
        pop = nxt

    f = bench.save(champ, args.out, name=args.name)
    print(f"champion (vs benchmark {f:.3f}) -> {args.out}")


if __name__ == "__main__":
    main()
