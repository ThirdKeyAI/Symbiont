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
//! # Enterprise
//! - Teams adapter (feature: `teams`) — Bot Framework webhook + OAuth2
//! - Mattermost adapter (feature: `mattermost`) — Outgoing webhooks + REST API
//! - `EnterpriseChannelHooks` trait for policy, DLP, and crypto audit (feature: `enterprise-hooks`)

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

#[cfg(feature = "teams")]
pub use adapters::teams::TeamsAdapter;
#[cfg(feature = "teams")]
pub use config::TeamsConfig;

#[cfg(feature = "mattermost")]
pub use adapters::mattermost::MattermostAdapter;
#[cfg(feature = "mattermost")]
pub use config::MattermostConfig;
