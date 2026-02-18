//! Vector Database integration for Qdrant

use async_trait::async_trait;
#[cfg(feature = "vector-qdrant")]
use qdrant_client::config::QdrantConfig as ClientConfig;
#[cfg(feature = "vector-qdrant")]
use qdrant_client::qdrant::{
    Condition, CreateCollection, DeletePoints, Distance, FieldCondition, Filter, Match, PointId,
    PointStruct, PointsIdsList, PointsSelector, SearchPoints, UpsertPoints, Value as QdrantValue,
    VectorParams, VectorsConfig, WithPayloadSelector, WithVectorsSelector,
};
#[cfg(feature = "vector-qdrant")]
use qdrant_client::Qdrant;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::types::*;
use crate::types::AgentId;

/// Convert Qdrant errors to ContextError with specific mappings
#[cfg(feature = "vector-qdrant")]
fn map_qdrant_error(error: qdrant_client::QdrantError) -> ContextError {
    match error {
        qdrant_client::QdrantError::ResponseError { status, .. } => {
            let status_code = status.code() as u16;
            match status_code {
                404 => ContextError::StorageError {
                    reason: "Collection or point not found in Qdrant".to_string(),
                },
                401 | 403 => ContextError::AccessDenied {
                    reason: "Authentication failed for Qdrant database".to_string(),
                },
                400 => ContextError::InvalidOperation {
                    reason: "Invalid request to Qdrant database".to_string(),
                },
                500..=599 => ContextError::StorageError {
                    reason: format!("Qdrant server error: {}", status),
                },
                _ => ContextError::StorageError {
                    reason: format!("Qdrant API error: {}", status),
                },
            }
        }
        qdrant_client::QdrantError::ConversionError { .. } => ContextError::InvalidOperation {
            reason: "Data conversion error with Qdrant".to_string(),
        },
        _ => ContextError::StorageError {
            reason: format!("Qdrant database error: {}", error),
        },
    }
}

/// Configuration for Qdrant vector database
#[derive(Debug, Clone)]
pub struct QdrantConfig {
    /// Qdrant server URL
    pub url: String,
    /// API key for authentication (optional)
    pub api_key: Option<String>,
    /// Collection name for storing embeddings
    pub collection_name: String,
    /// Vector dimension
    pub vector_dimension: usize,
    /// Distance metric for similarity calculation
    pub distance_metric: QdrantDistance,
    /// Maximum number of vectors per batch operation
    pub batch_size: usize,
    /// Connection timeout in seconds
    pub timeout_seconds: u64,
}

impl Default for QdrantConfig {
    fn default() -> Self {
        Self {
            url: "http://localhost:6333".to_string(),
            api_key: None,
            collection_name: "symbiont_context".to_string(),
            vector_dimension: 384, // Common embedding dimension
            distance_metric: QdrantDistance::Cosine,
            batch_size: 100,
            timeout_seconds: 30,
        }
    }
}

/// Distance metrics supported by Qdrant
#[derive(Debug, Clone)]
pub enum QdrantDistance {
    Cosine,
    Euclidean,
    Dot,
}

#[cfg(feature = "vector-qdrant")]
impl From<QdrantDistance> for Distance {
    fn from(distance: QdrantDistance) -> Self {
        match distance {
            QdrantDistance::Cosine => Distance::Cosine,
            QdrantDistance::Euclidean => Distance::Euclid,
            QdrantDistance::Dot => Distance::Dot,
        }
    }
}

/// Vector database operations trait
#[async_trait]
pub trait VectorDatabase: Send + Sync {
    /// Initialize the vector database connection and collection
    async fn initialize(&self) -> Result<(), ContextError>;

    /// Store a knowledge item with its embedding
    async fn store_knowledge_item(
        &self,
        item: &KnowledgeItem,
        embedding: Vec<f32>,
    ) -> Result<VectorId, ContextError>;

    /// Store memory item with embedding
    async fn store_memory_item(
        &self,
        agent_id: AgentId,
        memory: &MemoryItem,
        embedding: Vec<f32>,
    ) -> Result<VectorId, ContextError>;

    /// Batch store multiple items for performance
    async fn batch_store(&self, batch: VectorBatchOperation)
        -> Result<Vec<VectorId>, ContextError>;

    /// Search for similar knowledge items
    async fn search_knowledge_base(
        &self,
        agent_id: AgentId,
        query_embedding: Vec<f32>,
        limit: usize,
    ) -> Result<Vec<KnowledgeItem>, ContextError>;

    /// Perform semantic search with text query
    async fn semantic_search(
        &self,
        agent_id: AgentId,
        query_embedding: Vec<f32>,
        limit: usize,
        threshold: f32,
    ) -> Result<Vec<ContextItem>, ContextError>;

    /// Advanced search with filters and metadata
    async fn advanced_search(
        &self,
        agent_id: AgentId,
        query_embedding: Vec<f32>,
        filters: HashMap<String, String>,
        limit: usize,
        threshold: f32,
    ) -> Result<Vec<VectorSearchResult>, ContextError>;

    /// Delete knowledge item by ID
    async fn delete_knowledge_item(&self, vector_id: VectorId) -> Result<(), ContextError>;

    /// Batch delete multiple items
    async fn batch_delete(&self, vector_ids: Vec<VectorId>) -> Result<(), ContextError>;

    /// Update knowledge item metadata
    async fn update_metadata(
        &self,
        vector_id: VectorId,
        metadata: HashMap<String, Value>,
    ) -> Result<(), ContextError>;

    /// Get collection statistics
    async fn get_stats(&self) -> Result<VectorDatabaseStats, ContextError>;

    /// Create index for better performance
    async fn create_index(&self, field_name: &str) -> Result<(), ContextError>;

    /// Optimize collection for better search performance
    async fn optimize_collection(&self) -> Result<(), ContextError>;
}

