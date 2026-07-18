//! Integration test: spawn the `echo_mcp_server` fixture as a real subprocess
//! and drive it end-to-end through `RmcpStdioClient` over stdio — list tools,
//! then call the `echo` tool and check the round-tripped text.
#![cfg(feature = "mcp-client")]

use symbi_runtime::integrations::mcp::registry::StdioServerSpec;
use symbi_runtime::integrations::mcp::stdio_client::RmcpStdioClient;

#[tokio::test]
async fn lists_and_calls_echo_tool_over_stdio() {
    // Cargo sets `CARGO_BIN_EXE_<name>` for `[[bin]]` targets when running
    // integration tests (crates/runtime/tests/*.rs), pointing at the built
    // fixture binary.
    let exe = env!("CARGO_BIN_EXE_echo_mcp_server");
    let spec = StdioServerSpec {
        command: exe.to_string(),
        args: vec![],
        env: Default::default(),
        public_key_url: None,
    };

    let tools = RmcpStdioClient::list_tools(&spec).await.expect("list");
    assert!(tools.iter().any(|t| t.name == "echo"));

    let mut args = serde_json::Map::new();
    args.insert("text".into(), serde_json::json!("hello"));
    let result = RmcpStdioClient::call_tool(&spec, "echo", args)
        .await
        .expect("call");
    assert!(result.to_string().contains("hello"));
}

#[tokio::test]
async fn enforce_blocks_unverified_tool() {
    let exe = env!("CARGO_BIN_EXE_echo_mcp_server");
    let spec = StdioServerSpec {
        command: exe.to_string(),
        args: vec![],
        env: Default::default(),
        public_key_url: None,
    };
    let mut args = serde_json::Map::new();
    args.insert("text".into(), serde_json::json!("hi"));

    // enforced: unsigned tool blocked (fail-closed)
    let blocked = RmcpStdioClient::verified_invoke(&spec, "echo", args.clone(), true).await;
    assert!(
        blocked.is_err(),
        "unverified tool must be blocked under enforcement"
    );

    // not enforced (local dev opt-out): runs
    let ok = RmcpStdioClient::verified_invoke(&spec, "echo", args, false).await;
    assert!(ok.is_ok(), "unenforced invoke should run: {ok:?}");
}
