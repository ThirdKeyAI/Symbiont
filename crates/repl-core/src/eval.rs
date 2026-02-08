//! REPL Evaluation Engine
//!
//! Handles evaluation of DSL expressions and commands in the REPL.

use crate::dsl::{evaluator::DslEvaluator, lexer::Lexer, parser::Parser, DslValue};
use crate::error::{ReplError, Result};
use crate::runtime_bridge::RuntimeBridge;
use std::sync::Arc;
use uuid::Uuid;

/// REPL Engine that coordinates DSL evaluation
pub struct ReplEngine {
    /// DSL evaluator
    evaluator: DslEvaluator,
}

impl ReplEngine {
    /// Create a new REPL engine
    pub fn new(runtime_bridge: Arc<RuntimeBridge>) -> Self {
        let evaluator = DslEvaluator::new(runtime_bridge);

        Self { evaluator }
    }

    /// Evaluate an expression or command
    pub async fn evaluate(&self, input: &str) -> Result<String> {
        let trimmed = input.trim();

        if trimmed.is_empty() {
            return Err(ReplError::Evaluation("Empty expression".to_string()));
        }

        // Check for special commands first
        if let Some(result) = self.handle_repl_command(trimmed).await? {
            return Ok(result);
        }

        // Parse and evaluate as DSL
        match self.evaluate_dsl(trimmed).await {
            Ok(value) => Ok(self.format_value(value)),
            Err(e) => {
                // Try to evaluate as a simple expression for better UX
                if trimmed.contains('=') || trimmed.contains('+') || trimmed.contains('-') {
                    self.evaluate_simple_expression(trimmed)
                } else {
                    Err(e)
                }
            }
        }
    }

