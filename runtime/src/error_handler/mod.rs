//! Agent Error Handler
//! 
//! Handles error recovery, escalation, and system resilience

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use std::boxed::Box;
use async_trait::async_trait;
use parking_lot::RwLock;
use tokio::sync::{mpsc, Notify};
use tokio::time::interval;

use crate::types::*;

/// Error handler trait
#[async_trait]
pub trait ErrorHandler {
    /// Handle an error for a specific agent
    async fn handle_error(&self, agent_id: AgentId, error: RuntimeError) -> Result<ErrorAction, ErrorHandlerError>;
    
    /// Register an error recovery strategy
    async fn register_strategy(&self, error_type: ErrorType, strategy: RecoveryStrategy) -> Result<(), ErrorHandlerError>;
    
    /// Get error statistics for an agent
    async fn get_error_stats(&self, agent_id: AgentId) -> Result<ErrorStatistics, ErrorHandlerError>;
    
    /// Get system-wide error statistics
    async fn get_system_error_stats(&self) -> SystemErrorStatistics;
    
    /// Set error thresholds for an agent
    async fn set_error_thresholds(&self, agent_id: AgentId, thresholds: ErrorThresholds) -> Result<(), ErrorHandlerError>;
    
    /// Clear error history for an agent
    async fn clear_error_history(&self, agent_id: AgentId) -> Result<(), ErrorHandlerError>;
    
    /// Shutdown the error handler
    async fn shutdown(&self) -> Result<(), ErrorHandlerError>;
}

/// Error handler configuration
#[derive(Debug, Clone)]
pub struct ErrorHandlerConfig {
    pub max_error_history: usize,
    pub error_aggregation_window: Duration,
    pub escalation_threshold: u32,
    pub circuit_breaker_threshold: u32,
    pub circuit_breaker_timeout: Duration,
    pub enable_auto_recovery: bool,
    pub max_recovery_attempts: u32,
    pub recovery_backoff_multiplier: f32,
}

impl Default for ErrorHandlerConfig {
    fn default() -> Self {
        Self {
            max_error_history: 1000,
            error_aggregation_window: Duration::from_secs(300), // 5 minutes
            escalation_threshold: 5,
            circuit_breaker_threshold: 10,
            circuit_breaker_timeout: Duration::from_secs(60),
            enable_auto_recovery: true,
            max_recovery_attempts: 3,
            recovery_backoff_multiplier: 2.0,
        }
    }
}

/// Default implementation of the error handler
pub struct DefaultErrorHandler {
    config: ErrorHandlerConfig,
    error_history: Arc<RwLock<HashMap<AgentId, Vec<ErrorRecord>>>>,
    recovery_strategies: Arc<RwLock<HashMap<ErrorType, RecoveryStrategy>>>,
    error_thresholds: Arc<RwLock<HashMap<AgentId, ErrorThresholds>>>,
    circuit_breakers: Arc<RwLock<HashMap<AgentId, CircuitBreaker>>>,
    event_sender: mpsc::UnboundedSender<ErrorEvent>,
    shutdown_notify: Arc<Notify>,
    is_running: Arc<RwLock<bool>>,
}

impl DefaultErrorHandler {
    /// Create a new error handler
    pub async fn new(config: ErrorHandlerConfig) -> Result<Self, ErrorHandlerError> {
        let error_history = Arc::new(RwLock::new(HashMap::new()));
        let recovery_strategies = Arc::new(RwLock::new(Self::default_strategies()));
        let error_thresholds = Arc::new(RwLock::new(HashMap::new()));
        let circuit_breakers = Arc::new(RwLock::new(HashMap::new()));
        let (event_sender, event_receiver) = mpsc::unbounded_channel();
        let shutdown_notify = Arc::new(Notify::new());
        let is_running = Arc::new(RwLock::new(true));

        let handler = Self {
            config,
            error_history,
            recovery_strategies,
            error_thresholds,
            circuit_breakers,
            event_sender,
            shutdown_notify,
            is_running,
        };

        // Start background tasks
        handler.start_event_loop(event_receiver).await;
        handler.start_cleanup_loop().await;

        Ok(handler)
    }

