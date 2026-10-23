//! A raw `ActionExecutor::execute_actions` proposal never reaches an MCP
//! server through ToolClad. MCP-backed tools run only from an authorization
//! grant issued by the governed dispatcher (`execute_authorized`), so an
//! ungoverned call is refused before any registry lookup or process launch.
//!
//! The governed MCP path is covered by the dispatcher and sandbox suites;
//! the ungoverned fixture helper by `toolclad_mcp.rs` (feature
//! `toolclad-direct`).
#![cfg(feature = "mcp-client")]

use symbi_runtime::reasoning::circuit_breaker::CircuitBreakerRegistry;
use symbi_runtime::reasoning::executor::ActionExecutor;
use symbi_runtime::reasoning::loop_types::{LoopConfig, ProposedAction};
use symbi_runtime::toolclad::{parse_manifest, ToolCladExecutor};

const MANIFEST: &str = r#"
[tool]
name = "echo"
version = "1.0.0"
description = "Echo via MCP"

[args.text]
position = 1
required = true
type = "string"
description = "Text to echo"

[mcp]
server = "hang"
tool = "echo"

[output]
format = "json"
"#;

#[tokio::test(flavor = "multi_thread")]
async fn execute_actions_refuses_ungoverned_mcp_calls_without_launching_a_server() {
    // The manifest names a server no registry defines; the refusal comes
    // before any registry lookup or process launch.
    let manifest = parse_manifest(MANIFEST, &Default::default()).expect("manifest");
    let executor = ToolCladExecutor::new(vec![("echo".to_string(), manifest)])
        .with_development_host_execution()
        .with_mcp_verification(false);
    let action = ProposedAction::ToolCall {
        call_id: "c1".to_string(),
        name: "echo".to_string(),
        arguments: r#"{"text":"hi"}"#.to_string(),
    };
    let start = std::time::Instant::now();
    let observations = executor
        .execute_actions(
            &[action],
            &LoopConfig::default(),
            &CircuitBreakerRegistry::default(),
        )
        .await;
    assert_eq!(observations.len(), 1);
    assert!(observations[0].is_error);
    assert_eq!(observations[0].call_id.as_deref(), Some("c1"));
    assert!(
        observations[0].content.contains("authorization grant"),
        "{}",
        observations[0].content
    );
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
}
