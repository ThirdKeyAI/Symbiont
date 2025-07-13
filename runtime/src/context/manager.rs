//! Context Manager implementation for agent memory and knowledge management

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;
use std::path::PathBuf;
use tokio::sync::RwLock;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::types::*;
use super::vector_db::{VectorDatabase, QdrantClientWrapper, QdrantConfig, MockEmbeddingService, EmbeddingService};
use crate::types::AgentId;

/// Context Manager trait for agent memory and knowledge management
#[async_trait]
pub trait ContextManager: Send + Sync {
    /// Store agent context
    async fn store_context(&self, agent_id: AgentId, context: AgentContext) -> Result<ContextId, ContextError>;
    
    /// Retrieve agent context
    async fn retrieve_context(&self, agent_id: AgentId, session_id: Option<SessionId>) -> Result<Option<AgentContext>, ContextError>;
    
    /// Query context with semantic search
    async fn query_context(&self, agent_id: AgentId, query: ContextQuery) -> Result<Vec<ContextItem>, ContextError>;
    
    /// Update specific memory items
    async fn update_memory(&self, agent_id: AgentId, memory_updates: Vec<MemoryUpdate>) -> Result<(), ContextError>;
    
    /// Add knowledge to agent's knowledge base
    async fn add_knowledge(&self, agent_id: AgentId, knowledge: Knowledge) -> Result<KnowledgeId, ContextError>;
    
    /// Search knowledge base
    async fn search_knowledge(&self, agent_id: AgentId, query: &str, limit: usize) -> Result<Vec<KnowledgeItem>, ContextError>;
    
    /// Share knowledge between agents
    async fn share_knowledge(&self, from_agent: AgentId, to_agent: AgentId, knowledge_id: KnowledgeId, access_level: AccessLevel) -> Result<(), ContextError>;
    
    /// Get shared knowledge available to agent
    async fn get_shared_knowledge(&self, agent_id: AgentId) -> Result<Vec<SharedKnowledgeRef>, ContextError>;
    
    /// Archive old context based on retention policy
    async fn archive_context(&self, agent_id: AgentId, before: SystemTime) -> Result<u32, ContextError>;
    
    /// Get context statistics
    async fn get_context_stats(&self, agent_id: AgentId) -> Result<ContextStats, ContextError>;
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
    vector_db: Arc<dyn VectorDatabase>,
    /// Embedding service for generating vector embeddings
    embedding_service: Arc<MockEmbeddingService>,
    /// Persistent storage for contexts
    persistence: Arc<dyn ContextPersistence>,
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
    /// Qdrant vector database configuration
    pub qdrant_config: QdrantConfig,
    /// Enable vector database integration
    pub enable_vector_db: bool,
    /// File persistence configuration
    pub persistence_config: FilePersistenceConfig,
    /// Enable persistent storage
    pub enable_persistence: bool,
}

