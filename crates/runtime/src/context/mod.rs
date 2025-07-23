//! Context management module for agent memory and knowledge systems
//!
//! This module provides the core infrastructure for managing agent contexts,
//! including hierarchical memory systems, knowledge bases, and persistent
//! context storage with session continuity.
//!
//! # Architecture
//!
//! The context system is built around several key components:
//!
//! - **AgentContext**: The main container for an agent's memory and knowledge
//! - **HierarchicalMemory**: Multi-layered memory system (short-term, long-term, working, episodic, semantic)
//! - **KnowledgeBase**: Structured storage for facts, procedures, and learned patterns
//! - **ContextManager**: Trait and implementation for context operations
//!
//! # Usage
//!
//! ```rust,no_run
//! use symbiont_runtime::context::{StandardContextManager, ContextManagerConfig};
//! use symbiont_runtime::types::AgentId;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = ContextManagerConfig::default();
//! let context_manager = StandardContextManager::new(config);
//! context_manager.initialize().await?;
//!
//! let agent_id = AgentId::new();
//! let session_id = context_manager.create_session(agent_id).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Features
//!
//! - **Hierarchical Memory**: Multiple memory types with different retention policies
//! - **Knowledge Management**: Structured storage and retrieval of facts, procedures, and patterns
//! - **Semantic Search**: Vector-based similarity search across memory and knowledge
//! - **Session Management**: Persistent context across agent sessions
//! - **Knowledge Sharing**: Secure sharing of knowledge between agents
//! - **Retention Policies**: Automatic archiving and cleanup of old context data
//! - **Access Control**: Policy-driven access control for context operations

pub mod manager;
pub mod types;
pub mod vector_db;

// Re-export commonly used types and traits
pub use types::{
    AccessLevel, AgentContext, ContextError, ContextId, ContextPersistence, ContextQuery,
    FilePersistenceConfig, HierarchicalMemory, Knowledge, KnowledgeBase, KnowledgeId,
    KnowledgeItem, KnowledgeSource, KnowledgeType, MemoryItem, MemoryType, QueryType,
    RetentionPolicy, SessionId, StorageStats, VectorBatchItem, VectorBatchOperation,
    VectorContentType, VectorId, VectorMetadata, VectorOperationType, VectorSearchResult,
};

pub use manager::{ContextManager, ContextManagerConfig, FilePersistence, StandardContextManager};

pub use vector_db::{
    EmbeddingService, MockEmbeddingService, QdrantClientWrapper, QdrantConfig, QdrantDistance,
    TfIdfEmbeddingService, VectorDatabase, VectorDatabaseStats,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::AgentId;
    use std::time::SystemTime;

    #[tokio::test]
    async fn test_context_manager_creation() {
        let config = ContextManagerConfig::default();
        let manager = StandardContextManager::new(config);
        assert!(manager.initialize().await.is_ok());
    }

    #[tokio::test]
    async fn test_session_creation() {
        let config = ContextManagerConfig::default();
        let manager = StandardContextManager::new(config);
        manager.initialize().await.unwrap();

        let agent_id = AgentId::new();
        let session_id = manager.create_session(agent_id).await.unwrap();

        // Verify session was created
        let context = manager
            .retrieve_context(agent_id, Some(session_id))
            .await
            .unwrap();
        assert!(context.is_some());
        assert_eq!(context.unwrap().session_id, session_id);
    }

    #[tokio::test]
    async fn test_memory_operations() {
        let config = ContextManagerConfig::default();
        let manager = StandardContextManager::new(config);
        manager.initialize().await.unwrap();

        let agent_id = AgentId::new();
        let _session_id = manager.create_session(agent_id).await.unwrap();

        // Test memory updates
        let memory_updates = vec![types::MemoryUpdate {
            target: types::MemoryTarget::Working("test_key".to_string()),
            operation: types::UpdateOperation::Add,
            data: serde_json::Value::String("test_value".to_string()),
        }];

        assert!(manager
            .update_memory(agent_id, memory_updates)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn test_knowledge_operations() {
        let config = ContextManagerConfig::default();
        let manager = StandardContextManager::new(config);
        manager.initialize().await.unwrap();

        let agent_id = AgentId::new();
        let _session_id = manager.create_session(agent_id).await.unwrap();

        // Test adding knowledge
        let fact = types::KnowledgeFact {
            id: KnowledgeId::new(),
            subject: "test".to_string(),
            predicate: "is".to_string(),
            object: "example".to_string(),
            confidence: 0.9,
            source: types::KnowledgeSource::UserProvided,
            created_at: SystemTime::now(),
            verified: true,
        };

        let _knowledge_id = manager
            .add_knowledge(agent_id, Knowledge::Fact(fact))
            .await
            .unwrap();

        // Test searching knowledge
        let results = manager
            .search_knowledge(agent_id, "test", 10)
            .await
            .unwrap();
        assert!(!results.is_empty());
    }

    #[tokio::test]
    async fn test_context_query() {
        let config = ContextManagerConfig::default();
        let manager = StandardContextManager::new(config);
        manager.initialize().await.unwrap();

        let agent_id = AgentId::new();
        let _session_id = manager.create_session(agent_id).await.unwrap();

        // Test context query
        let query = ContextQuery {
            query_type: QueryType::Semantic,
            search_terms: vec!["test".to_string()],
            max_results: 10,
            time_range: None,
            memory_types: vec![],
            relevance_threshold: 0.7,
            include_embeddings: false,
        };

        let results = manager.query_context(agent_id, query).await.unwrap();
        // Results may be empty for a new context, but the operation should succeed
        assert!(results.len() <= 10);
    }

    #[tokio::test]
    async fn test_context_stats() {
        let config = ContextManagerConfig::default();
        let manager = StandardContextManager::new(config);
        manager.initialize().await.unwrap();

        let agent_id = AgentId::new();
        let _session_id = manager.create_session(agent_id).await.unwrap();

        let stats = manager.get_context_stats(agent_id).await.unwrap();
        assert_eq!(stats.total_memory_items, 0);
        assert_eq!(stats.total_knowledge_items, 0);
        assert_eq!(stats.total_conversations, 0);
    }
}
