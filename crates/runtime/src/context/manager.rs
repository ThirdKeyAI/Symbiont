//! Context Manager implementation for agent memory and knowledge management

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::RwLock;

use super::embedding::create_embedding_service_from_env;
use super::types::*;
use super::vector_db::{EmbeddingService, NoOpVectorDatabase, QdrantConfig};
use super::vector_db_factory::{create_vector_backend, resolve_vector_config, VectorBackendConfig};
use super::vector_db_trait::VectorDb;
use crate::integrations::policy_engine::{MockPolicyEngine, PolicyEngine};
use crate::secrets::{SecretStore, SecretsConfig};
use crate::types::AgentId;

/// Context Manager trait for agent memory and knowledge management
#[async_trait]
pub trait ContextManager: Send + Sync {
    /// Store agent context
    async fn store_context(
        &self,
        agent_id: AgentId,
        context: AgentContext,
    ) -> Result<ContextId, ContextError>;

    /// Retrieve agent context
    async fn retrieve_context(
        &self,
        agent_id: AgentId,
        session_id: Option<SessionId>,
    ) -> Result<Option<AgentContext>, ContextError>;

    /// Query context with semantic search
    async fn query_context(
        &self,
        agent_id: AgentId,
        query: ContextQuery,
    ) -> Result<Vec<ContextItem>, ContextError>;

    /// Update specific memory items
    async fn update_memory(
        &self,
        agent_id: AgentId,
        memory_updates: Vec<MemoryUpdate>,
    ) -> Result<(), ContextError>;

    /// Add knowledge to agent's knowledge base
    async fn add_knowledge(
        &self,
        agent_id: AgentId,
        knowledge: Knowledge,
    ) -> Result<KnowledgeId, ContextError>;

    /// Search knowledge base
    async fn search_knowledge(
        &self,
        agent_id: AgentId,
        query: &str,
        limit: usize,
    ) -> Result<Vec<KnowledgeItem>, ContextError>;

    /// Share knowledge between agents
    async fn share_knowledge(
        &self,
        from_agent: AgentId,
        to_agent: AgentId,
        knowledge_id: KnowledgeId,
        access_level: AccessLevel,
    ) -> Result<(), ContextError>;

    /// Get shared knowledge available to agent
    async fn get_shared_knowledge(
        &self,
        agent_id: AgentId,
    ) -> Result<Vec<SharedKnowledgeRef>, ContextError>;

    /// Archive old context based on retention policy
    async fn archive_context(
        &self,
        agent_id: AgentId,
        before: SystemTime,
    ) -> Result<u32, ContextError>;

    /// Get context statistics
    async fn get_context_stats(&self, agent_id: AgentId) -> Result<ContextStats, ContextError>;

    /// Shutdown the context manager gracefully
    async fn shutdown(&self) -> Result<(), ContextError>;
}

/// Standard implementation of ContextManager
pub struct StandardContextManager {
    /// In-memory storage for contexts (cache layer)
    contexts: Arc<RwLock<HashMap<AgentId, AgentContext>>>,
    /// Configuration for the context manager
    config: ContextManagerConfig,
    /// Shared knowledge store
    shared_knowledge: Arc<RwLock<HashMap<KnowledgeId, SharedKnowledgeItem>>>,
    /// Vector database for semantic search and knowledge storage
    vector_db: Arc<dyn VectorDb>,
    /// Embedding service for generating vector embeddings
    embedding_service: Arc<dyn EmbeddingService>,
    /// Persistent storage for contexts
    persistence: Arc<dyn ContextPersistence>,
    /// Secrets store for secure secret management
    secrets: Box<dyn SecretStore + Send + Sync>,
    /// Policy engine for access control and permissions
    #[allow(dead_code)]
    policy_engine: Arc<dyn PolicyEngine>,
    /// Shutdown flag to ensure idempotent shutdown
    shutdown_flag: Arc<RwLock<bool>>,
    /// Background task handles (for future retention scheduler)
    background_tasks: Arc<RwLock<Vec<tokio::task::JoinHandle<()>>>>,
}

/// Configuration for the Context Manager
#[derive(Debug, Clone)]
pub struct ContextManagerConfig {
    /// Maximum number of contexts to keep in memory
    pub max_contexts_in_memory: usize,
    /// Default retention policy for new contexts
    pub default_retention_policy: RetentionPolicy,
    /// Enable automatic archiving
    pub enable_auto_archiving: bool,
    /// Archiving check interval
    pub archiving_interval: std::time::Duration,
    /// Maximum memory items per agent
    pub max_memory_items_per_agent: usize,
    /// Maximum knowledge items per agent
    pub max_knowledge_items_per_agent: usize,
    /// Vector backend configuration (if set, used instead of qdrant_config)
    pub vector_backend: Option<VectorBackendConfig>,
    /// Qdrant vector database configuration (legacy, use vector_backend instead)
    pub qdrant_config: QdrantConfig,
    /// Enable vector database integration
    pub enable_vector_db: bool,
    /// File persistence configuration
    pub persistence_config: FilePersistenceConfig,
    /// Enable persistent storage
    pub enable_persistence: bool,
    /// Secrets configuration for secure secret management
    pub secrets_config: SecretsConfig,
}

impl Default for ContextManagerConfig {
    fn default() -> Self {
        use std::path::PathBuf;

        Self {
            max_contexts_in_memory: 1000,
            default_retention_policy: RetentionPolicy::default(),
            enable_auto_archiving: true,
            archiving_interval: std::time::Duration::from_secs(3600), // 1 hour
            max_memory_items_per_agent: 10000,
            max_knowledge_items_per_agent: 5000,
            vector_backend: None,
            qdrant_config: QdrantConfig::default(),
            enable_vector_db: false,
            persistence_config: FilePersistenceConfig::default(),
            enable_persistence: true,
            secrets_config: SecretsConfig::file_json(PathBuf::from("secrets.json")),
        }
    }
}

/// Configuration for importance calculation weights
#[derive(Debug, Clone)]
struct ImportanceWeights {
    /// Weight for base importance score
    pub base_importance: f32,
    /// Weight for access frequency factor
    pub access_frequency: f32,
    /// Weight for recency factor
    pub recency: f32,
    /// Weight for user feedback factor
    pub user_feedback: f32,
    /// Penalty for memories that have never been accessed
    pub no_access_penalty: f32,
}

impl Default for ImportanceWeights {
    fn default() -> Self {
        Self {
            base_importance: 0.3,
            access_frequency: 0.25,
            recency: 0.3,
            user_feedback: 0.15,
            no_access_penalty: 0.1,
        }
    }
}

/// Shared knowledge item with metadata
#[derive(Debug, Clone)]
struct SharedKnowledgeItem {
    knowledge: Knowledge,
    source_agent: AgentId,
    access_level: AccessLevel,
    created_at: SystemTime,
    access_count: u32,
}

/// Archived context structure for storing old items
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ArchivedContext {
    agent_id: AgentId,
    archived_at: SystemTime,
    archive_reason: String,
    memory: HierarchicalMemory,
    conversation_history: Vec<ConversationItem>,
    knowledge_base: KnowledgeBase,
    metadata: HashMap<String, String>,
}

impl ArchivedContext {
    fn new(agent_id: AgentId, before: SystemTime) -> Self {
        Self {
            agent_id,
            archived_at: SystemTime::now(),
            archive_reason: format!("Archiving items before {:?}", before),
            memory: HierarchicalMemory::default(),
            conversation_history: Vec::new(),
            knowledge_base: KnowledgeBase::default(),
            metadata: HashMap::new(),
        }
    }
}

/// File-based persistence implementation
pub struct FilePersistence {
    config: FilePersistenceConfig,
}

impl FilePersistence {
    /// Create a new FilePersistence instance
    pub fn new(config: FilePersistenceConfig) -> Self {
        Self { config }
    }

    /// Initialize storage directory
    pub async fn initialize(&self) -> Result<(), ContextError> {
        self.config
            .ensure_directories()
            .await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to create storage directories: {}", e),
            })?;
        Ok(())
    }

    /// Get file path for agent context
    fn get_context_path(&self, agent_id: AgentId) -> PathBuf {
        let filename = if self.config.enable_compression {
            format!("{}.json.gz", agent_id)
        } else {
            format!("{}.json", agent_id)
        };
        self.config.agent_contexts_path().join(filename)
    }

    /// Serialize context to bytes
    async fn serialize_context(&self, context: &AgentContext) -> Result<Vec<u8>, ContextError> {
        let json_data =
            serde_json::to_vec_pretty(context).map_err(|e| ContextError::SerializationError {
                reason: format!("Failed to serialize context: {}", e),
            })?;

        if self.config.enable_compression {
            use flate2::write::GzEncoder;
            use flate2::Compression;
            use std::io::Write;

            let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
            encoder
                .write_all(&json_data)
                .map_err(|e| ContextError::SerializationError {
                    reason: format!("Failed to compress context: {}", e),
                })?;
            encoder
                .finish()
                .map_err(|e| ContextError::SerializationError {
                    reason: format!("Failed to finalize compression: {}", e),
                })
        } else {
            Ok(json_data)
        }
    }

    /// Deserialize context from bytes
    async fn deserialize_context(&self, data: Vec<u8>) -> Result<AgentContext, ContextError> {
        let json_data = if self.config.enable_compression {
            use flate2::read::GzDecoder;
            use std::io::Read;

            let mut decoder = GzDecoder::new(&data[..]);
            let mut decompressed = Vec::new();
            decoder.read_to_end(&mut decompressed).map_err(|e| {
                ContextError::SerializationError {
                    reason: format!("Failed to decompress context: {}", e),
                }
            })?;
            decompressed
        } else {
            data
        };

        serde_json::from_slice(&json_data).map_err(|e| ContextError::SerializationError {
            reason: format!("Failed to deserialize context: {}", e),
        })
    }

    /// Create backup of existing context file
    async fn create_backup(&self, agent_id: AgentId) -> Result<(), ContextError> {
        let context_path = self.get_context_path(agent_id);
        if !context_path.exists() {
            return Ok(());
        }

        let backup_path = context_path.with_extension(format!(
            "backup.{}.json",
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        ));

        fs::copy(&context_path, &backup_path)
            .await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to create backup: {}", e),
            })?;

        // Clean up old backups
        self.cleanup_old_backups(agent_id).await?;
        Ok(())
    }

    /// Clean up old backup files
    async fn cleanup_old_backups(&self, agent_id: AgentId) -> Result<(), ContextError> {
        let mut backup_files = Vec::new();
        let mut dir = fs::read_dir(&self.config.agent_contexts_path())
            .await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to read storage directory: {}", e),
            })?;

        while let Some(entry) = dir
            .next_entry()
            .await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to read directory entry: {}", e),
            })?
        {
            let path = entry.path();
            if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                if filename.starts_with(&format!("{}.backup.", agent_id)) {
                    if let Ok(metadata) = entry.metadata().await {
                        if let Ok(modified) = metadata.modified() {
                            backup_files.push((path, modified));
                        }
                    }
                }
            }
        }

        // Sort by modification time (newest first)
        backup_files.sort_by(|a, b| b.1.cmp(&a.1));

        // Remove excess backups
        for (path, _) in backup_files.into_iter().skip(self.config.backup_count) {
            if let Err(e) = fs::remove_file(&path).await {
                eprintln!(
                    "Warning: Failed to remove old backup {}: {}",
                    path.display(),
                    e
                );
            }
        }

        Ok(())
    }
}

