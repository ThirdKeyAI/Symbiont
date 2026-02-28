//! Execution Monitoring and Debugging for Agent Behaviors
//!
//! Provides tracing, logging, and debugging capabilities for agent execution.

use crate::dsl::evaluator::{AgentInstance, DslValue};
use crate::error::{ReplError, Result};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Execution trace entry
#[derive(Debug, Clone)]
pub struct TraceEntry {
    pub id: Uuid,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub agent_id: Option<Uuid>,
    pub behavior_name: Option<String>,
    pub event_type: TraceEventType,
    pub data: JsonValue,
    pub duration: Option<Duration>,
}

/// Types of trace events
#[derive(Debug, Clone)]
pub enum TraceEventType {
    AgentCreated,
    AgentStarted,
    AgentStopped,
    AgentPaused,
    AgentResumed,
    AgentDestroyed,
    BehaviorStarted,
    BehaviorCompleted,
    BehaviorFailed,
    FunctionCalled,
    CapabilityChecked,
    PolicyEvaluated,
    VariableAssigned,
    ExpressionEvaluated,
    Error,
}

/// Execution statistics
#[derive(Debug, Clone, Default)]
pub struct ExecutionStats {
    pub total_executions: u64,
    pub successful_executions: u64,
    pub failed_executions: u64,
    pub average_duration: Duration,
    pub total_duration: Duration,
    pub behaviors_executed: HashMap<String, u64>,
    pub capabilities_checked: HashMap<String, u64>,
}

/// Execution monitor for tracking and debugging agent behavior
pub struct ExecutionMonitor {
    /// Trace entries
    traces: Arc<Mutex<Vec<TraceEntry>>>,
    /// Execution statistics
    stats: Arc<Mutex<ExecutionStats>>,
    /// Maximum number of trace entries to keep
    max_traces: usize,
    /// Active execution contexts
    active_executions: Arc<Mutex<HashMap<Uuid, ExecutionContext>>>,
}

/// Active execution context for monitoring
#[derive(Debug, Clone)]
pub struct ExecutionContext {
    pub id: Uuid,
    pub agent_id: Option<Uuid>,
    pub behavior_name: Option<String>,
    pub start_time: Instant,
    pub stack: Vec<String>,
    pub variables: HashMap<String, DslValue>,
}