    /// Handle REPL-specific commands
    async fn handle_repl_command(&self, input: &str) -> Result<Option<String>> {
        let parts: Vec<&str> = input.split_whitespace().collect();
        if parts.is_empty() {
            return Ok(None);
        }

        match parts[0] {
            ":help" | ":h" => Ok(Some(self.show_help())),
            ":agents" => Ok(Some(self.list_agents().await)),
            ":agent" => {
                if parts.len() > 1 {
                    match parts[1] {
                        "list" => Ok(Some(self.list_agents().await)),
                        "start" => {
                            if parts.len() > 2 {
                                let agent_id = parts[2].parse::<Uuid>().map_err(|_| {
                                    ReplError::Evaluation("Invalid agent ID".to_string())
                                })?;
                                self.start_agent(agent_id).await
                            } else {
                                Err(ReplError::Evaluation(
                                    "Usage: :agent start <agent_id>".to_string(),
                                ))
                            }
                        }
                        "stop" => {
                            if parts.len() > 2 {
                                let agent_id = parts[2].parse::<Uuid>().map_err(|_| {
                                    ReplError::Evaluation("Invalid agent ID".to_string())
                                })?;
                                self.stop_agent(agent_id).await
                            } else {
                                Err(ReplError::Evaluation(
                                    "Usage: :agent stop <agent_id>".to_string(),
                                ))
                            }
                        }
                        "pause" => {
                            if parts.len() > 2 {
                                let agent_id = parts[2].parse::<Uuid>().map_err(|_| {
                                    ReplError::Evaluation("Invalid agent ID".to_string())
                                })?;
                                self.pause_agent(agent_id).await
                            } else {
                                Err(ReplError::Evaluation(
                                    "Usage: :agent pause <agent_id>".to_string(),
                                ))
                            }
                        }
                        "resume" => {
                            if parts.len() > 2 {
                                let agent_id = parts[2].parse::<Uuid>().map_err(|_| {
                                    ReplError::Evaluation("Invalid agent ID".to_string())
                                })?;
                                self.resume_agent(agent_id).await
                            } else {
                                Err(ReplError::Evaluation(
                                    "Usage: :agent resume <agent_id>".to_string(),
                                ))
                            }
                        }
                        "destroy" => {
                            if parts.len() > 2 {
                                let agent_id = parts[2].parse::<Uuid>().map_err(|_| {
                                    ReplError::Evaluation("Invalid agent ID".to_string())
                                })?;
                                self.destroy_agent(agent_id).await
                            } else {
                                Err(ReplError::Evaluation(
                                    "Usage: :agent destroy <agent_id>".to_string(),
                                ))
                            }
                        }
                        "execute" => {
                            if parts.len() > 3 {
                                let agent_id = parts[2].parse::<Uuid>().map_err(|_| {
                                    ReplError::Evaluation("Invalid agent ID".to_string())
                                })?;
                                let behavior_name = parts[3];
                                // Parse remaining parts as arguments
                                let args = parts[4..].join(" ");
                                self.execute_agent_behavior(agent_id, behavior_name, &args)
                                    .await
                            } else {
                                Err(ReplError::Evaluation(
                                    "Usage: :agent execute <agent_id> <behavior> [args...]"
                                        .to_string(),
                                ))
                            }
                        }
                        "debug" => {
                            if parts.len() > 2 {
                                let agent_id = parts[2].parse::<Uuid>().map_err(|_| {
                                    ReplError::Evaluation("Invalid agent ID".to_string())
                                })?;
                                self.debug_agent(agent_id).await
                            } else {
                                Err(ReplError::Evaluation(
                                    "Usage: :agent debug <agent_id>".to_string(),
                                ))
                            }
                        }
                        _ => Err(ReplError::Evaluation(
                            "Unknown agent command. Use :help for available commands".to_string(),
                        )),
                    }
                } else {
                    Ok(Some(self.list_agents().await))
                }
            }
            ":snapshot" => {
                let snapshot = self.evaluator.create_snapshot().await;
                Ok(Some(format!("Created snapshot: {}", snapshot.id)))
            }
            ":monitor" => {
                if parts.len() > 1 {
                    match parts[1] {
                        "stats" => self.show_monitor_stats().await,
                        "traces" => {
                            let limit = if parts.len() > 2 {
                                parts[2].parse().unwrap_or(20)
                            } else {
                                20
                            };
                            self.show_traces(limit).await
                        }
                        "report" => self.show_monitor_report().await,
                        "clear" => self.clear_monitor().await,
                        _ => Err(ReplError::Evaluation(
                            "Unknown monitor command. Use :help for available commands".to_string(),
                        )),
                    }
                } else {
                    self.show_monitor_stats().await
                }
            }
            ":clear" => Ok(Some("Session cleared".to_string())),
            ":version" => Ok(Some("Symbiont REPL v0.3.0".to_string())),
            _ => Ok(None), // Not a REPL command
        }
    }

    /// Evaluate DSL code
    async fn evaluate_dsl(&self, input: &str) -> Result<DslValue> {
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize()?;

        let mut parser = Parser::new(tokens);
        let program = parser.parse()?;

        self.evaluator.execute_program(program).await
    }

    /// Simple expression evaluation for basic arithmetic
    fn evaluate_simple_expression(&self, input: &str) -> Result<String> {
        // Very basic arithmetic parser for immediate feedback
        if let Some(result) = self.try_basic_arithmetic(input) {
            Ok(result.to_string())
        } else {
            Err(ReplError::Evaluation(format!(
                "Unable to evaluate: {}",
                input
            )))
        }
    }

