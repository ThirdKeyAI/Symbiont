//! A fixed-principal tool session for untrusted broker clients.
//!
//! Clients submit only a call identity, registered tool name and arguments.
//! The runtime owns policy, registry, trusted context, deadlines and journal.

use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, OnceLock,
    },
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use tokio::sync::{watch, Mutex};

use super::{
    circuit_breaker::CircuitBreakerRegistry,
    dispatch::GovernedToolDispatcher,
    executor::{ActionExecutor, ExecutionRunGuard},
    inference::ToolDefinition,
    loop_types::{
        JournalEntry, JournalError, JournalWriter, LoopConfig, LoopEvent, LoopState, Observation,
        ProposedAction, TerminationReason,
    },
    policy_bridge::ReasoningPolicyGate,
    prepared::{digest_json, execution_run_key},
};

const MAX_CALL_BYTES: usize = 1024 * 1024;

/// This deliberately has no principal, approval, policy or sandbox fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerToolCall {
    pub call_id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

struct Run {
    state: LoopState,
    seen: HashSet<String>,
    attempts: u32,
    guard: Option<ExecutionRunGuard>,
    host_guard: Option<ExecutionRunGuard>,
}

/// Serialize tool requests through the same prepared-call dispatcher as ORGA.
/// No child-supplied context participates in the principal or grant binding.
pub struct GovernedToolSession {
    executor: Arc<dyn ActionExecutor>,
    gate: Arc<dyn ReasoningPolicyGate>,
    journal: Arc<dyn JournalWriter>,
    config: LoopConfig,
    registry_digest: String,
    breakers: CircuitBreakerRegistry,
    run: Mutex<Run>,
    run_key: String,
    deadline: Instant,
    cancelled: watch::Sender<bool>,
    poisoned: Arc<AtomicBool>,
    failure_reason: std::sync::Mutex<Option<&'static str>>,
    host_executor: OnceLock<Arc<dyn ActionExecutor>>,
    host_active: AtomicBool,
    host_done: tokio::sync::Notify,
}

impl GovernedToolSession {
    /// Record the session before accepting a client. Journal durability and
    /// protection are responsibilities of the supplied trusted writer.
    pub async fn start(
        executor: Arc<dyn ActionExecutor>,
        gate: Arc<dyn ReasoningPolicyGate>,
        journal: Arc<dyn JournalWriter>,
        mut state: LoopState,
        mut config: LoopConfig,
    ) -> Result<Self, String> {
        if config.max_iterations == 0
            || config.max_iterations > 1000
            || config.timeout.is_zero()
            || config.timeout > Duration::from_secs(86400)
            || config.tool_timeout.is_zero()
        {
            return Err("invalid broker session bounds".into());
        }
        executor.validate_configuration()?;
        for (key, value) in executor.execution_context() {
            if state
                .trusted_context
                .get(&key)
                .is_some_and(|current| current != &value)
            {
                return Err(
                    "broker context conflicts with its executor's retained contract".into(),
                );
            }
            state.trusted_context.insert(key, value);
        }
        let registry = executor.tool_definitions();
        let mut names = HashSet::new();
        if registry.is_empty()
            || registry.len() > 256
            || registry
                .iter()
                .any(|tool| !valid_identifier(&tool.name) || !names.insert(tool.name.clone()))
        {
            return Err("broker requires a nonempty, unambiguous tool registry".into());
        }
        let registry_value = serde_json::to_value(&registry).map_err(|e| e.to_string())?;
        if serde_json::to_vec(&registry_value)
            .map_err(|e| e.to_string())?
            .len()
            > MAX_CALL_BYTES
        {
            return Err("broker tool registry exceeds its size limit".into());
        }
        let registry_digest = digest_json(&registry_value)?;
        let journal: Arc<dyn JournalWriter> = Arc::new(MeteredJournal {
            writer: journal,
            bytes: AtomicUsize::new(0),
        });
        if config.tool_definitions.is_empty() {
            config.tool_definitions = registry;
        } else {
            let mut allowed = HashSet::new();
            for requested in &config.tool_definitions {
                if !allowed.insert(&requested.name)
                    || !registry.iter().any(|tool| {
                        serde_json::to_value(tool).ok() == serde_json::to_value(requested).ok()
                    })
                {
                    return Err("broker tool subset differs from its registered contracts".into());
                }
            }
        }
        let remaining = config
            .timeout
            .saturating_sub(state.elapsed().to_std().unwrap_or(Duration::ZERO));
        if remaining.is_zero() {
            return Err("broker session already expired".into());
        }
        let deadline = Instant::now()
            .checked_add(remaining)
            .ok_or("broker deadline overflow")?;
        let run_key = execution_run_key(&state);
        let guard = ExecutionRunGuard::new(executor.clone(), &state, &config)?;
        let started = LoopEvent::Started {
            agent_id: state.agent_id,
            config: Box::new(config.clone()),
            execution_context: state.trusted_context.clone(),
        };
        tokio::time::timeout(
            remaining.min(Duration::from_secs(10)),
            append(journal.as_ref(), &state, started),
        )
        .await
        .map_err(|_| "broker start journal timed out")??;
        let (cancelled, _) = watch::channel(false);
        Ok(Self {
            executor,
            gate,
            journal,
            config,
            registry_digest,
            breakers: CircuitBreakerRegistry::default(),
            run: Mutex::new(Run {
                state,
                seen: HashSet::new(),
                attempts: 0,
                guard: Some(guard),
                host_guard: None,
            }),
            run_key,
            deadline,
            cancelled,
            poisoned: Arc::new(AtomicBool::new(false)),
            failure_reason: std::sync::Mutex::new(None),
            host_executor: OnceLock::new(),
            host_active: AtomicBool::new(false),
            host_done: tokio::sync::Notify::new(),
        })
    }

