//! Agent Lifecycle Controller
//!
//! Manages agent state transitions and execution lifecycle

use async_trait::async_trait;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::{mpsc, Notify};
use tokio::time::interval;

use crate::types::*;

/// Agent lifecycle controller trait
#[async_trait]
pub trait LifecycleController {
    /// Initialize a new agent
    async fn initialize_agent(&self, config: AgentConfig) -> Result<AgentId, LifecycleError>;

    /// Start an agent
    async fn start_agent(&self, agent_id: AgentId) -> Result<(), LifecycleError>;

    /// Suspend an agent
    async fn suspend_agent(&self, agent_id: AgentId) -> Result<(), LifecycleError>;

    /// Resume a suspended agent
    async fn resume_agent(&self, agent_id: AgentId) -> Result<(), LifecycleError>;

    /// Terminate an agent
    async fn terminate_agent(&self, agent_id: AgentId) -> Result<(), LifecycleError>;

    /// Get agent state
    async fn get_agent_state(&self, agent_id: AgentId) -> Result<AgentState, LifecycleError>;

    /// Get all agents in a specific state
    async fn get_agents_by_state(&self, state: AgentState) -> Vec<AgentId>;

    /// Shutdown the lifecycle controller
    async fn shutdown(&self) -> Result<(), LifecycleError>;

    /// Check the health of the lifecycle controller
    async fn check_health(&self) -> Result<ComponentHealth, LifecycleError>;
}

/// Lifecycle controller configuration
#[derive(Debug, Clone)]
pub struct LifecycleConfig {
    pub max_agents: usize,
    pub initialization_timeout: Duration,
    pub termination_timeout: Duration,
    pub state_check_interval: Duration,
    pub enable_auto_recovery: bool,
    pub max_restart_attempts: u32,
}

impl Default for LifecycleConfig {
    fn default() -> Self {
        Self {
            max_agents: 10000,
            initialization_timeout: Duration::from_secs(30),
            termination_timeout: Duration::from_secs(10),
            state_check_interval: Duration::from_secs(5),
            enable_auto_recovery: true,
            max_restart_attempts: 3,
        }
    }
}

/// Default implementation of the lifecycle controller
pub struct DefaultLifecycleController {
    config: LifecycleConfig,
    agents: Arc<RwLock<HashMap<AgentId, AgentInstance>>>,
    state_machine: Arc<AgentStateMachine>,
    event_sender: mpsc::UnboundedSender<LifecycleEvent>,
    shutdown_notify: Arc<Notify>,
    is_running: Arc<RwLock<bool>>,
}

impl DefaultLifecycleController {
    /// Create a new lifecycle controller
    pub async fn new(config: LifecycleConfig) -> Result<Self, LifecycleError> {
        let agents = Arc::new(RwLock::new(HashMap::new()));
        let state_machine = Arc::new(AgentStateMachine::new());
        let (event_sender, event_receiver) = mpsc::unbounded_channel();
        let shutdown_notify = Arc::new(Notify::new());
        let is_running = Arc::new(RwLock::new(true));

        let controller = Self {
            config,
            agents,
            state_machine,
            event_sender,
            shutdown_notify,
            is_running,
        };

        // Start background tasks
        controller.start_event_loop(event_receiver).await;
        controller.start_state_monitor().await;

        Ok(controller)
    }

