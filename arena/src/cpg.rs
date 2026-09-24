//! Central pattern generator: a chain of amplitude-controlled phase oscillators.
//!
//! Model from Sproewitz et al. 2008, "Learning to move in modular robots using
//! central pattern generators and online optimization":
//!
//! ```text
//! dψ_i/dt = 2πf + Σ_j w r_j sin(ψ_j − ψ_i − φ_ij)
//! d²r_i/dt² = a (a/4 (R_i − r_i) − dr_i/dt)
//! d²x_i/dt² = a (a/4 (X_i − x_i) − dx_i/dt)
//! θ_i = x_i + r_i cos(ψ_i)
//! ```
//!
//! Oscillators are coupled to their chain neighbours (i−1, i+1) and the
//! desired phase lag is `φ_ij = phase_j − phase_i`, so the network converges to
//! the phase pattern written in the creature file.
//!
//! Note: the original MATLAB `oscillator` block overwrote `dpsi(i)` inside the
//! coupling loop (so only the last oscillator was coupled) and used
//! `ar*(ar/4*(R-r-dr))`, which is under-damped. Both are fixed here.

use std::f64::consts::TAU;

/// Convergence gain of amplitude and offset (the paper's a_r = a_x).
pub const GAIN: f64 = 4.0;

/// Target of one oscillator.
#[derive(Clone, Copy, Debug, Default)]
pub struct OscParams {
    /// Amplitude R (rad).
    pub amplitude: f64,
    /// Offset X (rad).
    pub offset: f64,
    /// Desired absolute phase (rad); only differences between neighbours matter.
    pub phase: f64,
}

#[derive(Clone, Copy, Debug, Default)]
struct OscState {
    psi: f64,
    r: f64,
    dr: f64,
    x: f64,
    dx: f64,
}

/// Joint command produced by the CPG for one step.
#[derive(Clone, Copy, Debug, Default)]
pub struct JointCmd {
    pub angle: f64,
    pub velocity: f64,
}

#[derive(Clone, Debug)]
pub struct Cpg {
    pub frequency: f64,
    pub coupling: f64,
    params: Vec<OscParams>,
    state: Vec<OscState>,
}

impl Cpg {
    /// All oscillators start at rest (ψ = r = x = 0), so the gait fades in
    /// smoothly and the phase pattern emerges through coupling.
    pub fn new(frequency: f64, coupling: f64, params: Vec<OscParams>) -> Self {
        let state = vec![OscState::default(); params.len()];
        Self { frequency, coupling, params, state }
    }

    pub fn len(&self) -> usize {
        self.params.len()
    }

    pub fn is_empty(&self) -> bool {
        self.params.is_empty()
    }

    /// dψ_i/dt: intrinsic frequency plus coupling to chain neighbours.
    fn dpsi(&self, s: &[OscState], i: usize) -> f64 {
        let mut dpsi = TAU * self.frequency;
        for j in [i.wrapping_sub(1), i + 1] {
            if j < s.len() {
                let lag = self.params[j].phase - self.params[i].phase;
                dpsi += self.coupling * s[j].r * (s[j].psi - s[i].psi - lag).sin();
            }
        }
        dpsi
    }

    fn derivative(&self, s: &[OscState], out: &mut [OscState]) {
        for i in 0..s.len() {
            let p = &self.params[i];
            out[i] = OscState {
                psi: self.dpsi(s, i),
                r: s[i].dr,
                dr: GAIN * (GAIN / 4.0 * (p.amplitude - s[i].r) - s[i].dr),
                x: s[i].dx,
                dx: GAIN * (GAIN / 4.0 * (p.offset - s[i].x) - s[i].dx),
            };
        }
    }

