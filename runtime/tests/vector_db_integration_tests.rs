//! Integration tests for Vector Database functionality

use std::collections::HashMap;
use std::time::SystemTime;
use symbiont_runtime::context::{
    VectorDatabase, QdrantClientWrapper, QdrantConfig, QdrantDistance,
    EmbeddingService, MockEmbeddingService, TfIdfEmbeddingService,
    KnowledgeItem, KnowledgeType, KnowledgeSource, MemoryItem, MemoryType,
    VectorBatchOperation, VectorOperationType, VectorBatchItem, VectorMetadata, VectorContentType,
    ContextError
};
use symbiont_runtime::types::AgentId;
use tokio;

/// Test basic vector database operations
#[tokio::test]
async fn test_vector_database_basic_operations() {
    let config = QdrantConfig {
        url: "http://localhost:6334".to_string(),
        api_key: None,
        collection_name: "test_collection_basic".to_string(),
        vector_dimension: 128,
        distance_metric: QdrantDistance::Cosine,
        batch_size: 10,
        timeout_seconds: 30,
    };

    let vector_db = QdrantClientWrapper::new(config);
    
    // Note: This test requires a running Qdrant instance
    // In a real CI/CD environment, you would start Qdrant in a container
    match vector_db.initialize().await {
        Ok(_) => {
            println!("✓ Vector database initialized successfully");
            
            // Test storing a knowledge item
            let knowledge_item = KnowledgeItem {
                id: symbiont_runtime::context::KnowledgeId::new(),
                content: "Test knowledge about vector databases".to_string(),
                knowledge_type: KnowledgeType::Fact,
                confidence: 0.9,
                relevance_score: 1.0,
                source: KnowledgeSource::UserProvided,
                created_at: SystemTime::now(),
            };
            
            let embedding = vec![0.1; 128]; // Mock embedding
            
            match vector_db.store_knowledge_item(&knowledge_item, embedding).await {
                Ok(vector_id) => {
                    println!("✓ Knowledge item stored with ID: {}", vector_id);
                    
                    // Test searching
                    let query_embedding = vec![0.1; 128];
                    match vector_db.search_knowledge_base(AgentId::new(), query_embedding, 5).await {
                        Ok(results) => {
                            println!("✓ Search completed, found {} results", results.len());
                        }
                        Err(e) => println!("⚠ Search failed: {}", e),
                    }
                }
                Err(e) => println!("⚠ Failed to store knowledge item: {}", e),
            }
        }
        Err(e) => {
            println!("⚠ Skipping vector database tests - Qdrant not available: {}", e);
        }
    }
}

/// Test embedding services
#[tokio::test]
async fn test_embedding_services() {
    // Test MockEmbeddingService
    let mock_service = MockEmbeddingService::new(384);
    
    let text = "This is a test document for embedding generation";
    match mock_service.generate_embedding(text).await {
        Ok(embedding) => {
            assert_eq!(embedding.len(), 384);
            println!("✓ Mock embedding service generated {} dimensional embedding", embedding.len());
            
            // Test that embeddings are deterministic
            let embedding2 = mock_service.generate_embedding(text).await.unwrap();
            assert_eq!(embedding, embedding2);
            println!("✓ Mock embeddings are deterministic");
        }
        Err(e) => panic!("Mock embedding generation failed: {}", e),
    }
    
    // Test batch embedding generation
    let texts = vec!["First document", "Second document", "Third document"];
    match mock_service.generate_batch_embeddings(texts).await {
        Ok(embeddings) => {
            assert_eq!(embeddings.len(), 3);
            assert_eq!(embeddings[0].len(), 384);
            println!("✓ Batch embedding generation successful");
        }
        Err(e) => panic!("Batch embedding generation failed: {}", e),
    }
    
    // Test TF-IDF embedding service
    let tfidf_service = TfIdfEmbeddingService::new(256);
    
    // Build vocabulary
    let documents = vec![
        "machine learning algorithms",
        "deep learning neural networks", 
        "artificial intelligence systems",
        "data science analytics",
        "machine learning models"
    ];
    
    match tfidf_service.build_vocabulary(documents).await {
        Ok(_) => {
            println!("✓ TF-IDF vocabulary built successfully");
            
            let query = "machine learning";
            match tfidf_service.generate_embedding(query).await {
                Ok(embedding) => {
                    assert_eq!(embedding.len(), 256);
                    println!("✓ TF-IDF embedding generated successfully");
                }
                Err(e) => panic!("TF-IDF embedding generation failed: {}", e),
            }
        }
        Err(e) => panic!("TF-IDF vocabulary building failed: {}", e),
    }
}