    /// Create default recovery strategies
    fn default_strategies() -> HashMap<ErrorType, RecoveryStrategy> {
        let mut strategies = HashMap::new();
        
        strategies.insert(ErrorType::ResourceExhaustion, RecoveryStrategy::Retry {
            max_attempts: 3,
            backoff: Duration::from_secs(1),
        });
        
        strategies.insert(ErrorType::NetworkError, RecoveryStrategy::Retry {
            max_attempts: 5,
            backoff: Duration::from_millis(500),
        });
        
        strategies.insert(ErrorType::SecurityViolation, RecoveryStrategy::Terminate {
            cleanup: true,
        });
        
        strategies.insert(ErrorType::PolicyViolation, RecoveryStrategy::Manual {
            reason: "Policy violation requires manual review".to_string(),
        });
        
        strategies.insert(ErrorType::SystemError, RecoveryStrategy::Restart {
            preserve_state: false,
        });
        
        strategies.insert(ErrorType::ValidationError, RecoveryStrategy::Failover {
            backup_agent: None,
        });
        
        strategies
    }

    /// Start the event processing loop
    async fn start_event_loop(&self, mut event_receiver: mpsc::UnboundedReceiver<ErrorEvent>) {
        let error_history = self.error_history.clone();
        let recovery_strategies = self.recovery_strategies.clone();
        let error_thresholds = self.error_thresholds.clone();
        let circuit_breakers = self.circuit_breakers.clone();
        let shutdown_notify = self.shutdown_notify.clone();
        let config = self.config.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    event = event_receiver.recv() => {
                        if let Some(event) = event {
                            Self::process_error_event(
                                event,
                                &error_history,
                                &recovery_strategies,
                                &error_thresholds,
                                &circuit_breakers,
                                &config,
                            ).await;
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

    /// Start the cleanup loop for old error records
    async fn start_cleanup_loop(&self) {
        let error_history = self.error_history.clone();
        let circuit_breakers = self.circuit_breakers.clone();
        let shutdown_notify = self.shutdown_notify.clone();
        let is_running = self.is_running.clone();
        let max_history = self.config.max_error_history;
        let aggregation_window = self.config.error_aggregation_window;

        tokio::spawn(async move {
            let mut interval = interval(Duration::from_secs(300)); // Cleanup every 5 minutes
            
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if !*is_running.read() {
                            break;
                        }

                        Self::cleanup_old_records(&error_history, &circuit_breakers, max_history, aggregation_window).await;
                    }
                    _ = shutdown_notify.notified() => {
                        break;
                    }
                }
            }
        });
    }

