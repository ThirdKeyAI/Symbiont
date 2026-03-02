//! Inference provider implementations
//!
//! Wraps existing `LlmClient` and `SlmRunner` with the unified `InferenceProvider` trait.

#[cfg(feature = "cloud-llm")]
pub mod cloud;

pub mod slm;