    /// Try to evaluate basic arithmetic expressions
    fn try_basic_arithmetic(&self, input: &str) -> Option<f64> {
        // Very simple arithmetic - just for demo purposes
        if let Ok(num) = input.parse::<f64>() {
            return Some(num);
        }

        // Handle simple addition
        if let Some(pos) = input.find('+') {
            let left = input[..pos].trim().parse::<f64>().ok()?;
            let right = input[pos + 1..].trim().parse::<f64>().ok()?;
            return Some(left + right);
        }

        // Handle simple subtraction
        if let Some(pos) = input.rfind('-') {
            if pos > 0 {
                // Not a negative number
                let left = input[..pos].trim().parse::<f64>().ok()?;
                let right = input[pos + 1..].trim().parse::<f64>().ok()?;
                return Some(left - right);
            }
        }

        // Handle simple multiplication
        if let Some(pos) = input.find('*') {
            let left = input[..pos].trim().parse::<f64>().ok()?;
            let right = input[pos + 1..].trim().parse::<f64>().ok()?;
            return Some(left * right);
        }

        // Handle simple division
        if let Some(pos) = input.find('/') {
            let left = input[..pos].trim().parse::<f64>().ok()?;
            let right = input[pos + 1..].trim().parse::<f64>().ok()?;
            if right != 0.0 {
                return Some(left / right);
            }
        }

        None
    }

    /// Format a DSL value for display
    fn format_value(&self, value: DslValue) -> String {
        Self::format_value_impl(value)
    }

    /// Internal implementation for formatting DSL values
    fn format_value_impl(value: DslValue) -> String {
        match value {
            DslValue::String(s) => format!("\"{}\"", s),
            DslValue::Number(n) => n.to_string(),
            DslValue::Integer(i) => i.to_string(),
            DslValue::Boolean(b) => b.to_string(),
            DslValue::Duration { value, unit } => {
                let unit_str = match unit {
                    crate::dsl::ast::DurationUnit::Milliseconds => "ms",
                    crate::dsl::ast::DurationUnit::Seconds => "s",
                    crate::dsl::ast::DurationUnit::Minutes => "m",
                    crate::dsl::ast::DurationUnit::Hours => "h",
                    crate::dsl::ast::DurationUnit::Days => "d",
                };
                format!("{}{}", value, unit_str)
            }
            DslValue::Size { value, unit } => {
                let unit_str = match unit {
                    crate::dsl::ast::SizeUnit::Bytes => "B",
                    crate::dsl::ast::SizeUnit::KB => "KB",
                    crate::dsl::ast::SizeUnit::MB => "MB",
                    crate::dsl::ast::SizeUnit::GB => "GB",
                    crate::dsl::ast::SizeUnit::TB => "TB",
                };
                format!("{}{}", value, unit_str)
            }
            DslValue::List(items) => {
                let formatted_items: Vec<String> =
                    items.into_iter().map(Self::format_value_impl).collect();
                format!("[{}]", formatted_items.join(", "))
            }
            DslValue::Map(entries) => {
                let formatted_entries: Vec<String> = entries
                    .into_iter()
                    .map(|(k, v)| format!("{}: {}", k, Self::format_value_impl(v)))
                    .collect();
                format!("{{{}}}", formatted_entries.join(", "))
            }
            DslValue::Null => "null".to_string(),
            DslValue::Agent(agent) => {
                format!("Agent(id: {}, state: {:?})", agent.id, agent.state)
            }
            DslValue::Function(name) => format!("Function({})", name),
            DslValue::Lambda(lambda) => format!("Lambda({} params)", lambda.parameters.len()),
        }
    }

    /// Show help message
    fn show_help(&self) -> String {
        r#"Symbiont REPL Commands:

DSL Expressions:
  agent MyAgent { ... }     - Define an agent
  behavior MyBehavior { ... } - Define a behavior
  function myFunc(...) { ... } - Define a function
  let x = 42               - Variable assignment
  x + y                    - Arithmetic expressions

REPL Commands:
  :help, :h               - Show this help
  :agents                 - List all agents
  :agent list             - List all agents
  :agent start <id>       - Start an agent
  :agent stop <id>        - Stop an agent
  :agent pause <id>       - Pause an agent
  :agent resume <id>      - Resume a paused agent
  :agent destroy <id>     - Destroy an agent
  :agent execute <id> <behavior> [args] - Execute agent behavior
  :agent debug <id>       - Show debug info for an agent
  :snapshot               - Create a session snapshot
  :monitor stats          - Show execution statistics
  :monitor traces [limit] - Show execution traces
  :monitor report         - Show detailed execution report
  :monitor clear          - Clear monitoring data
  :clear                  - Clear the session
  :version                - Show version information

Examples:
  agent TestAgent {
    name: "My Test Agent"
    version: "1.0.0"
  }
  
  behavior Greet {
    input { name: string }
    output { greeting: string }
    steps {
      let greeting = format("Hello, {}!", name)
      return greeting
    }
  }"#
        .to_string()
    }

