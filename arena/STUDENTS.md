# CPG Arena

Design a 2D creature, evolve how it moves (and even its body) with a genetic algorithm, then race it or fight it in sumo against your classmates' creatures.

## What you need

- Windows 10 or 11.
- Python 3.8 or newer, for training: <https://www.python.org/downloads/>. When installing, tick **"Add python.exe to PATH"**. Designing, racing and sumo work without Python. The Train tab finds Python by itself and shows its version next to the **Python** field.

## Start

Unzip the folder anywhere and double-click **`arena-gui.exe`**. If Windows says it protected your PC from an unknown app, click **More info**, then **Run anyway**.

| In this folder | |
|---|---|
| `arena-gui.exe` | the program |
| `arena.exe` | the simulator; the Python scripts call it |
| `creatures/` | example creatures. Your designs and trained creatures are saved here |
| `python/` | the genetic algorithms: `ga.py` (ready to use), `my_ga.py` (write your own), and more |
| `GUIDE.md` | the full guide: the creature file format, the Python interface, the rules and scoring. (Its "Quick start" is about building from source; you can skip that.) |

## 1. Design a body: the Design tab

Give your creature a name, then drag blocks (Leg, Short leg, Block, Plate) from the right onto the creature. Click a block to change its size, angle and motor; drag a block to turn it. The panel tells you when you break a rule (for example the body area budget). **Try it** shows how it moves; **Save** stores it in `creatures/`.

## 2. Evolve it: the Train tab

Pick an algorithm, your creature as the template, race or sumo, and press **Train**. Tick **Watch the population** to see every generation move, from random flailing to a gait. Each champion is saved in `creatures/` as a new creature with the time in its name, so you can race your runs against each other.

- The **Fitness** tab compares runs: try your GA, the default GA and random search with the same budget.
- The **DNA** tab shows what the GA really works with: a list of numbers between 0 and 1. The GA is the same at every level; only what the numbers are decoded into changes (movement at `brain`, also the body at `body` and `structure`).
- To evolve the shape of *your* design, use the `body` level, or the **Structure evolution** algorithm (the `structure` level soon forgets the template).
- **Training environments**: train on several terrains (and frictions) at once, so your creature does not only learn the one track it trained on.

## Hills and slopes: environment seeds

The Race and Train tabs run on **flat** ground (or on the **class track**, when the open folder has your teacher's `arena.toml`), or on an **environment seed**: one number (0 to 9999) that gives the hills, the terrain, the slope and the friction. The same seed always gives the same track, and the tab shows it before you start. If your teacher gives you a class `arena.toml` or an environment seed, train with it, so your creature is ready for the competition track.

## 3. Write your own GA

Open `python/my_ga.py` in an editor (for example VS Code or IDLE) and fill in `select`, `crossover` and `mutate`. Run it from the Train tab (Algorithm: **My GA**), or from a terminal in this folder:

```
python python/my_ga.py creatures/worm.toml --out creatures/me.toml --name "My Worm" --budget 1000
```

(If `python` does not start Python in the terminal, use what the Train tab shows in its **Python** field instead, for example `py -3`.)

Your GA should beat **Random search** with the same budget. A copy of `my_ga.py` with your own name can be run with **Custom script…** in the Train tab.

## 4. Compete

The **Race** and **Sumo** tabs run your creatures against each other. In the Race tab you can also race on an **environment seed**: does your creature still win on a track it has never seen? For sumo, an environment seed puts hills on the platform (the same on both sides). To hand in, copy your creature's `.toml` file from `creatures/` to the class folder your teacher gives you.

## Problems

- **The Train tab says "Python 3 not found"**: install Python (see above), then press **find** next to the **Python** field. If Python is installed somewhere unusual, type the path to its `python.exe` in that field (or a command that starts it, for example `py -3`).
- **A creature is red in the list**: it breaks the rules; the reason is shown under its name.
