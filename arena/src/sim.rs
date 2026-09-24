//! 2D side-view physics: creatures made of boxes, joints driven by the CPG.

use crate::cpg::{Cpg, OscParams};
use crate::creature::{Creature, Rules};
use rapier2d::prelude::*;
use std::f64::consts::PI;

const GROUND: Group = Group::GROUP_1;
const TEAM: [Group; 2] = [Group::GROUP_2, Group::GROUP_3];
/// Hard cap on how far a joint may bend from rest (deg).
const MAX_JOINT: f64 = 170.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Endless flat ground, one creature, go right.
    Race,
    /// Two creatures on a floating platform; push the other one off.
    Sumo,
}

/// One creature living in a world.
pub struct Fighter {
    pub name: String,
    pub bodies: Vec<RigidBodyHandle>,
    pub half: Vec<[f32; 2]>,
    joints: Vec<ImpulseJointHandle>,
    limits: Vec<[f64; 2]>,
    /// −1 when mirrored (facing left).
    sign: f64,
    cpg: Cpg,
    pub start_x: f64,
}

/// A segment ready to draw.
#[derive(Clone, Copy, Debug)]
pub struct SegView {
    pub fighter: usize,
    pub center: [f32; 2],
    pub angle: f32,
    pub half: [f32; 2],
}

pub struct Arena {
    pub world: PhysicsWorld,
    pub fighters: Vec<Fighter>,
    pub stage: Stage,
    pub rules: Rules,
    pub time: f64,
    /// Static ground boxes (center, half extents) for drawing.
    pub ground: Vec<([f32; 2], [f32; 2])>,
    /// Race terrain surface (x, y) for drawing; empty when flat.
    pub terrain: Vec<[f32; 2]>,
}

#[derive(Clone, Debug)]
pub struct SumoResult {
    /// Index of the winner (0 or 1), None for a draw.
    pub winner: Option<usize>,
    pub time: f64,
    pub reason: String,
    /// In [-1, 1], positive when fighter 0 controls the ring. Useful as a
    /// smooth fitness signal for the GA.
    pub margin: f64,
}

impl Arena {
    fn empty(stage: Stage, rules: &Rules) -> Self {
        let mut world = PhysicsWorld::new();
        world.integration_parameters.dt = rules.dt as f32;
        Self { world, fighters: Vec::new(), stage, rules: rules.clone(), time: 0.0, ground: Vec::new(), terrain: Vec::new() }
    }

    fn add_ground(&mut self, center: [f32; 2], half: [f32; 2]) {
        let h = self.world.bodies.insert(RigidBodyBuilder::fixed().translation(Vec2::new(center[0], center[1])));
        let col = ColliderBuilder::cuboid(half[0], half[1])
            .friction(self.rules.friction as f32)
            .collision_groups(InteractionGroups::new(GROUND, Group::ALL, InteractionTestMode::And));
        self.world.colliders.insert_with_parent(col, h, &mut self.world.bodies);
        self.ground.push((center, half));
    }

    pub fn race(c: &Creature, rules: &Rules) -> Self {
        let mut a = Self::empty(Stage::Race, rules);
        // Slope: tilt gravity instead of the ground (x stays "along the track").
        let s = rules.slope.to_radians() as f32;
        a.world.gravity = Vec2::new(-9.81 * s.sin(), -9.81 * s.cos());
        a.add_ground([0.0, -0.5], [5000.0, 0.5]);
        if rules.terrain_roughness > 0.0 {
            a.add_terrain(rules.terrain_roughness, rules.terrain_seed);
        }
        a.spawn(c, 0, 0.0, false);
        a
    }

    /// Smooth random hills from x = 1 m on (the start area stays flat).
    /// Heights are ≥ 0, so the flat ground box underneath never pokes out.
    fn add_terrain(&mut self, roughness: f64, seed: u64) {
        const X0: f64 = -20.0;
        const X1: f64 = 300.0;
        const STEP: f64 = 0.1;
        let n = ((X1 - X0) / STEP) as usize + 1;
        let heights: Vec<f32> =
            (0..n).map(|i| terrain_height(X0 + i as f64 * STEP, roughness, seed) as f32).collect();
        let body = self.world.bodies.insert(RigidBodyBuilder::fixed().translation(Vec2::new(((X0 + X1) / 2.0) as f32, 0.0)));
        let col = ColliderBuilder::heightfield(heights.clone(), Vec2::new((X1 - X0) as f32, 1.0))
            .friction(self.rules.friction as f32)
            .collision_groups(InteractionGroups::new(GROUND, Group::ALL, InteractionTestMode::And));
        self.world.colliders.insert_with_parent(col, body, &mut self.world.bodies);
        self.terrain = heights.iter().enumerate().map(|(i, &h)| [(X0 + i as f64 * STEP) as f32, h]).collect();
    }

