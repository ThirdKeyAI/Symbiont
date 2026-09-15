//! One durable reservation history for a root and all delegated scopes.
use super::*;
use crate::{
    reasoning::{
        loop_types::{JournalEntry, JournalWriter, LoopEvent},
        run_audit::RunAuditReference,
    },
    types::AgentId,
};
use std::sync::Weak;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeIdentity {
    pub scope: usize,
    pub parent: Option<usize>,
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferenceReservation {
    pub id: uuid::Uuid,
    pub root_id: uuid::Uuid,
    /// Leaf first, root last. Each identity is immutable within the family.
    pub ancestors: Vec<ScopeIdentity>,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub agent_id: AgentId,
    pub audit: Option<RunAuditReference>,
    pub iteration: u32,
    pub provider: String,
    pub model: String,
    pub request_hash: String,
    pub request_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountingOutcome {
    Confirmed,
    Unknown,
    Exceeded,
}

fn outcome(usage: &Usage, amount: u32, output: u32) -> AccountingOutcome {
    if usage == &Usage::default() {
        return AccountingOutcome::Unknown;
    }
    if usage
        .prompt_tokens
        .checked_add(usage.completion_tokens)
        .is_none_or(|sum| usage.total_tokens < sum)
        || usage.total_tokens > amount
        || usage.completion_tokens > output
    {
        AccountingOutcome::Exceeded
    } else {
        AccountingOutcome::Confirmed
    }
}

/// The weak writer avoids a cycle with explicit in-memory journal implementations
/// that retain Started.config.shared_budget. Losing the root writer closes work.
pub(super) struct BudgetJournal {
    writer: Weak<dyn JournalWriter>,
    principal: AgentId,
    gate: tokio::sync::Mutex<()>,
    ready: std::sync::atomic::AtomicBool,
}
impl std::fmt::Debug for BudgetJournal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BudgetJournal")
            .field("principal", &self.principal)
            .finish_non_exhaustive()
    }
}
impl BudgetJournal {
    async fn record(&self, iteration: u32, event: LoopEvent) -> Result<(), String> {
        let writer = self
            .writer
            .upgrade()
            .ok_or("root budget journal is no longer available")?;
        writer
            .append(JournalEntry {
                sequence: writer.next_sequence().await,
                timestamp: chrono::Utc::now(),
                agent_id: self.principal,
                iteration,
                event,
            })
            .await
            .map_err(|e| e.to_string())
    }
}

impl SharedBudget {
    fn close(&self) {
        let mut ledger = self.lock();
        for id in self.ancestors(&ledger) {
            ledger.scopes[id].exceeded = true;
        }
    }

    /// Bind only after the required Started record. A ledger cannot move to a
    /// fresh root journal or import an allowance already consumed without audit.
    pub async fn attach(
        &self,
        writer: &Arc<dyn JournalWriter>,
        principal: AgentId,
    ) -> Result<(), String> {
        let (audit, fresh, root_id, limit) = {
            let mut ledger = self.lock();
            let fresh = ledger.audit.is_none();
            if fresh {
                let root = &ledger.scopes[0];
                if self.scope != 0
                    || root.reserved != 0
                    || root.uncertain != 0
                    || root.usage != Usage::default()
                    || root.exceeded
                {
                    return Err(
                        "budget needs an unused root journal before delegated inference".into(),
                    );
                }
                ledger.audit = Some(Arc::new(BudgetJournal {
                    writer: Arc::downgrade(writer),
                    principal,
                    gate: tokio::sync::Mutex::new(()),
                    ready: std::sync::atomic::AtomicBool::new(false),
                }));
            }
            let audit = ledger.audit.clone().unwrap();
            if self.scope == 0 && (!fresh || !Weak::ptr_eq(&audit.writer, &Arc::downgrade(writer)))
            {
                return Err(
                    "budget root is already attached; it cannot start another invocation".into(),
                );
            }
            (audit, fresh, ledger.root_id, ledger.scopes[0].limit)
        };
        let _guard = audit.gate.lock().await;
        let result = if fresh {
            let result = audit
                .record(0, LoopEvent::BudgetOpened { root_id, limit })
                .await;
            if result.is_ok() {
                audit
                    .ready
                    .store(true, std::sync::atomic::Ordering::Release);
            }
            result
        } else {
            if !audit.ready.load(std::sync::atomic::Ordering::Acquire) {
                return Err("root budget journal is not ready".into());
            }
            let root = audit
                .writer
                .upgrade()
                .ok_or("root budget journal is unavailable")?;
            writer
                .append(JournalEntry {
                    sequence: writer.next_sequence().await,
                    timestamp: chrono::Utc::now(),
                    agent_id: principal,
                    iteration: 0,
                    event: LoopEvent::BudgetScopeLinked {
                        root_id,
                        scope: self.scope,
                        root_audit: root.audit_reference(),
                    },
                })
                .await
                .map_err(|e| e.to_string())
        };
        if result.is_err() {
            self.close();
        }
        result
    }

