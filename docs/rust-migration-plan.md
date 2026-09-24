# CPGanimal: Plan for porting from MATLAB to Rust

> Status: draft, pending discussion
> Scope: rewrite the Snake5 / chain_CPG simulations in Rust and expose a clean interface for genetic-algorithm (and other black-box optimizer) training.

---

## 0. First principles: what is the actual goal?

The MATLAB project does one thing:

$$
\text{params} \xrightarrow{\text{CPG}} \theta_i(t) \xrightarrow{\text{multibody + ground contact}} \text{trajectory} \xrightarrow{\text{metric}} \text{fitness}
$$

and then uses `ga()` to search the parameter space for the gait that goes furthest.

Where "the platform is too restrictive" actually hurts:

| Pain point | Cause | What the Rust version should give us |
|---|---|---|
| Slow evaluation | Simscape is a variable-step DAE solver plus the `sim()` startup cost | Single-threaded, one episode ≪ 1 s; parallel with `rayon` |
| Hard to parallelize | `parsim` needs a license plus worker overhead | `Evaluator: Sync`, run a whole population in parallel |
| Algorithm locked to `ga()` | Toolbox interface | Separate the **evaluation function** from the **optimizer** and swap freely (GA / CMA-ES / Python) |
| Hard to extend (feedback, morphology, sim2real) | Simulink graphical models are not version-control friendly | Plain code plus TOML config, diffable and testable |

**Conclusion: the core deliverable is a pure function `fn evaluate(genome) -> Fitness` that is deterministic, fast, and callable in parallel.** Visualization and the GA itself are secondary.

---

## 1. Inventory of the existing model (reverse-engineered from the `.slx` files)

### 1.1 CPG (MATLAB Function block inside `Snake5.slx`)

State per oscillator $i$: phase $\psi_i$, amplitude $r_i$, offset $x_i$.

```matlab
dpsi(i) = omega(i) + Σ_j omegaM(i,j) * r(j) * sin(psi(j) - psi(i) - phiM(i,j))
ddr(i)  = ar * (ar/4 * (R(i) - r(i) - dr(i)))
ddx(i)  = ax * (ax/4 * (X(i) - x(i) - dx(i)))
theta(i)= x(i) + r(i) * cos(psi(i))
```

Parameters: `N=5`, `omega=0.6π`, `omegaM=2` (all-to-all), `phiM=PHI` (every entry the same), `ar=ax=2`, `X=0`.

### ⚠️ Two issues found in the MATLAB code (confirm before porting)

1. **The coupling term overwrites instead of accumulating**:
   ```matlab
   for j=1:N
       dpsi(i) = omegaM(i,j)*r(j)*sin(...);   % should be dpsi(i) = dpsi(i) + ...
   end
   ```
   In effect each oscillator is coupled **only to oscillator 5**, not all-to-all.
   So the "optimal PHI" the GA found was found under this topology.
2. **Amplitude damping differs from the paper (Sproewitz 2008)**:
   - Paper: $\ddot r = a_r\left(\frac{a_r}{4}(R-r) - \dot r\right)$ → critically damped
   - Code: $\ddot r = \frac{a_r^2}{4}(R - r - \dot r)$ → with $a_r=2$ this is $\ddot r = R - r - \dot r$, **underdamped** (damping ratio 0.5), so the amplitude overshoots.

   The same applies to $x$.

Also note: `phiM = ones(N,N)*PHI` means $\phi_{ij}=\phi_{ji}=\text{PHI}$, which is not antisymmetric, so a stable phase-locked state is not guaranteed. The usual convention is $\phi_{ij} = -\phi_{ji}$, with chain-neighbor coupling.

### 1.2 Mechanical model (Simscape Multibody, `Snake5.slx`)