impl Default for ContextManagerConfig {
    fn default() -> Self {
        Self {
            max_contexts_in_memory: 1000,
            default_retention_policy: RetentionPolicy::default(),
            enable_auto_archiving: true,
            archiving_interval: std::time::Duration::from_secs(3600), // 1 hour
            max_memory_items_per_agent: 10000,
            max_knowledge_items_per_agent: 5000,
            qdrant_config: QdrantConfig::default(),
            enable_vector_db: true,
            persistence_config: FilePersistenceConfig::default(),
            enable_persistence: true,
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
        fs::create_dir_all(&self.config.storage_path).await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to create storage directory: {}", e)
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
        self.config.storage_path.join(filename)
    }

    /// Serialize context to bytes
    async fn serialize_context(&self, context: &AgentContext) -> Result<Vec<u8>, ContextError> {
        let json_data = serde_json::to_vec_pretty(context)
            .map_err(|e| ContextError::SerializationError {
                reason: format!("Failed to serialize context: {}", e)
            })?;

        if self.config.enable_compression {
            use flate2::write::GzEncoder;
            use flate2::Compression;
            use std::io::Write;

            let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(&json_data)
                .map_err(|e| ContextError::SerializationError {
                    reason: format!("Failed to compress context: {}", e)
                })?;
            encoder.finish()
                .map_err(|e| ContextError::SerializationError {
                    reason: format!("Failed to finalize compression: {}", e)
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
            decoder.read_to_end(&mut decompressed)
                .map_err(|e| ContextError::SerializationError {
                    reason: format!("Failed to decompress context: {}", e)
                })?;
            decompressed
        } else {
            data
        };

        serde_json::from_slice(&json_data)
            .map_err(|e| ContextError::SerializationError {
                reason: format!("Failed to deserialize context: {}", e)
            })
    }

    /// Create backup of existing context file
    async fn create_backup(&self, agent_id: AgentId) -> Result<(), ContextError> {
        let context_path = self.get_context_path(agent_id);
        if !context_path.exists() {
            return Ok(());
        }

        let backup_path = context_path.with_extension(
            format!("backup.{}.json", SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs())
        );

        fs::copy(&context_path, &backup_path).await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to create backup: {}", e)
            })?;

        // Clean up old backups
        self.cleanup_old_backups(agent_id).await?;
        Ok(())
    }

    /// Clean up old backup files
    async fn cleanup_old_backups(&self, agent_id: AgentId) -> Result<(), ContextError> {
        let mut backup_files = Vec::new();
        let mut dir = fs::read_dir(&self.config.storage_path).await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to read storage directory: {}", e)
            })?;

        while let Some(entry) = dir.next_entry().await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to read directory entry: {}", e)
            })? {
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
                eprintln!("Warning: Failed to remove old backup {}: {}", path.display(), e);
            }
        }

        Ok(())
    }
}

