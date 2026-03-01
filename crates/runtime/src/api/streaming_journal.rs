//! Streaming journal bridge for WebSocket event forwarding.
//!
//! Wraps a [`BufferedJournal`] and additionally pushes every
//! [`JournalEntry`] into a `tokio::sync::mpsc` channel so that a
//! WebSocket writer task can forward events to the browser in real-time.

#[cfg(feature = "http-api")]
use std::sync::Arc;

#[cfg(feature = "http-api")]
use tokio::sync::mpsc;

#[cfg(feature = "http-api")]
use crate::reasoning::loop_types::{BufferedJournal, JournalEntry, JournalError, JournalWriter};

/// A journal that writes to an inner [`BufferedJournal`] and simultaneously
/// forwards each entry to an mpsc channel for real-time streaming.
#[cfg(feature = "http-api")]
pub struct StreamingJournal {
    inner: Arc<BufferedJournal>,
    tx: mpsc::Sender<JournalEntry>,
}

#[cfg(feature = "http-api")]
impl StreamingJournal {
    /// Create a new streaming journal.
    ///
    /// * `inner` — The underlying buffered journal for persistence.
    /// * `tx` — Channel sender; the receiver end is read by the WebSocket
    ///   writer task.
    pub fn new(inner: Arc<BufferedJournal>, tx: mpsc::Sender<JournalEntry>) -> Self {
        Self { inner, tx }
    }
}

#[cfg(feature = "http-api")]
#[async_trait::async_trait]
impl JournalWriter for StreamingJournal {
    async fn append(&self, entry: JournalEntry) -> Result<(), JournalError> {
        // Write to the inner journal first (always succeeds for BufferedJournal).
        self.inner.append(entry.clone()).await?;

        // Forward to the channel. Use `try_send` so we never block the
        // reasoning loop if the WebSocket consumer is slow — we just drop
        // the event (the inner journal still has it).
        if let Err(e) = self.tx.try_send(entry) {
            tracing::debug!("Streaming journal channel full or closed: {}", e);
        }

        Ok(())
    }

    async fn next_sequence(&self) -> u64 {
        self.inner.next_sequence().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reasoning::loop_types::{LoopConfig, LoopEvent};
    use crate::types::AgentId;

    #[tokio::test]
    async fn streaming_journal_forwards_to_channel() {
        let inner = Arc::new(BufferedJournal::new(100));
        let (tx, mut rx) = mpsc::channel(16);
        let journal = StreamingJournal::new(inner.clone(), tx);

        let entry = JournalEntry {
            sequence: 0,
            timestamp: chrono::Utc::now(),
            agent_id: AgentId::new(),
            iteration: 0,
            event: LoopEvent::Started {
                agent_id: AgentId::new(),
                config: LoopConfig::default(),
            },
        };

        journal.append(entry).await.unwrap();

        // Inner journal has the entry
        assert_eq!(inner.entries().await.len(), 1);

        // Channel received the entry
        let received = rx.try_recv().unwrap();
        assert_eq!(received.sequence, 0);
    }

    #[tokio::test]
    async fn streaming_journal_does_not_block_when_channel_full() {
        let inner = Arc::new(BufferedJournal::new(100));
        // Channel with capacity 1
        let (tx, _rx) = mpsc::channel(1);
        let journal = StreamingJournal::new(inner.clone(), tx);

        let make_entry = |seq: u64| JournalEntry {
            sequence: seq,
            timestamp: chrono::Utc::now(),
            agent_id: AgentId::new(),
            iteration: 0,
            event: LoopEvent::Started {
                agent_id: AgentId::new(),
                config: LoopConfig::default(),
            },
        };

        // Fill the channel
        journal.append(make_entry(0)).await.unwrap();
        // This should not block — just drops the forwarded event
        journal.append(make_entry(1)).await.unwrap();

        // Inner journal still has both
        assert_eq!(inner.entries().await.len(), 2);
    }
}