    /// Start the event processing loop
    async fn start_event_loop(&self, mut event_receiver: mpsc::UnboundedReceiver<LifecycleEvent>) {
        let agents = self.agents.clone();
        let state_machine = self.state_machine.clone();
        let shutdown_notify = self.shutdown_notify.clone();
        let is_running = self.is_running.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    event = event_receiver.recv() => {
                        if !*is_running.read() {
                            break;
                        }
                        if let Some(event) = event {
                            Self::process_lifecycle_event(event, &agents, &state_machine).await;
                        } else {
                            break;
                        }
                    }
                    _ = shutdown_notify.notified() => {
                        break;
                    }
                }
            }
        });
    }

    /// Start the state monitoring loop
    async fn start_state_monitor(&self) {
        let agents = self.agents.clone();
        let state_machine = self.state_machine.clone();
        let shutdown_notify = self.shutdown_notify.clone();
        let is_running = self.is_running.clone();
        let check_interval = self.config.state_check_interval;
        let enable_auto_recovery = self.config.enable_auto_recovery;
        let max_restart_attempts = self.config.max_restart_attempts;

        tokio::spawn(async move {
            let mut interval = interval(check_interval);

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if !*is_running.read() {
                            break;
                        }

                        Self::monitor_agent_states(&agents, &state_machine, enable_auto_recovery, max_restart_attempts).await;
                    }
                    _ = shutdown_notify.notified() => {
                        break;
                    }
                }
            }
        });
    }

    /// Process a lifecycle event
    async fn process_lifecycle_event(
        event: LifecycleEvent,
        agents: &Arc<RwLock<HashMap<AgentId, AgentInstance>>>,
        state_machine: &Arc<AgentStateMachine>,
    ) {
        match event {
            LifecycleEvent::StateTransition {
                agent_id,
                from_state,
                to_state,
            } => {
                if let Some(agent) = agents.write().get_mut(&agent_id) {
                    if state_machine.is_valid_transition(&from_state, &to_state) {
                        agent.state = to_state.clone();
                        agent.last_state_change = SystemTime::now();

                        tracing::info!(
                            "Agent {} transitioned from {:?} to {:?}",
                            agent_id,
                            from_state,
                            to_state
                        );
                    } else {
                        tracing::error!(
                            "Invalid state transition for agent {}: {:?} -> {:?}",
                            agent_id,
                            from_state,
                            to_state
                        );
                    }
                }
            }
            LifecycleEvent::AgentError {
                agent_id,
                error,
                timestamp,
            } => {
                tracing::error!(
                    "Agent {} encountered error: {} at {:?}",
                    agent_id,
                    error,
                    timestamp
                );

                if let Some(agent) = agents.write().get_mut(&agent_id) {
                    agent.error_count += 1;
                    agent.last_error = Some(error);
                    // Validate the transition to Failed state
                    if state_machine.is_valid_transition(&agent.state, &AgentState::Failed) {
                        agent.state = AgentState::Failed;
                        agent.last_state_change = timestamp;
                    } else {
                        tracing::warn!(
                            "Cannot transition agent {} to Failed state from {:?}",
                            agent_id,
                            agent.state
                        );
                    }
                }
            }
            LifecycleEvent::ResourceExhausted {
                agent_id,
                resource_type,
                timestamp,
            } => {
                tracing::warn!(
                    "Agent {} exhausted resource {} at {:?}",
                    agent_id,
                    resource_type,
                    timestamp
                );

                if let Some(agent) = agents.write().get_mut(&agent_id) {
                    // Validate the transition to Suspended state
                    if state_machine.is_valid_transition(&agent.state, &AgentState::Suspended) {
                        agent.state = AgentState::Suspended;
                        agent.last_state_change = timestamp;
                    } else {
                        tracing::warn!(
                            "Cannot transition agent {} to Suspended state from {:?}",
                            agent_id,
                            agent.state
                        );
                    }
                }
            }
        }
    }

    /// Monitor agent states and perform auto-recovery if enabled
    async fn monitor_agent_states(
        agents: &Arc<RwLock<HashMap<AgentId, AgentInstance>>>,
        state_machine: &Arc<AgentStateMachine>,
        enable_auto_recovery: bool,
        max_restart_attempts: u32,
    ) {
        let mut agents_to_restart = Vec::new();
        let mut error_events = Vec::new();
        let mut resource_events = Vec::new();

        {
            let agents_read = agents.read();
            for (agent_id, agent) in agents_read.iter() {
                // Check for failed agents that can be restarted
                if enable_auto_recovery
                    && agent.state == AgentState::Failed
                    && agent.restart_count < max_restart_attempts
                {
                    agents_to_restart.push(*agent_id);
                }

                // Check for agents stuck in transitional states
                let time_in_state = SystemTime::now()
                    .duration_since(agent.last_state_change)
                    .unwrap_or_default();

                if time_in_state > Duration::from_secs(300) {
                    // 5 minutes
                    match agent.state {
                        AgentState::Initializing | AgentState::Terminating => {
                            tracing::warn!(
                                "Agent {} stuck in {:?} state for {:?}",
                                agent_id,
                                agent.state,
                                time_in_state
                            );
                            // Generate error event for stuck agents
                            error_events.push(LifecycleEvent::AgentError {
                                agent_id: *agent_id,
                                error: format!(
                                    "Agent stuck in {:?} state for {:?}",
                                    agent.state, time_in_state
                                ),
                                timestamp: SystemTime::now(),
                            });
                        }
                        _ => {}
                    }
                }

                // Check for resource exhaustion (simulate by high error count)
                if agent.error_count > 5 && agent.state == AgentState::Running {
                    resource_events.push(LifecycleEvent::ResourceExhausted {
                        agent_id: *agent_id,
                        resource_type: "error_threshold".to_string(),
                        timestamp: SystemTime::now(),
                    });
                }
            }
        }

        // Restart failed agents
        for agent_id in agents_to_restart {
            if let Some(agent) = agents.write().get_mut(&agent_id) {
                // Validate state transition before restarting
                if state_machine.is_valid_transition(&agent.state, &AgentState::Initializing) {
                    agent.restart_count += 1;
                    agent.state = AgentState::Initializing;
                    agent.last_state_change = SystemTime::now();

                    tracing::info!(
                        "Auto-restarting failed agent {} (attempt {})",
                        agent_id,
                        agent.restart_count
                    );
                } else {
                    tracing::warn!(
                        "Cannot restart agent {} from state {:?}",
                        agent_id,
                        agent.state
                    );
                }
            }
        }

        // Process error and resource exhaustion events
        for event in error_events {
            Self::process_lifecycle_event(event, agents, state_machine).await;
        }
        for event in resource_events {
            Self::process_lifecycle_event(event, agents, state_machine).await;
        }
    }

    /// Send a lifecycle event
    fn send_event(&self, event: LifecycleEvent) -> Result<(), LifecycleError> {
        self.event_sender
            .send(event)
            .map_err(|_| LifecycleError::EventProcessingFailed {
                reason: "Failed to send lifecycle event".to_string(),
            })
    }
}