    /// Process an error event
    async fn process_error_event(
        event: ErrorEvent,
        error_history: &Arc<RwLock<HashMap<AgentId, Vec<ErrorRecord>>>>,
        recovery_strategies: &Arc<RwLock<HashMap<ErrorType, RecoveryStrategy>>>,
        error_thresholds: &Arc<RwLock<HashMap<AgentId, ErrorThresholds>>>,
        circuit_breakers: &Arc<RwLock<HashMap<AgentId, CircuitBreaker>>>,
        config: &ErrorHandlerConfig,
    ) {
        match event {
            ErrorEvent::ErrorOccurred { agent_id, error } => {
                // Record the error
                let error_record = ErrorRecord::new(error.clone());
                error_history.write().entry(agent_id).or_default().push(error_record);
                
                // Check circuit breaker
                let circuit_breaker_open = {
                    let mut breakers = circuit_breakers.write();
                    let breaker = breakers.entry(agent_id).or_insert_with(|| CircuitBreaker::new(config.circuit_breaker_threshold, config.circuit_breaker_timeout));
                    
                    if breaker.is_open() {
                        tracing::warn!("Circuit breaker open for agent {}, blocking error handling", agent_id);
                        return;
                    }
                    
                    breaker.record_failure();
                    false
                };
                
                if circuit_breaker_open {
                    return;
                }
                
                // Check thresholds
                let thresholds = error_thresholds.read().get(&agent_id).cloned().unwrap_or_default();
                let recent_errors = Self::count_recent_errors(error_history, agent_id, config.error_aggregation_window);
                
                if recent_errors >= thresholds.max_errors_per_window {
                    tracing::error!("Agent {} exceeded error threshold: {} errors in window", agent_id, recent_errors);
                    // Could trigger escalation here
                }
                
                // Determine recovery action
                let error_type = Self::classify_error(&error);
                let strategy_option = {
                    let strategies = recovery_strategies.read();
                    strategies.get(&error_type).cloned()
                };
                
                if let Some(strategy) = strategy_option {
                    tracing::info!("Applying recovery strategy {:?} for agent {} error: {}", strategy, agent_id, error);
                    
                    // Simulate recovery attempt (in real implementation this would call actual recovery logic)
                    let success = match strategy {
                        RecoveryStrategy::Retry { .. } => true, // Assume retry succeeds
                        RecoveryStrategy::Restart { .. } => true, // Assume restart succeeds
                        RecoveryStrategy::Terminate { .. } => true, // Terminate always succeeds
                        RecoveryStrategy::Failover { .. } => false, // Failover might fail without backup
                        RecoveryStrategy::Manual { .. } => false, // Manual requires intervention
                        RecoveryStrategy::None => false, // No recovery means failure
                    };
                    
                    // Send recovery event (this would be done by the actual recovery system)
                    let recovery_event = ErrorEvent::RecoveryAttempted {
                        agent_id,
                        strategy: strategy.clone(),
                        success,
                        timestamp: SystemTime::now(),
                    };
                    
                    // Process the recovery event to demonstrate its usage
                    Box::pin(Self::process_error_event(
                        recovery_event,
                        error_history,
                        recovery_strategies,
                        error_thresholds,
                        circuit_breakers,
                        config,
                    )).await;
                } else {
                    tracing::warn!("No recovery strategy found for error type {:?} in agent {}", error_type, agent_id);
                }
            }
            ErrorEvent::RecoveryAttempted { agent_id, strategy, success, timestamp } => {
                if success {
                    tracing::info!("Recovery successful for agent {} using strategy {:?} at {:?}", agent_id, strategy, timestamp);
                    
                    // Reset circuit breaker on successful recovery
                    {
                        if let Some(breaker) = circuit_breakers.write().get_mut(&agent_id) {
                            breaker.record_success();
                        }
                    }
                } else {
                    tracing::error!("Recovery failed for agent {} using strategy {:?} at {:?}", agent_id, strategy, timestamp);
                }
            }
        }
    }

    /// Cleanup old error records
    async fn cleanup_old_records(
        error_history: &Arc<RwLock<HashMap<AgentId, Vec<ErrorRecord>>>>,
        circuit_breakers: &Arc<RwLock<HashMap<AgentId, CircuitBreaker>>>,
        max_history: usize,
        aggregation_window: Duration,
    ) {
        let now = SystemTime::now();
        
        // Cleanup error history
        {
            let mut history = error_history.write();
            for records in history.values_mut() {
                // Remove old records
                records.retain(|record| {
                    now.duration_since(record.timestamp).unwrap_or_default() < aggregation_window * 2
                });
                
                // Limit history size
                if records.len() > max_history {
                    records.drain(0..records.len() - max_history);
                }
            }
            
            // Remove empty entries
            history.retain(|_, records| !records.is_empty());
        }
        
        // Update circuit breakers
        {
            let mut breakers = circuit_breakers.write();
            for breaker in breakers.values_mut() {
                breaker.update(now);
            }
            
            // Remove closed breakers that haven't been used recently
            breakers.retain(|_, breaker| breaker.is_open() || breaker.last_failure_time.map(|t| now.duration_since(t).unwrap_or_default() < aggregation_window).unwrap_or(false));
        }
    }

