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

#[cfg(all(unix, feature = "http-api"))]
mod chat_invocations;
#[cfg(feature = "http-api")]
pub mod invocations;

#[cfg(feature = "http-api")]
pub mod api_keys;

#[cfg(feature = "http-api")]
pub mod types;

#[cfg(feature = "http-api")]
pub mod traits;

#[cfg(feature = "http-api")]
pub mod ws_types;

#[cfg(feature = "http-api")]
pub mod streaming_journal;

#[cfg(feature = "http-api")]
pub mod coordinator_executor;

#[cfg(feature = "http-api")]
pub mod coordinator;

#[cfg(feature = "http-api")]
pub mod ws_handler;

#[cfg(feature = "http-api")]
pub mod escalation_routes;

#[cfg(feature = "http-api")]
pub use server::HttpApiServer;

#[cfg(feature = "http-api")]
pub use traits::RuntimeApiProvider;

#[cfg(all(test, unix, feature = "http-api"))]
mod coordinator_tests;
