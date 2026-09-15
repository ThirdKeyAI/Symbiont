//! Immutable calls carried from validation through policy and dispatch.

use std::any::Any;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::loop_types::{LoopConfig, LoopState, ProposedAction};

/// The executable contract identified by a prepared tool invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolContract {
    pub name: String,
    pub version: String,
    pub digest: String,
    pub action_type: String,
    pub action_id: String,
    pub resource_type: String,
    pub resource_id: String,
    pub requires_approval: bool,
}

/// Validated input and its executor-owned snapshot. This is not authorization.
/// Fields are immutable; a modification requires preparing a new invocation.
pub struct PreparedAction {
    action: ProposedAction,
    contract: Option<ToolContract>,
    fingerprint: String,
    resolved: Value,
    backend: Option<Arc<dyn Any + Send + Sync>>,
    pub(crate) source_policy: Option<Arc<super::source_policy::BoundSourcePolicy>>,
}

impl PreparedAction {
    pub fn new(action: ProposedAction, contract: Option<ToolContract>) -> Result<Self, String> {
        let fingerprint =
            digest_json(&serde_json::json!({"action": action, "contract": contract}))?;
        Ok(Self {
            action,
            contract,
            fingerprint,
            resolved: serde_json::json!({}),
            backend: None,
            source_policy: None,
        })
    }

    /// Record the resolved effect descriptor (for example argv or a URL),
    /// binding generated values as well as normalized model arguments.
    pub fn with_resolved(mut self, resolved: Value) -> Result<Self, String> {
        self.fingerprint = digest_json(
            &serde_json::json!({"action": self.action, "contract": self.contract, "resolved": resolved}),
        )?;
        self.resolved = resolved;
        if let Some(policy) = &self.source_policy {
            self.fingerprint = digest_json(
                &serde_json::json!({"call": self.fingerprint, "source_policy": policy.metadata()}),
            )?;
        }
        Ok(self)
    }

    pub(crate) fn with_source_policy(
        mut self,
        policy: Arc<super::source_policy::BoundSourcePolicy>,
    ) -> Result<Self, String> {
        if self.source_policy.is_some() {
            return Err("multiple source policy executors are unsupported".into());
        }
        self.source_policy = Some(policy);
        let resolved = self.resolved.clone();
        self.with_resolved(resolved)
    }

    pub(crate) fn check_source_policy(&self, state: &LoopState) -> Result<(), String> {
        self.source_policy
            .as_ref()
            .map_or(Ok(()), |policy| policy.check(self, state))
    }

    /// Compare a freshly normalized backend call while retaining the already
    /// authorized source restriction. A changed or nested restriction fails.
    pub fn matches_authorized_call(self, authorized: &PreparedAction) -> Result<bool, String> {
        let current = if let Some(policy) = &authorized.source_policy {
            if let Some(own) = &self.source_policy {
                if !Arc::ptr_eq(own, policy) {
                    return Ok(false);
                }
                self
            } else {
                self.with_source_policy(policy.clone())?
            }
        } else {
            self
        };
        Ok(current.fingerprint == authorized.fingerprint)
    }

    /// Attach executor-owned, immutable dispatch state, such as a manifest and
    /// rendered argv. Authorization cannot change or replace this state.
    pub fn with_backend<T: Any + Send + Sync>(mut self, backend: T) -> Self {
        self.backend = Some(Arc::new(backend));
        self
    }

    pub fn action(&self) -> &ProposedAction {
        &self.action
    }
    pub fn contract(&self) -> Option<&ToolContract> {
        self.contract.as_ref()
    }
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
    pub fn backend<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.backend.as_deref()?.downcast_ref()
    }

    pub fn policy_context(&self) -> Value {
        let arguments = match &self.action {
            ProposedAction::ToolCall { arguments, .. } => {
                serde_json::from_str(arguments).unwrap_or(Value::Null)
            }
            _ => Value::Null,
        };
        let mut context = serde_json::json!({"fingerprint": self.fingerprint, "action": self.action, "arguments": arguments, "contract": self.contract, "resolved": self.resolved});
        if let Some(policy) = &self.source_policy {
            context["source_policy"] = policy.metadata().clone();
        }
        context
    }
}

