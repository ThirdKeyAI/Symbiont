//! LanceDB embedded vector backend.
//!
//! Zero-config: stores data in `./data/vector_db/` by default.
//! No external services required — ships with the binary.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::types::Float32Type;
use arrow_array::{
    Array, FixedSizeListArray, Int64Array, RecordBatch, RecordBatchIterator, StringArray,
};
use arrow_schema::{DataType, Field, Schema};
use async_trait::async_trait;
use futures::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase};
use serde_json::Value;
use tokio::sync::RwLock;

use crate::context::types::{
    ContextError, ContextItem, KnowledgeItem, KnowledgeSource, KnowledgeType, MemoryItem,
    VectorBatchOperation, VectorId,
};
use crate::context::vector_db::VectorDatabaseStats;
use crate::context::vector_db_trait::{DistanceMetric, VectorDb};
use crate::types::AgentId;

/// Configuration for the embedded LanceDB backend.
#[derive(Debug, Clone)]
pub struct LanceDbConfig {
    /// Path to the LanceDB data directory.
    pub data_path: PathBuf,
    /// Collection/table name.
    pub collection_name: String,
    /// Vector dimension.
    pub vector_dimension: usize,
    /// Distance metric.
    pub distance_metric: DistanceMetric,
}

impl Default for LanceDbConfig {
    fn default() -> Self {
        Self {
            data_path: PathBuf::from("./data/vector_db"),
            collection_name: "symbiont_context".to_string(),
            vector_dimension: 384,
            distance_metric: DistanceMetric::Cosine,
        }
    }
}

pub struct LanceDbBackend {
    db: lancedb::Connection,
    config: LanceDbConfig,
    table: Arc<RwLock<Option<lancedb::Table>>>,
}

impl LanceDbBackend {
    pub async fn new(config: LanceDbConfig) -> Result<Self, ContextError> {
        std::fs::create_dir_all(&config.data_path).map_err(|e| ContextError::StorageError {
            reason: format!(
                "Failed to create LanceDB data dir {:?}: {}",
                config.data_path, e
            ),
        })?;

        let db = lancedb::connect(config.data_path.to_str().unwrap_or("./data/vector_db"))
            .execute()
            .await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to connect to LanceDB: {}", e),
            })?;

        Ok(Self {
            db,
            config,
            table: Arc::new(RwLock::new(None)),
        })
    }

    fn build_schema(&self) -> Arc<Schema> {
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("content", DataType::Utf8, false),
            Field::new("agent_id", DataType::Utf8, true),
            Field::new(
                "vector",
                DataType::FixedSizeList(
                    Arc::new(Field::new("item", DataType::Float32, true)),
                    self.config.vector_dimension as i32,
                ),
                true,
            ),
            Field::new("metadata_json", DataType::Utf8, true),
            Field::new("source", DataType::Utf8, true),
            Field::new("content_type", DataType::Utf8, true),
            Field::new("created_at", DataType::Int64, true),
        ]))
    }

    fn distance_type(&self) -> lancedb::DistanceType {
        match self.config.distance_metric {
            DistanceMetric::Cosine => lancedb::DistanceType::Cosine,
            DistanceMetric::Euclidean => lancedb::DistanceType::L2,
            DistanceMetric::DotProduct => lancedb::DistanceType::Dot,
        }
    }

    async fn get_table(&self) -> Result<lancedb::Table, ContextError> {
        let guard = self.table.read().await;
        guard.clone().ok_or_else(|| ContextError::StorageError {
            reason: "LanceDB table not initialized — call initialize() first".into(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn make_record_batch(
        &self,
        schema: &Arc<Schema>,
        id: &str,
        content: &str,
        agent_id: &str,
        embedding: &[f32],
        metadata_json: &str,
        source: &str,
        content_type: &str,
    ) -> Result<RecordBatch, ContextError> {
        if embedding.len() != self.config.vector_dimension {
            return Err(ContextError::StorageError {
                reason: format!(
                    "Dimension mismatch: expected {}, got {}",
                    self.config.vector_dimension,
                    embedding.len()
                ),
            });
        }

        let vector_array = FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
            vec![Some(embedding.iter().map(|v| Some(*v)).collect::<Vec<_>>())],
            self.config.vector_dimension as i32,
        );

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec![id])),
                Arc::new(StringArray::from(vec![content])),
                Arc::new(StringArray::from(vec![agent_id])),
                Arc::new(vector_array),
                Arc::new(StringArray::from(vec![metadata_json])),
                Arc::new(StringArray::from(vec![source])),
                Arc::new(StringArray::from(vec![content_type])),
                Arc::new(Int64Array::from(vec![now_ms])),
            ],
        )
        .map_err(|e| ContextError::StorageError {
            reason: format!("Failed to create RecordBatch: {}", e),
        })
    }

    fn parse_knowledge_item_from_batch(
        &self,
        batch: &RecordBatch,
        row: usize,
    ) -> Option<KnowledgeItem> {
        let id_col = batch
            .column_by_name("id")
            .and_then(|c| c.as_any().downcast_ref::<StringArray>())?;
        let content_col = batch
            .column_by_name("content")
            .and_then(|c| c.as_any().downcast_ref::<StringArray>())?;
        let source_col = batch
            .column_by_name("source")
            .and_then(|c| c.as_any().downcast_ref::<StringArray>())?;
        let created_col = batch
            .column_by_name("created_at")
            .and_then(|c| c.as_any().downcast_ref::<Int64Array>())?;

        let id_str = id_col.value(row);
        let content = content_col.value(row);
        let source_str = source_col.value(row);
        let created_ms = created_col.value(row);

        let kid = uuid::Uuid::parse_str(id_str)
            .ok()
            .map(crate::context::types::KnowledgeId)
            .unwrap_or_default();

        let source = match source_str {
            "UserProvided" => KnowledgeSource::UserProvided,
            "Experience" => KnowledgeSource::Experience,
            "Learning" => KnowledgeSource::Learning,
            _ => KnowledgeSource::UserProvided,
        };

        let created_at =
            std::time::UNIX_EPOCH + std::time::Duration::from_millis(created_ms.max(0) as u64);

        Some(KnowledgeItem {
            id: kid,
            content: content.to_string(),
            knowledge_type: KnowledgeType::Fact,
            confidence: 0.9,
            relevance_score: 0.8,
            source,
            created_at,
        })
    }
}

