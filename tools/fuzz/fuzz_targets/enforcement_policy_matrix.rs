#![no_main]

use std::collections::HashMap;

use futures::executor::block_on;
use libfuzzer_sys::{arbitrary::Arbitrary, fuzz_target};
use symbi_runtime::integrations::{
    DefaultToolInvocationEnforcer, EnforcementDecision, EnforcementPolicy, InvocationContext,
    InvocationEnforcementConfig, McpTool, ToolInvocationEnforcer, ToolProvider, VerificationResult,
    VerificationStatus,
};
use symbi_runtime::types::AgentId;

#[derive(Arbitrary, Debug)]
struct Input {
    /// Maps to 4 policies: Strict=0, Permissive=1, Development=2, Disabled=3
    policy_tag: u8,
    /// Maps to 4 statuses: Verified=0, Failed=1, Pending=2, Skipped=3
    status_tag: u8,
    block_failed_verification: bool,
    block_pending_verification: bool,
    allow_skipped_in_dev: bool,
    /// Clamped to 1..=20
    max_warnings_before_escalation: u8,
    tool_name: String,
    reason: String,
}

fn clamp(mut s: String, max: usize, fallback: &str) -> String {
    if s.is_empty() {
        return fallback.to_string();
    }
    if s.len() > max {
        // Find the nearest char boundary at or before `max` to avoid
        // panicking on multi-byte UTF-8 sequences.
        let mut end = max;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
    s
}

fn build_status(tag: u8, reason: String) -> VerificationStatus {
    match tag % 4 {
        0 => VerificationStatus::Verified {
            result: Box::new(VerificationResult {
                success: true,
                message: "fuzz verified".to_string(),
                schema_hash: Some("fuzz-hash".to_string()),
                public_key_url: Some("https://provider.example.com/pubkey".to_string()),
                signature: None,
                metadata: None,
                timestamp: Some("2026-01-01T00:00:00Z".to_string()),
            }),
            verified_at: "2026-01-01T00:00:00Z".to_string(),
        },
        1 => VerificationStatus::Failed {
            reason,
            failed_at: "2026-01-01T00:00:00Z".to_string(),
        },
        2 => VerificationStatus::Pending,
        _ => VerificationStatus::Skipped { reason },
    }
}

fn build_policy(tag: u8) -> EnforcementPolicy {
    match tag % 4 {
        0 => EnforcementPolicy::Strict,
        1 => EnforcementPolicy::Permissive,
        2 => EnforcementPolicy::Development,
        _ => EnforcementPolicy::Disabled,
    }
}

fuzz_target!(|input: Input| {
    let tool_name = clamp(input.tool_name, 64, "fuzz_tool");
    let reason = clamp(input.reason, 128, "fuzz-reason");
    let policy = build_policy(input.policy_tag);
    let status = build_status(input.status_tag, reason);

    // Clamp max_warnings_before_escalation to 1..=20
    let max_warnings = input.max_warnings_before_escalation.clamp(1, 20) as usize;

    let config = InvocationEnforcementConfig {
        policy: policy.clone(),
        block_failed_verification: input.block_failed_verification,
        block_pending_verification: input.block_pending_verification,
        allow_skipped_in_dev: input.allow_skipped_in_dev,
        max_warnings_before_escalation: max_warnings,
        ..Default::default()
    };

    let tool = McpTool {
        name: tool_name.clone(),
        description: "fuzzed tool".to_string(),
        schema: serde_json::json!({"type": "object"}),
        provider: ToolProvider {
            identifier: "provider.example.com".to_string(),
            name: "Provider".to_string(),
            public_key_url: "https://provider.example.com/pubkey".to_string(),
            version: Some("1.0.0".to_string()),
        },
        verification_status: status.clone(),
        metadata: None,
        sensitive_params: vec![],
    };

    let context = InvocationContext {
        agent_id: AgentId::new(),
        tool_name,
        arguments: serde_json::json!({"arg": "value"}),
        timestamp: chrono::Utc::now(),
        metadata: HashMap::new(),
        agent_credential: None,
    };

    let enforcer = DefaultToolInvocationEnforcer::with_config(config);
    let decision = block_on(enforcer.check_invocation_allowed(&tool, &context)).expect("decision");

    // ---------------------------------------------------------------
    // Security invariant 1: Strict mode — anything NOT Verified MUST Block
    // ---------------------------------------------------------------
    if matches!(policy, EnforcementPolicy::Strict)
        && !matches!(status, VerificationStatus::Verified { .. })
    {
        assert!(
            matches!(decision, EnforcementDecision::Block { .. }),
            "SECURITY VIOLATION: Strict mode allowed a non-Verified tool (status={:?})",
            status
        );
    }

    // ---------------------------------------------------------------
    // Security invariant 2: Disabled mode — MUST always Allow
    // ---------------------------------------------------------------
    if matches!(policy, EnforcementPolicy::Disabled) {
        assert!(
            matches!(decision, EnforcementDecision::Allow),
            "SECURITY VIOLATION: Disabled mode did not Allow (decision={:?}, status={:?})",
            decision,
            status
        );
    }

    // ---------------------------------------------------------------
    // Security invariant 3: Fail-closed execution — any Block decision
    // must cause execute_tool_with_enforcement to return Err(InvocationBlocked)
    // ---------------------------------------------------------------
    if matches!(decision, EnforcementDecision::Block { .. }) {
        let exec_result = block_on(enforcer.execute_tool_with_enforcement(&tool, context.clone()));
        assert!(
            matches!(
                exec_result,
                Err(symbi_runtime::integrations::ToolInvocationError::InvocationBlocked { .. })
            ),
            "FAIL-CLOSED VIOLATION: Block decision did not produce InvocationBlocked error (got {:?})",
            exec_result
        );
    }

    // ---------------------------------------------------------------
    // Security invariant 4: Permissive + block_failed_verification +
    // status=Failed → MUST Block
    // ---------------------------------------------------------------
    if matches!(policy, EnforcementPolicy::Permissive)
        && input.block_failed_verification
        && matches!(status, VerificationStatus::Failed { .. })
    {
        assert!(
            matches!(decision, EnforcementDecision::Block { .. }),
            "SECURITY VIOLATION: Permissive mode with block_failed=true did not Block a Failed tool (decision={:?})",
            decision
        );
    }

    // ---------------------------------------------------------------
    // Security invariant 5: Development + block_failed_verification +
    // status=Failed → MUST Block
    // ---------------------------------------------------------------
    if matches!(policy, EnforcementPolicy::Development)
        && input.block_failed_verification
        && matches!(status, VerificationStatus::Failed { .. })
    {
        assert!(
            matches!(decision, EnforcementDecision::Block { .. }),
            "SECURITY VIOLATION: Development mode with block_failed=true did not Block a Failed tool (decision={:?})",
            decision
        );
    }
});