/// An in-memory, single-use receipt from the configured approval relay.
/// Only trusted runtime relay code can construct it; caller-supplied context
/// booleans are never approval credentials.
#[derive(Serialize)]
pub struct ApprovalReceipt {
    id: uuid::Uuid,
    fingerprint: String,
    binding: String,
    expires_at_utc: chrono::DateTime<chrono::Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resolution: Option<crate::escalation::AuditEvent>,
    #[serde(skip)]
    expires_at: Instant,
}

impl ApprovalReceipt {
    pub(crate) fn issue(
        prepared: &PreparedAction,
        state: &LoopState,
        config: &LoopConfig,
        lifetime: Duration,
        id: uuid::Uuid,
    ) -> Result<Self, String> {
        let remaining = config
            .timeout
            .saturating_sub(state.elapsed().to_std().unwrap_or(Duration::ZERO));
        let lifetime = lifetime.min(remaining).min(config.tool_timeout);
        if lifetime.is_zero() {
            return Err("approval expired before grant".into());
        }
        Ok(Self {
            id,
            resolution: None,
            fingerprint: prepared.fingerprint().into(),
            binding: state_binding(state, config)?,
            expires_at_utc: chrono::Utc::now()
                .checked_add_signed(
                    chrono::Duration::from_std(lifetime)
                        .map_err(|_| "approval lifetime exceeds supported duration")?,
                )
                .ok_or("approval expiration exceeds supported date range")?,
            expires_at: Instant::now()
                .checked_add(lifetime)
                .ok_or("approval deadline overflow")?,
        })
    }

    pub(crate) fn with_resolution(
        mut self,
        resolution: crate::escalation::AuditEvent,
        agent_id: crate::types::AgentId,
    ) -> Result<Self, String> {
        if resolution.agent_id != agent_id.to_string()
            || !matches!(
                resolution.decision,
                crate::escalation::Decision::Approve { .. }
            )
        {
            return Err("approval resolution does not approve this principal".into());
        }
        self.resolution = Some(resolution);
        Ok(self)
    }

    pub(super) fn validate(
        &self,
        prepared: &PreparedAction,
        state: &LoopState,
        config: &LoopConfig,
    ) -> Result<(), String> {
        if self.fingerprint != prepared.fingerprint()
            || self.binding != state_binding(state, config)?
        {
            return Err("approval does not match this call, principal, session, or context".into());
        }
        if Instant::now() >= self.expires_at {
            return Err("approval expired".into());
        }
        Ok(())
    }
}

/// A non-cloneable, non-deserializable policy grant. Only the phase gate can
/// construct one. Consuming it binds dispatch to its original prepared call.
pub struct AuthorizedAction {
    prepared: PreparedAction,
    principal: crate::types::AgentId,
    binding: String,
    run_key: String,
    session_binding: String,
    run_deadline: Instant,
    expires_at: Instant,
    approval: Option<ApprovalReceipt>,
    iteration: u32,
    effect_journal: Option<super::effect_journal::EffectJournal>,
}

impl AuthorizedAction {
    pub(super) fn issue(
        prepared: PreparedAction,
        state: &LoopState,
        config: &LoopConfig,
        approval: Option<ApprovalReceipt>,
    ) -> Result<Self, String> {
        prepared.check_source_policy(state)?;
        if prepared
            .contract()
            .is_some_and(|contract| contract.requires_approval)
            && approval.is_none()
        {
            return Err("manifest requires an exact-call approval receipt".into());
        }
        if let Some(receipt) = &approval {
            receipt.validate(&prepared, state, config)?;
        }
        let elapsed = state.elapsed().to_std().unwrap_or(Duration::ZERO);
        let remaining = config.timeout.saturating_sub(elapsed);
        let budget = if matches!(
            prepared.action(),
            ProposedAction::ToolCall { .. } | ProposedAction::Delegate { .. }
        ) {
            remaining.min(config.tool_timeout)
        } else {
            remaining
        };
        if budget.is_zero() {
            return Err("authorization budget exhausted".into());
        }
        let expires_at = Instant::now()
            .checked_add(budget)
            .ok_or("authorization deadline overflow")?;
        Ok(Self {
            prepared,
            iteration: state.iteration,
            effect_journal: None,
            principal: state.agent_id,
            binding: state_binding(state, config)?,
            run_key: execution_run_key(state),
            session_binding: digest_json(&serde_json::json!({
                "run": execution_run_key(state), "trusted_context": state.trusted_context, "config": config,
            }))?,
            run_deadline: Instant::now()
                .checked_add(remaining)
                .ok_or("run deadline overflow")?,
            expires_at: approval
                .as_ref()
                .map_or(expires_at, |receipt| expires_at.min(receipt.expires_at)),
            approval,
        })
    }

