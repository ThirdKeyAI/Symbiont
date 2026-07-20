//! End-to-end: a reasoning turn's `ActionExecutor::execute_actions` path
//! invokes a real stdio MCP tool through ToolClad — exercising the async MCP
//! dispatch branch and the real `McpServerRegistry::load()` (CWD `mcp-config.toml`)
//! path that the reasoning loop uses, plus the fail-closed verification gate.
//!
//! These tests change the process working directory (so `load()` finds the
//! test's `mcp-config.toml`) and are therefore `#[serial]`.
#![cfg(feature = "mcp-client")]

use std::collections::HashMap;

use serial_test::serial;
use symbi_runtime::reasoning::circuit_breaker::CircuitBreakerRegistry;
use symbi_runtime::reasoning::executor::ActionExecutor;
use symbi_runtime::reasoning::loop_types::{LoopConfig, ProposedAction};
use symbi_runtime::toolclad::manifest::{
    ArgDef, CommandDef, Manifest, McpProxyDef, OutputDef, ToolMeta,
};
use symbi_runtime::toolclad::ToolCladExecutor;

fn build_echo_manifest() -> Manifest {
    let mut args = HashMap::new();
    args.insert(
        "text".to_string(),
        ArgDef {
            position: 1,
            required: true,
            type_name: "string".to_string(),
            description: "Text to echo".to_string(),
            allowed: None,
            default: None,
            pattern: None,
            sanitize: None,
            min: None,
            max: None,
            clamp: false,
            schemes: None,
            scope_check: false,
            feeds_decision: false,
        },
    );

    Manifest {
        tool: ToolMeta {
            name: "echo".to_string(),
            version: "1.0.0".to_string(),
            binary: String::new(),
            description: "Echo via MCP proxy".to_string(),
            mode: "oneshot".to_string(),
            timeout_seconds: 30,
            risk_tier: "low".to_string(),
            human_approval: false,
            cedar: None,
            evidence: None,
        },
        args,
        command: CommandDef::default(),
        output: OutputDef {
            format: "json".to_string(),
            parser: None,
            envelope: true,
            schema: serde_json::json!({"type": "object"}),
        },
        http: None,
        mcp: Some(McpProxyDef {
            server: "echo".to_string(),
            tool: "echo".to_string(),
            field_map: HashMap::new(),
        }),
        session: None,
        browser: None,
    }
}

/// Run `body` with the process CWD set to a fresh temp dir containing an
/// `mcp-config.toml` that points the "echo" server at the fixture binary.
/// Restores the original CWD afterward.
async fn with_echo_registry_cwd<F, Fut, T>(body: F) -> T
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = T>,
{
    let exe = env!("CARGO_BIN_EXE_echo_mcp_server");
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("mcp-config.toml"),
        format!(
            "[servers.echo]\ncommand = \"{}\"\n",
            exe.replace('\\', "\\\\")
        ),
    )
    .expect("write mcp-config.toml");

    let original = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(dir.path()).expect("set cwd");
    let result = body().await;
    std::env::set_current_dir(&original).expect("restore cwd");
    result
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn execute_actions_invokes_real_mcp_tool() {
    let observations = with_echo_registry_cwd(|| async {
        let executor = ToolCladExecutor::new(vec![("echo".to_string(), build_echo_manifest())])
            .with_mcp_verification(false);
        let action = ProposedAction::ToolCall {
            call_id: "c1".to_string(),
            name: "echo".to_string(),
            arguments: r#"{"text":"hi"}"#.to_string(),
        };
        executor
            .execute_actions(
                &[action],
                &LoopConfig::default(),
                &CircuitBreakerRegistry::default(),
            )
            .await
    })
    .await;

    assert_eq!(observations.len(), 1);
    let obs = &observations[0];
    assert!(!obs.is_error, "tool call should succeed, got: {obs:?}");
    assert_eq!(obs.call_id.as_deref(), Some("c1"));
    assert!(
        obs.content.contains("hi"),
        "observation should carry the echoed text, got: {}",
        obs.content
    );
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn execute_actions_fails_closed_when_verification_enforced() {
    // The fixture `echo` tool is unsigned and the registry entry has no
    // public_key_url, so with enforcement on it must be blocked — surfaced as
    // an is_error observation, never a fabricated success.
    let observations = with_echo_registry_cwd(|| async {
        let executor = ToolCladExecutor::new(vec![("echo".to_string(), build_echo_manifest())])
            .with_mcp_verification(true);
        let action = ProposedAction::ToolCall {
            call_id: "c1".to_string(),
            name: "echo".to_string(),
            arguments: r#"{"text":"hi"}"#.to_string(),
        };
        executor
            .execute_actions(
                &[action],
                &LoopConfig::default(),
                &CircuitBreakerRegistry::default(),
            )
            .await
    })
    .await;

    assert_eq!(observations.len(), 1);
    assert!(
        observations[0].is_error,
        "enforced verification must block the unsigned tool (fail-closed), got: {:?}",
        observations[0]
    );
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn execute_actions_times_out_a_hung_mcp_server() {
    // A registry entry pointing at `sleep` never completes the MCP handshake, so
    // the call would hang forever without a per-call timeout. execute_actions
    // must bound it (config.tool_timeout here) and surface an is_error
    // observation quickly — this is what protects the DSL tool_call() path,
    // which has no outer timeout of its own.
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("mcp-config.toml"),
        "[servers.hang]\ncommand = \"sleep\"\nargs = [\"30\"]\n",
    )
    .expect("write mcp-config.toml");

    // Manifest routes to the "hang" server; verification off so we exercise the
    // invoke path, not the schemapin gate.
    let mut manifest = build_echo_manifest();
    manifest.mcp = Some(McpProxyDef {
        server: "hang".to_string(),
        tool: "noop".to_string(),
        field_map: HashMap::new(),
    });

    let original = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(dir.path()).expect("set cwd");

    let executor =
        ToolCladExecutor::new(vec![("echo".to_string(), manifest)]).with_mcp_verification(false);
    let action = ProposedAction::ToolCall {
        call_id: "c1".to_string(),
        name: "echo".to_string(),
        arguments: r#"{"text":"hi"}"#.to_string(),
    };
    let config = LoopConfig {
        tool_timeout: std::time::Duration::from_millis(300),
        ..Default::default()
    };

    let start = std::time::Instant::now();
    let observations = executor
        .execute_actions(&[action], &config, &CircuitBreakerRegistry::default())
        .await;
    let elapsed = start.elapsed();

    std::env::set_current_dir(&original).expect("restore cwd");

    assert_eq!(observations.len(), 1);
    assert!(
        observations[0].is_error,
        "a hung MCP server must surface as an error, got: {:?}",
        observations[0]
    );
    assert!(
        observations[0].content.contains("timed out"),
        "observation should report a timeout, got: {}",
        observations[0].content
    );
    // Must return near the 300ms bound, not the 30s sleep.
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "timeout should fire promptly, took {elapsed:?}"
    );
}
