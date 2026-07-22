//! End-to-end: a KnowledgeBridge over a real StandardContextManager (LanceDB)
//! round-trips a stored fact through the vector store — the concrete proof that
//! the wired RAG path retrieves what it stored. Requires `vector-lancedb`.
#![cfg(feature = "vector-lancedb")]

use std::sync::Arc;

use symbi_runtime::context::manager::{ContextManagerConfig, StandardContextManager};
use symbi_runtime::context::vector_db_factory::VectorBackendConfig;
use symbi_runtime::context::vector_db_lance::LanceDbConfig;
use symbi_runtime::reasoning::knowledge_bridge::{KnowledgeBridge, KnowledgeConfig};
use symbi_runtime::secrets::config::SecretsConfig;
use symbi_runtime::types::AgentId;

#[tokio::test(flavor = "multi_thread")]
async fn store_then_recall_round_trips_through_lancedb() {
    // Ensure mock embeddings are used deterministically: if an embedding
    // provider is configured in the ambient environment (e.g. OPENAI_API_KEY
    // set on a dev machine or CI runner), StandardContextManager::new builds
    // a real (network, dim-1536) embedding service, which mismatches this
    // test's 384-dim LanceDB table and fails non-deterministically.
    for k in [
        "EMBEDDING_API_KEY",
        "OPENAI_API_KEY",
        "EMBEDDING_API_BASE_URL",
        "OPENAI_API_BASE_URL",
        "EMBEDDING_PROVIDER",
        "EMBEDDING_MODEL",
        "VECTOR_DIMENSION",
    ] {
        std::env::remove_var(k);
    }

    let dir = tempfile::tempdir().expect("tempdir");

    // Real context manager backed by a temp LanceDB store. No embedding provider
    // is configured, so it uses deterministic mock embeddings — fine here: we
    // store exactly one fact, so a nearest-neighbour search returns it.
    let cfg = ContextManagerConfig {
        enable_vector_db: true,
        vector_backend: Some(VectorBackendConfig::LanceDb(LanceDbConfig {
            data_path: dir.path().to_path_buf(),
            vector_dimension: 384,
            ..Default::default()
        })),
        // Keep the secrets store inside the temp dir (avoid CWD writes/races).
        secrets_config: SecretsConfig::file_json(dir.path().join("secrets.json")),
        ..Default::default()
    };

    let scm = StandardContextManager::new(cfg, "test-agent")
        .await
        .expect("build StandardContextManager");
    let bridge = KnowledgeBridge::new(Arc::new(scm), KnowledgeConfig::default());
    let agent = AgentId::new(); // reused for both store and recall (same namespace)

    // Store one fact through the bridge's store_knowledge tool.
    let stored = bridge
        .handle_tool_call(
            &agent,
            "store_knowledge",
            r#"{"subject":"France","predicate":"capital","object":"Paris"}"#,
        )
        .await
        .expect("store_knowledge should succeed");
    assert!(!stored.is_empty());

    // Recall through the bridge — the single stored fact is the nearest neighbour.
    let recalled = bridge
        .handle_tool_call(
            &agent,
            "recall_knowledge",
            r#"{"query":"what is the capital of France"}"#,
        )
        .await
        .expect("recall_knowledge should succeed");

    assert!(
        recalled.contains("Paris"),
        "the wired RAG path must retrieve the stored fact, got: {recalled}"
    );
}