    pub async fn reserve_audited(
        &self,
        input: u32,
        output: u32,
        proof: impl FnOnce(u32) -> Result<InferenceReservation, String>,
    ) -> Result<TokenReservation, String> {
        let audit = self
            .lock()
            .audit
            .clone()
            .ok_or("inference needs an attached root budget journal")?;
        let _guard = audit.gate.lock().await;
        if !audit.ready.load(std::sync::atomic::Ordering::Acquire) {
            return Err("root budget journal is not ready".into());
        }
        let mut reservation = self.reserve_inner(input, output, true)?;
        let mut proof = proof(reservation.output_tokens)?;
        {
            let ledger = self.lock();
            proof.id = reservation.id;
            proof.root_id = ledger.root_id;
            proof.ancestors = self
                .ancestors(&ledger)
                .into_iter()
                .map(|scope| ScopeIdentity {
                    scope,
                    parent: ledger.scopes[scope].parent,
                    limit: ledger.scopes[scope].limit,
                })
                .collect();
        }
        proof.input_tokens = input;
        proof.output_tokens = reservation.output_tokens;
        // A cancelled append may still finish in the storage worker. Retain the
        // charge even when control never returns to dispatch the provider call.
        reservation.mark_dispatched();
        let iteration_of_proof = proof.iteration;
        if let Err(error) = audit
            .record(
                proof.iteration,
                LoopEvent::BudgetReservationStarted { reservation: proof },
            )
            .await
        {
            self.close();
            return Err(format!("required inference reservation failed: {error}"));
        }
        self.track_outstanding(reservation.id, iteration_of_proof);
        Ok(reservation)
    }
}

impl super::SharedBudget {
    /// Journal a settlement for every reservation still open, so a run that was
    /// cancelled or timed out mid-inference leaves a terminal record instead of
    /// a gap. Usage is zero because it is genuinely unknown; what this
    /// establishes is that the reservation is no longer outstanding, which is
    /// the difference between a recorded failure and an unresolved run.
    pub async fn settle_outstanding(&self) {
        let (audit, open) = {
            let mut ledger = self.lock();
            (
                ledger.audit.clone(),
                std::mem::take(&mut ledger.outstanding),
            )
        };
        let Some(audit) = audit else { return };
        let root_id = self.lock().root_id;
        for (id, iteration) in open {
            let _guard = audit.gate.lock().await;
            let event = crate::reasoning::loop_types::LoopEvent::BudgetReservationFinished {
                root_id,
                reservation_id: id,
                usage: crate::reasoning::inference::Usage::default(),
                outcome: AccountingOutcome::Unknown,
            };
            if let Err(error) = audit.record(iteration, event).await {
                tracing::error!(%error, %id, "outstanding inference reservation could not be settled");
            }
        }
    }
}

