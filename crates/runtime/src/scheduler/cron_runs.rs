use super::*;
use crate::{
    reasoning::invocation::ExistingInvocation,
    scheduler::{
        invocations::{recorded_completion, Admission, InvocationIdentity},
        occurrences::{Occurrence, StoredOccurrence},
        task_manager::TaskHandle,
    },
};
use uuid::Uuid;

// Keep manual admission and timer recovery from racing for the same intent.
// The durable invocation claim remains the authority across scheduler processes.
struct OccurrenceAttempt {
    ids: Arc<RwLock<std::collections::HashSet<Uuid>>>,
    id: Uuid,
}
impl Drop for OccurrenceAttempt {
    fn drop(&mut self) {
        self.ids.write().remove(&self.id);
    }
}

struct CronAdmissionGate {
    owner: CronScheduler,
    occurrence: Occurrence,
    reservation: std::sync::Mutex<Option<RunReservation>>,
    refusal: std::sync::Mutex<Option<CronSchedulerError>>,
}
impl CronAdmissionGate {
    /// Preserve the specific refusal before releasing the invocation claim.
    /// Refused identities remain closed; operator reconciliation cannot replay them.
    async fn record_refusal(&self, reason: &str) {
        if let Err(error) = self
            .owner
            .store
            .note_run_refusal(self.occurrence.identity.id, reason)
            .await
        {
            tracing::error!(%error, "refused cron run could not record its reason");
        }
    }

    async fn check_inner(
        &self,
        audit: &crate::reasoning::run_audit::RunAuditReference,
    ) -> Result<(), CronSchedulerError> {
        let mut authority = self.occurrence.job.clone();
        let current = self
            .owner
            .store
            .get_job(authority.job_id)
            .await?
            .ok_or(CronSchedulerError::NotFound(authority.job_id))?;
        authority.failure_count = current.failure_count;
        authority.run_count = current.run_count;
        if self
            .owner
            .store
            .job_has_unresolved_occurrence(authority.job_id)
            .await?
        {
            return Err(CronSchedulerError::Scheduler(
                "this job has an unresolved occurrence; reconcile it before further execution"
                    .into(),
            ));
        }
        if self.occurrence.scheduled_for.is_some()
            && (current.status != CronJobStatus::Active
                || (!current.enabled && !self.occurrence.job.one_shot))
        {
            return Err(CronSchedulerError::Scheduler(
                "timer intent cannot start while its job is paused or terminal".into(),
            ));
        }
        if let Err(error) = self.owner.verify_job_credential(&authority).await {
            self.owner.metrics.write().runs_skipped_identity += 1;
            self.record_refusal(&error.to_string()).await;
            return Err(error);
        }
        let gate = self.owner.policy_gate.read().clone();
        let decision = CronScheduler::evaluate_schedule_policy(gate.as_deref(), &authority);
        if !matches!(decision, SchedulePolicyDecision::Allow) {
            self.owner.metrics.write().runs_skipped_policy += 1;
            let refusal = CronSchedulerError::PolicyDenied(
                authority.job_id,
                CronScheduler::describe_policy_refusal(&decision),
            );
            self.record_refusal(&refusal.to_string()).await;
            return Err(refusal);
        }
        let reservation = self.owner.reserve(&authority).ok_or_else(|| {
            CronSchedulerError::Scheduler(
                "cron scheduler is stopped or its concurrency limit is reached".into(),
            )
        })?;
        *self.reservation.lock().unwrap_or_else(|p| p.into_inner()) = Some(reservation);
        self.owner
            .store
            .occurrence_running(&self.occurrence, audit)
            .await?;
        Ok(())
    }
}
#[async_trait::async_trait]
impl crate::scheduler::invocations::InvocationAdmissionGate for CronAdmissionGate {
    async fn check(
        &self,
        audit: &crate::reasoning::run_audit::RunAuditReference,
    ) -> Result<(), String> {
        match self.check_inner(audit).await {
            Ok(()) => Ok(()),
            Err(error) => {
                let message = error.to_string();
                *self.refusal.lock().unwrap_or_else(|p| p.into_inner()) = Some(error);
                Err(message)
            }
        }
    }
}