/// Statistics for vector database operations
#[derive(Debug, Clone)]
pub struct VectorDatabaseStats {
    pub total_vectors: usize,
    pub collection_size_bytes: usize,
    pub avg_query_time_ms: f32,
}

/// Qdrant client wrapper implementation
#[cfg(feature = "vector-qdrant")]
pub struct QdrantClientWrapper {
    client: Arc<RwLock<Option<Arc<Qdrant>>>>,
    config: QdrantConfig,
}

#[cfg(feature = "vector-qdrant")]
impl QdrantClientWrapper {
    /// Create a new QdrantClientWrapper
    pub fn new(config: QdrantConfig) -> Self {
        Self {
            client: Arc::new(RwLock::new(None)),
            config,
        }
    }

    /// Get or create Qdrant client
    async fn get_client(&self) -> Result<Arc<Qdrant>, ContextError> {
        let client_guard = self.client.read().await;
        if let Some(client) = client_guard.as_ref() {
            Ok(Arc::clone(client))
        } else {
            drop(client_guard);

            // Create new client with updated API
            let mut client_config = ClientConfig::from_url(&self.config.url);

            if let Some(api_key) = &self.config.api_key {
                client_config.api_key = Some(api_key.clone());
            }

            let client = Qdrant::new(client_config).map_err(map_qdrant_error)?;

            let client_arc = Arc::new(client);
            let mut client_guard = self.client.write().await;
            *client_guard = Some(Arc::clone(&client_arc));

            Ok(client_arc)
        }
    }

    /// Convert KnowledgeItem to Qdrant metadata
    fn knowledge_item_to_metadata(
        &self,
        item: &KnowledgeItem,
        agent_id: AgentId,
    ) -> HashMap<String, QdrantValue> {
        let mut metadata = HashMap::new();

        metadata.insert(
            "agent_id".to_string(),
            QdrantValue::from(agent_id.to_string()),
        );
        metadata.insert(
            "knowledge_id".to_string(),
            QdrantValue::from(item.id.to_string()),
        );
        metadata.insert(
            "content".to_string(),
            QdrantValue::from(item.content.clone()),
        );
        metadata.insert(
            "knowledge_type".to_string(),
            QdrantValue::from(format!("{:?}", item.knowledge_type)),
        );
        metadata.insert(
            "confidence".to_string(),
            QdrantValue::from(item.confidence as f64),
        );
        metadata.insert(
            "relevance_score".to_string(),
            QdrantValue::from(item.relevance_score as f64),
        );
        metadata.insert(
            "source".to_string(),
            QdrantValue::from(format!("{:?}", item.source)),
        );
        metadata.insert(
            "created_at".to_string(),
            QdrantValue::from(
                item.created_at
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64,
            ),
        );

        metadata
    }

    /// Convert Qdrant point to KnowledgeItem
    fn point_to_knowledge_item(
        &self,
        point: &qdrant_client::qdrant::ScoredPoint,
    ) -> Result<KnowledgeItem, ContextError> {
        let payload = &point.payload;

        let knowledge_id_str = payload
            .get("knowledge_id")
            .and_then(|v| self.extract_string_value(v))
            .ok_or_else(|| ContextError::StorageError {
                reason: "Missing knowledge_id in payload".to_string(),
            })?;

        let knowledge_id = KnowledgeId(uuid::Uuid::parse_str(&knowledge_id_str).map_err(|e| {
            ContextError::StorageError {
                reason: format!("Invalid knowledge_id UUID: {}", e),
            }
        })?);

        let content = payload
            .get("content")
            .and_then(|v| self.extract_string_value(v))
            .unwrap_or_default();

        let knowledge_type_str = payload
            .get("knowledge_type")
            .and_then(|v| self.extract_string_value(v))
            .unwrap_or_else(|| "Fact".to_string());

        let knowledge_type = match knowledge_type_str.as_str() {
            "Fact" => KnowledgeType::Fact,
            "Procedure" => KnowledgeType::Procedure,
            "Pattern" => KnowledgeType::Pattern,
            "Shared" => KnowledgeType::Shared,
            _ => KnowledgeType::Fact,
        };

        let confidence = payload
            .get("confidence")
            .and_then(|v| self.extract_f64_value(v))
            .unwrap_or(0.0) as f32;

        let relevance_score = point.score;

        let source = KnowledgeSource::Learning; // Default, could be parsed from payload

        let created_at = payload
            .get("created_at")
            .and_then(|v| self.extract_i64_value(v))
            .map(|timestamp| {
                std::time::UNIX_EPOCH + std::time::Duration::from_secs(timestamp as u64)
            })
            .unwrap_or_else(std::time::SystemTime::now);

        Ok(KnowledgeItem {
            id: knowledge_id,
            content,
            knowledge_type,
            confidence,
            relevance_score,
            source,
            created_at,
        })
    }

    /// Extract string value from QdrantValue
    fn extract_string_value(&self, value: &QdrantValue) -> Option<String> {
        match value {
            QdrantValue {
                kind: Some(qdrant_client::qdrant::value::Kind::StringValue(s)),
            } => Some(s.clone()),
            _ => None,
        }
    }

    /// Extract f64 value from QdrantValue
    fn extract_f64_value(&self, value: &QdrantValue) -> Option<f64> {
        match value {
            QdrantValue {
                kind: Some(qdrant_client::qdrant::value::Kind::DoubleValue(d)),
            } => Some(*d),
            QdrantValue {
                kind: Some(qdrant_client::qdrant::value::Kind::IntegerValue(i)),
            } => Some(*i as f64),
            _ => None,
        }
    }

