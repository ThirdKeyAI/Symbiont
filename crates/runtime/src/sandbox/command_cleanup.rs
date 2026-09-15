//! Run-scoped acknowledgements for detached command owners.
//!
//! Registration happens before spawning a worker. A dropped dispatch future
//! cancels its worker, while the run retains the acknowledgement until removal.
use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

type Completion = Option<Result<(), String>>;
const MAX_RUNS: usize = 1024;
const MAX_OWNERS: usize = 256;

tokio::task_local! { static ACTIVE: Arc<Run>; }

#[derive(Default)]
struct State {
    closed: bool,
    owners: Vec<watch::Receiver<Completion>>,
}
#[derive(Default)]
pub(crate) struct Run {
    state: Mutex<State>,
    stop: CancellationToken,
}
impl Run {
    pub async fn scope<T>(self: Arc<Self>, future: impl Future<Output = T>) -> T {
        ACTIVE.scope(self, future).await
    }
    fn cancel(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
        }
        self.stop.cancel();
    }
    async fn close(&self) -> Result<(), String> {
        self.cancel();
        let owners = self
            .state
            .lock()
            .map_err(|_| "command cleanup registry is poisoned")?
            .owners
            .clone();
        let mut failures = Vec::new();
        for mut owner in owners {
            loop {
                if let Some(result) = owner.borrow().clone() {
                    if let Err(error) = result {
                        failures.push(error);
                    }
                    break;
                }
                if owner.changed().await.is_err() {
                    failures.push("command owner ended without a cleanup acknowledgement".into());
                    break;
                }
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("; "))
        }
    }
    fn settled(&self) -> bool {
        self.state.lock().is_ok_and(|state| {
            state
                .owners
                .iter()
                .all(|owner| matches!(&*owner.borrow(), Some(Ok(()))))
        })
    }
}

struct Slot {
    deadline: Instant,
    run: Arc<Run>,
}
#[derive(Default)]
pub(crate) struct Registry(Mutex<HashMap<String, Slot>>);
impl Registry {
    pub fn admit(&self, key: &str, deadline: Instant) -> Result<Arc<Run>, String> {
        let now = Instant::now();
        if deadline <= now {
            return Err("command run authorization expired".into());
        }
        let mut slots = self
            .0
            .lock()
            .map_err(|_| "command run registry is poisoned")?;
        slots.retain(|_, slot| slot.deadline > now || !slot.run.settled());
        if let Some(slot) = slots.get(key) {
            if slot.run.stop.is_cancelled() {
                return Err("command execution run is closed".into());
            }
            return Ok(slot.run.clone());
        }
        if slots.len() >= MAX_RUNS {
            return Err("command run capacity exhausted".into());
        }
        let run = Arc::new(Run::default());
        slots.insert(
            key.to_owned(),
            Slot {
                deadline,
                run: run.clone(),
            },
        );
        Ok(run)
    }
    pub fn cancel(&self, key: &str) {
        if let Ok(slots) = self.0.lock() {
            if let Some(slot) = slots.get(key) {
                slot.run.cancel();
            }
        }
    }
    pub async fn close(&self, key: &str) -> Result<(), String> {
        let run = self
            .0
            .lock()
            .map_err(|_| "command run registry is poisoned")?
            .get(key)
            .map(|slot| slot.run.clone());
        if let Some(run) = run {
            run.close().await?;
        }
        Ok(())
    }
}
impl Drop for Registry {
    fn drop(&mut self) {
        if let Ok(slots) = self.0.lock() {
            for slot in slots.values() {
                slot.run.cancel();
            }
        }
    }
}

pub(super) struct Registration {
    done: watch::Sender<Completion>,
    pub stop: CancellationToken,
}
impl Registration {
    pub fn finish(&self, result: Result<(), String>) {
        self.done.send_replace(Some(result));
    }
}
impl Drop for Registration {
    fn drop(&mut self) {
        if self.done.borrow().is_none() {
            self.done.send_replace(Some(Err(
                "command owner ended before confirming worker removal".into(),
            )));
        }
    }
}

pub(super) fn register() -> Result<Option<Registration>, String> {
    ACTIVE
        .try_with(|run| {
            let mut state = run
                .state
                .lock()
                .map_err(|_| "command cleanup registry is poisoned")?;
            if state.closed {
                return Err("command execution run is closed".into());
            }
            state
                .owners
                .retain(|owner| !matches!(&*owner.borrow(), Some(Ok(()))));
            if state.owners.len() >= MAX_OWNERS {
                return Err("command cleanup capacity exhausted".into());
            }
            let (done, completion) = watch::channel(None);
            state.owners.push(completion);
            Ok(Some(Registration {
                done,
                stop: run.stop.clone(),
            }))
        })
        .unwrap_or(Ok(None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn closed_runs_reject_admission_and_wait_for_owner_acknowledgement() {
        let registry = Arc::new(Registry::default());
        let deadline = Instant::now() + Duration::from_secs(30);
        let run = registry.admit("fixture", deadline).unwrap();
        let owner = run
            .clone()
            .scope(async { register().unwrap().unwrap() })
            .await;
        registry.cancel("fixture");
        assert!(owner.stop.is_cancelled());
        assert!(registry.admit("fixture", deadline).is_err());
        assert!(run.scope(async { register().is_err() }).await);
        assert!(
            tokio::time::timeout(Duration::from_millis(5), registry.close("fixture"))
                .await
                .is_err()
        );
        owner.finish(Ok(()));
        registry.close("fixture").await.unwrap();
    }

    #[tokio::test]
    async fn lost_or_failed_cleanup_is_retained_as_failure() {
        for lost in [false, true] {
            let registry = Registry::default();
            let run = registry
                .admit("fixture", Instant::now() + Duration::from_secs(1))
                .unwrap();
            let owner = run.scope(async { register().unwrap().unwrap() }).await;
            if !lost {
                owner.finish(Err("fixture cleanup failure".into()));
            }
            drop(owner);
            assert!(registry.close("fixture").await.is_err());
            assert!(registry.close("fixture").await.is_err());
        }
    }
}
