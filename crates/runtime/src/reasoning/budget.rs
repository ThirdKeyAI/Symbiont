//! Shared token accounting for a root invocation and its descendants.
//!
//! Reservations cover input and maximum output before inference. Confirmed
//! usage settles the reservation; cancellation or missing usage retains its
//! entire charge. One lock owns all ancestor balances, including sibling races.

use super::inference::Usage;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, MutexGuard};

pub mod journal;

const MAX_SCOPES: usize = 4096;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetSnapshot {
    pub root_id: uuid::Uuid,
    pub scope: usize,
    pub limit: u32,
    pub usage: Usage,
    pub reserved_tokens: u32,
    pub uncertain_tokens: u32,
    pub available_tokens: u32,
    pub exceeded: bool,
}

#[derive(Debug)]
struct Scope {
    parent: Option<usize>,
    limit: u32,
    usage: Usage,
    reserved: u32,
    uncertain: u32,
    exceeded: bool,
}

impl Scope {
    fn new(parent: Option<usize>, limit: u32) -> Self {
        Self {
            parent,
            limit,
            usage: Usage::default(),
            reserved: 0,
            uncertain: 0,
            exceeded: false,
        }
    }

    fn available(&self) -> u32 {
        if self.exceeded {
            return 0;
        }
        self.limit
            .saturating_sub(self.usage.total_tokens)
            .saturating_sub(self.uncertain)
            .saturating_sub(self.reserved)
    }
}

#[derive(Debug)]
struct Ledger {
    root_id: uuid::Uuid,
    scopes: Vec<Scope>,
    audit: Option<Arc<journal::BudgetJournal>>,
    outstanding: Vec<OutstandingReservation>,
}

#[derive(Debug, Clone, Copy)]
struct OutstandingReservation {
    id: uuid::Uuid,
    scope: usize,
    iteration: u32,
    abandoned: bool,
    // An interrupted append may still become durable. Never append a second
    // finish merely because its caller did not observe the first write return.
    settlement_started: bool,
}

/// An unforgeable in-process reference to one scope in a shared ledger.
/// Serialized configuration never reconstructs or supplies this authority.
#[derive(Debug, Clone)]
pub struct SharedBudget {
    ledger: Arc<Mutex<Ledger>>,
    scope: usize,
}

impl SharedBudget {
    pub(crate) fn track_outstanding(&self, id: uuid::Uuid, iteration: u32) {
        self.lock().outstanding.push(OutstandingReservation {
            id,
            scope: self.scope,
            iteration,
            abandoned: false,
            settlement_started: false,
        });
    }

    pub(crate) fn forget_outstanding(&self, id: uuid::Uuid) {
        self.lock().outstanding.retain(|open| open.id != id);
    }

    fn begin_settlement(&self, id: uuid::Uuid) -> Result<(), String> {
        let mut ledger = self.lock();
        let open = ledger
            .outstanding
            .iter_mut()
            .find(|open| open.id == id && open.scope == self.scope)
            .ok_or("inference reservation is not outstanding in this scope")?;
        if open.settlement_started {
            return Err("inference settlement was already attempted".into());
        }
        open.settlement_started = true;
        Ok(())
    }

