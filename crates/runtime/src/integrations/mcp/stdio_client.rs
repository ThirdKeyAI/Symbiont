//! Stdio MCP client (connect-per-invocation). Spawns the configured server
//! subprocess, performs the MCP handshake, and lists/calls tools over stdio.
//! Connection caching is deliberately deferred (v1): each call spawns a fresh
//! subprocess — correct and simple; optimize later if it matters.

use crate::integrations::mcp::registry::StdioServerSpec;
use crate::integrations::mcp::types::{McpTool, ToolProvider, VerificationStatus};
use crate::integrations::schemapin::key_store::LocalKeyStore;
use crate::integrations::schemapin::native_client::{NativeSchemaPinClient, SchemaPinClient};
use crate::integrations::schemapin::types::{PinnedKey, VerifyArgs};
use rmcp::{
    model::CallToolRequestParams,
    transport::{ConfigureCommandExt, TokioChildProcess},
    ServiceExt,
};
use sha2::Digest;
use std::io::Write;

pub struct RmcpStdioClient;

impl RmcpStdioClient {
    fn transport(spec: &StdioServerSpec) -> Result<TokioChildProcess, String> {
        let args = spec.args.clone();
        let env = spec.env.clone();
        TokioChildProcess::new(
            tokio::process::Command::new(&spec.command).configure(|cmd| {
                cmd.args(&args);
                for (k, v) in &env {
                    cmd.env(k, v);
                }
            }),
        )
        .map_err(|e| format!("failed to spawn MCP server '{}': {}", spec.command, e))
    }

    /// Connect to the stdio MCP server described by `spec`, list its tools,
    /// then disconnect.
    pub async fn list_tools(spec: &StdioServerSpec) -> Result<Vec<rmcp::model::Tool>, String> {
        let client = ().serve(Self::transport(spec)?).await.map_err(|e| e.to_string())?;
        let tools = client.list_all_tools().await.map_err(|e| e.to_string());
        let _ = client.cancel().await;
        tools
    }

    /// Connect to the stdio MCP server described by `spec`, invoke `tool`
    /// with `args`, then disconnect. Returns the tool's content normalized to
    /// JSON. Fails closed: a `CallToolResult` with `is_error == true` is
    /// surfaced as `Err`, not `Ok`.
    pub async fn call_tool(
        spec: &StdioServerSpec,
        tool: &str,
        args: serde_json::Map<String, serde_json::Value>,
    ) -> Result<serde_json::Value, String> {
        let client = ().serve(Self::transport(spec)?).await.map_err(|e| e.to_string())?;
        let params = CallToolRequestParams::new(tool.to_string()).with_arguments(args);
        let out = client.call_tool(params).await.map_err(|e| e.to_string());
        let _ = client.cancel().await;
        let result = out?;
        if result.is_error.unwrap_or(false) {
            return Err(format!(
                "tool '{}' reported error: {}",
                tool,
                serde_json::to_string(&result.content).unwrap_or_default()
            ));
        }
        // Normalize content to JSON for the ToolClad envelope.
        serde_json::to_value(&result.content).map_err(|e| e.to_string())
    }

    /// Discover `tool` on the stdio MCP server described by `spec`, gate its
    /// invocation behind SchemaPin/TOFU verification, then invoke it.
    ///
    /// Fail-closed: when `enforce` is `true`, the tool's schema must carry a
    /// verifiable SchemaPin signature (checked via [`NativeSchemaPinClient`] +
    /// [`LocalKeyStore`] TOFU pinning, mirroring `SecureMcpClient::verify_schema`
    /// in `integrations/mcp/client.rs`) or the call is blocked before it ever
    /// reaches the server — no side effects on the target tool occur. When
    /// `enforce` is `false` (local dev opt-out), the tool runs unconditionally.
    pub async fn verified_invoke(
        spec: &StdioServerSpec,
        tool: &str,
        args: serde_json::Map<String, serde_json::Value>,
        enforce: bool,
    ) -> Result<serde_json::Value, String> {
        if enforce {
            // Discover the tool schema from the live server.
            let tools = Self::list_tools(spec).await?;
            let rmcp_tool = tools
                .iter()
                .find(|t| t.name.as_ref() == tool)
                .ok_or_else(|| format!("tool '{}' not found on server", tool))?;

            let mcp_tool = McpTool {
                name: rmcp_tool.name.to_string(),
                description: rmcp_tool
                    .description
                    .clone()
                    .map(|c| c.to_string())
                    .unwrap_or_default(),
                schema: serde_json::to_value(&*rmcp_tool.input_schema)
                    .map_err(|e| e.to_string())?,
                provider: ToolProvider {
                    identifier: spec.command.clone(),
                    name: spec.command.clone(),
                    // Sourced from the server registry entry; empty when the
                    // operator hasn't configured a key, which blocks the tool
                    // fail-closed under enforcement (verify_via_schemapin).
                    public_key_url: spec.public_key_url.clone().unwrap_or_default(),
                    version: None,
                },
                verification_status: VerificationStatus::Pending,
                metadata: None,
                sensitive_params: Vec::new(),
            };

            let key_store = LocalKeyStore::new().map_err(|e| e.to_string())?;
            let client = NativeSchemaPinClient::new();
            let verified = verify_via_schemapin(&client, &key_store, &mcp_tool).await?;
            if !verified {
                return Err(format!(
                    "tool '{}' is not SchemaPin-verified and enforcement is on (fail-closed). \
                     Sign the tool schema or run with verification disabled for local dev.",
                    tool
                ));
            }
        }

        Self::call_tool(spec, tool, args).await
    }
}

