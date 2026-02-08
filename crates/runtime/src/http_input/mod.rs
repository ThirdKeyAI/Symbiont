//! HTTP Input module for Symbiont Runtime
//!
//! This module provides HTTP input capabilities that allow external systems to invoke
//! Symbiont agents via HTTP requests. The entire module is conditionally compiled
//! based on the `http-input` feature flag.

#[cfg(feature = "http-input")]
pub mod config;

#[cfg(feature = "http-input")]
pub mod server;

#[cfg(feature = "http-input")]
pub use config::{AgentRoutingRule, HttpInputConfig, ResponseControlConfig, RouteMatch};

#[cfg(feature = "http-input")]
pub use server::{start_http_input, HttpInputServer};
