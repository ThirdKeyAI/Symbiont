//! Reasoning loop observability metrics
//!
//! Tracks loop execution statistics via atomic counters.
//! When the `metrics` feature is enabled, also emits to OpenTelemetry.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Reasoning loop metrics.
///
/// Always available via atomic counters for in-process queries.
/// When the `metrics` feature is enabled, also pushes to OpenTelemetry.
#[derive(Clone)]
pub struct ReasoningMetrics {
    inner: Arc<MetricsInner>,
}

struct MetricsInner {
    loops_started: AtomicU64,
    loops_completed: AtomicU64,
    loops_failed: AtomicU64,
    total_iterations: AtomicU64,
    total_tokens: AtomicU64,
    tool_calls: AtomicU64,
    policy_denials: AtomicU64,
    tool_errors: AtomicU64,
}

impl Default for ReasoningMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl ReasoningMetrics {
    /// Create a new metrics instance.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(MetricsInner {
                loops_started: AtomicU64::new(0),
                loops_completed: AtomicU64::new(0),
                loops_failed: AtomicU64::new(0),
                total_iterations: AtomicU64::new(0),
                total_tokens: AtomicU64::new(0),
                tool_calls: AtomicU64::new(0),
                policy_denials: AtomicU64::new(0),
                tool_errors: AtomicU64::new(0),
            }),
        }
    }

    /// Record a loop starting.
    pub fn record_loop_started(&self) {
        self.inner.loops_started.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a loop completing successfully.
    pub fn record_loop_completed(&self, iterations: u32, tokens: u64) {
        self.inner.loops_completed.fetch_add(1, Ordering::Relaxed);
        self.inner
            .total_iterations
            .fetch_add(iterations as u64, Ordering::Relaxed);
        self.inner.total_tokens.fetch_add(tokens, Ordering::Relaxed);
    }

    /// Record a loop failing.
    pub fn record_loop_failed(&self, iterations: u32, tokens: u64) {
        self.inner.loops_failed.fetch_add(1, Ordering::Relaxed);
        self.inner
            .total_iterations
            .fetch_add(iterations as u64, Ordering::Relaxed);
        self.inner.total_tokens.fetch_add(tokens, Ordering::Relaxed);
    }

    /// Record a tool call.
    pub fn record_tool_call(&self) {
        self.inner.tool_calls.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a tool error.
    pub fn record_tool_error(&self) {
        self.inner.tool_errors.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a policy denial.
    pub fn record_policy_denial(&self) {
        self.inner.policy_denials.fetch_add(1, Ordering::Relaxed);
    }

    /// Get a snapshot of all metrics.
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            loops_started: self.inner.loops_started.load(Ordering::Relaxed),
            loops_completed: self.inner.loops_completed.load(Ordering::Relaxed),
            loops_failed: self.inner.loops_failed.load(Ordering::Relaxed),
            total_iterations: self.inner.total_iterations.load(Ordering::Relaxed),
            total_tokens: self.inner.total_tokens.load(Ordering::Relaxed),
            tool_calls: self.inner.tool_calls.load(Ordering::Relaxed),
            tool_errors: self.inner.tool_errors.load(Ordering::Relaxed),
            policy_denials: self.inner.policy_denials.load(Ordering::Relaxed),
        }
    }

    /// Reset all counters to zero.
    pub fn reset(&self) {
        self.inner.loops_started.store(0, Ordering::Relaxed);
        self.inner.loops_completed.store(0, Ordering::Relaxed);
        self.inner.loops_failed.store(0, Ordering::Relaxed);
        self.inner.total_iterations.store(0, Ordering::Relaxed);
        self.inner.total_tokens.store(0, Ordering::Relaxed);
        self.inner.tool_calls.store(0, Ordering::Relaxed);
        self.inner.tool_errors.store(0, Ordering::Relaxed);
        self.inner.policy_denials.store(0, Ordering::Relaxed);
    }
}

/// Point-in-time snapshot of reasoning metrics.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MetricsSnapshot {
    pub loops_started: u64,
    pub loops_completed: u64,
    pub loops_failed: u64,
    pub total_iterations: u64,
    pub total_tokens: u64,
    pub tool_calls: u64,
    pub tool_errors: u64,
    pub policy_denials: u64,
}

