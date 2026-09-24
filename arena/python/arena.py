"""Python interface to CPG Arena (standard library only).

    from arena import Problem

    p = Problem("creatures/worm.toml", mode="race")
    p.dim                          # number of genes; a genome is p.dim floats in [0, 1]
    p.start                        # the template creature as a genome
    p.evaluate(population)         # list of genomes -> list of fitness (higher is better)
    p.save(best, "me.toml", name="My Creature")

Each `evaluate` call runs the whole population in parallel in one process,
so call it once per generation rather than once per individual.
"""

import json
import os
import shutil
import subprocess
from pathlib import Path

__all__ = ["Problem"]


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
    body:      also let genes change segment sizes, attach points and angles
    opponents: sumo only, folder of creatures to fight (default: the Rock)
    rules:     rules file (default: arena.toml next to the template)
    """

    def __init__(self, template, mode="race", body=False, opponents=None, rules=None):
        self._bin = _find_binary()
        self._args = [str(template), "--mode", mode]
        if body:
            self._args.append("--body")
        if opponents:
            self._args += ["--opponents", str(opponents)]
        if rules:
            self._args += ["--rules", str(rules)]
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
