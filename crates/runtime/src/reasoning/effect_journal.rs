//! Call-bound records for effects discovered during an authorized invocation.
//!
//! The dispatcher lends its journal through a bounded channel. Workers never
//! receive the writer or signing material; returning or cancelling dispatch
//! closes the channel and prevents further acknowledged effects.
use super::loop_types::{JournalEntry, JournalError, JournalWriter, LoopEvent};
use crate::types::AgentId;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};
use tokio::sync::{mpsc, oneshot};

/// Request and response hashes describe exact application bytes, not TLS wire
/// framing. Bodies and credential-bearing headers stay out of these records.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolEffect {
    FilePublicationPrepared {
        intent: crate::sandbox::files::PublicationIntent,
    },
    FilePublicationFinished {
        publication_id: uuid::Uuid,
        confirmed: bool,
        error: Option<String>,
    },
    NetworkRequestStarted {
        request_id: String,
        method: String,
        url: String,
        request_hash: String,
        request_bytes: u64,
    },
    NetworkRequestFinished {
        request_id: String,
        status: Option<u16>,
        response_hash: Option<String>,
        response_headers_hash: Option<String>,
        response_bytes: u64,
        error: Option<String>,
    },
}

pub(super) struct Record {
    agent_id: AgentId,
    iteration: u32,
    run_key: String,
    fingerprint: String,
    event: ToolEffect,
    deadline: Instant,
    reply: oneshot::Sender<Result<(), String>>,
    outcome: Arc<Mutex<EffectState>>,
}

#[derive(Debug, Default)]
struct EffectState {
    pending: HashSet<String>,
    failed: bool,
}

impl EffectState {
    fn record(&mut self, effect: &ToolEffect) {
        match effect {
            ToolEffect::FilePublicationPrepared { intent } => {
                self.failed |= !self
                    .pending
                    .insert(format!("file:{}", intent.publication_id));
            }
            ToolEffect::FilePublicationFinished {
                publication_id,
                confirmed,
                error,
            } => {
                self.failed |= !self.pending.remove(&format!("file:{publication_id}"))
                    || !confirmed
                    || error.is_some();
            }
            ToolEffect::NetworkRequestStarted { request_id, .. } => {
                self.failed |= !self.pending.insert(request_id.clone());
            }
            ToolEffect::NetworkRequestFinished {
                request_id,
                status,
                response_hash,
                error,
                ..
            } => {
                self.failed |= !self.pending.remove(request_id)
                    || status.is_none()
                    || response_hash.is_none()
                    || error.is_some();
            }
        }
    }
}

/// Non-deserializable authority, issued only after the policy checkpoint.
#[derive(Clone, Debug)]
pub(crate) struct EffectJournal {
    sender: mpsc::Sender<Record>,
    agent_id: AgentId,
    iteration: u32,
    run_key: String,
    fingerprint: String,
    deadline: Instant,
    records: Arc<AtomicUsize>,
    outcome: Arc<Mutex<EffectState>>,
}

impl EffectJournal {
    pub(super) fn new(
        sender: mpsc::Sender<Record>,
        grant: &super::prepared::AuthorizedAction,
    ) -> Self {
        Self {
            sender,
            agent_id: grant.principal(),
            iteration: grant.iteration(),
            run_key: grant.run_key().into(),
            fingerprint: grant.prepared().fingerprint().into(),
            deadline: grant.deadline(),
            records: Arc::new(AtomicUsize::new(0)),
            outcome: Arc::new(Mutex::new(EffectState::default())),
        }
    }

    pub(crate) fn check_live(&self) -> Result<(), String> {
        self.check_channel()?;
        if self.outcome.lock().map_or(true, |state| state.failed) {
            return Err("an earlier effect is unconfirmed; further requests are refused".into());
        }
        Ok(())
    }

    fn check_channel(&self) -> Result<(), String> {
        if self.sender.is_closed() || Instant::now() >= self.deadline {
            return Err("effect journal authority is closed or expired".into());
        }
        Ok(())
    }

    pub(super) fn has_unconfirmed_effect(&self) -> bool {
        self.outcome
            .lock()
            .map_or(true, |state| state.failed || !state.pending.is_empty())
    }

    pub(crate) async fn append(&self, event: ToolEffect) -> Result<(), String> {
        // Outcomes from work already in flight must remain recordable after
        // another request fails. New effects still require live authority.
        let starts_effect = matches!(
            event,
            ToolEffect::NetworkRequestStarted { .. } | ToolEffect::FilePublicationPrepared { .. }
        );
        if starts_effect {
            self.check_live()?;
        } else {
            self.check_channel()?;
        }
        if self.records.fetch_add(1, Ordering::Relaxed) >= 2048 {
            return Err("tool effect journal record limit exceeded".into());
        }
        if serde_json::to_vec(&event).map_err(|e| e.to_string())?.len() > 32 * 1024 {
            return Err("tool effect journal record exceeds its byte limit".into());
        }
        tokio::time::timeout_at(self.deadline.into(), async {
            let (reply, received) = oneshot::channel();
            self.sender
                .send(Record {
                    agent_id: self.agent_id,
                    iteration: self.iteration,
                    run_key: self.run_key.clone(),
                    fingerprint: self.fingerprint.clone(),
                    event,
                    deadline: self.deadline,
                    reply,
                    outcome: self.outcome.clone(),
                })
                .await
                .map_err(|_| "effect journal dispatcher closed")?;
            received
                .await
                .map_err(|_| "effect journal ended without acknowledgement")?
        })
        .await
        .map_err(|_| "effect journal acknowledgement timed out".to_string())??;
        if starts_effect {
            self.check_live()
        } else {
            self.check_channel()
        }
    }
}

pub(super) async fn serve(
    writer: &dyn JournalWriter,
    mut receiver: mpsc::Receiver<Record>,
) -> Result<(), JournalError> {
    while let Some(record) = receiver.recv().await {
        if record.reply.is_closed() || Instant::now() >= record.deadline {
            let _ = record
                .reply
                .send(Err("effect journal request expired".into()));
            continue;
        }
        if matches!(
            record.event,
            ToolEffect::NetworkRequestStarted { .. } | ToolEffect::FilePublicationPrepared { .. }
        ) && record.outcome.lock().map_or(true, |state| state.failed)
        {
            let _ = record.reply.send(Err(
                "an earlier effect is unconfirmed; further requests are refused".into(),
            ));
            continue;
        }
        let result = writer
            .append(JournalEntry {
                sequence: writer.next_sequence().await,
                timestamp: chrono::Utc::now(),
                agent_id: record.agent_id,
                iteration: record.iteration,
                event: LoopEvent::ToolEffect {
                    run_key: record.run_key,
                    call_fingerprint: record.fingerprint,
                    effect: record.event.clone(),
                },
            })
            .await;
        if result.is_ok() {
            if let Ok(mut state) = record.outcome.lock() {
                state.record(&record.event);
            }
        }
        let _ = record
            .reply
            .send(result.as_ref().map(|_| ()).map_err(ToString::to_string));
        result?;
    }
    Ok(())
}