    pub fn tool_definitions(&self) -> &[ToolDefinition] {
        &self.config.tool_definitions
    }

    #[cfg(all(unix, feature = "cli-executor"))]
    pub(crate) fn external_guard(&self) -> InFlight<'_> {
        InFlight {
            session: self,
            armed: true,
        }
    }

    #[cfg(all(unix, feature = "cli-executor"))]
    pub(crate) async fn record_external(&self, event: LoopEvent) -> Result<(), String> {
        let run = self.run.lock().await;
        if run.guard.is_none()
            || self.poisoned.load(Ordering::Acquire)
            || *self.cancelled.borrow()
            || Instant::now() >= self.deadline
        {
            return Err("broker session cannot record new effects".into());
        }
        append(self.journal.as_ref(), &run.state, event).await
    }

    #[cfg(all(unix, feature = "cli-executor"))]
    pub(crate) fn deadline(&self) -> Instant {
        self.deadline
    }

    #[cfg(all(unix, feature = "cli-executor"))]
    pub(crate) fn cancellation(&self) -> watch::Receiver<bool> {
        self.cancelled.subscribe()
    }

    /// Every new call is prepared, approved and journalled independently. A
    /// reused identity cannot replay a prior grant, including after a denial.
    pub async fn call(&self, request: BrokerToolCall) -> Result<Observation, String> {
        let mut stopped = self.cancelled.subscribe();
        let mut run = tokio::select! {
            biased;
            _ = stopped.wait_for(|stop| *stop) => return Err("broker session is closed".into()),
            _ = tokio::time::sleep_until(self.deadline.into()) => {
                self.cancel();
                return Err("broker session expired".into());
            },
            run = self.run.lock() => run,
        };
        if run.guard.is_none() || self.poisoned.load(Ordering::Acquire) {
            return Err("broker session cannot accept more calls".into());
        }
        if run.attempts >= self.config.max_iterations {
            self.cancel();
            return Err("broker call budget exhausted".into());
        }
        run.attempts += 1;
        if !valid_identifier(&request.call_id)
            || !valid_identifier(&request.name)
            || !request.arguments.is_object()
            || serde_json::to_vec(&request)
                .map_err(|e| e.to_string())?
                .len()
                > MAX_CALL_BYTES
        {
            return Err("invalid broker tool call".into());
        }
        if !run.seen.insert(request.call_id.clone()) {
            return Err("broker call identity was already used".into());
        }
        let mut in_flight = InFlight {
            session: self,
            armed: true,
        };
        let current =
            serde_json::to_value(self.executor.tool_definitions()).map_err(|e| e.to_string())?;
        if digest_json(&current)? != self.registry_digest {
            return Err("broker tool registry changed during the session".into());
        }
        run.state.iteration = run.attempts - 1;
        let action = ProposedAction::ToolCall {
            call_id: request.call_id,
            name: request.name,
            arguments: serde_json::to_string(&request.arguments).map_err(|e| e.to_string())?,
        };
        let dispatcher = GovernedToolDispatcher {
            executor: self.executor.as_ref(),
            gate: self.gate.as_ref(),
            journal: self.journal.as_ref(),
            circuit_breakers: &self.breakers,
        };
        let result = tokio::select! {
            biased;
            _ = stopped.wait_for(|stop| *stop) => Err("broker call cancelled; completion is unconfirmed".into()),
            _ = tokio::time::sleep_until(self.deadline.into()) => Err("broker session expired; completion is unconfirmed".into()),
            result = dispatcher.dispatch(std::slice::from_ref(&action), &run.state, &self.config) => result.map_err(|e| e.to_string()),
        }?;
        if result.len() != 1 {
            return Err("broker received an ambiguous tool result".into());
        }
        if result[0].has_unconfirmed_effect() {
            self.cancel_with_reason("tool effects are unconfirmed; reconciliation is required");
        }
        in_flight.complete();
        Ok(result.into_iter().next().unwrap())
    }

    /// Authorize and execute one host-owned admission while allowing the child
    /// to use its separate broker tool subset. This method is never exposed on
    /// the broker wire protocol. The session retains worker cleanup ownership.
    pub async fn dispatch_host_action(
        &self,
        executor: Arc<dyn ActionExecutor>,
        action: ProposedAction,
    ) -> Result<Vec<Observation>, String> {
        executor.validate_configuration()?;
        let mut config = self.config.clone();
        config.tool_definitions = executor.tool_definitions();
        config.tool_timeout = config.timeout;
        let state = {
            let mut run = self.run.lock().await;
            if run.guard.is_none()
                || run.attempts != 0
                || self.host_executor.get().is_some()
                || self.poisoned.load(Ordering::Acquire)
                || *self.cancelled.borrow()
                || Instant::now() >= self.deadline
            {
                return Err("broker cannot admit a host action".into());
            }
            let host_context = executor.execution_context();
            if run.state.trusted_context.get("source_policy") != host_context.get("source_policy") {
                return Err("host admission must retain the session's source policy".into());
            }
            for (key, value) in host_context {
                if run.state.trusted_context.get(&key) != Some(&value) {
                    return Err(
                        "host admission differs from the session's retained execution context"
                            .into(),
                    );
                }
            }
            let ProposedAction::ToolCall { call_id, .. } = &action else {
                return Err("host admission requires a tool action".into());
            };
            if !valid_identifier(call_id) || !run.seen.insert(call_id.clone()) {
                return Err("invalid or reused host admission identity".into());
            }
            let guard = ExecutionRunGuard::new(executor.clone(), &run.state, &config)?;
            self.host_executor
                .set(executor.clone())
                .map_err(|_| "host admission was already attempted")?;
            run.host_guard = Some(guard);
            self.host_active.store(true, Ordering::Release);
            run.state.clone()
        };
        let _active = HostOperation(self);
        let mut flight = InFlight {
            session: self,
            armed: true,
        };
        let mut stopped = self.cancelled.subscribe();
        let dispatcher = GovernedToolDispatcher {
            executor: executor.as_ref(),
            gate: self.gate.as_ref(),
            journal: self.journal.as_ref(),
            circuit_breakers: &self.breakers,
        };
        let result = tokio::select! {
            biased;
            _ = stopped.wait_for(|stop| *stop) => Err(format!("host admission cancelled: {}; completion is unconfirmed", self.failure_reason().unwrap_or("session interrupted"))),
            _ = tokio::time::sleep_until(self.deadline.into()) => {
                self.cancel_with_reason("host admission deadline expired");
                Err("host admission deadline expired; completion is unconfirmed".into())
            },
            result = dispatcher.dispatch(std::slice::from_ref(&action), &state, &config) => result.map_err(|error| error.to_string()),
        };
        if result
            .as_ref()
            .is_ok_and(|observations| observations.len() == 1 && !observations[0].is_error)
        {
            flight.complete();
        }
        result
    }

    /// Immediately reject queued/new calls and signal owned backend workers.
    pub fn cancel(&self) {
        // Stream, inference and admission owners can expire concurrently. Keep
        // the known deadline cause even if a generic cancellation arrives first,
        // while preserving any earlier specific failure.
        if Instant::now() >= self.deadline {
            if let Ok(mut stored) = self.failure_reason.lock() {
                stored.get_or_insert("broker session deadline expired");
            }
        }
        self.poisoned.store(true, Ordering::Release);
        self.stop();
    }

    /// Retain a fixed runtime diagnostic without copying upstream response data.
    pub(crate) fn cancel_with_reason(&self, reason: &'static str) {
        if let Ok(mut stored) = self.failure_reason.lock() {
            stored.get_or_insert(reason);
        }
        self.cancel();
    }

    fn failure_reason(&self) -> Option<&'static str> {
        self.failure_reason.lock().ok().and_then(|reason| *reason)
    }

    fn stop(&self) {
        self.cancelled.send_replace(true);
        self.executor.cancel_run(&self.run_key, self.deadline);
        if let Some(executor) = self.host_executor.get() {
            executor.cancel_run(&self.run_key, self.deadline);
        }
    }

    /// Close backend ownership and record a terminal checkpoint. Interrupted
    /// dispatch and cleanup failure cannot record successful completion.
    pub async fn close(&self) -> Result<(), String> {
        if Instant::now() >= self.deadline {
            self.cancel();
        }
        self.stop();
        loop {
            let done = self.host_done.notified();
            if !self.host_active.load(Ordering::Acquire) {
                break;
            }
            done.await;
        }
        let mut run = self.run.lock().await;
        let Some(guard) = run.guard.take() else {
            return Err("broker session was already closed".into());
        };
        let tools_cleanup = guard.close().await;
        let host_cleanup = match run.host_guard.take() {
            Some(guard) => guard.close().await,
            None => Ok(()),
        };
        let cleanup = tools_cleanup.and(host_cleanup);
        let failed = self.poisoned.load(Ordering::Acquire) || cleanup.is_err();
        let reason = if failed {
            TerminationReason::Error {
                message: self
                    .failure_reason()
                    .unwrap_or("broker interrupted or failed; inspect recorded call outcomes")
                    .into(),
            }
        } else {
            TerminationReason::Completed
        };
        let event = LoopEvent::Terminated {
            reason,
            iterations: run.attempts,
            total_usage: run.state.total_usage.clone(),
            duration: run.state.elapsed().to_std().unwrap_or(Duration::ZERO),
        };
        let recorded = tokio::time::timeout(
            Duration::from_secs(5),
            append(self.journal.as_ref(), &run.state, event),
        )
        .await
        .map_err(|_| "broker terminal journal timed out".to_string())?;
        cleanup?;
        recorded?;
        if failed {
            return Err("broker session did not complete cleanly".into());
        }
        Ok(())
    }
}

