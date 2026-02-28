//! Markdown-backed agent memory persistence
//!
//! Stores agent memory as human-readable Markdown files, providing
//! a transparent and inspectable format for agent context data.

use async_trait::async_trait;
use chrono::Utc;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use tempfile::NamedTempFile;

use super::types::{
    AgentContext, ContextError, ContextId, ContextPersistence, HierarchicalMemory, KnowledgeBase,
    MemoryItem, MemoryType, RetentionPolicy, SemanticMemoryItem, SessionId, StorageStats,
};
use crate::types::AgentId;

/// Markdown-backed memory store for agent contexts.
///
/// Stores agent memory as human-readable Markdown files in the following layout:
/// ```text
/// {root_dir}/
///   {agent_id}/
///     memory.md          # Current memory state
///     logs/
///       2026-02-14.md    # Daily interaction log
/// ```
pub struct MarkdownMemoryStore {
    root_dir: PathBuf,
    retention: Duration,
}

impl MarkdownMemoryStore {
    /// Create a new MarkdownMemoryStore.
    ///
    /// # Arguments
    /// * `root_dir` - Root directory for storing agent memory files
    /// * `retention` - How long to keep daily log files before compaction
    pub fn new(root_dir: PathBuf, retention: Duration) -> Self {
        Self {
            root_dir,
            retention,
        }
    }

    /// Get the directory for a specific agent.
    fn agent_dir(&self, agent_id: AgentId) -> PathBuf {
        self.root_dir.join(agent_id.to_string())
    }

    /// Get the path to the agent's memory.md file.
    fn memory_path(&self, agent_id: AgentId) -> PathBuf {
        self.agent_dir(agent_id).join("memory.md")
    }

    /// Get the path to the agent's logs directory.
    fn logs_dir(&self, agent_id: AgentId) -> PathBuf {
        self.agent_dir(agent_id).join("logs")
    }

    /// Convert a `HierarchicalMemory` into Markdown format.
    fn memory_to_markdown(&self, agent_id: AgentId, memory: &HierarchicalMemory) -> String {
        let now = Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
        let mut md = format!("# Agent Memory: {}\nUpdated: {}\n", agent_id, now);

        // Facts section: long_term items with MemoryType::Factual
        let facts: Vec<&MemoryItem> = memory
            .long_term
            .iter()
            .filter(|item| item.memory_type == MemoryType::Factual)
            .collect();
        if !facts.is_empty() {
            md.push_str("\n## Facts\n");
            for fact in &facts {
                md.push_str(&format!("- {}\n", fact.content));
            }
        }

        // Procedures section: long_term items with MemoryType::Procedural
        let procedures: Vec<&MemoryItem> = memory
            .long_term
            .iter()
            .filter(|item| item.memory_type == MemoryType::Procedural)
            .collect();
        if !procedures.is_empty() {
            md.push_str("\n## Procedures\n");
            for proc in &procedures {
                md.push_str(&format!("- {}\n", proc.content));
            }
        }

        // Learned Patterns section: semantic_memory items
        if !memory.semantic_memory.is_empty() {
            md.push_str("\n## Learned Patterns\n");
            for item in &memory.semantic_memory {
                md.push_str(&format!("- {}\n", item.concept));
            }
        }

        md
    }

    /// Parse Markdown content back into a `HierarchicalMemory`.
    fn markdown_to_memory(&self, markdown: &str) -> HierarchicalMemory {
        let mut memory = HierarchicalMemory::default();
        let mut current_section: Option<&str> = None;

        for line in markdown.lines() {
            let trimmed = line.trim();

            if trimmed == "## Facts" {
                current_section = Some("facts");
                continue;
            } else if trimmed == "## Procedures" {
                current_section = Some("procedures");
                continue;
            } else if trimmed == "## Learned Patterns" {
                current_section = Some("patterns");
                continue;
            } else if trimmed.starts_with("## ") || trimmed.starts_with("# ") {
                current_section = None;
                continue;
            }

            if let Some(content) = trimmed.strip_prefix("- ") {
                let now = SystemTime::now();
                match current_section {
                    Some("facts") => {
                        memory.long_term.push(MemoryItem {
                            id: ContextId::new(),
                            content: content.to_string(),
                            memory_type: MemoryType::Factual,
                            importance: 0.5,
                            access_count: 0,
                            last_accessed: now,
                            created_at: now,
                            embedding: None,
                            metadata: HashMap::new(),
                        });
                    }
                    Some("procedures") => {
                        memory.long_term.push(MemoryItem {
                            id: ContextId::new(),
                            content: content.to_string(),
                            memory_type: MemoryType::Procedural,
                            importance: 0.5,
                            access_count: 0,
                            last_accessed: now,
                            created_at: now,
                            embedding: None,
                            metadata: HashMap::new(),
                        });
                    }
                    Some("patterns") => {
                        memory.semantic_memory.push(SemanticMemoryItem {
                            id: ContextId::new(),
                            concept: content.to_string(),
                            relationships: vec![],
                            properties: HashMap::new(),
                            confidence: 0.5,
                            created_at: now,
                            updated_at: now,
                        });
                    }
                    _ => {}
                }
            }
        }

        memory
    }