impl CronScheduler {
    /// The UUID and caller context come from a trusted, authenticated adapter.
    pub async fn trigger_identified(
        &self,
        job_id: CronJobId,
        identity: InvocationIdentity,
    ) -> Result<Admission, CronSchedulerError> {
        let job = self
            .store
            .get_job(job_id)
            .await?
            .ok_or(CronSchedulerError::NotFound(job_id))?;
        // Register ownership before publishing the intent to the recovery scan.
        let attempt = self.begin_occurrence_attempt(identity.id);
        let occurrence = Occurrence::manual(job, identity);
        let stored = self
            .store
            .prepare_occurrence(&occurrence, None, self.config.max_concurrent_cron_jobs)
            .await?
            .ok_or_else(|| {
                CronSchedulerError::Scheduler("cron occurrence capacity exhausted".into())
            })?;
        let Some(_attempt) = attempt else {
            // prepare_occurrence above still checks the request fingerprint.
            return Ok(Admission::Existing(ExistingInvocation::InProgress));
        };
        self.admit_occurrence(stored).await
    }

    fn begin_occurrence_attempt(&self, id: Uuid) -> Option<OccurrenceAttempt> {
        self.pending_attempts
            .write()
            .insert(id)
            .then(|| OccurrenceAttempt {
                ids: self.pending_attempts.clone(),
                id,
            })
    }

