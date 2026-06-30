//! LLM client for OpenAI-compatible chat completions
//!
//! Auto-detects provider from environment variables and provides a unified
//! interface for chat completion requests.

#[cfg(feature = "http-input")]
use crate::types::RuntimeError;

/// Supported LLM providers
#[cfg(feature = "http-input")]
#[derive(Debug, Clone)]
pub enum LlmProvider {
    OpenRouter,
    OpenAI,
    Anthropic,
    #[cfg(feature = "bedrock")]
    Bedrock,
}

#[cfg(feature = "http-input")]
impl std::fmt::Display for LlmProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmProvider::OpenRouter => write!(f, "OpenRouter"),
            LlmProvider::OpenAI => write!(f, "OpenAI"),
            LlmProvider::Anthropic => write!(f, "Anthropic"),
            #[cfg(feature = "bedrock")]
            LlmProvider::Bedrock => write!(f, "Bedrock"),
        }
    }
}

/// Returns OpenRouter app-attribution headers from env, if the operator opted in.
///
/// Reads `OPENROUTER_REFERER` and `OPENROUTER_TITLE`. Both are optional and
/// default to empty (no attribution sent), honoring zero-trust defaults.
/// See https://openrouter.ai/docs/app-attribution.
#[cfg(feature = "http-input")]
pub(crate) fn openrouter_attribution_headers() -> Vec<(&'static str, String)> {
    let mut headers = Vec::new();
    if let Ok(v) = std::env::var("OPENROUTER_REFERER") {
        if !v.is_empty() {
            headers.push(("HTTP-Referer", v));
        }
    }
    if let Ok(v) = std::env::var("OPENROUTER_TITLE") {
        if !v.is_empty() {
            headers.push(("X-Title", v));
        }
    }
    headers
}

/// OpenAI-compatible chat completions client
#[cfg(feature = "http-input")]
pub struct LlmClient {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    model: String,
    provider: LlmProvider,
    #[cfg(feature = "bedrock")]
    region: String,
    // Populated by `from_env_or_secrets`; used by the SigV4 signing path in Task 4.
    #[cfg(feature = "bedrock")]
    #[allow(dead_code)]
    credentials: Option<aws_credential_types::provider::SharedCredentialsProvider>,
}