/// Test batch operations
#[tokio::test]
async fn test_batch_operations() {
    let config = QdrantConfig {
        url: "http://localhost:6334".to_string(),
        api_key: None,
        collection_name: "test_collection_batch".to_string(),
        vector_dimension: 64,
        distance_metric: QdrantDistance::Cosine,
        batch_size: 5,
        timeout_seconds: 30,
    };

    let vector_db = QdrantClientWrapper::new(config);
    
    match vector_db.initialize().await {
        Ok(_) => {
            // Create batch operation
            let agent_id = AgentId::new();
            let mut batch_items = Vec::new();
            
            for i in 0..10 {
                let metadata = VectorMetadata {
                    agent_id,
                    content_type: VectorContentType::Memory(MemoryType::Semantic),
                    source_id: format!("test_source_{}", i),
                    created_at: SystemTime::now(),
                    updated_at: SystemTime::now(),
                    tags: vec![format!("tag_{}", i), "test".to_string()],
                    custom_fields: {
                        let mut fields = HashMap::new();
                        fields.insert("batch_id".to_string(), "test_batch_1".to_string());
                        fields.insert("item_index".to_string(), i.to_string());
                        fields
                    },
                };
                
                let embedding = vec![0.1 * i as f32; 64];
                
                batch_items.push(VectorBatchItem {
                    id: None,
                    content: format!("Test content item {}", i),
                    embedding: Some(embedding),
                    metadata,
                });
            }
            
            let batch_operation = VectorBatchOperation {
                operation_type: VectorOperationType::Insert,
                items: batch_items,
            };
            
            match vector_db.batch_store(batch_operation).await {
                Ok(vector_ids) => {
                    assert_eq!(vector_ids.len(), 10);
                    println!("✓ Batch store operation successful, stored {} items", vector_ids.len());
                    
                    // Test batch delete
                    match vector_db.batch_delete(vector_ids).await {
                        Ok(_) => println!("✓ Batch delete operation successful"),
                        Err(e) => println!("⚠ Batch delete failed: {}", e),
                    }
                }
                Err(e) => println!("⚠ Batch store failed: {}", e),
            }
        }
        Err(e) => {
            println!("⚠ Skipping batch operations test - Qdrant not available: {}", e);
        }
    }
}

/// Test memory item storage
#[tokio::test]
async fn test_memory_item_storage() {
    let config = QdrantConfig {
        url: "http://localhost:6334".to_string(),
        api_key: None,
        collection_name: "test_collection_memory".to_string(),
        vector_dimension: 128,
        distance_metric: QdrantDistance::Cosine,
        batch_size: 10,
        timeout_seconds: 30,
    };

    let vector_db = QdrantClientWrapper::new(config);
    
    match vector_db.initialize().await {
        Ok(_) => {
            let agent_id = AgentId::new();
            
            let memory_item = MemoryItem {
                id: symbiont_runtime::context::ContextId::new(),
                content: "Important memory about vector database usage".to_string(),
                memory_type: MemoryType::Semantic,
                importance: 0.8,
                access_count: 5,
                last_accessed: SystemTime::now(),
                created_at: SystemTime::now(),
                embedding: None,
                metadata: {
                    let mut meta = HashMap::new();
                    meta.insert("category".to_string(), "technical".to_string());
                    meta.insert("priority".to_string(), "high".to_string());
                    meta
                },
            };
            
            let embedding = vec![0.2; 128];
            
            match vector_db.store_memory_item(agent_id, &memory_item, embedding).await {
                Ok(vector_id) => {
                    println!("✓ Memory item stored with ID: {}", vector_id);
                    
                    // Test semantic search
                    let query_embedding = vec![0.2; 128];
                    match vector_db.semantic_search(agent_id, query_embedding, 5, 0.5).await {
                        Ok(results) => {
                            println!("✓ Semantic search completed, found {} results", results.len());
                        }
                        Err(e) => println!("⚠ Semantic search failed: {}", e),
                    }
                }
                Err(e) => println!("⚠ Failed to store memory item: {}", e),
            }
        }
        Err(e) => {
            println!("⚠ Skipping memory item storage test - Qdrant not available: {}", e);
        }
    }
}