#[async_trait]
impl LifecycleController for DefaultLifecycleController {
    async fn initialize_agent(&self, config: AgentConfig) -> Result<AgentId, LifecycleError> {
        if !*self.is_running.read() {
            return Err(LifecycleError::ShuttingDown);
        }

        let agents_count = self.agents.read().len();
        if agents_count >= self.config.max_agents {
            return Err(LifecycleError::ResourceExhausted {
                reason: format!(
                    "Agent slots exhausted: {} / {}",
                    agents_count, self.config.max_agents
                ),
            });
        }

        let agent_id = config.id;
        let instance = AgentInstance::new(config);

        // Add to agents map
        self.agents.write().insert(agent_id, instance);

        // Send state transition event
        self.send_event(LifecycleEvent::StateTransition {
            agent_id,
            from_state: AgentState::Created,
            to_state: AgentState::Initializing,
        })?;

        tracing::info!("Initialized agent {}", agent_id);
        Ok(agent_id)
    }

    async fn start_agent(&self, agent_id: AgentId) -> Result<(), LifecycleError> {
        let current_state = {
            let agents = self.agents.read();
            agents
                .get(&agent_id)
                .map(|agent| agent.state.clone())
                .ok_or(LifecycleError::AgentNotFound { agent_id })?
        };

        if !self
            .state_machine
            .is_valid_transition(&current_state, &AgentState::Running)
        {
            return Err(LifecycleError::InvalidStateTransition {
                from: format!("{:?}", current_state),
                to: format!("{:?}", AgentState::Running),
            });
        }

        self.send_event(LifecycleEvent::StateTransition {
            agent_id,
            from_state: current_state,
            to_state: AgentState::Running,
        })?;

        tracing::info!("Started agent {}", agent_id);
        Ok(())
    }

    async fn suspend_agent(&self, agent_id: AgentId) -> Result<(), LifecycleError> {
        let current_state = {
            let agents = self.agents.read();
            agents
                .get(&agent_id)
                .map(|agent| agent.state.clone())
                .ok_or(LifecycleError::AgentNotFound { agent_id })?
        };

        if !self
            .state_machine
            .is_valid_transition(&current_state, &AgentState::Suspended)
        {
            return Err(LifecycleError::InvalidStateTransition {
                from: format!("{:?}", current_state),
                to: format!("{:?}", AgentState::Suspended),
            });
        }

        self.send_event(LifecycleEvent::StateTransition {
            agent_id,
            from_state: current_state,
            to_state: AgentState::Suspended,
        })?;

        tracing::info!("Suspended agent {}", agent_id);
        Ok(())
    }

    async fn resume_agent(&self, agent_id: AgentId) -> Result<(), LifecycleError> {
        let current_state = {
            let agents = self.agents.read();
            agents
                .get(&agent_id)
                .map(|agent| agent.state.clone())
                .ok_or(LifecycleError::AgentNotFound { agent_id })?
        };

        if current_state != AgentState::Suspended {
            return Err(LifecycleError::InvalidStateTransition {
                from: format!("{:?}", current_state),
                to: format!("{:?}", AgentState::Running),
            });
        }

        self.send_event(LifecycleEvent::StateTransition {
            agent_id,
            from_state: current_state,
            to_state: AgentState::Running,
        })?;

        tracing::info!("Resumed agent {}", agent_id);
        Ok(())
    }

