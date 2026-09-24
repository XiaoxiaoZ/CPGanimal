"""Python interface to CPG Arena (standard library only).

    from arena import Problem

    p = Problem("creatures/worm.toml", mode="race")
    p.dim                          # number of genes; a genome is p.dim floats in [0, 1]
    p.start                        # the template creature as a genome
    p.evaluate(population)         # list of genomes -> list of fitness (higher is better)
    p.save(best, "me.toml", name="My Creature")

Each `evaluate` call runs the whole population in parallel in one process,
so call it once per generation rather than once per individual.

To evolve structure with your own operators (add a leg, remove a segment,
...), work on creatures directly instead of genomes:

    from arena import Judge, load_creature, save_creature

    j = Judge(mode="race")
    worm = load_creature("creatures/worm.toml")   # a dict, same fields as the file
    j.rules["max_segments"]                        # the limits a creature must respect
    j.evaluate([worm, ...])                        # -> fitness list; rule breakers get -inf,
    j.errors                                       #    and the reasons are here
    save_creature(worm, "me.toml")
"""

import json
import os
import shutil
import subprocess
from pathlib import Path

__all__ = ["Problem", "Judge", "load_creature", "save_creature"]


def _find_binary():
    if os.environ.get("ARENA_BIN"):
        return os.environ["ARENA_BIN"]
    here = Path(__file__).resolve().parent.parent
    for name in ("arena", "arena.exe"):
        p = here / "target" / "release" / name
        if p.exists():
            return str(p)
    found = shutil.which("arena")
    if found:
        return found
    raise FileNotFoundError("arena binary not found: run `cargo build --release` in arena/ or set ARENA_BIN")


class Problem:
    """A creature-evolution problem.

    template:  creature file whose body plan is kept; genes fill in the numbers
    mode:      "race" (distance in metres) or "sumo" (win/loss + ring control)
    level:     what the genes control:
               "brain"     CPG only (frequency, coupling, joint amplitude/offset/phase)
               "body"      + each segment's length, width, attach point and angle
               "structure" + how many segments and who attaches to whom
    opponents: sumo only, folder of creatures to fight (default: the Rock)
    rules:     rules file (default: arena.toml next to the template)

    Environments (for generalisation; defaults = exactly the rules):
    trials:          evaluate every genome on this many environments
    env_seed:        terrain seed of the first one; trial k uses env_seed + k
    friction_jitter: scale friction by a random factor in [1 - j, 1 + j]
    aggregate:       combine the scores by "mean" or "min" (worst case)
    """

    def __init__(self, template, mode="race", level="brain", opponents=None, rules=None,
                 trials=1, env_seed=None, friction_jitter=0.0, aggregate="mean"):
        self._bin = _find_binary()
        self._args = [str(template), "--mode", mode, "--level", level]
        if opponents:
            self._args += ["--opponents", str(opponents)]
        if rules:
            self._args += ["--rules", str(rules)]
        self._args += _env_args(trials, env_seed, friction_jitter, aggregate)
        self.info = json.loads(self._run("info", "--json"))
        self.dim = self.info["dim"]
        self.genes = [g["name"] for g in self.info["genes"]]
        self.start = [g["start"] for g in self.info["genes"]]
        self.evaluations = 0

    def _run(self, cmd, *extra, stdin=None):
        res = subprocess.run([self._bin, cmd, *self._args, *extra], input=stdin, capture_output=True, text=True)
        if res.returncode != 0:
            raise ValueError(res.stderr.strip())
        return res.stdout

    def evaluate(self, population):
        """Fitness of every genome in `population` (a list of lists of floats in [0, 1])."""
        for k, g in enumerate(population):
            if len(g) != self.dim:
                raise ValueError(f"genome {k} has {len(g)} genes, expected {self.dim}")
        text = "\n".join(",".join(repr(float(x)) for x in g) for g in population)
        out = self._run("batch", stdin=text)
        self.evaluations += len(population)
        return [float(line) for line in out.split()]

    def fight(self, pairs):
        """Sumo duels for co-evolution. `pairs` is a list of (genome_a, genome_b);
        returns the score of a against b for each (+1 win / 0 draw / -1 loss,
        plus ring control). b's score is the negative, so one bout rates both."""
        for k, (a, b) in enumerate(pairs):
            if len(a) != self.dim or len(b) != self.dim:
                raise ValueError(f"pair {k}: genomes must have {self.dim} genes")
        text = "\n".join(",".join(repr(float(x)) for x in a) + " | " + ",".join(repr(float(x)) for x in b)
                         for a, b in pairs)
        out = self._run("fight", stdin=text)
        self.evaluations += len(pairs)
        return [float(line) for line in out.split()]

    def save(self, genome, path, name=None):
        """Write the creature for `genome` to `path`; returns its fitness."""
        extra = ["--genes", ",".join(repr(float(x)) for x in genome), "--out", str(path)]
        if name:
            extra += ["--name", name]
        out = self._run("save", *extra)
        return float(out.split()[1])

    def describe(self, genome):
        """Human-readable gene values, handy for debugging."""
        lines = []
        for g, x in zip(self.info["genes"], genome):
            lines.append(f"{g['name']:<22} {g['lo'] + x * (g['hi'] - g['lo']):8.2f}")
        return "\n".join(lines)