#[async_trait]
impl VectorDb for LanceDbBackend {
    async fn initialize(&self) -> Result<(), ContextError> {
        let table_names =
            self.db
                .table_names()
                .execute()
                .await
                .map_err(|e| ContextError::StorageError {
                    reason: format!("Failed to list LanceDB tables: {}", e),
                })?;

        let table = if table_names.contains(&self.config.collection_name) {
            self.db
                .open_table(&self.config.collection_name)
                .execute()
                .await
                .map_err(|e| ContextError::StorageError {
                    reason: format!("Failed to open LanceDB table: {}", e),
                })?
        } else {
            // Create table with an initial empty batch
            let schema = self.build_schema();
            let empty_batch = RecordBatch::new_empty(schema.clone());
            let batches = RecordBatchIterator::new(vec![Ok(empty_batch)], schema);

            self.db
                .create_table(&self.config.collection_name, Box::new(batches))
                .execute()
                .await
                .map_err(|e| ContextError::StorageError {
                    reason: format!("Failed to create LanceDB table: {}", e),
                })?
        };

        let mut guard = self.table.write().await;
        *guard = Some(table);
        Ok(())
    }

    async fn store_knowledge_item(
        &self,
        item: &KnowledgeItem,
        embedding: Vec<f32>,
    ) -> Result<VectorId, ContextError> {
        let table = self.get_table().await?;
        let schema = self.build_schema();
        let vector_id = VectorId::new();

        let metadata = serde_json::json!({
            "knowledge_type": format!("{:?}", item.knowledge_type),
            "confidence": item.confidence,
            "relevance_score": item.relevance_score,
        });

        let source_str = format!("{:?}", item.source);

        let batch = self.make_record_batch(
            &schema,
            &vector_id.to_string(),
            &item.content,
            "",
            &embedding,
            &metadata.to_string(),
            &source_str,
            "knowledge",
        )?;

        let batches = RecordBatchIterator::new(vec![Ok(batch)], schema);
        table
            .add(Box::new(batches))
            .execute()
            .await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to store knowledge item: {}", e),
            })?;

        Ok(vector_id)
    }

    async fn store_memory_item(
        &self,
        agent_id: AgentId,
        memory: &MemoryItem,
        embedding: Vec<f32>,
    ) -> Result<VectorId, ContextError> {
        let table = self.get_table().await?;
        let schema = self.build_schema();
        let vector_id = VectorId::new();

        let metadata = serde_json::json!({
            "memory_type": format!("{:?}", memory.memory_type),
            "importance": memory.importance,
        });

        let batch = self.make_record_batch(
            &schema,
            &vector_id.to_string(),
            &memory.content,
            &agent_id.to_string(),
            &embedding,
            &metadata.to_string(),
            "memory",
            &format!("{:?}", memory.memory_type),
        )?;

        let batches = RecordBatchIterator::new(vec![Ok(batch)], schema);
        table
            .add(Box::new(batches))
            .execute()
            .await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to store memory item: {}", e),
            })?;

        Ok(vector_id)
    }

    async fn batch_store(
        &self,
        batch: VectorBatchOperation,
    ) -> Result<Vec<VectorId>, ContextError> {
        let mut ids = Vec::with_capacity(batch.items.len());
        for item in &batch.items {
            let vector_id = VectorId::new();
            let embedding = item.embedding.clone().unwrap_or_default();
            if embedding.is_empty() {
                ids.push(vector_id);
                continue;
            }

            let table = self.get_table().await?;
            let schema = self.build_schema();
            let metadata_json = serde_json::json!({
                "source_id": item.metadata.source_id,
                "tags": item.metadata.tags,
            })
            .to_string();

            let record = self.make_record_batch(
                &schema,
                &vector_id.to_string(),
                &item.content,
                &item.metadata.agent_id.to_string(),
                &embedding,
                &metadata_json,
                &item.metadata.source_id,
                &format!("{:?}", item.metadata.content_type),
            )?;

            let batches = RecordBatchIterator::new(vec![Ok(record)], schema);
            table.add(Box::new(batches)).execute().await.map_err(|e| {
                ContextError::StorageError {
                    reason: format!("Failed to batch store item: {}", e),
                }
            })?;

            ids.push(vector_id);
        }
        Ok(ids)
    }

    async fn search_knowledge_base(
        &self,
        _agent_id: AgentId,
        query_embedding: Vec<f32>,
        limit: usize,
    ) -> Result<Vec<KnowledgeItem>, ContextError> {
        let table = self.get_table().await?;

        let results = table
            .vector_search(query_embedding)
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to create vector search: {}", e),
            })?
            .distance_type(self.distance_type())
            .limit(limit)
            .execute()
            .await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Vector search failed: {}", e),
            })?
            .try_collect::<Vec<_>>()
            .await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to collect search results: {}", e),
            })?;

        let mut items = Vec::new();
        for batch in &results {
            for row in 0..batch.num_rows() {
                if let Some(item) = self.parse_knowledge_item_from_batch(batch, row) {
                    items.push(item);
                }
            }
        }

        Ok(items)
    }

    async fn semantic_search(
        &self,
        agent_id: AgentId,
        query_embedding: Vec<f32>,
        limit: usize,
        _threshold: f32,
    ) -> Result<Vec<ContextItem>, ContextError> {
        let knowledge_items = self
            .search_knowledge_base(agent_id, query_embedding, limit)
            .await?;

        Ok(knowledge_items
            .into_iter()
            .map(|ki| ContextItem {
                id: crate::context::types::ContextId::new(),
                content: ki.content,
                item_type: crate::context::types::ContextItemType::Knowledge(ki.knowledge_type),
                relevance_score: ki.relevance_score,
                timestamp: ki.created_at,
                metadata: HashMap::new(),
            })
            .collect())
    }

    async fn advanced_search(
        &self,
        agent_id: AgentId,
        query_embedding: Vec<f32>,
        _filters: HashMap<String, String>,
        limit: usize,
        _threshold: f32,
    ) -> Result<Vec<crate::context::types::VectorSearchResult>, ContextError> {
        let knowledge_items = self
            .search_knowledge_base(agent_id, query_embedding, limit)
            .await?;

        Ok(knowledge_items
            .into_iter()
            .map(|ki| crate::context::types::VectorSearchResult {
                id: VectorId::new(),
                content: ki.content,
                score: ki.relevance_score,
                metadata: HashMap::new(),
                embedding: None,
            })
            .collect())
    }

    async fn delete_knowledge_item(&self, vector_id: VectorId) -> Result<(), ContextError> {
        let table = self.get_table().await?;
        table
            .delete(&format!("id = '{}'", vector_id))
            .await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to delete item: {}", e),
            })?;
        Ok(())
    }

    async fn batch_delete(&self, vector_ids: Vec<VectorId>) -> Result<(), ContextError> {
        for id in vector_ids {
            self.delete_knowledge_item(id).await?;
        }
        Ok(())
    }

    async fn update_metadata(
        &self,
        _vector_id: VectorId,
        _metadata: HashMap<String, Value>,
    ) -> Result<(), ContextError> {
        // LanceDB doesn't have native metadata update — would need delete+reinsert
        Ok(())
    }

    async fn get_stats(&self) -> Result<VectorDatabaseStats, ContextError> {
        let table = self.get_table().await?;
        let count = table
            .count_rows(None)
            .await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to count rows: {}", e),
            })?;

        Ok(VectorDatabaseStats {
            total_vectors: count,
            collection_size_bytes: 0,
            avg_query_time_ms: 0.0,
        })
    }

    async fn create_index(&self, _field_name: &str) -> Result<(), ContextError> {
        // LanceDB creates indexes automatically during optimization
        Ok(())
    }

    async fn optimize_collection(&self) -> Result<(), ContextError> {
        let table = self.get_table().await?;
        table
            .optimize(lancedb::table::OptimizeAction::All)
            .await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to optimize collection: {}", e),
            })?;
        Ok(())
    }

    async fn health_check(&self) -> Result<bool, ContextError> {
        let result = self.db.table_names().execute().await;
        Ok(result.is_ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::types::KnowledgeId;
    use tempfile::TempDir;

    fn make_test_config(tmp: &TempDir) -> LanceDbConfig {
        LanceDbConfig {
            data_path: tmp.path().to_path_buf(),
            collection_name: "test_collection".to_string(),
            vector_dimension: 4,
            distance_metric: DistanceMetric::Cosine,
        }
    }

    fn make_knowledge_item(content: &str) -> KnowledgeItem {
        KnowledgeItem {
            id: KnowledgeId::new(),
            content: content.to_string(),
            knowledge_type: KnowledgeType::Fact,
            confidence: 0.9,
            relevance_score: 0.8,
            source: KnowledgeSource::UserProvided,
            created_at: std::time::SystemTime::now(),
        }
    }

    #[tokio::test]
    async fn test_lance_initialize_and_health() {
        let tmp = TempDir::new().unwrap();
        let backend = LanceDbBackend::new(make_test_config(&tmp)).await.unwrap();
        backend.initialize().await.unwrap();
        assert!(backend.health_check().await.unwrap());
    }

    #[tokio::test]
    async fn test_lance_store_and_count() {
        let tmp = TempDir::new().unwrap();
        let backend = LanceDbBackend::new(make_test_config(&tmp)).await.unwrap();
        backend.initialize().await.unwrap();

        let item = make_knowledge_item("Rust is a systems language");
        let embedding = vec![0.1, 0.2, 0.3, 0.4];
        let id = backend
            .store_knowledge_item(&item, embedding)
            .await
            .unwrap();
        assert_ne!(id, VectorId::default());

        let stats = backend.get_stats().await.unwrap();
        assert_eq!(stats.total_vectors, 1);
    }

    #[tokio::test]
    async fn test_lance_search() {
        let tmp = TempDir::new().unwrap();
        let backend = LanceDbBackend::new(make_test_config(&tmp)).await.unwrap();
        backend.initialize().await.unwrap();

        let item1 = make_knowledge_item("Rust is fast");
        backend
            .store_knowledge_item(&item1, vec![1.0, 0.0, 0.0, 0.0])
            .await
            .unwrap();

        let item2 = make_knowledge_item("Python is easy");
        backend
            .store_knowledge_item(&item2, vec![0.0, 1.0, 0.0, 0.0])
            .await
            .unwrap();

        let agent_id = AgentId::new();
        let results = backend
            .search_knowledge_base(agent_id, vec![0.9, 0.1, 0.0, 0.0], 1)
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("Rust"));
    }

    #[tokio::test]
    async fn test_lance_delete() {
        let tmp = TempDir::new().unwrap();
        let backend = LanceDbBackend::new(make_test_config(&tmp)).await.unwrap();
        backend.initialize().await.unwrap();

        let item = make_knowledge_item("Delete me");
        let id = backend
            .store_knowledge_item(&item, vec![0.1, 0.2, 0.3, 0.4])
            .await
            .unwrap();

        backend.delete_knowledge_item(id).await.unwrap();
        let stats = backend.get_stats().await.unwrap();
        assert_eq!(stats.total_vectors, 0);
    }

    #[tokio::test]
    async fn test_lance_optimize() {
        let tmp = TempDir::new().unwrap();
        let backend = LanceDbBackend::new(make_test_config(&tmp)).await.unwrap();
        backend.initialize().await.unwrap();
        // Should not error on empty collection
        backend.optimize_collection().await.unwrap();
    }
}
