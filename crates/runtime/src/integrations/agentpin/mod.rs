//! AgentPin Integration Module
//!
//! Provides integration with AgentPin for domain-anchored cryptographic
//! identity verification of AI agents.

pub mod discovery;
pub mod key_store;
pub mod types;
pub mod verifier;

// Re-export main types and traits for convenience
pub use key_store::AgentPinKeyStore;
pub use types::{AgentPinConfig, AgentPinError, AgentVerificationResult};
pub use verifier::{AgentPinVerifier, DefaultAgentPinVerifier, MockAgentPinVerifier};