    /// Extract i64 value from QdrantValue
    fn extract_i64_value(&self, value: &QdrantValue) -> Option<i64> {
        match value {
            QdrantValue {
                kind: Some(qdrant_client::qdrant::value::Kind::IntegerValue(i)),
            } => Some(*i),
            QdrantValue {
                kind: Some(qdrant_client::qdrant::value::Kind::DoubleValue(d)),
            } => Some(*d as i64),
            _ => None,
        }
    }
}

#[cfg(feature = "vector-qdrant")]
#[async_trait]
impl VectorDatabase for QdrantClientWrapper {
    async fn initialize(&self) -> Result<(), ContextError> {
        let client = self.get_client().await?;

        // Check if collection exists
        let collections = client.list_collections().await.map_err(map_qdrant_error)?;

        let collection_exists = collections
            .collections
            .iter()
            .any(|c| c.name == self.config.collection_name);

        if !collection_exists {
            // Create collection with updated API
            let vectors_config = VectorsConfig {
                config: Some(qdrant_client::qdrant::vectors_config::Config::Params(
                    VectorParams {
                        size: self.config.vector_dimension as u64,
                        distance: Distance::from(self.config.distance_metric.clone()) as i32,
                        hnsw_config: None,
                        quantization_config: None,
                        on_disk: None,
                        datatype: None,
                        multivector_config: None,
                    },
                )),
            };

            let create_collection = CreateCollection {
                collection_name: self.config.collection_name.clone(),
                vectors_config: Some(vectors_config),
                hnsw_config: None,
                wal_config: None,
                optimizers_config: None,
                shard_number: None,
                on_disk_payload: None,
                timeout: Some(self.config.timeout_seconds),
                replication_factor: None,
                write_consistency_factor: None,
                init_from_collection: None,
                quantization_config: None,
                sharding_method: None,
                sparse_vectors_config: None,
                strict_mode_config: None,
            };

            client
                .create_collection(create_collection)
                .await
                .map_err(map_qdrant_error)?;
        }

        Ok(())
    }

    async fn store_knowledge_item(
        &self,
        item: &KnowledgeItem,
        embedding: Vec<f32>,
    ) -> Result<VectorId, ContextError> {
        let client = self.get_client().await?;
        let vector_id = VectorId::new();

        // Extract agent_id from context - for now use a default agent id based on content hash
        let agent_id = {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};

            let mut hasher = DefaultHasher::new();
            item.content.hash(&mut hasher);
            item.id.0.hash(&mut hasher);
            let hash_value = hasher.finish();

            // Create a deterministic UUID from hash
            let uuid_bytes = [
                (hash_value >> 56) as u8,
                (hash_value >> 48) as u8,
                (hash_value >> 40) as u8,
                (hash_value >> 32) as u8,
                (hash_value >> 24) as u8,
                (hash_value >> 16) as u8,
                (hash_value >> 8) as u8,
                hash_value as u8,
                // Fill remaining 8 bytes with more hash data
                (hash_value >> 56) as u8,
                (hash_value >> 48) as u8,
                (hash_value >> 40) as u8,
                (hash_value >> 32) as u8,
                (hash_value >> 24) as u8,
                (hash_value >> 16) as u8,
                (hash_value >> 8) as u8,
                hash_value as u8,
            ];

            AgentId(uuid::Uuid::from_bytes(uuid_bytes))
        };

        // Create point for Qdrant
        let point = PointStruct::new(
            vector_id.0.to_string(),
            embedding,
            self.knowledge_item_to_metadata(item, agent_id),
        );

        let upsert_points = UpsertPoints {
            collection_name: self.config.collection_name.clone(),
            wait: Some(true),
            points: vec![point],
            ordering: None,
            shard_key_selector: None,
        };

        client
            .upsert_points(upsert_points)
            .await
            .map_err(map_qdrant_error)?;

