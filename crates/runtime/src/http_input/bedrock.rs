//! AWS Bedrock Converse request building + response parsing (pure, network-free).
//! Behind the `bedrock` feature.

use serde_json::{json, Value};

/// Build a Bedrock Converse request body from the unified message/tool shape.
pub(crate) fn build_converse_request(
    system: &str,
    messages: &[Value],
    tools: &[Value],
    temperature: f32,
    max_tokens: u32,
) -> Value {
    let conv_messages: Vec<Value> = messages
        .iter()
        .map(|m| {
            let role = m.get("role").and_then(|r| r.as_str()).unwrap_or("user");
            let text = m.get("content").and_then(|c| c.as_str()).unwrap_or("");
            json!({ "role": role, "content": [ { "text": text } ] })
        })
        .collect();

    let mut req = json!({
        "messages": conv_messages,
        "system": [ { "text": system } ],
        "inferenceConfig": { "temperature": temperature, "maxTokens": max_tokens },
    });

    if !tools.is_empty() {
        let specs: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({ "toolSpec": {
                    "name": t.get("name").and_then(|n| n.as_str()).unwrap_or(""),
                    "description": t.get("description").and_then(|d| d.as_str()).unwrap_or(""),
                    "inputSchema": { "json": t.get("input_schema").cloned().unwrap_or_else(|| json!({"type":"object"})) }
                }})
            })
            .collect();
        req["toolConfig"] = json!({ "tools": specs });
    }
    req
}

/// Parse a Bedrock Converse response into the unified `{content, stop_reason}` shape.
pub(crate) fn parse_converse_response(resp: &Value) -> Value {
    let mut content_blocks = Vec::new();
    if let Some(blocks) = resp
        .get("output")
        .and_then(|o| o.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array())
    {
        for b in blocks {
            if let Some(text) = b.get("text").and_then(|t| t.as_str()) {
                if !text.is_empty() {
                    content_blocks.push(json!({ "type": "text", "text": text }));
                }
            } else if let Some(tu) = b.get("toolUse") {
                content_blocks.push(json!({
                    "type": "tool_use",
                    "id": tu.get("toolUseId").and_then(|i| i.as_str()).unwrap_or("unknown"),
                    "name": tu.get("name").and_then(|n| n.as_str()).unwrap_or("unknown"),
                    "input": tu.get("input").cloned().unwrap_or_else(|| json!({})),
                }));
            }
        }
    }
    let stop_reason = match resp.get("stopReason").and_then(|s| s.as_str()) {
        Some("tool_use") => "tool_use",
        _ => "end_turn",
    };
    json!({ "content": content_blocks, "stop_reason": stop_reason })
}

use crate::error::RuntimeError;
use aws_credential_types::provider::ProvideCredentials;
use std::time::SystemTime;

/// Sign a Bedrock Converse POST and return the headers to attach (incl. the
/// SigV4 `Authorization` header). Pure given (region, model, creds, body, time)
/// — used by the deterministic signing test. Targets aws-sigv4 1.4.5.
pub(crate) fn sign_converse_request(
    region: &str,
    model: &str,
    creds: &aws_credential_types::Credentials,
    body: &[u8],
    time: SystemTime,
) -> Result<Vec<(String, String)>, RuntimeError> {
    use aws_sigv4::http_request::{sign, SignableBody, SignableRequest, SigningSettings};
    use aws_sigv4::sign::v4;

    let url = format!("https://bedrock-runtime.{region}.amazonaws.com/model/{model}/converse");

    let identity = creds.clone().into();
    let signing_params = v4::SigningParams::builder()
        .identity(&identity)
        .region(region)
        .name("bedrock")
        .time(time)
        .settings(SigningSettings::default())
        .build()
        .map_err(|e| RuntimeError::Internal(format!("sigv4 params: {e}")))?;
    let params = signing_params.into();

    let headers = [("content-type", "application/json")];
    let signable = SignableRequest::new(
        "POST",
        &url,
        headers.iter().map(|(k, v)| (*k, *v)),
        SignableBody::Bytes(body),
    )
    .map_err(|e| RuntimeError::Internal(format!("sigv4 signable: {e}")))?;

    let out =
        sign(signable, &params).map_err(|e| RuntimeError::Internal(format!("sigv4 sign: {e}")))?;
    let (instructions, _signature) = out.into_parts();

    // Start with the request headers we declared, then add the SigV4 outputs
    // (Authorization, x-amz-date, and any others the signer wants applied).
    let mut result = vec![("content-type".to_string(), "application/json".to_string())];
    for (name, value) in instructions.headers() {
        result.push((name.to_string(), value.to_string()));
    }
    Ok(result)
}