    pub fn sumo(c0: &Creature, c1: &Creature, rules: &Rules) -> Self {
        let mut a = Self::empty(Stage::Sumo, rules);
        a.add_ground([0.0, -0.5], [rules.ring_width as f32 / 2.0, 0.5]);
        a.spawn(c0, 0, -rules.sumo_start, false);
        a.spawn(c1, 1, rules.sumo_start, true);
        a
    }

    /// Build a creature with its centre of area at `x`, resting just above y = 0.
    fn spawn(&mut self, c: &Creature, team: usize, x: f64, mirror: bool) {
        let rules = &self.rules;
        // Forward kinematics of the rest pose: (center, angle) per segment.
        let mut pose: Vec<([f64; 2], f64)> = Vec::with_capacity(c.segments.len());
        for (i, s) in c.segments.iter().enumerate() {
            if i == 0 {
                pose.push(([0.0, 0.0], 0.0));
                continue;
            }
            let p = &c.segments[s.parent];
            let (pc, pa) = pose[s.parent];
            let d = s.attach * p.length / 2.0;
            let joint = [pc[0] + d * pa.cos(), pc[1] + d * pa.sin()];
            let a = pa + s.angle.to_radians();
            let h = s.length / 2.0;
            pose.push(([joint[0] + h * a.cos(), joint[1] + h * a.sin()], a));
        }
        let sign = if mirror { -1.0 } else { 1.0 };
        if mirror {
            for (p, a) in &mut pose {
                p[0] = -p[0];
                *a = PI - *a;
            }
        }
        // Centre of area and lowest corner.
        let area: f64 = c.area();
        let cx = pose.iter().zip(&c.segments).map(|((p, _), s)| p[0] * s.length * s.width).sum::<f64>() / area;
        let mut min_y = f64::INFINITY;
        for ((p, a), s) in pose.iter().zip(&c.segments) {
            for (u, v) in [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
                let (lx, ly) = (u * s.length / 2.0, v * s.width / 2.0);
                min_y = min_y.min(p[1] + lx * a.sin() + ly * a.cos());
            }
        }
        let dx = x - cx;
        let dy = 0.02 - min_y;

        let other = TEAM[1 - team];
        let groups = InteractionGroups::new(TEAM[team], GROUND | other, InteractionTestMode::And);
        let mut bodies = Vec::new();
        let mut half = Vec::new();
        for ((p, a), s) in pose.iter().zip(&c.segments) {
            let rb = RigidBodyBuilder::dynamic()
                .translation(Vec2::new((p[0] + dx) as f32, (p[1] + dy) as f32))
                .rotation(*a as f32)
                .can_sleep(false);
            let h = self.world.bodies.insert(rb);
            let hx = s.length as f32 / 2.0;
            let hy = s.width as f32 / 2.0;
            let col = ColliderBuilder::cuboid(hx, hy)
                .density(rules.density as f32)
                .friction(rules.friction as f32)
                .collision_groups(groups);
            self.world.colliders.insert_with_parent(col, h, &mut self.world.bodies);
            bodies.push(h);
            half.push([hx, hy]);
        }

        let mut joints = Vec::new();
        let mut limits = Vec::new();
        let mut osc = Vec::new();
        // The rest angle lives in the parent-side joint frame, so the joint
        // angle is the swing around rest and never wraps around ±180°.
        let range = rules.joint_range.min(MAX_JOINT).to_radians();
        for (i, s) in c.segments.iter().enumerate().skip(1) {
            let p = &c.segments[s.parent];
            let rest = sign * s.angle.to_radians();
            let anchor1 = Vec2::new((s.attach * p.length / 2.0) as f32, 0.0);
            let anchor2 = Vec2::new(-(s.length / 2.0) as f32, 0.0);
            let mut j: GenericJoint = RevoluteJointBuilder::new()
                .contacts_enabled(false)
                .limits([-range as f32, range as f32])
                .motor_model(MotorModel::ForceBased)
                .motor_position(0.0, rules.motor_stiffness as f32, rules.motor_damping as f32)
                .motor_max_force(rules.motor_torque as f32)
                .into();
            j.set_local_frame1(Pose::new(anchor1, rest as f32));
            j.set_local_frame2(Pose::new(anchor2, 0.0));
            joints.push(self.world.impulse_joints.insert(bodies[s.parent], bodies[i], j, true));
            limits.push([-range, range]);
            osc.push(OscParams {
                amplitude: s.amplitude.to_radians(),
                offset: s.offset.to_radians(),
                phase: s.phase.to_radians(),
            });
        }
        self.fighters.push(Fighter {
            name: c.name.clone(),
            bodies,
            half,
            joints,
            limits,
            sign,
            cpg: Cpg::new(c.brain.frequency, c.brain.coupling, osc),
            start_x: x,
        });
    }

