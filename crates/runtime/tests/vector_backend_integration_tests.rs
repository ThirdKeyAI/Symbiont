//! Integration tests for vector backends.
//!
//! Tests both LanceDB (always) and Qdrant (when feature enabled + server available).

use std::sync::Arc;
use symbi_runtime::context::types::{KnowledgeId, KnowledgeItem, KnowledgeSource, KnowledgeType};
use symbi_runtime::context::{create_vector_backend, LanceDbConfig, VectorBackendConfig, VectorDb};
use symbi_runtime::types::AgentId;
use tempfile::TempDir;

fn make_test_item(content: &str) -> KnowledgeItem {
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

async fn run_store_and_search_suite(backend: Arc<dyn VectorDb>) {
    backend.initialize().await.unwrap();

    // Store
    let item = make_test_item("The quick brown fox");
    let id = backend
        .store_knowledge_item(&item, vec![1.0, 0.0, 0.0, 0.0])
        .await
        .unwrap();

    // Stats
    let stats = backend.get_stats().await.unwrap();
    assert!(stats.total_vectors >= 1);

    // Search
    let agent_id = AgentId::new();
    let results = backend
        .search_knowledge_base(agent_id, vec![0.9, 0.1, 0.0, 0.0], 5)
        .await
        .unwrap();
    assert!(!results.is_empty());

    // Delete
    backend.delete_knowledge_item(id).await.unwrap();

    // Health
    assert!(backend.health_check().await.unwrap());
}

#[tokio::test]
async fn test_lancedb_backend_integration() {
    let tmp = TempDir::new().unwrap();
    let config = VectorBackendConfig::LanceDb(LanceDbConfig {
        data_path: tmp.path().to_path_buf(),
        collection_name: "integration_test".to_string(),
        vector_dimension: 4,
        ..Default::default()
    });
    let backend = create_vector_backend(config).await.unwrap();
    run_store_and_search_suite(backend).await;
}

#[cfg(feature = "vector-qdrant")]
#[tokio::test]
#[ignore] // Requires running Qdrant: docker run -p 6333:6333 qdrant/qdrant
async fn test_qdrant_backend_integration() {
    use symbi_runtime::context::QdrantConfig;
    let config = VectorBackendConfig::Qdrant(QdrantConfig {
        url: "http://localhost:6333".to_string(),
        collection_name: "integration_test".to_string(),
        vector_dimension: 4,
        ..Default::default()
    });
    let backend = create_vector_backend(config).await.unwrap();
    run_store_and_search_suite(backend).await;
}