    /// Advance by `dt` with classic RK4.
    pub fn step(&mut self, dt: f64) {
        let n = self.state.len();
        if n == 0 {
            return;
        }
        let axpy = |a: &[OscState], k: &[OscState], h: f64| -> Vec<OscState> {
            a.iter()
                .zip(k)
                .map(|(a, k)| OscState {
                    psi: a.psi + h * k.psi,
                    r: a.r + h * k.r,
                    dr: a.dr + h * k.dr,
                    x: a.x + h * k.x,
                    dx: a.dx + h * k.dx,
                })
                .collect()
        };
        let s0 = self.state.clone();
        let mut k1 = vec![OscState::default(); n];
        let mut k2 = k1.clone();
        let mut k3 = k1.clone();
        let mut k4 = k1.clone();
        self.derivative(&s0, &mut k1);
        self.derivative(&axpy(&s0, &k1, dt / 2.0), &mut k2);
        self.derivative(&axpy(&s0, &k2, dt / 2.0), &mut k3);
        self.derivative(&axpy(&s0, &k3, dt), &mut k4);
        for i in 0..n {
            let s = &mut self.state[i];
            let c = |a: f64, b: f64, c: f64, d: f64| dt / 6.0 * (a + 2.0 * b + 2.0 * c + d);
            s.psi += c(k1[i].psi, k2[i].psi, k3[i].psi, k4[i].psi);
            s.r += c(k1[i].r, k2[i].r, k3[i].r, k4[i].r);
            s.dr += c(k1[i].dr, k2[i].dr, k3[i].dr, k4[i].dr);
            s.x += c(k1[i].x, k2[i].x, k3[i].x, k4[i].x);
            s.dx += c(k1[i].dx, k2[i].dx, k3[i].dx, k4[i].dx);
        }
    }

    /// Current joint angle θ_i and its time derivative.
    pub fn output(&self, i: usize) -> JointCmd {
        let s = &self.state[i];
        let dpsi = self.dpsi(&self.state, i);
        JointCmd {
            angle: s.x + s.r * s.psi.cos(),
            velocity: s.dx + s.dr * s.psi.cos() - s.r * s.psi.sin() * dpsi,
        }
    }

    /// Phase of oscillator i (for plotting / tests).
    pub fn phase(&self, i: usize) -> f64 {
        self.state[i].psi
    }

    pub fn amplitude(&self, i: usize) -> f64 {
        self.state[i].r
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn wrap(a: f64) -> f64 {
        (a + PI).rem_euclid(TAU) - PI
    }

    #[test]
    fn amplitude_and_offset_converge() {
        let p = OscParams { amplitude: 0.5, offset: -0.2, phase: 0.0 };
        let mut cpg = Cpg::new(1.0, 4.0, vec![p]);
        for _ in 0..(10.0 / 0.01) as usize {
            cpg.step(0.01);
        }
        assert!((cpg.amplitude(0) - 0.5).abs() < 1e-4);
        assert!((cpg.state[0].x + 0.2).abs() < 1e-4);
    }

    #[test]
    fn chain_locks_to_requested_phase_lags() {
        let phases = [0.0, 1.0, -0.5, 2.0];
        let params = phases
            .iter()
            .map(|&phase| OscParams { amplitude: 0.4, offset: 0.0, phase })
            .collect();
        let mut cpg = Cpg::new(0.8, 4.0, params);
        for _ in 0..(20.0 / 0.01) as usize {
            cpg.step(0.01);
        }
        for i in 1..phases.len() {
            let got = wrap(cpg.phase(i) - cpg.phase(0));
            let want = wrap(phases[i] - phases[0]);
            assert!((got - want).abs() < 1e-3, "osc {i}: {got} vs {want}");
        }
    }

    #[test]
    fn frequency_is_respected() {
        let mut cpg = Cpg::new(1.5, 4.0, vec![OscParams { amplitude: 0.3, ..Default::default() }]);
        let dt = 0.001;
        for _ in 0..2000 {
            cpg.step(dt);
        }
        assert!((cpg.phase(0) - TAU * 1.5 * 2.0).abs() < 1e-6);
    }
}