    /// Advance CPGs and physics by one `rules.dt`.
    pub fn step(&mut self) {
        let dt = self.rules.dt;
        for f in &mut self.fighters {
            f.cpg.step(dt);
            for (k, &jh) in f.joints.iter().enumerate() {
                let cmd = f.cpg.output(k);
                let target = (f.sign * cmd.angle).clamp(f.limits[k][0], f.limits[k][1]);
                if let Some(j) = self.world.impulse_joints.get_mut(jh, false) {
                    j.data.set_motor(
                        JointAxis::AngX,
                        target as f32,
                        (f.sign * cmd.velocity) as f32,
                        self.rules.motor_stiffness as f32,
                        self.rules.motor_damping as f32,
                    );
                }
            }
        }
        self.world.step();
        self.time += dt;
    }

    /// Centre of mass of fighter `k`.
    pub fn com(&self, k: usize) -> [f64; 2] {
        let mut m = 0.0;
        let mut c = [0.0; 2];
        for &h in &self.fighters[k].bodies {
            let b = &self.world.bodies[h];
            let p = b.center_of_mass();
            m += b.mass() as f64;
            c[0] += b.mass() as f64 * p.x as f64;
            c[1] += b.mass() as f64 * p.y as f64;
        }
        [c[0] / m, c[1] / m]
    }

    /// Distance travelled to the right in a race.
    pub fn progress(&self, k: usize) -> f64 {
        self.com(k)[0] - self.fighters[k].start_x
    }

    pub fn is_broken(&self) -> bool {
        (0..self.fighters.len()).any(|k| {
            let c = self.com(k);
            !c[0].is_finite() || !c[1].is_finite() || c[1].abs() > 1e3
        })
    }

    pub fn segments(&self) -> Vec<SegView> {
        let mut out = Vec::new();
        for (k, f) in self.fighters.iter().enumerate() {
            for (h, half) in f.bodies.iter().zip(&f.half) {
                let b = &self.world.bodies[*h];
                let t = b.translation();
                out.push(SegView { fighter: k, center: [t.x, t.y], angle: b.rotation().angle(), half: *half });
            }
        }
        out
    }

    fn fallen(&self, k: usize) -> bool {
        self.com(k)[1] < -0.5
    }

    /// Current sumo standing; `Some` once the bout is decided or time is up.
    pub fn sumo_status(&self) -> Option<SumoResult> {
        let half = self.rules.ring_width / 2.0;
        let (f0, f1) = (self.fallen(0), self.fallen(1));
        let res = |winner, reason: &str, margin| SumoResult { winner, time: self.time, reason: reason.into(), margin };
        if self.is_broken() {
            return Some(res(None, "physics blew up", 0.0));
        }
        match (f0, f1) {
            (true, true) => return Some(res(None, "both fell off", 0.0)),
            (true, false) => return Some(res(Some(1), "pushed out", -1.0)),
            (false, true) => return Some(res(Some(0), "pushed out", 1.0)),
            _ => {}
        }
        if self.time + 1e-9 < self.rules.sumo_time {
            return None;
        }
        // Time up: whoever is closer to the centre controls the ring.
        let d0 = self.com(0)[0].abs().min(half);
        let d1 = self.com(1)[0].abs().min(half);
        let margin = ((d1 - d0) / half).clamp(-1.0, 1.0) * 0.5;
        let winner = if margin > 0.05 {
            Some(0)
        } else if margin < -0.05 {
            Some(1)
        } else {
            None
        };
        Some(res(winner, "time up, ring control", margin))
    }
}

/// Deterministic pseudo-random number in [0, 1) from an integer and a seed.
pub fn hash01(i: i64, seed: u64) -> f64 {
    let mut z = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ seed.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
}

/// Terrain height at `x`: two octaves of smooth value noise, faded in over
/// x ∈ [1, 3] m so every creature starts on flat ground.
pub fn terrain_height(x: f64, roughness: f64, seed: u64) -> f64 {
    fn noise(x: f64, seed: u64) -> f64 {
        let i = x.floor();
        let t = x - i;
        let t = t * t * (3.0 - 2.0 * t);
        let (a, b) = (hash01(i as i64, seed), hash01(i as i64 + 1, seed));
        a + (b - a) * t
    }
    let fade = ((x - 1.0) / 2.0).clamp(0.0, 1.0);
    let h = 0.7 * noise(x / 1.2, seed) + 0.3 * noise(x / 0.35, seed ^ 0xA5A5);
    roughness * fade * h
}

/// Headless race: metres travelled to the right.
pub fn run_race(c: &Creature, rules: &Rules) -> f64 {
    let mut a = Arena::race(c, rules);
    let steps = (rules.race_time / rules.dt).round() as usize;
    for _ in 0..steps {
        a.step();
    }
    if a.is_broken() { f64::NEG_INFINITY } else { a.progress(0) }
}

/// Headless sumo bout.
pub fn run_sumo(c0: &Creature, c1: &Creature, rules: &Rules) -> SumoResult {
    let mut a = Arena::sumo(c0, c1, rules);
    loop {
        a.step();
        if let Some(r) = a.sumo_status() {
            return r;
        }
    }
}