    pub fn audit_context(&self) -> Value {
        let mut context = self.prepared.policy_context();
        context["approval"] = serde_json::to_value(&self.approval).unwrap_or(Value::Null);
        context["authorization_binding"] = Value::String(self.binding.clone());
        context
    }

    pub fn prepared(&self) -> &PreparedAction {
        &self.prepared
    }
    /// The runtime principal covered by this action's authorization binding.
    pub fn principal(&self) -> crate::types::AgentId {
        self.principal
    }
    pub(super) fn iteration(&self) -> u32 {
        self.iteration
    }
    pub(super) fn attach_effect_journal(
        &mut self,
        sender: tokio::sync::mpsc::Sender<super::effect_journal::Record>,
    ) {
        self.effect_journal = Some(super::effect_journal::EffectJournal::new(sender, self));
    }
    pub(crate) fn effect_journal(&self) -> Option<super::effect_journal::EffectJournal> {
        self.effect_journal.clone()
    }
    pub fn action(&self) -> &ProposedAction {
        self.prepared.action()
    }

    pub(super) fn check_binding(
        &self,
        state: &LoopState,
        config: &LoopConfig,
    ) -> Result<(), String> {
        if self.binding != state_binding(state, config)? {
            return Err("authorization context changed before dispatch".into());
        }
        self.check_live()
    }

    pub(crate) fn run_key(&self) -> &str {
        &self.run_key
    }
    pub(crate) fn session_binding(&self) -> &str {
        &self.session_binding
    }
    pub(crate) fn run_deadline(&self) -> Instant {
        self.run_deadline
    }

    pub fn deadline(&self) -> Instant {
        self.expires_at
    }

    pub fn check_live(&self) -> Result<(), String> {
        if Instant::now() >= self.expires_at {
            return Err("authorization expired before dispatch".into());
        }
        Ok(())
    }

    pub fn into_prepared(self) -> Result<PreparedAction, String> {
        self.check_live()?;
        Ok(self.prepared)
    }
}

/// Stable run identity; iteration and model-controlled metadata do not select
/// an interactive worker. The context/configuration binding remains separate.
pub(crate) fn execution_run_key(state: &LoopState) -> String {
    format!("{}:{}", state.agent_id, state.started_at.to_rfc3339())
}

fn state_binding(state: &LoopState, config: &LoopConfig) -> Result<String, String> {
    digest_json(&serde_json::json!({
        "agent_id": state.agent_id,
        "started_at": state.started_at,
        "iteration": state.iteration,
        "trusted_context": state.trusted_context,
        "config": config,
    }))
}

/// Canonical object ordering keeps contract identities stable across map
/// insertion order and serde_json feature combinations.
pub fn canonical_json(value: &Value) -> Result<String, String> {
    fn ordered(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let sorted: std::collections::BTreeMap<_, _> =
                    map.iter().map(|(k, v)| (k.clone(), ordered(v))).collect();
                Value::Object(sorted.into_iter().collect())
            }
            Value::Array(values) => Value::Array(values.iter().map(ordered).collect()),
            value => value.clone(),
        }
    }
    serde_json::to_string(&ordered(value)).map_err(|e| e.to_string())
}