    /// Classify an error into a type
    fn classify_error(error: &RuntimeError) -> ErrorType {
        match error {
            RuntimeError::Resource(_) => ErrorType::ResourceExhaustion,
            RuntimeError::Communication(_) => ErrorType::NetworkError,
            RuntimeError::Security(_) => ErrorType::SecurityViolation,
            RuntimeError::Scheduler(_) => ErrorType::SystemError,
            RuntimeError::Lifecycle(_) => ErrorType::SystemError,
            RuntimeError::ErrorHandler(_) => ErrorType::SystemError,
            RuntimeError::Configuration(_) => ErrorType::SystemError,
            RuntimeError::Policy(_) => ErrorType::SecurityViolation,
            RuntimeError::Sandbox(_) => ErrorType::SecurityViolation,
            RuntimeError::Audit(_) => ErrorType::SystemError,
            RuntimeError::Internal(_) => ErrorType::SystemError,
        }
    }

    /// Count recent errors for an agent
    fn count_recent_errors(
        error_history: &Arc<RwLock<HashMap<AgentId, Vec<ErrorRecord>>>>,
        agent_id: AgentId,
        window: Duration,
    ) -> u32 {
        let history = error_history.read();
        if let Some(records) = history.get(&agent_id) {
            let now = SystemTime::now();
            records.iter()
                .filter(|record| now.duration_since(record.timestamp).unwrap_or_default() < window)
                .count() as u32
        } else {
            0
        }
    }

    /// Send an error event
    fn send_event(&self, event: ErrorEvent) -> Result<(), ErrorHandlerError> {
        self.event_sender.send(event)
            .map_err(|_| ErrorHandlerError::EventProcessingFailed {
                reason: "Failed to send error event".to_string(),
            })
    }
}

#[async_trait]
impl ErrorHandler for DefaultErrorHandler {
    async fn handle_error(&self, agent_id: AgentId, error: RuntimeError) -> Result<ErrorAction, ErrorHandlerError> {
        if !*self.is_running.read() {
            return Err(ErrorHandlerError::ShuttingDown);
        }

        // Send error event for processing
        self.send_event(ErrorEvent::ErrorOccurred { agent_id, error: error.clone() })?;

        // Determine immediate action
        let error_type = Self::classify_error(&error);
        let strategies = self.recovery_strategies.read();
        
        if let Some(strategy) = strategies.get(&error_type) {
            let action = match strategy {
                RecoveryStrategy::Retry { max_attempts, backoff } => {
                    ErrorAction::Retry {
                        max_attempts: *max_attempts,
                        backoff: *backoff,
                    }
                }
                RecoveryStrategy::Restart { .. } => ErrorAction::Restart,
                RecoveryStrategy::Terminate { .. } => ErrorAction::Terminate,
                RecoveryStrategy::Failover { .. } => ErrorAction::Failover,
                RecoveryStrategy::Manual { .. } => ErrorAction::Suspend, // Manual intervention maps to suspend
                RecoveryStrategy::None => ErrorAction::Terminate, // No recovery maps to terminate
            };
            
            Ok(action)
        } else {
            // Default action for unknown error types
            Ok(ErrorAction::Retry {
                max_attempts: 1,
                backoff: Duration::from_secs(1),
            })
        }
    }

    async fn register_strategy(&self, error_type: ErrorType, strategy: RecoveryStrategy) -> Result<(), ErrorHandlerError> {
        let error_type_clone = error_type;
        self.recovery_strategies.write().insert(error_type, strategy);
        tracing::info!("Registered recovery strategy for error type {:?}", error_type_clone);
        Ok(())
    }

    async fn get_error_stats(&self, agent_id: AgentId) -> Result<ErrorStatistics, ErrorHandlerError> {
        let history = self.error_history.read();
        if let Some(records) = history.get(&agent_id) {
            let now = SystemTime::now();
            let window = self.config.error_aggregation_window;
            
            let recent_errors = records.iter()
                .filter(|record| now.duration_since(record.timestamp).unwrap_or_default() < window)
                .count() as u32;
            
            let total_errors = records.len() as u32;
            
            let error_types = records.iter()
                .map(|record| Self::classify_error(&record.error))
                .fold(HashMap::new(), |mut acc, error_type| {
                    *acc.entry(error_type).or_insert(0) += 1;
                    acc
                });
            
            Ok(ErrorStatistics {
                agent_id,
                total_errors,
                recent_errors,
                error_types,
                last_error: records.last().map(|r| r.timestamp),
            })
        } else {
            Ok(ErrorStatistics {
                agent_id,
                total_errors: 0,
                recent_errors: 0,
                error_types: HashMap::new(),
                last_error: None,
            })
        }
    }