#[cfg(feature = "http-input")]
impl LlmClient {
    /// Auto-detect LLM provider from environment variables.
    ///
    /// Checks in order:
    /// 1. `OPENROUTER_API_KEY` → OpenRouter (model from `OPENROUTER_MODEL`)
    /// 2. `OPENAI_API_KEY` → OpenAI (model from `CHAT_MODEL`)
    /// 3. `ANTHROPIC_API_KEY` → Anthropic (model from `ANTHROPIC_MODEL`)
    ///
    /// Returns `None` if no API key is found.
    pub fn from_env() -> Option<Self> {
        // LLM providers legitimately redirect (e.g. `/v1` → `/v1/`) and the
        // LLM hostname comes from env, so we explicitly keep redirect
        // following within reason — but force DNS through the SSRF-safe
        // resolver so a malicious DNS response can't point us at an
        // internal IP for the legitimate hostname.
        let client = crate::net_guard::customise_ssrf_safe_client(
            std::time::Duration::from_secs(120),
            |b| b.redirect(reqwest::redirect::Policy::limited(2)),
        )
        .ok()?;

        // Validate a base URL against the SSRF guard before the API key is
        // ever sent to it. Rejects private IPs, loopback, cloud metadata,
        // non-http(s) schemes, and obfuscated IPv4 literals.
        fn validate_base_url(env_var: &str, url: &str) -> bool {
            if let Err(reason) = crate::net_guard::reject_ssrf_url(url) {
                tracing::error!(
                    "Refusing LLM base URL from {}: {} — falling back or disabling provider",
                    env_var,
                    reason
                );
                return false;
            }
            if url.starts_with("http://") {
                tracing::warn!(
                    "LLM base URL from {} uses plaintext HTTP ({}); \
                     API keys will be sent in the clear. Configure HTTPS in production.",
                    env_var,
                    url
                );
            }
            true
        }

        if let Ok(api_key) = std::env::var("OPENROUTER_API_KEY") {
            let model = std::env::var("OPENROUTER_MODEL")
                .unwrap_or_else(|_| "anthropic/claude-sonnet-4".to_string());
            let base_url = std::env::var("OPENROUTER_BASE_URL")
                .unwrap_or_else(|_| "https://openrouter.ai/api/v1".to_string());
            if !validate_base_url("OPENROUTER_BASE_URL", &base_url) {
                return None;
            }
            tracing::info!(
                "LLM client initialized: provider=OpenRouter model={}",
                model
            );
            return Some(Self {
                client,
                api_key,
                base_url,
                model,
                provider: LlmProvider::OpenRouter,
                #[cfg(feature = "bedrock")]
                region: String::new(),
                #[cfg(feature = "bedrock")]
                credentials: None,
            });
        }

        if let Ok(api_key) = std::env::var("OPENAI_API_KEY") {
            let model = std::env::var("CHAT_MODEL").unwrap_or_else(|_| "gpt-4o".to_string());
            let base_url = std::env::var("OPENAI_BASE_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
            if !validate_base_url("OPENAI_BASE_URL", &base_url) {
                return None;
            }
            tracing::info!("LLM client initialized: provider=OpenAI model={}", model);
            return Some(Self {
                client,
                api_key,
                base_url,
                model,
                provider: LlmProvider::OpenAI,
                #[cfg(feature = "bedrock")]
                region: String::new(),
                #[cfg(feature = "bedrock")]
                credentials: None,
            });
        }

        if let Ok(api_key) = std::env::var("ANTHROPIC_API_KEY") {
            let model = std::env::var("ANTHROPIC_MODEL")
                .unwrap_or_else(|_| "claude-sonnet-4-20250514".to_string());
            let base_url = std::env::var("ANTHROPIC_BASE_URL")
                .unwrap_or_else(|_| "https://api.anthropic.com/v1".to_string());
            if !validate_base_url("ANTHROPIC_BASE_URL", &base_url) {
                return None;
            }
            tracing::info!("LLM client initialized: provider=Anthropic model={}", model);
            return Some(Self {
                client,
                api_key,
                base_url,
                model,
                provider: LlmProvider::Anthropic,
                #[cfg(feature = "bedrock")]
                region: String::new(),
                #[cfg(feature = "bedrock")]
                credentials: None,
            });
        }

        // Bedrock: no API key — uses AWS credential chain instead.
        // `from_env` is sync so credentials are resolved lazily at call time
        // (Task 4 builds the credential chain inside the async dispatch method).
        #[cfg(feature = "bedrock")]
        if let Ok(model) = std::env::var("BEDROCK_MODEL_ID") {
            let region = std::env::var("AWS_REGION")
                .or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
                .ok();
            if let Some(region) = region {
                tracing::info!(
                    "LLM client initialized: provider=Bedrock model={model} region={region}"
                );
                return Some(Self {
                    client,
                    api_key: String::new(),
                    base_url: String::new(),
                    model,
                    provider: LlmProvider::Bedrock,
                    region,
                    // `DefaultCredentialsChain::builder().build()` is async;
                    // stored as None here and resolved at call time in Task 4.
                    credentials: None,
                });
            } else {
                tracing::warn!(
                    "BEDROCK_MODEL_ID set but no AWS_REGION/AWS_DEFAULT_REGION — skipping Bedrock"
                );
            }
        }

        tracing::info!("No LLM API key found in environment, LLM invocation disabled");
        None
    }

    /// Like `from_env`, but also accepts an optional `SecretStore` from which
    /// API keys can be resolved when the corresponding `*_API_KEY_REF`
    /// environment variable is set.
    ///
    /// Operators point at vault/openbao keys with these env vars:
    /// * `OPENROUTER_API_KEY_REF` — secret-store key for the OpenRouter API key
    /// * `OPENAI_API_KEY_REF` — secret-store key for the OpenAI API key
    /// * `ANTHROPIC_API_KEY_REF` — secret-store key for the Anthropic API key
    ///
    /// When a `*_REF` env var is set and `store` is `Some`, the value is
    /// fetched from the store. On store miss / failure, it falls back to the
    /// regular `*_API_KEY` env var. When `*_REF` is unset, the regular env
    /// var is used directly. The provider-detection order matches `from_env`:
    /// OpenRouter → OpenAI → Anthropic.
    ///
    /// The `*_BASE_URL` and `*_MODEL` env vars are unchanged from `from_env`.
    pub async fn from_env_or_secrets(
        store: Option<std::sync::Arc<dyn crate::secrets::SecretStore + Send + Sync>>,
    ) -> Option<Self> {
        let client = crate::net_guard::customise_ssrf_safe_client(
            std::time::Duration::from_secs(120),
            |b| b.redirect(reqwest::redirect::Policy::limited(2)),
        )
        .ok()?;

        fn validate_base_url(env_var: &str, url: &str) -> bool {
            if let Err(reason) = crate::net_guard::reject_ssrf_url(url) {
                tracing::error!(
                    "Refusing LLM base URL from {}: {} — falling back or disabling provider",
                    env_var,
                    reason
                );
                return false;
            }
            if url.starts_with("http://") {
                tracing::warn!(
                    "LLM base URL from {} uses plaintext HTTP ({}); \
                     API keys will be sent in the clear. Configure HTTPS in production.",
                    env_var,
                    url
                );
            }
            true
        }

        let store_ref: Option<&(dyn crate::secrets::SecretStore + Send + Sync)> = store.as_deref();

        // OpenRouter
        let openrouter_ref = std::env::var("OPENROUTER_API_KEY_REF").ok();
        if let Some(api_key) = crate::secrets::resolve_secret_or_env(
            "OPENROUTER_API_KEY",
            openrouter_ref.as_deref(),
            store_ref,
        )
        .await
        {
            let model = std::env::var("OPENROUTER_MODEL")
                .unwrap_or_else(|_| "anthropic/claude-sonnet-4".to_string());
            let base_url = std::env::var("OPENROUTER_BASE_URL")
                .unwrap_or_else(|_| "https://openrouter.ai/api/v1".to_string());
            if !validate_base_url("OPENROUTER_BASE_URL", &base_url) {
                return None;
            }
            tracing::info!(
                "LLM client initialized: provider=OpenRouter model={} (key source: {})",
                model,
                if openrouter_ref.is_some() && store_ref.is_some() {
                    "secret store (with env fallback)"
                } else {
                    "env"
                }
            );
            return Some(Self {
                client,
                api_key,
                base_url,
                model,
                provider: LlmProvider::OpenRouter,
                #[cfg(feature = "bedrock")]
                region: String::new(),
                #[cfg(feature = "bedrock")]
                credentials: None,
            });
        }

        // OpenAI
        let openai_ref = std::env::var("OPENAI_API_KEY_REF").ok();
        if let Some(api_key) = crate::secrets::resolve_secret_or_env(
            "OPENAI_API_KEY",
            openai_ref.as_deref(),
            store_ref,
        )
        .await
        {
            let model = std::env::var("CHAT_MODEL").unwrap_or_else(|_| "gpt-4o".to_string());
            let base_url = std::env::var("OPENAI_BASE_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
            if !validate_base_url("OPENAI_BASE_URL", &base_url) {
                return None;
            }
            tracing::info!(
                "LLM client initialized: provider=OpenAI model={} (key source: {})",
                model,
                if openai_ref.is_some() && store_ref.is_some() {
                    "secret store (with env fallback)"
                } else {
                    "env"
                }
            );
            return Some(Self {
                client,
                api_key,
                base_url,
                model,
                provider: LlmProvider::OpenAI,
                #[cfg(feature = "bedrock")]
                region: String::new(),
                #[cfg(feature = "bedrock")]
                credentials: None,
            });
        }

        // Anthropic
        let anthropic_ref = std::env::var("ANTHROPIC_API_KEY_REF").ok();
        if let Some(api_key) = crate::secrets::resolve_secret_or_env(
            "ANTHROPIC_API_KEY",
            anthropic_ref.as_deref(),
            store_ref,
        )
        .await
        {
            let model = std::env::var("ANTHROPIC_MODEL")
                .unwrap_or_else(|_| "claude-sonnet-4-20250514".to_string());
            let base_url = std::env::var("ANTHROPIC_BASE_URL")
                .unwrap_or_else(|_| "https://api.anthropic.com/v1".to_string());
            if !validate_base_url("ANTHROPIC_BASE_URL", &base_url) {
                return None;
            }
            tracing::info!(
                "LLM client initialized: provider=Anthropic model={} (key source: {})",
                model,
                if anthropic_ref.is_some() && store_ref.is_some() {
                    "secret store (with env fallback)"
                } else {
                    "env"
                }
            );
            return Some(Self {
                client,
                api_key,
                base_url,
                model,
                provider: LlmProvider::Anthropic,
                #[cfg(feature = "bedrock")]
                region: String::new(),
                #[cfg(feature = "bedrock")]
                credentials: None,
            });
        }

        // Bedrock: uses AWS credential chain, not a secret-store API key.
        // `from_env_or_secrets` is async so we can build the credentials chain here.
        #[cfg(feature = "bedrock")]
        if let Ok(model) = std::env::var("BEDROCK_MODEL_ID") {
            let region = std::env::var("AWS_REGION")
                .or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
                .ok();
            if let Some(region) = region {
                let provider =
                    aws_config::default_provider::credentials::DefaultCredentialsChain::builder()
                        .region(aws_config::Region::new(region.clone()))
                        .build()
                        .await;
                let credentials =
                    Some(aws_credential_types::provider::SharedCredentialsProvider::new(provider));
                tracing::info!(
                    "LLM client initialized: provider=Bedrock model={model} region={region}"
                );
                return Some(Self {
                    client,
                    api_key: String::new(),
                    base_url: String::new(),
                    model,
                    provider: LlmProvider::Bedrock,
                    region,
                    credentials,
                });
            } else {
                tracing::warn!(
                    "BEDROCK_MODEL_ID set but no AWS_REGION/AWS_DEFAULT_REGION — skipping Bedrock"
                );
            }
        }

        tracing::info!(
            "No LLM API key found in environment or secret store, LLM invocation disabled"
        );
        None
    }

    /// Get the model name
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Get the provider
    pub fn provider(&self) -> &LlmProvider {
        &self.provider
    }

    /// Get the AWS region (only populated for the Bedrock provider).
    // Used in `from_env_selects_bedrock_when_model_id_set` test and Task 4 signing.
    #[cfg(feature = "bedrock")]
    #[allow(dead_code)]
    pub(crate) fn region(&self) -> &str {
        &self.region
    }

    /// Get the configured API base URL.
    ///
    /// Crate-private and gated on `cloud-llm`: only the cloud inference
    /// provider needs this to make HTTP requests directly without re-reading
    /// the env var on every call. External callers should keep using the
    /// public `chat_completion` methods, which never expose the URL.
    #[cfg(feature = "cloud-llm")]
    pub(crate) fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Get the configured API key.
    ///
    /// Crate-private and gated on `cloud-llm`: only the cloud inference
    /// provider needs raw access to the key in order to construct
    /// provider-specific HTTP headers. Public callers must go through
    /// `chat_completion`, which never exposes the key. The value never
    /// appears in `Debug` output.
    #[cfg(feature = "cloud-llm")]
    pub(crate) fn api_key(&self) -> &str {
        &self.api_key
    }

    /// Send a chat completion request with system and user messages.
    pub async fn chat_completion(&self, system: &str, user: &str) -> Result<String, RuntimeError> {
        match self.provider {
            #[cfg(feature = "bedrock")]
            LlmProvider::Bedrock => {
                let messages = vec![serde_json::json!({"role": "user", "content": user})];
                let resp = self
                    .bedrock_converse_with_tools(system, &messages, &[])
                    .await?;
                Ok(resp["content"]
                    .as_array()
                    .and_then(|b| {
                        b.iter()
                            .find_map(|x| x.get("text").and_then(|t| t.as_str()))
                    })
                    .unwrap_or("")
                    .to_string())
            }
            LlmProvider::Anthropic => self.anthropic_completion(system, user).await,
            _ => self.openai_completion(system, user).await,
        }
    }

    /// Send a chat completion with tool definitions. Returns a normalized response:
    /// `{ "content": [...], "stop_reason": "end_turn"|"tool_use" }`
    /// Content blocks are `{"type":"text","text":"..."}` or
    /// `{"type":"tool_use","id":"...","name":"...","input":{...}}`
    ///
    /// Works with Anthropic (native tool_use), OpenAI/OpenRouter (function calling
    /// converted to the same normalized format).
    pub async fn chat_with_tools(
        &self,
        system: &str,
        messages: &[serde_json::Value],
        tools: &[serde_json::Value],
    ) -> Result<serde_json::Value, RuntimeError> {
        match self.provider {
            #[cfg(feature = "bedrock")]
            LlmProvider::Bedrock => {
                self.bedrock_converse_with_tools(system, messages, tools)
                    .await
            }
            LlmProvider::Anthropic => {
                self.anthropic_completion_with_tools(system, messages, tools)
                    .await
            }
            _ => {
                self.openai_completion_with_tools(system, messages, tools)
                    .await
            }
        }
    }

    /// SigV4-signed Bedrock Converse call with explicit generation parameters.
    ///
    /// `from_env` is sync and stores `credentials: None`, so when the provider
    /// was not pre-resolved we build the default AWS credentials chain on demand
    /// here rather than erroring out.
    #[cfg(feature = "bedrock")]
    pub(crate) async fn bedrock_converse(
        &self,
        system: &str,
        messages: &[serde_json::Value],
        tools: &[serde_json::Value],
        temperature: f32,
        max_tokens: u32,
    ) -> Result<serde_json::Value, RuntimeError> {
        use aws_credential_types::provider::SharedCredentialsProvider;

        let creds: SharedCredentialsProvider = match &self.credentials {
            Some(c) => c.clone(),
            None => {
                let chain =
                    aws_config::default_provider::credentials::DefaultCredentialsChain::builder()
                        .region(aws_config::Region::new(self.region.clone()))
                        .build()
                        .await;
                SharedCredentialsProvider::new(chain)
            }
        };

        let body = crate::http_input::bedrock::build_converse_request(
            system,
            messages,
            tools,
            temperature,
            max_tokens,
        );
        crate::http_input::bedrock::converse(&self.client, &self.region, &self.model, &creds, &body)
            .await
    }

    /// SigV4-signed Bedrock Converse call using default generation parameters.
    ///
    /// Delegates to [`bedrock_converse`] with the fixed defaults used by the
    /// http_input / `chat_with_tools` path (temperature=0.3, max_tokens=4096).
    #[cfg(feature = "bedrock")]
    async fn bedrock_converse_with_tools(
        &self,
        system: &str,
        messages: &[serde_json::Value],
        tools: &[serde_json::Value],
    ) -> Result<serde_json::Value, RuntimeError> {
        self.bedrock_converse(system, messages, tools, 0.3, 4096)
            .await
    }

    /// Convert Anthropic-format tool definitions to OpenAI function-calling format.
    fn tools_to_openai_functions(tools: &[serde_json::Value]) -> Vec<serde_json::Value> {
        tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.get("name").and_then(|n| n.as_str()).unwrap_or("unknown"),
                        "description": t.get("description").and_then(|d| d.as_str()).unwrap_or(""),
                        "parameters": t.get("input_schema").cloned().unwrap_or(serde_json::json!({"type": "object", "properties": {}}))
                    }
                })
            })
            .collect()
    }

    /// Convert OpenAI messages format to include system message
    fn build_openai_messages(
        system: &str,
        messages: &[serde_json::Value],
    ) -> Vec<serde_json::Value> {
        let mut result = vec![serde_json::json!({"role": "system", "content": system})];
        for msg in messages {
            result.push(msg.clone());
        }
        result
    }

    /// OpenAI/OpenRouter completion with function calling, normalized to Anthropic format
    async fn openai_completion_with_tools(
        &self,
        system: &str,
        messages: &[serde_json::Value],
        tools: &[serde_json::Value],
    ) -> Result<serde_json::Value, RuntimeError> {
        let openai_messages = Self::build_openai_messages(system, messages);
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": openai_messages,
            "max_tokens": 4096,
            "temperature": 0.3
        });
        if !tools.is_empty() {
            body["tools"] = serde_json::Value::Array(Self::tools_to_openai_functions(tools));
        }

        let mut req = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json");
        if matches!(self.provider, LlmProvider::OpenRouter) {
            for (k, v) in openrouter_attribution_headers() {
                req = req.header(k, v);
            }
        }
        let response = req
            .json(&body)
            .send()
            .await
            .map_err(|e| RuntimeError::Internal(format!("LLM request failed: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(RuntimeError::Internal(format!(
                "LLM API error ({}): {}",
                status, error_text
            )));
        }

        let resp: serde_json::Value = response
            .json()
            .await
            .map_err(|e| RuntimeError::Internal(format!("Failed to parse response: {}", e)))?;

        if let Some(usage) = resp.get("usage") {
            tracing::info!(
                "LLM usage: provider={} model={} prompt_tokens={} completion_tokens={}",
                self.provider,
                self.model,
                usage
                    .get("prompt_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                usage
                    .get("completion_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
            );
        }

        // Normalize OpenAI response to Anthropic-like format
        let choice = resp
            .get("choices")
            .and_then(|c| c.get(0))
            .ok_or_else(|| RuntimeError::Internal("No choices in response".to_string()))?;

        let finish_reason = choice
            .get("finish_reason")
            .and_then(|f| f.as_str())
            .unwrap_or("stop");

        let message = choice
            .get("message")
            .ok_or_else(|| RuntimeError::Internal("No message in choice".to_string()))?;

        let mut content_blocks = Vec::new();

        // Add text content if present
        if let Some(text) = message.get("content").and_then(|c| c.as_str()) {
            if !text.is_empty() {
                content_blocks.push(serde_json::json!({"type": "text", "text": text}));
            }
        }

        // Convert function/tool calls to Anthropic tool_use format
        if let Some(tool_calls) = message.get("tool_calls").and_then(|t| t.as_array()) {
            for tc in tool_calls {
                let id = tc.get("id").and_then(|i| i.as_str()).unwrap_or("unknown");
                let func = tc.get("function").unwrap_or(tc);
                let name = func
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("unknown");
                let args_str = func
                    .get("arguments")
                    .and_then(|a| a.as_str())
                    .unwrap_or("{}");
                let args: serde_json::Value =
                    serde_json::from_str(args_str).unwrap_or(serde_json::json!({}));
                content_blocks.push(serde_json::json!({
                    "type": "tool_use",
                    "id": id,
                    "name": name,
                    "input": args
                }));
            }
        }

        let stop_reason = if finish_reason == "tool_calls" || finish_reason == "function_call" {
            "tool_use"
        } else {
            "end_turn"
        };

        Ok(serde_json::json!({
            "content": content_blocks,
            "stop_reason": stop_reason
        }))
    }

    /// Anthropic Messages API with tool definitions
    async fn anthropic_completion_with_tools(
        &self,
        system: &str,
        messages: &[serde_json::Value],
        tools: &[serde_json::Value],
    ) -> Result<serde_json::Value, RuntimeError> {
        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": 4096,
            "system": system,
            "messages": messages
        });
        if !tools.is_empty() {
            body["tools"] = serde_json::Value::Array(tools.to_vec());
        }

        let response = self
            .client
            .post(format!("{}/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| RuntimeError::Internal(format!("Anthropic request failed: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(RuntimeError::Internal(format!(
                "Anthropic API error ({}): {}",
                status, error_text
            )));
        }

        let resp_json: serde_json::Value = response.json().await.map_err(|e| {
            RuntimeError::Internal(format!("Failed to parse Anthropic response: {}", e))
        })?;

        if let Some(usage) = resp_json.get("usage") {
            tracing::info!(
                "LLM usage: provider=Anthropic model={} input_tokens={} output_tokens={}",
                self.model,
                usage
                    .get("input_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                usage
                    .get("output_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
            );
        }

        Ok(resp_json)
    }

    /// OpenAI-compatible chat completion (works for OpenRouter and OpenAI)
    async fn openai_completion(&self, system: &str, user: &str) -> Result<String, RuntimeError> {
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user }
            ],
            "max_tokens": 4096,
            "temperature": 0.3
        });

        let start = std::time::Instant::now();

        let mut req = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json");
        if matches!(self.provider, LlmProvider::OpenRouter) {
            for (k, v) in openrouter_attribution_headers() {
                req = req.header(k, v);
            }
        }
        let response = req
            .json(&body)
            .send()
            .await
            .map_err(|e| RuntimeError::Internal(format!("LLM request failed: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(RuntimeError::Internal(format!(
                "LLM API error ({}): {}",
                status, error_text
            )));
        }

        let resp_json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| RuntimeError::Internal(format!("Failed to parse LLM response: {}", e)))?;

        let latency = start.elapsed();

        // Log usage if available
        if let Some(usage) = resp_json.get("usage") {
            tracing::info!(
                "LLM usage: provider={} model={} prompt_tokens={} completion_tokens={} total_tokens={} latency={:?}",
                self.provider,
                self.model,
                usage.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                usage.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                usage.get("total_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                latency,
            );
        }

        resp_json
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| RuntimeError::Internal("No content in LLM response choices".to_string()))
    }

    /// Anthropic Messages API completion
    async fn anthropic_completion(&self, system: &str, user: &str) -> Result<String, RuntimeError> {
        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": 4096,
            "system": system,
            "messages": [
                { "role": "user", "content": user }
            ]
        });

        let start = std::time::Instant::now();

        let response = self
            .client
            .post(format!("{}/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| RuntimeError::Internal(format!("Anthropic request failed: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(RuntimeError::Internal(format!(
                "Anthropic API error ({}): {}",
                status, error_text
            )));
        }

        let resp_json: serde_json::Value = response.json().await.map_err(|e| {
            RuntimeError::Internal(format!("Failed to parse Anthropic response: {}", e))
        })?;

        let latency = start.elapsed();

        // Log usage
        if let Some(usage) = resp_json.get("usage") {
            tracing::info!(
                "LLM usage: provider=Anthropic model={} input_tokens={} output_tokens={} latency={:?}",
                self.model,
                usage.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                usage.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                latency,
            );
        }

        // Anthropic returns content as array of content blocks
        resp_json
            .get("content")
            .and_then(|c| c.as_array())
            .and_then(|blocks| {
                blocks
                    .iter()
                    .find(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
            })
            .and_then(|b| b.get("text"))
            .and_then(|t| t.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| {
                RuntimeError::Internal("No text content in Anthropic response".to_string())
            })
    }
}