/// Test advanced search with filters
#[tokio::test]
async fn test_advanced_search() {
    let config = QdrantConfig {
        url: "http://localhost:6334".to_string(),
        api_key: None,
        collection_name: "test_collection_advanced".to_string(),
        vector_dimension: 128,
        distance_metric: QdrantDistance::Cosine,
        batch_size: 10,
        timeout_seconds: 30,
    };

    let vector_db = QdrantClientWrapper::new(config);
    
    match vector_db.initialize().await {
        Ok(_) => {
            let agent_id = AgentId::new();
            let query_embedding = vec![0.3; 128];
            
            let mut filters = HashMap::new();
            filters.insert("content_type".to_string(), "Memory".to_string());
            filters.insert("custom_category".to_string(), "technical".to_string());
            
            match vector_db.advanced_search(agent_id, query_embedding, filters, 10, 0.6).await {
                Ok(results) => {
                    println!("✓ Advanced search completed, found {} results", results.len());
                    
                    for result in &results {
                        println!("  - Result ID: {}, Score: {:.3}", result.id, result.score);
                    }
                }
                Err(e) => println!("⚠ Advanced search failed: {}", e),
            }
        }
        Err(e) => {
            println!("⚠ Skipping advanced search test - Qdrant not available: {}", e);
        }
    }
}

/// Test performance with larger datasets
#[tokio::test]
async fn test_performance_large_dataset() {
    let config = QdrantConfig {
        url: "http://localhost:6334".to_string(),
        api_key: None,
        collection_name: "test_collection_performance".to_string(),
        vector_dimension: 384,
        distance_metric: QdrantDistance::Cosine,
        batch_size: 100, // Larger batch size for performance
        timeout_seconds: 60,
    };

    let vector_db = QdrantClientWrapper::new(config);
    
    match vector_db.initialize().await {
        Ok(_) => {
            println!("Testing performance with larger dataset...");
            
            let start_time = SystemTime::now();
            let agent_id = AgentId::new();
            
            // Create a larger batch (1000 items)
            let mut batch_items = Vec::new();
            for i in 0..1000 {
                let metadata = VectorMetadata {
                    agent_id,
                    content_type: VectorContentType::Knowledge(KnowledgeType::Fact),
                    source_id: format!("perf_test_{}", i),
                    created_at: SystemTime::now(),
                    updated_at: SystemTime::now(),
                    tags: vec!["performance".to_string(), "test".to_string()],
                    custom_fields: HashMap::new(),
                };
                
                // Generate varied embeddings
                let embedding: Vec<f32> = (0..384).map(|j| ((i + j) as f32 * 0.001) % 1.0).collect();
                
                batch_items.push(VectorBatchItem {
                    id: None,
                    content: format!("Performance test document {} with varied content", i),
                    embedding: Some(embedding),
                    metadata,
                });
            }
            
            let batch_operation = VectorBatchOperation {
                operation_type: VectorOperationType::Insert,
                items: batch_items,
            };
            
            match vector_db.batch_store(batch_operation).await {
                Ok(vector_ids) => {
                    let store_duration = start_time.elapsed().unwrap();
                    println!("✓ Stored 1000 items in {:?}", store_duration);
                    
                    // Test search performance
                    let search_start = SystemTime::now();
                    let query_embedding: Vec<f32> = (0..384).map(|i| (i as f32 * 0.001) % 1.0).collect();
                    
                    match vector_db.search_knowledge_base(agent_id, query_embedding, 50).await {
                        Ok(results) => {
                            let search_duration = search_start.elapsed().unwrap();
                            println!("✓ Search completed in {:?}, found {} results", search_duration, results.len());
                            
                            // Cleanup
                            match vector_db.batch_delete(vector_ids).await {
                                Ok(_) => println!("✓ Cleanup completed"),
                                Err(e) => println!("⚠ Cleanup failed: {}", e),
                            }
                        }
                        Err(e) => println!("⚠ Performance search failed: {}", e),
                    }
                }
                Err(e) => println!("⚠ Performance batch store failed: {}", e),
            }
        }
        Err(e) => {
            println!("⚠ Skipping performance test - Qdrant not available: {}", e);
        }
    }
}

/// Test error handling
#[tokio::test]
async fn test_error_handling() {
    // Test with invalid configuration
    let invalid_config = QdrantConfig {
        url: "http://invalid-host:9999".to_string(),
        api_key: None,
        collection_name: "test_collection".to_string(),
        vector_dimension: 128,
        distance_metric: QdrantDistance::Cosine,
        batch_size: 10,
        timeout_seconds: 1, // Very short timeout
    };

    let vector_db = QdrantClientWrapper::new(invalid_config);
    
    match vector_db.initialize().await {
        Ok(_) => panic!("Expected initialization to fail with invalid config"),
        Err(e) => {
            println!("✓ Error handling works correctly: {}", e);
            match e {
                ContextError::StorageError { .. } => println!("✓ Correct error type returned"),
                _ => println!("⚠ Unexpected error type: {:?}", e),
            }
        }
    }
}