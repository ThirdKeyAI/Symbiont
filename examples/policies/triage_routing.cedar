// Grounded escalation: permit exec escalation ONLY when the severity
// derived from the TRUSTED ticket is critical. The worker's self-reported
// (claimed) severity is present in context for audit but is NOT the gate.
permit (
    principal,
    action == Action::"tool_call::escalate",
    resource
)
when { context.ticket_severity == "critical" };
