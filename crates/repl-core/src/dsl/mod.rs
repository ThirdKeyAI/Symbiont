//! Symbiont DSL implementation
//!
//! This module provides parsing and evaluation capabilities for the Symbiont DSL,
//! enabling declarative agent behavior definitions and runtime execution.

pub mod ast;
pub mod evaluator;
pub mod lexer;
pub mod parser;

#[cfg(test)]
mod tests;

pub use ast::*;
pub use evaluator::{AgentInstance, AgentState, DslEvaluator, DslValue};
pub use lexer::Lexer;
pub use parser::Parser;