        Ok(vector_id)
    }

    async fn store_memory_item(
        &self,
        agent_id: AgentId,
        memory: &MemoryItem,
        embedding: Vec<f32>,
    ) -> Result<VectorId, ContextError> {
        let client = self.get_client().await?;
        let vector_id = VectorId::new();

        // Create metadata for memory item
        let mut metadata = HashMap::new();
        metadata.insert(
            "agent_id".to_string(),
            QdrantValue::from(agent_id.to_string()),
        );
        metadata.insert(
            "memory_id".to_string(),
            QdrantValue::from(memory.id.to_string()),
        );
        metadata.insert(
            "content".to_string(),
            QdrantValue::from(memory.content.clone()),
        );
        metadata.insert(
            "memory_type".to_string(),
            QdrantValue::from(format!("{:?}", memory.memory_type)),
        );
        metadata.insert(
            "importance".to_string(),
            QdrantValue::from(memory.importance as f64),
        );
        metadata.insert(
            "access_count".to_string(),
            QdrantValue::from(memory.access_count as i64),
        );
        metadata.insert(
            "created_at".to_string(),
            QdrantValue::from(
                memory
                    .created_at
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64,
            ),
        );
        metadata.insert(
            "last_accessed".to_string(),
            QdrantValue::from(
                memory
                    .last_accessed
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64,
            ),
        );

        // Add custom metadata
        for (key, value) in &memory.metadata {
            metadata.insert(format!("meta_{}", key), QdrantValue::from(value.clone()));
        }

        let point = PointStruct::new(vector_id.0.to_string(), embedding, metadata);

        let upsert_points = UpsertPoints {
            collection_name: self.config.collection_name.clone(),
            wait: Some(true),
            points: vec![point],
            ordering: None,
            shard_key_selector: None,
        };

        client
            .upsert_points(upsert_points)
            .await
            .map_err(map_qdrant_error)?;

        Ok(vector_id)
    }

    async fn batch_store(
        &self,
        batch: VectorBatchOperation,
    ) -> Result<Vec<VectorId>, ContextError> {
        let client = self.get_client().await?;
        let mut points = Vec::new();
        let mut vector_ids = Vec::new();

        for item in &batch.items {
            let vector_id = item.id.unwrap_or_else(VectorId::new);
            vector_ids.push(vector_id);

            // Create metadata from VectorMetadata
            let mut metadata = HashMap::new();
            metadata.insert(
                "agent_id".to_string(),
                QdrantValue::from(item.metadata.agent_id.to_string()),
            );
            metadata.insert(
                "content_type".to_string(),
                QdrantValue::from(format!("{:?}", item.metadata.content_type)),
            );
            metadata.insert(
                "source_id".to_string(),
                QdrantValue::from(item.metadata.source_id.clone()),
            );
            metadata.insert(
                "created_at".to_string(),
                QdrantValue::from(
                    item.metadata
                        .created_at
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64,
                ),
            );
            metadata.insert(
                "updated_at".to_string(),
                QdrantValue::from(
                    item.metadata
                        .updated_at
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64,
                ),
            );

            // Add tags
            for (i, tag) in item.metadata.tags.iter().enumerate() {
                metadata.insert(format!("tag_{}", i), QdrantValue::from(tag.clone()));
            }

            // Add custom fields
            for (key, value) in &item.metadata.custom_fields {
                metadata.insert(format!("custom_{}", key), QdrantValue::from(value.clone()));
            }

            let embedding = item
                .embedding
                .clone()
                .unwrap_or_else(|| vec![0.0; self.config.vector_dimension]);

            let point = PointStruct::new(vector_id.0.to_string(), embedding, metadata);

            points.push(point);
        }

        // Process in batches to avoid overwhelming the server
        let batch_size = self.config.batch_size;
        for chunk in points.chunks(batch_size) {
            let upsert_points = UpsertPoints {
                collection_name: self.config.collection_name.clone(),
                wait: Some(true),
                points: chunk.to_vec(),
                ordering: None,
                shard_key_selector: None,
            };

            client
                .upsert_points(upsert_points)
                .await
                .map_err(map_qdrant_error)?;
        }

        Ok(vector_ids)
    }

    async fn search_knowledge_base(
        &self,
        agent_id: AgentId,
        query_embedding: Vec<f32>,
        limit: usize,
    ) -> Result<Vec<KnowledgeItem>, ContextError> {
        let client = self.get_client().await?;

        // Create filter for agent-specific knowledge
        let filter = Filter {
            should: vec![],
            min_should: None,
            must: vec![Condition {
                condition_one_of: Some(qdrant_client::qdrant::condition::ConditionOneOf::Field(
                    FieldCondition {
                        key: "agent_id".to_string(),
                        r#match: Some(Match {
                            match_value: Some(qdrant_client::qdrant::r#match::MatchValue::Keyword(
                                agent_id.to_string(),
                            )),
                        }),
                        range: None,
                        geo_bounding_box: None,
                        geo_radius: None,
                        values_count: None,
                        geo_polygon: None,
                        datetime_range: None,
                        is_empty: None,
                        is_null: None,
                    },
                )),
            }],
            must_not: vec![],
        };

        let search_points = SearchPoints {
            collection_name: self.config.collection_name.clone(),
            vector: query_embedding,
            vector_name: None,
            filter: Some(filter),
            limit: limit as u64,
            with_payload: Some(WithPayloadSelector {
                selector_options: Some(
                    qdrant_client::qdrant::with_payload_selector::SelectorOptions::Enable(true),
                ),
            }),
            params: None,
            score_threshold: None,
            offset: None,
            with_vectors: None,
            read_consistency: None,
            shard_key_selector: None,
            sparse_indices: None,
            timeout: None,
        };

        let search_result = client
            .search_points(search_points)
            .await
            .map_err(map_qdrant_error)?;

        let mut knowledge_items = Vec::new();
        for point in search_result.result {
            match self.point_to_knowledge_item(&point) {
                Ok(item) => knowledge_items.push(item),
                Err(e) => {
                    // Log error but continue processing other points
                    eprintln!("Failed to convert point to knowledge item: {}", e);
                }
            }
        }

        Ok(knowledge_items)
    }

    async fn semantic_search(
        &self,
        agent_id: AgentId,
        query_embedding: Vec<f32>,
        limit: usize,
        threshold: f32,
    ) -> Result<Vec<ContextItem>, ContextError> {
        let client = self.get_client().await?;

        // Create filter for agent-specific context
        let filter = Filter {
            should: vec![],
            min_should: None,
            must: vec![Condition {
                condition_one_of: Some(qdrant_client::qdrant::condition::ConditionOneOf::Field(
                    FieldCondition {
                        key: "agent_id".to_string(),
                        r#match: Some(Match {
                            match_value: Some(qdrant_client::qdrant::r#match::MatchValue::Keyword(
                                agent_id.to_string(),
                            )),
                        }),
                        range: None,
                        geo_bounding_box: None,
                        geo_radius: None,
                        values_count: None,
                        geo_polygon: None,
                        datetime_range: None,
                        is_empty: None,
                        is_null: None,
                    },
                )),
            }],
            must_not: vec![],
        };

        let search_points = SearchPoints {
            collection_name: self.config.collection_name.clone(),
            vector: query_embedding,
            vector_name: None,
            filter: Some(filter),
            limit: limit as u64,
            with_payload: Some(WithPayloadSelector {
                selector_options: Some(
                    qdrant_client::qdrant::with_payload_selector::SelectorOptions::Enable(true),
                ),
            }),
            params: None,
            score_threshold: Some(threshold),
            offset: None,
            with_vectors: None,
            read_consistency: None,
            shard_key_selector: None,
            sparse_indices: None,
            timeout: None,
        };

        let search_result = client
            .search_points(search_points)
            .await
            .map_err(map_qdrant_error)?;

        let mut context_items = Vec::new();
        for point in search_result.result {
            let payload = &point.payload;
            let context_id_str = payload
                .get("context_id")
                .and_then(|v| self.extract_string_value(v))
                .unwrap_or_default();

            let context_id = ContextId(
                uuid::Uuid::parse_str(&context_id_str).unwrap_or_else(|_| uuid::Uuid::new_v4()),
            );

            let content = payload
                .get("content")
                .and_then(|v| self.extract_string_value(v))
                .unwrap_or_default();

            let item_type = ContextItemType::Memory(MemoryType::Semantic); // Default

            let timestamp = payload
                .get("timestamp")
                .and_then(|v| self.extract_i64_value(v))
                .map(|ts| std::time::UNIX_EPOCH + std::time::Duration::from_secs(ts as u64))
                .unwrap_or_else(std::time::SystemTime::now);

            // Extract custom metadata
            let mut metadata = HashMap::new();
            for (key, value) in payload {
                if key.starts_with("meta_") {
                    let meta_key = key.strip_prefix("meta_").unwrap_or(key);
                    if let Some(str_value) = self.extract_string_value(value) {
                        metadata.insert(meta_key.to_string(), str_value);
                    }
                }
            }

            context_items.push(ContextItem {
                id: context_id,
                content,
                item_type,
                relevance_score: point.score,
                timestamp,
                metadata,
            });
        }

        Ok(context_items)
    }

    async fn delete_knowledge_item(&self, vector_id: VectorId) -> Result<(), ContextError> {
        let client = self.get_client().await?;

        let delete_points = DeletePoints {
            collection_name: self.config.collection_name.clone(),
            wait: Some(true),
            points: Some(PointsSelector {
                points_selector_one_of: Some(
                    qdrant_client::qdrant::points_selector::PointsSelectorOneOf::Points(
                        PointsIdsList {
                            ids: vec![PointId::from(vector_id.0.to_string())],
                        },
                    ),
                ),
            }),
            ordering: None,
            shard_key_selector: None,
        };

        client
            .delete_points(delete_points)
            .await
            .map_err(map_qdrant_error)?;

        Ok(())
    }

    async fn advanced_search(
        &self,
        agent_id: AgentId,
        query_embedding: Vec<f32>,
        filters: HashMap<String, String>,
        limit: usize,
        threshold: f32,
    ) -> Result<Vec<VectorSearchResult>, ContextError> {
        let client = self.get_client().await?;

        // Build complex filter with agent_id and additional filters
        let mut conditions = vec![Condition {
            condition_one_of: Some(qdrant_client::qdrant::condition::ConditionOneOf::Field(
                FieldCondition {
                    key: "agent_id".to_string(),
                    r#match: Some(Match {
                        match_value: Some(qdrant_client::qdrant::r#match::MatchValue::Keyword(
                            agent_id.to_string(),
                        )),
                    }),
                    range: None,
                    geo_bounding_box: None,
                    geo_radius: None,
                    values_count: None,
                    geo_polygon: None,
                    datetime_range: None,
                    is_empty: None,
                    is_null: None,
                },
            )),
        }];

        // Add additional filters
        for (key, value) in filters {
            conditions.push(Condition {
                condition_one_of: Some(qdrant_client::qdrant::condition::ConditionOneOf::Field(
                    FieldCondition {
                        key,
                        r#match: Some(Match {
                            match_value: Some(qdrant_client::qdrant::r#match::MatchValue::Keyword(
                                value,
                            )),
                        }),
                        range: None,
                        geo_bounding_box: None,
                        geo_radius: None,
                        values_count: None,
                        geo_polygon: None,
                        datetime_range: None,
                        is_empty: None,
                        is_null: None,
                    },
                )),
            });
        }

        let filter = Filter {
            should: vec![],
            min_should: None,
            must: conditions,
            must_not: vec![],
        };

        let search_points = SearchPoints {
            collection_name: self.config.collection_name.clone(),
            vector: query_embedding,
            vector_name: None,
            filter: Some(filter),
            limit: limit as u64,
            with_payload: Some(WithPayloadSelector {
                selector_options: Some(
                    qdrant_client::qdrant::with_payload_selector::SelectorOptions::Enable(true),
                ),
            }),
            params: None,
            score_threshold: Some(threshold),
            offset: None,
            with_vectors: Some(WithVectorsSelector {
                selector_options: Some(
                    qdrant_client::qdrant::with_vectors_selector::SelectorOptions::Enable(true),
                ),
            }),
            read_consistency: None,
            shard_key_selector: None,
            sparse_indices: None,
            timeout: None,
        };

        let search_result = client
            .search_points(search_points)
            .await
            .map_err(map_qdrant_error)?;

        let mut results = Vec::new();
        for point in search_result.result {
            let payload = &point.payload;
            let vector_id_str = point
                .id
                .map(|id| match id.point_id_options {
                    Some(qdrant_client::qdrant::point_id::PointIdOptions::Uuid(uuid)) => uuid,
                    Some(qdrant_client::qdrant::point_id::PointIdOptions::Num(num)) => {
                        num.to_string()
                    }
                    None => uuid::Uuid::new_v4().to_string(),
                })
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

            let vector_id = VectorId(
                uuid::Uuid::parse_str(&vector_id_str).unwrap_or_else(|_| uuid::Uuid::new_v4()),
            );

            let content = payload
                .get("content")
                .and_then(|v| self.extract_string_value(v))
                .unwrap_or_default();

            // Extract metadata
            let mut metadata = HashMap::new();
            for (key, value) in payload {
                if let Some(str_value) = self.extract_string_value(value) {
                    metadata.insert(key.clone(), str_value);
                }
            }

            // Extract embedding if available
            let embedding = point
                .vectors
                .and_then(|vectors| match vectors.vectors_options {
                    Some(qdrant_client::qdrant::vectors_output::VectorsOptions::Vector(vector)) => {
                        Some(vector.data)
                    }
                    _ => None,
                });

            results.push(VectorSearchResult {
                id: vector_id,
                content,
                score: point.score,
                metadata,
                embedding,
            });
        }

        Ok(results)
    }

    async fn batch_delete(&self, vector_ids: Vec<VectorId>) -> Result<(), ContextError> {
        let client = self.get_client().await?;

        // Process in batches to avoid overwhelming the server
        let batch_size = self.config.batch_size;
        for chunk in vector_ids.chunks(batch_size) {
            let ids: Vec<PointId> = chunk
                .iter()
                .map(|id| PointId::from(id.0.to_string()))
                .collect();

            let delete_points = DeletePoints {
                collection_name: self.config.collection_name.clone(),
                wait: Some(true),
                points: Some(PointsSelector {
                    points_selector_one_of: Some(
                        qdrant_client::qdrant::points_selector::PointsSelectorOneOf::Points(
                            PointsIdsList { ids },
                        ),
                    ),
                }),
                ordering: None,
                shard_key_selector: None,
            };

            client
                .delete_points(delete_points)
                .await
                .map_err(map_qdrant_error)?;
        }

        Ok(())
    }

    async fn update_metadata(
        &self,
        _vector_id: VectorId,
        _metadata: HashMap<String, Value>,
    ) -> Result<(), ContextError> {
        // Qdrant doesn't support direct metadata updates, would need to re-upsert the point
        // For now, return an error indicating this operation is not supported
        Err(ContextError::InvalidOperation {
            reason: "Direct metadata updates not supported, use store_knowledge_item to update"
                .to_string(),
        })
    }

    async fn create_index(&self, field_name: &str) -> Result<(), ContextError> {
        let client = self.get_client().await?;

        // Create payload index for better filtering performance
        let create_index = qdrant_client::qdrant::CreateFieldIndexCollection {
            collection_name: self.config.collection_name.clone(),
            wait: Some(true),
            field_name: field_name.to_string(),
            field_type: Some(qdrant_client::qdrant::FieldType::Keyword as i32),
            field_index_params: None,
            ordering: None,
        };

        client
            .create_field_index(create_index)
            .await
            .map_err(map_qdrant_error)?;

        Ok(())
    }

    async fn optimize_collection(&self) -> Result<(), ContextError> {
        let _client = self.get_client().await?;

        // For now, just return success as collection optimization
        // can be done through Qdrant's admin interface or specific optimization calls
        // The collection is already optimized during creation with appropriate settings

        // In a production environment, you might want to:
        // 1. Call collection optimization endpoints
        // 2. Adjust HNSW parameters
        // 3. Configure memory mapping settings
        // 4. Set up proper indexing strategies

        Ok(())
    }

    async fn get_stats(&self) -> Result<VectorDatabaseStats, ContextError> {
        let client = self.get_client().await?;

        let collection_info = client
            .collection_info(&self.config.collection_name)
            .await
            .map_err(map_qdrant_error)?;

        Ok(VectorDatabaseStats {
            total_vectors: collection_info
                .result
                .map(|r| r.points_count.unwrap_or(0) as usize)
                .unwrap_or(0),
            collection_size_bytes: 0, // Not directly available from Qdrant API
            avg_query_time_ms: 0.0,   // Would need to track this separately
        })
    }
}