    async fn admit_occurrence(
        &self,
        stored: StoredOccurrence,
    ) -> Result<Admission, CronSchedulerError> {
        let occurrence = stored.occurrence;
        let (config, input, identity) = occurrence.admission();
        let existing = match self
            .agent_scheduler
            .lookup_identified_invocation(&config, &input, &identity)
            .await
        {
            Ok(existing) => existing,
            Err(error) => {
                self.finish_unknown(
                    &occurrence,
                    None,
                    &format!(
                        "protected execution lookup failed; inspect retained evidence: {error}"
                    ),
                )
                .await?;
                return Err(CronSchedulerError::Scheduler(error));
            }
        };
        if let Some(existing) = existing {
            self.reconcile_existing(&occurrence, &existing).await?;
            return Ok(Admission::Existing(existing));
        }
        if stored.state != "prepared" {
            self.finish_unknown(
                &occurrence,
                None,
                "execution claim is missing; no replay permitted",
            )
            .await?;
            return Ok(Admission::Existing(ExistingInvocation::Unresolved {
                audit: None,
            }));
        }
        if self.stopping.is_cancelled() {
            return Err(CronSchedulerError::Scheduler(
                "cron scheduler is stopping".into(),
            ));
        }
        let gate = Arc::new(CronAdmissionGate {
            owner: self.clone(),
            occurrence: occurrence.clone(),
            reservation: std::sync::Mutex::new(None),
            refusal: std::sync::Mutex::new(None),
        });
        let admitted = self
            .agent_scheduler
            .schedule_identified_with_gate(config, input.clone(), identity.clone(), gate.clone())
            .await;
        let admission = match admitted {
            Ok(admission) => admission,
            Err(error) => {
                let refusal = gate
                    .refusal
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .take();
                // Record why the run was refused, not merely that it was. The
                // gate holds the specific reason -- a policy denial, a rejected
                // credential -- while `error` is the scheduler's generic string,
                // so persisting the latter loses the only useful detail.
                let recorded = refusal
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| error.clone());
                let (config, _, _) = occurrence.admission();
                let existing = self
                    .agent_scheduler
                    .lookup_identified_invocation(&config, &input, &identity)
                    .await
                    .map_err(CronSchedulerError::Scheduler)?;
                let audit = match existing {
                    Some(ExistingInvocation::Unresolved { audit }) => audit,
                    _ => None,
                };
                // finish_occurrence is idempotent and reports whether it applied,
                // so this is safe whatever the lookup found. Skipping it left a
                // refused run with no reason recorded at all.
                self.finish_unknown(&occurrence, audit.as_ref(), &recorded)
                    .await?;
                return Err(refusal.unwrap_or(CronSchedulerError::Scheduler(error)));
            }
        };
        let Admission::Queued { handle, audit } = admission else {
            if let Admission::Existing(existing) = &admission {
                self.reconcile_existing(&occurrence, existing).await?;
            }
            return Ok(admission);
        };
        let reservation = gate
            .reservation
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        let Some(reservation) = reservation else {
            handle.cancel();
            let _ = handle.wait().await;
            return Err(CronSchedulerError::Scheduler(
                "execution service bypassed its cron admission gate".into(),
            ));
        };
        let public = TaskHandle::with_run_id(handle.agent_id(), handle.run_id());
        let returned = public.clone();
        let saved_audit = audit.clone();
        let owner = self.clone();
        tokio::spawn(async move {
            struct CancelOnDrop(TaskHandle);
            impl Drop for CancelOnDrop {
                fn drop(&mut self) {
                    self.0.cancel();
                }
            }
            let guard = CancelOnDrop(handle);
            let _reservation = reservation;
            let mut result = tokio::select! {
                biased;
                _=owner.stopping.cancelled()=>{guard.0.cancel();guard.0.wait().await},
                _=public.cancelled()=>{guard.0.cancel();guard.0.wait().await},
                result=guard.0.wait()=>result,
            };
            match owner
                .store
                .finish_occurrence(&occurrence, Some(&result), Some(&saved_audit), None, false)
                .await
            {
                Ok(true) => owner.record_occurrence_metrics(&result),
                Ok(false) => {}
                Err(error) => {
                    result.status = TaskStatus::Unresolved;
                    result.output = None;
                    result.error =
                        Some(format!("cron bookkeeping requires reconciliation: {error}"));
                }
            }
            public.finish(result);
        });
        Ok(Admission::Queued {
            handle: returned,
            audit,
        })
    }

    async fn reconcile_existing(
        &self,
        occurrence: &Occurrence,
        existing: &ExistingInvocation,
    ) -> Result<(), CronSchedulerError> {
        match existing {
            ExistingInvocation::InProgress => {}
            ExistingInvocation::Unresolved { audit } => {
                self.finish_unknown(
                    occurrence,
                    audit.as_ref(),
                    "original execution requires reconciliation; automatic replay refused",
                )
                .await?;
            }
            ExistingInvocation::Reconciled { .. } => {
                self.store
                    .reconcile_occurrence(
                        self.agent_scheduler
                            .invocation_project()
                            .map_err(CronSchedulerError::Scheduler)?,
                        occurrence.identity.id,
                    )
                    .await?;
            }
            ExistingInvocation::Recorded { audit, result } => {
                let completion = recorded_completion(audit, result.clone())
                    .map_err(CronSchedulerError::Scheduler)?;
                if self
                    .store
                    .finish_occurrence(occurrence, Some(&completion), Some(audit), None, false)
                    .await?
                {
                    self.record_occurrence_metrics(&completion);
                }
            }
        }
        Ok(())
    }

    async fn finish_unknown(
        &self,
        occurrence: &Occurrence,
        audit: Option<&crate::reasoning::run_audit::RunAuditReference>,
        error: &str,
    ) -> Result<(), CronSchedulerError> {
        if self
            .store
            .finish_occurrence(occurrence, None, audit, Some(error), true)
            .await?
        {
            let mut metrics = self.metrics.write();
            metrics.runs_total += 1;
            metrics.runs_failed += 1;
        }
        Ok(())
    }

    fn record_occurrence_metrics(&self, result: &TaskCompletion) {
        let mut metrics = self.metrics.write();
        metrics.runs_total += 1;
        if result.status == TaskStatus::Completed {
            metrics.runs_succeeded += 1;
        } else {
            metrics.runs_failed += 1;
        }
        let ms = u64::try_from(result.duration.as_millis()).unwrap_or(u64::MAX);
        metrics.longest_run_ms = metrics.longest_run_ms.max(ms);
        metrics.average_execution_time_ms +=
            (ms as f64 - metrics.average_execution_time_ms) / metrics.runs_total as f64;
    }

    pub(super) fn start_occurrence_loop(&self) {
        let owner = self.clone();
        tokio::spawn(async move {
            let mut ticker = interval(owner.config.tick_interval);
            let mut recovery_after = None;
            loop {
                tokio::select! {biased;_=owner.stopping.cancelled()=>break,_=ticker.tick()=>{}}
                match owner.store.pending_occurrences_after(recovery_after).await {
                    Ok(pending) => {
                        recovery_after = if pending.len() == 32 {
                            pending.last().map(|p| p.occurrence.identity.id)
                        } else {
                            None
                        };
                        for occurrence in pending {
                            owner.spawn_occurrence_attempt(occurrence);
                        }
                    }
                    Err(error) => {
                        tracing::error!("cron recovery query failed: {error}");
                        continue;
                    }
                }
                let now = Utc::now();
                let due = match owner.store.get_due_jobs(now).await {
                    Ok(due) => due,
                    Err(error) => {
                        tracing::error!("cron query failed: {error}");
                        continue;
                    }
                };
                for job in due {
                    let occurrence = match Occurrence::timer(job) {
                        Ok(occurrence) => occurrence,
                        Err(error) => {
                            tracing::error!("invalid timer identity: {error}");
                            continue;
                        }
                    };
                    let next = compute_next_run_static(
                        &occurrence.job.cron_expression,
                        &occurrence.job.timezone,
                        Some(now),
                    );
                    match owner
                        .store
                        .prepare_occurrence(
                            &occurrence,
                            next,
                            owner.config.max_concurrent_cron_jobs,
                        )
                        .await
                    {
                        Ok(Some(stored)) => owner.spawn_occurrence_attempt(stored),
                        Ok(None) => {}
                        Err(error) => {
                            tracing::error!("cron intent could not be persisted: {error}")
                        }
                    }
                }
            }
        });
    }

    fn spawn_occurrence_attempt(&self, stored: StoredOccurrence) {
        let id = stored.occurrence.identity.id;
        let Some(attempt) = self.begin_occurrence_attempt(id) else {
            return;
        };
        let owner = self.clone();
        tokio::spawn(async move {
            let _attempt = attempt;
            if stored.state == "prepared"
                && stored.occurrence.scheduled_for.is_some()
                && stored.occurrence.job.jitter_max_secs > 0
            {
                // Stable jitter preserves the original start window on restart.
                let bound = u64::from(stored.occurrence.job.jitter_max_secs) * 1000 + 1;
                let delay =
                    u64::from_le_bytes(id.as_bytes()[..8].try_into().expect("UUID prefix")) % bound;
                let elapsed = (Utc::now() - stored.occurrence.created_at)
                    .num_milliseconds()
                    .max(0) as u64;
                tokio::select! {biased;_=owner.stopping.cancelled()=>return,_=tokio::time::sleep(Duration::from_millis(delay.saturating_sub(elapsed)))=>{}}
            }
            if let Err(error) = owner.admit_occurrence(stored).await {
                tracing::warn!("cron occurrence {id} was not admitted: {error}");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn manual_admission_owns_intent_before_recovery_can_claim_it() {
        use super::super::tests::{deny_all_gate, make_scheduler, test_agent_config};
        let (cron, scheduler) = make_scheduler().await;
        let cron = cron.with_policy_gate(Arc::new(deny_all_gate()));
        let job = CronJobDefinition::new(
            "manual-recovery-race".into(),
            "0 0 0 1 1 * 2099".into(),
            "UTC".into(),
            test_agent_config(),
        );
        let agent_id = job.agent_config.id;
        let id = cron.add_job(job).await.unwrap();
        let identity = InvocationIdentity {
            id: Uuid::new_v4(),
            context: serde_json::json!({"caller":"fixture"}),
        };
        let mut admission = Box::pin(cron.trigger_identified(id, identity.clone()));
        // First poll publishes the SQLite intent, then suspends for the durable
        // claim lookup. Reproduce recovery at precisely that pre-claim boundary.
        assert!(futures::poll!(admission.as_mut()).is_pending());
        assert!(cron.pending_attempts.read().contains(&identity.id));
        let pending = cron.store.pending_occurrences_after(None).await.unwrap();
        assert_eq!(pending.len(), 1);
        cron.spawn_occurrence_attempt(pending.into_iter().next().unwrap());
        assert!(matches!(
            admission.await,
            Err(CronSchedulerError::PolicyDenied(_, _))
        ));
        assert!(!cron.pending_attempts.read().contains(&identity.id));
        let history = cron.get_run_history(id, 10).await.unwrap();
        assert_eq!(history.len(), 1);
        assert!(history[0].error.as_deref().unwrap().contains("policy"));
        assert_eq!(cron.metrics().runs_skipped_policy, 1);
        assert!(!scheduler.has_agent(agent_id));
        cron.shutdown().await;
        scheduler.shutdown().await.unwrap();
    }
}