impl TokenReservation {
    pub async fn settle_audited(mut self, usage: &Usage, iteration: u32) -> Result<(), String> {
        let audit = self
            .budget
            .lock()
            .audit
            .clone()
            .ok_or("inference budget journal unavailable")?;
        let _guard = audit.gate.lock().await;
        let root_id = self.budget.lock().root_id;
        let event = LoopEvent::BudgetReservationFinished {
            root_id,
            reservation_id: self.id,
            usage: usage.clone(),
            outcome: outcome(usage, self.amount, self.output_tokens),
        };
        if let Err(error) = audit.record(iteration, event).await {
            self.budget.close();
            return Err(format!("required inference settlement failed: {error}"));
        }
        self.budget.forget_outstanding(self.id);
        self.apply_settlement(usage)
    }
}

#[derive(Debug, Serialize)]
pub struct RecoveredReservation {
    pub reservation: InferenceReservation,
    pub start_sequence: u64,
    pub finish_sequence: Option<u64>,
    pub outcome: AccountingOutcome,
    pub usage: Option<Usage>,
}

/// Reconstructed accounting is evidence, not an execution or replay capability.
/// Unsettled requests may still be active; their full reservations stay charged.
#[derive(Debug, Serialize)]
pub struct RecoveredBudget {
    pub root_id: uuid::Uuid,
    pub scopes: Vec<BudgetSnapshot>,
    pub reservations: Vec<RecoveredReservation>,
}

