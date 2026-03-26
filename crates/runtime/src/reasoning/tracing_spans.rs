//! Structured tracing for reasoning loops
//!
//! Instruments the reasoning loop with `tracing` spans for each phase.
//! Spans are emitted via the `tracing` crate and can be connected to
//! any subscriber (stdout, OpenTelemetry, etc.).

use crate::types::AgentId;
use std::time::{Duration, Instant};

/// Tracing context for a single reasoning loop execution.
///
/// Creates a parent span for the loop and child spans for each phase.
pub struct LoopTracer {
    agent_id: AgentId,
    loop_id: String,
    start: Instant,
    iteration: u32,
    /// Optional external request ID for correlation with API requests.
    request_id: Option<String>,
}

impl LoopTracer {
    /// Start tracing a new reasoning loop.
    pub fn start(agent_id: AgentId) -> Self {
        let loop_id = uuid::Uuid::new_v4().to_string();
        // Pre-compute traceparent for the initial log entry
        let trace_id_hex = loop_id.replace('-', "");
        let trace_id_str = if trace_id_hex.len() >= 32 {
            &trace_id_hex[..32]
        } else {
            &trace_id_hex
        };
        let parent_id_str = &trace_id_str[..16];
        let traceparent = format!("00-{trace_id_str}-{parent_id_str}-01");
        tracing::info!(
            agent_id = %agent_id,
            loop_id = %loop_id,
            traceparent = %traceparent,
            "Reasoning loop started"
        );
        Self {
            agent_id,
            loop_id,
            start: Instant::now(),
            iteration: 0,
            request_id: None,
        }
    }

    /// Set an external request ID for correlation with API requests.
    pub fn with_request_id(mut self, request_id: String) -> Self {
        tracing::info!(
            agent_id = %self.agent_id,
            loop_id = %self.loop_id,
            request_id = %request_id,
            "Correlated with API request"
        );
        self.request_id = Some(request_id);
        self
    }

    /// Get the request ID, if set.
    pub fn request_id(&self) -> Option<&str> {
        self.request_id.as_deref()
    }

    /// Begin a new iteration.
    pub fn begin_iteration(&mut self) {
        self.iteration += 1;
        tracing::debug!(
            agent_id = %self.agent_id,
            loop_id = %self.loop_id,
            iteration = self.iteration,
            "Iteration started"
        );
    }

    /// Trace the reasoning (LLM inference) phase.
    pub fn trace_reasoning(&self, prompt_tokens: u64, completion_tokens: u64, duration: Duration) {
        tracing::debug!(
            agent_id = %self.agent_id,
            loop_id = %self.loop_id,
            iteration = self.iteration,
            prompt_tokens,
            completion_tokens,
            duration_ms = duration.as_millis() as u64,
            "Reasoning phase completed"
        );
    }

    /// Trace a policy evaluation.
    pub fn trace_policy_check(&self, actions_proposed: usize, actions_allowed: usize) {
        tracing::debug!(
            agent_id = %self.agent_id,
            loop_id = %self.loop_id,
            iteration = self.iteration,
            actions_proposed,
            actions_allowed,
            "Policy check completed"
        );
    }

    /// Trace a tool dispatch.
    pub fn trace_tool_dispatch(&self, tool_name: &str, success: bool, duration: Duration) {
        if success {
            tracing::debug!(
                agent_id = %self.agent_id,
                loop_id = %self.loop_id,
                iteration = self.iteration,
                tool_name,
                duration_ms = duration.as_millis() as u64,
                "Tool call succeeded"
            );
        } else {
            tracing::warn!(
                agent_id = %self.agent_id,
                loop_id = %self.loop_id,
                iteration = self.iteration,
                tool_name,
                duration_ms = duration.as_millis() as u64,
                "Tool call failed"
            );
        }
    }

    /// Trace a policy denial.
    pub fn trace_policy_denial(&self, action: &str, reason: &str) {
        tracing::warn!(
            agent_id = %self.agent_id,
            loop_id = %self.loop_id,
            iteration = self.iteration,
            action,
            reason,
            "Policy denied action"
        );
    }

    /// Trace loop completion (success).
    pub fn trace_completed(&self, total_iterations: u32, total_tokens: u64) {
        let elapsed = self.start.elapsed();
        tracing::info!(
            agent_id = %self.agent_id,
            loop_id = %self.loop_id,
            total_iterations,
            total_tokens,
            duration_ms = elapsed.as_millis() as u64,
            "Reasoning loop completed successfully"
        );
    }

    /// Trace loop termination (failure or limit).
    pub fn trace_terminated(&self, reason: &str, total_iterations: u32, total_tokens: u64) {
        let elapsed = self.start.elapsed();
        tracing::warn!(
            agent_id = %self.agent_id,
            loop_id = %self.loop_id,
            reason,
            total_iterations,
            total_tokens,
            duration_ms = elapsed.as_millis() as u64,
            "Reasoning loop terminated"
        );
    }

    /// Generate a W3C traceparent header value for this loop.
    /// Format: {version}-{trace-id}-{parent-id}-{trace-flags}
    /// Example: 00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01
    pub fn traceparent(&self) -> String {
        // Use loop_id as trace-id (pad/truncate to 32 hex chars)
        let trace_id = self.loop_id.replace('-', "");
        let trace_id = if trace_id.len() >= 32 {
            &trace_id[..32]
        } else {
            &trace_id
        };
        // Generate a parent span id (16 hex chars from first 8 bytes of loop_id)
        let parent_id = &trace_id[..16];
        format!("00-{trace_id}-{parent_id}-01")
    }