    /// List all agents
    async fn list_agents(&self) -> String {
        let agents = self.evaluator.list_agents().await;

        if agents.is_empty() {
            "No agents created.".to_string()
        } else {
            let mut output = String::from("Agents:\n");
            for agent in agents {
                let state_str = format!("{:?}", agent.state);
                output.push_str(&format!(
                    "  {} - {} ({})\n",
                    agent.id, agent.definition.name, state_str
                ));
            }
            output
        }
    }

    /// Start an agent
    async fn start_agent(&self, agent_id: Uuid) -> Result<Option<String>> {
        match self.evaluator.start_agent(agent_id).await {
            Ok(()) => Ok(Some(format!("Started agent {}", agent_id))),
            Err(e) => Err(e),
        }
    }

    /// Stop an agent
    async fn stop_agent(&self, agent_id: Uuid) -> Result<Option<String>> {
        match self.evaluator.stop_agent(agent_id).await {
            Ok(()) => Ok(Some(format!("Stopped agent {}", agent_id))),
            Err(e) => Err(e),
        }
    }

    /// Pause an agent
    async fn pause_agent(&self, agent_id: Uuid) -> Result<Option<String>> {
        match self.evaluator.pause_agent(agent_id).await {
            Ok(()) => Ok(Some(format!("Paused agent {}", agent_id))),
            Err(e) => Err(e),
        }
    }

    /// Resume an agent
    async fn resume_agent(&self, agent_id: Uuid) -> Result<Option<String>> {
        match self.evaluator.resume_agent(agent_id).await {
            Ok(()) => Ok(Some(format!("Resumed agent {}", agent_id))),
            Err(e) => Err(e),
        }
    }

    /// Destroy an agent
    async fn destroy_agent(&self, agent_id: Uuid) -> Result<Option<String>> {
        match self.evaluator.destroy_agent(agent_id).await {
            Ok(()) => Ok(Some(format!("Destroyed agent {}", agent_id))),
            Err(e) => Err(e),
        }
    }

    /// Execute agent behavior
    async fn execute_agent_behavior(
        &self,
        agent_id: Uuid,
        behavior_name: &str,
        args: &str,
    ) -> Result<Option<String>> {
        match self
            .evaluator
            .execute_agent_behavior(agent_id, behavior_name, args)
            .await
        {
            Ok(result) => Ok(Some(format!(
                "Executed behavior '{}' on agent {}: {}",
                behavior_name,
                agent_id,
                self.format_value(result)
            ))),
            Err(e) => Err(e),
        }
    }

    /// Debug agent
    async fn debug_agent(&self, agent_id: Uuid) -> Result<Option<String>> {
        match self.evaluator.debug_agent(agent_id).await {
            Ok(debug_info) => Ok(Some(debug_info)),
            Err(e) => Err(e),
        }
    }

    /// Get the DSL evaluator (for advanced operations)
    pub fn evaluator(&self) -> &DslEvaluator {
        &self.evaluator
    }

    /// Show monitoring statistics
    async fn show_monitor_stats(&self) -> Result<Option<String>> {
        let stats = self.evaluator.monitor().get_stats();
        let mut output = String::from("Execution Monitor Statistics:\n");
        output.push_str(&format!("  Total Executions: {}\n", stats.total_executions));
        output.push_str(&format!("  Successful: {}\n", stats.successful_executions));
        output.push_str(&format!("  Failed: {}\n", stats.failed_executions));

        if stats.total_executions > 0 {
            let success_rate =
                (stats.successful_executions as f64 / stats.total_executions as f64) * 100.0;
            output.push_str(&format!("  Success Rate: {:.1}%\n", success_rate));
            output.push_str(&format!(
                "  Average Duration: {:?}\n",
                stats.average_duration
            ));
            output.push_str(&format!("  Total Duration: {:?}\n", stats.total_duration));
        }

        let active = self.evaluator.monitor().get_active_executions();
        output.push_str(&format!("  Active Executions: {}\n", active.len()));

        Ok(Some(output))
    }

