//! CPG Arena: evolve 2D creatures driven by central pattern generators, then
//! race them or make them wrestle.
//!
//! - [`creature`]: the file format students edit, the rules, genome encoding
//! - [`cpg`]: the oscillator network (Sproewitz et al. 2008)
//! - [`sim`]: 2D physics (rapier2d)
//! - [`problem`]: the black-box optimisation interface students' GAs talk to
//! - [`game`]: fitness per mode, folder loading, tournaments

pub mod cpg;
pub mod creature;
pub mod game;
pub mod problem;
pub mod sim;
