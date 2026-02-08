//! Model management module for Symbiont runtime
//!
//! This module provides infrastructure for managing Small Language Models (SLMs)
//! and their execution within the Symbiont platform. It includes:
//!
//! - [`ModelCatalog`]: Central registry for model definitions and capabilities
//! - [`SlmRunner`]: Trait for executing models with security constraints
//! - Concrete runner implementations for different model types
//!
//! # Security
//!
//! All model execution respects the configured [`SandboxProfile`] to ensure
//! proper resource isolation and security constraints.

pub mod catalog;
pub mod runners;

pub use catalog::{ModelCatalog, ModelCatalogError};
pub use runners::{LocalGgufRunner, SlmRunner, SlmRunnerError};
