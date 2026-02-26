//! Inference provider implementations
//!
//! Wraps existing `LlmClient` and `SlmRunner` with the unified `InferenceProvider` trait.

#[cfg(feature = "http-input")]
pub mod cloud;

pub mod slm;