#[cfg(all(test, feature = "http-input"))]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    fn test_provider_display() {
        assert_eq!(format!("{}", LlmProvider::OpenRouter), "OpenRouter");
        assert_eq!(format!("{}", LlmProvider::OpenAI), "OpenAI");
        assert_eq!(format!("{}", LlmProvider::Anthropic), "Anthropic");
    }

    #[serial]
    #[test]
    fn test_from_env_no_keys() {
        // Remove any existing keys for the test
        std::env::remove_var("OPENROUTER_API_KEY");
        std::env::remove_var("OPENAI_API_KEY");
        std::env::remove_var("ANTHROPIC_API_KEY");
        #[cfg(feature = "bedrock")]
        std::env::remove_var("BEDROCK_MODEL_ID");

        let client = LlmClient::from_env();
        assert!(client.is_none());
    }

    #[cfg(feature = "bedrock")]
    #[serial]
    #[test]
    fn from_env_selects_bedrock_when_model_id_set() {
        std::env::set_var(
            "BEDROCK_MODEL_ID",
            "anthropic.claude-3-5-sonnet-20241022-v2:0",
        );
        std::env::set_var("AWS_REGION", "us-east-1");
        // Ensure higher-priority providers don't preempt:
        for k in ["OPENROUTER_API_KEY", "OPENAI_API_KEY", "ANTHROPIC_API_KEY"] {
            std::env::remove_var(k);
        }
        let client = LlmClient::from_env().expect("bedrock client");
        assert!(matches!(client.provider(), LlmProvider::Bedrock));
        assert_eq!(client.region(), "us-east-1");
        std::env::remove_var("BEDROCK_MODEL_ID");
        std::env::remove_var("AWS_REGION");
    }

    #[test]
    fn test_tools_to_openai_functions() {
        let tools = vec![serde_json::json!({
            "name": "nmap_scan",
            "description": "Run an nmap scan",
            "input_schema": {
                "type": "object",
                "properties": {
                    "target": { "type": "string" }
                },
                "required": ["target"]
            }
        })];

        let funcs = LlmClient::tools_to_openai_functions(&tools);
        assert_eq!(funcs.len(), 1);
        let f = &funcs[0];
        assert_eq!(f["type"], "function");
        assert_eq!(f["function"]["name"], "nmap_scan");
        assert_eq!(f["function"]["description"], "Run an nmap scan");
        assert_eq!(f["function"]["parameters"]["type"], "object");
        assert!(f["function"]["parameters"]["properties"]["target"].is_object());
    }

    #[test]
    fn test_tools_to_openai_functions_missing_fields() {
        let tools = vec![serde_json::json!({})];
        let funcs = LlmClient::tools_to_openai_functions(&tools);
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0]["function"]["name"], "unknown");
        assert_eq!(funcs[0]["function"]["description"], "");
    }

    #[test]
    fn test_build_openai_messages() {
        let messages = vec![
            serde_json::json!({"role": "user", "content": "hello"}),
            serde_json::json!({"role": "assistant", "content": "hi"}),
        ];
        let result = LlmClient::build_openai_messages("system prompt", &messages);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0]["role"], "system");
        assert_eq!(result[0]["content"], "system prompt");
        assert_eq!(result[1]["role"], "user");
        assert_eq!(result[2]["role"], "assistant");
    }
}
