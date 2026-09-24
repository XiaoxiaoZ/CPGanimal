"""Evolve body structure with your own mutation operators.

Instead of a fixed-length genome, each individual is a creature (a dict with
the same fields as the .toml files). Structural mutations grow a limb, cut a
limb, or move a limb to another parent; parametric mutations nudge numbers.
This is the Karl Sims idea in miniature.

    python3 python/evolve_structure.py creatures/worm.toml --out creatures/me.toml

Every operator below is a starting point: change them, add your own, and
compare against `python3 python/ga.py ... --level structure` at the same budget.
"""

import argparse
import copy
import random

from arena import Judge, load_creature, save_creature


def clip(x, bounds):
    return min(bounds[1], max(bounds[0], x))


# ------------------------------------------------------ structural mutations

def add_limb(c, rules):
    """Attach a new segment to a random existing one."""
    if len(c["segment"]) >= rules["max_segments"]:
        return c
    c["segment"].append({
        "parent": random.randrange(len(c["segment"])),
        "attach": random.uniform(-1, 1),
        "angle": random.uniform(-90, 90),
        "length": random.uniform(*rules["length"]),
        "width": random.uniform(rules["width"][0], sum(rules["width"]) / 2),
        "amplitude": random.uniform(*rules["amplitude"]),
        "offset": 0.0,
        "phase": random.uniform(-180, 180),
    })
    return c


def remove_limb(c, rules):
    """Remove a leaf segment (one nothing else hangs off). Never the torso."""
    segs = c["segment"]
    parents = {s.get("parent", 0) for s in segs[1:]}
    leaves = [i for i in range(1, len(segs)) if i not in parents]
    if not leaves:
        return c
    k = random.choice(leaves)
    del segs[k]
    for s in segs[1:]:
        if s.get("parent", 0) > k:
            s["parent"] -= 1
    return c


def reattach(c, rules):
    """Move a segment (with everything hanging off it) to another earlier parent."""
    segs = c["segment"]
    if len(segs) < 3:
        return c
    k = random.randrange(2, len(segs))
    segs[k]["parent"] = random.randrange(k)
    segs[k]["attach"] = random.uniform(-1, 1)
    return c


# ------------------------------------------------------ parametric mutation

def nudge(c, rules, sigma=0.1):
    """Gaussian noise on numbers, scaled to each range."""
    b = c["brain"]
    b["frequency"] = clip(b["frequency"] + random.gauss(0, sigma * 2.8), rules["frequency"])
    for i, s in enumerate(c["segment"]):
        for key, bounds in (("length", rules["length"]), ("width", rules["width"])):
            if random.random() < 0.2:
                s[key] = clip(s.get(key, 0.3) + random.gauss(0, sigma * (bounds[1] - bounds[0])), bounds)
        if i == 0:
            continue
        for key, bounds in (("amplitude", rules["amplitude"]), ("offset", rules["offset"]),
                            ("angle", rules["angle"]), ("attach", [-1, 1]), ("phase", [-180, 180])):
            if random.random() < 0.2:
                s[key] = clip(s.get(key, 0.0) + random.gauss(0, sigma * (bounds[1] - bounds[0])), bounds)
    return c


def shrink_to_budget(c, rules):
    """Scale the body down if it exceeds the area budget (otherwise it scores -inf)."""
    area = sum(s["length"] * s["width"] for s in c["segment"])
    if area > rules["max_area"]:
        k = (rules["max_area"] / area) ** 0.5 * 0.999
        for s in c["segment"]:
            s["length"] = max(rules["length"][0], s["length"] * k)
            s["width"] = max(rules["width"][0], s["width"] * k)
    return c


def mutate(c, rules, p_structure=0.3):
    c = copy.deepcopy(c)
    if random.random() < p_structure:
        c = random.choice([add_limb, remove_limb, reattach])(c, rules)
    c = nudge(c, rules)
    return shrink_to_budget(c, rules)


# ------------------------------------------------------ (mu + lambda) evolution

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("template")
    ap.add_argument("--out", required=True)
    ap.add_argument("--name", default="Structure Evolved")
    ap.add_argument("--mode", default="race", choices=["race", "sumo"])
    ap.add_argument("--opponents")
    ap.add_argument("--rules")
    ap.add_argument("--budget", type=int, default=2000)
    ap.add_argument("--mu", type=int, default=10, help="parents kept each generation")
    ap.add_argument("--lam", type=int, default=40, help="children per generation")
    ap.add_argument("--seed", type=int, default=1)
    args = ap.parse_args()

    random.seed(args.seed)
    judge = Judge(mode=args.mode, opponents=args.opponents, rules=args.rules)
    rules = judge.rules
    start = load_creature(args.template)
    start["name"] = args.name
    parents = [start] + [mutate(start, rules, p_structure=1.0) for _ in range(args.mu - 1)]
    fitness = judge.evaluate(parents)
    generation = 0
    while judge.evaluations + args.lam <= args.budget:
        children = [mutate(random.choice(parents), rules) for _ in range(args.lam)]
        child_fitness = judge.evaluate(children)
        ranked = sorted(zip(fitness + child_fitness, parents + children), key=lambda t: t[0], reverse=True)
        fitness = [f for f, _ in ranked[: args.mu]]
        parents = [c for _, c in ranked[: args.mu]]
        generation += 1
        sizes = [len(c["segment"]) for c in parents]
        print(f"gen {generation:3d}  evaluations {judge.evaluations:5d}  best {fitness[0]:8.3f}  "
              f"segments best {sizes[0]}  range {min(sizes)}-{max(sizes)}")
    save_creature(parents[0], args.out)
    print(f"champion fitness {fitness[0]:.3f}, {len(parents[0]['segment'])} segments -> {args.out}")


if __name__ == "__main__":
    main()
