//! RAG (Retrieval-Augmented Generation) Example
//!
//! Demonstrates basic RAG implementation including:
//! - RAG engine initialization
//! - Document ingestion to knowledge base
//! - Query processing through the RAG pipeline
//! - Response generation with citations

use std::sync::Arc;
use std::time::{Duration, SystemTime};
use symbiont_runtime::context::manager::{ContextManagerConfig, StandardContextManager};
use symbiont_runtime::rag::*;
use symbiont_runtime::types::AgentId;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    println!("=== Symbiont Agent Runtime - RAG Example ===");

    // Step 1: Initialize RAG Engine
    println!("\n=== Initializing RAG Engine ===");

    let context_manager_config = ContextManagerConfig::default();
    let context_manager = Arc::new(StandardContextManager::new(context_manager_config));
    let rag_engine = StandardRAGEngine::new(context_manager);

    // Configure the RAG engine
    let rag_config = RAGConfig {
        embedding_model: EmbeddingModelConfig {
            model_name: "text-embedding-ada-002".to_string(),
            model_type: EmbeddingModelType::OpenAI,
            dimension: 384,
            max_tokens: 8192,
            batch_size: 100,
        },
        retrieval_config: RetrievalConfig {
            max_documents: 10,
            similarity_threshold: 0.7,
            context_window: 4000,
            enable_hybrid_search: true,
            reranking_enabled: true,
        },
        ranking_config: RankingConfig {
            ranking_algorithm: RankingAlgorithm::Hybrid,
            relevance_weight: 0.4,
            recency_weight: 0.2,
            authority_weight: 0.2,
            diversity_weight: 0.2,
        },
        generation_config: GenerationConfig {
            max_response_length: 2000,
            temperature: 0.7,
            top_p: 0.9,
            enable_citations: true,
            response_format: ResponseFormat::Markdown,
        },
        validation_config: ValidationConfig {
            enable_policy_check: true,
            enable_content_filter: true,
            enable_fact_check: false, // Disabled for demo
            confidence_threshold: 0.8,
        },
    };

    rag_engine.initialize(rag_config).await?;
    println!("✓ RAG engine initialized with configuration");

    // Step 2: Add Documents to Knowledge Base
    println!("\n=== Adding Documents to Knowledge Base ===");

    let documents = vec![
        DocumentInput {
            title: "Machine Learning Fundamentals".to_string(),
            content: "Machine learning is a subset of artificial intelligence that enables computers to learn and improve from experience without being explicitly programmed. It involves algorithms that can identify patterns in data and make predictions or decisions based on those patterns. Common types include supervised learning, unsupervised learning, and reinforcement learning.".to_string(),
            metadata: DocumentMetadata {
                document_type: DocumentType::Text,
                author: Some("AI Research Team".to_string()),
                created_at: SystemTime::now(),
                updated_at: SystemTime::now(),
                language: "en".to_string(),
                domain: "artificial_intelligence".to_string(),
                access_level: AccessLevel::Public,
                tags: vec!["machine_learning".to_string(), "ai".to_string(), "algorithms".to_string()],
                source_url: Some("https://example.com/ml-fundamentals".to_string()),
                file_path: None,
            },
            chunking_strategy: ChunkingStrategy::Semantic { min_size: 100, max_size: 500 },
        },
        DocumentInput {
            title: "Neural Networks Overview".to_string(),
            content: "Neural networks are computing systems inspired by biological neural networks. They consist of interconnected nodes (neurons) organized in layers. Deep neural networks with multiple hidden layers are the foundation of deep learning. They excel at pattern recognition, image classification, natural language processing, and many other AI tasks.".to_string(),
            metadata: DocumentMetadata {
                document_type: DocumentType::Research,
                author: Some("Deep Learning Lab".to_string()),
                created_at: SystemTime::now(),
                updated_at: SystemTime::now(),
                language: "en".to_string(),
                domain: "deep_learning".to_string(),
                access_level: AccessLevel::Public,
                tags: vec!["neural_networks".to_string(), "deep_learning".to_string(), "ai".to_string()],
                source_url: Some("https://example.com/neural-networks".to_string()),
                file_path: None,
            },
            chunking_strategy: ChunkingStrategy::Paragraph,
        },
        DocumentInput {
            title: "RAG Systems Guide".to_string(),
            content: "Retrieval-Augmented Generation (RAG) combines the power of large language models with external knowledge retrieval. RAG systems first retrieve relevant documents from a knowledge base, then use this context to generate more accurate and informed responses. This approach helps reduce hallucinations and provides up-to-date information beyond the model's training data.".to_string(),
            metadata: DocumentMetadata {
                document_type: DocumentType::Manual,
                author: Some("NLP Engineering Team".to_string()),
                created_at: SystemTime::now(),
                updated_at: SystemTime::now(),
                language: "en".to_string(),
                domain: "natural_language_processing".to_string(),
                access_level: AccessLevel::Public,
                tags: vec!["rag".to_string(), "retrieval".to_string(), "generation".to_string(), "llm".to_string()],
                source_url: Some("https://example.com/rag-guide".to_string()),
                file_path: None,
            },
            chunking_strategy: ChunkingStrategy::FixedSize { size: 300, overlap: 50 },
        },
    ];

    let document_ids = rag_engine.ingest_documents(documents).await?;
    println!("✓ Added {} documents to knowledge base", document_ids.len());
    for (i, doc_id) in document_ids.iter().enumerate() {
        println!("  Document {}: {}", i + 1, doc_id.0);
    }

    // Step 3: Perform RAG Queries
    println!("\n=== Performing RAG Queries ===");

    let agent_id = AgentId::new();
    let queries = [
        "What is machine learning?",
        "How do neural networks work?",
        "Explain RAG systems and their benefits",
        "What are the different types of machine learning?",
    ];

    for (i, query) in queries.iter().enumerate() {
        println!("\n--- Query {}: {} ---", i + 1, query);

        // Create RAG request
        let rag_request = RAGRequest {
            agent_id,
            query: query.to_string(),
            preferences: QueryPreferences {
                response_length: ResponseLength::Standard,
                include_citations: true,
                preferred_sources: vec!["research".to_string(), "manual".to_string()],
                response_format: ResponseFormat::Markdown,
                language: "en".to_string(),
            },
            constraints: QueryConstraints {
                max_documents: 5,
                time_limit: Duration::from_secs(30),
                security_level: AccessLevel::Public,
                allowed_sources: vec!["public".to_string()],
                excluded_sources: vec![],
            },
        };

        // Process the query through RAG pipeline
        match rag_engine.process_query(rag_request).await {
            Ok(response) => {
                println!("✓ Query processed successfully");
                println!("Processing time: {:?}", response.processing_time);
                println!("Confidence score: {:.2}", response.confidence_score);
                println!("Sources used: {}", response.sources_used.len());

                println!("\nResponse:");
                println!("{}", response.response.content);

                if !response.sources_used.is_empty() {
                    println!("\nSources:");
                    for (j, citation) in response.sources_used.iter().enumerate() {
                        println!(
                            "  {}. {} (relevance: {:.2})",
                            j + 1,
                            citation.title,
                            citation.relevance_score
                        );
                        if let Some(url) = &citation.url {
                            println!("     URL: {}", url);
                        }
                    }
                }

                if !response.follow_up_suggestions.is_empty() {
                    println!("\nFollow-up suggestions:");
                    for suggestion in &response.follow_up_suggestions {
                        println!("  • {}", suggestion);
                    }
                }
            }
            Err(e) => {
                println!("✗ Query failed: {}", e);
            }
        }
    }

    // Step 4: Demonstrate Individual Pipeline Steps
    println!("\n=== Demonstrating Individual Pipeline Steps ===");

    let demo_query = "What are neural networks?";
    println!("Demo query: {}", demo_query);

    // Step 4a: Query Analysis
    println!("\n--- Step 1: Query Analysis ---");
    let analyzed_query = rag_engine.analyze_query(demo_query, None).await?;
    println!("Original query: {}", analyzed_query.original_query);
    println!("Intent: {:?}", analyzed_query.intent);
    println!("Keywords: {:?}", analyzed_query.keywords);
    println!("Entities: {} found", analyzed_query.entities.len());
    for entity in &analyzed_query.entities {
        println!(
            "  • {} ({:?}, confidence: {:.2})",
            entity.text, entity.entity_type, entity.confidence
        );
    }
    println!("Expanded terms: {:?}", analyzed_query.expanded_terms);

    // Step 4b: Document Retrieval
    println!("\n--- Step 2: Document Retrieval ---");
    let retrieved_docs = rag_engine.retrieve_documents(&analyzed_query).await?;
    println!("Retrieved {} documents", retrieved_docs.len());
    for (i, doc) in retrieved_docs.iter().enumerate() {
        println!(
            "  {}. {} (type: {:?})",
            i + 1,
            doc.title,
            doc.metadata.document_type
        );
    }

    // Step 4c: Document Ranking
    println!("\n--- Step 3: Document Ranking ---");
    let ranked_docs = rag_engine
        .rank_documents(retrieved_docs, &analyzed_query)
        .await?;
    println!("Ranked {} documents by relevance", ranked_docs.len());
    for (i, ranked_doc) in ranked_docs.iter().enumerate() {
        println!(
            "  {}. {} (score: {:.3})",
            i + 1,
            ranked_doc.document.title,
            ranked_doc.relevance_score
        );
        let factors = &ranked_doc.ranking_factors;
        println!(
            "     Semantic: {:.2}, Keywords: {:.2}, Recency: {:.2}, Authority: {:.2}",
            factors.semantic_similarity,
            factors.keyword_match,
            factors.recency_score,
            factors.authority_score
        );
    }

    // Step 4d: Context Augmentation
    println!("\n--- Step 4: Context Augmentation ---");
    let augmented_context = rag_engine
        .augment_context(&analyzed_query, ranked_docs)
        .await?;
    println!("Context summary: {}", augmented_context.context_summary);
    println!("Citations: {}", augmented_context.citations.len());

    // Step 4e: Response Generation
    println!("\n--- Step 5: Response Generation ---");
    let generated_response = rag_engine.generate_response(augmented_context).await?;
    println!(
        "Generated response ({} characters)",
        generated_response.content.len()
    );
    println!("Confidence: {:.2}", generated_response.confidence);
    println!(
        "Sources consulted: {}",
        generated_response.metadata.sources_consulted
    );
    println!(
        "Generation time: {:?}",
        generated_response.metadata.generation_time
    );

    // Step 4f: Response Validation
    println!("\n--- Step 6: Response Validation ---");
    let validation_result = rag_engine
        .validate_response(&generated_response, agent_id)
        .await?;
    println!(
        "Validation result: {}",
        if validation_result.is_valid {
            "✓ Valid"
        } else {
            "✗ Invalid"
        }
    );
    println!(
        "Confidence score: {:.2}",
        validation_result.confidence_score
    );
    println!(
        "Policy violations: {}",
        validation_result.policy_violations.len()
    );
    println!("Content issues: {}", validation_result.content_issues.len());

    // Step 5: Get RAG Engine Statistics
    println!("\n=== RAG Engine Statistics ===");
    let stats = rag_engine.get_stats().await?;
    println!("Total documents: {}", stats.total_documents);
    println!("Total queries: {}", stats.total_queries);
    println!("Average response time: {:?}", stats.avg_response_time);
    println!("Cache hit rate: {:.2}%", stats.cache_hit_rate * 100.0);
    println!(
        "Validation pass rate: {:.2}%",
        stats.validation_pass_rate * 100.0
    );

    if !stats.top_query_types.is_empty() {
        println!("Top query types:");
        for (intent, count) in &stats.top_query_types {
            println!("  {:?}: {} queries", intent, count);
        }
    }

    println!("\n=== RAG Example Complete ===");
    println!("This example demonstrated:");
    println!("✓ RAG engine initialization and configuration");
    println!("✓ Document ingestion with different chunking strategies");
    println!("✓ End-to-end query processing through the RAG pipeline");
    println!("✓ Individual pipeline step execution and analysis");
    println!("✓ Response generation with citations and validation");
    println!("✓ RAG engine statistics and monitoring");

    Ok(())
}
