use clap::ArgMatches;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use symbi_channel_adapter::{
    AgentInvoker, BasicInteractionLogger, ChannelAdapterManager, ChannelConfig, ChatPlatform,
    MattermostConfig, PlatformSettings, SlackConfig, TeamsConfig,
};
use symbi_runtime::api::server::{HttpApiConfig, HttpApiServer};
use symbi_runtime::http_input::{start_http_input, HttpInputConfig};
use symbi_runtime::types::{AgentId, SecurityTier};
use symbi_runtime::AgentRuntime;
use symbi_runtime::RuntimeConfig;
use symbi_runtime::SecretsConfig;

mod chat_response;
use chat_response::LlmAgentInvoker;

/// Map an approval-channel platform string to a [`ChatPlatform`].
/// Returns `None` for unrecognised platform identifiers.
fn parse_platform(s: &str) -> Option<ChatPlatform> {
    match s.to_lowercase().as_str() {
        "slack" => Some(ChatPlatform::Slack),
        "teams" => Some(ChatPlatform::Teams),
        "mattermost" => Some(ChatPlatform::Mattermost),
        _ => None,
    }
}

/// Resolve the same validated execution boundary used by the CLI tool runner.
pub(super) fn resolve_security_tier(source: &str, selector: &str) -> Result<SecurityTier, String> {
    use symbi_runtime::sandbox::command::{CommandBoundary, CommandTier};
    let settings = dsl::resolve_execution_settings(source, selector)?;
    let boundary = CommandBoundary::load_for_agent(Path::new("."), &settings)?;
    Ok(match boundary.tier {
        CommandTier::DevelopmentHost => SecurityTier::None,
        CommandTier::Docker => SecurityTier::Tier1,
        CommandTier::GVisor => SecurityTier::Tier2,
        CommandTier::Firecracker => SecurityTier::Tier3,
        CommandTier::E2B => SecurityTier::Hosted,
        // No SecurityTier names the landlock tier, so it cannot be reported
        // for a registered agent. Refuse rather than mapping it onto a
        // neighboring tier, which would misreport the isolation in use.
        CommandTier::Landlock => {
            return Err(
                "the landlock tier cannot be selected for a registered agent; \
                        declare it in [sandbox] for direct runs"
                    .into(),
            )
        }
    })
}

/// A schedule may name an agent in its own file or a separate agent file.
/// Resolve the actual source before reading its boundary; another definition's
/// sandbox settings must never be used merely because it contains a schedule.
#[cfg(feature = "cron")]
fn scheduled_agent_source(
    agents_dir: &Path,
    current_file: &Path,
    source: &str,
    name: &str,
) -> Result<String, String> {
    if let Ok(settings) = dsl::resolve_execution_settings(source, name) {
        if settings.agent_name == name
            || current_file.file_stem().and_then(|s| s.to_str()) == Some(name)
        {
            return Ok(source.to_owned());
        }
    }
    if name.is_empty()
        || name == "."
        || name == ".."
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
    {
        return Err("invalid scheduled agent name".into());
    }
    for extension in [dsl::SYMBI_EXTENSION, dsl::LEGACY_DSL_EXTENSION] {
        let path = agents_dir.join(format!("{name}.{extension}"));
        match fs::read_to_string(&path) {
            Ok(selected) => {
                dsl::resolve_execution_settings(&selected, name)?;
                return Ok(selected);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("cannot read scheduled agent: {error}")),
        }
    }
    Err(format!("scheduled agent {name:?} is unavailable"))
}

