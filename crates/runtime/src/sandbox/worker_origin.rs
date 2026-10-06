//! Dispatch attribution follows only explicitly inherited runtime tasks.
//! It carries no policy authority and is never accepted from configuration.
use std::future::Future;
pub(crate) use symbi_sandbox_supervisor::origin::WorkerOrigin;

tokio::task_local! { static CURRENT: Option<WorkerOrigin>; }

pub(crate) fn current() -> Option<WorkerOrigin> {
    CURRENT.try_with(Clone::clone).ok().flatten()
}

pub(crate) fn scope<T>(
    origin: Option<WorkerOrigin>,
    future: impl Future<Output = T>,
) -> impl Future<Output = T> {
    CURRENT.scope(origin, future)
}

/// Capture before spawning; an async function would capture too late.
pub(crate) fn inherit<T>(future: impl Future<Output = T>) -> impl Future<Output = T> {
    scope(current(), future)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn origin(name: &str) -> WorkerOrigin {
        WorkerOrigin {
            agent_id: uuid::Uuid::new_v4(),
            run_id: uuid::Uuid::new_v4(),
            public_key: "a".repeat(64),
            dispatch_id: uuid::Uuid::new_v4(),
            call_fingerprint: format!("sha256:{}", "b".repeat(64)),
            tool_name: name.into(),
            iteration: 1,
        }
    }
    #[tokio::test]
    async fn concurrent_and_detached_tasks_keep_exact_origins_without_ambient_leakage() {
        let a = origin("a");
        let b = origin("b");
        let a_task = tokio::spawn(scope(Some(a.clone()), async {
            assert!(tokio::spawn(async { current() }).await.unwrap().is_none());
            scope(None, async {
                assert!(current().is_none());
            })
            .await;
            let detached = tokio::spawn(inherit(async {
                tokio::task::yield_now().await;
                current()
            }));
            detached.await.unwrap()
        }));
        let b_task = tokio::spawn(scope(Some(b.clone()), async {
            tokio::task::yield_now().await;
            current()
        }));
        assert_eq!(a_task.await.unwrap(), Some(a));
        assert_eq!(b_task.await.unwrap(), Some(b));
        assert!(current().is_none());
    }
}