    /// Remove log files older than the configured retention period.
    pub async fn compact(&self, agent_id: AgentId) -> Result<(), ContextError> {
        let logs_dir = self.logs_dir(agent_id);
        let retention = self.retention;

        tokio::task::spawn_blocking(move || {
            if !logs_dir.exists() {
                return Ok(());
            }

            let cutoff = SystemTime::now()
                .checked_sub(retention)
                .unwrap_or(SystemTime::UNIX_EPOCH);

            let entries = std::fs::read_dir(&logs_dir).map_err(|e| ContextError::StorageError {
                reason: format!("Failed to read logs directory: {}", e),
            })?;

            for entry in entries {
                let entry = entry.map_err(|e| ContextError::StorageError {
                    reason: format!("Failed to read log entry: {}", e),
                })?;

                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("md") {
                    continue;
                }

                let metadata =
                    std::fs::metadata(&path).map_err(|e| ContextError::StorageError {
                        reason: format!("Failed to read log metadata: {}", e),
                    })?;

                let modified = metadata
                    .modified()
                    .map_err(|e| ContextError::StorageError {
                        reason: format!("Failed to read modification time: {}", e),
                    })?;

                if modified < cutoff {
                    std::fs::remove_file(&path).map_err(|e| ContextError::StorageError {
                        reason: format!("Failed to remove old log file: {}", e),
                    })?;
                }
            }

            Ok(())
        })
        .await
        .map_err(|e| ContextError::StorageError {
            reason: format!("Blocking task failed: {}", e),
        })?
    }
}

/// Append a summary entry to today's daily log (sync, for use in spawn_blocking).
fn append_daily_log_sync(
    logs_dir: &std::path::Path,
    context: &AgentContext,
) -> Result<(), ContextError> {
    std::fs::create_dir_all(logs_dir).map_err(|e| ContextError::StorageError {
        reason: format!("Failed to create logs directory: {}", e),
    })?;

    let today = Utc::now().format("%Y-%m-%d").to_string();
    let log_path = logs_dir.join(format!("{}.md", today));

    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| ContextError::StorageError {
            reason: format!("Failed to open daily log: {}", e),
        })?;

    let now = Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
    let memory_count = context.memory.long_term.len() + context.memory.short_term.len();
    let knowledge_count = context.knowledge_base.facts.len()
        + context.knowledge_base.procedures.len()
        + context.knowledge_base.learned_patterns.len();

    writeln!(
        file,
        "### {}\n- Memory items: {}\n- Knowledge items: {}\n",
        now, memory_count, knowledge_count
    )
    .map_err(|e| ContextError::StorageError {
        reason: format!("Failed to write daily log: {}", e),
    })?;

    Ok(())
}

/// Walk all files under a directory and sum their sizes (sync, for use in spawn_blocking).
fn dir_size_sync(path: &std::path::Path) -> Result<u64, ContextError> {
    let mut total: u64 = 0;
    if !path.exists() {
        return Ok(0);
    }
    let entries = std::fs::read_dir(path).map_err(|e| ContextError::StorageError {
        reason: format!("Failed to read directory: {}", e),
    })?;
    for entry in entries {
        let entry = entry.map_err(|e| ContextError::StorageError {
            reason: format!("Failed to read entry: {}", e),
        })?;
        let meta = entry.metadata().map_err(|e| ContextError::StorageError {
            reason: format!("Failed to read metadata: {}", e),
        })?;
        if meta.is_dir() {
            total += dir_size_sync(&entry.path())?;
        } else {
            total += meta.len();
        }
    }
    Ok(total)
}

