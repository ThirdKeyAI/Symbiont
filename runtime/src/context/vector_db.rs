//! Vector Database integration for Qdrant

use async_trait::async_trait;
use qdrant_client::Qdrant;
use qdrant_client::config::QdrantConfig as ClientConfig;
use qdrant_client::qdrant::{
    CreateCollection, Distance, PointStruct, SearchPoints, UpsertPoints, VectorParams,
    VectorsConfig, DeletePoints, PointId, Filter, Condition, FieldCondition, Match, 
    Value as QdrantValue, WithPayloadSelector, PointsSelector, PointsIdsList
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::types::*;
use crate::types::AgentId;

/// Convert Qdrant errors to ContextError with specific mappings
fn map_qdrant_error(error: qdrant_client::QdrantError) -> ContextError {
    match error {
        qdrant_client::QdrantError::ResponseError { status, .. } => {
            let status_code = status.code() as u16;
            match status_code {
                404 => ContextError::StorageError { 
                    reason: "Collection or point not found in Qdrant".to_string() 
                },
                401 | 403 => ContextError::AccessDenied { 
                    reason: "Authentication failed for Qdrant database".to_string() 
                },
                400 => ContextError::InvalidOperation { 
                    reason: "Invalid request to Qdrant database".to_string() 
                },
                500..=599 => ContextError::StorageError { 
                    reason: format!("Qdrant server error: {}", status) 
                },
                _ => ContextError::StorageError { 
                    reason: format!("Qdrant API error: {}", status) 
                }
            }
        }
        qdrant_client::QdrantError::ConversionError { .. } => {
            ContextError::InvalidOperation { 
                reason: "Data conversion error with Qdrant".to_string() 
            }
        }
        _ => {
            ContextError::StorageError { 
                reason: format!("Qdrant database error: {}", error) 
            }
        }
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
            url: "http://localhost:6334".to_string(),
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
    async fn store_knowledge_item(&self, item: &KnowledgeItem, embedding: Vec<f32>) -> Result<VectorId, ContextError>;
    
    /// Search for similar knowledge items
    async fn search_knowledge_base(&self, agent_id: AgentId, query_embedding: Vec<f32>, limit: usize) -> Result<Vec<KnowledgeItem>, ContextError>;
    
    /// Perform semantic search with text query
    async fn semantic_search(&self, agent_id: AgentId, query_embedding: Vec<f32>, limit: usize, threshold: f32) -> Result<Vec<ContextItem>, ContextError>;
    
    /// Delete knowledge item by ID
    async fn delete_knowledge_item(&self, vector_id: VectorId) -> Result<(), ContextError>;
    
    /// Update knowledge item metadata
    async fn update_metadata(&self, vector_id: VectorId, metadata: HashMap<String, Value>) -> Result<(), ContextError>;
    
    /// Get collection statistics
    async fn get_stats(&self) -> Result<VectorDatabaseStats, ContextError>;
}

/// Statistics for vector database operations
#[derive(Debug, Clone)]
pub struct VectorDatabaseStats {
    pub total_vectors: usize,
    pub collection_size_bytes: usize,
    pub avg_query_time_ms: f32,
}

/// Qdrant client wrapper implementation
pub struct QdrantClientWrapper {
    client: Arc<RwLock<Option<Arc<Qdrant>>>>,
    config: QdrantConfig,
}

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
            
            let client = Qdrant::new(client_config)
                .map_err(map_qdrant_error)?;
            
            let client_arc = Arc::new(client);
            let mut client_guard = self.client.write().await;
            *client_guard = Some(Arc::clone(&client_arc));
            
            Ok(client_arc)
        }
    }

    /// Convert KnowledgeItem to Qdrant metadata
    fn knowledge_item_to_metadata(&self, item: &KnowledgeItem, agent_id: AgentId) -> HashMap<String, QdrantValue> {
        let mut metadata = HashMap::new();
        
        metadata.insert("agent_id".to_string(), QdrantValue::from(agent_id.to_string()));
        metadata.insert("knowledge_id".to_string(), QdrantValue::from(item.id.to_string()));
        metadata.insert("content".to_string(), QdrantValue::from(item.content.clone()));
        metadata.insert("knowledge_type".to_string(), QdrantValue::from(format!("{:?}", item.knowledge_type)));
        metadata.insert("confidence".to_string(), QdrantValue::from(item.confidence as f64));
        metadata.insert("relevance_score".to_string(), QdrantValue::from(item.relevance_score as f64));
        metadata.insert("source".to_string(), QdrantValue::from(format!("{:?}", item.source)));
        metadata.insert("created_at".to_string(), QdrantValue::from(
            item.created_at.duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default().as_secs() as i64
        ));
        
        metadata
    }

    /// Convert Qdrant point to KnowledgeItem
    fn point_to_knowledge_item(&self, point: &qdrant_client::qdrant::ScoredPoint) -> Result<KnowledgeItem, ContextError> {
        let payload = &point.payload;

        let knowledge_id_str = payload.get("knowledge_id")
            .and_then(|v| self.extract_string_value(v))
            .ok_or_else(|| ContextError::StorageError {
                reason: "Missing knowledge_id in payload".to_string()
            })?;

        let knowledge_id = KnowledgeId(uuid::Uuid::parse_str(&knowledge_id_str)
            .map_err(|e| ContextError::StorageError {
                reason: format!("Invalid knowledge_id UUID: {}", e)
            })?);

        let content = payload.get("content")
            .and_then(|v| self.extract_string_value(v))
            .unwrap_or_default();

        let knowledge_type_str = payload.get("knowledge_type")
            .and_then(|v| self.extract_string_value(v))
            .unwrap_or_else(|| "Fact".to_string());

        let knowledge_type = match knowledge_type_str.as_str() {
            "Fact" => KnowledgeType::Fact,
            "Procedure" => KnowledgeType::Procedure,
            "Pattern" => KnowledgeType::Pattern,
            "Shared" => KnowledgeType::Shared,
            _ => KnowledgeType::Fact,
        };

        let confidence = payload.get("confidence")
            .and_then(|v| self.extract_f64_value(v))
            .unwrap_or(0.0) as f32;

        let relevance_score = point.score;

        let source = KnowledgeSource::Learning; // Default, could be parsed from payload

        let created_at = payload.get("created_at")
            .and_then(|v| self.extract_i64_value(v))
            .map(|timestamp| std::time::UNIX_EPOCH + std::time::Duration::from_secs(timestamp as u64))
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
            QdrantValue { kind: Some(qdrant_client::qdrant::value::Kind::StringValue(s)) } => Some(s.clone()),
            _ => None,
        }
    }

    /// Extract f64 value from QdrantValue
    fn extract_f64_value(&self, value: &QdrantValue) -> Option<f64> {
        match value {
            QdrantValue { kind: Some(qdrant_client::qdrant::value::Kind::DoubleValue(d)) } => Some(*d),
            QdrantValue { kind: Some(qdrant_client::qdrant::value::Kind::IntegerValue(i)) } => Some(*i as f64),
            _ => None,
        }
    }

    /// Extract i64 value from QdrantValue
    fn extract_i64_value(&self, value: &QdrantValue) -> Option<i64> {
        match value {
            QdrantValue { kind: Some(qdrant_client::qdrant::value::Kind::IntegerValue(i)) } => Some(*i),
            QdrantValue { kind: Some(qdrant_client::qdrant::value::Kind::DoubleValue(d)) } => Some(*d as i64),
            _ => None,
        }
    }
}