    /// Parse a W3C traceparent header into (trace_id, parent_id).
    /// Returns None if the header is malformed.
    pub fn parse_traceparent(header: &str) -> Option<(String, String)> {
        let parts: Vec<&str> = header.split('-').collect();
        if parts.len() != 4 || parts[0] != "00" {
            return None;
        }
        if parts[1].len() != 32 || parts[2].len() != 16 {
            return None;
        }
        Some((parts[1].to_string(), parts[2].to_string()))
    }

    /// Get the trace ID for this loop (for external correlation).
    pub fn trace_id(&self) -> &str {
        &self.loop_id
    }

    /// Get the loop ID.
    pub fn loop_id(&self) -> &str {
        &self.loop_id
    }

    /// Get elapsed time since loop start.
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    /// Get current iteration.
    pub fn current_iteration(&self) -> u32 {
        self.iteration
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_loop_tracer_creation() {
        let agent_id = AgentId::new();
        let tracer = LoopTracer::start(agent_id);
        assert_eq!(tracer.current_iteration(), 0);
        assert!(!tracer.loop_id().is_empty());
    }

    #[test]
    fn test_iteration_tracking() {
        let agent_id = AgentId::new();
        let mut tracer = LoopTracer::start(agent_id);

        tracer.begin_iteration();
        assert_eq!(tracer.current_iteration(), 1);

        tracer.begin_iteration();
        assert_eq!(tracer.current_iteration(), 2);
    }

    #[test]
    fn test_trace_methods_dont_panic() {
        let agent_id = AgentId::new();
        let mut tracer = LoopTracer::start(agent_id);

        tracer.begin_iteration();
        tracer.trace_reasoning(100, 50, Duration::from_millis(500));
        tracer.trace_policy_check(3, 2);
        tracer.trace_tool_dispatch("search", true, Duration::from_millis(200));
        tracer.trace_tool_dispatch("write", false, Duration::from_millis(100));
        tracer.trace_policy_denial("write_file", "blocked by policy");
        tracer.trace_completed(1, 150);
    }

    #[test]
    fn test_trace_terminated() {
        let agent_id = AgentId::new();
        let tracer = LoopTracer::start(agent_id);
        tracer.trace_terminated("max_iterations", 10, 5000);
    }

    #[test]
    fn test_traceparent_format() {
        let agent_id = AgentId::new();
        let tracer = LoopTracer::start(agent_id);
        let tp = tracer.traceparent();

        // Should match W3C format: version-trace_id-parent_id-flags
        let parts: Vec<&str> = tp.split('-').collect();
        assert_eq!(parts.len(), 4);
        assert_eq!(parts[0], "00"); // version
        assert_eq!(parts[1].len(), 32); // trace-id: 32 hex chars
        assert_eq!(parts[2].len(), 16); // parent-id: 16 hex chars
        assert_eq!(parts[3], "01"); // flags: sampled

        // parent-id should be the first 16 chars of trace-id
        assert_eq!(parts[2], &parts[1][..16]);
    }

    #[test]
    fn test_parse_traceparent_valid() {
        let header = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
        let result = LoopTracer::parse_traceparent(header);
        assert!(result.is_some());
        let (trace_id, parent_id) = result.unwrap();
        assert_eq!(trace_id, "4bf92f3577b34da6a3ce929d0e0e4736");
        assert_eq!(parent_id, "00f067aa0ba902b7");
    }

    #[test]
    fn test_parse_traceparent_invalid_version() {
        assert!(LoopTracer::parse_traceparent("01-abc-def-01").is_none());
    }

    #[test]
    fn test_parse_traceparent_wrong_lengths() {
        // trace-id too short
        assert!(LoopTracer::parse_traceparent("00-abc-00f067aa0ba902b7-01").is_none());
        // parent-id too short
        assert!(
            LoopTracer::parse_traceparent("00-4bf92f3577b34da6a3ce929d0e0e4736-short-01").is_none()
        );
    }

    #[test]
    fn test_parse_traceparent_too_few_parts() {
        assert!(LoopTracer::parse_traceparent("00-abc-def").is_none());
    }

    #[test]
    fn test_traceparent_roundtrip() {
        let agent_id = AgentId::new();
        let tracer = LoopTracer::start(agent_id);
        let tp = tracer.traceparent();
        let parsed = LoopTracer::parse_traceparent(&tp);
        assert!(parsed.is_some());
    }

    #[test]
    fn test_with_request_id() {
        let agent_id = AgentId::new();
        let tracer = LoopTracer::start(agent_id).with_request_id("req-abc-123".to_string());
        assert_eq!(tracer.request_id(), Some("req-abc-123"));
    }

    #[test]
    fn test_request_id_default_none() {
        let agent_id = AgentId::new();
        let tracer = LoopTracer::start(agent_id);
        assert!(tracer.request_id().is_none());
    }

    #[test]
    fn test_elapsed_increases() {
        let agent_id = AgentId::new();
        let tracer = LoopTracer::start(agent_id);

        // Elapsed should return a valid duration after creation
        let elapsed = tracer.elapsed();
        assert!(elapsed.as_nanos() < u128::MAX);
    }
}