struct HostOperation<'a>(&'a GovernedToolSession);
impl Drop for HostOperation<'_> {
    fn drop(&mut self) {
        self.0.host_active.store(false, Ordering::Release);
        self.0.host_done.notify_waiters();
    }
}

impl Drop for GovernedToolSession {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub(crate) struct InFlight<'a> {
    session: &'a GovernedToolSession,
    armed: bool,
}
impl InFlight<'_> {
    pub(crate) fn complete(&mut self) {
        self.armed = false;
    }
}
impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.session.poisoned.store(true, Ordering::Release);
            self.session.cancel();
        }
    }
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-.:/".contains(&byte))
}

/// Bound even an entry-count-only memory writer against adversarial output.
/// Keep one entry's capacity available for an interrupted terminal checkpoint.
struct MeteredJournal {
    writer: Arc<dyn JournalWriter>,
    bytes: AtomicUsize,
}
#[async_trait::async_trait]
impl JournalWriter for MeteredJournal {
    fn audit_reference(&self) -> Option<super::run_audit::RunAuditReference> {
        self.writer.audit_reference()
    }

    // `fetch_update` is deprecated for `try_update` from Rust 1.99, which is
    // past this workspace's 1.89 MSRV. Switch the call once the floor moves.
    #[allow(deprecated)]
    async fn append(&self, entry: JournalEntry) -> Result<(), JournalError> {
        let size = serde_json::to_vec(&entry)
            .map_err(|e| JournalError::WriteFailed(e.to_string()))?
            .len();
        let limit = if matches!(&entry.event, LoopEvent::Terminated { .. }) {
            64
        } else {
            63
        } * MAX_CALL_BYTES;
        if size > MAX_CALL_BYTES
            || self
                .bytes
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                    used.checked_add(size).filter(|next| *next <= limit)
                })
                .is_err()
        {
            return Err(JournalError::WriteFailed(
                "broker journal capacity exceeded".into(),
            ));
        }
        self.writer.append(entry).await
    }
    async fn next_sequence(&self) -> u64 {
        self.writer.next_sequence().await
    }
}

