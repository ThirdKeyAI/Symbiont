//! Registered chat responses with policy-bound platform delivery.
use async_trait::async_trait;
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc};
use symbi_channel_adapter::{
    manager::build_platform_response, AgentInvoker, ChannelAdapter, InboundMessage, OutboundMessage,
};
use symbi_runtime::{
    integrations::mcp::project::ProjectReader,
    reasoning::{
        inference::InferenceProvider,
        policy_bridge::ReasoningPolicyGate,
        response_delivery::{DeliveryReceipt, ResponseDestination},
        response_run::{run_response, run_response_to, ResponseRequest},
    },
};

pub(super) struct LlmAgentInvoker {
    project: PathBuf,
    provider: Option<Arc<dyn InferenceProvider>>,
    sources: Vec<(String, String)>,
    gate: Arc<dyn ReasoningPolicyGate>,
}

impl LlmAgentInvoker {
    pub(super) fn new(
        project: PathBuf,
        provider: Option<Arc<dyn InferenceProvider>>,
        gate: Arc<dyn ReasoningPolicyGate>,
    ) -> Result<Self, String> {
        let reader = ProjectReader::open(&project)?;
        let mut sources = Vec::new();
        let mut bytes = 0;
        if let Ok(entries) = std::fs::read_dir(reader.path().join("agents")) {
            for entry in entries.take(1024).flatten() {
                let path = std::path::Path::new("agents").join(entry.file_name());
                if dsl::is_symbi_file(&path) {
                    if let Ok(source) = reader.read_text(&path, 1024 * 1024) {
                        bytes += source.len();
                        if bytes > 16 * 1024 * 1024 {
                            return Err("chat source registry exceeds 16 MiB".into());
                        }
                        sources.push((entry.file_name().to_string_lossy().into_owned(), source));
                    }
                }
            }
        }
        Ok(Self {
            project: reader.path().into(),
            provider,
            sources,
            gate,
        })
    }

    fn selected_agent(&self, name: &str) -> Result<dsl::ConversationalAgent, String> {
        let mut candidates = Vec::new();
        for (filename, source) in &self.sources {
            let settings = dsl::resolve_execution_settings(source, name);
            if dsl::strip_symbi_extension(filename) == Some(name)
                || settings
                    .as_ref()
                    .is_ok_and(|settings| settings.agent_name == name)
            {
                candidates.push((source, settings));
            }
        }
        if candidates.len() != 1 {
            return Err("chat agent name must identify one registered source".into());
        }
        let (source, settings) = candidates.pop().unwrap();
        dsl::ConversationalAgent::parse(source, &settings?.agent_name)
    }

    fn request(&self, name: &str, input: &str) -> Result<ResponseRequest, String> {
        let agent = self.selected_agent(name)?;
        Ok(ResponseRequest {
            project: self.project.clone(),
            agent,
            surface: "chat-platform".into(),
            input: input.into(),
            caller_instructions: None,
            provider: self.provider.clone().ok_or("no LLM provider configured")?,
            gate: self.gate.clone(),
            cancellation: Default::default(),
        })
    }
}

#[async_trait]
impl AgentInvoker for LlmAgentInvoker {
    async fn invoke(&self, name: &str, input: &str) -> Result<String, String> {
        run_response(self.request(name, input)?).await.result
    }

    async fn invoke_and_deliver(
        &self,
        name: &str,
        message: &InboundMessage,
        adapter: Arc<dyn ChannelAdapter>,
    ) -> Result<(), String> {
        let request = self.request(name, &message.content)?;
        let destination = Arc::new(ChatDestination {
            agent_name: request.agent.settings().agent_name.clone(),
            message: message.clone(),
            adapter,
        });
        let outcome = run_response_to(request, destination).await;
        tracing::info!(agent_id = %outcome.agent_id, audit = ?outcome.audit,
            completed = outcome.result.is_ok(), "Chat response run finished");
        outcome.result.map(|_| ())
    }
}

struct ChatDestination {
    agent_name: String,
    message: InboundMessage,
    adapter: Arc<dyn ChannelAdapter>,
}

#[async_trait]
impl ResponseDestination for ChatDestination {
    fn context(&self) -> Value {
        json!({"platform":self.message.platform, "workspace_id":self.message.workspace_id,
            "channel_id":self.message.channel_id, "thread_id":self.message.thread_id,
            "sender_id":self.message.sender_id, "message_id":self.message.id})
    }
    fn validate(&self) -> Result<(), String> {
        if self.adapter.platform() != self.message.platform
            || self.message.channel_id.is_empty()
            || self.message.sender_id.is_empty()
        {
            return Err("chat response destination is missing or mismatched".into());
        }
        Ok(())
    }
    fn prepare(&self, content: &str) -> Result<Value, String> {
        self.validate()?;
        let message = build_platform_response(&self.message, content, &self.agent_name);
        let transport = self
            .adapter
            .prepare_response(&message)
            .map_err(|error| error.to_string())?;
        Ok(json!({"destination":self.context(),
            "message":message, "transport":transport}))
    }
    async fn send(&self, request: &Value) -> Result<DeliveryReceipt, String> {
        if request.get("destination") != Some(&self.context()) {
            return Err("prepared chat destination changed".into());
        }
        let message: OutboundMessage = serde_json::from_value(request["message"].clone())
            .map_err(|error| error.to_string())?;
        if message.channel_id != self.message.channel_id
            || message.thread_id != self.message.thread_id
        {
            return Err("prepared chat routing changed".into());
        }
        let transport = self
            .adapter
            .prepare_response(&message)
            .map_err(|error| error.to_string())?;
        if request.get("transport") != Some(&transport) {
            return Err("prepared chat transport changed".into());
        }
        let receipt = self
            .adapter
            .send_response(message)
            .await
            .map_err(|error| error.to_string())?;
        let confirmed = receipt.success
            && receipt.platform == self.message.platform
            && receipt.channel_id == self.message.channel_id
            && receipt.message_ts.as_ref().is_some_and(|id| !id.is_empty());
        Ok(DeliveryReceipt {
            receipt: json!(receipt),
            confirmed,
        })
    }
}

#[cfg(test)]
mod tests;
