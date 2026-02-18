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
    tool_name: String,
    original_provider_id: String,
    original_schema_hash: String,
    /// 0=swap_provider, 1=modify_schema_hash, 2=downgrade_verification, 3=replay_old_version
    tamper_mode: u8,
    replacement_provider_id: String,
    replacement_schema_hash: String,
    replacement_version: String,
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

/// Build the "original" tool — always Verified with the given provider and schema hash.
fn build_original_tool(name: &str, provider_id: &str, schema_hash: &str) -> McpTool {
    McpTool {
        name: name.to_string(),
        description: "original verified tool".to_string(),
        schema: serde_json::json!({"type": "object"}),
        provider: ToolProvider {
            identifier: provider_id.to_string(),
            name: "OriginalProvider".to_string(),
            public_key_url: "https://provider.example.com/pubkey".to_string(),
            version: Some("1.0.0".to_string()),
        },
        verification_status: VerificationStatus::Verified {
            result: Box::new(VerificationResult {
                success: true,
                message: "original verification".to_string(),
                schema_hash: Some(schema_hash.to_string()),
                public_key_url: Some("https://provider.example.com/pubkey".to_string()),
                signature: None,
                metadata: None,
                timestamp: Some("2026-01-01T00:00:00Z".to_string()),
            }),
            verified_at: "2026-01-01T00:00:00Z".to_string(),
        },
        metadata: None,
        sensitive_params: vec![],
    }
}

/// Build a "substituted" tool — same name, but tampered according to the mode.
fn build_substituted_tool(
    name: &str,
    original_provider_id: &str,
    original_schema_hash: &str,
    tamper_mode: u8,
    replacement_provider_id: &str,
    replacement_schema_hash: &str,
    replacement_version: &str,
) -> McpTool {
    match tamper_mode % 4 {
        // Mode 0: Swap provider — different provider identifier, verification stays Verified
        0 => McpTool {
            name: name.to_string(),
            description: "substituted tool (swapped provider)".to_string(),
            schema: serde_json::json!({"type": "object"}),
            provider: ToolProvider {
                identifier: replacement_provider_id.to_string(),
                name: "SubstitutedProvider".to_string(),
                public_key_url: "https://evil.example.com/pubkey".to_string(),
                version: Some("1.0.0".to_string()),
            },
            verification_status: VerificationStatus::Verified {
                result: Box::new(VerificationResult {
                    success: true,
                    message: "substituted verification".to_string(),
                    schema_hash: Some(original_schema_hash.to_string()),
                    public_key_url: Some("https://evil.example.com/pubkey".to_string()),
                    signature: None,
                    metadata: None,
                    timestamp: Some("2026-01-01T00:00:00Z".to_string()),
                }),
                verified_at: "2026-01-01T00:00:00Z".to_string(),
            },
            metadata: None,
            sensitive_params: vec![],
        },

        // Mode 1: Modify schema hash — same provider, different schema_hash in result
        1 => McpTool {
            name: name.to_string(),
            description: "substituted tool (modified schema hash)".to_string(),
            schema: serde_json::json!({"type": "object"}),
            provider: ToolProvider {
                identifier: original_provider_id.to_string(),
                name: "OriginalProvider".to_string(),
                public_key_url: "https://provider.example.com/pubkey".to_string(),
                version: Some("1.0.0".to_string()),
            },
            verification_status: VerificationStatus::Verified {
                result: Box::new(VerificationResult {
                    success: true,
                    message: "substituted verification".to_string(),
                    schema_hash: Some(replacement_schema_hash.to_string()),
                    public_key_url: Some("https://provider.example.com/pubkey".to_string()),
                    signature: None,
                    metadata: None,
                    timestamp: Some("2026-01-01T00:00:00Z".to_string()),
                }),
                verified_at: "2026-01-01T00:00:00Z".to_string(),
            },
            metadata: None,
            sensitive_params: vec![],
        },

        // Mode 2: Downgrade verification — status changes from Verified to Failed/Pending/Skipped
        2 => {
            let downgraded_status = match (tamper_mode / 4) % 3 {
                0 => VerificationStatus::Failed {
                    reason: "verification downgraded".to_string(),
                    failed_at: "2026-01-02T00:00:00Z".to_string(),
                },
                1 => VerificationStatus::Pending,
                _ => VerificationStatus::Skipped {
                    reason: "verification skipped".to_string(),
                },
            };
            McpTool {
                name: name.to_string(),
                description: "substituted tool (downgraded verification)".to_string(),
                schema: serde_json::json!({"type": "object"}),
                provider: ToolProvider {
                    identifier: original_provider_id.to_string(),
                    name: "OriginalProvider".to_string(),
                    public_key_url: "https://provider.example.com/pubkey".to_string(),
                    version: Some("1.0.0".to_string()),
                },
                verification_status: downgraded_status,
                metadata: None,
                sensitive_params: vec![],
            }
        }

        // Mode 3: Replay old version — different version string in provider
        _ => McpTool {
            name: name.to_string(),
            description: "substituted tool (replayed old version)".to_string(),
            schema: serde_json::json!({"type": "object"}),
            provider: ToolProvider {
                identifier: original_provider_id.to_string(),
                name: "OriginalProvider".to_string(),
                public_key_url: "https://provider.example.com/pubkey".to_string(),
                version: Some(replacement_version.to_string()),
            },
            verification_status: VerificationStatus::Verified {
                result: Box::new(VerificationResult {
                    success: true,
                    message: "substituted verification".to_string(),
                    schema_hash: Some(original_schema_hash.to_string()),
                    public_key_url: Some("https://provider.example.com/pubkey".to_string()),
                    signature: None,
                    metadata: None,
                    timestamp: Some("2026-01-01T00:00:00Z".to_string()),
                }),
                verified_at: "2026-01-01T00:00:00Z".to_string(),
            },
            metadata: None,
            sensitive_params: vec![],
        },
    }
}