#[async_trait]
impl ContextPersistence for MarkdownMemoryStore {
    async fn save_context(
        &self,
        agent_id: AgentId,
        context: &AgentContext,
    ) -> Result<(), ContextError> {
        let agent_dir = self.agent_dir(agent_id);
        let markdown = self.memory_to_markdown(agent_id, &context.memory);
        let memory_path = self.memory_path(agent_id);
        let logs_dir = self.logs_dir(agent_id);
        let context_clone = context.clone();

        tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&agent_dir).map_err(|e| ContextError::StorageError {
                reason: format!("Failed to create agent directory: {}", e),
            })?;

            // Write memory.md atomically via tempfile
            let temp =
                NamedTempFile::new_in(&agent_dir).map_err(|e| ContextError::StorageError {
                    reason: format!("Failed to create temp file: {}", e),
                })?;

            std::fs::write(temp.path(), markdown.as_bytes()).map_err(|e| {
                ContextError::StorageError {
                    reason: format!("Failed to write temp file: {}", e),
                }
            })?;

            temp.persist(&memory_path)
                .map_err(|e| ContextError::StorageError {
                    reason: format!("Failed to persist memory file: {}", e),
                })?;

            // Append session summary to today's daily log
            append_daily_log_sync(&logs_dir, &context_clone)
        })
        .await
        .map_err(|e| ContextError::StorageError {
            reason: format!("Blocking task failed: {}", e),
        })?
    }

    async fn load_context(&self, agent_id: AgentId) -> Result<Option<AgentContext>, ContextError> {
        let memory_path = self.memory_path(agent_id);

        let markdown = match tokio::fs::read_to_string(&memory_path).await {
            Ok(content) => content,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(ContextError::StorageError {
                    reason: format!("Failed to read memory file: {}", e),
                })
            }
        };

        let memory = self.markdown_to_memory(&markdown);
        let now = SystemTime::now();

        let context = AgentContext {
            agent_id,
            session_id: SessionId::new(),
            memory,
            knowledge_base: KnowledgeBase::default(),
            conversation_history: vec![],
            metadata: HashMap::new(),
            created_at: now,
            updated_at: now,
            retention_policy: RetentionPolicy::default(),
        };

        Ok(Some(context))
    }

    async fn delete_context(&self, agent_id: AgentId) -> Result<(), ContextError> {
        let agent_dir = self.agent_dir(agent_id);
        match tokio::fs::remove_dir_all(&agent_dir).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(ContextError::StorageError {
                reason: format!("Failed to delete agent directory: {}", e),
            }),
        }
    }

    async fn list_agent_contexts(&self) -> Result<Vec<AgentId>, ContextError> {
        let root_dir = self.root_dir.clone();
        tokio::task::spawn_blocking(move || {
            let mut agent_ids = Vec::new();

            if !root_dir.exists() {
                return Ok(agent_ids);
            }

            let entries = std::fs::read_dir(&root_dir).map_err(|e| ContextError::StorageError {
                reason: format!("Failed to read root directory: {}", e),
            })?;

            for entry in entries {
                let entry = entry.map_err(|e| ContextError::StorageError {
                    reason: format!("Failed to read directory entry: {}", e),
                })?;

                if entry.metadata().map(|m| m.is_dir()).unwrap_or(false) {
                    if let Some(name) = entry.file_name().to_str() {
                        if let Ok(uuid) = uuid::Uuid::parse_str(name) {
                            agent_ids.push(AgentId(uuid));
                        }
                    }
                }
            }

            Ok(agent_ids)
        })
        .await
        .map_err(|e| ContextError::StorageError {
            reason: format!("Blocking task failed: {}", e),
        })?
    }

    async fn context_exists(&self, agent_id: AgentId) -> Result<bool, ContextError> {
        Ok(tokio::fs::try_exists(self.memory_path(agent_id))
            .await
            .unwrap_or(false))
    }

    async fn get_storage_stats(&self) -> Result<StorageStats, ContextError> {
        let root_dir = self.root_dir.clone();
        let (total_contexts, total_size_bytes) = tokio::task::spawn_blocking(move || {
            let mut total_contexts: usize = 0;
            let mut total_size_bytes: u64 = 0;

            if root_dir.exists() {
                let entries =
                    std::fs::read_dir(&root_dir).map_err(|e| ContextError::StorageError {
                        reason: format!("Failed to read root directory: {}", e),
                    })?;

                for entry in entries {
                    let entry = entry.map_err(|e| ContextError::StorageError {
                        reason: format!("Failed to read entry: {}", e),
                    })?;

                    if entry.metadata().map(|m| m.is_dir()).unwrap_or(false) {
                        total_contexts += 1;
                        total_size_bytes += dir_size_sync(&entry.path())?;
                    }
                }
            }

            Ok::<_, ContextError>((total_contexts, total_size_bytes))
        })
        .await
        .map_err(|e| ContextError::StorageError {
            reason: format!("Blocking task failed: {}", e),
        })??;

        Ok(StorageStats {
            total_contexts,
            total_size_bytes,
            last_cleanup: SystemTime::now(),
            storage_path: self.root_dir.clone(),
        })
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Create a sample `AgentContext` for testing.
    fn sample_context(agent_id: AgentId) -> AgentContext {
        let now = SystemTime::now();

        let factual_item = MemoryItem {
            id: ContextId::new(),
            content: "User prefers dark mode".to_string(),
            memory_type: MemoryType::Factual,
            importance: 0.8,
            access_count: 1,
            last_accessed: now,
            created_at: now,
            embedding: None,
            metadata: HashMap::new(),
        };

        let procedural_item = MemoryItem {
            id: ContextId::new(),
            content: "Deploy via cargo shuttle deploy".to_string(),
            memory_type: MemoryType::Procedural,
            importance: 0.7,
            access_count: 2,
            last_accessed: now,
            created_at: now,
            embedding: None,
            metadata: HashMap::new(),
        };

        let semantic_item = SemanticMemoryItem {
            id: ContextId::new(),
            concept: "User asks about metrics after deployments".to_string(),
            relationships: vec![],
            properties: HashMap::new(),
            confidence: 0.6,
            created_at: now,
            updated_at: now,
        };

        let memory = HierarchicalMemory {
            working_memory: Default::default(),
            short_term: vec![],
            long_term: vec![factual_item, procedural_item],
            episodic_memory: vec![],
            semantic_memory: vec![semantic_item],
        };

        AgentContext {
            agent_id,
            session_id: SessionId::new(),
            memory,
            knowledge_base: KnowledgeBase::default(),
            conversation_history: vec![],
            metadata: HashMap::new(),
            created_at: now,
            updated_at: now,
            retention_policy: RetentionPolicy::default(),
        }
    }

    #[tokio::test]
    async fn test_save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = MarkdownMemoryStore::new(dir.path().to_path_buf(), Duration::from_secs(86400));

        let agent_id = AgentId::new();
        let context = sample_context(agent_id);

        store.save_context(agent_id, &context).await.unwrap();
        let loaded = store.load_context(agent_id).await.unwrap().unwrap();

        // Verify the memory content round-trips
        assert_eq!(loaded.agent_id, agent_id);
        assert_eq!(loaded.memory.long_term.len(), 2);
        assert_eq!(loaded.memory.semantic_memory.len(), 1);

        // Verify factual content
        let facts: Vec<&MemoryItem> = loaded
            .memory
            .long_term
            .iter()
            .filter(|i| i.memory_type == MemoryType::Factual)
            .collect();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].content, "User prefers dark mode");

        // Verify procedural content
        let procs: Vec<&MemoryItem> = loaded
            .memory
            .long_term
            .iter()
            .filter(|i| i.memory_type == MemoryType::Procedural)
            .collect();
        assert_eq!(procs.len(), 1);
        assert_eq!(procs[0].content, "Deploy via cargo shuttle deploy");

        // Verify semantic content
        assert_eq!(
            loaded.memory.semantic_memory[0].concept,
            "User asks about metrics after deployments"
        );
    }

    #[tokio::test]
    async fn test_load_missing_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let store = MarkdownMemoryStore::new(dir.path().to_path_buf(), Duration::from_secs(86400));

        let agent_id = AgentId::new();
        let result = store.load_context(agent_id).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_delete_context() {
        let dir = tempfile::tempdir().unwrap();
        let store = MarkdownMemoryStore::new(dir.path().to_path_buf(), Duration::from_secs(86400));

        let agent_id = AgentId::new();
        let context = sample_context(agent_id);

        store.save_context(agent_id, &context).await.unwrap();
        assert!(store.context_exists(agent_id).await.unwrap());

        store.delete_context(agent_id).await.unwrap();
        assert!(!store.context_exists(agent_id).await.unwrap());
    }

    #[tokio::test]
    async fn test_list_agent_contexts() {
        let dir = tempfile::tempdir().unwrap();
        let store = MarkdownMemoryStore::new(dir.path().to_path_buf(), Duration::from_secs(86400));

        let agent1 = AgentId::new();
        let agent2 = AgentId::new();

        store
            .save_context(agent1, &sample_context(agent1))
            .await
            .unwrap();
        store
            .save_context(agent2, &sample_context(agent2))
            .await
            .unwrap();

        let agents = store.list_agent_contexts().await.unwrap();
        assert_eq!(agents.len(), 2);
    }

    #[tokio::test]
    async fn test_daily_log_created() {
        let dir = tempfile::tempdir().unwrap();
        let store = MarkdownMemoryStore::new(dir.path().to_path_buf(), Duration::from_secs(86400));

        let agent_id = AgentId::new();
        let context = sample_context(agent_id);

        store.save_context(agent_id, &context).await.unwrap();

        let logs_dir = store.logs_dir(agent_id);
        assert!(logs_dir.exists());

        let today = Utc::now().format("%Y-%m-%d").to_string();
        let log_file = logs_dir.join(format!("{}.md", today));
        assert!(log_file.exists());
    }

    #[tokio::test]
    async fn test_storage_stats() {
        let dir = tempfile::tempdir().unwrap();
        let store = MarkdownMemoryStore::new(dir.path().to_path_buf(), Duration::from_secs(86400));

        let agent_id = AgentId::new();
        let context = sample_context(agent_id);

        store.save_context(agent_id, &context).await.unwrap();

        let stats = store.get_storage_stats().await.unwrap();
        assert_eq!(stats.total_contexts, 1);
        assert!(stats.total_size_bytes > 0);
    }

    #[tokio::test]
    async fn test_memory_to_markdown_format() {
        let dir = tempfile::tempdir().unwrap();
        let store = MarkdownMemoryStore::new(dir.path().to_path_buf(), Duration::from_secs(86400));

        let agent_id = AgentId::new();
        let context = sample_context(agent_id);

        let markdown = store.memory_to_markdown(agent_id, &context.memory);

        assert!(markdown.contains(&format!("# Agent Memory: {}", agent_id)));
        assert!(markdown.contains("## Facts"));
        assert!(markdown.contains("- User prefers dark mode"));
        assert!(markdown.contains("## Procedures"));
        assert!(markdown.contains("- Deploy via cargo shuttle deploy"));
        assert!(markdown.contains("## Learned Patterns"));
        assert!(markdown.contains("- User asks about metrics after deployments"));
    }

    #[tokio::test]
    async fn test_compact_removes_old_logs() {
        let dir = tempfile::tempdir().unwrap();
        // Use a 1-day retention so we can test expiration
        let store = MarkdownMemoryStore::new(dir.path().to_path_buf(), Duration::from_secs(86400));

        let agent_id = AgentId::new();
        let context = sample_context(agent_id);
        store.save_context(agent_id, &context).await.unwrap();

        // Create a stale log file and set its mtime to 3 days ago
        let logs_dir = store.logs_dir(agent_id);
        let stale_log = logs_dir.join("2020-01-01.md");
        std::fs::write(&stale_log, "# Old log\n").unwrap();

        let old_time = filetime::FileTime::from_system_time(
            SystemTime::now() - Duration::from_secs(86400 * 3),
        );
        filetime::set_file_mtime(&stale_log, old_time).unwrap();

        assert!(stale_log.exists());

        store.compact(agent_id).await.unwrap();

        // The stale log should be removed; today's log should remain
        assert!(!stale_log.exists());

        let today = Utc::now().format("%Y-%m-%d").to_string();
        let today_log = logs_dir.join(format!("{}.md", today));
        assert!(today_log.exists());
    }
}
