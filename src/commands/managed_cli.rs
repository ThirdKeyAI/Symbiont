//! Managed CLI execution through runtime-owned tool and inference brokers.

#[cfg(unix)]
pub use platform::run_claude_code;

#[cfg(not(unix))]
pub async fn run_claude_code(
    _: &clap::ArgMatches,
    _: &std::collections::HashMap<String, String>,
    _: &str,
    _: &dsl::ExecutionPolicy,
) {
    eprintln!("managed CLI requires the Unix sandbox broker; no host fallback");
    std::process::exit(1);
}

#[cfg(unix)]
mod platform {
    use clap::ArgMatches;
    use std::{
        collections::{HashMap, HashSet},
        path::PathBuf,
        sync::Arc,
        time::{Duration, Instant, SystemTime},
    };
    use symbi_runtime::{
        cli_executor::{
            broker::McpToolBroker,
            governed::ManagedCliActionExecutor,
            inference_broker::{InferenceBrokerConfig, ProtectedInference},
            AiCliAdapter, ClaudeCodeAdapter, CliExecutorConfig, CodeGenRequest, CodeGenResult,
            StdinStrategy,
        },
        reasoning::{
            conversation::Conversation,
            executor::ActionExecutor,
            governed_session::GovernedToolSession,
            inference::ToolDefinition,
            loop_types::{LoopConfig, LoopState},
            policy_bridge::ReasoningPolicyGate,
            protected_journal::ProtectedJournal,
            source_policy::SourcePolicyExecutor,
        },
        sandbox::{
            command::{CommandBoundary, CommandTier},
            ExecutionResult,
        },
        toolclad::{manifest::load_manifests_from_dir, ToolCladExecutor},
        types::AgentId,
    };

    const MANAGED_CLI_SURFACE: &str = "managed-cli";