/// Build the signed reqwest POST, send it, and parse the Converse response.
pub(crate) async fn converse(
    http: &reqwest::Client,
    region: &str,
    model: &str,
    creds: &aws_credential_types::provider::SharedCredentialsProvider,
    body: &serde_json::Value,
) -> Result<serde_json::Value, RuntimeError> {
    let url = format!("https://bedrock-runtime.{region}.amazonaws.com/model/{model}/converse");
    let body_bytes = serde_json::to_vec(body)
        .map_err(|e| RuntimeError::Internal(format!("serialize converse body: {e}")))?;

    let resolved = creds
        .provide_credentials()
        .await
        .map_err(|e| RuntimeError::Internal(format!("AWS credentials unavailable: {e}")))?;

    let headers = sign_converse_request(region, model, &resolved, &body_bytes, SystemTime::now())?;

    let mut req = http.post(&url).body(body_bytes);
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| RuntimeError::Internal(format!("bedrock request failed: {e}")))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(RuntimeError::Internal(format!(
            "bedrock converse error {status}: {text}"
        )));
    }
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| RuntimeError::Internal(format!("bedrock response parse: {e}")))?;
    Ok(parse_converse_response(&json))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_converse_request_with_system_messages_tools() {
        let messages = vec![serde_json::json!({"role": "user", "content": "hi"})];
        let tools = vec![serde_json::json!({
            "name": "get_time",
            "description": "Get the time",
            "input_schema": {"type": "object", "properties": {}}
        })];
        let req = build_converse_request("be brief", &messages, &tools, 0.3, 1024);

        assert_eq!(req["system"][0]["text"], "be brief");
        assert_eq!(req["messages"][0]["role"], "user");
        assert_eq!(req["messages"][0]["content"][0]["text"], "hi");
        let spec = &req["toolConfig"]["tools"][0]["toolSpec"];
        assert_eq!(spec["name"], "get_time");
        assert_eq!(spec["description"], "Get the time");
        assert_eq!(spec["inputSchema"]["json"]["type"], "object");
        assert_eq!(req["inferenceConfig"]["maxTokens"], 1024);
    }

    #[test]
    fn omits_tool_config_when_no_tools() {
        let messages = vec![serde_json::json!({"role": "user", "content": "hi"})];
        let req = build_converse_request("s", &messages, &[], 0.3, 512);
        assert!(req.get("toolConfig").is_none());
    }

    #[test]
    fn parses_text_and_tooluse_response() {
        let resp = serde_json::json!({
            "output": {"message": {"role": "assistant", "content": [
                {"text": "hello"},
                {"toolUse": {"toolUseId": "tu_1", "name": "get_time", "input": {"tz": "utc"}}}
            ]}},
            "stopReason": "tool_use"
        });
        let unified = parse_converse_response(&resp);
        assert_eq!(unified["stop_reason"], "tool_use");
        assert_eq!(unified["content"][0]["type"], "text");
        assert_eq!(unified["content"][0]["text"], "hello");
        assert_eq!(unified["content"][1]["type"], "tool_use");
        assert_eq!(unified["content"][1]["id"], "tu_1");
        assert_eq!(unified["content"][1]["name"], "get_time");
        assert_eq!(unified["content"][1]["input"]["tz"], "utc");
    }

    #[test]
    fn maps_end_turn_stop_reason() {
        let resp = serde_json::json!({
            "output": {"message": {"content": [{"text": "done"}]}},
            "stopReason": "end_turn"
        });
        let unified = parse_converse_response(&resp);
        assert_eq!(unified["stop_reason"], "end_turn");
        assert_eq!(unified["content"][0]["text"], "done");
    }

    #[test]
    fn sigv4_produces_authorization_header() {
        use aws_credential_types::Credentials;
        let creds = Credentials::new("AKIDEXAMPLE", "SECRETEXAMPLEKEY", None, None, "test");
        let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        let headers = sign_converse_request(
            "us-east-1",
            "model.id:0",
            &creds,
            br#"{"messages":[]}"#,
            when,
        )
        .expect("sign");
        let auth = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("authorization"));
        let auth = auth.expect("authorization header present").1.clone();
        assert!(auth.starts_with("AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/"));
        assert!(auth.contains("/us-east-1/bedrock/aws4_request"));
        assert!(auth.contains("Signature="));
    }
}
