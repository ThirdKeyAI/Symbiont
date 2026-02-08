metadata {
    version = "1.0.0"
    author = "Symbiont Community"
    description = "Multi-step workflow orchestrator with dependency resolution and failure recovery"
    tags = ["workflow", "orchestration", "coordination", "state-management", "resilience"]
}

agent workflow_orchestrator(workflow_definition: WorkflowSpec) -> WorkflowResult {
    capabilities = ["workflow_management", "agent_coordination", "task_scheduling", "state_management"]

    policy workflow_execution {
        allow: ["invoke_agent", "message_passing", "state_read", "state_write", "checkpoint"]
            if agent.id in workflow_definition.allowed_agents && workflow.depth < 10
        deny: ["spawn_unlimited_agents", "recursive_orchestration", "external_network"]

        require: {
            workflow_approval: true,
            max_concurrent_agents: 50,
            total_timeout: "600000ms",
            checkpoint_enabled: true,
            circuit_breaker: true,
            failure_recovery: "retry_with_backoff",
            max_retry_attempts: 3
        }

        audit: {
            log_level: "info",
            include_workflow_graph: true,
            include_step_timing: true,
            include_dependencies: true,
            include_failure_trace: true,
            trace_id: true
        }
    }

    with
        memory = "persistent",
        coordination = "enabled",
        security = "high",
        sandbox = "Tier1",
        timeout = 600000,  // 10 minutes
        max_memory_mb = 2048,
        max_cpu_cores = 2.0
    {
        try {
            execution_context = WorkflowContext {
                workflow_id: generate_id(),
                start_time: now(),
                steps_completed: 0,
                current_step: 0,
                results: {},
                checkpoints: []
            };

            // Check if we're resuming from a checkpoint
            if let Some(checkpoint) = load_checkpoint(workflow_definition.id) {
                execution_context = checkpoint.context;
                log("INFO", "Resuming workflow from checkpoint: " + checkpoint.step_id);
            }

            for step in workflow_definition.steps {
                // Skip completed steps if resuming
                if execution_context.results.contains(step.name) {
                    continue;
                }

                try {
                    // Check if dependencies are met
                    if step.depends_on.length > 0 {
                        for dep in step.depends_on {
                            if !execution_context.results.contains(dep) {
                                return WorkflowResult {
                                    success: false,
                                    message: "Missing dependency: " + dep,
                                    execution_context: execution_context
                                };
                            }

                            if execution_context.results[dep].failed {
                                return WorkflowResult {
                                    success: false,
                                    message: "Dependency failure in step: " + step.name,
                                    execution_context: execution_context
                                };
                            }
                        }
                    }

                    // Execute step with circuit breaker
                    step_result = execute_workflow_step_with_retry(
                        step,
                        execution_context,
                        max_retries = 3
                    );

                    execution_context.results[step.name] = step_result;
                    execution_context.steps_completed += 1;

                    // Create checkpoint after each successful step
                    checkpoint(execution_context, step.name);

                } catch (StepExecutionError e) {
                    log("ERROR", "Workflow step failed: " + step.name + " - " + e.message);

                    if step.required {
                        return WorkflowResult {
                            success: false,
                            message: "Required step failed: " + step.name,
                            error: e.message,
                            execution_context: execution_context,
                            failed_at_step: step.name
                        };
                    } else {
                        // Mark optional step as failed but continue
                        execution_context.results[step.name] = {
                            failed: true,
                            error: e.message
                        };
                    }
                }

                execution_context.current_step += 1;
            }

            // Cleanup checkpoints on success
            clear_checkpoints(workflow_definition.id);

            return WorkflowResult {
                success: true,
                message: "Workflow completed successfully",
                execution_context: execution_context,
                execution_time_ms: time_since(execution_context.start_time)
            };

        } catch (error) {
            log("ERROR", "Workflow orchestration failed: " + error.message);
            return WorkflowResult {
                success: false,
                message: "Workflow failed",
                error: error.message,
                execution_context: execution_context
            };
        }
    }
}
