//! Integration test: ToolClad's `[mcp]` backend actually invokes the
//! configured MCP server over stdio — replacing the old fabricated
//! `"status":"delegated"` stub — using an in-memory registry so the test
//! doesn't depend on `mcp-config.toml` or the process's working directory.
#![cfg(all(feature = "mcp-client", feature = "toolclad-direct"))]

use std::collections::HashMap;

use symbi_runtime::integrations::mcp::registry::McpServerRegistry;
use symbi_runtime::toolclad::manifest::{parse_manifest, Manifest};
use symbi_runtime::toolclad::ToolCladExecutor;

fn build_echo_manifest() -> Manifest {
    parse_manifest(
        r#"
[tool]
name = "echo"
version = "1.0.0"
description = "Echo via MCP proxy"

[args.text]
position = 1
required = true
type = "string"
description = "Text to echo"

[mcp]
server = "echo"
tool = "echo"

[output]
format = "json"

[output.schema]
type = "object"
"#,
        &HashMap::new(),
    )
    .expect("echo manifest")
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
        .with_development_host_execution()
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
        .with_development_host_execution()
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
