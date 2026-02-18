//! Backend-agnostic vector database trait.
//!
//! The runtime selects the concrete implementation at startup based on
//! environment variables or config. LanceDB is the default embedded
//! backend; Qdrant is available behind the `vector-qdrant` feature.

use async_trait::async_trait;
use std::collections::HashMap;

use crate::context::types::{
    ContextError, ContextItem, KnowledgeItem, MemoryItem, VectorBatchOperation, VectorId,
};
use crate::context::vector_db::VectorDatabaseStats;
use crate::types::AgentId;
use serde_json::Value;

/// Distance metric for vector similarity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DistanceMetric {
    #[default]
    Cosine,
    Euclidean,
    DotProduct,
}

/// Backend-agnostic vector database trait.
///
/// All vector operations go through this trait. Implementations:
/// - `LanceDbBackend` — embedded, zero-config (default)
/// - `QdrantClientWrapper` — remote, feature-gated behind `vector-qdrant`
/// - `NoOpVectorDatabase` — fallback when no backend is configured
#[async_trait]
pub trait VectorDb: Send + Sync {
    /// Initialize the backend (create collection/table if needed).
    async fn initialize(&self) -> Result<(), ContextError>;

    /// Store a knowledge item with its embedding vector.
    async fn store_knowledge_item(
        &self,
        item: &KnowledgeItem,
        embedding: Vec<f32>,
    ) -> Result<VectorId, ContextError>;

    /// Store a memory item with its embedding vector.
    async fn store_memory_item(
        &self,
        agent_id: AgentId,
        memory: &MemoryItem,
        embedding: Vec<f32>,
    ) -> Result<VectorId, ContextError>;

    /// Store multiple items in batch.
    async fn batch_store(&self, batch: VectorBatchOperation)
        -> Result<Vec<VectorId>, ContextError>;

    /// Search the knowledge base by semantic similarity.
    async fn search_knowledge_base(
        &self,
        agent_id: AgentId,
        query_embedding: Vec<f32>,
        limit: usize,
    ) -> Result<Vec<KnowledgeItem>, ContextError>;

    /// Semantic similarity search returning context items.
    async fn semantic_search(
        &self,
        agent_id: AgentId,
        query_embedding: Vec<f32>,
        limit: usize,
        threshold: f32,
    ) -> Result<Vec<ContextItem>, ContextError>;

    /// Advanced search with metadata filters.
    async fn advanced_search(
        &self,
        agent_id: AgentId,
        query_embedding: Vec<f32>,
        filters: HashMap<String, String>,
        limit: usize,
        threshold: f32,
    ) -> Result<Vec<super::types::VectorSearchResult>, ContextError>;

    /// Delete a knowledge item by vector ID.
    async fn delete_knowledge_item(&self, vector_id: VectorId) -> Result<(), ContextError>;

    /// Delete multiple vectors by ID.
    async fn batch_delete(&self, vector_ids: Vec<VectorId>) -> Result<(), ContextError>;

    /// Update metadata on an existing vector.
    async fn update_metadata(
        &self,
        vector_id: VectorId,
        metadata: HashMap<String, Value>,
    ) -> Result<(), ContextError>;

    /// Get statistics about the vector database.
    async fn get_stats(&self) -> Result<VectorDatabaseStats, ContextError>;

    /// Create an index on a field.
    async fn create_index(&self, field_name: &str) -> Result<(), ContextError>;

    /// Optimize the collection (compact, reindex, etc.).
    async fn optimize_collection(&self) -> Result<(), ContextError>;

    /// Health check — can the backend be reached?
    async fn health_check(&self) -> Result<bool, ContextError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_distance_metric_default() {
        let metric = DistanceMetric::default();
        assert!(matches!(metric, DistanceMetric::Cosine));
    }
}
