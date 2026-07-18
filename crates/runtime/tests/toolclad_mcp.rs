//! Integration test: ToolClad's `[mcp]` backend actually invokes the
//! configured MCP server over stdio — replacing the old fabricated
//! `"status":"delegated"` stub — using an in-memory registry so the test
//! doesn't depend on `mcp-config.toml` or the process's working directory.
#![cfg(feature = "mcp-client")]

use std::collections::HashMap;

use symbi_runtime::integrations::mcp::registry::McpServerRegistry;
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

/// Build an in-memory registry pointing the "echo" server at the
/// `echo_mcp_server` fixture binary (built as a `[[bin]]` target of this
/// crate, gated behind `mcp-client`). Cargo sets `CARGO_BIN_EXE_<name>` for
/// integration tests, giving us the absolute path to the compiled fixture.
fn build_registry() -> McpServerRegistry {
    let exe = env!("CARGO_BIN_EXE_echo_mcp_server");
    let toml = format!(
        r#"
[servers.echo]
command = "{}"
"#,
        exe.replace('\\', "\\\\")
    );
    McpServerRegistry::from_toml_str(&toml).expect("valid registry toml")
}

#[tokio::test]
async fn mcp_backend_invokes_real_server_and_returns_executed_envelope() {
    let manifest = build_echo_manifest();
    let executor = ToolCladExecutor::new(vec![("echo".to_string(), manifest.clone())])
        .with_mcp_verification(false);
    let registry = build_registry();

    let mut validated = HashMap::new();
    validated.insert("text".to_string(), "hello".to_string());

    let envelope = executor
        .execute_mcp_backend_async_with_registry(&registry, "echo", &manifest, &validated)
        .await
        .expect("mcp call should succeed");

    assert_eq!(envelope["status"], "executed");
    assert_eq!(envelope["tool"], "echo");
    assert_eq!(envelope["mcp_server"], "echo");
    assert_eq!(envelope["mcp_tool"], "echo");
    assert_eq!(envelope["exit_code"], 0);
    assert_eq!(envelope["stderr"], "");

    let results = envelope["results"].to_string();
    assert!(
        results.contains("hello"),
        "results should contain echoed text, got: {results}"
    );
}

#[tokio::test]
async fn mcp_backend_reports_missing_server() {
    let manifest = build_echo_manifest();
    let executor = ToolCladExecutor::new(vec![("echo".to_string(), manifest.clone())])
        .with_mcp_verification(false);
    // Empty registry: "echo" is not registered.
    let registry = McpServerRegistry::from_toml_str("").unwrap();

    let mut validated = HashMap::new();
    validated.insert("text".to_string(), "hello".to_string());

    let err = executor
        .execute_mcp_backend_async_with_registry(&registry, "echo", &manifest, &validated)
        .await
        .expect_err("unregistered server must fail closed");
    assert!(err.contains("echo"), "error should name the server: {err}");
}