    async fn terminate_agent(&self, agent_id: AgentId) -> Result<(), LifecycleError> {
        let current_state = {
            let agents = self.agents.read();
            agents
                .get(&agent_id)
                .map(|agent| agent.state.clone())
                .ok_or(LifecycleError::AgentNotFound { agent_id })?
        };

        self.send_event(LifecycleEvent::StateTransition {
            agent_id,
            from_state: current_state,
            to_state: AgentState::Terminating,
        })?;

        // Simulate termination process
        tokio::time::sleep(Duration::from_millis(100)).await;

        self.send_event(LifecycleEvent::StateTransition {
            agent_id,
            from_state: AgentState::Terminating,
            to_state: AgentState::Terminated,
        })?;

        // Remove from agents map
        self.agents.write().remove(&agent_id);

        tracing::info!("Terminated agent {}", agent_id);
        Ok(())
    }

    async fn get_agent_state(&self, agent_id: AgentId) -> Result<AgentState, LifecycleError> {
        let agents = self.agents.read();
        agents
            .get(&agent_id)
            .map(|agent| agent.state.clone())
            .ok_or(LifecycleError::AgentNotFound { agent_id })
    }

    async fn get_agents_by_state(&self, state: AgentState) -> Vec<AgentId> {
        let agents = self.agents.read();
        agents
            .iter()
            .filter(|(_, agent)| agent.state == state)
            .map(|(id, _)| *id)
            .collect()
    }

    async fn shutdown(&self) -> Result<(), LifecycleError> {
        tracing::info!("Shutting down lifecycle controller");

        *self.is_running.write() = false;
        self.shutdown_notify.notify_waiters();

        // Terminate all running agents
        let agent_ids: Vec<AgentId> = self.agents.read().keys().copied().collect();

        for agent_id in agent_ids {
            if let Err(e) = self.terminate_agent(agent_id).await {
                tracing::error!(
                    "Failed to terminate agent {} during shutdown: {}",
                    agent_id,
                    e
                );
            }
        }

        Ok(())
    }

    async fn check_health(&self) -> Result<ComponentHealth, LifecycleError> {
        let is_running = *self.is_running.read();
        if !is_running {
            return Ok(ComponentHealth::unhealthy("Lifecycle controller is shut down".to_string()));
        }

        let agents = self.agents.read();
        let total_agents = agents.len();
        
        // Count agents by state
        let mut state_counts = std::collections::HashMap::new();
        let mut failed_count = 0;
        let mut stuck_count = 0;
        
        for agent in agents.values() {
            *state_counts.entry(agent.state.clone()).or_insert(0) += 1;
            
            if agent.state == AgentState::Failed {
                failed_count += 1;
            }
            
            // Check for stuck agents (in transitional states for too long)
            let time_in_state = SystemTime::now()
                .duration_since(agent.last_state_change)
                .unwrap_or_default();
            
            if time_in_state > Duration::from_secs(300) &&
               matches!(agent.state, AgentState::Initializing | AgentState::Terminating) {
                stuck_count += 1;
            }
        }

        let capacity_usage = total_agents as f64 / self.config.max_agents as f64;

        let status = if stuck_count > 0 {
            ComponentHealth::degraded(format!(
                "{} agents stuck in transitional states", stuck_count
            ))
        } else if failed_count > total_agents / 4 {
            ComponentHealth::degraded(format!(
                "High failure rate: {}/{} agents failed", failed_count, total_agents
            ))
        } else if capacity_usage > 0.9 {
            ComponentHealth::degraded(format!(
                "Near capacity: {}/{} agent slots used", total_agents, self.config.max_agents
            ))
        } else {
            ComponentHealth::healthy(Some(format!(
                "Managing {} agents across {} states", total_agents, state_counts.len()
            )))
        };

        let mut health = status
            .with_metric("total_agents".to_string(), total_agents.to_string())
            .with_metric("failed_agents".to_string(), failed_count.to_string())
            .with_metric("stuck_agents".to_string(), stuck_count.to_string())
            .with_metric("capacity_usage".to_string(), format!("{:.2}", capacity_usage))
            .with_metric("max_agents".to_string(), self.config.max_agents.to_string());

        // Add state counts as metrics
        for (state, count) in state_counts {
            health = health.with_metric(format!("state_{:?}", state).to_lowercase(), count.to_string());
        }

        Ok(health)
    }
}

/// Agent state machine for managing valid transitions
pub struct AgentStateMachine {
    valid_transitions: HashMap<AgentState, Vec<AgentState>>,
}

