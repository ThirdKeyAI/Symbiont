use super::*;
use crate::reasoning::loop_types::{BufferedJournal, JournalError};

fn proof(output: u32) -> Result<InferenceReservation, String> {
    Ok(InferenceReservation {
        id: uuid::Uuid::nil(),
        root_id: uuid::Uuid::nil(),
        ancestors: vec![],
        input_tokens: 0,
        output_tokens: output,
        agent_id: AgentId::new(),
        audit: None,
        iteration: 1,
        provider: "fixture".into(),
        model: "fixture".into(),
        request_hash: "sha256:".to_owned() + &"a".repeat(64),
        request_bytes: 10,
    })
}

async fn attached(limit: u32) -> (SharedBudget, Arc<BufferedJournal>) {
    let budget = SharedBudget::new(limit);
    let journal = Arc::new(BufferedJournal::new(100));
    budget
        .attach(&(journal.clone() as Arc<dyn JournalWriter>), AgentId::new())
        .await
        .unwrap();
    (budget, journal)
}

#[tokio::test]
async fn durable_siblings_recover_one_root_allowance_and_uncertain_charge() {
    let (budget, journal) = attached(100).await;
    let a = budget.child(80).unwrap();
    let b = budget.child(80).unwrap();
    let (a, b) = tokio::join!(
        a.reserve_audited(10, 40, proof),
        b.reserve_audited(10, 40, proof)
    );
    assert!(budget.reserve_audited(0, 1, proof).await.is_err());
    a.unwrap()
        .settle_audited(
            &Usage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
            },
            1,
        )
        .await
        .unwrap();
    drop(b.unwrap());
    let recovered = recover(&journal.entries().await).unwrap().unwrap();
    let root = &recovered.scopes[0];
    assert_eq!(root.usage.total_tokens, 15);
    assert_eq!(root.uncertain_tokens, 50);
    assert_eq!(root.available_tokens, 35);
    assert_eq!(root.available_tokens, budget.snapshot().available_tokens);
    assert_eq!(recovered.reservations.len(), 2);
    assert_eq!(recovered.scopes.len(), 3);
    assert!(recovered
        .reservations
        .iter()
        .any(|r| r.finish_sequence.is_none()));
}

#[tokio::test]
async fn missing_usage_stays_charged_and_invalid_usage_closes_all_scopes() {
    let (budget, journal) = attached(100).await;
    budget
        .reserve_audited(10, 20, proof)
        .await
        .unwrap()
        .settle_audited(&Usage::default(), 1)
        .await
        .unwrap();
    assert_eq!(budget.snapshot().uncertain_tokens, 30);
    let child = budget.child(100).unwrap();
    let pending = budget.reserve_audited(0, 10, proof).await.unwrap();
    let missing = budget.reserve_audited(0, 10, proof).await.unwrap();
    assert!(child
        .reserve_audited(0, 10, proof)
        .await
        .unwrap()
        .settle_audited(
            &Usage {
                prompt_tokens: 0,
                completion_tokens: 11,
                total_tokens: 11
            },
            1
        )
        .await
        .is_err());
    // A sibling already in flight cannot use its response after an ancestor closes.
    assert!(pending
        .settle_audited(
            &Usage {
                prompt_tokens: 0,
                completion_tokens: 1,
                total_tokens: 1
            },
            1
        )
        .await
        .is_err());
    assert!(missing.settle_audited(&Usage::default(), 1).await.is_err());
    let recovered = recover(&journal.entries().await).unwrap().unwrap();
    assert!(recovered
        .scopes
        .iter()
        .all(|s| s.exceeded && s.available_tokens == 0));
    assert_eq!(recovered.scopes[0].uncertain_tokens, 40);
    assert_eq!(recovered.scopes[0].usage.total_tokens, 12);
}

