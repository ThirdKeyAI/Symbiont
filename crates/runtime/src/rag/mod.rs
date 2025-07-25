//! RAG (Retrieval-Augmented Generation) Engine Module
//!
//! This module provides the RAG engine implementation for the Symbiont Agent Runtime.
//! It includes query analysis, document retrieval, ranking, and response generation capabilities.

pub mod engine;
pub mod types;

#[cfg(test)]
mod tests;

pub use engine::{RAGEngine, StandardRAGEngine};
pub use types::*;
