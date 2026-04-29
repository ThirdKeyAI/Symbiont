metadata {
    version = "1.0.0"
    author = "Symbiont Community"
    description = "Human-in-the-loop DevOps agent with read-only access that requires cryptographic approval before escalating privileges"
    tags = ["devops", "human-in-the-loop", "approval", "infrastructure", "least-privilege"]
}

# This agent monitors infrastructure with read-only access. Any action
# that modifies state (restart, deploy, scale) requires explicit human
# approval via a cryptographically signed approval token. Demonstrates
# the principle of least privilege with human oversight.

agent hitl_devops(action: String, target: Service) -> OperationResult {
    capabilities = ["read_logs", "read_metrics", "analyze", "request_approval"]

    policy least_privilege {
        // Always allow: reading logs, metrics, and status
        allow: read_logs(service) if true
        allow: read_metrics(service) if true
        allow: read_status(service) if true
        allow: analyze(data) if true

        // Require human approval for any state-changing action
        deny: restart(service) if not context.has_human_approval
        deny: deploy(service, version) if not context.has_human_approval
        deny: scale(service, count) if not context.has_human_approval
        deny: rollback(service) if not context.has_human_approval

        // Always deny destructive operations, even with approval
        deny: delete(service)
        deny: modify_network(any)
        deny: modify_secrets(any)

        require: {
            approval_mechanism: "cryptographic_token",
            approval_expiry: "5m",        // approval valid for 5 minutes
            approval_scope: "single_action", // one approval per action
            require_reason: true,          // approver must provide justification
            notify_on_escalation: true
        }

        audit: {
            log_level: "info",
            include_approval_chain: true,
            include_action_details: true,
            include_before_after_state: true,
            alert_on_escalation_request: true
        }
    }

    with
        sandbox = "Tier1",
        memory = "persistent",
        timeout = 300000,        // 5 minutes (includes human approval wait)
        max_memory_mb = 512
    {
        match action {
            "diagnose" => {
                // Read-only: no approval needed
                let logs = read_logs(target, lines = 500);
                let metrics = read_metrics(target, window = "15m");
                let status = read_status(target);

                let analysis = analyze({
                    logs: logs,
                    metrics: metrics,
                    status: status
                });

                return OperationResult {
                    action: "diagnose",
                    target: target.name,
                    status: "completed",
                    result: analysis,
                    approval_required: false
                };
            },

            "restart" => {
                // Step 1: Gather context (read-only)
                let status = read_status(target);
                let metrics = read_metrics(target, window = "5m");

                let reason = format(
                    "Service {} is {} (CPU: {}%, Memory: {}%, Error rate: {}/min). Restart recommended.",
                    target.name,
                    status.health,
                    metrics.cpu_percent,
                    metrics.memory_percent,
                    metrics.error_rate
                );

                // Step 2: Request human approval with context
                log("INFO", "Requesting approval to restart " + target.name);
                let approval = request_approval({
                    action: "restart",
                    target: target.name,
                    reason: reason,
                    risk_level: "medium",
                    rollback_available: true
                });

                // Step 3: Verify approval is cryptographically valid
                if !approval.verified {
                    return OperationResult {
                        action: "restart",
                        target: target.name,
                        status: "denied",
                        result: "Approval was not cryptographically verified",
                        approval_required: true
                    };
                }

                if approval.expired {
                    return OperationResult {
                        action: "restart",
                        target: target.name,
                        status: "denied",
                        result: "Approval token expired. Re-request required.",
                        approval_required: true
                    };
                }

                // Step 4: Execute with approval (policy gate checks context.has_human_approval)
                let before_state = read_status(target);
                restart(target);
                let after_state = read_status(target);

                return OperationResult {
                    action: "restart",
                    target: target.name,
                    status: "completed",
                    result: {
                        before: before_state,
                        after: after_state,
                        approved_by: approval.approver,
                        approval_reason: approval.reason
                    },
                    approval_required: true
                };
            },

            "deploy" => {
                // Similar pattern: gather context → request approval → execute
                log("INFO", "Deployment requires human approval");
                let approval = request_approval({
                    action: "deploy",
                    target: target.name,
                    risk_level: "high",
                    rollback_available: true
                });

                if !approval.verified || approval.expired {
                    return OperationResult {
                        action: "deploy",
                        target: target.name,
                        status: "denied",
                        result: "Valid approval required for deployment"
                    };
                }

                deploy(target, target.pending_version);

                return OperationResult {
                    action: "deploy",
                    target: target.name,
                    status: "completed",
                    result: { version: target.pending_version, approved_by: approval.approver }
                };
            },

            _ => {
                return error("Unknown action: " + action + ". Use: diagnose, restart, deploy");
            }
        }
    }
}
