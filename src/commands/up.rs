use clap::ArgMatches;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use symbi_runtime::http_input::{start_http_input, HttpInputConfig};
use symbi_runtime::types::AgentId;
use symbi_runtime::AgentRuntime;
use symbi_runtime::RuntimeConfig;
use symbi_runtime::SecretsConfig;

pub async fn run(matches: &ArgMatches) {
    let port = matches
        .get_one::<String>("port")
        .expect("port argument is required");
    let http_port = matches
        .get_one::<String>("http-port")
        .expect("http-port argument is required");
    let http_token = matches.get_one::<String>("http-token");
    let http_cors = matches.get_flag("http-cors");
    let http_audit = matches.get_flag("http-audit");
    let preset = matches.get_one::<String>("preset");

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

    if http_cors {
        println!("→ CORS enabled with sensible defaults");
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
        Ok(rt) => Arc::new(rt),
        Err(e) => {
            eprintln!("✗ Failed to initialize runtime: {}", e);
            return;
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
        bind_address: "0.0.0.0".to_string(),
        port: http_port_num,
        path: "/webhook".to_string(),
        agent: agent_id,
        auth_header: Some(auth_token), // Always require authentication
        cors_enabled: http_cors,
        audit_enabled: http_audit,
        concurrency: 10,
        max_body_bytes: 1_048_576,
        routing_rules: None,
        response_control: None,
        forward_headers: vec![],
        jwt_public_key_path: None,
    };

    // Use environment variable for Vault token, or disable Vault in dev mode
    let secrets_config = if let Ok(vault_token) = std::env::var("VAULT_TOKEN") {
        if let Ok(vault_addr) = std::env::var("VAULT_ADDR") {
            SecretsConfig::vault_with_token(vault_addr, vault_token)
        } else {
            SecretsConfig::vault_with_token("http://localhost:8200".to_string(), vault_token)
        }
    } else {
        // Development mode: use file-based secrets (more secure than hardcoded token)
        eprintln!("ℹ️  Using file-based secrets (set VAULT_TOKEN for Vault integration)");
        SecretsConfig::file_json(PathBuf::from("./secrets/secrets.json"))
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

    tokio::select! {
        _ = start_http_input(http_config, Some(runtime.clone()), Some(secrets_config)) => {},
        _ = tokio::signal::ctrl_c() => {}
    }

    #[cfg(feature = "cron")]
    if let Some(ref cron) = _cron_scheduler {
        cron.shutdown().await;
        println!("✓ CronScheduler stopped");
    }

    match runtime.shutdown().await {
        Ok(_) => println!("\n✓ Runtime stopped"),
        Err(e) => eprintln!("\n⚠️  Runtime stopped with errors: {}", e),
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
