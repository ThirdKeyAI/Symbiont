//! Policy-driven routing module for SLM-first architecture
//!
//! This module provides intelligent routing between Small Language Models (SLMs)
//! and Large Language Models (LLMs) based on configurable policies, task types,
//! resource constraints, and confidence thresholds.

pub mod classifier;
pub mod confidence;
pub mod config;
pub mod decision;
pub mod engine;
pub mod error;
pub mod policy;

pub use classifier::*;
pub use confidence::*;
pub use config::*;
pub use decision::*;
pub use engine::*;
pub use error::*;
pub use policy::*;
