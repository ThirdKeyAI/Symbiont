//! Agent registry for multi-agent composition
//!
//! Maps agent names to their configurations and IDs, enabling
//! runtime agent spawning and lifecycle management.

use crate::reasoning::inference::InferenceProvider;
use crate::types::AgentId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Configuration for a registered agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisteredAgent {
    /// Unique identifier.
    pub agent_id: AgentId,
    /// Human-readable name.
    pub name: String,
    /// System prompt for this agent.
    pub system_prompt: String,
    /// Tool names this agent has access to.
    pub tools: Vec<String>,
    /// Optional response format (e.g., "json", "text").
    pub response_format: Option<String>,
    /// When this agent was registered.
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Thread-safe registry of named agents.
#[derive(Clone)]
pub struct AgentRegistry {
    agents: Arc<RwLock<HashMap<String, RegisteredAgent>>>,
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            agents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Spawn (register) a new agent.
    pub async fn spawn_agent(
        &self,
        name: impl Into<String>,
        system_prompt: impl Into<String>,
        tools: Vec<String>,
        response_format: Option<String>,
    ) -> AgentId {
        let name = name.into();
        let agent_id = AgentId::new();

        let agent = RegisteredAgent {
            agent_id,
            name: name.clone(),
            system_prompt: system_prompt.into(),
            tools,
            response_format,
            created_at: chrono::Utc::now(),
        };

        self.agents.write().await.insert(name, agent);
        agent_id
    }

    /// Get a registered agent by name.
    pub async fn get_agent(&self, name: &str) -> Option<RegisteredAgent> {
        self.agents.read().await.get(name).cloned()
    }

    /// List all registered agents.
    pub async fn list_agents(&self) -> Vec<RegisteredAgent> {
        self.agents.read().await.values().cloned().collect()
    }

    /// Remove an agent by name.
    pub async fn remove_agent(&self, name: &str) -> bool {
        self.agents.write().await.remove(name).is_some()
    }

    /// Check if an agent exists.
    pub async fn has_agent(&self, name: &str) -> bool {
        self.agents.read().await.contains_key(name)
    }

    /// Send a message to an agent and get a response.
    ///
    /// Uses the agent's system prompt and the provided inference provider
    /// to run a single-turn conversation.
    pub async fn ask_agent(
        &self,
        name: &str,
        message: &str,
        provider: &dyn InferenceProvider,
    ) -> Result<String, AgentRegistryError> {
        let agent = self
            .get_agent(name)
            .await
            .ok_or_else(|| AgentRegistryError::NotFound {
                name: name.to_string(),
            })?;

        use crate::reasoning::conversation::{Conversation, ConversationMessage};
        use crate::reasoning::inference::InferenceOptions;

        let mut conv = Conversation::with_system(&agent.system_prompt);
        conv.push(ConversationMessage::user(message));

        let options = InferenceOptions::default();
        let response = provider.complete(&conv, &options).await.map_err(|e| {
            AgentRegistryError::InferenceError {
                agent_name: name.to_string(),
                message: e.to_string(),
            }
        })?;

        Ok(response.content)
    }
}

/// Errors from the agent registry.
#[derive(Debug, thiserror::Error)]
pub enum AgentRegistryError {
    #[error("Agent '{name}' not found")]
    NotFound { name: String },

    #[error("Inference error for agent '{agent_name}': {message}")]
    InferenceError { agent_name: String, message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_spawn_and_get_agent() {
        let registry = AgentRegistry::new();

        let id = registry
            .spawn_agent(
                "researcher",
                "You are a researcher.",
                vec!["search".into()],
                None,
            )
            .await;

        let agent = registry.get_agent("researcher").await.unwrap();
        assert_eq!(agent.agent_id, id);
        assert_eq!(agent.name, "researcher");
        assert_eq!(agent.tools, vec!["search"]);
    }

    #[tokio::test]
    async fn test_list_agents() {
        let registry = AgentRegistry::new();

        registry.spawn_agent("a", "Agent A", vec![], None).await;
        registry.spawn_agent("b", "Agent B", vec![], None).await;

        let agents = registry.list_agents().await;
        assert_eq!(agents.len(), 2);
    }

    #[tokio::test]
    async fn test_remove_agent() {
        let registry = AgentRegistry::new();

        registry
            .spawn_agent("temp", "Temporary", vec![], None)
            .await;
        assert!(registry.has_agent("temp").await);

        assert!(registry.remove_agent("temp").await);
        assert!(!registry.has_agent("temp").await);
    }

    #[tokio::test]
    async fn test_get_nonexistent() {
        let registry = AgentRegistry::new();
        assert!(registry.get_agent("nope").await.is_none());
    }

    #[tokio::test]
    async fn test_spawn_replaces_existing() {
        let registry = AgentRegistry::new();

        let id1 = registry.spawn_agent("agent", "v1", vec![], None).await;
        let id2 = registry.spawn_agent("agent", "v2", vec![], None).await;

        assert_ne!(id1, id2);
        let agent = registry.get_agent("agent").await.unwrap();
        assert_eq!(agent.system_prompt, "v2");
    }
}
