# symbi-channel-adapter

[![crates.io](https://img.shields.io/crates/v/symbi-channel-adapter.svg)](https://crates.io/crates/symbi-channel-adapter)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

Chat channel adapters for the [Symbi](https://crates.io/crates/symbi) platform — bidirectional Slack, Teams, and Mattermost integration for AI agents.

## Overview

`symbi-channel-adapter` provides a `ChannelAdapter` trait and platform-specific implementations that let users invoke Symbi agents directly from chat platforms and receive policy-enforced, audit-logged responses.

## Features

| Feature | Default | Platform |
|---------|---------|----------|
| `slack` | Yes | Slack Events API, slash commands, Socket Mode |
| `teams` | No | Microsoft Teams Bot Framework, OAuth2 |
| `mattermost` | No | Mattermost outgoing webhooks, REST API |
| `enterprise-hooks` | No | Policy, DLP, and crypto audit hooks |

## Usage

```rust
use symbi_channel_adapter::{ChannelAdapterManager, ChannelConfig, SlackConfig};

let config = ChannelConfig {
    platform: SlackConfig {
        bot_token: std::env::var("SLACK_BOT_TOKEN").unwrap(),
        app_token: std::env::var("SLACK_APP_TOKEN").unwrap(),
        signing_secret: std::env::var("SLACK_SIGNING_SECRET").unwrap(),
        ..Default::default()
    }.into(),
    ..Default::default()
};

let manager = ChannelAdapterManager::new(config);
```

### Enabling additional platforms

```toml
[dependencies]
symbi-channel-adapter = { version = "0.1", features = ["slack", "teams", "mattermost"] }
```

## Architecture

The crate is built around a few core abstractions:

- **`ChannelAdapter`** — trait for sending/receiving messages on a chat platform
- **`InboundHandler`** — trait for processing incoming messages and slash commands
- **`ChannelAdapterManager`** — orchestrates multiple adapters with health checks and lifecycle management
- **`AgentInvoker`** — trait bridging chat messages to Symbi agent execution

Each platform adapter handles authentication, signature verification, and message formatting specific to its platform.

## Part of Symbiont

This crate is part of the [Symbiont](https://github.com/thirdkeyai/symbiont) workspace. For the full agent framework, see the [`symbi`](https://crates.io/crates/symbi) crate.

## License

MIT
