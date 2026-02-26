//! Per-tool-endpoint circuit breaker
//!
//! Prevents cascade failures by tracking tool endpoint health and
//! fast-failing when a circuit is open. Implements the standard
//! Closed → Open → Half-Open state machine.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Circuit breaker state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CircuitState {
    /// Normal operation. Requests flow through.
    Closed,
    /// Failures exceeded threshold. Requests are immediately rejected.
    Open {
        /// When the circuit was opened.
        opened_at: Instant,
    },
    /// Recovery testing. A limited number of requests are allowed through.
    HalfOpen,
}

/// Configuration for a circuit breaker.
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of failures before opening the circuit.
    pub failure_threshold: u32,
    /// How long to wait before transitioning from Open to HalfOpen.
    pub recovery_timeout: Duration,
    /// Max requests to allow through in HalfOpen state.
    pub half_open_max_calls: u32,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            recovery_timeout: Duration::from_secs(30),
            half_open_max_calls: 2,
        }
    }
}

/// A circuit breaker for a single tool endpoint.
#[derive(Debug)]
pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    state: CircuitState,
    failure_count: u32,
    success_count: u32,
    half_open_calls: u32,
    last_failure: Option<Instant>,
}

/// Error returned when the circuit is open.
#[derive(Debug, thiserror::Error)]
#[error("Circuit open for tool '{tool_name}': {consecutive_failures} consecutive failures, recovery in {recovery_remaining:?}")]
pub struct CircuitOpenError {
    pub tool_name: String,
    pub consecutive_failures: u32,
    pub recovery_remaining: Duration,
}

impl CircuitBreaker {
    /// Create a new circuit breaker in the Closed state.
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            state: CircuitState::Closed,
            failure_count: 0,
            success_count: 0,
            half_open_calls: 0,
            last_failure: None,
        }
    }

    /// Check if a request should be allowed through.
    ///
    /// Returns `Ok(())` if the request can proceed, or `Err(CircuitOpenError)`
    /// if the circuit is open and the request should be fast-failed.
    pub fn check(&mut self, tool_name: &str) -> Result<(), CircuitOpenError> {
        match &self.state {
            CircuitState::Closed => Ok(()),
            CircuitState::Open { opened_at } => {
                // Check if recovery timeout has elapsed
                if opened_at.elapsed() >= self.config.recovery_timeout {
                    self.state = CircuitState::HalfOpen;
                    self.half_open_calls = 1; // This check counts as the first half-open call
                    tracing::info!(
                        "Circuit breaker for '{}' transitioning to HalfOpen",
                        tool_name
                    );
                    Ok(())
                } else {
                    let remaining = self.config.recovery_timeout - opened_at.elapsed();
                    Err(CircuitOpenError {
                        tool_name: tool_name.to_string(),
                        consecutive_failures: self.failure_count,
                        recovery_remaining: remaining,
                    })
                }
            }
            CircuitState::HalfOpen => {
                if self.half_open_calls < self.config.half_open_max_calls {
                    self.half_open_calls += 1;
                    Ok(())
                } else {
                    Err(CircuitOpenError {
                        tool_name: tool_name.to_string(),
                        consecutive_failures: self.failure_count,
                        recovery_remaining: Duration::from_secs(0),
                    })
                }
            }
        }
    }

    /// Record a successful request.
    pub fn record_success(&mut self, tool_name: &str) {
        match self.state {
            CircuitState::Closed => {
                self.failure_count = 0;
                self.success_count += 1;
            }
            CircuitState::HalfOpen => {
                // Recovery successful — close the circuit
                self.state = CircuitState::Closed;
                self.failure_count = 0;
                self.success_count = 1;
                self.half_open_calls = 0;
                tracing::info!("Circuit breaker for '{}' recovered, now Closed", tool_name);
            }
            CircuitState::Open { .. } => {
                // Should not happen, but handle gracefully
                self.state = CircuitState::Closed;
                self.failure_count = 0;
            }
        }
    }

    /// Record a failed request.
    pub fn record_failure(&mut self, tool_name: &str) {
        self.last_failure = Some(Instant::now());

        match self.state {
            CircuitState::Closed => {
                self.failure_count += 1;
                if self.failure_count >= self.config.failure_threshold {
                    self.state = CircuitState::Open {
                        opened_at: Instant::now(),
                    };
                    tracing::warn!(
                        "Circuit breaker for '{}' tripped OPEN after {} failures",
                        tool_name,
                        self.failure_count
                    );
                }
            }
            CircuitState::HalfOpen => {
                // Failed during recovery — go back to Open
                self.state = CircuitState::Open {
                    opened_at: Instant::now(),
                };
                self.half_open_calls = 0;
                tracing::warn!(
                    "Circuit breaker for '{}' recovery failed, back to OPEN",
                    tool_name
                );
            }
            CircuitState::Open { .. } => {
                // Already open, just increment
                self.failure_count += 1;
            }
        }
    }

    /// Get the current state.
    pub fn state(&self) -> &CircuitState {
        &self.state
    }

    /// Get the failure count.
    pub fn failure_count(&self) -> u32 {
        self.failure_count
    }
}

/// Registry of circuit breakers for all tool endpoints.
pub struct CircuitBreakerRegistry {
    breakers: Arc<RwLock<HashMap<String, CircuitBreaker>>>,
    default_config: CircuitBreakerConfig,
}

impl Default for CircuitBreakerRegistry {
    fn default() -> Self {
        Self::new(CircuitBreakerConfig::default())
    }
}

impl CircuitBreakerRegistry {
    /// Create a new registry with a default configuration for new breakers.
    pub fn new(default_config: CircuitBreakerConfig) -> Self {
        Self {
            breakers: Arc::new(RwLock::new(HashMap::new())),
            default_config,
        }
    }

