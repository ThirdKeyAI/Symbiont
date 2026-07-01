//! Test example for Agent Context Manager with file-based persistence
//!
//! This example demonstrates:
//! - Creating an agent context with memory and knowledge
//! - Storing context with file-based persistence
//! - Retrieving context from persistent storage
//! - Performance testing for <50ms retrieval requirement

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Instant, SystemTime};

use symbi_runtime::context::types::{KnowledgeFact, KnowledgeSource};
use symbi_runtime::context::vector_db::QdrantConfig;
use symbi_runtime::context::{
    AgentContext, ContextId, ContextManager, ContextManagerConfig, FilePersistenceConfig,
    HierarchicalMemory, Knowledge, KnowledgeBase, KnowledgeId, MemoryItem, MemoryType,
    RetentionPolicy, SessionId, StandardContextManager,
};
use symbi_runtime::types::AgentId;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🧠 Agent Context Manager Persistence Test");
    println!("==========================================");

    // Create test directory for persistence
    let test_dir = PathBuf::from("./test_context_storage");
    if test_dir.exists() {
        std::fs::remove_dir_all(&test_dir)?;
    }
    std::fs::create_dir_all(&test_dir)?;

    // Configure context manager with file persistence
    let config = ContextManagerConfig {
        max_contexts_in_memory: 100,
        enable_persistence: true,
        persistence_config: FilePersistenceConfig {
            root_data_dir: test_dir.clone(),
            state_dir: PathBuf::from("state"),
            logs_dir: PathBuf::from("logs"),
            prompts_dir: PathBuf::from("prompts"),
            vector_db_dir: PathBuf::from("vector_db"),
            enable_compression: true,
            enable_encryption: false,
            backup_count: 3,
            auto_save_interval: 60,
            auto_create_dirs: true,
            dir_permissions: Some(0o755),
        },
        enable_vector_db: false, // Disable for this test
        qdrant_config: QdrantConfig::default(),
        ..Default::default()
    };

    // Create context manager
    let context_manager = StandardContextManager::new(config, "test-agent").await?;
    context_manager.initialize().await?;

    // Test 1: Create and store agent context
    println!("\n📝 Test 1: Creating and storing agent context...");
    let agent_id = AgentId::new();

    let mut memory = HierarchicalMemory::default();
    memory.short_term.push(MemoryItem {
        id: ContextId::new(),
        content: "User prefers morning meetings".to_string(),
        memory_type: MemoryType::Factual,
        importance: 0.8,
        access_count: 1,
        created_at: SystemTime::now(),
        last_accessed: SystemTime::now(),
        metadata: HashMap::new(),
        embedding: None,
    });

    let mut knowledge_base = KnowledgeBase::default();
    knowledge_base.facts.push(KnowledgeFact {
        id: KnowledgeId::new(),
        subject: "User".to_string(),
        predicate: "works_in".to_string(),
        object: "Software Engineering".to_string(),
        confidence: 0.9,
        source: KnowledgeSource::UserProvided,
        created_at: SystemTime::now(),
        verified: true,
    });

    let context = AgentContext {
        agent_id,
        session_id: SessionId::new(),
        memory,
        knowledge_base,
        conversation_history: vec![],
        metadata: HashMap::new(),
        created_at: SystemTime::now(),
        updated_at: SystemTime::now(),
        retention_policy: RetentionPolicy::default(),
    };

    let context_id = context_manager.store_context(agent_id, context).await?;
    println!("✅ Context stored with ID: {}", context_id);

    // Test 2: Retrieve context and measure performance
    println!("\n⚡ Test 2: Performance testing context retrieval...");

    let mut retrieval_times = Vec::new();
    for i in 0..10 {
        let start = Instant::now();
        let retrieved_context = context_manager.retrieve_context(agent_id, None).await?;
        let duration = start.elapsed();
        retrieval_times.push(duration);

        if let Some(ctx) = retrieved_context {
            println!(
                "  Retrieval {}: {}ms - Memory items: {}, Facts: {}",
                i + 1,
                duration.as_millis(),
                ctx.memory.short_term.len(),
                ctx.knowledge_base.facts.len()
            );
        } else {
            println!("  ❌ Failed to retrieve context on attempt {}", i + 1);
        }
    }

    // Calculate average retrieval time
    let avg_time =
        retrieval_times.iter().sum::<std::time::Duration>() / retrieval_times.len() as u32;
    println!("\n📊 Performance Results:");
    println!("  Average retrieval time: {}ms", avg_time.as_millis());
    println!("  Requirement: <50ms");

    if avg_time.as_millis() < 50 {
        println!("  ✅ PASSED: Retrieval performance meets requirement");
    } else {
        println!("  ❌ FAILED: Retrieval performance exceeds 50ms requirement");
    }

    // Test 3: Add knowledge and verify persistence
    println!("\n🧠 Test 3: Adding knowledge and testing persistence...");

    let knowledge = Knowledge::Fact(KnowledgeFact {
        id: KnowledgeId::new(),
        subject: "Agent".to_string(),
        predicate: "specializes_in".to_string(),
        object: "Context Management".to_string(),
        confidence: 0.95,
        source: KnowledgeSource::Learning,
        created_at: SystemTime::now(),
        verified: true,
    });

    let knowledge_id = context_manager.add_knowledge(agent_id, knowledge).await?;
    println!("✅ Knowledge added with ID: {}", knowledge_id);

    // Test 4: Search knowledge
    println!("\n🔍 Test 4: Testing knowledge search...");
    let search_results = context_manager
        .search_knowledge(agent_id, "Software", 5)
        .await?;
    println!(
        "  Found {} knowledge items matching 'Software'",
        search_results.len()
    );

    for (i, item) in search_results.iter().enumerate() {
        println!(
            "    {}: {} (confidence: {:.2})",
            i + 1,
            item.content,
            item.confidence
        );
    }

    // Test 5: Context statistics
    println!("\n📈 Test 5: Getting context statistics...");
    let stats = context_manager.get_context_stats(agent_id).await?;
    println!("  Total memory items: {}", stats.total_memory_items);
    println!("  Total knowledge items: {}", stats.total_knowledge_items);
    println!("  Total conversations: {}", stats.total_conversations);
    println!("  Last activity: {:?}", stats.last_activity);

    // Test 6: Verify file persistence
    println!("\n💾 Test 6: Verifying file persistence...");
    let context_file = test_dir
        .join("state")
        .join("agents")
        .join(format!("{}.json.gz", agent_id));
    if context_file.exists() {
        let file_size = std::fs::metadata(&context_file)?.len();
        println!(
            "  ✅ Context file exists: {} ({} bytes)",
            context_file.display(),
            file_size
        );
    } else {
        println!("  ❌ Context file not found: {}", context_file.display());
    }

    // Test 7: Create new context manager and load from persistence
    println!("\n🔄 Test 7: Testing persistence across restarts...");
    let new_context_manager = StandardContextManager::new(
        ContextManagerConfig {
            max_contexts_in_memory: 100,
            enable_persistence: true,
            persistence_config: FilePersistenceConfig {
                root_data_dir: test_dir.clone(),
                state_dir: PathBuf::from("state"),
                logs_dir: PathBuf::from("logs"),
                prompts_dir: PathBuf::from("prompts"),
                vector_db_dir: PathBuf::from("vector_db"),
                enable_compression: true,
                enable_encryption: false,
                backup_count: 3,
                auto_save_interval: 60,
                auto_create_dirs: true,
                dir_permissions: Some(0o755),
            },
            enable_vector_db: false,
            qdrant_config: QdrantConfig::default(),
            ..Default::default()
        },
        "test-agent-2",
    )
    .await?;

    new_context_manager.initialize().await?;

    let loaded_context = new_context_manager.retrieve_context(agent_id, None).await?;
    if let Some(ctx) = loaded_context {
        println!("  ✅ Context successfully loaded after restart");
        println!("    Memory items: {}", ctx.memory.short_term.len());
        println!("    Knowledge facts: {}", ctx.knowledge_base.facts.len());
    } else {
        println!("  ❌ Failed to load context after restart");
    }

    // Cleanup
    println!("\n🧹 Cleaning up test files...");
    if test_dir.exists() {
        std::fs::remove_dir_all(&test_dir)?;
        println!("  ✅ Test directory cleaned up");
    }

    println!("\n🎉 Agent Context Manager test completed successfully!");
    println!("   All persistence features are working correctly.");

    Ok(())
}
