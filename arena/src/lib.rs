//! CPG Arena: evolve 2D creatures driven by central pattern generators, then
//! race them or make them wrestle.
//!
//! - [`creature`]: the file format students edit, the rules, genome encoding
//! - [`cpg`]: the oscillator network (Sproewitz et al. 2008)
//! - [`sim`]: 2D physics (rapier2d)
//! - [`ga`]: a small genetic algorithm with an ask/tell interface
//! - [`game`]: fitness per mode, training loop, folder loading, tournaments

pub mod cpg;
pub mod creature;
pub mod ga;
pub mod game;
pub mod sim;
