//! Chat channel adapters for the Symbi platform.
//!
//! Provides bidirectional chat integration — users invoke agents from Slack
//! (and enterprise: Teams, Mattermost), get policy-enforced, audit-logged
//! responses back.
//!
//! # Community (default)
//! - Slack adapter with Events API + slash commands
//! - Basic structured logging
//! - `ChannelAdapter` trait for building custom adapters
//!
//! # Enterprise (feature: `enterprise-hooks`)
//! - `EnterpriseChannelHooks` trait for policy, DLP, and crypto audit
//! - Teams and Mattermost platform support

pub mod config;
pub mod error;
pub mod logging;
pub mod manager;
pub mod traits;
pub mod types;

pub mod adapters;

// Re-export core types
pub use config::{ChannelConfig, PlatformSettings, SlackConfig};
pub use error::ChannelAdapterError;
pub use logging::BasicInteractionLogger;
pub use manager::{AgentInvoker, ChannelAdapterManager};
pub use traits::{ChannelAdapter, InboundHandler};
pub use types::{
    AdapterHealth, ChatDeliveryReceipt, ChatPlatform, FilteredContent, InboundMessage,
    InteractionAction, InteractionLog, OutboundMessage, PolicyDecision, SlashCommand,
};

#[cfg(feature = "enterprise-hooks")]
pub use traits::EnterpriseChannelHooks;

#[cfg(feature = "slack")]
pub use adapters::slack::SlackAdapter;