#[cfg(feature = "vector-qdrant")]
#[async_trait]
impl super::vector_db_trait::VectorDb for QdrantClientWrapper {
    async fn initialize(&self) -> Result<(), ContextError> {
        <Self as VectorDatabase>::initialize(self).await
    }
    async fn store_knowledge_item(
        &self,
        item: &KnowledgeItem,
        embedding: Vec<f32>,
    ) -> Result<VectorId, ContextError> {
        <Self as VectorDatabase>::store_knowledge_item(self, item, embedding).await
    }
    async fn store_memory_item(
        &self,
        agent_id: AgentId,
        memory: &MemoryItem,
        embedding: Vec<f32>,
    ) -> Result<VectorId, ContextError> {
        <Self as VectorDatabase>::store_memory_item(self, agent_id, memory, embedding).await
    }
    async fn batch_store(
        &self,
        batch: VectorBatchOperation,
    ) -> Result<Vec<VectorId>, ContextError> {
        <Self as VectorDatabase>::batch_store(self, batch).await
    }
    async fn search_knowledge_base(
        &self,
        agent_id: AgentId,
        query_embedding: Vec<f32>,
        limit: usize,
    ) -> Result<Vec<KnowledgeItem>, ContextError> {
        <Self as VectorDatabase>::search_knowledge_base(self, agent_id, query_embedding, limit)
            .await
    }
    async fn semantic_search(
        &self,
        agent_id: AgentId,
        query_embedding: Vec<f32>,
        limit: usize,
        threshold: f32,
    ) -> Result<Vec<ContextItem>, ContextError> {
        <Self as VectorDatabase>::semantic_search(self, agent_id, query_embedding, limit, threshold)
            .await
    }
    async fn advanced_search(
        &self,
        agent_id: AgentId,
        query_embedding: Vec<f32>,
        filters: HashMap<String, String>,
        limit: usize,
        threshold: f32,
    ) -> Result<Vec<VectorSearchResult>, ContextError> {
        <Self as VectorDatabase>::advanced_search(
            self,
            agent_id,
            query_embedding,
            filters,
            limit,
            threshold,
        )
        .await
    }
    async fn delete_knowledge_item(&self, vector_id: VectorId) -> Result<(), ContextError> {
        <Self as VectorDatabase>::delete_knowledge_item(self, vector_id).await
    }
    async fn batch_delete(&self, vector_ids: Vec<VectorId>) -> Result<(), ContextError> {
        <Self as VectorDatabase>::batch_delete(self, vector_ids).await
    }
    async fn update_metadata(
        &self,
        vector_id: VectorId,
        metadata: HashMap<String, Value>,
    ) -> Result<(), ContextError> {
        <Self as VectorDatabase>::update_metadata(self, vector_id, metadata).await
    }
    async fn get_stats(&self) -> Result<VectorDatabaseStats, ContextError> {
        <Self as VectorDatabase>::get_stats(self).await
    }
    async fn create_index(&self, field_name: &str) -> Result<(), ContextError> {
        <Self as VectorDatabase>::create_index(self, field_name).await
    }
    async fn optimize_collection(&self) -> Result<(), ContextError> {
        <Self as VectorDatabase>::optimize_collection(self).await
    }
    async fn health_check(&self) -> Result<bool, ContextError> {
        <Self as VectorDatabase>::get_stats(self)
            .await
            .map(|_| true)
    }
}

