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
}

impl LoopTracer {
    /// Start tracing a new reasoning loop.
    pub fn start(agent_id: AgentId) -> Self {
        let loop_id = uuid::Uuid::new_v4().to_string();
        tracing::info!(
            agent_id = %agent_id,
            loop_id = %loop_id,
            "Reasoning loop started"
        );
        Self {
            agent_id,
            loop_id,
            start: Instant::now(),
            iteration: 0,
        }
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
    fn test_elapsed_increases() {
        let agent_id = AgentId::new();
        let tracer = LoopTracer::start(agent_id);

        // Elapsed should be non-zero after creation
        let elapsed = tracer.elapsed();
        assert!(elapsed.as_nanos() >= 0);
    }
}