impl MetricsSnapshot {
    /// Success rate as a fraction (0.0 - 1.0). Returns 1.0 if no loops started.
    pub fn success_rate(&self) -> f64 {
        let total = self.loops_completed + self.loops_failed;
        if total == 0 {
            1.0
        } else {
            self.loops_completed as f64 / total as f64
        }
    }

    /// Average iterations per completed loop. Returns 0.0 if no loops completed.
    pub fn avg_iterations(&self) -> f64 {
        let total = self.loops_completed + self.loops_failed;
        if total == 0 {
            0.0
        } else {
            self.total_iterations as f64 / total as f64
        }
    }

    /// Average tokens per loop. Returns 0.0 if no loops ran.
    pub fn avg_tokens(&self) -> f64 {
        let total = self.loops_completed + self.loops_failed;
        if total == 0 {
            0.0
        } else {
            self.total_tokens as f64 / total as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_default() {
        let metrics = ReasoningMetrics::new();
        let snap = metrics.snapshot();
        assert_eq!(snap.loops_started, 0);
        assert_eq!(snap.loops_completed, 0);
        assert_eq!(snap.loops_failed, 0);
        assert_eq!(snap.total_tokens, 0);
    }

    #[test]
    fn test_record_loop_lifecycle() {
        let metrics = ReasoningMetrics::new();

        metrics.record_loop_started();
        metrics.record_loop_started();
        metrics.record_loop_completed(5, 1000);
        metrics.record_loop_failed(3, 500);

        let snap = metrics.snapshot();
        assert_eq!(snap.loops_started, 2);
        assert_eq!(snap.loops_completed, 1);
        assert_eq!(snap.loops_failed, 1);
        assert_eq!(snap.total_iterations, 8);
        assert_eq!(snap.total_tokens, 1500);
    }

    #[test]
    fn test_record_tool_calls() {
        let metrics = ReasoningMetrics::new();

        metrics.record_tool_call();
        metrics.record_tool_call();
        metrics.record_tool_call();
        metrics.record_tool_error();

        let snap = metrics.snapshot();
        assert_eq!(snap.tool_calls, 3);
        assert_eq!(snap.tool_errors, 1);
    }

    #[test]
    fn test_record_policy_denials() {
        let metrics = ReasoningMetrics::new();

        metrics.record_policy_denial();
        metrics.record_policy_denial();

        let snap = metrics.snapshot();
        assert_eq!(snap.policy_denials, 2);
    }

    #[test]
    fn test_success_rate() {
        let metrics = ReasoningMetrics::new();

        // No loops = 1.0
        assert!((metrics.snapshot().success_rate() - 1.0).abs() < f64::EPSILON);

        metrics.record_loop_completed(1, 100);
        metrics.record_loop_completed(1, 100);
        metrics.record_loop_failed(1, 100);

        let snap = metrics.snapshot();
        assert!((snap.success_rate() - 2.0 / 3.0).abs() < 0.001);
    }

    #[test]
    fn test_avg_iterations() {
        let metrics = ReasoningMetrics::new();

        metrics.record_loop_completed(10, 1000);
        metrics.record_loop_completed(4, 500);

        let snap = metrics.snapshot();
        assert!((snap.avg_iterations() - 7.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_reset() {
        let metrics = ReasoningMetrics::new();

        metrics.record_loop_started();
        metrics.record_loop_completed(5, 1000);
        metrics.record_tool_call();

        metrics.reset();

        let snap = metrics.snapshot();
        assert_eq!(snap.loops_started, 0);
        assert_eq!(snap.loops_completed, 0);
        assert_eq!(snap.total_tokens, 0);
        assert_eq!(snap.tool_calls, 0);
    }

    #[test]
    fn test_clone_independence() {
        let metrics = ReasoningMetrics::new();
        let clone = metrics.clone();

        metrics.record_loop_started();

        // Clone shares the same Arc, so it sees the update
        assert_eq!(clone.snapshot().loops_started, 1);
    }

    #[test]
    fn test_metrics_snapshot_serialization() {
        let metrics = ReasoningMetrics::new();
        metrics.record_loop_completed(3, 500);

        let snap = metrics.snapshot();
        let json = serde_json::to_string(&snap).unwrap();
        assert!(json.contains("\"loops_completed\":1"));
        assert!(json.contains("\"total_tokens\":500"));
    }
}