pub async fn run(matches: &ArgMatches) {
    // Initialize tracing for structured logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let port = matches
        .get_one::<String>("port")
        .expect("port argument is required");
    let http_port = matches
        .get_one::<String>("http-port")
        .expect("http-port argument is required");
    let http_bind = matches
        .get_one::<String>("http-bind")
        .expect("http-bind argument has default value");
    let http_token = matches.get_one::<String>("http-token");
    let cors_origins: Vec<String> = matches
        .get_one::<String>("http-cors-origins")
        .map(|s| s.split(',').map(|o| o.trim().to_string()).collect())
        .unwrap_or_default();
    let http_audit = matches.get_flag("http-audit");
    let serve_agents_md = matches.get_flag("serve-agents-md");
    let insecure_allow_all = matches.get_flag("insecure-allow-all")
        || std::env::var("SYMBI_INSECURE_ALLOW_ALL").as_deref() == Ok("1");
    let preset = matches.get_one::<String>("preset");
    let slack_token = matches.get_one::<String>("slack-token");
    let slack_signing_secret = matches.get_one::<String>("slack-signing-secret");
    let slack_port = matches
        .get_one::<String>("slack-port")
        .expect("slack-port has default value");
    let slack_agent = matches.get_one::<String>("slack-agent");

    // Teams flags (enterprise)
    let teams_tenant_id = matches.get_one::<String>("teams-tenant-id");
    let teams_client_id = matches.get_one::<String>("teams-client-id");
    let teams_client_secret = matches.get_one::<String>("teams-client-secret");
    let teams_bot_id = matches.get_one::<String>("teams-bot-id");
    let teams_port = matches
        .get_one::<String>("teams-port")
        .expect("teams-port has default value");
    let teams_agent = matches.get_one::<String>("teams-agent");

    // Mattermost flags (enterprise)
    let mm_server_url = matches.get_one::<String>("mm-server-url");
    let mm_token = matches.get_one::<String>("mm-token");
    let mm_webhook_secret = matches.get_one::<String>("mm-webhook-secret");
    let mm_port = matches
        .get_one::<String>("mm-port")
        .expect("mm-port has default value");
    let mm_agent = matches.get_one::<String>("mm-agent");

    println!("✓ Starting Symbiont runtime...");

    // Determine the authentication token to use
    let auth_token = if let Some(token) = http_token {
        // User provided a token explicitly
        token.clone()
    } else if !Path::new("symbi.toml").exists() && !Path::new("symbi.quick.toml").exists() {
        // No config exists, create one with generated token
        match create_quick_config(None) {
            Ok(generated_token) => {
                println!("✓ Created symbi.quick.toml with secure generated token");
                generated_token
            }
            Err(e) => {
                eprintln!("✗ Failed to create config file: {}", e);
                eprintln!("Please create symbi.toml manually or fix file permissions");
                return;
            }
        }
    } else {
        // Config exists but no token provided - generate one for this session
        let generated_token = generate_secure_token();
        eprintln!("\n⚠️  SECURITY WARNING: No authentication token provided!");
        eprintln!(
            "⚠️  Using session token (not persisted): {}",
            generated_token
        );
        eprintln!("⚠️  This token is only valid for this session.");
        eprintln!("⚠️  For persistent auth, add token to your config or use --http-token\n");
        generated_token
    };

    // Scan agents directory
    let agents_found = scan_agents_directory();

    // Load ToolClad manifests from tools/
    let toolclad_manifests =
        symbi_runtime::toolclad::manifest::load_manifests_from_dir(Path::new("tools"));
    if !toolclad_manifests.is_empty() {
        println!("✓ {} tool(s) loaded from tools/", toolclad_manifests.len());
        for (name, manifest) in &toolclad_manifests {
            println!(
                "  → {} [{}] ({})",
                name, manifest.tool.risk_tier, manifest.tool.binary
            );
        }
    }

    // Display clear auth info at startup
    let api_token_source = if std::env::var("SYMBIONT_API_TOKEN").is_ok() {
        "SYMBIONT_API_TOKEN env var"
    } else {
        "none (unauthenticated)"
    };
    println!(
        "✓ Runtime API on {}:{} (auth: {})",
        http_bind, port, api_token_source
    );
    println!(
        "✓ HTTP Input on {}:{} (auth: --http.token Bearer)",
        http_bind, http_port
    );
    if std::env::var("SYMBIONT_MASTER_KEY").is_err() {
        eprintln!(
            "⚠  SYMBIONT_MASTER_KEY not set — crypto operations will fail.\n\
             \x20  Generate one:  export SYMBIONT_MASTER_KEY=$(openssl rand -hex 32)\n\
             \x20  Or run `symbi init`, which writes a key to .env for you."
        );
    }

    if agents_found.len() > 1 {
        println!("→ Auto-routing by agent name:");
        for agent in &agents_found {
            let name = dsl::strip_symbi_extension(agent).unwrap_or(agent);
            println!("    /webhook/{} → {}", name, agent);
        }
    } else if let Some(agent) = agents_found.first() {
        println!("→ Auto-routing /webhook → {}", agent);
    } else {
        // No agents/ directory or no agent files. Point the user at init so
        // they don't end up with a runtime that literally does nothing.
        let has_dir = Path::new("agents").is_dir();
        if !has_dir {
            eprintln!(
                "\n⚠  No agents/ directory found — the runtime will start with no agents loaded."
            );
        } else {
            eprintln!(
                "\n⚠  agents/ directory is empty — the runtime will start with no agents loaded."
            );
        }
        eprintln!("   To scaffold a starter project:");
        eprintln!("     symbi init --profile assistant     # governed assistant agent");
        eprintln!("     symbi init --profile dev-agent     # CliExecutor agent for coding tasks");
        eprintln!("     symbi init --catalog list          # browse pre-built agents");
        eprintln!();
    }

    if let Some(preset_name) = preset {
        println!("→ Using preset: {}", preset_name);
    }

    if !cors_origins.is_empty() {
        if cors_origins.iter().any(|o| o == "*") {
            println!("→ CORS enabled with wildcard origin (not recommended for production)");
        } else {
            println!("→ CORS enabled for origins: {}", cors_origins.join(", "));
        }
    }

    if http_audit {
        println!("→ HTTP audit logging enabled");
    }

    println!("\n📝 Next steps:");
    println!("  • Test webhook: curl -H \"Authorization: Bearer {}\" http://localhost:{}/webhook -d '{{\"test\":true}}'", auth_token, http_port);
    println!("  • View status: symbi status");
    println!("  • View logs: symbi logs -f");
    println!("\nPress Ctrl+C to stop the runtime");

    let runtime = match AgentRuntime::new(RuntimeConfig::default()).await {
        Ok(rt) => Some(Arc::new(rt)),
        Err(e) => {
            eprintln!(
                "⚠️  Runtime init failed ({}), continuing with HTTP-only mode",
                e
            );
            None
        }
    };

    // Load agents from DSL files into the scheduler registry
    let loaded_agents = if let Some(ref rt) = runtime {
        let loaded = load_agents_into_registry(rt).await;
        if !loaded.is_empty() {
            println!("✓ {} agent(s) loaded from agents/", loaded.len());
        }
        loaded
    } else {
        vec![]
    };

    let first_agent_id = loaded_agents.first().map(|(_, id)| *id);
    let agent_id = first_agent_id.unwrap_or_else(AgentId::new);

    let http_port_num = match http_port.parse::<u16>() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("✗ Invalid HTTP port number '{}': {}", http_port, e);
            return;
        }
    };

    // Auto-generate routing rules when multiple agents are loaded
    let routing_rules = if loaded_agents.len() > 1 {
        use symbi_runtime::http_input::config::{AgentRoutingRule, RouteMatch};
        let rules: Vec<AgentRoutingRule> = loaded_agents
            .iter()
            .map(|(name, id)| AgentRoutingRule {
                condition: RouteMatch::PathPrefix(format!("/webhook/{}", name)),
                agent: *id,
            })
            .collect();
        Some(rules)
    } else {
        None
    };

    let http_config = HttpInputConfig {
        bind_address: http_bind.clone(),
        port: http_port_num,
        path: "/webhook".to_string(),
        agent: agent_id,
        auth_header: Some(format!("Bearer {}", auth_token)), // Always require authentication
        cors_origins,
        audit_enabled: http_audit,
        concurrency: 10,
        max_body_bytes: 1_048_576,
        routing_rules,
        response_control: None,
        forward_headers: vec![],
        jwt_public_key_path: None,
        webhook_verify: None,
    };

    // Use environment variable for Vault token, or disable Vault in dev mode
    let secrets_config = if let Ok(vault_token) = std::env::var("VAULT_TOKEN") {
        if let Ok(vault_addr) = std::env::var("VAULT_ADDR") {
            Some(SecretsConfig::vault_with_token(vault_addr, vault_token))
        } else {
            Some(SecretsConfig::vault_with_token(
                "http://localhost:8200".to_string(),
                vault_token,
            ))
        }
    } else {
        let secrets_path = PathBuf::from("./secrets/secrets.json");
        if secrets_path.exists() {
            eprintln!("ℹ️  Using file-based secrets (set VAULT_TOKEN for Vault integration)");
            Some(SecretsConfig::file_json(secrets_path))
        } else {
            eprintln!("ℹ️  No secrets configured (auth handled via --http.token)");
            None
        }
    };

    // Start CronScheduler if the cron feature is enabled and schedule files exist.
    #[cfg(feature = "cron")]
    let _cron_scheduler: Option<Arc<symbi_runtime::CronScheduler>> = {
        use symbi_runtime::{CronScheduler, CronSchedulerConfig};

        let cron_config = CronSchedulerConfig::default();
        if let Some(ref rt) = runtime {
            match CronScheduler::new_with_guards(
                cron_config,
                rt.scheduler.clone(),
                rt.agentpin_verifier.clone(),
                None,
            )
            .await
            {
                Ok(cron_sched) => {
                    let cron_arc = Arc::new(cron_sched);
                    let schedule_count = load_dsl_schedules(&cron_arc).await;
                    if schedule_count > 0 {
                        println!(
                            "✓ {} scheduled job(s) loaded from DSL files",
                            schedule_count
                        );
                    }
                    println!("✓ CronScheduler started");
                    Some(cron_arc)
                }
                Err(e) => {
                    eprintln!("⚠️  Failed to start CronScheduler: {}", e);
                    None
                }
            }
        } else {
            None
        }
    };

    // Attach the CronScheduler to the runtime if both are available.
    #[cfg(feature = "cron")]
    let runtime = match (runtime, &_cron_scheduler) {
        (Some(rt), Some(cron)) => {
            let inner = Arc::try_unwrap(rt).unwrap_or_else(|arc| (*arc).clone());
            Some(Arc::new(inner.with_cron_scheduler(cron.clone())))
        }
        (rt, _) => rt,
    };

    // Build the escalation queue (one shared instance) and read its config.
    // Escalation config is sourced from symbi.toml / symbi.quick.toml when present;
    // parse failures fall back to defaults (no approval channels).
    let escalation_cfg = ["symbi.toml", "symbi.quick.toml"]
        .iter()
        .filter(|p| Path::new(p).exists())
        .find_map(|p| symbi_runtime::config::Config::from_file(p).ok())
        .and_then(|c| c.escalation)
        .unwrap_or_default();
    let escalation_queue = Arc::new(symbi_runtime::escalation::EscalationQueue::new());
    let escalation_timeout = std::time::Duration::from_secs(
        std::env::var("SYMBIONT_ESCALATION_TIMEOUT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(escalation_cfg.timeout_seconds),
    );
    let require_approval_tools: Vec<String> = std::env::var("SYMBIONT_REQUIRE_APPROVAL_TOOLS")
        .ok()
        .map(|s| {
            s.split(',')
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect()
        })
        .unwrap_or_default();

    // The governed policy-gate ladder (permissive-if-opted-in -> Cedar ->
    // fail-closed), wrapped so flagged tool calls are held for human
    // approval. Built unconditionally, so no entry point that acts on model
    // output in this process can end up permissive by omission.
    //
    // One ladder per surface, not one gate for the process. Coordinator Chat
    // only gates what the model says back to an operator; the HTTP input
    // server dispatches real tool calls on behalf of unattended callers.
    // Sharing a single gate meant every grant written for either had to be
    // held by both. Each surface now also layers `policies/<surface>/*.cedar`
    // on top of the shared `policies/*.cedar` set, so a permit can be scoped
    // to the surface it was actually written for.
    if insecure_allow_all {
        eprintln!("\n");
        eprintln!("================================================================");
        eprintln!("WARNING: --insecure-allow-all / SYMBI_INSECURE_ALLOW_ALL=1 is set");
        eprintln!("Policy gate is in PERMISSIVE mode.");
        eprintln!("Every LLM-proposed tool call and delegation will be allowed.");
        eprintln!("This is only safe for local development. Do NOT use in production.");
        eprintln!("================================================================\n");
    }
    // Both gates share the one escalation queue, so a held action reaches the
    // same approvers (chat interceptor / Gate panel) whichever surface raised
    // it. Only the policy surface differs.
    let escalation_gate_config = symbi_runtime::escalation::EscalationGateConfig {
        require_approval_tools: require_approval_tools.clone(),
        timeout: escalation_timeout,
    };
    let policy_gate =
        symbi_runtime::reasoning::governed_gate(symbi_runtime::reasoning::GateOptions {
            policies_dir: PathBuf::from("policies"),
            surface: Some("coordinator".to_string()),
            insecure_allow_all,
            escalation: Some((escalation_queue.clone(), escalation_gate_config.clone())),
        })
        .await;
    let http_input_policy_gate =
        symbi_runtime::reasoning::governed_gate(symbi_runtime::reasoning::GateOptions {
            policies_dir: PathBuf::from("policies"),
            surface: Some("http-input".to_string()),
            insecure_allow_all,
            escalation: Some((escalation_queue.clone(), escalation_gate_config)),
        })
        .await;

    // Start chat adapters if any are configured
    let any_adapter = slack_token.is_some() || teams_tenant_id.is_some() || mm_server_url.is_some();
    let mut channel_manager: Option<Arc<ChannelAdapterManager>> = None;

    if any_adapter {
        let provider =
            symbi_runtime::reasoning::providers::cloud::CloudInferenceProvider::from_env().map(
                |provider| {
                    Arc::new(provider)
                        as Arc<dyn symbi_runtime::reasoning::inference::InferenceProvider>
                },
            );
        if provider.is_none() {
            eprintln!("No LLM provider configured; chat invocations will be refused");
        }
        let invoker: Arc<dyn AgentInvoker> = match std::env::current_dir()
            .map_err(|error| error.to_string())
            .and_then(|project| LlmAgentInvoker::new(project, provider, policy_gate.clone()))
        {
            Ok(invoker) => Arc::new(invoker),
            Err(error) => {
                eprintln!("Chat response initialization failed: {error}");
                return;
            }
        };

        let logger = Arc::new(BasicInteractionLogger::new(None));
        let mut manager = ChannelAdapterManager::new(invoker, logger);

        // Wire the escalation command interceptor BEFORE registering any adapter
        // (adapter command handlers are built at register time). Authorization is
        // scoped per (platform, channel_id): a sender may only resolve held actions
        // from a configured approval channel that lists them — never cross-channel
        // or from an arbitrary channel the bot also reads.
        if !escalation_cfg.approval_channels.is_empty() {
            let mut channel_approvers: symbi_runtime::escalation::ChannelApprovers =
                std::collections::HashMap::new();
            for c in &escalation_cfg.approval_channels {
                if let Some(platform) = parse_platform(&c.platform) {
                    channel_approvers
                        .entry((platform, c.channel_id.clone()))
                        .or_default()
                        .extend(c.approvers.iter().cloned());
                } else {
                    eprintln!(
                        "⚠️  escalation.approval_channels: unknown platform '{}' (channel {}) — skipped",
                        c.platform, c.channel_id
                    );
                }
            }
            manager.set_interceptor(Arc::new(
                symbi_runtime::escalation::EscalationCommandInterceptor::new(
                    escalation_queue.clone(),
                    channel_approvers,
                ),
            ));
        }

        // Register Slack adapter if --slack.token is provided
        if let Some(token) = slack_token {
            let slack_port_num = match slack_port.parse::<u16>() {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("✗ Invalid Slack port number '{}': {}", slack_port, e);
                    return;
                }
            };

            let default_agent = slack_agent.cloned().or_else(|| {
                agents_found
                    .first()
                    .map(|f| dsl::strip_symbi_extension(f).unwrap_or(f).to_string())
            });

            let slack_config = SlackConfig {
                bot_token: token.clone(),
                app_token: None,
                signing_secret: slack_signing_secret.cloned(),
                workspace_id: None,
                channels: Vec::new(),
                webhook_port: slack_port_num,
                bind_address: "0.0.0.0".to_string(),
                default_agent: default_agent.clone(),
            };

            let channel_config = ChannelConfig {
                name: "slack".to_string(),
                platform: ChatPlatform::Slack,
                settings: PlatformSettings::Slack(slack_config),
            };

            match manager.register_adapter(channel_config).await {
                Ok(()) => {
                    println!("✓ Slack adapter started on :{}", slack_port_num);
                    if let Some(ref agent) = default_agent {
                        println!("→ Default Slack agent: {}", agent);
                    }
                    println!("\n📎 Slack setup:");
                    println!(
                        "  1. Configure Event Subscriptions URL: https://<your-host>:{}/slack/events",
                        slack_port_num
                    );
                    println!("  2. Subscribe to bot event: app_mention");
                    println!("  3. Add bot scopes: app_mentions:read, chat:write");
                    println!("  4. Invite @bot to a channel, then @mention it");
                }
                Err(e) => {
                    eprintln!("✗ Failed to start Slack adapter: {}", e);
                    return;
                }
            }
        }

        // Register Teams adapter if --teams.tenant-id is provided
        if let Some(tenant_id) = teams_tenant_id {
            let client_id = match teams_client_id {
                Some(id) => id.clone(),
                None => {
                    eprintln!("✗ --teams.client-id is required when using Teams adapter");
                    return;
                }
            };
            let client_secret = match teams_client_secret {
                Some(s) => s.clone(),
                None => {
                    eprintln!("✗ --teams.client-secret is required when using Teams adapter");
                    return;
                }
            };

            let teams_port_num = match teams_port.parse::<u16>() {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("✗ Invalid Teams port number '{}': {}", teams_port, e);
                    return;
                }
            };

            let default_agent = teams_agent.cloned().or_else(|| {
                agents_found
                    .first()
                    .map(|f| dsl::strip_symbi_extension(f).unwrap_or(f).to_string())
            });

            let teams_config = TeamsConfig {
                tenant_id: tenant_id.clone(),
                client_id,
                client_secret,
                bot_id: teams_bot_id.cloned().unwrap_or_default(),
                webhook_url: None,
                webhook_port: teams_port_num,
                bind_address: "0.0.0.0".to_string(),
                default_agent: default_agent.clone(),
                skip_jwks_verification: false,
            };

            let channel_config = ChannelConfig {
                name: "teams".to_string(),
                platform: ChatPlatform::Teams,
                settings: PlatformSettings::Teams(teams_config),
            };

            match manager.register_adapter(channel_config).await {
                Ok(()) => {
                    println!("✓ Teams adapter started on :{}", teams_port_num);
                    if let Some(ref agent) = default_agent {
                        println!("→ Default Teams agent: {}", agent);
                    }
                    println!("\n📎 Teams setup:");
                    println!(
                        "  1. Configure Bot Framework messaging endpoint: https://<your-host>:{}/teams/messages",
                        teams_port_num
                    );
                    println!("  2. Add the bot to a Teams channel");
                    println!("  3. @mention the bot to invoke an agent");
                }
                Err(e) => {
                    eprintln!("✗ Failed to start Teams adapter: {}", e);
                    return;
                }
            }
        }

        // Register Mattermost adapter if --mm.server-url is provided
        if let Some(server_url) = mm_server_url {
            let bot_token = match mm_token {
                Some(t) => t.clone(),
                None => {
                    eprintln!("✗ --mm.token is required when using Mattermost adapter");
                    return;
                }
            };

            let mm_port_num = match mm_port.parse::<u16>() {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("✗ Invalid Mattermost port number '{}': {}", mm_port, e);
                    return;
                }
            };

            let default_agent = mm_agent.cloned().or_else(|| {
                agents_found
                    .first()
                    .map(|f| dsl::strip_symbi_extension(f).unwrap_or(f).to_string())
            });

            let mm_config = MattermostConfig {
                server_url: server_url.clone(),
                bot_token,
                webhook_secret: mm_webhook_secret.cloned(),
                team_id: None,
                channels: Vec::new(),
                webhook_port: mm_port_num,
                bind_address: "0.0.0.0".to_string(),
                default_agent: default_agent.clone(),
            };

            let channel_config = ChannelConfig {
                name: "mattermost".to_string(),
                platform: ChatPlatform::Mattermost,
                settings: PlatformSettings::Mattermost(mm_config),
            };

            match manager.register_adapter(channel_config).await {
                Ok(()) => {
                    println!("✓ Mattermost adapter started on :{}", mm_port_num);
                    if let Some(ref agent) = default_agent {
                        println!("→ Default Mattermost agent: {}", agent);
                    }
                    println!("\n📎 Mattermost setup:");
                    println!(
                        "  1. Create an outgoing webhook pointing to: https://<your-host>:{}/mattermost/webhook",
                        mm_port_num
                    );
                    println!("  2. Set a trigger word (e.g. @symbi)");
                    println!("  3. Use 'run <agent-name> <input>' in the channel");
                }
                Err(e) => {
                    eprintln!("✗ Failed to start Mattermost adapter: {}", e);
                    return;
                }
            }
        }

        // Adapters are registered; share the manager and subscribe one chat
        // notifier per approval channel so held actions are announced.
        let manager = Arc::new(manager);
        for ch in &escalation_cfg.approval_channels {
            if let Some(platform) = parse_platform(&ch.platform) {
                escalation_queue
                    .subscribe(Arc::new(
                        symbi_runtime::escalation::ChatEscalationNotifier::new(
                            manager.clone(),
                            platform,
                            ch.channel_id.clone(),
                        ),
                    ))
                    .await;
            } else {
                eprintln!(
                    "⚠️  Unknown approval channel platform '{}' — notifier skipped",
                    ch.platform
                );
            }
        }
        channel_manager = Some(manager);
    }

    // Parse the API port and configure the management API server
    let api_port_num = match port.parse::<u16>() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("✗ Invalid API port number '{}': {}", port, e);
            return;
        }
    };

    let api_config = HttpApiConfig {
        bind_address: http_bind.clone(),
        port: api_port_num,
        enable_cors: true,
        enable_tracing: true,
        enable_rate_limiting: true,
        api_keys_file: None,
        serve_agents_md,
    };

    let mut api_server =
        HttpApiServer::new(api_config).with_escalation_queue(escalation_queue.clone());
    if let Some(ref rt) = runtime {
        api_server = api_server.with_runtime_provider(rt.clone());

        // Wire up Coordinator Chat if an LLM provider is available
        if let Some(cloud_provider) =
            symbi_runtime::reasoning::providers::cloud::CloudInferenceProvider::from_env()
        {
            let delegation_registry = match symbi_runtime::reasoning::delegation_executor::RegisteredDelegationRegistry::load(Path::new(".")) {
                Ok(registry) => registry,
                Err(error) => {
                    eprintln!("Cannot load the governed delegation registry: {error}");
                    return;
                }
            };
            let coordinator_state = Arc::new(
                symbi_runtime::api::coordinator::CoordinatorState::new(
                    Arc::new(cloud_provider),
                    policy_gate.clone(),
                    rt.clone(),
                )
                .with_rag("symbi-coordinator")
                .await
                .with_registered_delegation(delegation_registry),
            );
            api_server = api_server.with_coordinator(coordinator_state);
            println!("✓ Coordinator Chat enabled on /ws/chat");
        } else {
            println!("ℹ️  No LLM API key found — Coordinator Chat disabled");
        }
    }

    tokio::select! {
        result = api_server.start() => {
            if let Err(e) = result {
                eprintln!("✗ API server error: {}", e);
            }
        },
        _ = start_http_input(http_config, runtime.clone(), secrets_config, Some(http_input_policy_gate.clone())) => {},
        _ = tokio::signal::ctrl_c() => {}
    }

    // Shutdown chat adapters. We try to reclaim unique ownership of the manager
    // for the `&mut` shutdown. Note: when approval channels are configured, the
    // escalation queue (still held by the running api_server / coordinator) keeps
    // notifier Arc clones of the manager alive, so `try_unwrap` will fail and the
    // adapters stop on process exit instead — graceful cleanup only, not required
    // for correctness. The no-chat / no-approval-channel path reclaims cleanly.
    drop(escalation_queue);
    if let Some(manager) = channel_manager.take() {
        match Arc::try_unwrap(manager) {
            Ok(mut manager) => {
                let results = manager.shutdown().await;
                for (name, result) in results {
                    match result {
                        Ok(()) => println!("✓ {} adapter stopped", name),
                        Err(e) => eprintln!("⚠️  {} adapter stop error: {}", name, e),
                    }
                }
            }
            Err(_) => {
                eprintln!(
                    "⚠️  Channel manager still referenced — adapters will stop on process exit"
                );
            }
        }
    }

    #[cfg(feature = "cron")]
    if let Some(ref cron) = _cron_scheduler {
        cron.shutdown().await;
        println!("✓ CronScheduler stopped");
    }

    if let Some(rt) = runtime {
        match rt.shutdown().await {
            Ok(_) => println!("\n✓ Runtime stopped"),
            Err(e) => eprintln!("\n⚠️  Runtime stopped with errors: {}", e),
        }
    } else {
        println!("\n✓ HTTP-only mode stopped");
    }
}