#[tokio::test]
async fn recovery_rejects_duplicate_settlements_and_changed_scope_ancestry() {
    let (budget, journal) = attached(100).await;
    let child = budget.child(80).unwrap();
    for _ in 0..2 {
        child
            .reserve_audited(1, 9, proof)
            .await
            .unwrap()
            .settle_audited(
                &Usage {
                    prompt_tokens: 1,
                    completion_tokens: 1,
                    total_tokens: 2,
                },
                1,
            )
            .await
            .unwrap();
    }
    let original = journal.entries().await;
    let mut duplicate = original.clone();
    duplicate.push(original.last().unwrap().clone());
    assert!(recover(&duplicate).unwrap_err().contains("duplicate"));
    let mut changed = original.clone();
    let starts: Vec<_> = changed
        .iter_mut()
        .filter_map(|entry| match &mut entry.event {
            LoopEvent::BudgetReservationStarted { reservation } => Some(reservation),
            _ => None,
        })
        .collect();
    starts.into_iter().last().unwrap().ancestors[0].limit = 79;
    assert!(recover(&changed).unwrap_err().contains("identity changed"));
    let mut disconnected = original;
    if let LoopEvent::BudgetReservationStarted { reservation } = &mut disconnected[1].event {
        reservation.ancestors[0].parent = Some(reservation.ancestors[0].scope);
    }
    assert!(recover(&disconnected).is_err());
}

struct ControlledWriter {
    inner: BufferedJournal,
    entered: tokio::sync::Notify,
    block_start: bool,
    block_finish: bool,
    fail_finish: bool,
}
#[async_trait::async_trait]
impl JournalWriter for ControlledWriter {
    async fn append(&self, entry: JournalEntry) -> Result<(), JournalError> {
        let is_start = matches!(entry.event, LoopEvent::BudgetReservationStarted { .. });
        let is_finish = matches!(entry.event, LoopEvent::BudgetReservationFinished { .. });
        if self.fail_finish && matches!(entry.event, LoopEvent::BudgetReservationFinished { .. }) {
            return Err(JournalError::WriteFailed(
                "fixture settlement outage".into(),
            ));
        }
        self.inner.append(entry).await?;
        if (is_start && self.block_start) || (is_finish && self.block_finish) {
            self.entered.notify_one();
            std::future::pending::<()>().await;
        }
        Ok(())
    }
    async fn next_sequence(&self) -> u64 {
        self.inner.next_sequence().await
    }
}

#[tokio::test]
async fn cancellation_during_required_append_does_not_refund_durable_intent() {
    let budget = SharedBudget::new(100);
    let writer = Arc::new(ControlledWriter {
        inner: BufferedJournal::new(100),
        entered: Default::default(),
        block_start: true,
        block_finish: false,
        fail_finish: false,
    });
    budget
        .attach(&(writer.clone() as Arc<dyn JournalWriter>), AgentId::new())
        .await
        .unwrap();
    tokio::select! {
        _ = writer.entered.notified() => {},
        _ = budget.reserve_audited(10,30,proof) => panic!("append should remain pending"),
    }
    assert_eq!(budget.snapshot().uncertain_tokens, 40);
    let recovered = recover(&writer.inner.entries().await).unwrap().unwrap();
    assert_eq!(recovered.scopes[0].uncertain_tokens, 40);
    assert_eq!(recovered.scopes[0].available_tokens, 60);
}

#[tokio::test]
async fn failed_settlement_retains_full_charge_and_blocks_more_calls() {
    let budget = SharedBudget::new(100);
    let writer = Arc::new(ControlledWriter {
        inner: BufferedJournal::new(100),
        entered: Default::default(),
        block_start: false,
        block_finish: false,
        fail_finish: true,
    });
    budget
        .attach(&(writer.clone() as Arc<dyn JournalWriter>), AgentId::new())
        .await
        .unwrap();
    assert!(budget
        .reserve_audited(10, 30, proof)
        .await
        .unwrap()
        .settle_audited(
            &Usage {
                prompt_tokens: 1,
                completion_tokens: 1,
                total_tokens: 2
            },
            1
        )
        .await
        .is_err());
    assert_eq!(budget.snapshot().uncertain_tokens, 40);
    assert!(budget.snapshot().exceeded);
    assert!(budget.reserve_audited(1, 1, proof).await.is_err());
    let recovered = recover(&writer.inner.entries().await).unwrap().unwrap();
    assert_eq!(recovered.scopes[0].uncertain_tokens, 40);
}

#[tokio::test]
async fn root_cannot_rebind_or_bypass_durable_accounting_and_writer_is_not_retained() {
    let (budget, journal) = attached(100).await;
    assert!(budget.reserve(1, 1).is_err());
    assert!(budget
        .attach(
            &(Arc::new(BufferedJournal::new(10)) as Arc<dyn JournalWriter>),
            AgentId::new()
        )
        .await
        .is_err());
    drop(journal);
    assert!(budget.reserve_audited(1, 1, proof).await.is_err());
    assert!(budget.snapshot().exceeded);
    assert!(recover(&[]).unwrap().is_none());
}