| Item | Value |
|---|---|
| Links | 5 × Brick, `[0.7 0.7 1]*l`, `l=1 m`, density 1000 → **490 kg per link** |
| Joints | 4 × Revolute, **InputMotion** (position prescribed by the CPG, torque computed) |
| Base | 6-DOF joint (floating base) |
| Contact | Each brick ↔ infinite plane, SmoothSpringDamper: k=1e5, c=1e3, μs=0.5, μd=0.3, v_crit=1e-3 |
| Joint frames | Rigid Transforms rotated 88° / 92° about ±X, so neighboring links tilt ±2°, a slight V-shaped zigzag (**intentional?**) |
| Gravity | -9.80665 z |
| Solver | Variable step, RelTol 1e-3, 10 s |
| Output | `distance = sqrt(Px² + Py²)` (horizontal displacement of the base), fitness = `-distance` |

Only 5 of the 6 CPG outputs are used: `theta(1..4)` drive the 4 joints (to be confirmed from the wiring).

### 1.3 Optimization problem

| Model | Decision variables | Bounds |
|---|---|---|
| `chain_CPG` (3 modules) | R1, R2, R3, PHI | [0,0.6]³ × [0,π] |
| `Snake5` (5 modules) | R1..R5, PHI | [0,0.6]⁵ × [0,π] |

The GA uses MATLAB defaults (population 50 for ≤5 variables, 200 otherwise; crossover 0.8; Gaussian mutation; 2 elites).

---

## 2. Key technical choice: the physics engine

This is the only non-trivial decision in the whole port. The CPG itself is just a few lines of ODE.

| Option | Pros | Cons | Fit |
|---|---|---|---|
| **Rapier 3D** (pure Rust) | No C dependency, `cargo build` just works; has multibody (reduced-coordinate) joints and motors; deterministic (`enhanced-determinism`); easy to parallelize | Contact is velocity-level impulses (not the Simscape penalty model); less proven for locomotion research than MuJoCo; prescribed position motion has to be approximated with a stiff PD motor | ★★★ the default |
| **MuJoCo** (via `mujoco-rs` or your own FFI) | De facto standard for locomotion; soft contact is tunable; position actuators; excellent performance; MJCF describes the robot | C library dependency, binding maintenance cost; FFI hurts Rust purity somewhat | ★★★ the research-grade choice |
| Hand-written (penalty contact + generalized coordinates) | Can reproduce the Simscape contact model exactly, full control | Articulated-body dynamics and a stiff-system integrator are real work that doesn't serve the goal | ★ |

**Recommendation: define a `trait PhysicsBackend` and implement Rapier first.** Put MuJoCo behind a cargo feature and add it as needed. That way the CPG, evaluation, and GA layers never depend on the engine choice, and you can later do cross-engine consistency checks (a nice sim2real robustness experiment in its own right).

> A shorter path worth knowing about: Python + MuJoCo + an off-the-shelf evolutionary library (pymoo / evotorch) could reproduce this in a day or two.
> The Rust version pays off in long-term maintainability, performance, embedded reuse (the CPG crate is `no_std` and can go straight onto an MCU), and fit with your stack.
> The two don't conflict: section 4.4 provides PyO3 bindings, so you can still call the Rust simulator from Python.

### About "the same"

Simscape's contact model differs from Rapier's and MuJoCo's, so **the numeric distance will never match exactly**. A reasonable acceptance standard:

1. **CPG layer: numerically equal** (same ODE, error vs. MATLAB-exported trajectories < 1e-6)
2. **Mechanical layer: qualitatively equal** (same morphology and parameters, forward rather than backward motion, similar ordering and landscape shape of the optimal PHI)

---

## 3. Architecture (Cargo workspace)

