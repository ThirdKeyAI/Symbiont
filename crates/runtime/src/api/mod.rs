//! HTTP API module for Symbiont Runtime
//!
//! This module provides an optional HTTP API interface for the Symbiont Runtime System.
//! The entire module is conditionally compiled based on the `http-api` feature flag.

#[cfg(feature = "http-api")]
pub mod server;

#[cfg(feature = "http-api")]
pub mod routes;

#[cfg(feature = "http-api")]
pub mod middleware;

#[cfg(feature = "http-api")]
pub mod types;

#[cfg(feature = "http-api")]
pub mod traits;

#[cfg(feature = "http-api")]
pub use server::HttpApiServer;

#[cfg(feature = "http-api")]
pub use traits::RuntimeApiProvider;