/// Generate a cryptographically secure random token
fn generate_secure_token() -> String {
    use std::io::Read;
    let mut bytes = [0u8; 24];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        let _ = f.read_exact(&mut bytes);
    }
    let encoded: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
    format!("symbi_dev_{}", encoded)
}

fn create_quick_config(http_token: Option<&str>) -> Result<String, std::io::Error> {
    let token = if let Some(t) = http_token {
        t.to_string()
    } else {
        // Generate a secure random token instead of using "dev"
        let generated_token = generate_secure_token();
        eprintln!("\n⚠️  SECURITY WARNING: No authentication token provided!");
        eprintln!(
            "⚠️  Generated secure development token: {}",
            generated_token
        );
        eprintln!("⚠️  Save this token! You'll need it to authenticate requests.");
        eprintln!("⚠️  For production, use: --http-token <your-secure-token>\n");
        generated_token
    };

    let config = format!(
        r#"# Symbiont Quick Start Configuration
# Generated by symbi up

[runtime]
mode = "dev"
hot_reload = true

[http]
enabled = true
port = 8081
# ⚠️  SECURITY: Keep this token secret!
# ⚠️  For production, use environment variables or secure secret management
dev_token = "{}"

[storage]
type = "sqlite"
path = "./symbi.db"

[logging]
level = "info"
format = "pretty"
"#,
        token
    );

    fs::write("symbi.quick.toml", &config)?;
    Ok(token)
}

