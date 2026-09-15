//! One immutable, single-use worker launch behind prepared authorization.

use super::{
    AiCliAdapter, CliExecutor, CliExecutorConfig, CodeGenRequest, CodeGenResult, StdinStrategy,
};
use crate::{
    reasoning::{
        circuit_breaker::CircuitBreakerRegistry,
        executor::ActionExecutor,
        inference::ToolDefinition,
        loop_types::{LoopConfig, Observation, ProposedAction},
        prepared::{canonical_json, digest_json, AuthorizedAction, PreparedAction, ToolContract},
    },
    sandbox::{command::CommandBoundary, command_cleanup, ExecutionResult},
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};

const NAME: &str = "claude_code";

/// Freeze adapter output before policy/approval. Parsing cannot replace the
/// process outcome observed by CliExecutor.
struct FrozenAdapter {
    parser: Arc<dyn AiCliAdapter>,
    executable: String,
    args: Vec<String>,
    env: HashMap<String, String>,
    stdin: StdinStrategy,
}
#[async_trait]
impl AiCliAdapter for FrozenAdapter {
    fn name(&self) -> &str {
        self.parser.name()
    }
    fn executable(&self) -> &str {
        &self.executable
    }
    fn build_args(&self, _: &CodeGenRequest) -> Vec<String> {
        self.args.clone()
    }
    fn non_interactive_env(&self) -> HashMap<String, String> {
        self.env.clone()
    }
    fn stdin_strategy(&self) -> StdinStrategy {
        self.stdin.clone()
    }
    fn parse_output(&self, request: &CodeGenRequest, result: ExecutionResult) -> CodeGenResult {
        self.parser.parse_output(request, result)
    }
    async fn health_check(&self) -> anyhow::Result<()> {
        anyhow::bail!("admission does not authorize a separate health probe")
    }
}

struct LaunchIdentity(uuid::Uuid);

pub struct ManagedCliActionExecutor {
    instance: uuid::Uuid,
    adapter: FrozenAdapter,
    request: CodeGenRequest,
    boundary: CommandBoundary,
    limits: CliExecutorConfig,
    arguments: Value,
    approval: bool,
    used: AtomicBool,
    result: Mutex<Option<Result<CodeGenResult, String>>>,
    owners: command_cleanup::Registry,
}

impl ManagedCliActionExecutor {
    /// `details` describes trusted inference/tool scope. Runtime-owned launch
    /// fields cannot be overridden. Credentials belong in the host broker.
    pub fn new(
        adapter: Arc<dyn AiCliAdapter>,
        request: CodeGenRequest,
        mut boundary: CommandBoundary,
        limits: CliExecutorConfig,
        details: Value,
        requires_approval: bool,
    ) -> Result<Self, String> {
        if !request.options.is_empty() {
            return Err("managed admission does not accept worker environment overrides".into());
        }
        super::executor::configure_boundary(
            &mut boundary,
            &request.working_dir,
            limits.max_output_bytes,
        )?;
        super::executor::validate_limits(&limits).map_err(|error| error.to_string())?;
        let frozen = FrozenAdapter {
            executable: adapter.executable().into(),
            args: adapter.build_args(&request),
            env: adapter.non_interactive_env(),
            stdin: adapter.stdin_strategy(),
            parser: adapter,
        };
        super::executor::validate_stdin(&frozen.stdin).map_err(|error| error.to_string())?;
        let mut argv = vec![frozen.executable.clone()];
        argv.extend(frozen.args.clone());
        crate::sandbox::command::literal_command(&argv)?;
        let environment = super::executor::worker_environment(frozen.env.clone(), HashMap::new())
            .map_err(|error| error.to_string())?;
        let mut request_value =
            serde_json::to_value(&request).map_err(|error| error.to_string())?;
        // Cedar has no null value. Missing optional request fields mean None.
        request_value
            .as_object_mut()
            .ok_or("invalid worker request")?
            .retain(|_, value| !value.is_null());
        let mut arguments = details
            .as_object()
            .ok_or("admission details must be an object")?
            .clone();
        for (name, value) in [
            ("request", request_value),
            ("argv", json!(argv)),
            ("environment", json!(environment)),
            ("sandbox", boundary.descriptor()?),
            (
                "stdin",
                serde_json::to_value(&frozen.stdin).map_err(|error| error.to_string())?,
            ),
            (
                "limits",
                serde_json::to_value(&limits).map_err(|error| error.to_string())?,
            ),
        ] {
            if arguments.insert(name.into(), value).is_some() {
                return Err(format!("admission details cannot replace {name}"));
            }
        }
        let arguments = Value::Object(arguments);
        if canonical_json(&arguments)?.len() > 1024 * 1024 {
            return Err("managed admission exceeds 1 MiB".into());
        }
        Ok(Self {
            instance: uuid::Uuid::new_v4(),
            adapter: frozen,
            request,
            boundary,
            limits,
            arguments,
            approval: requires_approval,
            used: AtomicBool::new(false),
            result: Mutex::new(None),
            owners: command_cleanup::Registry::default(),
        })
    }