#[async_trait]
impl VectorDatabase for QdrantClientWrapper {
    async fn initialize(&self) -> Result<(), ContextError> {
        let client = self.get_client().await?;
        
        // Check if collection exists
        let collections = client.list_collections().await
            .map_err(map_qdrant_error)?;

        let collection_exists = collections.collections.iter()
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
                    }
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

            client.create_collection(create_collection).await
                .map_err(map_qdrant_error)?;
        }

        Ok(())
    }

    async fn store_knowledge_item(&self, item: &KnowledgeItem, embedding: Vec<f32>) -> Result<VectorId, ContextError> {
        let client = self.get_client().await?;
        let vector_id = VectorId::new();
        
        // Create point for Qdrant
        let point = PointStruct::new(
            vector_id.0.to_string(),
            embedding,
            self.knowledge_item_to_metadata(item, AgentId::new()), // Using new AgentId as placeholder
        );

        let upsert_points = UpsertPoints {
            collection_name: self.config.collection_name.clone(),
            wait: Some(true),
            points: vec![point],
            ordering: None,
            shard_key_selector: None,
        };

        client.upsert_points(upsert_points).await
            .map_err(map_qdrant_error)?;

        Ok(vector_id)
    }

    async fn search_knowledge_base(&self, agent_id: AgentId, query_embedding: Vec<f32>, limit: usize) -> Result<Vec<KnowledgeItem>, ContextError> {
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
                                agent_id.to_string()
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
                    }
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
                selector_options: Some(qdrant_client::qdrant::with_payload_selector::SelectorOptions::Enable(true)),
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

        let search_result = client.search_points(search_points).await
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

    async fn semantic_search(&self, agent_id: AgentId, query_embedding: Vec<f32>, limit: usize, threshold: f32) -> Result<Vec<ContextItem>, ContextError> {
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
                                agent_id.to_string()
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
                    }
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
                selector_options: Some(qdrant_client::qdrant::with_payload_selector::SelectorOptions::Enable(true)),
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

        let search_result = client.search_points(search_points).await
            .map_err(map_qdrant_error)?;

        let mut context_items = Vec::new();
        for point in search_result.result {
            let payload = &point.payload;
            let context_id_str = payload.get("context_id")
                .and_then(|v| self.extract_string_value(v))
                .unwrap_or_default();

            let context_id = ContextId(uuid::Uuid::parse_str(&context_id_str)
                .unwrap_or_else(|_| uuid::Uuid::new_v4()));

            let content = payload.get("content")
                .and_then(|v| self.extract_string_value(v))
                .unwrap_or_default();

            let item_type = ContextItemType::Memory(MemoryType::Semantic); // Default

            let timestamp = payload.get("timestamp")
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
                points_selector_one_of: Some(qdrant_client::qdrant::points_selector::PointsSelectorOneOf::Points(
                    PointsIdsList {
                        ids: vec![PointId::from(vector_id.0.to_string())],
                    }
                )),
            }),
            ordering: None,
            shard_key_selector: None,
        };

        client.delete_points(delete_points).await
            .map_err(map_qdrant_error)?;

        Ok(())
    }

    async fn update_metadata(&self, _vector_id: VectorId, _metadata: HashMap<String, Value>) -> Result<(), ContextError> {
        // Qdrant doesn't support direct metadata updates, would need to re-upsert the point
        // For now, return an error indicating this operation is not supported
        Err(ContextError::InvalidOperation {
            reason: "Direct metadata updates not supported, use store_knowledge_item to update".to_string()
        })
    }

    async fn get_stats(&self) -> Result<VectorDatabaseStats, ContextError> {
        let client = self.get_client().await?;

        let collection_info = client.collection_info(&self.config.collection_name).await
            .map_err(map_qdrant_error)?;

        Ok(VectorDatabaseStats {
            total_vectors: collection_info.result.map(|r| r.points_count.unwrap_or(0) as usize).unwrap_or(0),
            collection_size_bytes: 0, // Not directly available from Qdrant API
            avg_query_time_ms: 0.0,   // Would need to track this separately
        })
    }
}

/// Mock embedding service for testing
pub struct MockEmbeddingService {
    dimension: usize,
}

impl MockEmbeddingService {
    pub fn new(dimension: usize) -> Self {
        Self { dimension }
    }

    /// Generate mock embeddings for text
    pub async fn generate_embedding(&self, _text: &str) -> Result<Vec<f32>, ContextError> {
        // Generate a simple mock embedding
        let mut embedding = vec![0.0; self.dimension];
        for (i, val) in embedding.iter_mut().enumerate() {
            *val = (i as f32 * 0.1) % 1.0;
        }
        Ok(embedding)
    }
}