pub fn recover(entries: &[JournalEntry]) -> Result<Option<RecoveredBudget>, String> {
    use std::collections::{BTreeMap, BTreeSet};
    let mut root = None;
    let mut scopes = BTreeMap::<usize, Scope>::new();
    let mut requests = BTreeMap::<uuid::Uuid, RecoveredReservation>::new();
    for entry in entries {
        match &entry.event {
            LoopEvent::BudgetOpened { root_id, limit } => {
                if root_id.is_nil() || root.replace(*root_id).is_some() {
                    return Err("duplicate root budget initialization".into());
                }
                scopes.insert(0, Scope::new(None, *limit));
            }
            LoopEvent::BudgetReservationStarted { reservation: r } => {
                if root != Some(r.root_id)
                    || r.id.is_nil()
                    || requests.contains_key(&r.id)
                    || requests.len() >= 32768
                {
                    return Err("invalid or duplicate budget reservation identity".into());
                }
                let amount = r
                    .input_tokens
                    .checked_add(r.output_tokens)
                    .ok_or("reservation amount overflow")?;
                if r.output_tokens == 0
                    || r.ancestors.is_empty()
                    || r.ancestors.len() > MAX_SCOPES
                    || r.ancestors
                        .last()
                        .is_none_or(|s| s.scope != 0 || s.parent.is_some())
                {
                    return Err("invalid reservation allowance or ancestry".into());
                }
                let mut seen = BTreeSet::new();
                for (index, identity) in r.ancestors.iter().enumerate().rev() {
                    if identity.scope >= MAX_SCOPES
                        || !seen.insert(identity.scope)
                        || identity.parent != r.ancestors.get(index + 1).map(|s| s.scope)
                    {
                        return Err("budget ancestry is cyclic, oversized or disconnected".into());
                    }
                    if let Some(parent) = identity.parent {
                        if identity.limit
                            > scopes.get(&parent).ok_or("missing budget ancestor")?.limit
                        {
                            return Err("child allowance exceeds its parent".into());
                        }
                    }
                    if let Some(existing) = scopes.get(&identity.scope) {
                        if existing.parent != identity.parent || existing.limit != identity.limit {
                            return Err("budget scope identity changed".into());
                        }
                    } else {
                        scopes.insert(identity.scope, Scope::new(identity.parent, identity.limit));
                    }
                    if scopes[&identity.scope].available() < amount {
                        return Err("recorded reservation exceeds the shared allowance".into());
                    }
                }
                for identity in &r.ancestors {
                    let scope = scopes.get_mut(&identity.scope).unwrap();
                    scope.reserved = scope
                        .reserved
                        .checked_add(amount)
                        .ok_or("reserved charge overflow")?;
                }
                requests.insert(
                    r.id,
                    RecoveredReservation {
                        reservation: r.clone(),
                        start_sequence: entry.sequence,
                        finish_sequence: None,
                        outcome: AccountingOutcome::Unknown,
                        usage: None,
                    },
                );
            }
            LoopEvent::BudgetReservationFinished {
                root_id,
                reservation_id,
                usage,
                outcome: recorded,
            } => {
                if root != Some(*root_id) {
                    return Err("settlement belongs to a different budget".into());
                }
                let request = requests
                    .get_mut(reservation_id)
                    .ok_or("settlement has no reservation")?;
                if request.finish_sequence.replace(entry.sequence).is_some() {
                    return Err("duplicate budget settlement".into());
                }
                let r = &request.reservation;
                let amount = r.input_tokens + r.output_tokens;
                let actual = outcome(usage, amount, r.output_tokens);
                if actual != *recorded {
                    return Err("settlement accounting outcome is inconsistent".into());
                }
                for identity in &r.ancestors {
                    let scope = scopes.get_mut(&identity.scope).unwrap();
                    scope.reserved = scope
                        .reserved
                        .checked_sub(amount)
                        .ok_or("settlement charge underflow")?;
                    if actual == AccountingOutcome::Unknown {
                        scope.uncertain = scope
                            .uncertain
                            .checked_add(amount)
                            .ok_or("uncertain charge overflow")?;
                    } else {
                        scope.usage.prompt_tokens = scope
                            .usage
                            .prompt_tokens
                            .saturating_add(usage.prompt_tokens);
                        scope.usage.completion_tokens = scope
                            .usage
                            .completion_tokens
                            .saturating_add(usage.completion_tokens);
                        let sum = usage.prompt_tokens.saturating_add(usage.completion_tokens);
                        scope.usage.total_tokens = scope
                            .usage
                            .total_tokens
                            .saturating_add(usage.total_tokens.max(sum));
                        scope.exceeded |= actual == AccountingOutcome::Exceeded;
                    }
                }
                request.outcome = actual;
                request.usage = Some(usage.clone());
            }
            _ => {}
        }
    }
    let Some(root_id) = root else {
        return Ok(None);
    };
    for request in requests.values().filter(|r| r.finish_sequence.is_none()) {
        let r = &request.reservation;
        let amount = r.input_tokens + r.output_tokens;
        for identity in &r.ancestors {
            let scope = scopes.get_mut(&identity.scope).unwrap();
            scope.reserved = scope
                .reserved
                .checked_sub(amount)
                .ok_or("unsettled charge underflow")?;
            scope.uncertain = scope
                .uncertain
                .checked_add(amount)
                .ok_or("unsettled charge overflow")?;
        }
    }
    let snapshots = scopes
        .iter()
        .map(|(id, scope)| {
            let mut available = scope.available();
            let mut exceeded = scope.exceeded;
            let mut parent = scope.parent;
            while let Some(id) = parent {
                let ancestor = &scopes[&id];
                available = available.min(ancestor.available());
                exceeded |= ancestor.exceeded;
                parent = ancestor.parent;
            }
            BudgetSnapshot {
                root_id,
                scope: *id,
                limit: scope.limit,
                usage: scope.usage.clone(),
                reserved_tokens: scope.reserved,
                uncertain_tokens: scope.uncertain,
                available_tokens: available,
                exceeded,
            }
        })
        .collect();
    let mut reservations: Vec<_> = requests.into_values().collect();
    reservations.sort_by_key(|request| request.start_sequence);
    Ok(Some(RecoveredBudget {
        root_id,
        scopes: snapshots,
        reservations,
    }))
}

#[cfg(test)]
mod tests;
