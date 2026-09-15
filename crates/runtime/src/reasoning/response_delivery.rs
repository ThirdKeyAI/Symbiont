//! Delivery of an immutable response authorized by the ordinary phase gate.
use super::{
    loop_types::{JournalEntry, JournalWriter, LoopEvent},
    prepared::{canonical_json, digest_json, AuthorizedAction, PreparedAction},
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

/// A trusted runtime adapter. Preparation must include the complete destination
/// and formatted message; sending consumes that exact prepared contract.
#[async_trait]
pub trait ResponseDestination: Send + Sync {
    fn context(&self) -> Value;
    fn validate(&self) -> Result<(), String>;
    fn prepare(&self, content: &str) -> Result<Value, String>;
    async fn send(&self, request: &Value) -> Result<DeliveryReceipt, String>;
}

pub struct DeliveryReceipt {
    pub receipt: Value,
    /// False includes a rejected, mismatched or otherwise unconfirmed delivery.
    pub confirmed: bool,
}

struct PreparedDelivery {
    destination: Arc<dyn ResponseDestination>,
    request: Value,
}

pub(super) fn prepare(
    action: PreparedAction,
    destination: Arc<dyn ResponseDestination>,
    content: &str,
) -> Result<PreparedAction, String> {
    let request = destination.prepare(content)?;
    if canonical_json(&request)?.len() > 4 * 1024 * 1024 {
        return Err("formatted response exceeds 4 MiB".into());
    }
    action
        .with_resolved(json!({"response_delivery":request}))
        .map(|action| {
            action.with_backend(PreparedDelivery {
                destination,
                request,
            })
        })
}

/// Called only after the required policy record and grant binding check. Plain
/// text responses have no delivery capability. Missing audit refuses a send.
pub(super) async fn dispatch(
    grant: AuthorizedAction,
    journal: Option<&dyn JournalWriter>,
) -> Result<(), String> {
    grant.check_live()?;
    let Some(delivery) = grant.prepared().backend::<PreparedDelivery>() else {
        return Ok(());
    };
    let journal = journal.ok_or("response delivery requires an audit writer")?;
    let fingerprint = grant.prepared().fingerprint().to_owned();
    let request_hash = digest_json(&delivery.request)?;
    let request_bytes = canonical_json(&delivery.request)?.len() as u64;
    let append = |event| async {
        journal
            .append(JournalEntry {
                sequence: journal.next_sequence().await,
                timestamp: chrono::Utc::now(),
                agent_id: grant.principal(),
                iteration: grant.iteration(),
                event,
            })
            .await
            .map_err(|error| format!("required response delivery audit failed: {error}"))
    };
    append(LoopEvent::ResponseDeliveryStarted {
        fingerprint: fingerprint.clone(),
        request_hash,
        request_bytes,
    })
    .await?;
    grant.check_live()?;
    let result = tokio::time::timeout(
        grant
            .deadline()
            .saturating_duration_since(std::time::Instant::now()),
        delivery.destination.send(&delivery.request),
    )
    .await
    .unwrap_or_else(|_| Err("response delivery timed out; receipt is unconfirmed".into()));
    let (receipt, confirmed, error) = match result {
        Ok(receipt) if canonical_json(&receipt.receipt)?.len() <= 64 * 1024 => {
            let error =
                (!receipt.confirmed).then(|| "response delivery was not confirmed".to_owned());
            (Some(receipt.receipt), receipt.confirmed, error)
        }
        Ok(_) => (None, false, Some("delivery receipt exceeds 64 KiB".into())),
        Err(error) => (None, false, Some(error.chars().take(4096).collect())),
    };
    append(LoopEvent::ResponseDeliveryFinished {
        fingerprint,
        receipt,
        confirmed,
        error: error.clone(),
    })
    .await?;
    match error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}
