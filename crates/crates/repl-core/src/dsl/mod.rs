//! Symbiont DSL implementation
//!
//! This module provides parsing and evaluation capabilities for the Symbiont DSL,
//! enabling declarative agent behavior definitions and runtime execution.

pub mod ast;
pub mod lexer;
pub mod parser;
pub mod evaluator;

#[cfg(test)]
mod tests;

pub use ast::*;
pub use lexer::Lexer;
pub use parser::Parser;
pub use evaluator::{DslEvaluator, DslValue, AgentInstance, AgentState};