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

struct CronAdmissionGate {
    owner: CronScheduler,
    occurrence: Occurrence,
    reservation: std::sync::Mutex<Option<RunReservation>>,
    refusal: std::sync::Mutex<Option<CronSchedulerError>>,
}
impl CronAdmissionGate {
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
            return Err(error);
        }
        let gate = self.owner.policy_gate.read().clone();
        let decision = CronScheduler::evaluate_schedule_policy(gate.as_deref(), &authority);
        if !matches!(decision, SchedulePolicyDecision::Allow) {
            self.owner.metrics.write().runs_skipped_policy += 1;
            return Err(CronSchedulerError::PolicyDenied(
                authority.job_id,
                CronScheduler::describe_policy_refusal(&decision),
            ));
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
        let occurrence = Occurrence::manual(job, identity);
        let stored = self
            .store
            .prepare_occurrence(&occurrence, None, self.config.max_concurrent_cron_jobs)
            .await?
            .ok_or_else(|| {
                CronSchedulerError::Scheduler("cron occurrence capacity exhausted".into())
            })?;
        self.admit_occurrence(stored).await
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
                let (config, _, _) = occurrence.admission();
                let existing = self
                    .agent_scheduler
                    .lookup_identified_invocation(&config, &input, &identity)
                    .await
                    .map_err(CronSchedulerError::Scheduler)?;
                if let Some(ExistingInvocation::Unresolved { audit }) = existing {
                    self.finish_unknown(&occurrence, audit.as_ref(), &error)
                        .await?;
                }
                return Err(gate
                    .refusal
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .take()
                    .unwrap_or(CronSchedulerError::Scheduler(error)));
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
        if !self.pending_attempts.write().insert(id) {
            return;
        }
        let owner = self.clone();
        tokio::spawn(async move {
            struct Attempt {
                ids: Arc<RwLock<std::collections::HashSet<Uuid>>>,
                id: Uuid,
            }
            impl Drop for Attempt {
                fn drop(&mut self) {
                    self.ids.write().remove(&self.id);
                }
            }
            let _attempt = Attempt {
                ids: owner.pending_attempts.clone(),
                id,
            };
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