#[async_trait]
impl ContextPersistence for FilePersistence {
    async fn save_context(
        &self,
        agent_id: AgentId,
        context: &AgentContext,
    ) -> Result<(), ContextError> {
        // Create backup of existing context
        self.create_backup(agent_id).await?;

        // Serialize context
        let data = self.serialize_context(context).await?;

        // Write to file
        let context_path = self.get_context_path(agent_id);
        let mut file =
            fs::File::create(&context_path)
                .await
                .map_err(|e| ContextError::StorageError {
                    reason: format!("Failed to create context file: {}", e),
                })?;

        file.write_all(&data)
            .await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to write context data: {}", e),
            })?;

        file.sync_all()
            .await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to sync context file: {}", e),
            })?;

        Ok(())
    }

    async fn load_context(&self, agent_id: AgentId) -> Result<Option<AgentContext>, ContextError> {
        let context_path = self.get_context_path(agent_id);

        if !context_path.exists() {
            return Ok(None);
        }

        let mut file =
            fs::File::open(&context_path)
                .await
                .map_err(|e| ContextError::StorageError {
                    reason: format!("Failed to open context file: {}", e),
                })?;

        let mut data = Vec::new();
        file.read_to_end(&mut data)
            .await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to read context file: {}", e),
            })?;

        let context = self.deserialize_context(data).await?;
        Ok(Some(context))
    }

    async fn delete_context(&self, agent_id: AgentId) -> Result<(), ContextError> {
        let context_path = self.get_context_path(agent_id);

        if context_path.exists() {
            fs::remove_file(&context_path)
                .await
                .map_err(|e| ContextError::StorageError {
                    reason: format!("Failed to delete context file: {}", e),
                })?;
        }

        Ok(())
    }

    async fn list_agent_contexts(&self) -> Result<Vec<AgentId>, ContextError> {
        let mut agent_ids = Vec::new();
        let mut dir = fs::read_dir(&self.config.agent_contexts_path())
            .await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to read storage directory: {}", e),
            })?;

        while let Some(entry) = dir
            .next_entry()
            .await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to read directory entry: {}", e),
            })?
        {
            let path = entry.path();
            if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                if filename.ends_with(".json") || filename.ends_with(".json.gz") {
                    let agent_id_str = filename
                        .strip_suffix(".json.gz")
                        .or_else(|| filename.strip_suffix(".json"))
                        .unwrap_or(filename);

                    if let Ok(uuid) = uuid::Uuid::parse_str(agent_id_str) {
                        agent_ids.push(AgentId(uuid));
                    }
                }
            }
        }

        Ok(agent_ids)
    }

    async fn context_exists(&self, agent_id: AgentId) -> Result<bool, ContextError> {
        let context_path = self.get_context_path(agent_id);
        Ok(context_path.exists())
    }

    async fn get_storage_stats(&self) -> Result<StorageStats, ContextError> {
        let mut total_contexts = 0;
        let mut total_size_bytes = 0;

        let mut dir = fs::read_dir(&self.config.agent_contexts_path())
            .await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to read storage directory: {}", e),
            })?;

        while let Some(entry) = dir
            .next_entry()
            .await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to read directory entry: {}", e),
            })?
        {
            let path = entry.path();
            if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                if filename.ends_with(".json") || filename.ends_with(".json.gz") {
                    total_contexts += 1;
                    if let Ok(metadata) = entry.metadata().await {
                        total_size_bytes += metadata.len();
                    }
                }
            }
        }

        Ok(StorageStats {
            total_contexts,
            total_size_bytes,
            last_cleanup: SystemTime::now(),
            storage_path: self.config.agent_contexts_path(),
        })
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl StandardContextManager {
    /// Create a new StandardContextManager
    pub async fn new(config: ContextManagerConfig, agent_id: &str) -> Result<Self, ContextError> {
        let vector_db: Arc<dyn VectorDb> = if config.enable_vector_db {
            if let Some(ref backend_config) = config.vector_backend {
                create_vector_backend(backend_config.clone())
                    .await
                    .unwrap_or_else(|e| {
                        tracing::warn!("Failed to create vector backend: {}, using NoOp", e);
                        Arc::new(NoOpVectorDatabase)
                    })
            } else {
                let resolved = resolve_vector_config();
                create_vector_backend(resolved).await.unwrap_or_else(|e| {
                    tracing::warn!("Failed to create vector backend: {}, using NoOp", e);
                    Arc::new(NoOpVectorDatabase)
                })
            }
        } else {
            Arc::new(NoOpVectorDatabase)
        };

        let embedding_service =
            create_embedding_service_from_env(config.qdrant_config.vector_dimension)?;

        let persistence: Arc<dyn ContextPersistence> = if config.enable_persistence {
            Arc::new(FilePersistence::new(config.persistence_config.clone()))
        } else {
            // Could use a no-op implementation for testing
            Arc::new(FilePersistence::new(config.persistence_config.clone()))
        };

        // Initialize secrets store
        let secrets = crate::secrets::new_secret_store(&config.secrets_config, agent_id)
            .await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to initialize secrets store: {}", e),
            })?;

        // Initialize policy engine
        let policy_engine: Arc<dyn PolicyEngine> = Arc::new(MockPolicyEngine::new());

        Ok(Self {
            contexts: Arc::new(RwLock::new(HashMap::new())),
            config,
            shared_knowledge: Arc::new(RwLock::new(HashMap::new())),
            vector_db,
            embedding_service,
            persistence,
            secrets,
            policy_engine,
            shutdown_flag: Arc::new(RwLock::new(false)),
            background_tasks: Arc::new(RwLock::new(Vec::new())),
        })
    }

    /// Get access to the secrets store
    pub fn secrets(&self) -> &(dyn SecretStore + Send + Sync) {
        self.secrets.as_ref()
    }

    /// Check token usage and run compaction if thresholds are crossed.
    ///
    /// Returns `Some(CompactionResult)` if compaction was performed, `None` if
    /// usage was below all thresholds.
    pub async fn check_and_compact(
        &self,
        agent_id: &AgentId,
        session_id: &SessionId,
        config: &super::compaction::CompactionConfig,
        counter: &dyn super::token_counter::TokenCounter,
    ) -> Result<Option<super::compaction::CompactionResult>, ContextError> {
        use super::compaction::{select_tier, truncate_items, CompactionTier};

        if !config.enabled {
            return Ok(None);
        }

        // Retrieve current context
        let context = match self.retrieve_context(*agent_id, Some(*session_id)).await? {
            Some(ctx) => ctx,
            None => return Ok(None),
        };

        // Count current token usage
        let current_tokens = counter.count_messages(&context.conversation_history);
        let limit = counter.model_context_limit();

        if limit == 0 {
            return Ok(None);
        }

        let usage_ratio = current_tokens as f32 / limit as f32;

        let tier = match select_tier(usage_ratio, config) {
            Some(t) => t,
            None => return Ok(None),
        };

        let start = std::time::Instant::now();

        match tier {
            CompactionTier::Truncate => {
                let (new_items, affected) = truncate_items(
                    &context.conversation_history,
                    config,
                    config.summarize_threshold,
                );

                if affected == 0 {
                    return Ok(None);
                }

                let tokens_after = counter.count_messages(&new_items);

                let mut updated = context.clone();
                updated.conversation_history = new_items;
                self.store_context(*agent_id, updated).await?;

                Ok(Some(super::compaction::CompactionResult {
                    tier_applied: CompactionTier::Truncate,
                    tokens_before: current_tokens,
                    tokens_after,
                    tokens_saved: current_tokens.saturating_sub(tokens_after),
                    items_affected: affected,
                    duration_ms: start.elapsed().as_millis() as u64,
                    summary_generated: None,
                }))
            }
            CompactionTier::Summarize => {
                // Summarize requires an LLM call — for now, fall back to truncate.
                // Full LLM integration deferred to heartbeat scheduler wiring.
                tracing::info!(
                    agent = %agent_id,
                    "compaction: Summarize tier selected but no LLM client in context manager, falling back to truncate"
                );
                let (new_items, affected) = truncate_items(
                    &context.conversation_history,
                    config,
                    config.summarize_threshold,
                );

                if affected == 0 {
                    return Ok(None);
                }

                let tokens_after = counter.count_messages(&new_items);
                let mut updated = context.clone();
                updated.conversation_history = new_items;
                self.store_context(*agent_id, updated).await?;

                Ok(Some(super::compaction::CompactionResult {
                    tier_applied: CompactionTier::Truncate, // Actually fell back
                    tokens_before: current_tokens,
                    tokens_after,
                    tokens_saved: current_tokens.saturating_sub(tokens_after),
                    items_affected: affected,
                    duration_ms: start.elapsed().as_millis() as u64,
                    summary_generated: None,
                }))
            }
            CompactionTier::CompressEpisodic | CompactionTier::ArchiveToMemory => {
                // Enterprise tiers — stub returns None in OSS
                Ok(None)
            }
        }
    }

    /// Initialize the context manager
    pub async fn initialize(&self) -> Result<(), ContextError> {
        // Initialize vector database connection and collection
        if self.config.enable_vector_db {
            self.vector_db.initialize().await?;
        }

        // Initialize persistence layer
        if self.config.enable_persistence {
            if let Some(file_persistence) =
                self.persistence.as_any().downcast_ref::<FilePersistence>()
            {
                file_persistence.initialize().await?;
            }

            // Load existing contexts from persistent storage
            self.load_existing_contexts().await?;
        }

        // Set up retention policy scheduler
        self.setup_retention_scheduler().await?;

        Ok(())
    }

    /// Load existing contexts from persistent storage
    async fn load_existing_contexts(&self) -> Result<(), ContextError> {
        let agent_ids = self.persistence.list_agent_contexts().await?;
        let mut contexts = self.contexts.write().await;

        for agent_id in agent_ids {
            if let Some(context) = self.persistence.load_context(agent_id).await? {
                contexts.insert(agent_id, context);
            }
        }

        Ok(())
    }

    /// Shutdown the context manager gracefully
    pub async fn shutdown(&self) -> Result<(), ContextError> {
        // Check if already shutdown (idempotent)
        {
            let shutdown_flag = self.shutdown_flag.read().await;
            if *shutdown_flag {
                tracing::info!("ContextManager already shutdown, skipping");
                return Ok(());
            }
        }

        tracing::info!("Starting ContextManager shutdown sequence");

        // Set shutdown flag to prevent new operations
        {
            let mut shutdown_flag = self.shutdown_flag.write().await;
            *shutdown_flag = true;
        }

        // 1. Stop all background tasks
        self.stop_background_tasks().await?;

        // 2. Save all contexts to persistent storage
        self.save_all_contexts().await?;

        // 3. Close vector database connections (if any cleanup is needed)
        // Note: Vector database connections are typically managed by the client
        // and don't require explicit cleanup, but we log the action
        tracing::info!("Vector database connections will be closed when client is dropped");

        // 4. Flush secrets store if needed
        // Note: Secrets store cleanup is typically handled by Drop trait
        tracing::info!("Secrets store cleanup handled by Drop trait");

        tracing::info!("ContextManager shutdown completed successfully");
        Ok(())
    }

    /// Stop all background tasks
    async fn stop_background_tasks(&self) -> Result<(), ContextError> {
        let mut tasks = self.background_tasks.write().await;

        if tasks.is_empty() {
            tracing::debug!("No background tasks to stop");
            return Ok(());
        }

        tracing::info!("Stopping {} background tasks", tasks.len());

        // Abort all background tasks
        for task in tasks.drain(..) {
            task.abort();

            // Wait for task to finish (with timeout to avoid hanging)
            match tokio::time::timeout(std::time::Duration::from_secs(5), task).await {
                Ok(result) => match result {
                    Ok(_) => tracing::debug!("Background task completed successfully"),
                    Err(e) if e.is_cancelled() => tracing::debug!("Background task was cancelled"),
                    Err(e) => tracing::warn!("Background task finished with error: {}", e),
                },
                Err(_) => tracing::warn!("Background task did not finish within timeout"),
            }
        }

        tracing::info!("All background tasks stopped");
        Ok(())
    }

    /// Save all in-memory contexts to persistent storage
    async fn save_all_contexts(&self) -> Result<(), ContextError> {
        if !self.config.enable_persistence {
            tracing::debug!("Persistence disabled, skipping context save");
            return Ok(());
        }

        let contexts = self.contexts.read().await;

        if contexts.is_empty() {
            tracing::debug!("No contexts to save");
            return Ok(());
        }

        tracing::info!("Saving {} contexts to persistent storage", contexts.len());

        let mut save_errors = Vec::new();

        for (agent_id, context) in contexts.iter() {
            match self.persistence.save_context(*agent_id, context).await {
                Ok(_) => tracing::debug!("Saved context for agent {}", agent_id),
                Err(e) => {
                    tracing::error!("Failed to save context for agent {}: {}", agent_id, e);
                    save_errors.push((*agent_id, e));
                }
            }
        }

        if !save_errors.is_empty() {
            let error_msg = format!(
                "Failed to save {} out of {} contexts during shutdown",
                save_errors.len(),
                contexts.len()
            );
            tracing::error!("{}", error_msg);

            // Return the first error, but log all of them
            if let Some((agent_id, error)) = save_errors.into_iter().next() {
                return Err(ContextError::StorageError {
                    reason: format!("Failed to save context for agent {}: {}", agent_id, error),
                });
            }
        }

        tracing::info!("All contexts saved successfully");
        Ok(())
    }

    /// Set up retention policy scheduler as a background task
    async fn setup_retention_scheduler(&self) -> Result<(), ContextError> {
        if !self.config.enable_auto_archiving {
            tracing::debug!("Auto-archiving disabled, skipping retention scheduler setup");
            return Ok(());
        }

        let contexts = self.contexts.clone();
        let persistence = self.persistence.clone();
        let config = self.config.clone();
        let shutdown_flag = self.shutdown_flag.clone();

        let task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(config.archiving_interval);

            tracing::info!(
                "Retention policy scheduler started with interval {:?}",
                config.archiving_interval
            );

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        // Check if we should shutdown
                        if *shutdown_flag.read().await {
                            tracing::info!("Retention scheduler shutting down");
                            break;
                        }

                        // Run retention check for all agents
                        Self::run_retention_check(&contexts, &persistence, &config).await;
                    }
                }
            }
        });

        self.add_background_task(task).await;
        tracing::info!("Retention policy scheduler initialized successfully");
        Ok(())
    }

    /// Run retention check across all agent contexts
    async fn run_retention_check(
        contexts: &Arc<RwLock<HashMap<AgentId, AgentContext>>>,
        persistence: &Arc<dyn ContextPersistence>,
        config: &ContextManagerConfig,
    ) {
        let current_time = SystemTime::now();

        // Collect agent IDs and stats first to avoid borrowing issues
        let agents_to_check: Vec<(AgentId, usize)> = {
            let context_guard = contexts.read().await;
            let agent_count = context_guard.len();

            if agent_count == 0 {
                tracing::debug!("No agent contexts to process for retention");
                return;
            }

            tracing::info!(
                "Starting retention check for {} agent contexts",
                agent_count
            );

            context_guard
                .iter()
                .map(|(agent_id, context)| {
                    let retention_stats = Self::calculate_retention_statistics_static(context);
                    (*agent_id, retention_stats.items_to_archive)
                })
                .collect()
        };

        let start_time = std::time::Instant::now();
        let total_agents = agents_to_check.len();

        // Process each agent that needs archiving
        for (agent_id, items_to_archive) in agents_to_check {
            if items_to_archive > 0 {
                tracing::debug!(
                    "Agent {} has {} items eligible for archiving",
                    agent_id,
                    items_to_archive
                );

                // Archive items for this agent
                let archive_result = Self::archive_agent_context_static(
                    agent_id,
                    current_time,
                    contexts,
                    persistence,
                    config,
                )
                .await;

                match archive_result {
                    Ok(archived_count) => {
                        tracing::info!(
                            "Successfully archived {} items for agent {}",
                            archived_count,
                            agent_id
                        );
                    }
                    Err(e) => {
                        tracing::error!("Failed to archive context for agent {}: {}", agent_id, e);
                    }
                }
            }
        }

        let elapsed = start_time.elapsed();
        tracing::info!(
            "Retention check completed for {} agents in {:?}",
            total_agents,
            elapsed
        );
    }

    /// Static version of calculate_retention_statistics for use in scheduler
    fn calculate_retention_statistics_static(context: &AgentContext) -> RetentionStatus {
        let now = SystemTime::now();
        let retention_policy = &context.retention_policy;

        let mut items_to_archive = 0;
        let items_to_delete = 0;

        // Calculate cutoff times based on retention policy
        let memory_cutoff = now
            .checked_sub(retention_policy.memory_retention)
            .unwrap_or(SystemTime::UNIX_EPOCH);

        let knowledge_cutoff = now
            .checked_sub(retention_policy.knowledge_retention)
            .unwrap_or(SystemTime::UNIX_EPOCH);

        let conversation_cutoff = now
            .checked_sub(retention_policy.session_retention)
            .unwrap_or(SystemTime::UNIX_EPOCH);

        // Count memory items eligible for archiving
        for item in &context.memory.short_term {
            if item.created_at < memory_cutoff || item.last_accessed < memory_cutoff {
                items_to_archive += 1;
            }
        }

        for item in &context.memory.long_term {
            if item.created_at < memory_cutoff || item.last_accessed < memory_cutoff {
                items_to_archive += 1;
            }
        }

        // Count conversation items eligible for archiving
        for item in &context.conversation_history {
            if item.timestamp < conversation_cutoff {
                items_to_archive += 1;
            }
        }

        // Count knowledge items eligible for archiving
        for fact in &context.knowledge_base.facts {
            if fact.created_at < knowledge_cutoff {
                items_to_archive += 1;
            }
        }

        // Calculate next cleanup time
        let next_cleanup = now + Duration::from_secs(86400); // 24 hours

        RetentionStatus {
            items_to_archive,
            items_to_delete,
            next_cleanup,
        }
    }

    /// Static version of archive_context for use in scheduler
    async fn archive_agent_context_static(
        agent_id: AgentId,
        before: SystemTime,
        contexts: &Arc<RwLock<HashMap<AgentId, AgentContext>>>,
        persistence: &Arc<dyn ContextPersistence>,
        config: &ContextManagerConfig,
    ) -> Result<u32, ContextError> {
        let mut total_archived = 0u32;
        let mut archived_context = ArchivedContext::new(agent_id, before);

        // Get mutable access to contexts
        let mut contexts_guard = contexts.write().await;
        if let Some(context) = contexts_guard.get_mut(&agent_id) {
            // Archive items based on retention policy
            let retention_policy = &context.retention_policy;

            // Calculate cutoff times
            let memory_cutoff = before
                .checked_sub(retention_policy.memory_retention)
                .unwrap_or(SystemTime::UNIX_EPOCH);

            let conversation_cutoff = before
                .checked_sub(retention_policy.session_retention)
                .unwrap_or(SystemTime::UNIX_EPOCH);

            let knowledge_cutoff = before
                .checked_sub(retention_policy.knowledge_retention)
                .unwrap_or(SystemTime::UNIX_EPOCH);

            // Archive short-term memory items
            let mut retained_short_term = Vec::new();
            for item in context.memory.short_term.drain(..) {
                if item.created_at < memory_cutoff || item.last_accessed < memory_cutoff {
                    archived_context.memory.short_term.push(item);
                    total_archived += 1;
                } else {
                    retained_short_term.push(item);
                }
            }
            context.memory.short_term = retained_short_term;

            // Archive long-term memory items
            let mut retained_long_term = Vec::new();
            for item in context.memory.long_term.drain(..) {
                if item.created_at < memory_cutoff || item.last_accessed < memory_cutoff {
                    archived_context.memory.long_term.push(item);
                    total_archived += 1;
                } else {
                    retained_long_term.push(item);
                }
            }
            context.memory.long_term = retained_long_term;

            // Archive conversation history
            let mut retained_conversations = Vec::new();
            for item in context.conversation_history.drain(..) {
                if item.timestamp < conversation_cutoff {
                    archived_context.conversation_history.push(item);
                    total_archived += 1;
                } else {
                    retained_conversations.push(item);
                }
            }
            context.conversation_history = retained_conversations;

            // Archive knowledge items
            let mut retained_facts = Vec::new();
            for fact in context.knowledge_base.facts.drain(..) {
                if fact.created_at < knowledge_cutoff {
                    archived_context.knowledge_base.facts.push(fact);
                    total_archived += 1;
                } else {
                    retained_facts.push(fact);
                }
            }
            context.knowledge_base.facts = retained_facts;

            // Update context metadata
            if total_archived > 0 {
                context.updated_at = SystemTime::now();
                context.metadata.insert(
                    "last_archived".to_string(),
                    SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs()
                        .to_string(),
                );
                context
                    .metadata
                    .insert("archived_count".to_string(), total_archived.to_string());

                // Save archived context to storage
                if config.enable_persistence {
                    Self::save_archived_context_static(
                        agent_id,
                        &archived_context,
                        persistence,
                        config,
                    )
                    .await?;
                    // Also save updated context
                    persistence.save_context(agent_id, context).await?;
                }
            }
        }

        Ok(total_archived)
    }

    /// Static version of save_archived_context for use in scheduler
    async fn save_archived_context_static(
        agent_id: AgentId,
        archived_context: &ArchivedContext,
        _persistence: &Arc<dyn ContextPersistence>,
        config: &ContextManagerConfig,
    ) -> Result<(), ContextError> {
        let archive_dir = config
            .persistence_config
            .agent_contexts_path()
            .join("archives")
            .join(agent_id.to_string());

        // Ensure archive directory exists
        tokio::fs::create_dir_all(&archive_dir)
            .await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to create archive directory: {}", e),
            })?;

        // Create archive filename with timestamp
        let timestamp = archived_context
            .archived_at
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let archive_filename = format!("archive_{}.json", timestamp);
        let archive_path = archive_dir.join(archive_filename);

        // Serialize archived context
        let archive_data = serde_json::to_vec_pretty(archived_context).map_err(|e| {
            ContextError::SerializationError {
                reason: format!("Failed to serialize archived context: {}", e),
            }
        })?;

        // Write to archive file
        tokio::fs::write(&archive_path, &archive_data)
            .await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to write archive file: {}", e),
            })?;

        tracing::debug!(
            "Saved archived context for agent {} to {}",
            agent_id,
            archive_path.display()
        );

        Ok(())
    }

    /// Add a background task to be tracked for shutdown
    pub async fn add_background_task(&self, task: tokio::task::JoinHandle<()>) {
        let mut tasks = self.background_tasks.write().await;
        tasks.push(task);
    }

    /// Check if the context manager is shutdown
    pub async fn is_shutdown(&self) -> bool {
        let shutdown_flag = self.shutdown_flag.read().await;
        *shutdown_flag
    }

    /// Create a new session for an agent
    pub async fn create_session(&self, agent_id: AgentId) -> Result<SessionId, ContextError> {
        let session_id = SessionId::new();

        // Create new context for the session
        let context = AgentContext {
            agent_id,
            session_id,
            memory: HierarchicalMemory::default(),
            knowledge_base: KnowledgeBase::default(),
            conversation_history: Vec::new(),
            metadata: HashMap::new(),
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
            retention_policy: self.config.default_retention_policy.clone(),
        };

        ContextManager::store_context(self, agent_id, context).await?;
        Ok(session_id)
    }

    /// Validate access permissions for context operations
    async fn validate_access(
        &self,
        agent_id: AgentId,
        operation: &str,
    ) -> Result<(), ContextError> {
        // For now, we'll implement a simplified access check
        // In a full implementation, this would create the appropriate PolicyRequest
        // and call the policy engine with proper AgentAction and PolicyContext

        // Simple validation - deny dangerous operations by default
        let is_dangerous_operation = matches!(operation, "archive_context");

        if is_dangerous_operation {
            tracing::warn!(
                "Potentially dangerous operation {} denied for agent {} by default policy",
                operation,
                agent_id
            );
            Err(ContextError::AccessDenied {
                reason: format!("Operation {} requires explicit approval", operation),
            })
        } else {
            tracing::debug!("Policy engine allowed {} for agent {}", operation, agent_id);
            Ok(())
        }
    }

    /// Generate embeddings for content
    async fn generate_embeddings(&self, content: &str) -> Result<Vec<f32>, ContextError> {
        self.embedding_service.generate_embedding(content).await
    }

    /// Perform semantic search on memory items
    async fn semantic_search_memory(
        &self,
        agent_id: AgentId,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ContextItem>, ContextError> {
        if self.config.enable_vector_db {
            // Generate embeddings for the query
            let query_embedding = self.generate_embeddings(query).await?;

            // Search the vector database with semantic similarity
            let threshold = 0.7; // Minimum similarity threshold
            self.vector_db
                .semantic_search(agent_id, query_embedding, limit, threshold)
                .await
        } else {
            // Fallback to simple keyword search
            let contexts = self.contexts.read().await;
            if let Some(context) = contexts.get(&agent_id) {
                let mut results = Vec::new();

                for memory_item in &context.memory.short_term {
                    if memory_item
                        .content
                        .to_lowercase()
                        .contains(&query.to_lowercase())
                    {
                        // Calculate relevance score based on importance and keyword match
                        let importance_score = self.calculate_importance(memory_item);
                        let relevance_score = (importance_score + 0.8) / 2.0; // Blend importance with match score

                        results.push(ContextItem {
                            id: memory_item.id,
                            content: memory_item.content.clone(),
                            item_type: ContextItemType::Memory(memory_item.memory_type.clone()),
                            relevance_score,
                            timestamp: memory_item.created_at,
                            metadata: memory_item.metadata.clone(),
                        });
                    }
                }

                results.truncate(limit);
                Ok(results)
            } else {
                Ok(Vec::new())
            }
        }
    }

    /// Calculate memory importance score using sophisticated multi-factor algorithm
    ///
    /// This algorithm considers:
    /// - Base importance: Initial importance score
    /// - Access frequency: How often the memory is accessed (logarithmic scaling)
    /// - Recency: How recently the memory was accessed or created
    /// - User feedback: Explicit or implicit feedback from user interactions
    /// - Memory type: Different types of memory have different base weightings
    /// - Age decay: Older memories naturally lose importance over time
    ///
    /// Returns a normalized score between 0.0 and 1.0
    fn calculate_importance(&self, memory_item: &MemoryItem) -> f32 {
        // Configurable weights for different factors
        let weights = ImportanceWeights::default();

        // 1. Base importance (0.0 - 1.0)
        let base_score = memory_item.importance.clamp(0.0, 1.0);

        // 2. Access frequency factor (logarithmic scaling to prevent dominance)
        let access_score = if memory_item.access_count == 0 {
            weights.no_access_penalty
        } else {
            let log_access = (memory_item.access_count as f32 + 1.0).ln();
            (log_access / 10.0).min(1.0) // Cap at ln(10) ≈ 2.3
        };

        // 3. Recency factor - considers both last access and creation time
        let recency_score = self.calculate_recency_factor(memory_item);

        // 4. User feedback factor from metadata
        let feedback_score = self.extract_user_feedback_score(memory_item);

        // 5. Memory type adjustment
        let type_multiplier = self.get_memory_type_multiplier(&memory_item.memory_type);

        // 6. Age decay factor
        let age_decay = self.calculate_age_decay(memory_item);

        // Combine all factors using weighted average
        let combined_score = (base_score * weights.base_importance
            + access_score * weights.access_frequency
            + recency_score * weights.recency
            + feedback_score * weights.user_feedback)
            * type_multiplier
            * age_decay;

        // Ensure the final score is within bounds [0.0, 1.0]
        combined_score.clamp(0.0, 1.0)
    }

    /// Calculate recency factor based on last access and creation time
    fn calculate_recency_factor(&self, memory_item: &MemoryItem) -> f32 {
        let now = SystemTime::now();

        // Use the more recent of last_accessed or created_at
        let most_recent = memory_item.last_accessed.max(memory_item.created_at);

        let time_since_access = now
            .duration_since(most_recent)
            .unwrap_or_else(|_| std::time::Duration::from_secs(0));

        let hours_since_access = time_since_access.as_secs() as f32 / 3600.0;

        // Exponential decay with configurable half-life
        let half_life_hours = 24.0; // 24 hours half-life
        let decay_factor = 2.0_f32.powf(-hours_since_access / half_life_hours);

        // Ensure minimum recency score to prevent complete obsolescence
        decay_factor.max(0.01)
    }

    /// Extract user feedback score from memory item metadata
    fn extract_user_feedback_score(&self, memory_item: &MemoryItem) -> f32 {
        let mut feedback_score = 0.5; // Neutral baseline

        // Check for explicit user ratings
        if let Some(rating_str) = memory_item.metadata.get("user_rating") {
            if let Ok(rating) = rating_str.parse::<f32>() {
                feedback_score = (rating / 5.0).clamp(0.0, 1.0); // Assume 1-5 rating scale
            }
        }

        // Check for implicit feedback indicators
        if let Some(helpful_str) = memory_item.metadata.get("helpful") {
            match helpful_str.to_lowercase().as_str() {
                "true" | "yes" | "1" => feedback_score = feedback_score.max(0.8),
                "false" | "no" | "0" => feedback_score = feedback_score.min(0.2),
                _ => {}
            }
        }

        // Check for correction indicators (negative feedback)
        if memory_item.metadata.contains_key("corrected")
            || memory_item.metadata.contains_key("incorrect")
        {
            feedback_score = feedback_score.min(0.3);
        }

        // Check for bookmark/favorite indicators (positive feedback)
        if memory_item.metadata.contains_key("bookmarked")
            || memory_item.metadata.contains_key("favorite")
        {
            feedback_score = feedback_score.max(0.9);
        }

        // Check for usage context that indicates importance
        if let Some(context) = memory_item.metadata.get("usage_context") {
            match context.to_lowercase().as_str() {
                "critical" | "important" => feedback_score = feedback_score.max(0.95),
                "routine" | "common" => feedback_score = feedback_score.max(0.7),
                "experimental" | "trial" => feedback_score = feedback_score.min(0.4),
                _ => {}
            }
        }

        feedback_score.clamp(0.0, 1.0)
    }

    /// Get multiplier based on memory type importance
    fn get_memory_type_multiplier(&self, memory_type: &MemoryType) -> f32 {
        match memory_type {
            MemoryType::Factual => 1.0,    // Base multiplier for facts
            MemoryType::Procedural => 1.2, // Procedures are often more important
            MemoryType::Episodic => 0.9,   // Episodes can vary in importance
            MemoryType::Semantic => 1.1,   // Concepts and relationships are valuable
            MemoryType::Working => 1.3,    // Current working memory is highly relevant
        }
    }

    /// Calculate age decay factor for long-term memory degradation
    fn calculate_age_decay(&self, memory_item: &MemoryItem) -> f32 {
        let now = SystemTime::now();
        let age = now
            .duration_since(memory_item.created_at)
            .unwrap_or_else(|_| std::time::Duration::from_secs(0));

        let days_old = age.as_secs() as f32 / 86400.0; // Convert to days

        // Different decay rates based on memory type
        let decay_rate = match memory_item.memory_type {
            MemoryType::Working => 0.1,      // Working memory decays quickly
            MemoryType::Factual => 0.01,     // Facts persist longer
            MemoryType::Procedural => 0.005, // Procedures are most persistent
            MemoryType::Episodic => 0.02,    // Episodes have moderate decay
            MemoryType::Semantic => 0.008,   // Semantic memory is quite persistent
        };

        // Exponential decay: importance = base * e^(-decay_rate * days)
        let decay_factor = (-decay_rate * days_old).exp();

        // Ensure minimum decay to prevent complete loss
        decay_factor.max(0.05)
    }

    /// Perform keyword search on memory items
    async fn keyword_search_memory(
        &self,
        agent_id: AgentId,
        query: &ContextQuery,
    ) -> Result<Vec<ContextItem>, ContextError> {
        let contexts = self.contexts.read().await;
        if let Some(context) = contexts.get(&agent_id) {
            let mut results = Vec::new();

            // Combine all search terms
            let search_terms: Vec<String> = query
                .search_terms
                .iter()
                .map(|term| term.to_lowercase())
                .collect();

            if search_terms.is_empty() {
                return Ok(results);
            }

            // Search through memory items
            for memory_item in context
                .memory
                .short_term
                .iter()
                .chain(context.memory.long_term.iter())
            {
                // Skip if memory type filter is specified and doesn't match
                if !query.memory_types.is_empty()
                    && !query.memory_types.contains(&memory_item.memory_type)
                {
                    continue;
                }

                let content_lower = memory_item.content.to_lowercase();
                let mut match_score = 0.0f32;
                let mut matched_terms = 0;

                // Calculate match score based on term presence
                for term in &search_terms {
                    if content_lower.contains(term) {
                        matched_terms += 1;
                        // Boost score for exact matches vs substring matches
                        if content_lower.split_whitespace().any(|word| word == term) {
                            match_score += 1.0;
                        } else {
                            match_score += 0.5;
                        }
                    }
                }

                if matched_terms > 0 {
                    // Calculate relevance score combining match score and importance
                    let importance_score = self.calculate_importance(memory_item);
                    let term_coverage = matched_terms as f32 / search_terms.len() as f32;
                    let relevance_score = (match_score * term_coverage + importance_score) / 2.0;

                    if relevance_score >= query.relevance_threshold {
                        results.push(ContextItem {
                            id: memory_item.id,
                            content: memory_item.content.clone(),
                            item_type: ContextItemType::Memory(memory_item.memory_type.clone()),
                            relevance_score,
                            timestamp: memory_item.created_at,
                            metadata: memory_item.metadata.clone(),
                        });
                    }
                }
            }

            // Search through episodic memory
            for episode in &context.memory.episodic_memory {
                let episode_content = format!("{} {}", episode.title, episode.description);
                let content_lower = episode_content.to_lowercase();
                let mut match_score = 0.0f32;
                let mut matched_terms = 0;

                for term in &search_terms {
                    if content_lower.contains(term) {
                        matched_terms += 1;
                        match_score += if content_lower.split_whitespace().any(|word| word == term)
                        {
                            1.0
                        } else {
                            0.5
                        };
                    }
                }

                if matched_terms > 0 {
                    let term_coverage = matched_terms as f32 / search_terms.len() as f32;
                    let relevance_score = (match_score * term_coverage + episode.importance) / 2.0;

                    if relevance_score >= query.relevance_threshold {
                        results.push(ContextItem {
                            id: episode.id,
                            content: episode_content,
                            item_type: ContextItemType::Episode,
                            relevance_score,
                            timestamp: episode.timestamp,
                            metadata: HashMap::new(),
                        });
                    }
                }
            }

            // Search through conversation history
            for conv_item in &context.conversation_history {
                let content_lower = conv_item.content.to_lowercase();
                let mut match_score = 0.0f32;
                let mut matched_terms = 0;

                for term in &search_terms {
                    if content_lower.contains(term) {
                        matched_terms += 1;
                        match_score += if content_lower.split_whitespace().any(|word| word == term)
                        {
                            1.0
                        } else {
                            0.5
                        };
                    }
                }

                if matched_terms > 0 {
                    let term_coverage = matched_terms as f32 / search_terms.len() as f32;
                    let relevance_score = match_score * term_coverage;

                    if relevance_score >= query.relevance_threshold {
                        results.push(ContextItem {
                            id: conv_item.id,
                            content: conv_item.content.clone(),
                            item_type: ContextItemType::Conversation,
                            relevance_score,
                            timestamp: conv_item.timestamp,
                            metadata: HashMap::new(),
                        });
                    }
                }
            }

            // Sort by relevance score (highest first) and limit results
            results.sort_by(|a, b| {
                b.relevance_score
                    .partial_cmp(&a.relevance_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            results.truncate(query.max_results);

            Ok(results)
        } else {
            Ok(Vec::new())
        }
    }

    /// Perform temporal search on memory items within a time range
    async fn temporal_search_memory(
        &self,
        agent_id: AgentId,
        query: &ContextQuery,
    ) -> Result<Vec<ContextItem>, ContextError> {
        let time_range = match &query.time_range {
            Some(range) => range,
            None => return Ok(Vec::new()), // No time range specified
        };

        let contexts = self.contexts.read().await;
        if let Some(context) = contexts.get(&agent_id) {
            let mut results = Vec::new();

            // Search through memory items
            for memory_item in context
                .memory
                .short_term
                .iter()
                .chain(context.memory.long_term.iter())
            {
                // Skip if memory type filter is specified and doesn't match
                if !query.memory_types.is_empty()
                    && !query.memory_types.contains(&memory_item.memory_type)
                {
                    continue;
                }

                // Check if item falls within time range
                if memory_item.created_at >= time_range.start
                    && memory_item.created_at <= time_range.end
                {
                    // Optional keyword filtering within temporal results
                    let passes_keyword_filter = if query.search_terms.is_empty() {
                        true
                    } else {
                        let content_lower = memory_item.content.to_lowercase();
                        query
                            .search_terms
                            .iter()
                            .any(|term| content_lower.contains(&term.to_lowercase()))
                    };

                    if passes_keyword_filter {
                        let importance_score = self.calculate_importance(memory_item);
                        let recency_score = self.calculate_recency_factor(memory_item);
                        let relevance_score = (importance_score + recency_score) / 2.0;

                        if relevance_score >= query.relevance_threshold {
                            results.push(ContextItem {
                                id: memory_item.id,
                                content: memory_item.content.clone(),
                                item_type: ContextItemType::Memory(memory_item.memory_type.clone()),
                                relevance_score,
                                timestamp: memory_item.created_at,
                                metadata: memory_item.metadata.clone(),
                            });
                        }
                    }
                }
            }

            // Search through episodic memory
            for episode in &context.memory.episodic_memory {
                if episode.timestamp >= time_range.start && episode.timestamp <= time_range.end {
                    let passes_keyword_filter = if query.search_terms.is_empty() {
                        true
                    } else {
                        let episode_content = format!("{} {}", episode.title, episode.description);
                        let content_lower = episode_content.to_lowercase();
                        query
                            .search_terms
                            .iter()
                            .any(|term| content_lower.contains(&term.to_lowercase()))
                    };

                    if passes_keyword_filter && episode.importance >= query.relevance_threshold {
                        results.push(ContextItem {
                            id: episode.id,
                            content: format!("{} {}", episode.title, episode.description),
                            item_type: ContextItemType::Episode,
                            relevance_score: episode.importance,
                            timestamp: episode.timestamp,
                            metadata: HashMap::new(),
                        });
                    }
                }
            }

            // Search through conversation history
            for conv_item in &context.conversation_history {
                if conv_item.timestamp >= time_range.start && conv_item.timestamp <= time_range.end
                {
                    let passes_keyword_filter = if query.search_terms.is_empty() {
                        true
                    } else {
                        let content_lower = conv_item.content.to_lowercase();
                        query
                            .search_terms
                            .iter()
                            .any(|term| content_lower.contains(&term.to_lowercase()))
                    };

                    if passes_keyword_filter {
                        // Calculate relevance based on recency within the time range
                        let time_since_start = conv_item
                            .timestamp
                            .duration_since(time_range.start)
                            .unwrap_or_default()
                            .as_secs() as f32;
                        let range_duration = time_range
                            .end
                            .duration_since(time_range.start)
                            .unwrap_or_default()
                            .as_secs() as f32;

                        let temporal_score = if range_duration > 0.0 {
                            1.0 - (time_since_start / range_duration)
                        } else {
                            1.0
                        };

                        if temporal_score >= query.relevance_threshold {
                            results.push(ContextItem {
                                id: conv_item.id,
                                content: conv_item.content.clone(),
                                item_type: ContextItemType::Conversation,
                                relevance_score: temporal_score,
                                timestamp: conv_item.timestamp,
                                metadata: HashMap::new(),
                            });
                        }
                    }
                }
            }

            // Sort by timestamp (most recent first) and limit results
            results.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
            results.truncate(query.max_results);

            Ok(results)
        } else {
            Ok(Vec::new())
        }
    }

    /// Perform similarity search using vector embeddings
    async fn similarity_search_memory(
        &self,
        agent_id: AgentId,
        query: &ContextQuery,
    ) -> Result<Vec<ContextItem>, ContextError> {
        if self.config.enable_vector_db && !query.search_terms.is_empty() {
            // Use vector database for similarity search
            let search_term = query.search_terms.join(" ");
            let query_embedding = self.generate_embeddings(&search_term).await?;

            // Search the vector database with semantic similarity
            let threshold = query.relevance_threshold;
            self.vector_db
                .semantic_search(agent_id, query_embedding, query.max_results, threshold)
                .await
        } else {
            // Fallback to embedding-based similarity if available in memory
            let contexts = self.contexts.read().await;
            if let Some(context) = contexts.get(&agent_id) {
                if query.search_terms.is_empty() {
                    return Ok(Vec::new());
                }

                // Generate query embedding
                let search_term = query.search_terms.join(" ");
                let query_embedding = match self.generate_embeddings(&search_term).await {
                    Ok(embedding) => embedding,
                    Err(_) => return Ok(Vec::new()), // Fall back to empty results if embedding fails
                };

                let mut results = Vec::new();

                // Compare with memory item embeddings
                for memory_item in context
                    .memory
                    .short_term
                    .iter()
                    .chain(context.memory.long_term.iter())
                {
                    // Skip if memory type filter is specified and doesn't match
                    if !query.memory_types.is_empty()
                        && !query.memory_types.contains(&memory_item.memory_type)
                    {
                        continue;
                    }

                    if let Some(ref item_embedding) = memory_item.embedding {
                        // Calculate cosine similarity
                        let similarity = self.cosine_similarity(&query_embedding, item_embedding);

                        if similarity >= query.relevance_threshold {
                            let importance_score = self.calculate_importance(memory_item);
                            let relevance_score = (similarity + importance_score) / 2.0;

                            results.push(ContextItem {
                                id: memory_item.id,
                                content: memory_item.content.clone(),
                                item_type: ContextItemType::Memory(memory_item.memory_type.clone()),
                                relevance_score,
                                timestamp: memory_item.created_at,
                                metadata: memory_item.metadata.clone(),
                            });
                        }
                    }
                }

                // Sort by relevance score (highest first) and limit results
                results.sort_by(|a, b| {
                    b.relevance_score
                        .partial_cmp(&a.relevance_score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                results.truncate(query.max_results);

                Ok(results)
            } else {
                Ok(Vec::new())
            }
        }
    }

    /// Perform hybrid search combining keyword and similarity search
    async fn hybrid_search_memory(
        &self,
        agent_id: AgentId,
        query: &ContextQuery,
    ) -> Result<Vec<ContextItem>, ContextError> {
        // Perform both keyword and similarity searches
        let keyword_results = self.keyword_search_memory(agent_id, query).await?;
        let similarity_results = self.similarity_search_memory(agent_id, query).await?;

        // Combine and deduplicate results
        let mut combined_results: HashMap<ContextId, ContextItem> = HashMap::new();

        // Weight factors for combining scores (configurable)
        let keyword_weight = 0.4;
        let similarity_weight = 0.6;

        // Add keyword results
        for mut item in keyword_results {
            item.relevance_score *= keyword_weight;
            combined_results.insert(item.id, item);
        }

        // Add similarity results, combining scores for duplicates
        for mut item in similarity_results {
            item.relevance_score *= similarity_weight;

            if let Some(existing_item) = combined_results.get_mut(&item.id) {
                // Combine scores for items found in both searches
                existing_item.relevance_score += item.relevance_score;
                // Take the higher of the two relevance scores as the final score
                existing_item.relevance_score =
                    existing_item.relevance_score.max(item.relevance_score);
            } else {
                combined_results.insert(item.id, item);
            }
        }

        // Convert to vector and sort by combined relevance score
        let mut final_results: Vec<ContextItem> = combined_results.into_values().collect();
        final_results.sort_by(|a, b| {
            b.relevance_score
                .partial_cmp(&a.relevance_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Filter by threshold and limit results
        final_results.retain(|item| item.relevance_score >= query.relevance_threshold);
        final_results.truncate(query.max_results);

        Ok(final_results)
    }

    /// Calculate cosine similarity between two vectors
    fn cosine_similarity(&self, a: &[f32], b: &[f32]) -> f32 {
        if a.len() != b.len() {
            return 0.0;
        }

        let dot_product: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        let magnitude_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let magnitude_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

        if magnitude_a == 0.0 || magnitude_b == 0.0 {
            0.0
        } else {
            dot_product / (magnitude_a * magnitude_b)
        }
    }

    /// Calculate knowledge relevance score for fallback search
    ///
    /// This method computes a meaningful relevance score based on:
    /// - Keyword match quality (exact vs partial matches)
    /// - Text coverage (how much of the query terms are found)
    /// - Knowledge confidence score
    /// - Content length factors
    ///
    /// Returns Some(score) if content matches query, None if no match
    fn calculate_knowledge_relevance(
        &self,
        content: &str,
        query: &str,
        confidence: f32,
    ) -> Option<f32> {
        let content_lower = content.to_lowercase();
        let query_lower = query.to_lowercase();
        let query_terms: Vec<&str> = query_lower.split_whitespace().collect();

        if query_terms.is_empty() {
            return None;
        }

        let mut total_match_score = 0.0f32;
        let mut matched_terms = 0;

        // Calculate match quality for each query term
        for term in &query_terms {
            if content_lower.contains(term) {
                matched_terms += 1;

                // Give higher score for exact word matches vs substring matches
                if content_lower.split_whitespace().any(|word| word == *term) {
                    total_match_score += 1.0; // Exact word match
                } else {
                    total_match_score += 0.6; // Substring match
                }

                // Bonus for terms appearing multiple times
                let occurrences = content_lower.matches(term).count();
                if occurrences > 1 {
                    total_match_score += (occurrences as f32 - 1.0) * 0.1;
                }
            }
        }

        // Return None if no matches found
        if matched_terms == 0 {
            return None;
        }

        // Calculate base relevance score
        let term_coverage = matched_terms as f32 / query_terms.len() as f32;
        let match_quality = total_match_score / query_terms.len() as f32;

        // Combine match quality, term coverage, and confidence
        let base_score = match_quality * 0.5 + term_coverage * 0.3 + confidence * 0.2;

        // Apply content length normalization
        let content_length_factor = if content.len() < 50 {
            1.0 // Short content gets full score
        } else if content.len() < 200 {
            0.95 // Medium content gets slight penalty
        } else {
            0.9 // Long content gets larger penalty for diluted relevance
        };

        // Apply position bonus if query terms appear early in content
        let position_bonus = if content_lower.starts_with(&query_lower) {
            0.1 // Exact prefix match
        } else if query_terms
            .iter()
            .any(|term| content_lower.starts_with(term))
        {
            0.05 // Partial prefix match
        } else {
            0.0
        };

        let final_score = (base_score + position_bonus) * content_length_factor;
        Some(final_score.clamp(0.0, 1.0))
    }

    /// Calculate trust score for shared knowledge based on usage patterns
    fn calculate_trust_score(&self, shared_item: &SharedKnowledgeItem) -> f32 {
        // Base trust score starts at 0.5
        let mut trust_score = 0.5;

        // Increase trust based on access count (more usage = more trusted)
        let access_factor = (shared_item.access_count as f32 + 1.0).ln() / 10.0;
        trust_score += access_factor;

        // Consider the type of knowledge (facts might be more trusted than patterns)
        let knowledge_factor = match &shared_item.knowledge {
            Knowledge::Fact(_) => 0.2,
            Knowledge::Procedure(_) => 0.1,
            Knowledge::Pattern(_) => 0.05,
        };
        trust_score += knowledge_factor;

        // Clamp between 0.0 and 1.0
        trust_score.clamp(0.0, 1.0)
    }

    /// Calculate accurate memory size in bytes for an agent context
    fn calculate_memory_size_bytes(&self, context: &AgentContext) -> usize {
        let mut total_size = 0;

        // Working memory size
        total_size += std::mem::size_of::<WorkingMemory>();
        for (key, value) in &context.memory.working_memory.variables {
            total_size += key.len();
            total_size += estimate_json_value_size(value);
        }
        for goal in &context.memory.working_memory.active_goals {
            total_size += goal.len();
        }
        if let Some(ref current_context) = context.memory.working_memory.current_context {
            total_size += current_context.len();
        }
        for focus in &context.memory.working_memory.attention_focus {
            total_size += focus.len();
        }

        // Short-term memory
        for item in &context.memory.short_term {
            total_size += estimate_memory_item_size(item);
        }

        // Long-term memory
        for item in &context.memory.long_term {
            total_size += estimate_memory_item_size(item);
        }

        // Episodic memory
        for episode in &context.memory.episodic_memory {
            total_size += estimate_episode_size(episode);
        }

        // Semantic memory
        for item in &context.memory.semantic_memory {
            total_size += estimate_semantic_memory_item_size(item);
        }

        // Knowledge base
        for fact in &context.knowledge_base.facts {
            total_size += estimate_knowledge_fact_size(fact);
        }
        for procedure in &context.knowledge_base.procedures {
            total_size += estimate_procedure_size(procedure);
        }
        for pattern in &context.knowledge_base.learned_patterns {
            total_size += estimate_pattern_size(pattern);
        }

        // Conversation history
        for item in &context.conversation_history {
            total_size += estimate_conversation_item_size(item);
        }

        // Metadata
        for (key, value) in &context.metadata {
            total_size += key.len() + value.len();
        }

        // Base struct overhead
        total_size += std::mem::size_of::<AgentContext>();

        total_size
    }

    /// Calculate retention statistics for archiving and deletion
    fn calculate_retention_statistics(&self, context: &AgentContext) -> RetentionStatus {
        let now = SystemTime::now();
        let retention_policy = &context.retention_policy;

        let mut items_to_archive = 0;
        let mut items_to_delete = 0;

        // Calculate cutoff times based on retention policy
        let memory_cutoff = now
            .checked_sub(retention_policy.memory_retention)
            .unwrap_or(SystemTime::UNIX_EPOCH);

        let knowledge_cutoff = now
            .checked_sub(retention_policy.knowledge_retention)
            .unwrap_or(SystemTime::UNIX_EPOCH);

        let conversation_cutoff = now
            .checked_sub(retention_policy.session_retention)
            .unwrap_or(SystemTime::UNIX_EPOCH);

        // Count memory items eligible for archiving
        for item in &context.memory.short_term {
            if item.created_at < memory_cutoff || item.last_accessed < memory_cutoff {
                items_to_archive += 1;
            }
        }

        for item in &context.memory.long_term {
            if item.created_at < memory_cutoff || item.last_accessed < memory_cutoff {
                items_to_archive += 1;
            }
        }

        for episode in &context.memory.episodic_memory {
            if episode.timestamp < memory_cutoff {
                items_to_archive += 1;
            }
        }

        // Semantic memory uses more conservative cutoff (2x memory retention)
        let semantic_cutoff = now
            .checked_sub(retention_policy.memory_retention * 2)
            .unwrap_or(SystemTime::UNIX_EPOCH);

        for item in &context.memory.semantic_memory {
            if item.created_at < semantic_cutoff {
                items_to_archive += 1;
            }
        }

        // Count knowledge items eligible for archiving
        for fact in &context.knowledge_base.facts {
            if fact.created_at < knowledge_cutoff {
                items_to_archive += 1;
            }
        }

        // For procedures, use success rate as archiving criteria
        for procedure in &context.knowledge_base.procedures {
            if procedure.success_rate < 0.3 {
                items_to_archive += 1;
            }
        }

        // For patterns, use confidence and occurrence count
        for pattern in &context.knowledge_base.learned_patterns {
            if pattern.confidence < 0.4 || pattern.occurrences < 2 {
                items_to_archive += 1;
            }
        }

        // Count conversation items eligible for archiving
        for item in &context.conversation_history {
            if item.timestamp < conversation_cutoff {
                items_to_archive += 1;
            }
        }

        // Calculate items to delete (very old items that exceed 2x retention period)
        let delete_cutoff_memory = now
            .checked_sub(retention_policy.memory_retention * 2)
            .unwrap_or(SystemTime::UNIX_EPOCH);

        let delete_cutoff_knowledge = now
            .checked_sub(retention_policy.knowledge_retention * 2)
            .unwrap_or(SystemTime::UNIX_EPOCH);

        let delete_cutoff_conversation = now
            .checked_sub(retention_policy.session_retention * 2)
            .unwrap_or(SystemTime::UNIX_EPOCH);

        // Count memory items eligible for deletion
        for item in &context.memory.short_term {
            if item.created_at < delete_cutoff_memory && item.last_accessed < delete_cutoff_memory {
                items_to_delete += 1;
            }
        }

        for item in &context.memory.long_term {
            if item.created_at < delete_cutoff_memory && item.last_accessed < delete_cutoff_memory {
                items_to_delete += 1;
            }
        }

        // Count knowledge items eligible for deletion
        for fact in &context.knowledge_base.facts {
            if fact.created_at < delete_cutoff_knowledge && !fact.verified {
                items_to_delete += 1;
            }
        }

        // Count conversation items eligible for deletion
        for item in &context.conversation_history {
            if item.timestamp < delete_cutoff_conversation {
                items_to_delete += 1;
            }
        }

        // Calculate next cleanup time (daily cleanup schedule)
        let next_cleanup = now + Duration::from_secs(86400); // 24 hours

        RetentionStatus {
            items_to_archive,
            items_to_delete,
            next_cleanup,
        }
    }

    /// Archive old memory items from context
    async fn archive_memory_items(
        &self,
        context: &mut AgentContext,
        before: SystemTime,
        archived_context: &mut ArchivedContext,
    ) -> Result<u32, ContextError> {
        let mut archived_count = 0u32;
        let retention_policy = &context.retention_policy;

        // Calculate cutoff times based on retention policy
        let memory_cutoff = before
            .checked_sub(retention_policy.memory_retention)
            .unwrap_or(SystemTime::UNIX_EPOCH);

        // Archive short-term memory items
        let mut retained_short_term = Vec::new();
        for item in context.memory.short_term.drain(..) {
            if item.created_at < memory_cutoff || item.last_accessed < memory_cutoff {
                archived_context.memory.short_term.push(item);
                archived_count += 1;
            } else {
                retained_short_term.push(item);
            }
        }
        context.memory.short_term = retained_short_term;

        // Archive long-term memory items (older than retention policy)
        let mut retained_long_term = Vec::new();
        for item in context.memory.long_term.drain(..) {
            if item.created_at < memory_cutoff || item.last_accessed < memory_cutoff {
                archived_context.memory.long_term.push(item);
                archived_count += 1;
            } else {
                retained_long_term.push(item);
            }
        }
        context.memory.long_term = retained_long_term;

        // Archive episodic memory
        let mut retained_episodes = Vec::new();
        for episode in context.memory.episodic_memory.drain(..) {
            if episode.timestamp < memory_cutoff {
                archived_context.memory.episodic_memory.push(episode);
                archived_count += 1;
            } else {
                retained_episodes.push(episode);
            }
        }
        context.memory.episodic_memory = retained_episodes;

        // Archive semantic memory (less aggressive - only archive very old items)
        let semantic_cutoff = before
            .checked_sub(retention_policy.memory_retention * 2)
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let mut retained_semantic = Vec::new();
        for item in context.memory.semantic_memory.drain(..) {
            if item.created_at < semantic_cutoff {
                archived_context.memory.semantic_memory.push(item);
                archived_count += 1;
            } else {
                retained_semantic.push(item);
            }
        }
        context.memory.semantic_memory = retained_semantic;

        Ok(archived_count)
    }

    /// Archive old conversation history
    async fn archive_conversation_history(
        &self,
        context: &mut AgentContext,
        before: SystemTime,
        archived_context: &mut ArchivedContext,
    ) -> Result<u32, ContextError> {
        let mut archived_count = 0u32;
        let retention_policy = &context.retention_policy;

        // Calculate cutoff time for conversation history
        let conversation_cutoff = before
            .checked_sub(retention_policy.session_retention)
            .unwrap_or(SystemTime::UNIX_EPOCH);

        // Archive old conversation items
        let mut retained_conversations = Vec::new();
        for item in context.conversation_history.drain(..) {
            if item.timestamp < conversation_cutoff {
                archived_context.conversation_history.push(item);
                archived_count += 1;
            } else {
                retained_conversations.push(item);
            }
        }
        context.conversation_history = retained_conversations;

        Ok(archived_count)
    }

    /// Archive old knowledge base items
    async fn archive_knowledge_items(
        &self,
        context: &mut AgentContext,
        before: SystemTime,
        archived_context: &mut ArchivedContext,
    ) -> Result<u32, ContextError> {
        let mut archived_count = 0u32;
        let retention_policy = &context.retention_policy;

        // Calculate cutoff time for knowledge items
        let knowledge_cutoff = before
            .checked_sub(retention_policy.knowledge_retention)
            .unwrap_or(SystemTime::UNIX_EPOCH);

        // Archive old facts
        let mut retained_facts = Vec::new();
        for fact in context.knowledge_base.facts.drain(..) {
            if fact.created_at < knowledge_cutoff {
                archived_context.knowledge_base.facts.push(fact);
                archived_count += 1;
            } else {
                retained_facts.push(fact);
            }
        }
        context.knowledge_base.facts = retained_facts;

        // Archive old procedures (be more conservative)
        // Note: Procedures don't have created_at in current schema, so we use success_rate as proxy
        let mut retained_procedures = Vec::new();
        for procedure in context.knowledge_base.procedures.drain(..) {
            // Archive procedures with very low success rate
            if procedure.success_rate < 0.3 {
                archived_context.knowledge_base.procedures.push(procedure);
                archived_count += 1;
            } else {
                retained_procedures.push(procedure);
            }
        }
        context.knowledge_base.procedures = retained_procedures;

        // Archive old patterns (if confidence is low or very old)
        let mut retained_patterns = Vec::new();
        for pattern in context.knowledge_base.learned_patterns.drain(..) {
            if pattern.confidence < 0.4 || pattern.occurrences < 2 {
                archived_context
                    .knowledge_base
                    .learned_patterns
                    .push(pattern);
                archived_count += 1;
            } else {
                retained_patterns.push(pattern);
            }
        }
        context.knowledge_base.learned_patterns = retained_patterns;

        Ok(archived_count)
    }

    /// Save archived context to persistent storage
    async fn save_archived_context(
        &self,
        agent_id: AgentId,
        archived_context: &ArchivedContext,
    ) -> Result<(), ContextError> {
        if !self.config.enable_persistence {
            return Ok(());
        }

        // Get archive directory path
        let archive_dir = self.get_archive_directory_path(agent_id).await?;

        // Create archive filename with timestamp
        let timestamp = archived_context
            .archived_at
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let archive_filename = format!("archive_{}.json", timestamp);
        let archive_path = archive_dir.join(archive_filename);

        // Serialize archived context
        let archive_data = serde_json::to_vec_pretty(archived_context).map_err(|e| {
            ContextError::SerializationError {
                reason: format!("Failed to serialize archived context: {}", e),
            }
        })?;

        // Write to archive file with compression if enabled
        let final_data = if self.config.persistence_config.enable_compression {
            self.compress_data(&archive_data)?
        } else {
            archive_data
        };

        // Ensure atomic write operation
        let temp_path = archive_path.with_extension("tmp");
        fs::write(&temp_path, &final_data)
            .await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to write archive file: {}", e),
            })?;

        // Atomically move temp file to final location
        fs::rename(&temp_path, &archive_path)
            .await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to finalize archive file: {}", e),
            })?;

        tracing::info!(
            "Saved archived context for agent {} to {}",
            agent_id,
            archive_path.display()
        );

        Ok(())
    }

    /// Get archive directory path for an agent
    async fn get_archive_directory_path(&self, agent_id: AgentId) -> Result<PathBuf, ContextError> {
        let archive_dir = self
            .config
            .persistence_config
            .agent_contexts_path()
            .join("archives")
            .join(agent_id.to_string());

        // Ensure archive directory exists
        fs::create_dir_all(&archive_dir)
            .await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to create archive directory: {}", e),
            })?;

        Ok(archive_dir)
    }

    /// Compress data using gzip
    fn compress_data(&self, data: &[u8]) -> Result<Vec<u8>, ContextError> {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(data)
            .map_err(|e| ContextError::SerializationError {
                reason: format!("Failed to compress archive data: {}", e),
            })?;
        encoder
            .finish()
            .map_err(|e| ContextError::SerializationError {
                reason: format!("Failed to finalize compression: {}", e),
            })
    }

    /// Convert Knowledge to KnowledgeItem for vector storage
    fn knowledge_to_item(
        &self,
        knowledge: &Knowledge,
        knowledge_id: KnowledgeId,
    ) -> Result<KnowledgeItem, ContextError> {
        match knowledge {
            Knowledge::Fact(fact) => {
                Ok(KnowledgeItem {
                    id: knowledge_id,
                    content: format!("{} {} {}", fact.subject, fact.predicate, fact.object),
                    knowledge_type: KnowledgeType::Fact,
                    confidence: fact.confidence,
                    relevance_score: 1.0, // Initial relevance
                    source: fact.source.clone(),
                    created_at: fact.created_at,
                })
            }
            Knowledge::Procedure(procedure) => {
                Ok(KnowledgeItem {
                    id: knowledge_id,
                    content: format!("{}: {}", procedure.name, procedure.description),
                    knowledge_type: KnowledgeType::Procedure,
                    confidence: procedure.success_rate,
                    relevance_score: 1.0, // Initial relevance
                    source: KnowledgeSource::Learning,
                    created_at: SystemTime::now(),
                })
            }
            Knowledge::Pattern(pattern) => {
                Ok(KnowledgeItem {
                    id: knowledge_id,
                    content: format!("Pattern: {}", pattern.description),
                    knowledge_type: KnowledgeType::Pattern,
                    confidence: pattern.confidence,
                    relevance_score: 1.0, // Initial relevance
                    source: KnowledgeSource::Learning,
                    created_at: SystemTime::now(),
                })
            }
        }
    }
}

/// Estimate memory size of a JSON value in bytes
fn estimate_json_value_size(value: &Value) -> usize {
    match value {
        Value::Null => 4,
        Value::Bool(_) => 1,
        Value::Number(n) => std::mem::size_of::<f64>() + n.to_string().len(),
        Value::String(s) => s.len(),
        Value::Array(arr) => {
            arr.iter().map(estimate_json_value_size).sum::<usize>()
                + std::mem::size_of::<Vec<Value>>()
        }
        Value::Object(obj) => {
            obj.iter()
                .map(|(k, v)| k.len() + estimate_json_value_size(v))
                .sum::<usize>()
                + std::mem::size_of::<serde_json::Map<String, Value>>()
        }
    }
}

/// Estimate memory size of a MemoryItem in bytes
fn estimate_memory_item_size(item: &MemoryItem) -> usize {
    let mut size = std::mem::size_of::<MemoryItem>();
    size += item.content.len();

    // Estimate embedding size if present
    if let Some(ref embedding) = item.embedding {
        size += embedding.len() * std::mem::size_of::<f32>();
    }

    // Estimate metadata size
    for (key, value) in &item.metadata {
        size += key.len() + value.len();
    }

    size
}

/// Estimate memory size of an Episode in bytes
fn estimate_episode_size(episode: &Episode) -> usize {
    let mut size = std::mem::size_of::<Episode>();
    size += episode.title.len();
    size += episode.description.len();

    if let Some(ref outcome) = episode.outcome {
        size += outcome.len();
    }

    for event in &episode.events {
        size += event.action.len();
        size += event.result.len();
        for (key, value) in &event.context {
            size += key.len() + value.len();
        }
    }

    for lesson in &episode.lessons_learned {
        size += lesson.len();
    }

    size
}

/// Estimate memory size of a SemanticMemoryItem in bytes
fn estimate_semantic_memory_item_size(item: &SemanticMemoryItem) -> usize {
    let mut size = std::mem::size_of::<SemanticMemoryItem>();
    size += item.concept.len();

    for relationship in &item.relationships {
        size += relationship.target_concept.len();
        size += std::mem::size_of::<ConceptRelationship>();
    }

    for (key, value) in &item.properties {
        size += key.len() + estimate_json_value_size(value);
    }

    size
}

/// Estimate memory size of a KnowledgeFact in bytes
fn estimate_knowledge_fact_size(fact: &KnowledgeFact) -> usize {
    std::mem::size_of::<KnowledgeFact>()
        + fact.subject.len()
        + fact.predicate.len()
        + fact.object.len()
}

/// Estimate memory size of a Procedure in bytes
fn estimate_procedure_size(procedure: &Procedure) -> usize {
    let mut size = std::mem::size_of::<Procedure>();
    size += procedure.name.len();
    size += procedure.description.len();

    for step in &procedure.steps {
        size += step.action.len();
        size += step.expected_result.len();
        if let Some(ref error_handling) = step.error_handling {
            size += error_handling.len();
        }
    }

    for condition in &procedure.preconditions {
        size += condition.len();
    }

    for condition in &procedure.postconditions {
        size += condition.len();
    }

    size
}

/// Estimate memory size of a Pattern in bytes
fn estimate_pattern_size(pattern: &Pattern) -> usize {
    let mut size = std::mem::size_of::<Pattern>();
    size += pattern.name.len();
    size += pattern.description.len();

    for condition in &pattern.conditions {
        size += condition.len();
    }

    for outcome in &pattern.outcomes {
        size += outcome.len();
    }

    size
}

/// Estimate memory size of a ConversationItem in bytes
fn estimate_conversation_item_size(item: &ConversationItem) -> usize {
    let mut size = std::mem::size_of::<ConversationItem>();
    size += item.content.len();

    // Estimate size of ID vectors
    size += item.context_used.len() * std::mem::size_of::<ContextId>();
    size += item.knowledge_used.len() * std::mem::size_of::<KnowledgeId>();

    size
}

#[async_trait]
impl ContextManager for StandardContextManager {
    async fn store_context(
        &self,
        agent_id: AgentId,
        mut context: AgentContext,
    ) -> Result<ContextId, ContextError> {
        self.validate_access(agent_id, "store_context").await?;

        context.updated_at = SystemTime::now();
        let context_id = ContextId::new();

        // Store in persistent storage if enabled
        if self.config.enable_persistence {
            self.persistence.save_context(agent_id, &context).await?;
        }

        // Store in memory cache
        let mut contexts = self.contexts.write().await;
        contexts.insert(agent_id, context);

        Ok(context_id)
    }

    async fn retrieve_context(
        &self,
        agent_id: AgentId,
        session_id: Option<SessionId>,
    ) -> Result<Option<AgentContext>, ContextError> {
        self.validate_access(agent_id, "retrieve_context").await?;

        // First check memory cache
        {
            let contexts = self.contexts.read().await;
            if let Some(context) = contexts.get(&agent_id) {
                // If session_id is specified, check if it matches
                if let Some(sid) = session_id {
                    if context.session_id == sid {
                        return Ok(Some(context.clone()));
                    }
                } else {
                    return Ok(Some(context.clone()));
                }
            }
        }

        // If not in memory and persistence is enabled, try loading from storage
        if self.config.enable_persistence {
            if let Some(context) = self.persistence.load_context(agent_id).await? {
                // Check session_id if specified
                if let Some(sid) = session_id {
                    if context.session_id != sid {
                        return Ok(None);
                    }
                }

                // Cache the loaded context
                let mut contexts = self.contexts.write().await;
                contexts.insert(agent_id, context.clone());

                return Ok(Some(context));
            }
        }

        Ok(None)
    }

    async fn query_context(
        &self,
        agent_id: AgentId,
        query: ContextQuery,
    ) -> Result<Vec<ContextItem>, ContextError> {
        self.validate_access(agent_id, "query_context").await?;

        match query.query_type {
            QueryType::Semantic => {
                let search_term = query.search_terms.join(" ");
                self.semantic_search_memory(agent_id, &search_term, query.max_results)
                    .await
            }
            QueryType::Keyword => self.keyword_search_memory(agent_id, &query).await,
            QueryType::Temporal => self.temporal_search_memory(agent_id, &query).await,
            QueryType::Similarity => self.similarity_search_memory(agent_id, &query).await,
            QueryType::Hybrid => self.hybrid_search_memory(agent_id, &query).await,
        }
    }

    async fn update_memory(
        &self,
        agent_id: AgentId,
        memory_updates: Vec<MemoryUpdate>,
    ) -> Result<(), ContextError> {
        self.validate_access(agent_id, "update_memory").await?;

        let mut contexts = self.contexts.write().await;
        if let Some(context) = contexts.get_mut(&agent_id) {
            for update in memory_updates {
                match update.operation {
                    UpdateOperation::Add => {
                        match update.target {
                            MemoryTarget::ShortTerm(_) => {
                                // Parse data to create MemoryItem for short-term memory
                                if let Ok(memory_item_data) =
                                    serde_json::from_value::<serde_json::Map<String, Value>>(
                                        update.data,
                                    )
                                {
                                    let memory_item = MemoryItem {
                                        id: ContextId::new(),
                                        content: memory_item_data
                                            .get("content")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_string(),
                                        memory_type: memory_item_data
                                            .get("memory_type")
                                            .and_then(|v| serde_json::from_value(v.clone()).ok())
                                            .unwrap_or(MemoryType::Factual),
                                        importance: memory_item_data
                                            .get("importance")
                                            .and_then(|v| v.as_f64())
                                            .unwrap_or(0.5)
                                            as f32,
                                        access_count: 0,
                                        last_accessed: SystemTime::now(),
                                        created_at: SystemTime::now(),
                                        embedding: None,
                                        metadata: memory_item_data
                                            .get("metadata")
                                            .and_then(|v| serde_json::from_value(v.clone()).ok())
                                            .unwrap_or_default(),
                                    };
                                    context.memory.short_term.push(memory_item);
                                }
                            }
                            MemoryTarget::LongTerm(_) => {
                                // Parse data to create MemoryItem for long-term memory
                                if let Ok(memory_item_data) =
                                    serde_json::from_value::<serde_json::Map<String, Value>>(
                                        update.data,
                                    )
                                {
                                    let memory_item = MemoryItem {
                                        id: ContextId::new(),
                                        content: memory_item_data
                                            .get("content")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_string(),
                                        memory_type: memory_item_data
                                            .get("memory_type")
                                            .and_then(|v| serde_json::from_value(v.clone()).ok())
                                            .unwrap_or(MemoryType::Factual),
                                        importance: memory_item_data
                                            .get("importance")
                                            .and_then(|v| v.as_f64())
                                            .unwrap_or(0.5)
                                            as f32,
                                        access_count: 0,
                                        last_accessed: SystemTime::now(),
                                        created_at: SystemTime::now(),
                                        embedding: None,
                                        metadata: memory_item_data
                                            .get("metadata")
                                            .and_then(|v| serde_json::from_value(v.clone()).ok())
                                            .unwrap_or_default(),
                                    };
                                    context.memory.long_term.push(memory_item);
                                }
                            }
                            MemoryTarget::Working(key) => {
                                // Add to working memory variables
                                context
                                    .memory
                                    .working_memory
                                    .variables
                                    .insert(key, update.data);
                            }
                            MemoryTarget::Episodic(_) => {
                                // Parse data to create Episode for episodic memory
                                if let Ok(episode_data) =
                                    serde_json::from_value::<serde_json::Map<String, Value>>(
                                        update.data,
                                    )
                                {
                                    let episode = Episode {
                                        id: ContextId::new(),
                                        title: episode_data
                                            .get("title")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("Untitled Episode")
                                            .to_string(),
                                        description: episode_data
                                            .get("description")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_string(),
                                        events: episode_data
                                            .get("events")
                                            .and_then(|v| serde_json::from_value(v.clone()).ok())
                                            .unwrap_or_default(),
                                        outcome: episode_data
                                            .get("outcome")
                                            .and_then(|v| v.as_str())
                                            .map(|s| s.to_string()),
                                        lessons_learned: episode_data
                                            .get("lessons_learned")
                                            .and_then(|v| serde_json::from_value(v.clone()).ok())
                                            .unwrap_or_default(),
                                        timestamp: SystemTime::now(),
                                        importance: episode_data
                                            .get("importance")
                                            .and_then(|v| v.as_f64())
                                            .unwrap_or(0.5)
                                            as f32,
                                    };
                                    context.memory.episodic_memory.push(episode);
                                }
                            }
                            MemoryTarget::Semantic(_) => {
                                // Parse data to create SemanticMemoryItem for semantic memory
                                if let Ok(semantic_data) =
                                    serde_json::from_value::<serde_json::Map<String, Value>>(
                                        update.data,
                                    )
                                {
                                    let semantic_item = SemanticMemoryItem {
                                        id: ContextId::new(),
                                        concept: semantic_data
                                            .get("concept")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_string(),
                                        relationships: semantic_data
                                            .get("relationships")
                                            .and_then(|v| serde_json::from_value(v.clone()).ok())
                                            .unwrap_or_default(),
                                        properties: semantic_data
                                            .get("properties")
                                            .and_then(|v| serde_json::from_value(v.clone()).ok())
                                            .unwrap_or_default(),
                                        confidence: semantic_data
                                            .get("confidence")
                                            .and_then(|v| v.as_f64())
                                            .unwrap_or(0.5)
                                            as f32,
                                        created_at: SystemTime::now(),
                                        updated_at: SystemTime::now(),
                                    };
                                    context.memory.semantic_memory.push(semantic_item);
                                }
                            }
                        }
                    }
                    UpdateOperation::Update => {
                        match update.target {
                            MemoryTarget::ShortTerm(target_id) => {
                                // Update existing short-term memory item
                                if let Some(memory_item) = context
                                    .memory
                                    .short_term
                                    .iter_mut()
                                    .find(|item| item.id == target_id)
                                {
                                    if let Ok(update_data) =
                                        serde_json::from_value::<serde_json::Map<String, Value>>(
                                            update.data,
                                        )
                                    {
                                        if let Some(content) =
                                            update_data.get("content").and_then(|v| v.as_str())
                                        {
                                            memory_item.content = content.to_string();
                                        }
                                        if let Some(importance) =
                                            update_data.get("importance").and_then(|v| v.as_f64())
                                        {
                                            memory_item.importance = importance as f32;
                                        }
                                        if let Some(metadata) = update_data.get("metadata") {
                                            if let Ok(new_metadata) =
                                                serde_json::from_value(metadata.clone())
                                            {
                                                memory_item.metadata = new_metadata;
                                            }
                                        }
                                        memory_item.last_accessed = SystemTime::now();
                                        memory_item.access_count += 1;
                                    }
                                }
                            }
                            MemoryTarget::LongTerm(target_id) => {
                                // Update existing long-term memory item
                                if let Some(memory_item) = context
                                    .memory
                                    .long_term
                                    .iter_mut()
                                    .find(|item| item.id == target_id)
                                {
                                    if let Ok(update_data) =
                                        serde_json::from_value::<serde_json::Map<String, Value>>(
                                            update.data,
                                        )
                                    {
                                        if let Some(content) =
                                            update_data.get("content").and_then(|v| v.as_str())
                                        {
                                            memory_item.content = content.to_string();
                                        }
                                        if let Some(importance) =
                                            update_data.get("importance").and_then(|v| v.as_f64())
                                        {
                                            memory_item.importance = importance as f32;
                                        }
                                        if let Some(metadata) = update_data.get("metadata") {
                                            if let Ok(new_metadata) =
                                                serde_json::from_value(metadata.clone())
                                            {
                                                memory_item.metadata = new_metadata;
                                            }
                                        }
                                        memory_item.last_accessed = SystemTime::now();
                                        memory_item.access_count += 1;
                                    }
                                }
                            }
                            MemoryTarget::Working(key) => {
                                // Update working memory variable
                                context
                                    .memory
                                    .working_memory
                                    .variables
                                    .insert(key, update.data);
                            }
                            MemoryTarget::Episodic(target_id) => {
                                // Update existing episode
                                if let Some(episode) = context
                                    .memory
                                    .episodic_memory
                                    .iter_mut()
                                    .find(|ep| ep.id == target_id)
                                {
                                    if let Ok(update_data) =
                                        serde_json::from_value::<serde_json::Map<String, Value>>(
                                            update.data,
                                        )
                                    {
                                        if let Some(title) =
                                            update_data.get("title").and_then(|v| v.as_str())
                                        {
                                            episode.title = title.to_string();
                                        }
                                        if let Some(description) =
                                            update_data.get("description").and_then(|v| v.as_str())
                                        {
                                            episode.description = description.to_string();
                                        }
                                        if let Some(outcome) =
                                            update_data.get("outcome").and_then(|v| v.as_str())
                                        {
                                            episode.outcome = Some(outcome.to_string());
                                        }
                                        if let Some(importance) =
                                            update_data.get("importance").and_then(|v| v.as_f64())
                                        {
                                            episode.importance = importance as f32;
                                        }
                                        if let Some(lessons) = update_data.get("lessons_learned") {
                                            if let Ok(new_lessons) =
                                                serde_json::from_value(lessons.clone())
                                            {
                                                episode.lessons_learned = new_lessons;
                                            }
                                        }
                                    }
                                }
                            }
                            MemoryTarget::Semantic(target_id) => {
                                // Update existing semantic memory item
                                if let Some(semantic_item) = context
                                    .memory
                                    .semantic_memory
                                    .iter_mut()
                                    .find(|item| item.id == target_id)
                                {
                                    if let Ok(update_data) =
                                        serde_json::from_value::<serde_json::Map<String, Value>>(
                                            update.data,
                                        )
                                    {
                                        if let Some(concept) =
                                            update_data.get("concept").and_then(|v| v.as_str())
                                        {
                                            semantic_item.concept = concept.to_string();
                                        }
                                        if let Some(confidence) =
                                            update_data.get("confidence").and_then(|v| v.as_f64())
                                        {
                                            semantic_item.confidence = confidence as f32;
                                        }
                                        if let Some(relationships) =
                                            update_data.get("relationships")
                                        {
                                            if let Ok(new_relationships) =
                                                serde_json::from_value(relationships.clone())
                                            {
                                                semantic_item.relationships = new_relationships;
                                            }
                                        }
                                        if let Some(properties) = update_data.get("properties") {
                                            if let Ok(new_properties) =
                                                serde_json::from_value(properties.clone())
                                            {
                                                semantic_item.properties = new_properties;
                                            }
                                        }
                                        semantic_item.updated_at = SystemTime::now();
                                    }
                                }
                            }
                        }
                    }
                    UpdateOperation::Delete => {
                        match update.target {
                            MemoryTarget::ShortTerm(target_id) => {
                                // Remove from short-term memory
                                context
                                    .memory
                                    .short_term
                                    .retain(|item| item.id != target_id);
                            }
                            MemoryTarget::LongTerm(target_id) => {
                                // Remove from long-term memory
                                context.memory.long_term.retain(|item| item.id != target_id);
                            }
                            MemoryTarget::Working(key) => {
                                // Remove from working memory variables
                                context.memory.working_memory.variables.remove(&key);
                            }
                            MemoryTarget::Episodic(target_id) => {
                                // Remove from episodic memory
                                context
                                    .memory
                                    .episodic_memory
                                    .retain(|ep| ep.id != target_id);
                            }
                            MemoryTarget::Semantic(target_id) => {
                                // Remove from semantic memory
                                context
                                    .memory
                                    .semantic_memory
                                    .retain(|item| item.id != target_id);
                            }
                        }
                    }
                    UpdateOperation::Increment => {
                        match update.target {
                            MemoryTarget::ShortTerm(target_id) => {
                                // Increment numeric fields in short-term memory
                                if let Some(memory_item) = context
                                    .memory
                                    .short_term
                                    .iter_mut()
                                    .find(|item| item.id == target_id)
                                {
                                    if let Ok(increment_data) =
                                        serde_json::from_value::<serde_json::Map<String, Value>>(
                                            update.data,
                                        )
                                    {
                                        if let Some(importance_increment) = increment_data
                                            .get("importance")
                                            .and_then(|v| v.as_f64())
                                        {
                                            memory_item.importance = (memory_item.importance
                                                + importance_increment as f32)
                                                .clamp(0.0, 1.0);
                                        }
                                        if let Some(access_increment) = increment_data
                                            .get("access_count")
                                            .and_then(|v| v.as_u64())
                                        {
                                            memory_item.access_count = memory_item
                                                .access_count
                                                .saturating_add(access_increment as u32);
                                        }
                                        memory_item.last_accessed = SystemTime::now();
                                    }
                                }
                            }
                            MemoryTarget::LongTerm(target_id) => {
                                // Increment numeric fields in long-term memory
                                if let Some(memory_item) = context
                                    .memory
                                    .long_term
                                    .iter_mut()
                                    .find(|item| item.id == target_id)
                                {
                                    if let Ok(increment_data) =
                                        serde_json::from_value::<serde_json::Map<String, Value>>(
                                            update.data,
                                        )
                                    {
                                        if let Some(importance_increment) = increment_data
                                            .get("importance")
                                            .and_then(|v| v.as_f64())
                                        {
                                            memory_item.importance = (memory_item.importance
                                                + importance_increment as f32)
                                                .clamp(0.0, 1.0);
                                        }
                                        if let Some(access_increment) = increment_data
                                            .get("access_count")
                                            .and_then(|v| v.as_u64())
                                        {
                                            memory_item.access_count = memory_item
                                                .access_count
                                                .saturating_add(access_increment as u32);
                                        }
                                        memory_item.last_accessed = SystemTime::now();
                                    }
                                }
                            }
                            MemoryTarget::Working(key) => {
                                // Increment numeric value in working memory
                                if let Some(existing_value) =
                                    context.memory.working_memory.variables.get(&key)
                                {
                                    if let (Some(current), Some(increment)) =
                                        (existing_value.as_f64(), update.data.as_f64())
                                    {
                                        let new_value = current + increment;
                                        context.memory.working_memory.variables.insert(
                                            key,
                                            Value::Number(
                                                serde_json::Number::from_f64(new_value)
                                                    .unwrap_or_else(|| serde_json::Number::from(0)),
                                            ),
                                        );
                                    } else if let (Some(current), Some(increment)) =
                                        (existing_value.as_i64(), update.data.as_i64())
                                    {
                                        let new_value = current.saturating_add(increment);
                                        context.memory.working_memory.variables.insert(
                                            key,
                                            Value::Number(serde_json::Number::from(new_value)),
                                        );
                                    }
                                }
                            }
                            MemoryTarget::Episodic(target_id) => {
                                // Increment numeric fields in episodic memory
                                if let Some(episode) = context
                                    .memory
                                    .episodic_memory
                                    .iter_mut()
                                    .find(|ep| ep.id == target_id)
                                {
                                    if let Some(importance_increment) = update.data.as_f64() {
                                        episode.importance = (episode.importance
                                            + importance_increment as f32)
                                            .clamp(0.0, 1.0);
                                    }
                                }
                            }
                            MemoryTarget::Semantic(target_id) => {
                                // Increment numeric fields in semantic memory
                                if let Some(semantic_item) = context
                                    .memory
                                    .semantic_memory
                                    .iter_mut()
                                    .find(|item| item.id == target_id)
                                {
                                    if let Some(confidence_increment) = update.data.as_f64() {
                                        semantic_item.confidence = (semantic_item.confidence
                                            + confidence_increment as f32)
                                            .clamp(0.0, 1.0);
                                        semantic_item.updated_at = SystemTime::now();
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Update context timestamp
            context.updated_at = SystemTime::now();

            // Persist changes to storage if enabled
            if self.config.enable_persistence {
                if let Err(e) = self.persistence.save_context(agent_id, context).await {
                    tracing::error!(
                        "Failed to persist memory updates for agent {}: {}",
                        agent_id,
                        e
                    );
                    return Err(e);
                }
            }
        } else {
            return Err(ContextError::NotFound {
                id: ContextId::new(),
            });
        }

        Ok(())
    }

    async fn add_knowledge(
        &self,
        agent_id: AgentId,
        knowledge: Knowledge,
    ) -> Result<KnowledgeId, ContextError> {
        self.validate_access(agent_id, "add_knowledge").await?;

        let knowledge_id = KnowledgeId::new();

        // Store in vector database if enabled
        if self.config.enable_vector_db {
            let knowledge_item = self.knowledge_to_item(&knowledge, knowledge_id)?;
            let embedding = self.generate_embeddings(&knowledge_item.content).await?;
            let _vector_id = self
                .vector_db
                .store_knowledge_item(&knowledge_item, embedding)
                .await?;
        }

        // Also store in local context for backward compatibility
        let mut contexts = self.contexts.write().await;
        if let Some(context) = contexts.get_mut(&agent_id) {
            match knowledge {
                Knowledge::Fact(fact) => {
                    context.knowledge_base.facts.push(fact);
                }
                Knowledge::Procedure(procedure) => {
                    context.knowledge_base.procedures.push(procedure);
                }
                Knowledge::Pattern(pattern) => {
                    context.knowledge_base.learned_patterns.push(pattern);
                }
            }
            context.updated_at = SystemTime::now();
        }

        Ok(knowledge_id)
    }

    async fn search_knowledge(
        &self,
        agent_id: AgentId,
        query: &str,
        limit: usize,
    ) -> Result<Vec<KnowledgeItem>, ContextError> {
        self.validate_access(agent_id, "search_knowledge").await?;

        if self.config.enable_vector_db {
            // Generate embeddings for the query
            let query_embedding = self.generate_embeddings(query).await?;

            // Search the vector database for knowledge items
            self.vector_db
                .search_knowledge_base(agent_id, query_embedding, limit)
                .await
        } else {
            // Fallback to simple keyword search
            let contexts = self.contexts.read().await;
            if let Some(context) = contexts.get(&agent_id) {
                let mut results = Vec::new();

                // Search facts
                for fact in &context.knowledge_base.facts {
                    let content = format!("{} {} {}", fact.subject, fact.predicate, fact.object);
                    if let Some(relevance_score) =
                        self.calculate_knowledge_relevance(&content, query, fact.confidence)
                    {
                        results.push(KnowledgeItem {
                            id: fact.id,
                            content,
                            knowledge_type: KnowledgeType::Fact,
                            confidence: fact.confidence,
                            relevance_score,
                            source: fact.source.clone(),
                            created_at: fact.created_at,
                        });
                    }
                }

                // Search procedures
                for procedure in &context.knowledge_base.procedures {
                    let searchable_content =
                        format!("{} {}", procedure.name, procedure.description);
                    if let Some(relevance_score) = self.calculate_knowledge_relevance(
                        &searchable_content,
                        query,
                        procedure.success_rate,
                    ) {
                        results.push(KnowledgeItem {
                            id: procedure.id,
                            content: format!("{}: {}", procedure.name, procedure.description),
                            knowledge_type: KnowledgeType::Procedure,
                            confidence: procedure.success_rate,
                            relevance_score,
                            source: KnowledgeSource::Learning,
                            created_at: SystemTime::now(), // Procedures don't store creation time in current schema
                        });
                    }
                }

                // Search patterns
                for pattern in &context.knowledge_base.learned_patterns {
                    let searchable_content = format!("{} {}", pattern.name, pattern.description);
                    if let Some(relevance_score) = self.calculate_knowledge_relevance(
                        &searchable_content,
                        query,
                        pattern.confidence,
                    ) {
                        results.push(KnowledgeItem {
                            id: pattern.id,
                            content: format!("Pattern: {}", pattern.description),
                            knowledge_type: KnowledgeType::Pattern,
                            confidence: pattern.confidence,
                            relevance_score,
                            source: KnowledgeSource::Learning,
                            created_at: SystemTime::now(), // Patterns don't store creation time in current schema
                        });
                    }
                }

                // Sort by relevance score (highest first) and limit results
                results.sort_by(|a, b| {
                    b.relevance_score
                        .partial_cmp(&a.relevance_score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });

                results.truncate(limit);
                Ok(results)
            } else {
                Ok(Vec::new())
            }
        }
    }

    async fn share_knowledge(
        &self,
        from_agent: AgentId,
        _to_agent: AgentId,
        knowledge_id: KnowledgeId,
        access_level: AccessLevel,
    ) -> Result<(), ContextError> {
        self.validate_access(from_agent, "share_knowledge").await?;

        // Find the knowledge item in the source agent's knowledge base
        let contexts = self.contexts.read().await;
        if let Some(from_context) = contexts.get(&from_agent) {
            // Find the knowledge item
            let knowledge = if let Some(fact) = from_context
                .knowledge_base
                .facts
                .iter()
                .find(|f| f.id == knowledge_id)
            {
                Some(Knowledge::Fact(fact.clone()))
            } else if let Some(procedure) = from_context
                .knowledge_base
                .procedures
                .iter()
                .find(|p| p.id == knowledge_id)
            {
                Some(Knowledge::Procedure(procedure.clone()))
            } else {
                from_context
                    .knowledge_base
                    .learned_patterns
                    .iter()
                    .find(|p| p.id == knowledge_id)
                    .map(|pattern| Knowledge::Pattern(pattern.clone()))
            };

            if let Some(knowledge) = knowledge {
                // Store in shared knowledge
                let shared_item = SharedKnowledgeItem {
                    knowledge,
                    source_agent: from_agent,
                    access_level,
                    created_at: SystemTime::now(),
                    access_count: 0,
                };

                let mut shared_knowledge = self.shared_knowledge.write().await;
                shared_knowledge.insert(knowledge_id, shared_item);

                Ok(())
            } else {
                Err(ContextError::KnowledgeNotFound { id: knowledge_id })
            }
        } else {
            Err(ContextError::NotFound {
                id: ContextId::new(),
            })
        }
    }

    async fn get_shared_knowledge(
        &self,
        agent_id: AgentId,
    ) -> Result<Vec<SharedKnowledgeRef>, ContextError> {
        self.validate_access(agent_id, "get_shared_knowledge")
            .await?;

        let shared_knowledge = self.shared_knowledge.read().await;
        let mut results = Vec::new();

        for (knowledge_id, shared_item) in shared_knowledge.iter() {
            // Check if agent has access to this knowledge
            match shared_item.access_level {
                AccessLevel::Public => {
                    // Calculate trust score based on access count and knowledge type
                    let trust_score = self.calculate_trust_score(shared_item);

                    tracing::debug!(
                        "Shared knowledge {} accessed {} times, trust score: {}",
                        knowledge_id,
                        shared_item.access_count,
                        trust_score
                    );

                    results.push(SharedKnowledgeRef {
                        knowledge_id: *knowledge_id,
                        source_agent: shared_item.source_agent,
                        shared_at: shared_item.created_at,
                        access_level: shared_item.access_level.clone(),
                        trust_score,
                    });
                }
                _ => {
                    // For other access levels, would check specific permissions
                }
            }
        }

        Ok(results)
    }

    async fn archive_context(
        &self,
        agent_id: AgentId,
        before: SystemTime,
    ) -> Result<u32, ContextError> {
        self.validate_access(agent_id, "archive_context").await?;

        let mut total_archived = 0u32;
        let mut archived_context = ArchivedContext::new(agent_id, before);

        // Get current context
        let mut contexts = self.contexts.write().await;
        if let Some(context) = contexts.get_mut(&agent_id) {
            // Archive old memory items
            total_archived += self
                .archive_memory_items(context, before, &mut archived_context)
                .await?;

            // Archive old conversation history
            total_archived += self
                .archive_conversation_history(context, before, &mut archived_context)
                .await?;

            // Archive old knowledge base items (based on retention policy)
            total_archived += self
                .archive_knowledge_items(context, before, &mut archived_context)
                .await?;

            // Persist archived data if we have items to archive
            if total_archived > 0 {
                self.save_archived_context(agent_id, &archived_context)
                    .await?;

                // Update context metadata
                context.updated_at = SystemTime::now();
                context.metadata.insert(
                    "last_archived".to_string(),
                    SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs()
                        .to_string(),
                );
                context
                    .metadata
                    .insert("archived_count".to_string(), total_archived.to_string());

                // Persist updated context
                if self.config.enable_persistence {
                    self.persistence.save_context(agent_id, context).await?;
                }
            }

            tracing::info!(
                "Archived {} items for agent {} before timestamp {:?}",
                total_archived,
                agent_id,
                before
            );
        } else {
            tracing::warn!("No context found for agent {} during archiving", agent_id);
        }

        Ok(total_archived)
    }

    async fn get_context_stats(&self, agent_id: AgentId) -> Result<ContextStats, ContextError> {
        self.validate_access(agent_id, "get_context_stats").await?;

        let contexts = self.contexts.read().await;
        if let Some(context) = contexts.get(&agent_id) {
            let total_memory_items = context.memory.short_term.len()
                + context.memory.long_term.len()
                + context.memory.episodic_memory.len()
                + context.memory.semantic_memory.len();

            let total_knowledge_items = context.knowledge_base.facts.len()
                + context.knowledge_base.procedures.len()
                + context.knowledge_base.learned_patterns.len();

            // Calculate accurate memory size in bytes
            let memory_size_bytes = self.calculate_memory_size_bytes(context);

            // Calculate retention statistics
            let retention_stats = self.calculate_retention_statistics(context);

            Ok(ContextStats {
                total_memory_items,
                total_knowledge_items,
                total_conversations: context.conversation_history.len(),
                total_episodes: context.memory.episodic_memory.len(),
                memory_size_bytes,
                last_activity: context.updated_at,
                retention_status: retention_stats,
            })
        } else {
            Err(ContextError::NotFound {
                id: ContextId::new(),
            })
        }
    }

    async fn shutdown(&self) -> Result<(), ContextError> {
        // Delegate to the concrete implementation's shutdown method
        StandardContextManager::shutdown(self).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn check_and_compact_noop_when_below_threshold() {
        use super::super::compaction::CompactionConfig;
        use super::super::token_counter::HeuristicTokenCounter;

        let config = ContextManagerConfig::default();
        let agent_id = AgentId::new();
        let manager = StandardContextManager::new(config, &agent_id.to_string())
            .await
            .unwrap();
        manager.initialize().await.unwrap();
        let session_id = manager.create_session(agent_id).await.unwrap();

        let compaction_config = CompactionConfig::default();
        let counter = HeuristicTokenCounter::new(200_000);

        let result = manager
            .check_and_compact(&agent_id, &session_id, &compaction_config, &counter)
            .await
            .unwrap();

        assert!(
            result.is_none(),
            "should be no-op when context is nearly empty"
        );
    }
}