/// No-op vector database for when no backend is configured
pub struct NoOpVectorDatabase;

#[async_trait]
impl VectorDatabase for NoOpVectorDatabase {
    async fn initialize(&self) -> Result<(), ContextError> {
        Ok(())
    }

    async fn store_knowledge_item(
        &self,
        _item: &KnowledgeItem,
        _embedding: Vec<f32>,
    ) -> Result<VectorId, ContextError> {
        Ok(VectorId::new())
    }

    async fn store_memory_item(
        &self,
        _agent_id: AgentId,
        _memory: &MemoryItem,
        _embedding: Vec<f32>,
    ) -> Result<VectorId, ContextError> {
        Ok(VectorId::new())
    }

    async fn batch_store(
        &self,
        batch: VectorBatchOperation,
    ) -> Result<Vec<VectorId>, ContextError> {
        Ok(batch.items.iter().map(|_| VectorId::new()).collect())
    }

    async fn search_knowledge_base(
        &self,
        _agent_id: AgentId,
        _query_embedding: Vec<f32>,
        _limit: usize,
    ) -> Result<Vec<KnowledgeItem>, ContextError> {
        Ok(Vec::new())
    }

    async fn semantic_search(
        &self,
        _agent_id: AgentId,
        _query_embedding: Vec<f32>,
        _limit: usize,
        _threshold: f32,
    ) -> Result<Vec<ContextItem>, ContextError> {
        Ok(Vec::new())
    }

