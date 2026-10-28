use super::{Credential, FrozenCredential};
use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

pub(super) const MAX_RUNS: usize = 1024;

struct Usage {
    closed: bool,
    calls: HashMap<String, u32>,
    /// Credentials resolved for this lease only, keyed by alias. Cleared on
    /// revocation and dropped with the lease; never shared across runs.
    credentials: HashMap<String, Arc<Credential>>,
}

/// One runtime-issued assignment. This is an in-memory authority, never a
/// worker-supplied token. Replacing an executor or session issues a new epoch.
pub(crate) struct Lease {
    pub epoch: uuid::Uuid,
    binding: String,
    deadline: Instant,
    usage: Mutex<Usage>,
    credential_resolution: tokio::sync::Mutex<()>,
    stop: CancellationToken,
}

impl Lease {
    pub fn new(binding: String, deadline: Instant) -> Arc<Self> {
        Self::with_epoch(binding, deadline, None)
    }

    /// Adopt an assignment epoch the runtime already issued for this run, so
    /// a governed session and its executor dispatches share one assignment.
    pub fn with_epoch(binding: String, deadline: Instant, epoch: Option<uuid::Uuid>) -> Arc<Self> {
        Arc::new(Self {
            epoch: epoch.unwrap_or_else(uuid::Uuid::new_v4),
            binding,
            deadline,
            usage: Mutex::new(Usage {
                closed: false,
                calls: HashMap::new(),
                credentials: HashMap::new(),
            }),
            stop: CancellationToken::new(),
            credential_resolution: tokio::sync::Mutex::new(()),
        })
    }

    pub fn check_live(&self) -> Result<(), String> {
        if self.stop.is_cancelled() || Instant::now() >= self.deadline {
            return Err("outbound execution authority is closed or expired".into());
        }
        Ok(())
    }

    pub fn revoke(&self) {
        // Serialize revocation against the last admission check. A request
        // admitted before this point may already have remote effects.
        if let Ok(mut usage) = self.usage.lock() {
            usage.closed = true;
            usage.credentials.clear();
        }
        self.stop.cancel();
    }

    pub fn admit_send(&self, capability: &str, max_calls: u32) -> Result<(), String> {
        let mut usage = self
            .usage
            .lock()
            .map_err(|_| "outbound authority is unavailable")?;
        self.check_live()?;
        if usage.closed {
            return Err("outbound execution authority is closed".into());
        }
        if !usage.calls.contains_key(capability) && usage.calls.len() >= 128 {
            return Err("outbound capability count exceeded".into());
        }
        let calls = usage.calls.entry(capability.to_owned()).or_default();
        if *calls >= max_calls {
            return Err("outbound request allowance exhausted".into());
        }
        *calls += 1; // Uncertain attempts retain their charge.
        Ok(())
    }

    /// Cache each successfully resolved credential once per lease, including
    /// concurrent SDK calls. Failed resolutions remain retryable.
    pub async fn credential(
        &self,
        alias: &str,
        source: &FrozenCredential,
        principal: &str,
    ) -> Result<Arc<Credential>, String> {
        // This async lock serializes resolution, never revocation or request
        // admission. Queued callers recheck the cache and authority below.
        let _resolution = self
            .during(async { Ok(self.credential_resolution.lock().await) })
            .await?;
        {
            let usage = self
                .usage
                .lock()
                .map_err(|_| "outbound authority is unavailable")?;
            self.check_live()?;
            if usage.closed {
                return Err("outbound execution authority is closed".into());
            }
            if let Some(credential) = usage.credentials.get(alias) {
                return Ok(credential.clone());
            }
        }
        let credential = Arc::new(self.during(source.resolve(principal)).await?);
        let mut usage = self
            .usage
            .lock()
            .map_err(|_| "outbound authority is unavailable")?;
        self.check_live()?;
        if usage.closed {
            return Err("outbound execution authority is closed".into());
        }
        Ok(usage
            .credentials
            .entry(alias.to_owned())
            .or_insert(credential)
            .clone())
    }

    pub async fn during<T>(
        &self,
        future: impl Future<Output = Result<T, String>>,
    ) -> Result<T, String> {
        self.check_live()?;
        tokio::select! {
            biased;
            _ = self.stop.cancelled() => Err("outbound execution cancelled; completion may be unconfirmed".into()),
            _ = tokio::time::sleep_until(self.deadline.into()) => Err("outbound execution expired; completion may be unconfirmed".into()),
            result = future => result,
        }
    }
}

