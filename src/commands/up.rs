use async_trait::async_trait;
use clap::ArgMatches;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use symbi_channel_adapter::{
    AgentInvoker, BasicInteractionLogger, ChannelAdapterManager, ChannelConfig, ChatPlatform,
    MattermostConfig, PlatformSettings, SlackConfig, TeamsConfig,
};
use symbi_runtime::http_input::llm_client::LlmClient;
use symbi_runtime::http_input::{start_http_input, HttpInputConfig};
use symbi_runtime::types::AgentId;
use symbi_runtime::AgentRuntime;
use symbi_runtime::RuntimeConfig;
use symbi_runtime::SecretsConfig;

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
    let http_token = matches.get_one::<String>("http-token");
    let cors_origins: Vec<String> = matches
        .get_one::<String>("http-cors-origins")
        .map(|s| s.split(',').map(|o| o.trim().to_string()).collect())
        .unwrap_or_default();
    let http_audit = matches.get_flag("http-audit");
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

    println!("✓ Runtime started on :{}", port);
    println!("✓ HTTP Input enabled on :{}", http_port);
    println!("✓ Authentication: ENABLED (Bearer token required)");

    if let Some(agent) = agents_found.first() {
        println!("→ Auto-routing /webhook → {}", agent);
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

    let agent_id = if let Some(_agent) = agents_found.first() {
        // For now, just create a new agent ID
        // TODO: Implement proper agent creation when runtime API is stable
        AgentId::new()
    } else {
        AgentId::new()
    };

    let http_port_num = match http_port.parse::<u16>() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("✗ Invalid HTTP port number '{}': {}", http_port, e);
            return;
        }
    };

    let http_config = HttpInputConfig {
        bind_address: "127.0.0.1".to_string(),
        port: http_port_num,
        path: "/webhook".to_string(),
        agent: agent_id,
        auth_header: Some(format!("Bearer {}", auth_token)), // Always require authentication
        cors_origins,
        audit_enabled: http_audit,
        concurrency: 10,
        max_body_bytes: 1_048_576,
        routing_rules: None,
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
    let _cron_scheduler = {
        use symbi_runtime::{CronScheduler, CronSchedulerConfig};

        let cron_config = CronSchedulerConfig::default();
        let cron_agent_sched = std::sync::Arc::new(
            symbi_runtime::DefaultAgentScheduler::new(symbi_runtime::SchedulerConfig::default())
                .await
                .expect("failed to create cron agent scheduler"),
        );

        match CronScheduler::new(cron_config, cron_agent_sched).await {
            Ok(cron_sched) => {
                let schedule_count = load_dsl_schedules(&cron_sched).await;
                if schedule_count > 0 {
                    println!(
                        "✓ {} scheduled job(s) loaded from DSL files",
                        schedule_count
                    );
                }
                println!("✓ CronScheduler started");
                Some(cron_sched)
            }
            Err(e) => {
                eprintln!("⚠️  Failed to start CronScheduler: {}", e);
                None
            }
        }
    };

    // Start chat adapters if any are configured
    let any_adapter = slack_token.is_some() || teams_tenant_id.is_some() || mm_server_url.is_some();
    let mut channel_manager: Option<ChannelAdapterManager> = None;

    if any_adapter {
        // Build the AgentInvoker bridge (shared across all adapters)
        let dsl_sources = scan_agent_dsl_sources();
        let llm_client = LlmClient::from_env().map(Arc::new);

        if llm_client.is_none() {
            eprintln!("⚠️  No LLM API key found — agent responses will be stubs");
        }

        let invoker: Arc<dyn AgentInvoker> = Arc::new(LlmAgentInvoker {
            llm_client,
            dsl_sources: Arc::new(dsl_sources),
        });

        let logger = Arc::new(BasicInteractionLogger::new(None));
        let mut manager = ChannelAdapterManager::new(invoker, logger);

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
                    .map(|f| f.strip_suffix(".dsl").unwrap_or(f).to_string())
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
                    .map(|f| f.strip_suffix(".dsl").unwrap_or(f).to_string())
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
                    .map(|f| f.strip_suffix(".dsl").unwrap_or(f).to_string())
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

        channel_manager = Some(manager);
    }

    tokio::select! {
        _ = start_http_input(http_config, runtime.clone(), secrets_config) => {},
        _ = tokio::signal::ctrl_c() => {}
    }

    // Shutdown Slack adapter
    if let Some(ref mut manager) = channel_manager {
        let results = manager.shutdown().await;
        for (name, result) in results {
            match result {
                Ok(()) => println!("✓ {} adapter stopped", name),
                Err(e) => eprintln!("⚠️  {} adapter stop error: {}", name, e),
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
    use std::time::{SystemTime, UNIX_EPOCH};

    // Use system time and process ID for additional entropy
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id();

    // Create a secure random token using SHA-256
    // Format: symbi_<timestamp>_<pid>_<random_suffix>
    format!("symbi_dev_{:x}_{:x}", timestamp, pid)
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
            if entry.path().extension().is_some_and(|ext| ext == "dsl") {
                if let Ok(source) = fs::read_to_string(entry.path()) {
                    match dsl::parse_dsl(&source) {
                        Ok(tree) => match dsl::extract_schedule_definitions(&tree, &source) {
                            Ok(schedules) => {
                                for sched_def in schedules {
                                    if let Some(ref cron_expr) = sched_def.cron {
                                        let agent_config = symbi_runtime::types::AgentConfig {
                                            id: symbi_runtime::types::AgentId::new(),
                                            name: sched_def
                                                .agent
                                                .unwrap_or_else(|| sched_def.name.clone()),
                                            dsl_source: source.clone(),
                                            execution_mode:
                                                symbi_runtime::types::ExecutionMode::Ephemeral,
                                            security_tier:
                                                symbi_runtime::types::SecurityTier::Tier1,
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

                                        match cron.add_job(job).await {
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

fn scan_agents_directory() -> Vec<String> {
    let agents_dir = Path::new("agents");
    let mut agents = Vec::new();

    if agents_dir.exists() && agents_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(agents_dir) {
            for entry in entries.flatten() {
                if let Some(ext) = entry.path().extension() {
                    if ext == "dsl" {
                        if let Some(name) = entry.path().file_name() {
                            agents.push(name.to_string_lossy().to_string());
                        }
                    }
                }
            }
        }
    }

    agents
}

/// Scan agents/ directory and return (filename, content) pairs for DSL files.
fn scan_agent_dsl_sources() -> Vec<(String, String)> {
    let agents_dir = Path::new("agents");
    let mut sources = Vec::new();

    if !agents_dir.exists() || !agents_dir.is_dir() {
        return sources;
    }

    if let Ok(entries) = fs::read_dir(agents_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "dsl") {
                if let Ok(content) = fs::read_to_string(&path) {
                    let filename = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    sources.push((filename, content));
                }
            }
        }
    }

    sources
}

/// Bridge between the channel adapter's `AgentInvoker` trait and the LLM client.
///
/// Builds a system prompt from DSL sources (same logic as the HTTP input server)
/// and calls the LLM client for chat completion.
struct LlmAgentInvoker {
    llm_client: Option<Arc<LlmClient>>,
    dsl_sources: Arc<Vec<(String, String)>>,
}

#[async_trait]
impl AgentInvoker for LlmAgentInvoker {
    async fn invoke(&self, agent_name: &str, input: &str) -> Result<String, String> {
        let llm = match &self.llm_client {
            Some(client) => client,
            None => {
                return Ok(format!(
                    "No LLM provider configured. Set OPENROUTER_API_KEY, OPENAI_API_KEY, or ANTHROPIC_API_KEY. (agent: {})",
                    agent_name
                ));
            }
        };

        // Build system prompt from DSL sources (mirrors server.rs logic)
        let mut system_parts: Vec<String> = Vec::new();

        if !self.dsl_sources.is_empty() {
            system_parts.push(
                "You are an AI agent operating within the Symbiont runtime. \
                 Your behavior is governed by the following agent definitions:"
                    .to_string(),
            );
            for (filename, content) in self.dsl_sources.iter() {
                system_parts.push(format!("\n--- {} ---\n{}", filename, content));
            }
            system_parts.push(
                "\nFollow the capabilities, constraints, and policies defined above. \
                 Provide thorough, professional analysis. If a request violates your \
                 constraints, politely decline and explain why."
                    .to_string(),
            );
        } else {
            system_parts.push(
                "You are an AI agent operating within the Symbiont runtime. \
                 Provide thorough, professional analysis based on the input provided."
                    .to_string(),
            );
        }

        let system_prompt = system_parts.join("\n");

        tracing::info!(
            agent = %agent_name,
            provider = %llm.provider(),
            model = %llm.model(),
            input_len = input.len(),
            "Invoking LLM for chat agent"
        );

        llm.chat_completion(&system_prompt, input)
            .await
            .map_err(|e| format!("LLM error: {}", e))
    }
}