    async fn advanced_search(
        &self,
        _agent_id: AgentId,
        _query_embedding: Vec<f32>,
        _filters: HashMap<String, String>,
        _limit: usize,
        _threshold: f32,
    ) -> Result<Vec<VectorSearchResult>, ContextError> {
        Ok(Vec::new())
    }

    async fn delete_knowledge_item(&self, _vector_id: VectorId) -> Result<(), ContextError> {
        Ok(())
    }

    async fn batch_delete(&self, _vector_ids: Vec<VectorId>) -> Result<(), ContextError> {
        Ok(())
    }

    async fn update_metadata(
        &self,
        _vector_id: VectorId,
        _metadata: HashMap<String, Value>,
    ) -> Result<(), ContextError> {
        Ok(())
    }

    async fn get_stats(&self) -> Result<VectorDatabaseStats, ContextError> {
        Ok(VectorDatabaseStats {
            total_vectors: 0,
            collection_size_bytes: 0,
            avg_query_time_ms: 0.0,
        })
    }

    async fn create_index(&self, _field_name: &str) -> Result<(), ContextError> {
        Ok(())
    }

    async fn optimize_collection(&self) -> Result<(), ContextError> {
        Ok(())
    }
}

#[async_trait]
impl super::vector_db_trait::VectorDb for NoOpVectorDatabase {
    async fn initialize(&self) -> Result<(), ContextError> {
        Ok(())
    }
    async fn store_knowledge_item(
        &self,
        _item: &KnowledgeItem,
        _embedding: Vec<f32>,
    ) -> Result<VectorId, ContextError> {
        Ok(VectorId::new())
    }
    async fn store_memory_item(
        &self,
        _agent_id: AgentId,
        _memory: &MemoryItem,
        _embedding: Vec<f32>,
    ) -> Result<VectorId, ContextError> {
        Ok(VectorId::new())
    }
    async fn batch_store(
        &self,
        batch: VectorBatchOperation,
    ) -> Result<Vec<VectorId>, ContextError> {
        Ok(batch.items.iter().map(|_| VectorId::new()).collect())
    }
    async fn search_knowledge_base(
        &self,
        _agent_id: AgentId,
        _query_embedding: Vec<f32>,
        _limit: usize,
    ) -> Result<Vec<KnowledgeItem>, ContextError> {
        Ok(Vec::new())
    }
    async fn semantic_search(
        &self,
        _agent_id: AgentId,
        _query_embedding: Vec<f32>,
        _limit: usize,
        _threshold: f32,
    ) -> Result<Vec<ContextItem>, ContextError> {
        Ok(Vec::new())
    }
    async fn advanced_search(
        &self,
        _agent_id: AgentId,
        _query_embedding: Vec<f32>,
        _filters: HashMap<String, String>,
        _limit: usize,
        _threshold: f32,
    ) -> Result<Vec<VectorSearchResult>, ContextError> {
        Ok(Vec::new())
    }
    async fn delete_knowledge_item(&self, _vector_id: VectorId) -> Result<(), ContextError> {
        Ok(())
    }
    async fn batch_delete(&self, _vector_ids: Vec<VectorId>) -> Result<(), ContextError> {
        Ok(())
    }
    async fn update_metadata(
        &self,
        _vector_id: VectorId,
        _metadata: HashMap<String, Value>,
    ) -> Result<(), ContextError> {
        Ok(())
    }
    async fn get_stats(&self) -> Result<VectorDatabaseStats, ContextError> {
        Ok(VectorDatabaseStats {
            total_vectors: 0,
            collection_size_bytes: 0,
            avg_query_time_ms: 0.0,
        })
    }
    async fn create_index(&self, _field_name: &str) -> Result<(), ContextError> {
        Ok(())
    }
    async fn optimize_collection(&self) -> Result<(), ContextError> {
        Ok(())
    }
    async fn health_check(&self) -> Result<bool, ContextError> {
        Ok(true)
    }
}