/// Scan DSL files in the agents directory for `schedule` blocks and register
/// them with the CronScheduler. Returns the number of schedules registered.
#[cfg(feature = "cron")]
async fn load_dsl_schedules(cron: &symbi_runtime::CronScheduler) -> usize {
    let agents_dir = Path::new("agents");
    if !agents_dir.exists() {
        return 0;
    }

    let mut count = 0;
    if let Ok(entries) = fs::read_dir(agents_dir) {
        for entry in entries.flatten() {
            if dsl::is_symbi_file(&entry.path()) {
                if let Ok(source) = fs::read_to_string(entry.path()) {
                    match dsl::parse_dsl(&source) {
                        Ok(tree) => match dsl::extract_schedule_definitions(&tree, &source) {
                            Ok(schedules) => {
                                for sched_def in schedules {
                                    if let Some(ref cron_expr) = sched_def.cron {
                                        let name = sched_def
                                            .agent
                                            .clone()
                                            .unwrap_or_else(|| sched_def.name.clone());
                                        let selected_source = match scheduled_agent_source(
                                            agents_dir,
                                            &entry.path(),
                                            &source,
                                            &name,
                                        ) {
                                            Ok(source) => source,
                                            Err(error) => {
                                                eprintln!(
                                                    "  ⚠ Invalid scheduled agent '{name}': {error}"
                                                );
                                                continue;
                                            }
                                        };
                                        let security_tier = match resolve_security_tier(
                                            &selected_source,
                                            &name,
                                        ) {
                                            Ok(tier) => tier,
                                            Err(error) => {
                                                eprintln!("  ⚠ Invalid scheduled sandbox for '{name}': {error}");
                                                continue;
                                            }
                                        };
                                        let agent_config = symbi_runtime::types::AgentConfig {
                                            id: symbi_runtime::types::AgentId::new(),
                                            name,
                                            dsl_source: selected_source,
                                            execution_mode:
                                                symbi_runtime::types::ExecutionMode::Ephemeral,
                                            security_tier,
                                            resource_limits:
                                                symbi_runtime::types::ResourceLimits::default(),
                                            capabilities: vec![],
                                            policies: vec![],
                                            metadata: std::collections::HashMap::new(),
                                            priority: symbi_runtime::types::Priority::Normal,
                                        };

                                        let mut job = symbi_runtime::CronJobDefinition::new(
                                            sched_def.name.clone(),
                                            cron_expr.clone(),
                                            sched_def.timezone.clone(),
                                            agent_config,
                                        );
                                        job.one_shot = sched_def.one_shot;
                                        if let Some(ref policy) = sched_def.policy {
                                            job.policy_ids.push(policy.clone());
                                        }

                                        match cron
                                            .add_source_job(&entry.path().to_string_lossy(), job)
                                            .await
                                        {
                                            Ok(id) => {
                                                println!(
                                                    "  → {} ({} {}) [{}]",
                                                    sched_def.name,
                                                    cron_expr,
                                                    sched_def.timezone,
                                                    id
                                                );
                                                count += 1;
                                            }
                                            Err(e) => {
                                                eprintln!(
                                                    "  ⚠ Failed to register schedule '{}': {}",
                                                    sched_def.name, e
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!(
                                    "  ⚠ Schedule extraction error in {}: {}",
                                    entry.path().display(),
                                    e
                                );
                            }
                        },
                        Err(e) => {
                            eprintln!("  ⚠ DSL parse error in {}: {}", entry.path().display(), e);
                        }
                    }
                }
            }
        }
    }
    count
}

/// Scan DSL files in the agents directory, parse each one, create an
/// `AgentConfig`, and register it with the runtime scheduler so that
/// `/api/v1/agents` lists them and `/api/v1/agents/:id/execute` works.
async fn load_agents_into_registry(runtime: &AgentRuntime) -> Vec<(String, AgentId)> {
    let agents_dir = Path::new("agents");
    if !agents_dir.exists() {
        return vec![];
    }

    let mut agents = Vec::new();
    if let Ok(entries) = fs::read_dir(agents_dir) {
        for entry in entries.flatten() {
            if dsl::is_symbi_file(&entry.path()) {
                if let Ok(source) = fs::read_to_string(entry.path()) {
                    let name = entry
                        .path()
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default();

                    let security_tier = match resolve_security_tier(&source, &name) {
                        Ok(tier) => tier,
                        Err(error) => {
                            eprintln!("  ⚠ Invalid agent execution settings for '{name}': {error}");
                            continue;
                        }
                    };

                    let agent_config = symbi_runtime::types::AgentConfig {
                        id: symbi_runtime::types::AgentId::new(),
                        name: name.clone(),
                        dsl_source: source,
                        execution_mode: symbi_runtime::types::ExecutionMode::Ephemeral,
                        security_tier: security_tier.clone(),
                        resource_limits: symbi_runtime::types::ResourceLimits::default(),
                        capabilities: vec![symbi_runtime::types::Capability::Computation],
                        policies: vec![],
                        metadata: std::collections::HashMap::new(),
                        priority: symbi_runtime::types::Priority::Normal,
                    };

                    match runtime.scheduler.register_agent(agent_config).await {
                        Ok(id) => {
                            println!("  → {} [{}] sandbox={}", name, id, security_tier);
                            agents.push((name, id));
                        }
                        Err(e) => {
                            eprintln!("  ⚠ Failed to register agent '{}': {}", name, e);
                        }
                    }
                }
            }
        }
    }
    agents
}

fn scan_agents_directory() -> Vec<String> {
    let agents_dir = Path::new("agents");
    let mut agents = Vec::new();

    if agents_dir.exists() && agents_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(agents_dir) {
            for entry in entries.flatten() {
                if dsl::is_symbi_file(&entry.path()) {
                    if let Some(name) = entry.path().file_name() {
                        agents.push(name.to_string_lossy().to_string());
                    }
                }
            }
        }
    }

    agents
}

#[cfg(all(test, feature = "cron"))]
mod execution_selection_tests {
    use super::*;

    #[test]
    fn schedule_binds_to_its_named_source_and_preserves_legacy_extension() {
        let root = tempfile::tempdir().unwrap();
        let current = root.path().join("coordinator.symbi");
        let source = r#"agent coordinator() { with sandbox = "docker" {} }"#;
        let target = r#"agent worker() { with sandbox = "gvisor" {} }"#;
        fs::write(root.path().join("worker.dsl"), target).unwrap();
        assert_eq!(
            scheduled_agent_source(root.path(), &current, source, "worker").unwrap(),
            target
        );
        assert_eq!(
            scheduled_agent_source(root.path(), &current, source, "coordinator").unwrap(),
            source
        );
        for name in ["../worker", "missing", "", "/worker"] {
            assert!(scheduled_agent_source(root.path(), &current, source, name).is_err());
        }
        fs::write(
            root.path().join("worker.symbi"),
            "agent worker() { with sandbox = }",
        )
        .unwrap();
        assert!(scheduled_agent_source(root.path(), &current, source, "worker").is_err());
    }
}
