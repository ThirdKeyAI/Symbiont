//! Transport-independent worker byte streams and lifetime ownership.
use super::docker::{StdioContainer, StdioContainerGuard};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};

pub(crate) type Reader = Box<dyn AsyncRead + Unpin + Send>;
pub(crate) type Writer = Box<dyn AsyncWrite + Unpin + Send>;

pub(crate) struct StdioStreams {
    pub stdin: Writer,
    pub stdout: Reader,
    pub stderr: Reader,
    pub guard: StreamGuard,
}
impl StdioStreams {
    #[cfg(feature = "mcp-client")]
    pub fn development(child: tokio::process::Child, output_limit: usize) -> Self {
        Self::process(child, None, output_limit)
    }
    /// A Landlock child retains its independent delegated-cgroup owner.
    #[cfg(target_os = "linux")]
    pub fn host(
        child: tokio::process::Child,
        lease: super::supervisor::Lease,
        output_limit: usize,
    ) -> Self {
        let mut streams = Self::process(child, None, output_limit);
        streams.guard.group_live = false;
        streams.guard.host = Some(lease);
        streams
    }
    pub fn container(worker: StdioContainer) -> Self {
        let output_limit = worker.guard.output_limit;
        Self::process(worker.child, Some(worker.guard), output_limit)
    }
    fn process(
        mut child: tokio::process::Child,
        container: Option<StdioContainerGuard>,
        output_limit: usize,
    ) -> Self {
        Self {
            stdin: Box::new(child.stdin.take().expect("worker stdin piped")),
            stdout: Box::new(child.stdout.take().expect("worker stdout piped")),
            stderr: Box::new(child.stderr.take().expect("worker stderr piped")),
            guard: StreamGuard {
                child: Some(child),
                group_live: container.is_none(),
                container,
                #[cfg(target_os = "linux")]
                host: None,
                #[cfg(unix)]
                vm: None,
                output_limit,
            },
        }
    }
    #[cfg(unix)]
    pub fn vm(worker: super::firecracker::FirecrackerStdio) -> Self {
        let output_limit = worker.guard.output_limit;
        Self {
            stdin: Box::new(worker.stdin),
            stdout: Box::new(worker.stdout),
            stderr: Box::new(worker.stderr),
            guard: StreamGuard {
                child: None,
                group_live: false,
                container: None,
                #[cfg(target_os = "linux")]
                host: None,
                vm: Some(worker.guard),
                output_limit,
            },
        }
    }
}

pub(crate) struct StreamGuard {
    child: Option<tokio::process::Child>,
    group_live: bool,
    container: Option<StdioContainerGuard>,
    #[cfg(target_os = "linux")]
    host: Option<super::supervisor::Lease>,
    #[cfg(unix)]
    vm: Option<super::firecracker::StdioGuard>,
    pub output_limit: usize,
}
impl StreamGuard {
    fn stop(&mut self) {
        if self.group_live {
            #[cfg(unix)]
            if let Some(pid) = self.child.as_ref().and_then(|child| child.id()) {
                // SAFETY: this unreaped child owns the dedicated process group.
                unsafe {
                    libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
                }
            }
            self.group_live = false;
        }
        if let Some(child) = &mut self.child {
            let _ = child.start_kill();
        }
    }
    #[cfg(feature = "toolclad-session")]
    pub async fn finish_cleanup(&mut self) -> Result<(), String> {
        #[cfg(unix)]
        if let Some(vm) = &mut self.vm {
            return vm.finish_cleanup().await.map_err(|e| e.to_string());
        }
        self.finish().await
    }
    pub async fn finish(&mut self) -> Result<(), String> {
        self.stop();
        #[cfg(unix)]
        if let Some(vm) = &mut self.vm {
            return vm.finish().await.map_err(|e| e.to_string());
        }
        #[cfg(target_os = "linux")]
        if let Some(host) = &mut self.host {
            let cleanup = host.finish().await.map_err(|e| e.to_string());
            if let Some(child) = &mut self.child {
                tokio::time::timeout(Duration::from_secs(3), child.wait())
                    .await
                    .map_err(|_| "Landlock child reaping timed out".to_string())?
                    .map_err(|e| e.to_string())?;
            }
            self.child.take();
            return cleanup;
        }
        let result = match &mut self.container {
            Some(guard) => guard.finish().await.map_err(|e| e.to_string()),
            None => Ok(()),
        };
        if let Some(child) = &mut self.child {
            tokio::time::timeout(Duration::from_secs(3), child.wait())
                .await
                .map_err(|_| "worker attachment cleanup timed out".to_string())?
                .map_err(|e| format!("worker attachment cleanup failed: {e}"))?;
        }
        self.child.take();
        result
    }
}
impl Drop for StreamGuard {
    fn drop(&mut self) {
        self.stop();
        if let Some(mut child) = self.child.take() {
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _ = child.wait().await;
                });
            }
        }
        // All supervised backend guards cancel their independent owners on drop.
    }
}
