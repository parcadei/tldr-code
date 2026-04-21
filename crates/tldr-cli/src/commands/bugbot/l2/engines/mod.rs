//! L2 analysis engines for bugbot.
//!
//! Each engine implements [`super::L2Engine`] and targets a specific set of
//! finding types. Engines are registered in [`super::l2_engine_registry`] and
//! invoked by the pipeline orchestrator.

pub mod tldr_differential;