/// Verify `tool`'s schema via SchemaPin, TOFU-pinning the provider's public
/// key on first use. Mirrors the verification shape used by
/// `SecureMcpClient::verify_schema` in `integrations/mcp/client.rs`: write the
/// schema to a temp file, pin the provider's public key, then delegate to
/// [`SchemaPinClient::verify_schema`].
///
/// Returns `Ok(false)` — not an error — when the schema carries no embedded
/// SchemaPin signature (SchemaPin convention: a top-level `signature` field in
/// the schema JSON, exactly what [`NativeSchemaPinClient::verify_schema`]
/// looks for) or has no usable public-key URL. The caller (`verified_invoke`)
/// blocks under enforcement in that case, since an unsigned/unverifiable tool
/// must never be treated as verified. Genuine verification errors (key-fetch
/// failure, I/O, malformed schema, etc.) propagate as `Err`, which also blocks
/// the call under enforcement — fail closed either way.
async fn verify_via_schemapin(
    client: &NativeSchemaPinClient,
    key_store: &LocalKeyStore,
    tool: &McpTool,
) -> Result<bool, String> {
    // SchemaPin convention: a signed schema embeds a top-level `signature`
    // field. No signature, or no public key to check it against, means there
    // is nothing to verify — unverified, not an error.
    let has_signature = tool
        .schema
        .get("signature")
        .and_then(|s| s.as_str())
        .is_some();
    if !has_signature || tool.provider.public_key_url.is_empty() {
        return Ok(false);
    }

    // TOFU: fetch the provider's public key and pin it on EVERY call. On first
    // contact this records the key; on every later contact `pin_key` re-affirms
    // it and rejects a swapped key (`KeyMismatch`) instead of silently trusting
    // it. This must NOT be gated on `!has_key` — doing so would make the pin
    // write-only and never detect a post-pin key swap. Mirrors
    // `SecureMcpClient::fetch_and_pin_key`.
    let fetch_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .https_only(true)
        .build()
        .map_err(|e| e.to_string())?;
    let response = fetch_client
        .get(&tool.provider.public_key_url)
        .send()
        .await
        .map_err(|e| format!("failed to fetch provider public key: {e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "provider public key fetch returned HTTP {}",
            response.status()
        ));
    }
    let key_data = response
        .text()
        .await
        .map_err(|e| format!("failed to read provider public key response: {e}"))?;
    if key_data.trim().is_empty() {
        return Err("provider public key response was empty".to_string());
    }
    let mut hasher = sha2::Sha256::new();
    hasher.update(key_data.as_bytes());
    let fingerprint = hex::encode(hasher.finalize());
    // `pin_key` is TOFU: Ok on first pin or a matching re-affirm, `KeyMismatch`
    // on a swap — which propagates as Err and blocks the call fail-closed.
    key_store
        .pin_key(PinnedKey::new(
            tool.provider.identifier.clone(),
            key_data,
            "ES256".to_string(),
            fingerprint,
        ))
        .map_err(|e| e.to_string())?;

    // `SchemaPinClient::verify_schema` takes a filesystem path, not inline
    // JSON, so write the schema to a temp file first.
    let mut temp_file = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    let schema_json = serde_json::to_string_pretty(&tool.schema).map_err(|e| e.to_string())?;
    temp_file
        .write_all(schema_json.as_bytes())
        .map_err(|e| e.to_string())?;
    let schema_path = temp_file.path().to_string_lossy().to_string();

    let verify_args = VerifyArgs::new(schema_path, tool.provider.public_key_url.clone());
    let result = client
        .verify_schema(verify_args)
        .await
        .map_err(|e| e.to_string())?;
    Ok(result.success)
}