impl Default for ExecutionMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl ExecutionMonitor {
    /// Create a new execution monitor
    pub fn new() -> Self {
        Self {
            traces: Arc::new(Mutex::new(Vec::new())),
            stats: Arc::new(Mutex::new(ExecutionStats::default())),
            max_traces: 1000,
            active_executions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Start monitoring an execution
    pub fn start_execution(&self, agent_id: Option<Uuid>, behavior_name: Option<String>) -> Uuid {
        let execution_id = Uuid::new_v4();
        let context = ExecutionContext {
            id: execution_id,
            agent_id,
            behavior_name: behavior_name.clone(),
            start_time: Instant::now(),
            stack: Vec::new(),
            variables: HashMap::new(),
        };

        self.active_executions
            .lock()
            .unwrap()
            .insert(execution_id, context);

        // Add trace entry
        self.add_trace(TraceEntry {
            id: Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            agent_id,
            behavior_name,
            event_type: TraceEventType::BehaviorStarted,
            data: serde_json::json!({
                "execution_id": execution_id,
                "started_at": chrono::Utc::now()
            }),
            duration: None,
        });

        execution_id
    }

    /// End monitoring an execution
    pub fn end_execution(&self, execution_id: Uuid, result: Result<DslValue>) -> Option<Duration> {
        let context = self
            .active_executions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&execution_id);

        if let Some(context) = context {
            let duration = context.start_time.elapsed();

            let (event_type, success) = match result {
                Ok(_) => (TraceEventType::BehaviorCompleted, true),
                Err(_) => (TraceEventType::BehaviorFailed, false),
            };

            // Add trace entry
            self.add_trace(TraceEntry {
                id: Uuid::new_v4(),
                timestamp: chrono::Utc::now(),
                agent_id: context.agent_id,
                behavior_name: context.behavior_name.clone(),
                event_type,
                data: serde_json::json!({
                    "execution_id": execution_id,
                    "duration_ms": duration.as_millis(),
                    "success": success,
                    "result": match &result {
                        Ok(value) => value.to_json(),
                        Err(e) => serde_json::json!({"error": e.to_string()})
                    }
                }),
                duration: Some(duration),
            });

            // Update statistics
            self.update_stats(duration, success, &context.behavior_name);

            Some(duration)
        } else {
            None
        }
    }

    /// Add a trace entry
    pub fn add_trace(&self, entry: TraceEntry) {
        let mut traces = self.traces.lock().unwrap_or_else(|e| e.into_inner());
        traces.push(entry);

        // Keep only the most recent traces
        if traces.len() > self.max_traces {
            let excess = traces.len() - self.max_traces;
            traces.drain(0..excess);
        }
    }

    /// Log agent lifecycle event
    pub fn log_agent_event(&self, agent: &AgentInstance, event_type: TraceEventType) {
        self.add_trace(TraceEntry {
            id: Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            agent_id: Some(agent.id),
            behavior_name: None,
            event_type,
            data: serde_json::json!({
                "agent_id": agent.id,
                "agent_name": agent.definition.name,
                "state": format!("{:?}", agent.state),
                "timestamp": chrono::Utc::now()
            }),
            duration: None,
        });
    }

    /// Log function call
    pub fn log_function_call(
        &self,
        execution_id: Option<Uuid>,
        function_name: &str,
        args: &[DslValue],
    ) {
        if let Some(exec_id) = execution_id {
            if let Some(context) = self
                .active_executions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get_mut(&exec_id)
            {
                context.stack.push(function_name.to_string());
            }
        }

        self.add_trace(TraceEntry {
            id: Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            agent_id: execution_id.and_then(|id| {
                self.active_executions
                    .lock()
                    .unwrap()
                    .get(&id)
                    .and_then(|ctx| ctx.agent_id)
            }),
            behavior_name: None,
            event_type: TraceEventType::FunctionCalled,
            data: serde_json::json!({
                "function_name": function_name,
                "argument_count": args.len(),
                "arguments": args.iter().map(|arg| arg.to_json()).collect::<Vec<_>>()
            }),
            duration: None,
        });
    }

    /// Log capability check
    pub fn log_capability_check(&self, agent_id: Option<Uuid>, capability: &str, allowed: bool) {
        self.add_trace(TraceEntry {
            id: Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            agent_id,
            behavior_name: None,
            event_type: TraceEventType::CapabilityChecked,
            data: serde_json::json!({
                "capability": capability,
                "allowed": allowed
            }),
            duration: None,
        });

        // Update capability statistics
        let mut stats = self.stats.lock().unwrap_or_else(|e| e.into_inner());
        *stats
            .capabilities_checked
            .entry(capability.to_string())
            .or_insert(0) += 1;
    }

    /// Log variable assignment
    pub fn log_variable_assignment(
        &self,
        execution_id: Option<Uuid>,
        var_name: &str,
        value: &DslValue,
    ) {
        if let Some(exec_id) = execution_id {
            if let Some(context) = self
                .active_executions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get_mut(&exec_id)
            {
                context
                    .variables
                    .insert(var_name.to_string(), value.clone());
            }
        }

        self.add_trace(TraceEntry {
            id: Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            agent_id: execution_id.and_then(|id| {
                self.active_executions
                    .lock()
                    .unwrap()
                    .get(&id)
                    .and_then(|ctx| ctx.agent_id)
            }),
            behavior_name: None,
            event_type: TraceEventType::VariableAssigned,
            data: serde_json::json!({
                "variable_name": var_name,
                "value": value.to_json(),
                "type": value.type_name()
            }),
            duration: None,
        });
    }

    /// Log error
    pub fn log_error(&self, agent_id: Option<Uuid>, error: &ReplError) {
        self.add_trace(TraceEntry {
            id: Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            agent_id,
            behavior_name: None,
            event_type: TraceEventType::Error,
            data: serde_json::json!({
                "error": error.to_string(),
                "error_type": match error {
                    ReplError::Lexing(_) => "Lexing",
                    ReplError::Parsing(_) => "Parsing",
                    ReplError::Execution(_) => "Execution",
                    ReplError::Security(_) => "Security",
                    ReplError::Runtime(_) => "Runtime",
                    ReplError::Evaluation(_) => "Evaluation",
                    ReplError::PolicyParsing(_) => "PolicyParsing",
                    ReplError::Io(_) => "Io",
                    ReplError::Json(_) => "Json",
                    ReplError::Uuid(_) => "Uuid",
                }
            }),
            duration: None,
        });
    }

    /// Get execution traces
    pub fn get_traces(&self, limit: Option<usize>) -> Vec<TraceEntry> {
        let traces = self.traces.lock().unwrap_or_else(|e| e.into_inner());
        let start_idx = if let Some(limit) = limit {
            traces.len().saturating_sub(limit)
        } else {
            0
        };
        traces[start_idx..].to_vec()
    }

    /// Get traces for a specific agent
    pub fn get_agent_traces(&self, agent_id: Uuid, limit: Option<usize>) -> Vec<TraceEntry> {
        let traces = self.traces.lock().unwrap_or_else(|e| e.into_inner());
        let agent_traces: Vec<_> = traces
            .iter()
            .filter(|trace| trace.agent_id == Some(agent_id))
            .cloned()
            .collect();

        if let Some(limit) = limit {
            let start_idx = agent_traces.len().saturating_sub(limit);
            agent_traces[start_idx..].to_vec()
        } else {
            agent_traces
        }
    }

    /// Get execution statistics
    pub fn get_stats(&self) -> ExecutionStats {
        self.stats.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Get active executions
    pub fn get_active_executions(&self) -> Vec<ExecutionContext> {
        self.active_executions
            .lock()
            .unwrap()
            .values()
            .cloned()
            .collect()
    }

    /// Clear all traces
    pub fn clear_traces(&self) {
        self.traces
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    /// Update execution statistics
    fn update_stats(&self, duration: Duration, success: bool, behavior_name: &Option<String>) {
        let mut stats = self.stats.lock().unwrap_or_else(|e| e.into_inner());

        stats.total_executions += 1;
        if success {
            stats.successful_executions += 1;
        } else {
            stats.failed_executions += 1;
        }

        stats.total_duration += duration;
        stats.average_duration = stats.total_duration / stats.total_executions as u32;

        if let Some(behavior) = behavior_name {
            *stats
                .behaviors_executed
                .entry(behavior.clone())
                .or_insert(0) += 1;
        }
    }

    /// Generate execution report
    pub fn generate_report(&self) -> String {
        let stats = self.get_stats();
        let active = self.get_active_executions();
        let recent_traces = self.get_traces(Some(20));

        let mut report = String::new();
        report.push_str("Execution Monitor Report\n");
        report.push_str("========================\n\n");

        // Statistics
        report.push_str("Execution Statistics:\n");
        report.push_str(&format!("  Total Executions: {}\n", stats.total_executions));
        report.push_str(&format!("  Successful: {}\n", stats.successful_executions));
        report.push_str(&format!("  Failed: {}\n", stats.failed_executions));
        if stats.total_executions > 0 {
            let success_rate =
                (stats.successful_executions as f64 / stats.total_executions as f64) * 100.0;
            report.push_str(&format!("  Success Rate: {:.1}%\n", success_rate));
        }
        report.push_str(&format!(
            "  Average Duration: {:?}\n",
            stats.average_duration
        ));
        report.push_str(&format!("  Total Duration: {:?}\n", stats.total_duration));

        // Active executions
        report.push_str(&format!("\nActive Executions: {}\n", active.len()));
        for exec in &active {
            let elapsed = exec.start_time.elapsed();
            report.push_str(&format!(
                "  {} - {:?} ({}s)\n",
                exec.id,
                exec.behavior_name.as_deref().unwrap_or("unknown"),
                elapsed.as_secs()
            ));
        }

        // Top behaviors
        if !stats.behaviors_executed.is_empty() {
            report.push_str("\nTop Behaviors:\n");
            let mut behaviors: Vec<_> = stats.behaviors_executed.iter().collect();
            behaviors.sort_by(|a, b| b.1.cmp(a.1));
            for (behavior, count) in behaviors.iter().take(10) {
                report.push_str(&format!("  {}: {} executions\n", behavior, count));
            }
        }

        // Recent traces
        if !recent_traces.is_empty() {
            report.push_str("\nRecent Activity:\n");
            for trace in recent_traces.iter().rev().take(10) {
                report.push_str(&format!(
                    "  {} - {:?}\n",
                    trace.timestamp.format("%H:%M:%S"),
                    trace.event_type
                ));
            }
        }

        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::evaluator::DslValue;

    #[test]
    fn test_execution_monitor_basic() {
        let monitor = ExecutionMonitor::new();

        let exec_id =
            monitor.start_execution(Some(Uuid::new_v4()), Some("test_behavior".to_string()));
        assert!(monitor.get_active_executions().len() == 1);

        let duration = monitor.end_execution(exec_id, Ok(DslValue::String("success".to_string())));
        assert!(duration.is_some());
        assert!(monitor.get_active_executions().is_empty());

        let traces = monitor.get_traces(None);
        assert_eq!(traces.len(), 2); // start + end
    }

    #[test]
    fn test_execution_monitor_stats() {
        let monitor = ExecutionMonitor::new();

        // Simulate some executions
        for i in 0..5 {
            let exec_id = monitor.start_execution(None, Some(format!("behavior_{}", i)));
            let result = if i % 2 == 0 {
                Ok(DslValue::Integer(42))
            } else {
                Err(ReplError::Execution("test error".to_string()))
            };
            monitor.end_execution(exec_id, result);
        }

        let stats = monitor.get_stats();
        assert_eq!(stats.total_executions, 5);
        assert_eq!(stats.successful_executions, 3);
        assert_eq!(stats.failed_executions, 2);
    }
}