```
cpganimal/
├── Cargo.toml                # workspace
├── crates/
│   ├── cpg/                  # no_std-friendly, depends only on nalgebra/libm
│   │   ├── oscillator.rs     # Sproewitz amplitude-controlled phase oscillator
│   │   ├── network.rs        # N oscillators + coupling topology (chain / all-to-all / custom matrix)
│   │   └── integrate.rs      # RK4 / fixed step, pure functions
│   ├── body/                 # robot description (engine-agnostic)
│   │   └── morphology.rs     # ChainRobot { links, joint_axes, dims, density, contact }
│   ├── sim/                  # couples CPG and physics
│   │   ├── backend.rs        # trait PhysicsBackend
│   │   ├── rapier.rs         # feature = "rapier"
│   │   ├── mujoco.rs         # feature = "mujoco" (later)
│   │   ├── episode.rs        # run one episode: dt, duration, recording
│   │   └── metrics.rs        # distance, energy (Σ|τ·ω|dt), COT, straightness, fall detection
│   ├── evo/                  # optimization interface + built-in algorithms
│   │   ├── space.rs          # ParamSpace: genome ⇄ physical parameters, bounds, normalization
│   │   ├── evaluator.rs      # trait Evaluator
│   │   ├── ga.rs             # real-coded GA (SBX/BLX-α, polynomial mutation, tournament, elitism)
│   │   └── cmaes.rs          # (optional) reference baseline
│   ├── cli/                  # `cpganimal simulate|train|replay|sweep`
│   └── py/                   # (optional) PyO3 bindings
├── configs/
│   ├── snake5.toml
│   └── chain3.toml
└── matlab/                   # keep the original MATLAB project as the reference implementation
```

Dependency direction: `cpg ← sim → body`, `evo → (Evaluator trait)`, `cli → all`.
**`evo` does not depend on `sim`.** The GA knows nothing about robots; it only knows `&[f64] -> Fitness`.

---

## 4. The GA training interface (the core)

### 4.1 Parameter space: decouple the genome from physical parameters

```rust
/// A named, bounded parameter. Genes are always in [0,1] so every optimizer is scale-agnostic.
pub struct ParamSpec {
    pub name: String,        // "R[0]", "phi", "omega", ...
    pub lo: f64,
    pub hi: f64,
    pub scale: Scale,        // Linear | Log | Angle (wrap-around)
}

pub struct ParamSpace { specs: Vec<ParamSpec> }

impl ParamSpace {
    pub fn dim(&self) -> usize;
    pub fn decode(&self, genome: &[f64]) -> Vec<f64>;       // [0,1]^n -> physical values
    pub fn encode(&self, values: &[f64]) -> Vec<f64>;
}
```

Which parameters to optimize is chosen **declaratively in config** (not by changing function signatures the way `animal(R1,R2,R3,PHI)` does):

```toml
[optimize]
R     = { per_joint = true, lo = 0.0, hi = 0.6 }
phi   = { shared = true,    lo = 0.0, hi = 3.14159 }
# omega = { shared = true, lo = 0.2, hi = 3.0 }   # uncomment to add a dimension
```

### 4.2 The evaluator

```rust
pub struct Fitness {
    pub objectives: SmallVec<[f64; 4]>, // maximized by convention; single objective = [distance]
    pub constraint_violation: f64,      // 0 = feasible (falls, joint limits, NaN, ...)
    pub info: EpisodeSummary,           // distance, energy, cot, heading, sim_time ...
}

pub trait Evaluator: Sync {
    fn space(&self) -> &ParamSpace;
    fn evaluate(&self, genome: &[f64], seed: u64) -> Fitness;

    /// Parallel by default; override for GPU/remote batching.
    fn evaluate_batch(&self, pop: &[Vec<f64>], seed: u64) -> Vec<Fitness> {
        pop.par_iter().enumerate()
            .map(|(k, g)| self.evaluate(g, seed ^ k as u64))
            .collect()
    }
}
```

Design notes:
- **`seed` is explicit**: needed later for domain randomization (friction, mass, initial phase noise) and for fair comparisons.
- **Multi-objective slot**: distance vs. energy is the classic trade-off, so NSGA-II can plug in directly.
- **`constraint_violation`**: handles falls, divergence, and NaN explicitly instead of hiding them behind a large penalty.
- **No exceptions**: a physics blow-up returns `violation = inf`, so one bad individual never takes down the population.

### 4.3 Optimizer interface (ask/tell style, compatible with any algorithm)

```rust
pub trait Optimizer {
    fn ask(&mut self) -> Vec<Vec<f64>>;                 // genomes in [0,1]^n
    fn tell(&mut self, genomes: &[Vec<f64>], fits: &[Fitness]);
    fn best(&self) -> Option<(&[f64], &Fitness)>;
}

pub fn run<O: Optimizer, E: Evaluator>(opt: &mut O, eval: &E, stop: StopCriteria,
                                       log: &mut dyn Logger) -> RunResult;
```