    /// Check if a tool call should be allowed.
    pub async fn check(&self, tool_name: &str) -> Result<(), CircuitOpenError> {
        let mut breakers = self.breakers.write().await;
        let breaker = breakers
            .entry(tool_name.to_string())
            .or_insert_with(|| CircuitBreaker::new(self.default_config.clone()));
        breaker.check(tool_name)
    }

    /// Record a successful tool call.
    pub async fn record_success(&self, tool_name: &str) {
        let mut breakers = self.breakers.write().await;
        if let Some(breaker) = breakers.get_mut(tool_name) {
            breaker.record_success(tool_name);
        }
    }

    /// Record a failed tool call.
    pub async fn record_failure(&self, tool_name: &str) {
        let mut breakers = self.breakers.write().await;
        let breaker = breakers
            .entry(tool_name.to_string())
            .or_insert_with(|| CircuitBreaker::new(self.default_config.clone()));
        breaker.record_failure(tool_name);
    }

    /// Get the state of a specific breaker.
    pub async fn get_state(&self, tool_name: &str) -> Option<CircuitState> {
        let breakers = self.breakers.read().await;
        breakers.get(tool_name).map(|b| b.state().clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_circuit_breaker_starts_closed() {
        let cb = CircuitBreaker::new(CircuitBreakerConfig::default());
        assert_eq!(*cb.state(), CircuitState::Closed);
        assert_eq!(cb.failure_count(), 0);
    }

    #[test]
    fn test_circuit_breaker_allows_when_closed() {
        let mut cb = CircuitBreaker::new(CircuitBreakerConfig::default());
        assert!(cb.check("test_tool").is_ok());
    }

    #[test]
    fn test_circuit_breaker_trips_after_threshold() {
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            recovery_timeout: Duration::from_secs(30),
            half_open_max_calls: 1,
        };
        let mut cb = CircuitBreaker::new(config);

        // First 2 failures keep it closed
        cb.record_failure("tool");
        assert_eq!(*cb.state(), CircuitState::Closed);
        cb.record_failure("tool");
        assert_eq!(*cb.state(), CircuitState::Closed);

        // Third failure trips it open
        cb.record_failure("tool");
        assert!(matches!(*cb.state(), CircuitState::Open { .. }));

        // Now check should fail
        assert!(cb.check("tool").is_err());
    }

    #[test]
    fn test_circuit_breaker_recovery() {
        let config = CircuitBreakerConfig {
            failure_threshold: 2,
            recovery_timeout: Duration::from_millis(1), // Very short for testing
            half_open_max_calls: 1,
        };
        let mut cb = CircuitBreaker::new(config);

        // Trip the breaker
        cb.record_failure("tool");
        cb.record_failure("tool");
        assert!(matches!(*cb.state(), CircuitState::Open { .. }));

        // Wait for recovery timeout
        std::thread::sleep(Duration::from_millis(5));

        // Should transition to HalfOpen
        assert!(cb.check("tool").is_ok());
        assert_eq!(*cb.state(), CircuitState::HalfOpen);

        // Success in HalfOpen closes it
        cb.record_success("tool");
        assert_eq!(*cb.state(), CircuitState::Closed);
    }

    #[test]
    fn test_circuit_breaker_half_open_failure() {
        let config = CircuitBreakerConfig {
            failure_threshold: 2,
            recovery_timeout: Duration::from_millis(1),
            half_open_max_calls: 1,
        };
        let mut cb = CircuitBreaker::new(config);

        // Trip
        cb.record_failure("tool");
        cb.record_failure("tool");

        // Wait and go HalfOpen
        std::thread::sleep(Duration::from_millis(5));
        assert!(cb.check("tool").is_ok());
        assert_eq!(*cb.state(), CircuitState::HalfOpen);

        // Fail in HalfOpen → back to Open
        cb.record_failure("tool");
        assert!(matches!(*cb.state(), CircuitState::Open { .. }));
    }

    #[test]
    fn test_circuit_breaker_success_resets_failures() {
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            ..Default::default()
        };
        let mut cb = CircuitBreaker::new(config);

        cb.record_failure("tool");
        cb.record_failure("tool");
        assert_eq!(cb.failure_count(), 2);

        cb.record_success("tool");
        assert_eq!(cb.failure_count(), 0);
    }

    #[test]
    fn test_half_open_limits_calls() {
        let config = CircuitBreakerConfig {
            failure_threshold: 1,
            recovery_timeout: Duration::from_millis(1),
            half_open_max_calls: 2,
        };
        let mut cb = CircuitBreaker::new(config);

        // Trip
        cb.record_failure("tool");
        std::thread::sleep(Duration::from_millis(5));

        // Should allow 2 calls in HalfOpen
        assert!(cb.check("tool").is_ok());
        assert!(cb.check("tool").is_ok());
        // Third should fail
        assert!(cb.check("tool").is_err());
    }

    #[tokio::test]
    async fn test_registry_basic() {
        let registry = CircuitBreakerRegistry::default();

        // New tools start Closed
        assert!(registry.check("new_tool").await.is_ok());

        // Record some failures
        for _ in 0..5 {
            registry.record_failure("failing_tool").await;
        }

        // Should be Open now
        assert!(registry.check("failing_tool").await.is_err());

        // Other tools unaffected
        assert!(registry.check("new_tool").await.is_ok());
    }

    #[tokio::test]
    async fn test_registry_get_state() {
        let registry = CircuitBreakerRegistry::default();
        assert!(registry.get_state("unknown").await.is_none());

        registry.check("known").await.unwrap();
        let state = registry.get_state("known").await;
        assert_eq!(state, Some(CircuitState::Closed));
    }
}