def _env_args(trials, env_seed, friction_jitter, aggregate):
    args = ["--trials", str(trials), "--friction-jitter", str(friction_jitter), "--aggregate", aggregate]
    if env_seed is not None:
        args += ["--env-seed", str(env_seed)]
    return args


def _arena(*args, stdin=None):
    res = subprocess.run([_find_binary(), *args], input=stdin, capture_output=True, text=True)
    if res.returncode != 0:
        raise ValueError(res.stderr.strip())
    return res


def load_creature(path):
    """Read a creature file (.toml or .json) into a dict."""
    return json.loads(_arena("convert", str(path), "-").stdout)


def save_creature(creature, path):
    """Write a creature dict to a .toml (or .json) file for the arena."""
    _arena("convert", "-", str(path), stdin=json.dumps(creature))


class Judge:
    """Scores whole creatures (dicts) instead of genomes.

    mode:      "race" or "sumo"
    opponents: sumo only, folder of creatures to fight (default: the Rock)
    rules:     rules file (default: built-in rules)
    trials, env_seed, friction_jitter, aggregate: see Problem
    """

    def __init__(self, mode="race", opponents=None, rules=None,
                 trials=1, env_seed=None, friction_jitter=0.0, aggregate="mean"):
        self._args = ["--mode", mode, *_env_args(trials, env_seed, friction_jitter, aggregate)]
        if opponents:
            self._args += ["--opponents", str(opponents)]
        self._rules_args = [str(rules)] if rules else []
        if rules:
            self._args += ["--rules", str(rules)]
        self.rules = json.loads(_arena("rules", *self._rules_args).stdout)
        self.errors = []
        self.evaluations = 0

    def evaluate(self, creatures):
        """Fitness of each creature, evaluated in parallel. Creatures that break
        the rules score -inf; `self.errors` lists why."""
        text = "\n".join(json.dumps(c) for c in creatures)
        res = _arena("judge", *self._args, stdin=text)
        self.errors = [line for line in res.stderr.splitlines() if line.strip()]
        self.evaluations += len(creatures)
        return [float(x) for x in res.stdout.split()]

    def fight(self, pairs):
        """Sumo duels between creature dicts: score of a against b for each (a, b)."""
        text = "\n".join(json.dumps([a, b]) for a, b in pairs)
        res = _arena("judge", "--pairs", *self._args, stdin=text)
        self.errors = [line for line in res.stderr.splitlines() if line.strip()]
        self.evaluations += len(pairs)
        return [float(x) for x in res.stdout.split()]