ask/tell keeps "who drives the loop" out of the algorithm, so it works equally well for:
- the built-in GA / CMA-ES
- external libraries (wrap them with an adapter)
- driving from Python (pymoo, evotorch, Nevergrad)
- future distributed evaluation (ask → send to workers → tell)

### 4.4 Python bindings (optional, M4)

```python
import cpganimal
env = cpganimal.Evaluator.from_toml("configs/snake5.toml")
env.dim, env.bounds
fits = env.evaluate_batch(np.random.rand(64, env.dim), seed=0)  # releases the GIL, rayon parallel
```

### 4.5 Logging and reproducibility

Each run writes `runs/<timestamp>/` containing:
- `config.toml` (full snapshot) + `git rev` + seed
- `generations.csv` (best/mean/std per generation, **for plotting convergence curves in Obsidian or Python**)
- `best.json` (best genome + decoded physical parameters)
- `cpganimal replay runs/.../best.json` for visualization

---

## 5. Visualization

Not on the training critical path, so it's a separate cargo feature:
- **Rerun** (`rerun` crate) is recommended: log link poses and joint angle/CPG curves per step, and you get a 3D scene plus time-series plots with almost no code
- Alternative: Bevy + bevy_rapier (more interactive, more code)

---

## 6. Milestones

| # | Content | Acceptance |
|---|---|---|
| **M0** | Workspace skeleton + `cpg` crate (RK4, chain/all-to-all/custom topology, `compat_matlab` switch) | Unit tests: phase locking, amplitude convergence; **matches MATLAB-exported trajectories** (needs your export) |
| **M1** | `body` + `sim` (Rapier): Snake5 morphology, prescribed-position joints, ground contact; Rerun replay | Visual check: the snake wiggles, doesn't sink into the ground, doesn't blow up; single-episode time < 100 ms |
| **M2** | `evo`: ParamSpace, Evaluator, real-coded GA, rayon parallelism, CLI `train` | Reproduces the 6-parameter Snake5 optimization; convergence curve; landscape sweep over PHI |
| **M3** | Config-driven morphology (chain3 / snake5 / arbitrary N), metric extensions (energy, COT) | `chain3.toml` reproduces the `chain_CPG` experiment |
| **M4** (optional) | MuJoCo backend, CMA-ES, NSGA-II, PyO3 | Cross-engine consistency comparison |

M0–M2 are the minimum closed loop. After M2 you already have a usable "Rust version + GA training".

---

## 7. Open questions (need your decision)

1. **How to handle the MATLAB bugs?**
   - (a) Reproduce them faithfully (overwritten coupling, underdamped amplitude) for comparison with old results
   - (b) Implement per the paper, keeping (a) as `compat_matlab = true`. **← recommended**
2. **Physics engine**: start with Rapier (pure Rust), or go straight to MuJoCo (the research standard, but with a C dependency)?
3. **Can you still run MATLAB to export reference data?** (CPG trajectories `psi, r, theta` + one set of `distance(t)`). If not, M0 validates against analytic properties only.
4. **What comes after?** This affects how far ahead the architecture should look:
   - Parameter search only → the plan above is enough
   - Add **sensory feedback** (closed-loop CPG, e.g. contact/body-shape feedback) → the `cpg` crate needs an external input port reserved
   - **Morphology co-optimization** → morphology also goes into `ParamSpace`
   - **Deploy to real hardware** → `cpg` stays `no_std`, and dt/quantization need to match the hardware
5. **Is the 88°/92° joint tilt intentional?** (Is it to create friction anisotropy? Real snake locomotion depends heavily on **anisotropic friction**. With isotropic Coulomb friction, forward propulsion from a serpentine gait is weak. This is a physical factor that deserves to be modeled explicitly.)
6. **Link size/mass**: 1 m × 490 kg per link is far from a typical modular robot (the paper's YaMoR modules are about 10 cm / 250 g). Keep it as is, or switch to a realistic scale?