    /// Show execution traces
    async fn show_traces(&self, limit: usize) -> Result<Option<String>> {
        let traces = self.evaluator.monitor().get_traces(Some(limit));

        if traces.is_empty() {
            return Ok(Some("No execution traces available.".to_string()));
        }

        let mut output = String::from("Recent Execution Traces:\n");
        for trace in traces.iter().rev() {
            let agent_info = if let Some(agent_id) = trace.agent_id {
                format!(" [Agent: {}]", agent_id)
            } else {
                String::new()
            };

            let duration_info = if let Some(duration) = trace.duration {
                format!(" ({:?})", duration)
            } else {
                String::new()
            };

            output.push_str(&format!(
                "  {} - {:?}{}{}\n",
                trace.timestamp.format("%H:%M:%S%.3f"),
                trace.event_type,
                agent_info,
                duration_info
            ));
        }

        Ok(Some(output))
    }

    /// Show detailed monitoring report
    async fn show_monitor_report(&self) -> Result<Option<String>> {
        let report = self.evaluator.monitor().generate_report();
        Ok(Some(report))
    }

    /// Clear monitoring data
    async fn clear_monitor(&self) -> Result<Option<String>> {
        self.evaluator.monitor().clear_traces();
        Ok(Some("Monitoring data cleared.".to_string()))
    }
}

/// Legacy function for backward compatibility
pub fn evaluate(expression: &str) -> Result<String> {
    // For simple expressions without async context
    if expression.is_empty() {
        return Err(ReplError::Evaluation("Empty expression".to_string()));
    }

    // Try basic arithmetic
    if let Ok(num) = expression.parse::<f64>() {
        return Ok(num.to_string());
    }

    // Handle simple addition
    if let Some(pos) = expression.find('+') {
        let left = expression[..pos]
            .trim()
            .parse::<f64>()
            .map_err(|_| ReplError::Evaluation("Invalid number".to_string()))?;
        let right = expression[pos + 1..]
            .trim()
            .parse::<f64>()
            .map_err(|_| ReplError::Evaluation("Invalid number".to_string()))?;
        return Ok((left + right).to_string());
    }

    // For now, just echo back complex expressions
    Ok(format!("ECHO: {}", expression))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_bridge::RuntimeBridge;

    async fn create_test_engine() -> ReplEngine {
        let runtime_bridge = Arc::new(RuntimeBridge::new());
        ReplEngine::new(runtime_bridge)
    }

    #[tokio::test]
    async fn test_basic_arithmetic() {
        let engine = create_test_engine().await;

        let result = engine.evaluate("2 + 3").await.unwrap();
        assert_eq!(result, "5");

        let result = engine.evaluate("10 - 4").await.unwrap();
        assert_eq!(result, "6");
    }

    #[tokio::test]
    async fn test_help_command() {
        let engine = create_test_engine().await;

        let result = engine.evaluate(":help").await.unwrap();
        assert!(result.contains("Symbiont REPL Commands"));
    }

    #[tokio::test]
    async fn test_version_command() {
        let engine = create_test_engine().await;

        let result = engine.evaluate(":version").await.unwrap();
        assert!(result.contains("Symbiont REPL"));
    }

    #[tokio::test]
    async fn test_agents_command() {
        let engine = create_test_engine().await;

        let result = engine.evaluate(":agents").await.unwrap();
        assert_eq!(result, "No agents created.");
    }

    #[test]
    fn test_legacy_evaluate() {
        let result = evaluate("42").unwrap();
        assert_eq!(result, "42");

        let result = evaluate("5 + 3").unwrap();
        assert_eq!(result, "8");
    }
}
