"""Generalisation experiment: does a creature trained on one track work on others?

Like train/test splits in machine learning: evolve on some terrains
(training seeds), then measure on terrains never seen during evolution
(test seeds 1000+). Compares three ways of training:

  single   one terrain (seed 0)
  multi    several terrains (seeds 0..trials-1), averaged
  robust   several terrains + friction jitter, worst case counts

    python3 python/generalize.py creatures/worm.toml --roughness 0.3

Every evaluation of a "multi"/"robust" genome costs `trials` simulations.
By default all methods get the same number of simulations (equal compute);
`--same evaluations` gives them the same number of GA evaluations instead
(multi/robust then use `trials` times more compute).
"""

import argparse
import os
import tempfile

import ga
from arena import Problem


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("template")
    ap.add_argument("--level", default="brain", choices=["brain", "body", "structure"])
    ap.add_argument("--roughness", type=float, default=0.3, help="hill height of the terrain (m)")
    ap.add_argument("--trials", type=int, default=4, help="training terrains for multi / robust")
    ap.add_argument("--budget", type=int, default=4000, help="budget per method (see --same)")
    ap.add_argument("--same", default="simulations", choices=["simulations", "evaluations"],
                    help="what the budget counts: equal compute, or equal GA evaluations")
    ap.add_argument("--test", type=int, default=20, help="number of unseen test terrains")
    args = ap.parse_args()

    # A rules file with rough terrain, next to nothing else.
    rules = os.path.join(tempfile.mkdtemp(), "arena.toml")
    with open(rules, "w") as f:
        f.write(f"terrain_roughness = {args.roughness}\n")

    def problem(**env):
        return Problem(args.template, mode="race", level=args.level, rules=rules, **env)

    methods = {
        "single": dict(trials=1, env_seed=0),
        "multi": dict(trials=args.trials, env_seed=0),
        "robust": dict(trials=args.trials, env_seed=0, friction_jitter=0.3, aggregate="min"),
    }
    test = problem(trials=args.test, env_seed=1000)
    test_worst = problem(trials=args.test, env_seed=1000, aggregate="min")
    test_slippery = problem(trials=args.test, env_seed=1000, friction_jitter=0.5, aggregate="min")

    print(f"{'method':<8} {'train':>8} {'test mean':>10} {'test worst':>11} {'slippery worst':>15}")
    for name, env in methods.items():
        train = problem(**env)
        budget = args.budget // env["trials"] if args.same == "simulations" else args.budget
        best, train_fit = ga.run(train, budget=budget, log=None)
        print(f"{name:<8} {train_fit:8.2f} {test.evaluate([best])[0]:10.2f} "
              f"{test_worst.evaluate([best])[0]:11.2f} {test_slippery.evaluate([best])[0]:15.2f}")


if __name__ == "__main__":
    main()