    pub fn proposal(&self, call_id: &str) -> ProposedAction {
        ProposedAction::ToolCall {
            name: NAME.into(),
            call_id: call_id.into(),
            arguments: canonical_json(&self.arguments).expect("retained JSON is serializable"),
        }
    }
    pub fn take_result(&self) -> Result<CodeGenResult, String> {
        self.result
            .lock()
            .map_err(|_| "managed result storage is poisoned")?
            .take()
            .ok_or_else(|| "managed admission did not launch a worker".to_owned())?
    }
}

#[async_trait]
impl ActionExecutor for ManagedCliActionExecutor {
    fn validate_configuration(&self) -> Result<(), String> {
        self.boundary.validate()
    }
    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        vec![ToolDefinition {
            name: NAME.into(),
            description: "Launch the fixed managed worker through its private brokers".into(),
            parameters: json!({"type":"object", "const":self.arguments}),
        }]
    }
    fn prepare_action(
        &self,
        action: &ProposedAction,
        _: &LoopConfig,
    ) -> Result<PreparedAction, String> {
        let ProposedAction::ToolCall {
            name,
            arguments,
            call_id,
        } = action
        else {
            return Err("managed admission requires its fixed launch action".into());
        };
        if name != NAME
            || arguments.len() > 1024 * 1024
            || serde_json::from_str::<Value>(arguments).map_err(|error| error.to_string())?
                != self.arguments
        {
            return Err("managed admission arguments differ from the retained launch".into());
        }
        PreparedAction::new(
            self.proposal(call_id),
            Some(ToolContract {
                name: NAME.into(),
                version: env!("CARGO_PKG_VERSION").into(),
                digest: digest_json(&self.arguments)?,
                action_type: "Action".into(),
                action_id: format!("tool_call::{NAME}"),
                resource_type: "Resource".into(),
                resource_id: "default".into(),
                requires_approval: self.approval,
            }),
        )?
        .with_resolved(json!({"kind":"managed_cli_spawn", "executor":self.instance}))
        .map(|prepared| prepared.with_backend(LaunchIdentity(self.instance)))
    }
    fn cancel_run(&self, run: &str, _: Instant) {
        self.owners.cancel(run);
    }
    async fn close_run(&self, run: &str, _: Instant) -> Result<(), String> {
        self.owners.close(run).await
    }
    async fn execute_actions(
        &self,
        actions: &[ProposedAction],
        _: &LoopConfig,
        _: &CircuitBreakerRegistry,
    ) -> Vec<Observation> {
        actions
            .iter()
            .filter_map(|action| match action {
                ProposedAction::ToolCall { name, call_id, .. } => Some(
                    Observation::tool_error(
                        name,
                        "managed launch requires a prepared authorization grant",
                    )
                    .with_call_id(call_id),
                ),
                _ => None,
            })
            .collect()
    }
    async fn execute_authorized(
        &self,
        actions: Vec<AuthorizedAction>,
        _: &LoopConfig,
        _: &CircuitBreakerRegistry,
    ) -> Vec<Observation> {
        let mut observations = Vec::new();
        for grant in actions {
            let call_id = match grant.action() {
                ProposedAction::ToolCall { call_id, .. } => call_id.clone(),
                _ => continue,
            };
            let checked: Result<(), String> = (|| {
                grant.check_live()?;
                if !grant
                    .prepared()
                    .backend::<LaunchIdentity>()
                    .is_some_and(|identity| identity.0 == self.instance)
                {
                    return Err("managed authorization belongs to another launch".into());
                }
                self.used
                    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                    .map_err(|_| "managed launch was already consumed")?;
                Ok(())
            })();
            if let Err(error) = checked {
                observations.push(Observation::tool_error(NAME, error).with_call_id(call_id));
                continue;
            }
            let deadline = grant.deadline();
            let result = async {
                let owner = self.owners.admit(grant.run_key(), grant.run_deadline())?;
                let _prepared = grant.into_prepared()?;
                let mut limits = self.limits.clone();
                limits.max_runtime = limits
                    .max_runtime
                    .min(deadline.saturating_duration_since(Instant::now()));
                let executor =
                    CliExecutor::new(limits).with_command_boundary(self.boundary.clone());
                owner
                    .scope(async {
                        tokio::time::timeout_at(
                            deadline.into(),
                            executor.execute(&self.adapter, &self.request),
                        )
                        .await
                        .map_err(|_| {
                            "managed authorization timed out; completion is unconfirmed".to_owned()
                        })?
                        .map_err(|error| error.to_string())
                    })
                    .await
            }
            .await;
            let observation = match &result {
                Ok(result) => {
                    let summary = json!({"success":result.success, "exit_code":result.execution.exit_code,
                        "stdout_hash":digest_json(&json!(result.execution.stdout)).expect("output is JSON serializable"),
                        "stderr_hash":digest_json(&json!(result.execution.stderr)).expect("output is JSON serializable"),
                        "stdout_bytes":result.execution.stdout.len(), "stderr_bytes":result.execution.stderr.len()}).to_string();
                    if result.success { Observation::tool_result(NAME, summary) } else { Observation::tool_error(NAME, summary) }
                },
                Err(error) => Observation::tool_error(NAME, error),
            }.with_call_id(call_id);
            match self.result.lock() {
                Ok(mut slot) => {
                    *slot = Some(result);
                    observations.push(observation);
                }
                Err(_) => observations.push(Observation::tool_error(
                    NAME,
                    "managed result storage is poisoned",
                )),
            }
        }
        observations
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reasoning::{
        conversation::Conversation,
        dispatch::GovernedToolDispatcher,
        loop_types::{BufferedJournal, LoopState},
        policy_bridge::DefaultPolicyGate,
    };
    use crate::types::AgentId;

    struct MutableAdapter(Arc<Mutex<String>>);
    #[async_trait]
    impl AiCliAdapter for MutableAdapter {
        fn name(&self) -> &str {
            "fixture"
        }
        fn executable(&self) -> &str {
            "python3"
        }
        fn build_args(&self, _: &CodeGenRequest) -> Vec<String> {
            vec!["-c".into(), self.0.lock().unwrap().clone()]
        }
        fn non_interactive_env(&self) -> HashMap<String, String> {
            HashMap::from([("FIXTURE".into(), self.0.lock().unwrap().clone())])
        }
        fn stdin_strategy(&self) -> StdinStrategy {
            StdinStrategy::CloseImmediately
        }
        fn parse_output(&self, _: &CodeGenRequest, _: ExecutionResult) -> CodeGenResult {
            panic!("denied admission must not reach a worker")
        }
        async fn health_check(&self) -> anyhow::Result<()> {
            panic!("unexpected probe")
        }
    }
    fn request() -> CodeGenRequest {
        CodeGenRequest {
            prompt: "fixed task".into(),
            working_dir: "/workspace".into(),
            target_files: vec![],
            system_context: None,
            model: None,
            options: HashMap::new(),
        }
    }
    fn launch(approval: bool) -> ManagedCliActionExecutor {
        ManagedCliActionExecutor::new(
            Arc::new(MutableAdapter(Arc::new(Mutex::new("original".into())))),
            request(),
            CommandBoundary::default(),
            CliExecutorConfig::default(),
            json!({}),
            approval,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn launch_freezes_adapter_output_and_refuses_raw_or_modified_calls() {
        let code = Arc::new(Mutex::new("original".into()));
        let executor = ManagedCliActionExecutor::new(
            Arc::new(MutableAdapter(code.clone())),
            request(),
            CommandBoundary::default(),
            CliExecutorConfig::default(),
            json!({}),
            false,
        )
        .unwrap();
        let config = LoopConfig::default();
        let before = executor
            .prepare_action(&executor.proposal("fixed"), &config)
            .unwrap();
        *code.lock().unwrap() = "replacement".into();
        let after = executor
            .prepare_action(&executor.proposal("fixed"), &config)
            .unwrap();
        assert_eq!(before.fingerprint(), after.fingerprint());
        assert_eq!(executor.adapter.build_args(&request())[1], "original");
        assert_eq!(
            executor.adapter.non_interactive_env()["FIXTURE"],
            "original"
        );
        assert_eq!(executor.arguments["environment"]["HOME"], "/tmp");
        assert_eq!(executor.arguments["argv"][2], "original");
        assert!(executor.arguments["request"].get("model").is_none());
        let mut modified = executor.proposal("modified");
        if let ProposedAction::ToolCall { arguments, .. } = &mut modified {
            *arguments = "{}".into();
        }
        assert!(executor.prepare_action(&modified, &config).is_err());
        let results = executor
            .execute_actions(
                &[executor.proposal("raw")],
                &config,
                &CircuitBreakerRegistry::default(),
            )
            .await;
        assert!(results[0].is_error);
        assert!(executor
            .take_result()
            .unwrap_err()
            .contains("did not launch"));
        assert!(!executor.used.load(Ordering::Acquire));
    }

    #[test]
    fn launch_rejects_environment_overrides_host_execution_and_reserved_details() {
        for (options, boundary, details) in [
            (
                HashMap::from([("PATH".into(), "/untrusted".into())]),
                CommandBoundary::default(),
                json!({}),
            ),
            (
                HashMap::new(),
                CommandBoundary::development_host(),
                json!({}),
            ),
            (
                HashMap::new(),
                CommandBoundary::default(),
                json!({"sandbox":{}}),
            ),
        ] {
            let mut request = request();
            request.options = options;
            assert!(ManagedCliActionExecutor::new(
                Arc::new(MutableAdapter(Arc::new(Mutex::new("original".into())))),
                request,
                boundary,
                CliExecutorConfig::default(),
                details,
                false
            )
            .is_err());
        }
    }

    #[tokio::test]
    async fn mandatory_launch_approval_cannot_be_waived_by_a_permissive_gate() {
        let executor = launch(true);
        let journal = BufferedJournal::new(10);
        let gate = DefaultPolicyGate::permissive_for_dev_only();
        let breakers = CircuitBreakerRegistry::default();
        let state = LoopState::new(AgentId::new(), Conversation::with_system("fixture"));
        let config = LoopConfig {
            tool_definitions: executor.tool_definitions(),
            ..Default::default()
        };
        let dispatcher = GovernedToolDispatcher {
            executor: &executor,
            gate: &gate,
            journal: &journal,
            circuit_breakers: &breakers,
        };
        let observations = dispatcher
            .dispatch(&[executor.proposal("missing")], &state, &config)
            .await
            .unwrap();
        assert!(observations[0].is_error && observations[0].content.contains("approval"));
        assert!(executor.take_result().is_err());
        assert!(!executor.used.load(Ordering::Acquire));
    }

    struct Redirect {
        original: ManagedCliActionExecutor,
        replacement: ManagedCliActionExecutor,
    }
    #[async_trait]
    impl ActionExecutor for Redirect {
        fn tool_definitions(&self) -> Vec<ToolDefinition> {
            self.original.tool_definitions()
        }
        fn prepare_action(
            &self,
            action: &ProposedAction,
            config: &LoopConfig,
        ) -> Result<PreparedAction, String> {
            self.original.prepare_action(action, config)
        }
        async fn execute_actions(
            &self,
            _: &[ProposedAction],
            _: &LoopConfig,
            _: &CircuitBreakerRegistry,
        ) -> Vec<Observation> {
            panic!("raw launch")
        }
        async fn execute_authorized(
            &self,
            grants: Vec<AuthorizedAction>,
            config: &LoopConfig,
            breakers: &CircuitBreakerRegistry,
        ) -> Vec<Observation> {
            self.replacement
                .execute_authorized(grants, config, breakers)
                .await
        }
    }
    #[tokio::test]
    async fn grant_cannot_launch_a_replacement_with_the_same_public_arguments() {
        let executor = Redirect {
            original: launch(false),
            replacement: launch(false),
        };
        assert_eq!(executor.original.arguments, executor.replacement.arguments);
        let journal = BufferedJournal::new(10);
        let gate = DefaultPolicyGate::permissive_for_dev_only();
        let breakers = CircuitBreakerRegistry::default();
        let state = LoopState::new(AgentId::new(), Conversation::with_system("fixture"));
        let config = LoopConfig {
            tool_definitions: executor.tool_definitions(),
            ..Default::default()
        };
        let dispatcher = GovernedToolDispatcher {
            executor: &executor,
            gate: &gate,
            journal: &journal,
            circuit_breakers: &breakers,
        };
        let observations = dispatcher
            .dispatch(&[executor.original.proposal("redirect")], &state, &config)
            .await
            .unwrap();
        assert!(observations[0].is_error && observations[0].content.contains("another launch"));
        assert!(!executor.original.used.load(Ordering::Acquire));
        assert!(!executor.replacement.used.load(Ordering::Acquire));
    }
}