/// Embedding service trait for generating vector embeddings
#[async_trait]
pub trait EmbeddingService: Send + Sync {
    /// Generate embedding for text content
    async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>, ContextError>;

    /// Generate embeddings for multiple texts in batch
    async fn generate_batch_embeddings(
        &self,
        texts: Vec<&str>,
    ) -> Result<Vec<Vec<f32>>, ContextError>;

    /// Get the dimension of embeddings produced by this service
    fn embedding_dimension(&self) -> usize;

    /// Get the maximum text length supported
    fn max_text_length(&self) -> usize;
}

/// Mock embedding service for testing and development
pub struct MockEmbeddingService {
    dimension: usize,
}

impl MockEmbeddingService {
    pub fn new(dimension: usize) -> Self {
        Self { dimension }
    }
}

#[async_trait]
impl EmbeddingService for MockEmbeddingService {
    async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>, ContextError> {
        // Generate a deterministic mock embedding based on text content
        let mut embedding = vec![0.0; self.dimension];
        let text_bytes = text.as_bytes();

        for (i, val) in embedding.iter_mut().enumerate() {
            let byte_index = i % text_bytes.len();
            let byte_val = text_bytes.get(byte_index).unwrap_or(&0);
            *val = (*byte_val as f32 / 255.0) * 2.0 - 1.0; // Normalize to [-1, 1]
        }

        // Normalize the vector
        let magnitude: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        if magnitude > 0.0 {
            for val in &mut embedding {
                *val /= magnitude;
            }
        }

        Ok(embedding)
    }

    async fn generate_batch_embeddings(
        &self,
        texts: Vec<&str>,
    ) -> Result<Vec<Vec<f32>>, ContextError> {
        let mut embeddings = Vec::new();
        for text in texts {
            embeddings.push(self.generate_embedding(text).await?);
        }
        Ok(embeddings)
    }

    fn embedding_dimension(&self) -> usize {
        self.dimension
    }

    fn max_text_length(&self) -> usize {
        8192 // Reasonable default for most embedding models
    }
}

/// Simple TF-IDF based embedding service for basic semantic similarity
pub struct TfIdfEmbeddingService {
    dimension: usize,
    vocabulary: Arc<RwLock<HashMap<String, usize>>>,
    idf_scores: Arc<RwLock<HashMap<String, f32>>>,
}

impl TfIdfEmbeddingService {
    pub fn new(dimension: usize) -> Self {
        Self {
            dimension,
            vocabulary: Arc::new(RwLock::new(HashMap::new())),
            idf_scores: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Build vocabulary from a corpus of documents
    pub async fn build_vocabulary(&self, documents: Vec<&str>) -> Result<(), ContextError> {
        let mut vocab = self.vocabulary.write().await;
        let mut doc_frequencies = HashMap::new();
        let total_docs = documents.len() as f32;

        // Build vocabulary and count document frequencies
        for doc in &documents {
            let words: std::collections::HashSet<String> = doc
                .to_lowercase()
                .split_whitespace()
                .map(|s| s.trim_matches(|c: char| !c.is_alphanumeric()))
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect();

            for word in words {
                let vocab_len = vocab.len();
                vocab.entry(word.clone()).or_insert(vocab_len);
                *doc_frequencies.entry(word).or_insert(0) += 1;
            }
        }

        // Calculate IDF scores
        let mut idf_scores = self.idf_scores.write().await;
        for (word, doc_freq) in doc_frequencies {
            let idf = (total_docs / doc_freq as f32).ln();
            idf_scores.insert(word, idf);
        }

        Ok(())
    }

    fn tokenize(&self, text: &str) -> Vec<String> {
        text.to_lowercase()
            .split_whitespace()
            .map(|s| s.trim_matches(|c: char| !c.is_alphanumeric()))
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect()
    }
}

#[async_trait]
impl EmbeddingService for TfIdfEmbeddingService {
    async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>, ContextError> {
        let vocab = self.vocabulary.read().await;
        let idf_scores = self.idf_scores.read().await;

        let mut embedding = vec![0.0; self.dimension];
        let tokens = self.tokenize(text);
        let total_tokens = tokens.len() as f32;

        if total_tokens == 0.0 {
            return Ok(embedding);
        }

        // Count term frequencies
        let mut tf_counts = HashMap::new();
        for token in &tokens {
            *tf_counts.entry(token.clone()).or_insert(0) += 1;
        }

        // Calculate TF-IDF and populate embedding
        for (token, count) in tf_counts {
            if let Some(&vocab_index) = vocab.get(&token) {
                if let Some(&idf) = idf_scores.get(&token) {
                    let tf = count as f32 / total_tokens;
                    let tfidf = tf * idf;

                    // Map to embedding dimension using hash
                    let embedding_index = vocab_index % self.dimension;
                    embedding[embedding_index] += tfidf;
                }
            }
        }

        // Normalize the vector
        let magnitude: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        if magnitude > 0.0 {
            for val in &mut embedding {
                *val /= magnitude;
            }
        }

        Ok(embedding)
    }

    async fn generate_batch_embeddings(
        &self,
        texts: Vec<&str>,
    ) -> Result<Vec<Vec<f32>>, ContextError> {
        let mut embeddings = Vec::new();
        for text in texts {
            embeddings.push(self.generate_embedding(text).await?);
        }
        Ok(embeddings)
    }

    fn embedding_dimension(&self) -> usize {
        self.dimension
    }

    fn max_text_length(&self) -> usize {
        16384 // Larger limit for TF-IDF
    }
}