impl Default for AgentStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentStateMachine {
    pub fn new() -> Self {
        let mut valid_transitions = HashMap::new();

        // Define valid state transitions
        valid_transitions.insert(AgentState::Created, vec![AgentState::Initializing]);
        valid_transitions.insert(
            AgentState::Initializing,
            vec![AgentState::Ready, AgentState::Failed],
        );
        valid_transitions.insert(
            AgentState::Ready,
            vec![
                AgentState::Running,
                AgentState::Suspended,
                AgentState::Terminating,
            ],
        );
        valid_transitions.insert(
            AgentState::Running,
            vec![
                AgentState::Suspended,
                AgentState::Completed,
                AgentState::Failed,
                AgentState::Terminating,
            ],
        );
        valid_transitions.insert(
            AgentState::Suspended,
            vec![AgentState::Running, AgentState::Terminating],
        );
        valid_transitions.insert(AgentState::Completed, vec![AgentState::Terminating]);
        valid_transitions.insert(
            AgentState::Failed,
            vec![AgentState::Initializing, AgentState::Terminating],
        );
        valid_transitions.insert(AgentState::Terminating, vec![AgentState::Terminated]);
        valid_transitions.insert(AgentState::Terminated, vec![]); // Terminal state

        Self { valid_transitions }
    }

    pub fn is_valid_transition(&self, from: &AgentState, to: &AgentState) -> bool {
        self.valid_transitions
            .get(from)
            .map(|transitions| transitions.contains(to))
            .unwrap_or(false)
    }
}

/// Lifecycle events for internal processing
#[derive(Debug, Clone)]
enum LifecycleEvent {
    StateTransition {
        agent_id: AgentId,
        from_state: AgentState,
        to_state: AgentState,
    },
    AgentError {
        agent_id: AgentId,
        error: String,
        timestamp: SystemTime,
    },
    ResourceExhausted {
        agent_id: AgentId,
        resource_type: String,
        timestamp: SystemTime,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ExecutionMode, Priority, ResourceLimits, SecurityTier};
    use std::collections::HashMap;

    fn create_test_config() -> AgentConfig {
        AgentConfig {
            id: AgentId::new(),
            name: "test".to_string(),
            dsl_source: "test".to_string(),
            execution_mode: ExecutionMode::Ephemeral,
            security_tier: SecurityTier::Tier1,
            resource_limits: ResourceLimits::default(),
            capabilities: vec![],
            policies: vec![],
            metadata: HashMap::new(),
            priority: Priority::Normal,
        }
    }

    #[tokio::test]
    async fn test_agent_initialization() {
        let controller = DefaultLifecycleController::new(LifecycleConfig::default())
            .await
            .unwrap();
        let config = create_test_config();

        let agent_id = controller.initialize_agent(config).await.unwrap();

        // Give the event loop time to process
        tokio::time::sleep(Duration::from_millis(50)).await;

        let state = controller.get_agent_state(agent_id).await.unwrap();
        assert_eq!(state, AgentState::Initializing);
    }

    #[tokio::test]
    async fn test_state_transitions() {
        let controller = DefaultLifecycleController::new(LifecycleConfig::default())
            .await
            .unwrap();
        let config = create_test_config();

        let agent_id = controller.initialize_agent(config).await.unwrap();

        // Give time for initialization
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Transition to ready (simulate successful initialization)
        controller
            .send_event(LifecycleEvent::StateTransition {
                agent_id,
                from_state: AgentState::Initializing,
                to_state: AgentState::Ready,
            })
            .unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;

        // Start the agent
        controller.start_agent(agent_id).await.unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;

        let state = controller.get_agent_state(agent_id).await.unwrap();
        assert_eq!(state, AgentState::Running);
    }

    #[tokio::test]
    async fn test_agent_termination() {
        let controller = DefaultLifecycleController::new(LifecycleConfig::default())
            .await
            .unwrap();
        let config = create_test_config();

        let agent_id = controller.initialize_agent(config).await.unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;

        controller.terminate_agent(agent_id).await.unwrap();

        tokio::time::sleep(Duration::from_millis(150)).await;

        let result = controller.get_agent_state(agent_id).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_state_machine() {
        let state_machine = AgentStateMachine::new();

        // Test valid transitions
        assert!(state_machine.is_valid_transition(&AgentState::Created, &AgentState::Initializing));
        assert!(state_machine.is_valid_transition(&AgentState::Initializing, &AgentState::Ready));
        assert!(state_machine.is_valid_transition(&AgentState::Ready, &AgentState::Running));

        // Test invalid transitions
        assert!(!state_machine.is_valid_transition(&AgentState::Created, &AgentState::Running));
        assert!(!state_machine.is_valid_transition(&AgentState::Terminated, &AgentState::Running));
    }
}