    pub fn new(limit: u32) -> Self {
        Self {
            ledger: Arc::new(Mutex::new(Ledger {
                root_id: uuid::Uuid::new_v4(),
                scopes: vec![Scope::new(None, limit)],
                audit: None,
                outstanding: Vec::new(),
            })),
            scope: 0,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Ledger> {
        // A panic during accounting must not reopen an allowance.
        self.ledger.lock().unwrap_or_else(|poison| {
            let mut ledger = poison.into_inner();
            for scope in &mut ledger.scopes {
                scope.exceeded = true;
            }
            ledger
        })
    }

    fn ancestors(&self, ledger: &Ledger) -> Vec<usize> {
        let mut chain = vec![self.scope];
        while let Some(parent) = ledger.scopes[*chain.last().unwrap()].parent {
            chain.push(parent);
        }
        chain
    }

    /// Create a tighter child scope. Creation cannot increase any allowance.
    pub fn child(&self, limit: u32) -> Result<Self, String> {
        let mut ledger = self.lock();
        if ledger.scopes.len() >= MAX_SCOPES {
            return Err("shared budget delegation scope limit reached".into());
        }
        let limit = limit.min(ledger.scopes[self.scope].limit);
        let scope = ledger.scopes.len();
        ledger.scopes.push(Scope::new(Some(self.scope), limit));
        Ok(Self {
            ledger: self.ledger.clone(),
            scope,
        })
    }

    pub fn snapshot(&self) -> BudgetSnapshot {
        let ledger = self.lock();
        let scope = &ledger.scopes[self.scope];
        let chain = self.ancestors(&ledger);
        BudgetSnapshot {
            root_id: ledger.root_id,
            scope: self.scope,
            limit: scope.limit,
            usage: scope.usage.clone(),
            reserved_tokens: scope.reserved,
            uncertain_tokens: scope.uncertain,
            available_tokens: chain
                .iter()
                .map(|id| ledger.scopes[*id].available())
                .min()
                .unwrap_or(0),
            exceeded: chain.iter().any(|id| ledger.scopes[*id].exceeded),
        }
    }

    /// Reserve input plus output atomically against every ancestor. Output is
    /// tightened to the remaining allowance; an exhausted budget sends no call.
    pub fn reserve(&self, input_tokens: u32, max_output: u32) -> Result<TokenReservation, String> {
        self.reserve_inner(input_tokens, max_output, false)
    }

    fn reserve_inner(
        &self,
        input_tokens: u32,
        max_output: u32,
        audited: bool,
    ) -> Result<TokenReservation, String> {
        let mut ledger = self.lock();
        if ledger.audit.is_some() && !audited {
            return Err("attached budgets require durable inference reservations".into());
        }
        let chain = self.ancestors(&ledger);
        let available = chain
            .iter()
            .map(|id| ledger.scopes[*id].available())
            .min()
            .unwrap_or(0);
        let output_tokens = available.saturating_sub(input_tokens).min(max_output);
        if output_tokens == 0 {
            return Err("shared token budget exhausted or already reserved".into());
        }
        let amount = input_tokens + output_tokens; // bounded by available
        for id in chain {
            ledger.scopes[id].reserved += amount;
        }
        Ok(TokenReservation {
            budget: self.clone(),
            amount,
            output_tokens,
            dispatched: false,
            active: true,
            id: uuid::Uuid::new_v4(),
        })
    }
}

/// A reservation is consumed exactly once. Dropping a dispatched reservation
/// conservatively charges uncertainty; dropping before dispatch refunds it.
pub struct TokenReservation {
    budget: SharedBudget,
    amount: u32,
    output_tokens: u32,
    dispatched: bool,
    active: bool,
    id: uuid::Uuid,
}

impl TokenReservation {
    pub fn output_tokens(&self) -> u32 {
        self.output_tokens
    }

    pub fn mark_dispatched(&mut self) {
        self.dispatched = true;
    }

    /// Missing counts remain uncertain. Inconsistent or over-reservation usage
    /// closes ancestor budgets and refuses response-driven effects.
    pub fn settle(mut self, usage: &Usage) -> Result<(), String> {
        if self.budget.lock().audit.is_some() {
            return Err("attached budgets require durable inference settlement".into());
        }
        self.apply_settlement(usage)
    }

    fn apply_settlement(&mut self, usage: &Usage) -> Result<(), String> {
        if usage.total_tokens == 0 && usage.prompt_tokens == 0 && usage.completion_tokens == 0 {
            // Drop preserves the full dispatched reservation. A sibling may
            // already have closed the ancestor while this request was pending.
            return if self.budget.snapshot().exceeded {
                Err("provider usage violated its shared token reservation".into())
            } else {
                Ok(())
            };
        }
        let sum = usage.prompt_tokens.checked_add(usage.completion_tokens);
        let invalid = sum.is_none_or(|sum| usage.total_tokens < sum)
            || usage.total_tokens > self.amount
            || usage.completion_tokens > self.output_tokens;
        let mut ledger = self.budget.lock();
        for id in self.budget.ancestors(&ledger) {
            let scope = &mut ledger.scopes[id];
            scope.reserved -= self.amount;
            scope.usage.prompt_tokens = scope
                .usage
                .prompt_tokens
                .saturating_add(usage.prompt_tokens);
            scope.usage.completion_tokens = scope
                .usage
                .completion_tokens
                .saturating_add(usage.completion_tokens);
            scope.usage.total_tokens = scope
                .usage
                .total_tokens
                .saturating_add(usage.total_tokens.max(sum.unwrap_or(u32::MAX)));
            scope.exceeded |= invalid;
        }
        self.active = false;
        if invalid
            || self
                .budget
                .ancestors(&ledger)
                .iter()
                .any(|id| ledger.scopes[*id].exceeded)
        {
            Err("provider usage violated its shared token reservation".into())
        } else {
            Ok(())
        }
    }
}

impl Drop for TokenReservation {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let mut ledger = self.budget.lock();
        for id in self.budget.ancestors(&ledger) {
            let scope = &mut ledger.scopes[id];
            scope.reserved -= self.amount;
            if self.dispatched {
                scope.uncertain = scope.uncertain.saturating_add(self.amount);
            }
        }
        if let Some(open) = ledger
            .outstanding
            .iter_mut()
            .find(|open| open.id == self.id)
        {
            open.abandoned = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(tokens: u32) -> Usage {
        Usage {
            prompt_tokens: 0,
            completion_tokens: tokens,
            total_tokens: tokens,
        }
    }

    #[test]
    fn descendants_share_remaining_allowance_and_settle_once() {
        let root = SharedBudget::new(10_000);
        root.reserve(0, 3_000)
            .unwrap()
            .settle(&usage(3_000))
            .unwrap();
        let a = root.child(10_000).unwrap();
        let b = root.child(10_000).unwrap();
        let reservation = a.reserve(500, 3_500).unwrap();
        assert_eq!(b.snapshot().available_tokens, 3_000);
        let sibling = b.reserve(500, 3_500).unwrap();
        assert_eq!(sibling.output_tokens(), 2_500);
        assert!(root.reserve(0, 1).is_err());
        drop(sibling); // never dispatched
        reservation.settle(&usage(2_500)).unwrap();
        assert_eq!(root.snapshot().usage.total_tokens, 5_500);
        assert_eq!(root.snapshot().available_tokens, 4_500);
        assert_eq!(a.snapshot().usage.total_tokens, 2_500);
        assert_eq!(b.snapshot().usage.total_tokens, 0);
        let grandchild = a.child(1_000).unwrap();
        grandchild
            .reserve(0, 5_000)
            .unwrap()
            .settle(&usage(800))
            .unwrap();
        assert_eq!(a.snapshot().usage.total_tokens, 3_300);
        assert_eq!(root.snapshot().usage.total_tokens, 6_300);
        assert_eq!(grandchild.snapshot().available_tokens, 200);
    }

    #[test]
    fn cancellation_and_missing_usage_never_mint_capacity() {
        let root = SharedBudget::new(100);
        let child = root.child(100).unwrap();
        let mut interrupted = child.reserve(10, 30).unwrap();
        interrupted.mark_dispatched();
        drop(interrupted);
        assert_eq!(root.snapshot().uncertain_tokens, 40);
        let mut missing = root.reserve(10, 20).unwrap();
        missing.mark_dispatched();
        missing.settle(&Usage::default()).unwrap();
        assert_eq!(root.snapshot().uncertain_tokens, 70);
        assert_eq!(root.snapshot().available_tokens, 30);
        assert_eq!(root.snapshot().reserved_tokens, 0);
    }

    #[test]
    fn untrusted_usage_cannot_overflow_or_reopen_ancestors() {
        for reported in [
            usage(51),
            Usage {
                prompt_tokens: u32::MAX,
                completion_tokens: 1,
                total_tokens: 1,
            },
        ] {
            let root = SharedBudget::new(100);
            let child = root.child(100).unwrap();
            assert!(child.reserve(0, 50).unwrap().settle(&reported).is_err());
            assert!(root.snapshot().exceeded);
            assert!(root.reserve(0, 1).is_err());
        }
    }

    #[test]
    fn simultaneous_siblings_cannot_overreserve() {
        let root = SharedBudget::new(100);
        let barrier = Arc::new(std::sync::Barrier::new(9));
        std::thread::scope(|threads| {
            for _ in 0..8 {
                let child = root.child(100).unwrap();
                let barrier = barrier.clone();
                threads.spawn(move || {
                    let reservation = child.reserve(0, 20).ok();
                    barrier.wait();
                    barrier.wait();
                    drop(reservation);
                });
            }
            barrier.wait();
            assert_eq!(root.snapshot().reserved_tokens, 100);
            assert_eq!(root.snapshot().available_tokens, 0);
            barrier.wait();
        });
        assert_eq!(root.snapshot().available_tokens, 100);
    }
}