#[tokio::test]
async fn timeout_settlement_preserves_live_and_other_scope_reservations() {
    let (root, journal) = attached(1000).await;
    let timed_out = root.child(100).unwrap();
    let sibling = root.child(100).unwrap();
    let abandoned = timed_out.reserve_audited(1, 9, proof).await.unwrap();
    let same_scope_live = timed_out.reserve_audited(1, 9, proof).await.unwrap();
    let sibling_live = sibling.reserve_audited(1, 9, proof).await.unwrap();
    let sibling_abandoned = sibling.reserve_audited(1, 9, proof).await.unwrap();
    drop(abandoned);
    drop(sibling_abandoned);
    timed_out.settle_outstanding().await.unwrap();
    let recovered = recover(&journal.entries().await).unwrap().unwrap();
    assert_eq!(
        recovered
            .reservations
            .iter()
            .filter(|r| r.finish_sequence.is_some())
            .count(),
        1,
        "a timeout must finish only its own abandoned reservation"
    );
    for reservation in [same_scope_live, sibling_live] {
        reservation
            .settle_audited(
                &Usage {
                    prompt_tokens: 1,
                    completion_tokens: 1,
                    total_tokens: 2,
                },
                1,
            )
            .await
            .unwrap();
    }
    let recovered = recover(&journal.entries().await).unwrap().unwrap();
    assert_eq!(
        recovered
            .reservations
            .iter()
            .filter(|r| r.finish_sequence.is_none())
            .count(),
        1
    );
    assert_eq!(recovered.scopes[0].uncertain_tokens, 20);
    assert_eq!(recovered.scopes[0].usage.total_tokens, 4);
    assert_eq!(
        recovered.scopes[0].available_tokens,
        root.snapshot().available_tokens
    );
    let before = journal.entries().await.len();
    timed_out.settle_outstanding().await.unwrap();
    assert_eq!(
        journal.entries().await.len(),
        before,
        "settlement must not duplicate terminal records"
    );
}

#[tokio::test]
async fn timeout_settlement_never_retries_a_cancelled_finish_append() {
    let budget = SharedBudget::new(100);
    let writer = Arc::new(ControlledWriter {
        inner: BufferedJournal::new(100),
        entered: Default::default(),
        block_start: false,
        block_finish: true,
        fail_finish: false,
    });
    budget
        .attach(&(writer.clone() as Arc<dyn JournalWriter>), AgentId::new())
        .await
        .unwrap();
    let reservation = budget.reserve_audited(10, 30, proof).await.unwrap();
    let usage = Usage {
        prompt_tokens: 1,
        completion_tokens: 1,
        total_tokens: 2,
    };
    tokio::select! {
        _ = writer.entered.notified() => {},
        _ = reservation.settle_audited(&usage, 1) => panic!("finish append should remain pending"),
    }
    let before = writer.inner.entries().await.len();
    budget.settle_outstanding().await.unwrap();
    assert_eq!(writer.inner.entries().await.len(), before);
    let recovered = recover(&writer.inner.entries().await).unwrap().unwrap();
    assert_eq!(recovered.reservations.len(), 1);
    assert_eq!(
        recovered.reservations[0]
            .usage
            .as_ref()
            .unwrap()
            .total_tokens,
        2
    );
    // The caller never observed settlement success, so its live allowance stays conservative.
    assert_eq!(budget.snapshot().uncertain_tokens, 40);
}

#[tokio::test]
async fn timeout_settlement_failure_closes_allowance_and_retains_uncertain_intent() {
    let budget = SharedBudget::new(100);
    let writer = Arc::new(ControlledWriter {
        inner: BufferedJournal::new(100),
        entered: Default::default(),
        block_start: false,
        block_finish: false,
        fail_finish: true,
    });
    budget
        .attach(&(writer.clone() as Arc<dyn JournalWriter>), AgentId::new())
        .await
        .unwrap();
    drop(budget.reserve_audited(10, 30, proof).await.unwrap());
    assert!(budget
        .settle_outstanding()
        .await
        .unwrap_err()
        .contains("fixture settlement outage"));
    assert_eq!(budget.snapshot().uncertain_tokens, 40);
    assert!(budget.snapshot().exceeded);
    assert!(budget.reserve_audited(1, 1, proof).await.is_err());
    let recovered = recover(&writer.inner.entries().await).unwrap().unwrap();
    assert!(recovered.reservations[0].finish_sequence.is_none());
    let retained = budget.lock().outstanding[0];
    assert!(retained.abandoned && retained.settlement_started);
}
