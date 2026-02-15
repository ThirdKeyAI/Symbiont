//! Adapter registry for AI CLI tools.

pub mod aider;
pub mod claude_code;
pub mod codex;

pub use aider::AiderAdapter;
pub use claude_code::ClaudeCodeAdapter;
pub use codex::{CodexAdapter, CodexApprovalMode};
