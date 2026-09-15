# symbi-channel-adapter

[![crates.io](https://img.shields.io/crates/v/symbi-channel-adapter.svg)](https://crates.io/crates/symbi-channel-adapter)
[![License](https://img.shields.io/badge/License-Apache--2.0-blue.svg)](https://www.apache.org/licenses/LICENSE-2.0)

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
use symbi_channel_adapter::{ChannelConfig, ChatPlatform, PlatformSettings, SlackConfig};

let config = ChannelConfig {
    name: "slack-support".into(),
    platform: ChatPlatform::Slack,
    settings: PlatformSettings::Slack(SlackConfig {
        bot_token: std::env::var("SLACK_BOT_TOKEN").expect("SLACK_BOT_TOKEN"),
        signing_secret: Some(
            std::env::var("SLACK_SIGNING_SECRET").expect("SLACK_SIGNING_SECRET"),
        ),
        ..Default::default()
    }),
};
```

Create `ChannelAdapterManager::new(invoker, logger)` with the application's
`AgentInvoker` and `BasicInteractionLogger`, then await
`manager.register_adapter(config)`. Install any approval command interceptor
before registering adapters so their handlers receive it.

### Slack authentication and approval commands

Before enabling Slack, configure a nonempty app signing secret. Startup fails
without it, and both callback routes reject unsigned, invalid or stale requests.
Verification covers the exact received body bytes and timestamp. Environment
labels and `SYMBIONT_SLACK_ALLOW_UNSIGNED` cannot bypass this requirement.

When the runtime approval queue and approver allowlist are configured, Slack
`/symbi gate` commands route to approval control. Use `/symbi gate show <id>`,
then copy `/symbi gate approve <id> <review-digest>` from the complete review;
`/symbi gate deny <id>` needs no review. ID-only approval is no longer accepted.
Chat reviews have an 8 KiB budget including commands and cannot be truncated to
make an action approvable. See [approval lifecycle](../../docs/approval-lifecycle.md)
for the other review surfaces and remaining platform/workspace trust limits.

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

Apache-2.0