async fn append(
    journal: &dyn JournalWriter,
    state: &LoopState,
    event: LoopEvent,
) -> Result<(), String> {
    journal
        .append(JournalEntry {
            sequence: journal.next_sequence().await,
            timestamp: chrono::Utc::now(),
            agent_id: state.agent_id,
            iteration: state.iteration,
            event,
        })
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod host_tests {
    use super::*;
    use crate::reasoning::{
        conversation::Conversation, loop_types::BufferedJournal, policy_bridge::DefaultPolicyGate,
    };
    use crate::types::AgentId;
    use async_trait::async_trait;
    use serde_json::json;

    struct Host {
        entered: tokio::sync::Notify,
        closing: tokio::sync::Notify,
        removed: tokio::sync::Notify,
        cancelled: AtomicBool,
    }
    #[async_trait]
    impl ActionExecutor for Host {
        fn tool_definitions(&self) -> Vec<ToolDefinition> {
            vec![ToolDefinition {
                name: "host_fixture".into(),
                description: "fixture".into(),
                parameters: json!({"type":"object"}),
            }]
        }
        fn cancel_run(&self, _: &str, _: Instant) {
            self.cancelled.store(true, Ordering::Release);
        }
        async fn close_run(&self, _: &str, _: Instant) -> Result<(), String> {
            self.closing.notify_one();
            self.removed.notified().await;
            Ok(())
        }
        async fn execute_actions(
            &self,
            _: &[ProposedAction],
            _: &LoopConfig,
            _: &CircuitBreakerRegistry,
        ) -> Vec<Observation> {
            self.entered.notify_one();
            std::future::pending().await
        }
    }
    struct Tools;
    #[async_trait]
    impl ActionExecutor for Tools {
        fn tool_definitions(&self) -> Vec<ToolDefinition> {
            vec![ToolDefinition {
                name: "tool_fixture".into(),
                description: "fixture".into(),
                parameters: json!({"type":"object"}),
            }]
        }
        async fn execute_actions(
            &self,
            _: &[ProposedAction],
            _: &LoopConfig,
            _: &CircuitBreakerRegistry,
        ) -> Vec<Observation> {
            panic!("unexpected tool")
        }
    }

    #[tokio::test]
    async fn deadline_cancellation_keeps_its_cause_and_preserves_earlier_failures() {
        for earlier in [
            None,
            Some("inference response contains protected credentials"),
        ] {
            let journal = Arc::new(BufferedJournal::new(20));
            let mut session = GovernedToolSession::start(
                Arc::new(Tools),
                Arc::new(DefaultPolicyGate::permissive_for_dev_only()),
                journal.clone(),
                LoopState::new(AgentId::new(), Conversation::with_system("fixture")),
                LoopConfig::default(),
            )
            .await
            .unwrap();
            if let Some(reason) = earlier {
                session.cancel_with_reason(reason);
            }
            session.deadline = Instant::now();
            session.cancel();
            assert!(session.close().await.is_err());
            let entries = journal.entries().await;
            let LoopEvent::Terminated {
                reason: TerminationReason::Error { message },
                ..
            } = &entries.last().unwrap().event
            else {
                panic!("expected an audited failure after cleanup");
            };
            assert_eq!(
                message,
                earlier.unwrap_or("broker session deadline expired")
            );
        }
    }

    #[tokio::test]
    async fn host_admission_cannot_omit_or_replace_the_retained_source_policy() {
        use crate::reasoning::source_policy::SourcePolicyExecutor;
        let source = dsl::ExecutionPolicy::parse("agent fixture() {}", "fixture").unwrap();
        let session = GovernedToolSession::start(
            Arc::new(SourcePolicyExecutor::new(Arc::new(Tools), source)),
            Arc::new(DefaultPolicyGate::permissive_for_dev_only()),
            Arc::new(BufferedJournal::new(20)),
            LoopState::new(AgentId::new(), Conversation::with_system("fixture")),
            LoopConfig::default(),
        )
        .await
        .unwrap();
        let plain: Arc<dyn ActionExecutor> = Arc::new(Host {
            entered: Default::default(),
            closing: Default::default(),
            removed: Default::default(),
            cancelled: AtomicBool::new(false),
        });
        let other: Arc<dyn ActionExecutor> = Arc::new(SourcePolicyExecutor::new(
            plain.clone(),
            dsl::ExecutionPolicy::parse("agent other() {}", "other").unwrap(),
        ));
        for executor in [plain, other] {
            let action = ProposedAction::ToolCall {
                name: "host_fixture".into(),
                call_id: "host".into(),
                arguments: "{}".into(),
            };
            let error = tokio::time::timeout(
                Duration::from_secs(1),
                session.dispatch_host_action(executor, action),
            )
            .await
            .unwrap()
            .unwrap_err();
            assert!(error.contains("retain the session's source policy"));
        }
        session.close().await.unwrap();
    }

    #[tokio::test]
    async fn close_waits_for_interrupted_host_cleanup_before_terminal_audit() {
        for abort_dispatch in [false, true] {
            let journal = Arc::new(BufferedJournal::new(20));
            let session = Arc::new(
                GovernedToolSession::start(
                    Arc::new(Tools),
                    Arc::new(DefaultPolicyGate::permissive_for_dev_only()),
                    journal.clone(),
                    LoopState::new(AgentId::new(), Conversation::with_system("fixture")),
                    LoopConfig::default(),
                )
                .await
                .unwrap(),
            );
            let host = Arc::new(Host {
                entered: Default::default(),
                closing: Default::default(),
                removed: Default::default(),
                cancelled: AtomicBool::new(false),
            });
            let active = session.clone();
            let executor = host.clone();
            let dispatch = tokio::spawn(async move {
                active
                    .dispatch_host_action(
                        executor,
                        ProposedAction::ToolCall {
                            name: "host_fixture".into(),
                            call_id: "host".into(),
                            arguments: "{}".into(),
                        },
                    )
                    .await
            });
            tokio::time::timeout(Duration::from_secs(3), host.entered.notified())
                .await
                .unwrap();
            if abort_dispatch {
                dispatch.abort();
            }
            let active = session.clone();
            let close = tokio::spawn(async move { active.close().await });
            tokio::time::timeout(Duration::from_secs(3), host.closing.notified())
                .await
                .unwrap();
            assert!(host.cancelled.load(Ordering::Acquire));
            assert!(!close.is_finished());
            assert!(!journal
                .entries()
                .await
                .iter()
                .any(|entry| matches!(entry.event, LoopEvent::Terminated { .. })));
            host.removed.notify_one();
            assert!(close.await.unwrap().is_err());
            let result = dispatch.await;
            assert!(result.is_err() || result.unwrap().is_err());
            assert!(matches!(
                journal.entries().await.last().unwrap().event,
                LoopEvent::Terminated {
                    reason: TerminationReason::Error { .. },
                    ..
                }
            ));
        }
    }
}
