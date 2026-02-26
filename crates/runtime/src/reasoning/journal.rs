//! Durable execution journal with SQLite storage
//!
//! Provides append-only, crash-recoverable journal storage for reasoning loops.
//! Each phase boundary is a checkpoint; crashed loops resume deterministically
//! by replaying journal entries.
//!
//! Feature-gated behind `cron` (which includes `rusqlite`).

use crate::reasoning::loop_types::{JournalEntry, JournalError, JournalWriter};
use crate::types::AgentId;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Trait for durable journal storage backends.
#[async_trait::async_trait]
pub trait JournalStorage: Send + Sync {
    /// Append an entry to persistent storage.
    async fn store(&self, entry: &JournalEntry) -> Result<(), JournalError>;

    /// Read all entries for a given agent, ordered by sequence.
    async fn read_entries(&self, agent_id: &AgentId) -> Result<Vec<JournalEntry>, JournalError>;

    /// Read entries starting from a given sequence number.
    async fn read_from(
        &self,
        agent_id: &AgentId,
        from_sequence: u64,
    ) -> Result<Vec<JournalEntry>, JournalError>;

    /// Get the latest sequence number for an agent (0 if none).
    async fn latest_sequence(&self, agent_id: &AgentId) -> Result<u64, JournalError>;

    /// Delete all entries for an agent (compaction after loop completion).
    async fn compact(&self, agent_id: &AgentId) -> Result<u64, JournalError>;
}

/// In-memory journal storage for testing and lightweight use.
pub struct MemoryJournalStorage {
    entries: Mutex<Vec<JournalEntry>>,
}

impl Default for MemoryJournalStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryJournalStorage {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl JournalStorage for MemoryJournalStorage {
    async fn store(&self, entry: &JournalEntry) -> Result<(), JournalError> {
        self.entries.lock().await.push(entry.clone());
        Ok(())
    }

    async fn read_entries(&self, agent_id: &AgentId) -> Result<Vec<JournalEntry>, JournalError> {
        let entries = self.entries.lock().await;
        Ok(entries
            .iter()
            .filter(|e| e.agent_id == *agent_id)
            .cloned()
            .collect())
    }

    async fn read_from(
        &self,
        agent_id: &AgentId,
        from_sequence: u64,
    ) -> Result<Vec<JournalEntry>, JournalError> {
        let entries = self.entries.lock().await;
        Ok(entries
            .iter()
            .filter(|e| e.agent_id == *agent_id && e.sequence >= from_sequence)
            .cloned()
            .collect())
    }

    async fn latest_sequence(&self, agent_id: &AgentId) -> Result<u64, JournalError> {
        let entries = self.entries.lock().await;
        Ok(entries
            .iter()
            .filter(|e| e.agent_id == *agent_id)
            .map(|e| e.sequence)
            .max()
            .unwrap_or(0))
    }

    async fn compact(&self, agent_id: &AgentId) -> Result<u64, JournalError> {
        let mut entries = self.entries.lock().await;
        let before = entries.len();
        entries.retain(|e| e.agent_id != *agent_id);
        Ok((before - entries.len()) as u64)
    }
}

/// Durable journal backed by a `JournalStorage` implementation.
///
/// Implements `JournalWriter` so it can be used as a drop-in replacement
/// for `BufferedJournal` in the reasoning loop.
pub struct DurableJournal {
    storage: Arc<dyn JournalStorage>,
    sequence: AtomicU64,
    agent_id: AgentId,
}

impl DurableJournal {
    /// Create a new durable journal for the given agent.
    pub fn new(storage: Arc<dyn JournalStorage>, agent_id: AgentId) -> Self {
        Self {
            storage,
            sequence: AtomicU64::new(0),
            agent_id,
        }
    }

    /// Initialize from storage, resuming the sequence counter.
    pub async fn initialize(&self) -> Result<(), JournalError> {
        let latest = self.storage.latest_sequence(&self.agent_id).await?;
        self.sequence.store(latest, Ordering::SeqCst);
        Ok(())
    }

    /// Replay all journal entries for this agent.
    pub async fn replay(&self) -> Result<Vec<JournalEntry>, JournalError> {
        self.storage.read_entries(&self.agent_id).await
    }

    /// Replay entries starting from a given sequence.
    pub async fn replay_from(&self, from_sequence: u64) -> Result<Vec<JournalEntry>, JournalError> {
        self.storage.read_from(&self.agent_id, from_sequence).await
    }

    /// Compact (remove) all entries for this agent after successful loop completion.
    pub async fn compact(&self) -> Result<u64, JournalError> {
        let removed = self.storage.compact(&self.agent_id).await?;
        self.sequence.store(0, Ordering::SeqCst);
        Ok(removed)
    }

    /// Determine the last completed iteration from the journal.
    pub async fn last_completed_iteration(&self) -> Result<u32, JournalError> {
        let entries = self.storage.read_entries(&self.agent_id).await?;
        Ok(entries.iter().map(|e| e.iteration).max().unwrap_or(0))
    }
}

#[async_trait::async_trait]
impl JournalWriter for DurableJournal {
    async fn append(&self, mut entry: JournalEntry) -> Result<(), JournalError> {
        let seq = self.sequence.fetch_add(1, Ordering::SeqCst);
        entry.sequence = seq;
        entry.agent_id = self.agent_id;
        self.storage.store(&entry).await
    }

