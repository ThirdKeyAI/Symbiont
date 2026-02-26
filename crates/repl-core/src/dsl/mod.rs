//! Symbiont DSL implementation
//!
//! This module provides parsing and evaluation capabilities for the Symbiont DSL,
//! enabling declarative agent behavior definitions and runtime execution.

pub mod agent_composition;
pub mod ast;
pub mod evaluator;
pub mod lexer;
pub mod parser;
pub mod pattern_builtins;
pub mod reasoning_builtins;

#[cfg(test)]
mod tests;

pub use ast::*;
pub use evaluator::{AgentInstance, AgentState, DslEvaluator, DslValue};
pub use lexer::Lexer;
pub use parser::Parser;