    async fn get_system_error_stats(&self) -> SystemErrorStatistics {
        let history = self.error_history.read();
        let now = SystemTime::now();
        let window = self.config.error_aggregation_window;
        
        let mut total_errors = 0;
        let mut recent_errors = 0;
        let mut agents_with_errors = 0;
        let mut error_types = HashMap::new();
        
        for records in history.values() {
            if !records.is_empty() {
                agents_with_errors += 1;
                total_errors += records.len() as u32;
                
                let agent_recent_errors = records.iter()
                    .filter(|record| now.duration_since(record.timestamp).unwrap_or_default() < window)
                    .count() as u32;
                
                recent_errors += agent_recent_errors;
                
                for record in records {
                    let error_type = Self::classify_error(&record.error);
                    *error_types.entry(error_type).or_insert(0) += 1;
                }
            }
        }
        
        SystemErrorStatistics {
            total_errors,
            recent_errors,
            agents_with_errors,
            error_types,
            last_updated: now,
        }
    }

    async fn set_error_thresholds(&self, agent_id: AgentId, thresholds: ErrorThresholds) -> Result<(), ErrorHandlerError> {
        self.error_thresholds.write().insert(agent_id, thresholds);
        tracing::info!("Set error thresholds for agent {}", agent_id);
        Ok(())
    }

    async fn clear_error_history(&self, agent_id: AgentId) -> Result<(), ErrorHandlerError> {
        self.error_history.write().remove(&agent_id);
        self.circuit_breakers.write().remove(&agent_id);
        tracing::info!("Cleared error history for agent {}", agent_id);
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), ErrorHandlerError> {
        tracing::info!("Shutting down error handler");
        
        *self.is_running.write() = false;
        self.shutdown_notify.notify_waiters();

        Ok(())
    }
}

/// Error record for tracking
#[derive(Debug, Clone)]
struct ErrorRecord {
    error: RuntimeError,
    timestamp: SystemTime,
}

impl ErrorRecord {
    fn new(error: RuntimeError) -> Self {
        Self {
            error,
            timestamp: SystemTime::now(),
        }
    }
}

/// Circuit breaker for error handling
#[derive(Debug, Clone)]
struct CircuitBreaker {
    failure_threshold: u32,
    timeout: Duration,
    failure_count: u32,
    last_failure_time: Option<SystemTime>,
    state: CircuitBreakerState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CircuitBreakerState {
    Closed,
    Open,
    HalfOpen,
}

impl CircuitBreaker {
    fn new(failure_threshold: u32, timeout: Duration) -> Self {
        Self {
            failure_threshold,
            timeout,
            failure_count: 0,
            last_failure_time: None,
            state: CircuitBreakerState::Closed,
        }
    }

    fn is_open(&self) -> bool {
        self.state == CircuitBreakerState::Open
    }

    fn record_failure(&mut self) {
        self.failure_count += 1;
        self.last_failure_time = Some(SystemTime::now());
        
        if self.failure_count >= self.failure_threshold {
            self.state = CircuitBreakerState::Open;
        }
    }

    fn record_success(&mut self) {
        self.failure_count = 0;
        self.state = CircuitBreakerState::Closed;
    }