pub fn digest_json(value: &Value) -> Result<String, String> {
    Ok(format!(
        "sha256:{}",
        hex::encode(Sha256::digest(canonical_json(value)?.as_bytes()))
    ))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::reasoning::circuit_breaker::CircuitBreakerRegistry;
    use crate::reasoning::conversation::Conversation;
    use crate::reasoning::executor::ActionExecutor;
    use crate::toolclad::{Manifest, ToolCladExecutor};
    use crate::types::AgentId;

    fn fixture(
        path: &std::path::Path,
        approval: bool,
    ) -> (ToolCladExecutor, Manifest, LoopConfig, ProposedAction) {
        let mut manifest: Manifest = toml::from_str(
            r#"
[tool]
name = "prepared_fixture"
version = "1"
binary = "/usr/bin/touch"
description = "Prepared capability fixture"
[args.value]
position = 1
required = true
type = "string"
[command]
template = "/usr/bin/touch {value}"
[output]
format = "text"
"#,
        )
        .unwrap();
        manifest.tool.human_approval = approval;
        let executor = ToolCladExecutor::new(vec![("prepared_fixture".into(), manifest.clone())])
            .with_development_host_execution();
        let config = LoopConfig {
            tool_definitions: executor.tool_definitions(),
            ..Default::default()
        };
        let action = ProposedAction::ToolCall {
            call_id: "call".into(),
            name: "prepared_fixture".into(),
            arguments: serde_json::json!({"value": path}).to_string(),
        };
        (executor, manifest, config, action)
    }

    #[tokio::test]
    async fn prepared_grant_cannot_be_dispatched_by_a_replacement_executor() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("effect");
        let (executor, mut manifest, config, action) = fixture(&marker, false);
        let state = LoopState::new(AgentId::new(), Conversation::new());
        let prepared = executor.prepare_action(&action, &config).unwrap();
        let grant = AuthorizedAction::issue(prepared, &state, &config, None).unwrap();
        manifest.tool.version = "2".into();
        let replacement = ToolCladExecutor::new(vec![("prepared_fixture".into(), manifest)])
            .with_development_host_execution();
        let observations = replacement
            .execute_authorized(vec![grant], &config, &CircuitBreakerRegistry::default())
            .await;
        assert!(observations[0].is_error);
        assert!(observations[0].content.contains("another executor"));
        assert!(!marker.exists());
    }

    #[tokio::test]
    async fn approval_receipts_reject_changed_calls_identity_context_and_expiry() {
        for change in [
            "arguments",
            "principal",
            "session",
            "context",
            "configuration",
            "expiry",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let marker = dir.path().join("effect");
            let (executor, _, mut config, action) = fixture(&marker, true);
            let mut state = LoopState::new(AgentId::new(), Conversation::new());
            let mut prepared = executor.prepare_action(&action, &config).unwrap();
            let receipt = ApprovalReceipt::issue(
                &prepared,
                &state,
                &config,
                if change == "expiry" {
                    Duration::from_millis(1)
                } else {
                    Duration::from_secs(30)
                },
                uuid::Uuid::new_v4(),
            )
            .unwrap();
            match change {
                "arguments" => {
                    let modified = ProposedAction::ToolCall {
                        call_id: "call".into(),
                        name: "prepared_fixture".into(),
                        arguments: serde_json::json!({"value":dir.path().join("substituted")})
                            .to_string(),
                    };
                    prepared = executor.prepare_action(&modified, &config).unwrap();
                }
                "principal" => state.agent_id = AgentId::new(),
                "session" => state.started_at += chrono::Duration::seconds(1),
                "context" => {
                    state
                        .trusted_context
                        .insert("access".into(), serde_json::json!("changed"));
                }
                "configuration" => config.tool_timeout += Duration::from_secs(1),
                "expiry" => tokio::time::sleep(Duration::from_millis(5)).await,
                _ => unreachable!(),
            }
            assert!(
                AuthorizedAction::issue(prepared, &state, &config, Some(receipt)).is_err(),
                "{change}"
            );
            assert!(!marker.exists());
        }
    }

    #[tokio::test]
    async fn scoped_custom_aliases_are_enforced_and_cycles_are_rejected() {
        use crate::toolclad::manifest::ArgDef;
        use std::collections::HashMap;
        let dir = tempfile::tempdir().unwrap();
        let (_, mut manifest, _, _) = fixture(&dir.path().join("unused"), false);
        manifest.args.get_mut("value").unwrap().type_name = "scoped_host".into();
        manifest.command.template = Some(format!(
            "/usr/bin/touch '{}/{{value}}'",
            dir.path().display()
        ));
        let custom = HashMap::from([(
            "scoped_host".into(),
            ArgDef {
                type_name: "scope_target".into(),
                ..Default::default()
            },
        )]);
        let executor = ToolCladExecutor::with_custom_types(
            vec![("prepared_fixture".into(), manifest.clone())],
            custom,
        )
        .with_development_host_execution();
        let config = LoopConfig {
            tool_definitions: executor.tool_definitions(),
            ..Default::default()
        };
        let action = ProposedAction::ToolCall {
            call_id: "call".into(),
            name: "prepared_fixture".into(),
            arguments: r#"{"value":"example.com"}"#.into(),
        };
        assert!(executor.prepare_action(&action, &config).is_err());
        let scope = crate::toolclad::Scope {
            targets: Vec::new(),
            domains: vec!["example.com".into()],
            exclude: Vec::new(),
        };
        let executor = executor.with_scope(scope);
        let state = LoopState::new(AgentId::new(), Conversation::new());
        let grant = AuthorizedAction::issue(
            executor.prepare_action(&action, &config).unwrap(),
            &state,
            &config,
            None,
        )
        .unwrap();
        let observations = executor
            .execute_authorized(vec![grant], &config, &CircuitBreakerRegistry::default())
            .await;
        assert!(!observations[0].is_error, "{observations:?}");
        assert!(dir.path().join("example.com").exists());
        let cycle = HashMap::from([(
            "scoped_host".into(),
            ArgDef {
                type_name: "scoped_host".into(),
                ..Default::default()
            },
        )]);
        let executor =
            ToolCladExecutor::with_custom_types(vec![("prepared_fixture".into(), manifest)], cycle);
        let error = executor
            .prepare_action(&action, &config)
            .err()
            .expect("cyclic type must fail");
        assert!(error.contains("cyclic"));
    }

    #[tokio::test]
    async fn unknown_and_additional_arguments_fail_before_any_effect() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("effect");
        let (executor, _, config, action) = fixture(&marker, false);
        let mut hidden = config.clone();
        hidden.tool_definitions.clear();
        assert!(executor.prepare_action(&action, &hidden).is_err());
        let extra = ProposedAction::ToolCall {
            call_id: "call".into(),
            name: "prepared_fixture".into(),
            arguments: serde_json::json!({"value":marker,"unexpected":"argument"}).to_string(),
        };
        assert!(executor.prepare_action(&extra, &config).is_err());
        assert!(!marker.exists());
    }

    #[test]
    fn persistent_session_binding_tracks_owner_and_context_across_iterations() {
        let mut state = LoopState::new(AgentId::new(), Conversation::new());
        let mut config = LoopConfig::default();
        let grant = |state: &LoopState, config: &LoopConfig| {
            AuthorizedAction::issue(
                PreparedAction::new(
                    ProposedAction::Respond {
                        content: "fixture".into(),
                    },
                    None,
                )
                .unwrap(),
                state,
                config,
                None,
            )
            .unwrap()
        };
        let first = grant(&state, &config);
        state.iteration += 1;
        let next = grant(&state, &config);
        assert_eq!(first.run_key(), next.run_key());
        assert_eq!(first.session_binding(), next.session_binding());
        assert_ne!(first.binding, next.binding);
        state
            .trusted_context
            .insert("scope".into(), serde_json::json!("changed"));
        assert_ne!(
            next.session_binding(),
            grant(&state, &config).session_binding()
        );
        state.trusted_context.clear();
        config.tool_timeout += Duration::from_secs(1);
        assert_ne!(
            next.session_binding(),
            grant(&state, &config).session_binding()
        );
        config.tool_timeout -= Duration::from_secs(1);
        let principal = state.agent_id;
        state.agent_id = AgentId::new();
        assert_ne!(next.run_key(), grant(&state, &config).run_key());
        state.agent_id = principal;
        state.started_at += chrono::Duration::milliseconds(1);
        assert_ne!(next.run_key(), grant(&state, &config).run_key());
    }

    #[test]
    fn session_scope_checks_the_complete_target_not_an_allowed_regex_prefix() {
        let manifest = toml::from_str(
            r#"
[tool]
name = "terminal"
version = "1"
description = "Scoped terminal fixture"
mode = "session"
[session]
startup_command = "fixture"
ready_pattern = "READY>"
[session.commands.connect]
pattern = 'connect (?P<target>allowed[.]example|allowed[.]example[.]evil)'
description = "Connect to a scoped host"
extract_target = true
[output]
format = "text"
"#,
        )
        .unwrap();
        let executor = ToolCladExecutor::new(vec![("terminal".into(), manifest)]).with_scope(
            crate::toolclad::Scope {
                targets: Vec::new(),
                domains: vec!["allowed.example".into()],
                exclude: Vec::new(),
            },
        );
        let config = LoopConfig {
            tool_definitions: executor.tool_definitions(),
            ..Default::default()
        };
        for (host, allowed) in [("allowed.example", true), ("allowed.example.evil", false)] {
            let action = ProposedAction::ToolCall {
                call_id: "fixture".into(),
                name: "terminal.connect".into(),
                arguments: serde_json::json!({"command":format!("connect {host}")}).to_string(),
            };
            assert_eq!(
                executor.prepare_action(&action, &config).is_ok(),
                allowed,
                "{host}"
            );
        }
    }
}
