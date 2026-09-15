//! CLI Executor module for AI tool orchestration
//!
//! Provides a universal stdin-protected process runner with per-tool adapters
//! for orchestrating AI CLI tools (Claude Code, Gemini, Aider, Codex) from
//! headless environments (cron jobs, scheduled agents, CI pipelines).
//!
//! Gated behind the `cli-executor` feature flag.

pub mod adapter;
pub mod adapters;
#[cfg(unix)]
pub mod broker;
pub mod executor;
#[cfg(unix)]
pub mod inference_broker;
mod monitor;
pub mod watchdog;

pub use adapter::{AiCliAdapter, CodeGenRequest, CodeGenResult};
#[cfg(unix)]
pub mod governed;
pub use adapters::{AiderAdapter, ClaudeCodeAdapter, CodexAdapter, CodexApprovalMode};
pub use executor::{CliExecutor, CliExecutorConfig, StdinStrategy};
pub use watchdog::{LineSink, OutputWatchdog, WatchdogOutput};