    pub async fn run_claude_code(
        matches: &ArgMatches,
        meta: &HashMap<String, String>,
        input: &str,
        source_policy: &dsl::ExecutionPolicy,
    ) {
        match execute(matches, meta, input, source_policy).await {
            Ok(result) => {
                if let Some(json) = result.parsed_output {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json).unwrap_or_default()
                    );
                } else {
                    println!("{}", result.execution.stdout);
                }
                eprintln!(
                    "\n--- managed run ok in {}ms (exit {}) ---",
                    result.execution.execution_time_ms, result.execution.exit_code
                );
            }
            Err(error) => {
                eprintln!("managed CLI failed: {error}");
                std::process::exit(1);
            }
        }
    }

    async fn execute(
        matches: &ArgMatches,
        meta: &HashMap<String, String>,
        input: &str,
        source_policy: &dsl::ExecutionPolicy,
    ) -> Result<CodeGenResult, String> {
        let settings = source_policy.settings();
        let agent_name = &settings.agent_name;
        let requires_approval = admission_approval(meta)?;
        let max_turns = bound(matches, "max-turns", 12, 1000)? as u32;
        let budget_tokens = bound(matches, "budget-tokens", 100_000, 1_000_000)?;
        let budget_secs = match matches.get_one::<String>("budget-timeout") {
            Some(value) => parse_duration_secs(value)
                .filter(|v| (1..=86400).contains(v))
                .ok_or("budget-timeout must be between 1 second and 1 day")?,
            None => 900,
        };
        let budget_secs = settings
            .timeout_seconds
            .map_or(budget_secs, |bound| budget_secs.min(bound));
        let deadline = Instant::now() + Duration::from_secs(budget_secs);
        let started_at = SystemTime::now();
        let max_tool_calls = max_turns.saturating_mul(8).min(1000);
        if matches.get_one::<String>("plugin-dir").is_some() {
            return Err("managed CLI no longer loads plugins; remove --plugin-dir and register governed ToolClad tools".into());
        }
        if meta_str(meta, "permission_mode").is_some_and(|mode| mode != "dontAsk") {
            return Err("managed CLI permissions are decided by the runtime; permission_mode must be omitted or dontAsk".into());
        }
        let project = std::env::current_dir().map_err(|e| e.to_string())?;
        let mut boundary = CommandBoundary::load_for_agent(&project, settings)?;
        let (target, worker_target) = if boundary.tier == CommandTier::Firecracker {
            let config = boundary
                .firecracker
                .as_mut()
                .ok_or("missing Firecracker configuration")?;
            let target = matches
                .get_one::<String>("target")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(&config.working_dir));
            if !target.is_absolute()
                || target.components().any(|component| {
                    matches!(
                        component,
                        std::path::Component::ParentDir | std::path::Component::CurDir
                    )
                })
            {
                return Err(
                    "Firecracker target must be an absolute guest path without traversal".into(),
                );
            }
            config.working_dir = target.to_str().ok_or("target must be UTF-8")?.into();
            (target.clone(), target)
        } else {
            let target = matches
                .get_one::<String>("target")
                .map(PathBuf::from)
                .unwrap_or_else(|| project.clone())
                .canonicalize()
                .map_err(|e| format!("cannot resolve target: {e}"))?;
            if !target.is_dir() {
                return Err("managed CLI target must be a directory".into());
            }
            let config = match boundary.tier {
                CommandTier::Docker => &mut boundary.docker,
                CommandTier::GVisor => &mut boundary.gvisor.docker,
                _ => {
                    return Err(
                        "managed CLI requires Docker, gVisor or Firecracker; no host fallback"
                            .into(),
                    )
                }
            };
            let worker_target = mounted_path(&config.volumes, &target, false)?;
            config.working_dir = worker_target.to_str().ok_or("target must be UTF-8")?.into();
            (target, worker_target)
        };
        boundary.validate()?;
        let inference_config = InferenceBrokerConfig::from_project(&project)?;
        if meta_str(meta, "model").is_some_and(|model| model != inference_config.model) {
            return Err(
                "agent model differs from the protected inference model in symbiont.toml".into(),
            );
        }
        let backend: Arc<dyn ActionExecutor> = Arc::new(
            ToolCladExecutor::new(load_manifests_from_dir(&project.join("tools")))
                .with_project_scope(&project)
                .with_command_boundary(boundary.clone()),
        );
        if backend
            .tool_definitions()
            .iter()
            .any(|tool| tool.name == "claude_code")
        {
            return Err(
                "claude_code is reserved for managed worker admission; rename the registered tool"
                    .into(),
            );
        }
        let backend: Arc<dyn ActionExecutor> =
            Arc::new(SourcePolicyExecutor::new(backend, source_policy.clone()));
        let tools = select_tools(
            agent_name,
            meta_str(meta, "allowed_tools").as_deref(),
            &backend.tool_definitions(),
        )?;
        let agent_id = AgentId::new();
        let mut state = LoopState::new(agent_id, Conversation::with_system("managed-cli"));
        state.started_at = started_at.into();
        state
            .trusted_context
            .insert("agent_name".into(), serde_json::json!(agent_name));
        state
            .trusted_context
            .insert("target".into(), serde_json::json!(target));
        state
            .trusted_context
            .insert("working_directory".into(), serde_json::json!(worker_target));
        let gate = build_policy_gate(matches).await?;
        let details = serde_json::json!({"agent":agent_name, "target":target,
            "working_directory":worker_target, "tool_sandbox":boundary.descriptor()?,
            "inference":inference_config, "tools":tools, "max_turns":max_turns,
            "max_tool_calls":max_tool_calls, "timeout_seconds":budget_secs,
            "reserved_output_tokens":budget_tokens});
        // Create required protected storage before accepting any worker request.
        let private = project.join(".symbiont/governed");
        let journal =
            Arc::new(ProtectedJournal::create(&private, agent_id).map_err(|e| e.to_string())?);
        let loop_config = LoopConfig {
            max_iterations: max_tool_calls,
            timeout: Duration::from_secs(budget_secs),
            max_total_tokens: budget_tokens as u32,
            tool_definitions: tools.clone(),
            ..Default::default()
        };
        let session = Arc::new(
            GovernedToolSession::start(backend, gate, journal.clone(), state, loop_config).await?,
        );
        println!("Managed Claude Code run: agent '{agent_name}', session {agent_id}");
        println!("  target: {} (through registered tools)", target.display());
        println!("  journal: {}", journal.path().display());
        println!("  audit public key: {}", hex::encode(journal.public_key()));
        let mut broker = None;
        let result: Result<CodeGenResult, String> = async {
            // Only this explicit operator-named variable is read. No subscription or
            // ambient provider credentials enter the child or its mounted channel.
            let credential = std::env::var(&inference_config.api_key_env).map_err(|_| {
                format!(
                    "missing configured inference credential variable {}",
                    inference_config.api_key_env
                )
            })?;
            let model = inference_config.model.clone();
            let max_output = inference_config
                .max_output_tokens_per_request
                .min(budget_tokens);
            let inference = ProtectedInference::new(
                inference_config,
                credential,
                budget_tokens,
                deadline,
                session.clone(),
            )?;
            broker = Some(McpToolBroker::start_with_inference(session.clone(), &private, Some(inference)).await?);
            let broker = broker.as_ref().ok_or("managed broker was not created")?;
            let child_boundary = broker.child_boundary(&boundary)?;
            let directory = if child_boundary.tier == symbi_runtime::sandbox::command::CommandTier::Firecracker { "/tmp" } else { "/workspace" };
            let adapter = BrokeredAdapter {
                inner: ClaudeCodeAdapter {
                    max_turns: Some(max_turns),
                    model: Some(model.clone()),
                    allowed_tools: tools
                        .iter()
                        .map(|tool| format!("mcp__symbi__{}", tool.name))
                        .collect(),
                    builtin_tools: Some(vec![]),
                    mcp_config: Some(broker.mcp_config_for(&child_boundary).to_string()),
                    strict_mcp_config: true,
                    bare: true,
                    permission_mode: Some("dontAsk".into()),
                    stream_json: true,
                    append_system_prompt: meta_str(meta, "system_prompt"),
                    managed: true,
                    session_id: Some(agent_id.to_string()),
                    project_dir: Some(directory.into()),
                    ..Default::default()
                },
                model,
                max_output,
                inference_args: broker.inference_bridge_args(&child_boundary)?,
            };
            let prompt = if input.trim().is_empty() || input.trim() == "{}" {
                if child_boundary.tier == CommandTier::Firecracker {
                    format!("Review guest target {worker_target:?} through the registered tools.")
                } else {
                    format!("Review {worker_target:?} through the registered tools. Source files are available only to governed tool backends.")
                }
            } else {
                input.to_owned()
            };
            let request = CodeGenRequest {
                prompt,
                working_dir: directory.into(),
                target_files: vec![],
                system_context: None,
                model: None,
                options: HashMap::new(),
            };
            let launch = Arc::new(ManagedCliActionExecutor::new(
                Arc::new(adapter), request, child_boundary,
                CliExecutorConfig { max_runtime: Duration::from_secs(budget_secs), ..Default::default() },
                details, requires_approval,
            )?);
            let action = launch.proposal(&format!("admission-{agent_id}"));
            let governed: Arc<dyn ActionExecutor> = Arc::new(SourcePolicyExecutor::new(launch.clone(), source_policy.clone()));
            let observations = session.dispatch_host_action(governed, action).await?;
            let worker_result = launch.take_result();
            if observations.len() != 1 || observations[0].is_error {
                let detail = observations.first().map(|observation| observation.content.as_str()).unwrap_or("missing admission outcome");
                return match worker_result {
                    Ok(result) => Ok(result),
                    Err(error) => Err(format!("policy gate denied claude_code spawn or worker failed: {detail}; {error}; inspect the signed journal and policies/{MANAGED_CLI_SURFACE}/")),
                };
            }
            worker_result
        }.await;
        if !result.as_ref().is_ok_and(|result| result.success) {
            session.cancel();
        }
        let cleanup = match broker {
            Some(broker) => broker.close().await,
            None => session.close().await,
        };
        match result {
            Ok(result) if result.success => {
                cleanup?;
                Ok(result)
            }
            Ok(result) => Err(format!(
                "worker FAILED (exit {}): {}; output: {}; cleanup: {:?}",
                result.execution.exit_code,
                symbi_runtime::text_util::truncate_utf8(&result.execution.stderr, 8192),
                symbi_runtime::text_util::truncate_utf8(&result.execution.stdout, 8192),
                cleanup.err()
            )),
            Err(error) => Err(format!("{error}; cleanup: {:?}", cleanup.err())),
        }
    }

    struct BrokeredAdapter {
        inner: ClaudeCodeAdapter,
        model: String,
        max_output: u64,
        inference_args: Vec<String>,
    }
    #[async_trait::async_trait]
    impl AiCliAdapter for BrokeredAdapter {
        fn name(&self) -> &str {
            self.inner.name()
        }
        fn executable(&self) -> &str {
            "python3"
        }
        fn build_args(&self, request: &CodeGenRequest) -> Vec<String> {
            let mut args = self.inference_args.clone();
            args.push(self.inner.executable().into());
            args.extend(self.inner.build_args(request));
            args
        }
        fn non_interactive_env(&self) -> HashMap<String, String> {
            let mut env = self.inner.non_interactive_env();
            env.extend([
                ("ANTHROPIC_BASE_URL".into(), "http://127.0.0.1:8765".into()),
                ("ANTHROPIC_API_KEY".into(), "private-broker-channel".into()),
                ("ANTHROPIC_MODEL".into(), self.model.clone()),
                (
                    "CLAUDE_CODE_MAX_OUTPUT_TOKENS".into(),
                    self.max_output.to_string(),
                ),
                (
                    "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".into(),
                    "1".into(),
                ),
            ]);
            env
        }
        fn stdin_strategy(&self) -> StdinStrategy {
            StdinStrategy::CloseImmediately
        }
        fn parse_output(&self, request: &CodeGenRequest, result: ExecutionResult) -> CodeGenResult {
            self.inner.parse_output(request, result)
        }
        async fn health_check(&self) -> anyhow::Result<()> {
            anyhow::bail!("managed CLI health probes require the selected sandbox")
        }
    }

    fn admission_approval(meta: &HashMap<String, String>) -> Result<bool, String> {
        match meta.get("human_approval").map(|value| value.trim()) {
            None | Some("false") => Ok(false),
            Some("true") => Ok(true),
            Some(_) => Err("managed human_approval metadata must be true or false".into()),
        }
    }

    fn select_tools(
        agent: &str,
        names: Option<&str>,
        registry: &[ToolDefinition],
    ) -> Result<Vec<ToolDefinition>, String> {
        let names = names.ok_or_else(|| {
            format!("agent '{agent}' must declare registered allowed_tools in metadata")
        })?;
        let mut seen = HashSet::new();
        let mut tools = Vec::new();
        for name in names.split(',').map(str::trim) {
            if name.is_empty() || !seen.insert(name) {
                return Err("allowed_tools must contain distinct registered names".into());
            }
            tools.push(registry.iter().find(|tool| tool.name == name).ok_or_else(|| format!("allowed tool '{name}' is not registered; built-in CLI tools are unavailable"))?.clone());
        }
        if tools.is_empty() {
            return Err("managed CLI requires at least one registered tool".into());
        }
        Ok(tools)
    }

    fn bound(matches: &ArgMatches, key: &str, default: u64, max: u64) -> Result<u64, String> {
        let value = matches
            .get_one::<String>(key)
            .map(|s| s.parse::<u64>())
            .transpose()
            .map_err(|_| format!("invalid {key}"))?
            .unwrap_or(default);
        if !(1..=max).contains(&value) {
            return Err(format!("{key} must be between 1 and {max}"));
        }
        Ok(value)
    }

    fn mounted_path(
        volumes: &[String],
        host: &std::path::Path,
        read_only: bool,
    ) -> Result<PathBuf, String> {
        let mut matches = Vec::new();
        for volume in volumes {
            let parts: Vec<_> = volume.split(':').collect();
            let source = PathBuf::from(parts.first().ok_or("missing mount source")?)
                .canonicalize()
                .map_err(|e| format!("cannot resolve mount source: {e}"))?;
            if let Ok(relative) = host.strip_prefix(&source) {
                let mount = PathBuf::from(parts.get(1).ok_or("missing mount destination")?);
                let destination = if relative.as_os_str().is_empty() {
                    mount
                } else {
                    mount.join(relative)
                };
                matches.push((
                    source.components().count(),
                    destination,
                    parts.get(2).is_none_or(|mode| *mode == "ro"),
                ));
            }
        }
        matches.sort_by_key(|entry| std::cmp::Reverse(entry.0));
        let Some((specificity, destination, mount_read_only)) = matches.first() else {
            return Err(format!(
                "{} is outside explicit sandbox mounts; configure a scoped volume",
                host.display()
            ));
        };
        if matches.get(1).is_some_and(|entry| entry.0 == *specificity) {
            return Err("ambiguous sandbox mount mapping".into());
        }
        if read_only && !mount_read_only {
            return Err("managed CLI plugin requires a read-only sandbox mount".into());
        }
        // A different host source mounted over the resolved destination would
        // substitute the authorized target or plugin at worker startup.
        for volume in volumes {
            let parts: Vec<_> = volume.split(':').collect();
            let mounted = std::path::Path::new(parts.get(1).ok_or("missing mount destination")?);
            if mounted.starts_with(destination) || destination.starts_with(mounted) {
                let source = PathBuf::from(parts[0])
                    .canonicalize()
                    .map_err(|e| e.to_string())?;
                if let Ok(relative) = destination.strip_prefix(mounted) {
                    if source.join(relative) != host {
                        return Err("sandbox mount shadows the requested path".into());
                    }
                } else {
                    return Err(
                        "nested sandbox mount substitutes content below the requested path".into(),
                    );
                }
            }
        }
        Ok(destination.clone())
    }

    fn meta_str(meta: &HashMap<String, String>, key: &str) -> Option<String> {
        meta.get(key)
            .map(|v| v.trim().trim_matches('"').to_string())
            .filter(|s| !s.is_empty())
    }

    /// Parse a duration like `15m`, `900s`, `2h`, or a bare number of seconds.
    fn parse_duration_secs(s: &str) -> Option<u64> {
        let s = s.trim();
        if let Some(rest) = s.strip_suffix('h') {
            return rest
                .trim()
                .parse::<u64>()
                .ok()
                .and_then(|v| v.checked_mul(3600));
        }
        if let Some(rest) = s.strip_suffix('m') {
            return rest
                .trim()
                .parse::<u64>()
                .ok()
                .and_then(|v| v.checked_mul(60));
        }
        if let Some(rest) = s.strip_suffix('s') {
            return rest.trim().parse::<u64>().ok();
        }
        s.parse::<u64>().ok()
    }

    async fn build_policy_gate(
        matches: &ArgMatches,
    ) -> Result<Arc<dyn ReasoningPolicyGate>, String> {
        Ok(
            symbi_runtime::reasoning::governed_gate(symbi_runtime::reasoning::GateOptions {
                policies_dir: PathBuf::from("policies"),
                surface: Some(MANAGED_CLI_SURFACE.into()),
                insecure_allow_all: std::env::var("SYMBI_INSECURE_ALLOW_ALL").as_deref() == Ok("1"),
                escalation: crate::commands::approval::from_matches(matches).await?,
            })
            .await,
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn admission_approval_accepts_only_boolean_metadata() {
            assert!(!admission_approval(&HashMap::new()).unwrap());
            for (value, expected) in [("true", true), ("false", false), (" true ", true)] {
                assert_eq!(
                    admission_approval(&HashMap::from([("human_approval".into(), value.into())]))
                        .unwrap(),
                    expected
                );
            }
            for value in ["yes", "1", "", "\"true\""] {
                assert!(admission_approval(&HashMap::from([(
                    "human_approval".into(),
                    value.into()
                )]))
                .is_err());
            }
        }
        #[test]
        fn tool_selection_is_an_exact_registered_subset() {
            let registry = vec![ToolDefinition {
                name: "read_file".into(),
                description: "read".into(),
                parameters: serde_json::json!({"type":"object"}),
            }];
            assert_eq!(
                select_tools("review", Some("read_file"), &registry)
                    .unwrap()
                    .len(),
                1
            );
            for names in [
                None,
                Some(""),
                Some("Read"),
                Some("read_file,read_file"),
                Some("read_file,Write"),
            ] {
                assert!(select_tools("review", names, &registry).is_err());
            }
        }
        #[test]
        fn overflowing_duration_is_invalid() {
            assert_eq!(parse_duration_secs("18446744073709551615h"), None);
            assert_eq!(parse_duration_secs("18446744073709551615m"), None);
            assert_eq!(parse_duration_secs("2h"), Some(7200));
        }
        #[test]
        fn mount_mapping_rejects_substitution_and_writable_plugins() {
            let root = tempfile::tempdir().unwrap();
            let source = root.path().join("source");
            let other = root.path().join("other");
            std::fs::create_dir(&source).unwrap();
            std::fs::create_dir(&other).unwrap();
            let read_only = vec![format!("{}:/workspace:ro", source.display())];
            assert_eq!(
                mounted_path(&read_only, &source, true).unwrap(),
                PathBuf::from("/workspace")
            );
            assert_eq!(
                mounted_path(&read_only, &source, true).unwrap().to_str(),
                Some("/workspace")
            );
            let writable = vec![format!("{}:/workspace:rw", source.display())];
            assert!(mounted_path(&writable, &source, true)
                .unwrap_err()
                .contains("read-only"));
            assert!(mounted_path(&read_only, &other, false).is_err());
            for destination in ["/workspace", "/workspace/nested"] {
                let mut shadowed = read_only.clone();
                shadowed.push(format!("{}:{destination}:ro", other.display()));
                assert!(mounted_path(&shadowed, &source, false).is_err());
            }
            let mut duplicate = read_only.clone();
            duplicate.push(format!("{}:/alternative:ro", source.display()));
            assert!(mounted_path(&duplicate, &source, false)
                .unwrap_err()
                .contains("ambiguous"));
        }
    }
}