    fn update(&mut self, now: SystemTime) {
        if self.state == CircuitBreakerState::Open {
            if let Some(last_failure) = self.last_failure_time {
                if now.duration_since(last_failure).unwrap_or_default() > self.timeout {
                    self.state = CircuitBreakerState::HalfOpen;
                }
            }
        }
    }
}

/// Error thresholds for an agent
#[derive(Debug, Clone)]
pub struct ErrorThresholds {
    pub max_errors_per_window: u32,
    pub escalation_threshold: u32,
}

impl Default for ErrorThresholds {
    fn default() -> Self {
        Self {
            max_errors_per_window: 10,
            escalation_threshold: 5,
        }
    }
}

/// Error statistics for an agent
#[derive(Debug, Clone)]
pub struct ErrorStatistics {
    pub agent_id: AgentId,
    pub total_errors: u32,
    pub recent_errors: u32,
    pub error_types: HashMap<ErrorType, u32>,
    pub last_error: Option<SystemTime>,
}

/// System-wide error statistics
#[derive(Debug, Clone)]
pub struct SystemErrorStatistics {
    pub total_errors: u32,
    pub recent_errors: u32,
    pub agents_with_errors: u32,
    pub error_types: HashMap<ErrorType, u32>,
    pub last_updated: SystemTime,
}

/// Error action to take
#[derive(Debug, Clone)]
pub enum ErrorAction {
    Retry { max_attempts: u32, backoff: Duration },
    Restart,
    Suspend,
    Terminate,
    Failover,
}

/// Error types for classification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorType {
    ResourceExhaustion,
    NetworkError,
    SecurityViolation,
    PolicyViolation,
    SystemError,
    ValidationError,
}

/// Error events for internal processing
#[derive(Debug, Clone)]
enum ErrorEvent {
    ErrorOccurred {
        agent_id: AgentId,
        error: RuntimeError,
    },
    RecoveryAttempted {
        agent_id: AgentId,
        strategy: RecoveryStrategy,
        success: bool,
        timestamp: SystemTime,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_error_handling() {
        let handler = DefaultErrorHandler::new(ErrorHandlerConfig::default()).await.unwrap();
        let agent_id = AgentId::new();
        let error = RuntimeError::Resource(ResourceError::Insufficient("Memory exhausted".to_string()));

        let action = handler.handle_error(agent_id, error).await.unwrap();
        
        match action {
            ErrorAction::Retry { max_attempts, .. } => {
                assert_eq!(max_attempts, 3);
            }
            _ => panic!("Expected retry action for resource error"),
        }
    }

    #[tokio::test]
    async fn test_error_statistics() {
        let handler = DefaultErrorHandler::new(ErrorHandlerConfig::default()).await.unwrap();
        let agent_id = AgentId::new();
        
        // Generate some errors
        for _ in 0..5 {
            let error = RuntimeError::Resource(ResourceError::Insufficient("Memory exhausted".to_string()));
            handler.handle_error(agent_id, error).await.unwrap();
        }

        // Give the event loop time to process
        tokio::time::sleep(Duration::from_millis(50)).await;

        let stats = handler.get_error_stats(agent_id).await.unwrap();
        assert_eq!(stats.total_errors, 5);
        assert_eq!(stats.recent_errors, 5);
        assert!(stats.error_types.contains_key(&ErrorType::ResourceExhaustion));
    }

    #[tokio::test]
    async fn test_recovery_strategy_registration() {
        let handler = DefaultErrorHandler::new(ErrorHandlerConfig::default()).await.unwrap();
        
        let strategy = RecoveryStrategy::Terminate { cleanup: true };
        let result = handler.register_strategy(ErrorType::SecurityViolation, strategy).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_error_thresholds() {
        let handler = DefaultErrorHandler::new(ErrorHandlerConfig::default()).await.unwrap();
        let agent_id = AgentId::new();
        
        let thresholds = ErrorThresholds {
            max_errors_per_window: 3,
            escalation_threshold: 2,
        };
        
        let result = handler.set_error_thresholds(agent_id, thresholds).await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_circuit_breaker() {
        let mut breaker = CircuitBreaker::new(3, Duration::from_secs(60));
        
        assert!(!breaker.is_open());
        
        // Record failures
        breaker.record_failure();
        breaker.record_failure();
        assert!(!breaker.is_open());
        
        breaker.record_failure();
        assert!(breaker.is_open());
        
        // Record success should reset
        breaker.record_success();
        assert!(!breaker.is_open());
    }
}