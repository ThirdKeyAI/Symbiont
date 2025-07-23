//! Context Management Example
//!
//! Demonstrates basic usage of the context management system including:
//! - StandardContextManager initialization
//! - Agent context creation and storage
//! - Memory management (working, short-term, long-term)
//! - Knowledge base operations
//! - Context querying and retrieval
//! - Knowledge sharing between agents

use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;
use symbiont_runtime::context::manager::{
    ContextManager, ContextManagerConfig, StandardContextManager,
};
use symbiont_runtime::context::types::*;
use symbiont_runtime::types::AgentId;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    println!("=== Symbiont Agent Runtime - Context Management Example ===");

    // Step 1: Initialize Context Manager
    println!("\n=== Initializing Context Manager ===");

    let config = ContextManagerConfig {
        max_contexts_in_memory: 100,
        enable_auto_archiving: true,
        max_memory_items_per_agent: 1000,
        max_knowledge_items_per_agent: 500,
        enable_vector_db: false, // Disable for simple example
        ..Default::default()
    };

    let context_manager = Arc::new(StandardContextManager::new(config));
    context_manager.initialize().await?;
    println!("✓ Context manager initialized");

    // Step 2: Create Agent Sessions
    println!("\n=== Creating Agent Sessions ===");

    let agent1_id = AgentId::new();
    let agent2_id = AgentId::new();

    let session1_id = context_manager.create_session(agent1_id).await?;
    let session2_id = context_manager.create_session(agent2_id).await?;

    println!("✓ Created session for Agent 1: {}", session1_id);
    println!("✓ Created session for Agent 2: {}", session2_id);

    // Step 3: Demonstrate Working Memory
    println!("\n=== Working Memory Operations ===");

    // Add variables to working memory
    let working_memory_updates = vec![
        MemoryUpdate {
            operation: UpdateOperation::Add,
            target: MemoryTarget::Working("current_task".to_string()),
            data: json!("Analyzing user preferences"),
        },
        MemoryUpdate {
            operation: UpdateOperation::Add,
            target: MemoryTarget::Working("user_id".to_string()),
            data: json!("user_12345"),
        },
        MemoryUpdate {
            operation: UpdateOperation::Add,
            target: MemoryTarget::Working("session_start".to_string()),
            data: json!(SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)?
                .as_secs()),
        },
    ];

    context_manager
        .update_memory(agent1_id, working_memory_updates)
        .await?;
    println!("✓ Added variables to Agent 1's working memory");

    // Step 4: Add Knowledge to Knowledge Base
    println!("\n=== Knowledge Base Operations ===");

    // Add a fact
    let fact = KnowledgeFact {
        id: KnowledgeId::new(),
        subject: "User preferences".to_string(),
        predicate: "includes".to_string(),
        object: "dark mode interface".to_string(),
        confidence: 0.9,
        source: KnowledgeSource::UserProvided,
        created_at: SystemTime::now(),
        verified: true,
    };

    let fact_id = context_manager
        .add_knowledge(agent1_id, Knowledge::Fact(fact))
        .await?;
    println!("✓ Added fact to Agent 1's knowledge base: {}", fact_id);

    // Add a procedure
    let procedure = Procedure {
        id: KnowledgeId::new(),
        name: "User Onboarding".to_string(),
        description: "Standard process for onboarding new users".to_string(),
        steps: vec![
            ProcedureStep {
                order: 1,
                action: "Collect user preferences".to_string(),
                expected_result: "User profile created".to_string(),
                error_handling: Some("Retry with simplified form".to_string()),
            },
            ProcedureStep {
                order: 2,
                action: "Setup user workspace".to_string(),
                expected_result: "Workspace configured".to_string(),
                error_handling: Some("Use default configuration".to_string()),
            },
        ],
        preconditions: vec!["User account exists".to_string()],
        postconditions: vec!["User can access system".to_string()],
        success_rate: 0.95,
    };

    let procedure_id = context_manager
        .add_knowledge(agent1_id, Knowledge::Procedure(procedure))
        .await?;
    println!(
        "✓ Added procedure to Agent 1's knowledge base: {}",
        procedure_id
    );

    // Add a learned pattern
    let pattern = Pattern {
        id: KnowledgeId::new(),
        name: "User Engagement Pattern".to_string(),
        description: "Users are more active in the morning hours".to_string(),
        conditions: vec![
            "Time between 8:00-11:00 AM".to_string(),
            "Weekday".to_string(),
        ],
        outcomes: vec![
            "Higher response rate".to_string(),
            "More feature usage".to_string(),
        ],
        confidence: 0.85,
        occurrences: 42,
    };

    let pattern_id = context_manager
        .add_knowledge(agent1_id, Knowledge::Pattern(pattern))
        .await?;
    println!(
        "✓ Added pattern to Agent 1's knowledge base: {}",
        pattern_id
    );

    // Step 5: Search Knowledge Base
    println!("\n=== Knowledge Search ===");

    let search_results = context_manager
        .search_knowledge(agent1_id, "user preferences", 5)
        .await?;
    println!(
        "✓ Found {} knowledge items for 'user preferences'",
        search_results.len()
    );

    for (i, item) in search_results.iter().enumerate() {
        println!(
            "  {}. {} (type: {:?}, confidence: {:.2})",
            i + 1,
            item.content,
            item.knowledge_type,
            item.confidence
        );
    }

    // Step 6: Context Querying
    println!("\n=== Context Querying ===");

    let context_query = ContextQuery {
        query_type: QueryType::Semantic,
        search_terms: vec!["user".to_string(), "preferences".to_string()],
        memory_types: vec![MemoryType::Working, MemoryType::Semantic],
        relevance_threshold: 0.5,
        max_results: 10,
        include_embeddings: false,
        ..Default::default()
    };

    let query_results = context_manager
        .query_context(agent1_id, context_query)
        .await?;
    println!("✓ Context query returned {} items", query_results.len());

    for (i, item) in query_results.iter().enumerate() {
        println!(
            "  {}. {} (relevance: {:.2})",
            i + 1,
            item.content,
            item.relevance_score
        );
    }

    // Step 7: Knowledge Sharing
    println!("\n=== Knowledge Sharing ===");

    // Share knowledge from Agent 1 to Agent 2
    context_manager
        .share_knowledge(agent1_id, agent2_id, fact_id, AccessLevel::Public)
        .await?;
    println!("✓ Shared knowledge from Agent 1 to Agent 2");

    // Get shared knowledge available to Agent 2
    let shared_knowledge = context_manager.get_shared_knowledge(agent2_id).await?;
    println!(
        "✓ Agent 2 has access to {} shared knowledge items",
        shared_knowledge.len()
    );

    for (i, shared_ref) in shared_knowledge.iter().enumerate() {
        println!(
            "  {}. Knowledge from Agent {} (trust: {:.2})",
            i + 1,
            shared_ref.source_agent,
            shared_ref.trust_score
        );
    }

    // Step 8: Context Retrieval
    println!("\n=== Context Retrieval ===");

    let retrieved_context = context_manager
        .retrieve_context(agent1_id, Some(session1_id))
        .await?;

    if let Some(context) = retrieved_context {
        println!("✓ Retrieved context for Agent 1");
        println!("  Session ID: {}", context.session_id);
        println!(
            "  Working memory variables: {}",
            context.memory.working_memory.variables.len()
        );
        println!("  Knowledge facts: {}", context.knowledge_base.facts.len());
        println!(
            "  Knowledge procedures: {}",
            context.knowledge_base.procedures.len()
        );
        println!(
            "  Knowledge patterns: {}",
            context.knowledge_base.learned_patterns.len()
        );
        println!("  Created at: {:?}", context.created_at);
        println!("  Updated at: {:?}", context.updated_at);
    } else {
        println!("✗ No context found for Agent 1");
    }

    // Step 9: Context Statistics
    println!("\n=== Context Statistics ===");

    let stats1 = context_manager.get_context_stats(agent1_id).await?;
    println!("✓ Agent 1 Statistics:");
    println!("  Total memory items: {}", stats1.total_memory_items);
    println!("  Total knowledge items: {}", stats1.total_knowledge_items);
    println!("  Total conversations: {}", stats1.total_conversations);
    println!("  Total episodes: {}", stats1.total_episodes);
    println!("  Last activity: {:?}", stats1.last_activity);

    // Step 10: Demonstrate Layered Context
    println!("\n=== Layered Context Example ===");

    // Create a context with different memory layers
    let layered_context = AgentContext {
        agent_id: agent2_id,
        session_id: session2_id,
        memory: HierarchicalMemory {
            working_memory: WorkingMemory {
                variables: {
                    let mut vars = HashMap::new();
                    vars.insert("current_focus".to_string(), json!("learning new concepts"));
                    vars.insert("attention_level".to_string(), json!(0.8));
                    vars
                },
                active_goals: vec![
                    "Understand context management".to_string(),
                    "Learn knowledge sharing".to_string(),
                ],
                current_context: Some("Educational session".to_string()),
                attention_focus: vec!["memory".to_string(), "knowledge".to_string()],
            },
            short_term: vec![
                MemoryItem {
                    id: ContextId::new(),
                    content: "User asked about context management".to_string(),
                    memory_type: MemoryType::Episodic,
                    importance: 0.7,
                    access_count: 1,
                    last_accessed: SystemTime::now(),
                    created_at: SystemTime::now(),
                    embedding: None,
                    metadata: HashMap::new(),
                },
            ],
            long_term: vec![
                MemoryItem {
                    id: ContextId::new(),
                    content: "Context management is crucial for agent intelligence".to_string(),
                    memory_type: MemoryType::Semantic,
                    importance: 0.9,
                    access_count: 5,
                    last_accessed: SystemTime::now(),
                    created_at: SystemTime::now(),
                    embedding: None,
                    metadata: HashMap::new(),
                },
            ],
            episodic_memory: vec![
                Episode {
                    id: ContextId::new(),
                    title: "First Context Management Session".to_string(),
                    description: "Learning about context management features".to_string(),
                    events: vec![
                        EpisodeEvent {
                            action: "Initialized context manager".to_string(),
                            result: "Successfully created agent sessions".to_string(),
                            timestamp: SystemTime::now(),
                            context: HashMap::new(),
                        },
                    ],
                    outcome: Some("Gained understanding of context layers".to_string()),
                    lessons_learned: vec![
                        "Working memory is for immediate processing".to_string(),
                        "Knowledge can be shared between agents".to_string(),
                    ],
                    timestamp: SystemTime::now(),
                    importance: 0.8,
                },
            ],
            semantic_memory: vec![
                SemanticMemoryItem {
                    id: ContextId::new(),
                    concept: "Context Management".to_string(),
                    relationships: vec![
                        ConceptRelationship {
                            relation_type: RelationType::IsA,
                            target_concept: "Memory System".to_string(),
                            strength: 0.9,
                            bidirectional: false,
                        },
                        ConceptRelationship {
                            relation_type: RelationType::Enables,
                            target_concept: "Agent Intelligence".to_string(),
                            strength: 0.8,
                            bidirectional: false,
                        },
                    ],
                    properties: {
                        let mut props = HashMap::new();
                        props.insert("complexity".to_string(), json!("high"));
                        props.insert("importance".to_string(), json!("critical"));
                        props
                    },
                    confidence: 0.85,
                    created_at: SystemTime::now(),
                    updated_at: SystemTime::now(),
                },
            ],
        },
        knowledge_base: KnowledgeBase::default(),
        conversation_history: vec![
            ConversationItem {
                id: ContextId::new(),
                role: ConversationRole::User,
                content: "How does context management work?".to_string(),
                timestamp: SystemTime::now(),
                context_used: vec![],
                knowledge_used: vec![],
            },
            ConversationItem {
                id: ContextId::new(),
                role: ConversationRole::Agent,
                content: "Context management provides persistent memory and knowledge capabilities for agents.".to_string(),
                timestamp: SystemTime::now(),
                context_used: vec![],
                knowledge_used: vec![],
            },
        ],
        metadata: {
            let mut meta = HashMap::new();
            meta.insert("session_type".to_string(), "educational".to_string());
            meta.insert("complexity_level".to_string(), "intermediate".to_string());
            meta
        },
        created_at: SystemTime::now(),
        updated_at: SystemTime::now(),
        retention_policy: RetentionPolicy::default(),
    };

    let layered_context_id = context_manager
        .store_context(agent2_id, layered_context)
        .await?;
    println!(
        "✓ Stored layered context for Agent 2: {}",
        layered_context_id
    );

    // Retrieve and display the layered context
    let retrieved_layered = context_manager
        .retrieve_context(agent2_id, Some(session2_id))
        .await?;
    if let Some(context) = retrieved_layered {
        println!("✓ Retrieved layered context:");
        println!(
            "  Working memory goals: {}",
            context.memory.working_memory.active_goals.len()
        );
        println!("  Short-term memories: {}", context.memory.short_term.len());
        println!("  Long-term memories: {}", context.memory.long_term.len());
        println!("  Episodes: {}", context.memory.episodic_memory.len());
        println!(
            "  Semantic concepts: {}",
            context.memory.semantic_memory.len()
        );
        println!(
            "  Conversation history: {}",
            context.conversation_history.len()
        );

        // Show semantic relationships
        if let Some(semantic_item) = context.memory.semantic_memory.first() {
            println!(
                "  Concept '{}' has {} relationships",
                semantic_item.concept,
                semantic_item.relationships.len()
            );
            for rel in &semantic_item.relationships {
                println!(
                    "    {} -> {} (strength: {:.2})",
                    rel.relation_type.to_string(),
                    rel.target_concept,
                    rel.strength
                );
            }
        }
    }

    // Step 11: Cleanup
    println!("\n=== Cleanup ===");
    context_manager.shutdown().await?;
    println!("✓ Context manager shutdown complete");

    println!("\n=== Context Management Example Complete ===");
    println!("This example demonstrated:");
    println!("✓ Context manager initialization and configuration");
    println!("✓ Agent session creation and management");
    println!("✓ Working memory operations with variables and goals");
    println!("✓ Knowledge base operations (facts, procedures, patterns)");
    println!("✓ Knowledge search and retrieval");
    println!("✓ Context querying with semantic search");
    println!("✓ Knowledge sharing between agents");
    println!("✓ Layered memory hierarchy (working, short-term, long-term, episodic, semantic)");
    println!("✓ Context statistics and monitoring");
    println!("✓ Proper resource cleanup");

    Ok(())
}

// Helper trait for displaying relation types
trait RelationTypeDisplay {
    fn to_string(&self) -> String;
}

impl RelationTypeDisplay for RelationType {
    fn to_string(&self) -> String {
        match self {
            RelationType::IsA => "is-a".to_string(),
            RelationType::PartOf => "part-of".to_string(),
            RelationType::RelatedTo => "related-to".to_string(),
            RelationType::Causes => "causes".to_string(),
            RelationType::Enables => "enables".to_string(),
            RelationType::Requires => "requires".to_string(),
            RelationType::Similar => "similar".to_string(),
            RelationType::Opposite => "opposite".to_string(),
            RelationType::Custom(s) => s.clone(),
        }
    }
}
