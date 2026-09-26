//! State, configuration, and experimental scheduling shared by Void.rs frontends.
//!
//! This crate never postpones authoritative simulation or network events. The
//! scheduler accepts only explicitly deferrable presentation work.

pub mod config;
pub mod metrics;
pub mod physics;
pub mod scheduler;
pub mod world;

pub use config::{Config, ConfigError, ConfigStore, EventHorizonConfig, SchedulingMode};
pub use scheduler::{FrameBudget, Job, JobKey, JobKind, Scheduler, WorldRevision};
pub use world::ClientWorld;