#[async_trait]
impl ContextPersistence for FilePersistence {
    async fn save_context(&self, agent_id: AgentId, context: &AgentContext) -> Result<(), ContextError> {
        // Create backup of existing context
        self.create_backup(agent_id).await?;

        // Serialize context
        let data = self.serialize_context(context).await?;

        // Write to file
        let context_path = self.get_context_path(agent_id);
        let mut file = fs::File::create(&context_path).await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to create context file: {}", e)
            })?;

        file.write_all(&data).await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to write context data: {}", e)
            })?;

        file.sync_all().await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to sync context file: {}", e)
            })?;

        Ok(())
    }

    async fn load_context(&self, agent_id: AgentId) -> Result<Option<AgentContext>, ContextError> {
        let context_path = self.get_context_path(agent_id);
        
        if !context_path.exists() {
            return Ok(None);
        }

        let mut file = fs::File::open(&context_path).await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to open context file: {}", e)
            })?;

        let mut data = Vec::new();
        file.read_to_end(&mut data).await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to read context file: {}", e)
            })?;

        let context = self.deserialize_context(data).await?;
        Ok(Some(context))
    }

    async fn delete_context(&self, agent_id: AgentId) -> Result<(), ContextError> {
        let context_path = self.get_context_path(agent_id);
        
        if context_path.exists() {
            fs::remove_file(&context_path).await
                .map_err(|e| ContextError::StorageError {
                    reason: format!("Failed to delete context file: {}", e)
                })?;
        }

        Ok(())
    }

    async fn list_agent_contexts(&self) -> Result<Vec<AgentId>, ContextError> {
        let mut agent_ids = Vec::new();
        let mut dir = fs::read_dir(&self.config.storage_path).await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to read storage directory: {}", e)
            })?;

        while let Some(entry) = dir.next_entry().await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to read directory entry: {}", e)
            })? {
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

        let mut dir = fs::read_dir(&self.config.storage_path).await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to read storage directory: {}", e)
            })?;

        while let Some(entry) = dir.next_entry().await
            .map_err(|e| ContextError::StorageError {
                reason: format!("Failed to read directory entry: {}", e)
            })? {
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
            storage_path: self.config.storage_path.clone(),
        })
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl StandardContextManager {
    /// Create a new StandardContextManager
    pub fn new(config: ContextManagerConfig) -> Self {
        let vector_db: Arc<dyn VectorDatabase> = if config.enable_vector_db {
            Arc::new(QdrantClientWrapper::new(config.qdrant_config.clone()))
        } else {
            // Could use a mock implementation for testing
            Arc::new(QdrantClientWrapper::new(config.qdrant_config.clone()))
        };
        
        let embedding_service = Arc::new(MockEmbeddingService::new(config.qdrant_config.vector_dimension));
        
        let persistence: Arc<dyn ContextPersistence> = if config.enable_persistence {
            Arc::new(FilePersistence::new(config.persistence_config.clone()))
        } else {
            // Could use a no-op implementation for testing
            Arc::new(FilePersistence::new(config.persistence_config.clone()))
        };
        
        Self {
            contexts: Arc::new(RwLock::new(HashMap::new())),
            config,
            shared_knowledge: Arc::new(RwLock::new(HashMap::new())),
            vector_db,
            embedding_service,
            persistence,
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
            if let Some(file_persistence) = self.persistence.as_any().downcast_ref::<FilePersistence>() {
                file_persistence.initialize().await?;
            }
            
            // Load existing contexts from persistent storage
            self.load_existing_contexts().await?;
        }
        
        // TODO: Set up retention policy scheduler
        
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

    /// Shutdown the context manager
    pub async fn shutdown(&self) -> Result<(), ContextError> {
        // Placeholder for cleanup operations
        // In a real implementation, this would:
        // - Save all contexts to persistent storage
        // - Close database connections
        // - Stop background tasks
        
        Ok(())
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
    async fn validate_access(&self, _agent_id: AgentId, _operation: &str) -> Result<(), ContextError> {
        // Placeholder for policy validation
        // In a real implementation, this would integrate with the Policy Engine
        // to check if the agent has permission to perform the operation
        
        Ok(())
    }

    /// Generate embeddings for content
    async fn generate_embeddings(&self, content: &str) -> Result<Vec<f32>, ContextError> {
        self.embedding_service.generate_embedding(content).await
    }

    /// Perform semantic search on memory items
    async fn semantic_search_memory(&self, agent_id: AgentId, query: &str, limit: usize) -> Result<Vec<ContextItem>, ContextError> {
        if self.config.enable_vector_db {
            // Generate embeddings for the query
            let query_embedding = self.generate_embeddings(query).await?;
            
            // Search the vector database with semantic similarity
            let threshold = 0.7; // Minimum similarity threshold
            self.vector_db.semantic_search(agent_id, query_embedding, limit, threshold).await
        } else {
            // Fallback to simple keyword search
            let contexts = self.contexts.read().await;
            if let Some(context) = contexts.get(&agent_id) {
                let mut results = Vec::new();
                
                for memory_item in &context.memory.short_term {
                    if memory_item.content.to_lowercase().contains(&query.to_lowercase()) {
                        results.push(ContextItem {
                            id: memory_item.id,
                            content: memory_item.content.clone(),
                            item_type: ContextItemType::Memory(memory_item.memory_type.clone()),
                            relevance_score: 0.8, // Placeholder score
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

    /// Calculate memory importance score
    fn calculate_importance(&self, memory_item: &MemoryItem) -> f32 {
        // Placeholder importance calculation
        // In a real implementation, this would consider:
        // - Access frequency
        // - Recency
        // - Content relevance
        // - User feedback
        
        let base_importance = memory_item.importance;
        let access_factor = (memory_item.access_count as f32).ln() + 1.0;
        let recency_factor = 1.0; // Would calculate based on time since creation
        
        base_importance * access_factor * recency_factor
    }

    /// Convert Knowledge to KnowledgeItem for vector storage
    fn knowledge_to_item(&self, knowledge: &Knowledge, knowledge_id: KnowledgeId) -> Result<KnowledgeItem, ContextError> {
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

#[async_trait]
impl ContextManager for StandardContextManager {
    async fn store_context(&self, agent_id: AgentId, mut context: AgentContext) -> Result<ContextId, ContextError> {
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

    async fn retrieve_context(&self, agent_id: AgentId, session_id: Option<SessionId>) -> Result<Option<AgentContext>, ContextError> {
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

    async fn query_context(&self, agent_id: AgentId, query: ContextQuery) -> Result<Vec<ContextItem>, ContextError> {
        self.validate_access(agent_id, "query_context").await?;
        
        match query.query_type {
            QueryType::Semantic => {
                let search_term = query.search_terms.join(" ");
                self.semantic_search_memory(agent_id, &search_term, query.max_results).await
            }
            QueryType::Keyword => {
                // Placeholder for keyword search
                Ok(Vec::new())
            }
            QueryType::Temporal => {
                // Placeholder for temporal search
                Ok(Vec::new())
            }
            QueryType::Similarity => {
                // Placeholder for similarity search
                Ok(Vec::new())
            }
            QueryType::Hybrid => {
                // Placeholder for hybrid search
                Ok(Vec::new())
            }
        }
    }

    async fn update_memory(&self, agent_id: AgentId, memory_updates: Vec<MemoryUpdate>) -> Result<(), ContextError> {
        self.validate_access(agent_id, "update_memory").await?;
        
        let mut contexts = self.contexts.write().await;
        if let Some(context) = contexts.get_mut(&agent_id) {
            for update in memory_updates {
                match update.operation {
                    UpdateOperation::Add => {
                        // Add new memory item based on target
                        match update.target {
                            MemoryTarget::ShortTerm(_) => {
                                // Add to short-term memory
                                // Implementation would parse update.data and create MemoryItem
                            }
                            MemoryTarget::LongTerm(_) => {
                                // Add to long-term memory
                            }
                            MemoryTarget::Working(key) => {
                                // Add to working memory
                                context.memory.working_memory.variables.insert(key, update.data);
                            }
                            _ => {
                                // Handle other memory targets
                            }
                        }
                    }
                    UpdateOperation::Update => {
                        // Update existing memory item
                    }
                    UpdateOperation::Delete => {
                        // Delete memory item
                    }
                    UpdateOperation::Increment => {
                        // Increment numeric values
                    }
                }
            }
            context.updated_at = SystemTime::now();
        }
        
        Ok(())
    }

    async fn add_knowledge(&self, agent_id: AgentId, knowledge: Knowledge) -> Result<KnowledgeId, ContextError> {
        self.validate_access(agent_id, "add_knowledge").await?;
        
        let knowledge_id = KnowledgeId::new();
        
        // Store in vector database if enabled
        if self.config.enable_vector_db {
            let knowledge_item = self.knowledge_to_item(&knowledge, knowledge_id)?;
            let embedding = self.generate_embeddings(&knowledge_item.content).await?;
            let _vector_id = self.vector_db.store_knowledge_item(&knowledge_item, embedding).await?;
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

    async fn search_knowledge(&self, agent_id: AgentId, query: &str, limit: usize) -> Result<Vec<KnowledgeItem>, ContextError> {
        self.validate_access(agent_id, "search_knowledge").await?;
        
        if self.config.enable_vector_db {
            // Generate embeddings for the query
            let query_embedding = self.generate_embeddings(query).await?;
            
            // Search the vector database for knowledge items
            self.vector_db.search_knowledge_base(agent_id, query_embedding, limit).await
        } else {
            // Fallback to simple keyword search
            let contexts = self.contexts.read().await;
            if let Some(context) = contexts.get(&agent_id) {
                let mut results = Vec::new();
                
                // Search facts
                for fact in &context.knowledge_base.facts {
                    let content = format!("{} {} {}", fact.subject, fact.predicate, fact.object);
                    if content.to_lowercase().contains(&query.to_lowercase()) {
                        results.push(KnowledgeItem {
                            id: fact.id,
                            content,
                            knowledge_type: KnowledgeType::Fact,
                            confidence: fact.confidence,
                            relevance_score: 0.8, // Placeholder
                            source: fact.source.clone(),
                            created_at: fact.created_at,
                        });
                    }
                }
                
                // Search procedures
                for procedure in &context.knowledge_base.procedures {
                    if procedure.name.to_lowercase().contains(&query.to_lowercase()) ||
                       procedure.description.to_lowercase().contains(&query.to_lowercase()) {
                        results.push(KnowledgeItem {
                            id: procedure.id,
                            content: format!("{}: {}", procedure.name, procedure.description),
                            knowledge_type: KnowledgeType::Procedure,
                            confidence: procedure.success_rate,
                            relevance_score: 0.8, // Placeholder
                            source: KnowledgeSource::Learning,
                            created_at: SystemTime::now(), // Placeholder
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

    async fn share_knowledge(&self, from_agent: AgentId, _to_agent: AgentId, knowledge_id: KnowledgeId, access_level: AccessLevel) -> Result<(), ContextError> {
        self.validate_access(from_agent, "share_knowledge").await?;
        
        // Find the knowledge item in the source agent's knowledge base
        let contexts = self.contexts.read().await;
        if let Some(from_context) = contexts.get(&from_agent) {
            // Find the knowledge item
            let knowledge = if let Some(fact) = from_context.knowledge_base.facts.iter().find(|f| f.id == knowledge_id) {
                Some(Knowledge::Fact(fact.clone()))
            } else if let Some(procedure) = from_context.knowledge_base.procedures.iter().find(|p| p.id == knowledge_id) {
                Some(Knowledge::Procedure(procedure.clone()))
            } else { from_context.knowledge_base.learned_patterns.iter().find(|p| p.id == knowledge_id).map(|pattern| Knowledge::Pattern(pattern.clone())) };
            
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
            Err(ContextError::NotFound { id: ContextId::new() })
        }
    }

    async fn get_shared_knowledge(&self, agent_id: AgentId) -> Result<Vec<SharedKnowledgeRef>, ContextError> {
        self.validate_access(agent_id, "get_shared_knowledge").await?;
        
        let shared_knowledge = self.shared_knowledge.read().await;
        let mut results = Vec::new();
        
        for (knowledge_id, shared_item) in shared_knowledge.iter() {
            // Check if agent has access to this knowledge
            match shared_item.access_level {
                AccessLevel::Public => {
                    results.push(SharedKnowledgeRef {
                        knowledge_id: *knowledge_id,
                        source_agent: shared_item.source_agent,
                        shared_at: shared_item.created_at,
                        access_level: shared_item.access_level.clone(),
                        trust_score: 0.8, // Placeholder trust calculation
                    });
                }
                _ => {
                    // For other access levels, would check specific permissions
                }
            }
        }
        
        Ok(results)
    }

    async fn archive_context(&self, agent_id: AgentId, _before: SystemTime) -> Result<u32, ContextError> {
        self.validate_access(agent_id, "archive_context").await?;
        
        // Placeholder for archiving logic
        // In a real implementation, this would:
        // - Move old context items to archive storage
        // - Update retention metadata
        // - Clean up in-memory storage
        
        Ok(0) // Return number of archived items
    }

    async fn get_context_stats(&self, agent_id: AgentId) -> Result<ContextStats, ContextError> {
        self.validate_access(agent_id, "get_context_stats").await?;
        
        let contexts = self.contexts.read().await;
        if let Some(context) = contexts.get(&agent_id) {
            let total_memory_items = context.memory.short_term.len() + 
                                   context.memory.long_term.len() + 
                                   context.memory.episodic_memory.len() + 
                                   context.memory.semantic_memory.len();
            
            let total_knowledge_items = context.knowledge_base.facts.len() + 
                                      context.knowledge_base.procedures.len() + 
                                      context.knowledge_base.learned_patterns.len();
            
            Ok(ContextStats {
                total_memory_items,
                total_knowledge_items,
                total_conversations: context.conversation_history.len(),
                total_episodes: context.memory.episodic_memory.len(),
                memory_size_bytes: 0, // Placeholder calculation
                last_activity: context.updated_at,
                retention_status: RetentionStatus {
                    items_to_archive: 0,
                    items_to_delete: 0,
                    next_cleanup: SystemTime::now(),
                },
            })
        } else {
            Err(ContextError::NotFound { id: ContextId::new() })
        }
    }
}