    async fn next_sequence(&self) -> u64 {
        self.sequence.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reasoning::loop_types::{LoopConfig, LoopEvent};

    fn make_entry(agent_id: AgentId, sequence: u64, iteration: u32) -> JournalEntry {
        JournalEntry {
            sequence,
            timestamp: chrono::Utc::now(),
            agent_id,
            iteration,
            event: LoopEvent::Started {
                agent_id,
                config: LoopConfig::default(),
            },
        }
    }

    #[tokio::test]
    async fn test_memory_storage_store_and_read() {
        let storage = MemoryJournalStorage::new();
        let agent = AgentId::new();

        storage.store(&make_entry(agent, 0, 0)).await.unwrap();
        storage.store(&make_entry(agent, 1, 1)).await.unwrap();

        let entries = storage.read_entries(&agent).await.unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].sequence, 0);
        assert_eq!(entries[1].sequence, 1);
    }

    #[tokio::test]
    async fn test_memory_storage_read_from() {
        let storage = MemoryJournalStorage::new();
        let agent = AgentId::new();

        for i in 0..5 {
            storage
                .store(&make_entry(agent, i, i as u32))
                .await
                .unwrap();
        }

        let entries = storage.read_from(&agent, 3).await.unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].sequence, 3);
        assert_eq!(entries[1].sequence, 4);
    }

    #[tokio::test]
    async fn test_memory_storage_latest_sequence() {
        let storage = MemoryJournalStorage::new();
        let agent = AgentId::new();

        assert_eq!(storage.latest_sequence(&agent).await.unwrap(), 0);

        storage.store(&make_entry(agent, 0, 0)).await.unwrap();
        storage.store(&make_entry(agent, 5, 2)).await.unwrap();

        assert_eq!(storage.latest_sequence(&agent).await.unwrap(), 5);
    }

    #[tokio::test]
    async fn test_memory_storage_compact() {
        let storage = MemoryJournalStorage::new();
        let agent = AgentId::new();

        storage.store(&make_entry(agent, 0, 0)).await.unwrap();
        storage.store(&make_entry(agent, 1, 1)).await.unwrap();

        let removed = storage.compact(&agent).await.unwrap();
        assert_eq!(removed, 2);

        let entries = storage.read_entries(&agent).await.unwrap();
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn test_memory_storage_agent_isolation() {
        let storage = MemoryJournalStorage::new();
        let agent_a = AgentId::new();
        let agent_b = AgentId::new();

        storage.store(&make_entry(agent_a, 0, 0)).await.unwrap();
        storage.store(&make_entry(agent_b, 0, 0)).await.unwrap();
        storage.store(&make_entry(agent_a, 1, 1)).await.unwrap();

        assert_eq!(storage.read_entries(&agent_a).await.unwrap().len(), 2);
        assert_eq!(storage.read_entries(&agent_b).await.unwrap().len(), 1);

        // Compacting agent_a shouldn't affect agent_b
        storage.compact(&agent_a).await.unwrap();
        assert_eq!(storage.read_entries(&agent_a).await.unwrap().len(), 0);
        assert_eq!(storage.read_entries(&agent_b).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_durable_journal_append_and_replay() {
        let storage = Arc::new(MemoryJournalStorage::new());
        let agent = AgentId::new();
        let journal = DurableJournal::new(storage, agent);

        journal.append(make_entry(agent, 0, 0)).await.unwrap();
        journal.append(make_entry(agent, 0, 1)).await.unwrap();

        assert_eq!(journal.next_sequence().await, 2);

        let entries = journal.replay().await.unwrap();
        assert_eq!(entries.len(), 2);
        // Sequence is set by the journal, not the caller
        assert_eq!(entries[0].sequence, 0);
        assert_eq!(entries[1].sequence, 1);
    }

    #[tokio::test]
    async fn test_durable_journal_replay_from() {
        let storage = Arc::new(MemoryJournalStorage::new());
        let agent = AgentId::new();
        let journal = DurableJournal::new(storage, agent);

        for _ in 0..5 {
            journal.append(make_entry(agent, 0, 0)).await.unwrap();
        }

        let entries = journal.replay_from(3).await.unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[tokio::test]
    async fn test_durable_journal_initialize_resumes_sequence() {
        let storage = Arc::new(MemoryJournalStorage::new());
        let agent = AgentId::new();

        // Write some entries directly to storage
        for i in 0..3 {
            storage
                .store(&make_entry(agent, i, i as u32))
                .await
                .unwrap();
        }

        // Create a new journal and initialize — should resume from sequence 2
        let journal = DurableJournal::new(storage, agent);
        journal.initialize().await.unwrap();
        assert_eq!(journal.next_sequence().await, 2);

        // Next append should get sequence 2
        journal.append(make_entry(agent, 0, 3)).await.unwrap();
        assert_eq!(journal.next_sequence().await, 3);
    }

    #[tokio::test]
    async fn test_durable_journal_compact() {
        let storage = Arc::new(MemoryJournalStorage::new());
        let agent = AgentId::new();
        let journal = DurableJournal::new(storage, agent);

        journal.append(make_entry(agent, 0, 0)).await.unwrap();
        journal.append(make_entry(agent, 0, 1)).await.unwrap();

        let removed = journal.compact().await.unwrap();
        assert_eq!(removed, 2);
        assert_eq!(journal.next_sequence().await, 0);

        let entries = journal.replay().await.unwrap();
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn test_last_completed_iteration() {
        let storage = Arc::new(MemoryJournalStorage::new());
        let agent = AgentId::new();
        let journal = DurableJournal::new(storage, agent);

        assert_eq!(journal.last_completed_iteration().await.unwrap(), 0);

        let mut entry = make_entry(agent, 0, 3);
        journal.append(entry.clone()).await.unwrap();
        entry.iteration = 7;
        journal.append(entry).await.unwrap();

        assert_eq!(journal.last_completed_iteration().await.unwrap(), 7);
    }
}
