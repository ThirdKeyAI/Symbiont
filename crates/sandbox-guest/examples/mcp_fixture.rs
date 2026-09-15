//! Deterministic MCP process for local VM tests. It has no provider integration.
use serde_json::{json, Value};
use std::io::{BufRead, Write};

#[cfg(unix)]
fn main() -> anyhow::Result<()> {
    let schema: Value = serde_json::from_str(&std::env::var("FIXTURE_SCHEMA")?)?;
    let session = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );
    let canary = std::env::var("HOST_CANARY")?;
    let mut listed = false;
    for line in std::io::stdin().lock().lines() {
        let request: Value = serde_json::from_str(&line?)?;
        let Some(id) = request.get("id") else {
            continue;
        };
        let result = match request["method"].as_str().unwrap_or("") {
            "initialize" => {
                json!({"protocolVersion":request["params"]["protocolVersion"],"capabilities":{"tools":{}},"serverInfo":{"name":"guest-fixture","version":"1"}})
            }
            "tools/list" => {
                listed = true;
                std::fs::write("/tmp/discovered-session", &session)?;
                json!({"tools":[{"name":"echo","description":"Exact guest fixture","inputSchema":schema}]})
            }
            "tools/call" => {
                let text = request["params"]["arguments"]["text"]
                    .as_str()
                    .unwrap_or("");
                std::fs::write("/tmp/allowed-effect", text)?;
                let payload = json!({"text":std::fs::read_to_string("/tmp/allowed-effect")?,
                    "same_session":listed && std::fs::read_to_string("/tmp/discovered-session")?==session,
                    "session":session, "host_visible":std::path::Path::new(&canary).exists(),
                    "explicit":std::env::var("EXPLICIT_FIXTURE").ok(),
                    "ambient_visible":std::env::var_os("SYMBI_FIRECRACKER_AMBIENT_CANARY").is_some(),
                    "key_visible":std::env::var_os("OPENAI_API_KEY").is_some(),
                    // SAFETY: identity queries have no pointer arguments.
                    "uid":unsafe {libc::getuid()}, "gid":unsafe {libc::getgid()}});
                json!({"content":[{"type":"text","text":payload.to_string()}],"isError":false})
            }
            _ => json!({}),
        };
        println!("{}", json!({"jsonrpc":"2.0","id":id,"result":result}));
        std::io::stdout().flush()?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn main() {
    panic!("MCP guest fixture requires a Unix guest");
}
