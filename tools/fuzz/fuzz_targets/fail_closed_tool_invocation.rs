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
    status_tag: u8,
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

fuzz_target!(|input: Input| {
    let tool_name = clamp(input.tool_name, 64, "fuzz_tool");
    let reason = clamp(input.reason, 128, "fuzz-reason");
    let status = build_status(input.status_tag, reason);

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

    let enforcer = DefaultToolInvocationEnforcer::with_config(InvocationEnforcementConfig {
        policy: EnforcementPolicy::Strict,
        ..Default::default()
    });

    let decision = block_on(enforcer.check_invocation_allowed(&tool, &context)).expect("decision");

    if matches!(status, VerificationStatus::Verified { .. }) {
        assert!(matches!(decision, EnforcementDecision::Allow));
    } else {
        assert!(matches!(decision, EnforcementDecision::Block { .. }));

        // In strict mode, non-verified tool execution must fail closed before execution.
        let exec_result = block_on(enforcer.execute_tool_with_enforcement(&tool, context.clone()));
        assert!(matches!(
            exec_result,
            Err(symbi_runtime::integrations::ToolInvocationError::InvocationBlocked { .. })
        ));
    }
});