/// Upper bound on remembered closed runs. Entries expire at their guard
/// deadline; while the set is full of unexpired entries, new runs are refused
/// until the overflowing run's deadline passes, so no closed run can acquire.
pub(super) const MAX_CLOSED: usize = 16 * 1024;
/// A closed run's guard outlives the deadline supplied at cancellation by this
/// margin. Session and grant deadlines both derive from the same run timeout
/// but are computed at slightly different instants.
pub(super) const CLOSED_GRACE: Duration = Duration::from_secs(60);

#[derive(Default)]
struct State {
    runs: HashMap<String, Arc<Lease>>,
    /// Closed runs and the instant until which they may not acquire.
    closed: HashMap<String, Instant>,
    /// Set when the closed-run set overflowed: new runs are refused until then.
    refuse_new_until: Option<Instant>,
}

impl State {
    /// Drop expired leases and guards. The closed-run set is swept when it
    /// reaches its bound, so routine calls stay cheap.
    fn prune(&mut self, now: Instant) {
        self.runs.retain(|_, lease| lease.deadline > now);
        if self.closed.len() >= MAX_CLOSED {
            self.closed.retain(|_, until| *until > now);
        }
        if self.refuse_new_until.is_some_and(|until| until <= now) {
            self.refuse_new_until = None;
        }
    }

    fn is_closed(&self, run: &str, now: Instant) -> bool {
        self.closed.get(run).is_some_and(|until| *until > now)
    }

    fn close(&mut self, run: &str, until: Instant, now: Instant) {
        if until <= now {
            return;
        }
        if let Some(existing) = self.closed.get_mut(run) {
            *existing = (*existing).max(until);
            return;
        }
        if self.closed.len() >= MAX_CLOSED {
            self.closed.retain(|_, until| *until > now);
        }
        if self.closed.len() >= MAX_CLOSED {
            // Forgetting this run could let a late grant recreate authority.
            // Refuse every new run until this one's guard would have expired.
            self.refuse_new_until = Some(self.refuse_new_until.map_or(until, |u| u.max(until)));
            return;
        }
        self.closed.insert(run.to_owned(), until);
    }
}

/// Runs that held a lease keep a closed-run guard until the later of the
/// cancellation deadline and the lease's own run deadline. Runs that never
/// acquired a lease are remembered only in the bounded closed-run set, so
/// closing them never consumes lease capacity. Capacity exhaustion refuses
/// only new acquisitions and never disables the registry.
#[derive(Default)]
pub(crate) struct Registry(Mutex<State>);

impl Registry {
    pub fn acquire(
        &self,
        run: &str,
        binding: &str,
        deadline: Instant,
        epoch: Option<uuid::Uuid>,
    ) -> Result<Arc<Lease>, String> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| "outbound registry is unavailable")?;
        let now = Instant::now();
        if deadline <= now {
            return Err("outbound grant expired".into());
        }
        state.prune(now);
        if state.is_closed(run, now) {
            // A grant can outlive the deadline supplied at cancellation.
            state.close(run, deadline, now);
            return Err("outbound execution authority is closed".into());
        }
        if let Some(lease) = state.runs.get(run) {
            lease.check_live()?;
            if lease.binding != binding || epoch.is_some_and(|epoch| epoch != lease.epoch) {
                lease.revoke();
                return Err("outbound execution context changed".into());
            }
            return Ok(lease.clone());
        }
        if state.refuse_new_until.is_some() || state.runs.len() >= MAX_RUNS {
            return Err("outbound run capacity exhausted".into());
        }
        let lease = Lease::with_epoch(binding.to_owned(), deadline, epoch);
        state.runs.insert(run.to_owned(), lease.clone());
        Ok(lease)
    }

    pub fn cancel(&self, run: &str, deadline: Instant) {
        if let Ok(mut state) = self.0.lock() {
            let now = Instant::now();
            state.prune(now);
            let mut until = deadline.checked_add(CLOSED_GRACE).unwrap_or(deadline);
            if let Some(lease) = state.runs.remove(run) {
                lease.revoke();
                until = until.max(lease.deadline);
            }
            state.close(run, until, now);
        }
    }

    #[cfg(test)]
    pub(super) fn counts(&self) -> (usize, usize) {
        let state = self.0.lock().unwrap();
        (state.runs.len(), state.closed.len())
    }

    #[cfg(test)]
    pub(super) fn closed_until(&self, run: &str) -> Option<Instant> {
        self.0.lock().unwrap().closed.get(run).copied()
    }
}

impl Drop for Registry {
    fn drop(&mut self) {
        if let Ok(state) = self.0.lock() {
            for lease in state.runs.values() {
                lease.revoke();
            }
        }
    }
}