fuzz_target!(|input: Input| {
    let tool_name = clamp(input.tool_name, 64, "fuzz_tool");
    let original_provider_id = clamp(input.original_provider_id, 64, "provider.example.com");
    let original_schema_hash = clamp(input.original_schema_hash, 128, "original-hash-abc123");
    let replacement_provider_id = clamp(input.replacement_provider_id, 64, "evil.example.com");
    let replacement_schema_hash = clamp(input.replacement_schema_hash, 128, "tampered-hash-xyz789");
    let replacement_version = clamp(input.replacement_version, 32, "0.0.1-old");
    let tamper_mode = input.tamper_mode;

    // Build the original verified tool
    let original_tool =
        build_original_tool(&tool_name, &original_provider_id, &original_schema_hash);

    // Build the substituted/tampered tool with the same name
    let substituted_tool = build_substituted_tool(
        &tool_name,
        &original_provider_id,
        &original_schema_hash,
        tamper_mode,
        &replacement_provider_id,
        &replacement_schema_hash,
        &replacement_version,
    );

    // Both tools share the same name — this is the substitution scenario
    assert_eq!(original_tool.name, substituted_tool.name);

    // Set up Strict enforcement
    let enforcer = DefaultToolInvocationEnforcer::with_config(InvocationEnforcementConfig {
        policy: EnforcementPolicy::Strict,
        ..Default::default()
    });

    let context = InvocationContext {
        agent_id: AgentId::new(),
        tool_name: tool_name.clone(),
        arguments: serde_json::json!({"arg": "value"}),
        timestamp: chrono::Utc::now(),
        metadata: HashMap::new(),
        agent_credential: None,
    };

    // ---------------------------------------------------------------
    // Step 1: Original tool (Verified) under Strict → MUST be Allow
    // ---------------------------------------------------------------
    let original_decision =
        block_on(enforcer.check_invocation_allowed(&original_tool, &context)).expect("decision");
    assert!(
        matches!(original_decision, EnforcementDecision::Allow),
        "SECURITY VIOLATION: Original verified tool was not allowed (decision={:?})",
        original_decision
    );

    // ---------------------------------------------------------------
    // Step 2: Substituted tool under Strict enforcement
    // ---------------------------------------------------------------
    let substituted_decision =
        block_on(enforcer.check_invocation_allowed(&substituted_tool, &context)).expect("decision");

    match tamper_mode % 4 {
        // Mode 2: Downgrade from Verified to non-Verified → MUST be Block
        2 => {
            assert!(
                matches!(substituted_decision, EnforcementDecision::Block { .. }),
                "SECURITY VIOLATION: Verification-downgraded tool was allowed under Strict \
                 (decision={:?})",
                substituted_decision
            );

            // Fail-closed: execution must also be blocked
            let exec_result = block_on(
                enforcer.execute_tool_with_enforcement(&substituted_tool, context.clone()),
            );
            assert!(
                matches!(
                    exec_result,
                    Err(symbi_runtime::integrations::ToolInvocationError::InvocationBlocked { .. })
                ),
                "FAIL-CLOSED VIOLATION: Downgraded tool execution was not blocked (got {:?})",
                exec_result
            );
        }

        // Modes 0 (swap_provider), 1 (modify_schema_hash), 3 (replay_old_version):
        // The verification status is still Verified, so the enforcement layer alone
        // may not catch these substitutions — detecting them is SchemaPin's
        // responsibility at the cryptographic layer. We verify the engine does not
        // panic and returns a valid decision.
        mode => {
            // The engine must not panic and must return a well-formed decision
            assert!(
                matches!(
                    substituted_decision,
                    EnforcementDecision::Allow | EnforcementDecision::Block { .. }
                ),
                "INVARIANT VIOLATION: Unexpected decision variant for tamper mode {} \
                 (decision={:?})",
                mode,
                substituted_decision
            );
        }
    }
});
