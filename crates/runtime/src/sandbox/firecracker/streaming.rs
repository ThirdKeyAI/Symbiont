//! Duplex guest stdio with independent, run-retained VMM cleanup.
use super::{connect_guest, guest, FirecrackerRunner, Lease};
use guest::stream::{self, Kind};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream},
    sync::{oneshot, watch},
};
use tokio_util::sync::CancellationToken;

pub struct FirecrackerStdio {
    pub stdin: DuplexStream,
    pub stdout: DuplexStream,
    pub stderr: DuplexStream,
    pub guard: StdioGuard,
}

/// Dropping a stream guard cancels its independent VM owner. Repeated finish
/// calls retain the same acknowledgement, including any transport failure.
pub struct StdioGuard {
    stop: CancellationToken,
    finalize: CancellationToken,
    completion: watch::Receiver<Option<Completion>>,
    pub output_limit: usize,
}
#[derive(Clone)]
struct Completion {
    outcome: Result<(), String>,
    cleanup: Result<(), String>,
    exit_code: Option<i32>,
}
impl StdioGuard {
    pub async fn finish(&mut self) -> anyhow::Result<()> {
        self.finalize.cancel();
        self.wait().await?.outcome.map_err(anyhow::Error::msg)
    }
    /// Retain operational failure for `finish`, while acknowledging successful
    /// removal separately to the run owner, including after cancellation.
    pub async fn finish_cleanup(&mut self) -> anyhow::Result<()> {
        self.completed().await?.cleanup.map_err(anyhow::Error::msg)
    }
    async fn completed(&mut self) -> anyhow::Result<Completion> {
        self.stop.cancel();
        self.wait().await
    }
    /// Wait without cancelling. Success requires a verified guest outcome;
    /// cancellation or owner disappearance cannot fabricate a process exit.
    pub async fn wait_for_exit(&mut self) -> anyhow::Result<i32> {
        let result = self.wait().await?;
        result.cleanup.map_err(anyhow::Error::msg)?;
        if let Some(code) = result.exit_code {
            return Ok(code);
        }
        result.outcome.map_err(anyhow::Error::msg)?;
        anyhow::bail!("VM stream ended without an observed guest exit")
    }
    async fn wait(&mut self) -> anyhow::Result<Completion> {
        loop {
            if let Some(result) = self.completion.borrow().clone() {
                return Ok(result);
            }
            self.completion.changed().await.map_err(|_| {
                anyhow::anyhow!("VM stdio owner ended without cleanup acknowledgement")
            })?;
        }
    }
}
impl Drop for StdioGuard {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

impl FirecrackerRunner {
    /// Start an exact command inside a fresh guest and expose bounded byte
    /// streams. The guard must live through discovery, verification and use.
    pub async fn spawn_stdio(
        &self,
        argv: &[String],
        environment: HashMap<String, String>,
        budget: Duration,
    ) -> anyhow::Result<FirecrackerStdio> {
        self.spawn_streams(
            argv,
            environment,
            budget,
            self.config.startup_timeout,
            guest::CommandMode::Stdio,
            None,
        )
        .await
    }

    /// Allocate a controlling terminal inside the guest. Startup consumes the
    /// current interaction budget; the VM retains its separately bounded life.
    pub async fn spawn_terminal(
        &self,
        argv: &[String],
        environment: HashMap<String, String>,
        startup: Duration,
        lifetime: Duration,
        cancellation: watch::Receiver<bool>,
    ) -> anyhow::Result<FirecrackerStdio> {
        self.spawn_streams(
            argv,
            environment,
            lifetime,
            startup,
            guest::CommandMode::Pty,
            Some(cancellation),
        )
        .await
    }

    async fn spawn_streams(
        &self,
        argv: &[String],
        environment: HashMap<String, String>,
        budget: Duration,
        startup_budget: Duration,
        mode: guest::CommandMode,
        cancellation: Option<watch::Receiver<bool>>,
    ) -> anyhow::Result<FirecrackerStdio> {
        anyhow::ensure!(
            self.config.snapshot.is_none(),
            "Git snapshot transfer requires a oneshot worker"
        );
        if startup_budget.is_zero() {
            anyhow::bail!("VM stream startup budget exhausted");
        }
        let started = Instant::now();
        let lifetime = budget.min(self.config.max_execution_time);
        let deadline = started + lifetime;
        let startup = deadline.min(started + self.config.startup_timeout.min(startup_budget));
        let id = uuid::Uuid::new_v4();
        let mut environment: std::collections::BTreeMap<String, String> =
            environment.into_iter().collect();
        for (name, value) in [
            ("PATH", "/usr/bin:/bin"),
            ("HOME", "/tmp"),
            ("LANG", "C.UTF-8"),
        ] {
            environment
                .entry(name.into())
                .or_insert_with(|| value.into());
        }
        let request = guest::Command {
            version: guest::VERSION,
            id: id.to_string(),
            mode,
            argv: argv.to_vec(),
            environment,
            working_dir: self.config.working_dir.clone(),
            input_length: 0,
            input_as_file: false,
            snapshot: None,
            files: self
                .config
                .files
                .as_ref()
                .map(|files| files.begin())
                .transpose()?,
            max_output_bytes: self.config.max_output_bytes,
            timeout_ms: lifetime.as_millis().min(u64::MAX as u128) as u64,
        };
        request.validate()?;
        guest::encode_header(&request)?;
        let specification = self.specification(id, lifetime);
        specification.validate()?;
        let registration = super::super::command_cleanup::register().map_err(anyhow::Error::msg)?;
        let run_stop = registration
            .as_ref()
            .map_or_else(CancellationToken::new, |r| r.stop.clone());
        let stop = CancellationToken::new();
        let finalize = CancellationToken::new();
        let mut startup_guard = super::CancelOnDrop(stop.clone());
        let config = self.config.clone();
        let (ready, receive) = oneshot::channel();
        tokio::spawn(async move {
            let mut ready = Some(ready);
            let mut lease =
                match Lease::register_vm(&config.supervisor, specification, startup).await {
                    Ok(lease) => lease,
                    Err(error) => {
                        if let Some(owner) = registration {
                            owner.finish(Err(format!(
                            "VM stdio registration failed without cleanup acknowledgement: {error}"
                        )));
                        }
                        let _ = ready.take().unwrap().send(Err(error));
                        return;
                    }
                };
            let initialization = async {
                let (_, path) = lease.vm_created(startup).await?;
                if let Some(services) = &config.services {
                    services.install(&path)?;
                }
                let mut socket = connect_guest(&path, startup).await?;
                let mut request = request;
                request.timeout_ms = deadline
                    .saturating_duration_since(Instant::now())
                    .as_millis()
                    .min(u64::MAX as u128) as u64;
                request.validate()?;
                let header = guest::encode_header(&request)?;
                socket.write_u32(header.len() as u32).await?;
                socket.write_all(&header).await?;
                if let Some(files) = &config.files {
                    files.upload(&mut socket).await?;
                }
                let (kind, bytes) = read_frame(&mut socket).await?;
                if kind != Kind::Started {
                    anyhow::bail!("guest did not acknowledge stdio command startup");
                }
                let started: stream::Started = serde_json::from_slice(&bytes)?;
                if started.version != guest::VERSION || started.id != request.id {
                    anyhow::bail!("mismatched guest stdio startup");
                }
                Ok::<_, anyhow::Error>((socket, request))
            };
            let initialized = tokio::select! {
                biased;
                _=stop.cancelled()=>Err(anyhow::anyhow!("VM stdio initialization cancelled")),
                _=startup_cancelled(cancellation)=>Err(anyhow::anyhow!("VM terminal initialization cancelled")),
                _=run_stop.cancelled()=>Err(anyhow::anyhow!("VM stdio run closed")),
                result=tokio::time::timeout_at(startup.into(), initialization)=>result.map_err(|_|anyhow::anyhow!("VM stdio startup deadline expired")).and_then(|r|r),
            };
            let (done, completion) = watch::channel(None);
            let result = match initialized {
                Err(error) => Err(error),
                Ok((socket, request)) => {
                    let (stdin, input) = tokio::io::duplex(stream::MAX_FRAME);
                    let (stdout, output) = tokio::io::duplex(stream::MAX_FRAME);
                    let (stderr, error_output) = tokio::io::duplex(stream::MAX_FRAME);
                    let worker = FirecrackerStdio {
                        stdin,
                        stdout,
                        stderr,
                        guard: StdioGuard {
                            stop: stop.clone(),
                            finalize: finalize.clone(),
                            completion,
                            output_limit: config.max_output_bytes,
                        },
                    };
                    // If the caller disappeared, dropping the undelivered guard
                    // cancels this owner while it still retains the VM lease.
                    let _ = ready.take().unwrap().send(Ok(worker));
                    let bridge = bridge(
                        socket,
                        &request,
                        input,
                        output,
                        error_output,
                        config.files.as_deref(),
                        &finalize,
                    );
                    tokio::pin!(bridge);
                    tokio::select! {
                        biased;
                        result=&mut bridge=>result.map(Some),
                        _=run_stop.cancelled()=>Err(anyhow::anyhow!("VM stdio run closed")),
                        _=tokio::time::sleep_until(deadline.into())=>Err(anyhow::anyhow!("VM stdio lifetime expired")),
                        _=stop.cancelled()=>Err(anyhow::anyhow!("VM stdio cancelled")),
                        _=finalize.cancelled()=> {
                            if config.files.as_ref().is_some_and(|files| files.has_output()) {
                                let grace = deadline.min(Instant::now() + Duration::from_secs(2));
                                tokio::select! {
                                    biased;
                                    _=stop.cancelled()=>Err(anyhow::anyhow!("VM file finalization cancelled")),
                                    _=run_stop.cancelled()=>Err(anyhow::anyhow!("VM stdio run closed")),
                                    result=tokio::time::timeout_at(grace.into(), &mut bridge)=>result
                                        .map_err(|_|anyhow::anyhow!("VM file finalization deadline expired"))
                                        .and_then(|result| result.map(Some)),
                                }
                            } else { Ok(None) }
                        },
                    }
                }
            };
            let result = if result.is_ok() && Instant::now() >= deadline {
                Err(anyhow::anyhow!("VM stdio lifetime expired"))
            } else {
                result
            };
            let outcome = lease.finish().await;
            let cleanup = lease.finish_cleanup().await.map_err(|e| e.to_string());
            if let Some(owner) = registration {
                owner.finish(cleanup.clone());
            }
            let result = match &cleanup {
                Ok(()) => result.and_then(|exit| outcome.map(|()| exit)),
                Err(error) => Err(anyhow::anyhow!("VM stdio cleanup failed: {error}")),
            };
            if result
                .as_ref()
                .is_ok_and(|exit| exit.is_some_and(|exit| exit.accepted()))
            {
                if let Some(files) = &config.files {
                    files.complete();
                }
            }
            if let Some(ready) = ready {
                let _ = ready.send(Err(anyhow::anyhow!(result
                    .err()
                    .map_or_else(|| "VM stdio startup ended".into(), |e| e.to_string()))));
            } else {
                done.send_replace(Some(Completion {
                    exit_code: result
                        .as_ref()
                        .ok()
                        .copied()
                        .flatten()
                        .filter(|exit| !exit.finalized)
                        .map(|exit| exit.exit_code),
                    outcome: result
                        .and_then(|exit| {
                            if let Some(exit) = exit.filter(|exit| !exit.accepted()) {
                                anyhow::bail!(
                                    "guest stdio command failed: exit {}",
                                    exit.exit_code
                                );
                            }
                            Ok(())
                        })
                        .map_err(|e| e.to_string()),
                    cleanup,
                }));
            }
        });
        let result = receive
            .await
            .map_err(|_| anyhow::anyhow!("VM stdio initialization owner ended"))?;
        // The returned guard now owns cancellation. Do not cancel it on normal startup.
        startup_guard.0 = CancellationToken::new();
        result
    }
}

async fn startup_cancelled(cancellation: Option<watch::Receiver<bool>>) {
    if let Some(mut cancellation) = cancellation {
        loop {
            if *cancellation.borrow_and_update() {
                return;
            }
            // A successful caller drops its temporary startup sender. Closure
            // does not revoke the independently owned live terminal session.
            if cancellation.changed().await.is_err() {
                break;
            }
        }
    }
    std::future::pending::<()>().await;
}

async fn read_frame(reader: &mut (impl AsyncRead + Unpin)) -> anyhow::Result<(Kind, Vec<u8>)> {
    let tag = reader.read_u8().await?;
    let length = reader.read_u32().await? as usize;
    let kind = Kind::decode(tag, length)?;
    let mut data = vec![0; length];
    reader.read_exact(&mut data).await?;
    Ok((kind, data))
}
async fn write_frame(
    writer: &mut (impl AsyncWrite + Unpin),
    kind: Kind,
    bytes: &[u8],
) -> anyhow::Result<()> {
    let frame = stream::encode(kind, bytes)?;
    writer.write_all(&frame).await?;
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct StreamExit {
    exit_code: i32,
    finalized: bool,
}
impl StreamExit {
    fn accepted(self) -> bool {
        self.exit_code == 0 || (self.finalized && self.exit_code == -9)
    }
}

async fn bridge(
    socket: tokio::net::UnixStream,
    request: &guest::Command,
    mut input: DuplexStream,
    mut stdout: DuplexStream,
    mut stderr: DuplexStream,
    files: Option<&super::files::Transfer>,
    finalize: &CancellationToken,
) -> anyhow::Result<StreamExit> {
    let (mut reader, mut writer) = socket.into_split();
    let finalized = std::sync::atomic::AtomicBool::new(false);
    let upload = async {
        let mut count = 0usize;
        let mut bytes = [0; 8192];
        let mut closed = false;
        loop {
            let n = tokio::select! {
                biased;
                _=finalize.cancelled(), if files.is_some_and(|files| files.has_output()) => {
                    let control = stream::Started { version: guest::VERSION, id:request.id.clone() };
                    // Never interrupt a frame write to insert a control message.
                    write_frame(&mut writer,Kind::Finalize,&serde_json::to_vec(&control)?).await?;
                    finalized.store(true,std::sync::atomic::Ordering::Release);
                    return std::future::pending::<anyhow::Result<StreamExit>>().await;
                },
                result=input.read(&mut bytes), if !closed => result?,
                else => return std::future::pending::<anyhow::Result<StreamExit>>().await,
            };
            if n == 0 {
                write_frame(&mut writer, Kind::InputClosed, b"").await?;
                closed = true;
                continue;
            }
            count = count
                .checked_add(n)
                .ok_or_else(|| anyhow::anyhow!("VM stdio input overflow"))?;
            if count > guest::MAX_INPUT {
                anyhow::bail!("VM stdio input limit exceeded");
            }
            write_frame(&mut writer, Kind::Input, &bytes[..n]).await?;
        }
    };
    let download = async {
        let mut counts = [0usize; 2];
        let mut closed = [false; 2];
        loop {
            let (kind, data) = read_frame(&mut reader).await?;
            let index = match kind {
                Kind::Stdout => 0,
                Kind::Stderr => 1,
                Kind::Outcome => {
                    let result: guest::Outcome = serde_json::from_slice(&data)?;
                    result.validate(request)?;
                    if result.finalized && !finalized.load(std::sync::atomic::Ordering::Acquire) {
                        anyhow::bail!("unsolicited VM file finalization");
                    }
                    if [result.stdout_length, result.stderr_length] != counts {
                        anyhow::bail!("VM stdio result disagrees with observed streams");
                    }
                    if result.stdout_truncated
                        || result.stderr_truncated
                        || result.timed_out
                        || result.error.is_some()
                    {
                        anyhow::bail!("guest stdio command failed: exit {}, stdout_truncated={}, stderr_truncated={}, timed_out={}, error={:?}", result.exit_code, result.stdout_truncated, result.stderr_truncated, result.timed_out, result.error);
                    }
                    if let Some(receipt) = &result.file_output {
                        files
                            .ok_or_else(|| anyhow::anyhow!("unsolicited VM file output"))?
                            .receive(&mut reader, receipt)
                            .await?;
                    }
                    if reader.read(&mut [0; 1]).await? != 0 {
                        anyhow::bail!("unexpected data after VM stdio result");
                    }
                    return Ok(StreamExit {
                        exit_code: result.exit_code,
                        finalized: result.finalized,
                    });
                }
                _ => anyhow::bail!("unexpected guest-to-host stream frame"),
            };
            counts[index] = counts[index]
                .checked_add(data.len())
                .ok_or_else(|| anyhow::anyhow!("VM stdio output overflow"))?;
            if counts[index] > request.max_output_bytes {
                anyhow::bail!("VM stdio output limit exceeded");
            }
            if !closed[index] {
                let output = if index == 0 { &mut stdout } else { &mut stderr };
                match output.write_all(&data).await {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => closed[index] = true,
                    Err(e) => return Err(e.into()),
                }
            }
        }
    };
    tokio::select! { biased; result=download=>result, result=upload=>result }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lost_owner_never_acknowledges_removal() {
        let (owner, completion) = watch::channel(None);
        let mut guard = StdioGuard {
            stop: CancellationToken::new(),
            finalize: CancellationToken::new(),
            completion,
            output_limit: 4096,
        };
        drop(owner);
        assert!(guard.wait_for_exit().await.is_err());
        assert!(!guard.stop.is_cancelled());
        for result in [guard.finish_cleanup().await, guard.finish().await] {
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("without cleanup acknowledgement"));
        }
    }

    #[tokio::test]
    async fn passive_exit_preserves_failure_without_fabricating_cancellation_status() {
        for (exit_code, cleanup) in [
            (Some(7), Ok(())),
            (None, Ok(())),
            (Some(0), Err("removal unconfirmed".to_string())),
        ] {
            let (_owner, completion) = watch::channel(Some(Completion {
                outcome: exit_code.map_or(Ok(()), |_| Err("command failed".into())),
                cleanup: cleanup.clone(),
                exit_code,
            }));
            let mut guard = StdioGuard {
                stop: CancellationToken::new(),
                finalize: CancellationToken::new(),
                completion,
                output_limit: 4096,
            };
            let result = guard.wait_for_exit().await;
            assert!(!guard.stop.is_cancelled());
            if exit_code == Some(7) && cleanup.is_ok() {
                assert_eq!(result.unwrap(), 7);
                assert!(guard.finish().await.is_err());
                guard.finish_cleanup().await.unwrap();
            } else {
                assert!(result.is_err());
            }
        }
    }

    #[tokio::test]
    async fn stream_results_require_correlated_complete_bounded_evidence() {
        for mode in [
            "valid",
            "wrong_id",
            "wrong_count",
            "failed_exit",
            "extra_bytes",
            "unknown_frame",
            "oversize_frame",
            "missing_outcome",
        ] {
            let (socket, mut peer) = tokio::net::UnixStream::pair().unwrap();
            let request = guest::Command {
                version: guest::VERSION,
                id: uuid::Uuid::new_v4().to_string(),
                mode: guest::CommandMode::Stdio,
                argv: vec!["/bin/true".into()],
                environment: Default::default(),
                working_dir: "/tmp".into(),
                input_length: 0,
                input_as_file: false,
                files: None,
                snapshot: None,
                max_output_bytes: 4,
                timeout_ms: 1000,
            };
            let mut outcome = guest::Outcome {
                version: guest::VERSION,
                id: request.id.clone(),
                exit_code: 0,
                stdout_length: 1,
                stderr_length: 0,
                stdout_truncated: false,
                stderr_truncated: false,
                timed_out: false,
                error: None,
                file_output: None,
                snapshot: None,
                finalized: false,
            };
            if mode == "wrong_id" {
                outcome.id = uuid::Uuid::new_v4().to_string();
            }
            if mode == "wrong_count" {
                outcome.stdout_length = 0;
            }
            if mode == "failed_exit" {
                outcome.exit_code = 17;
            }
            let server = tokio::spawn(async move {
                if mode == "unknown_frame" {
                    peer.write_all(&[255, 0, 0, 0, 1]).await.unwrap();
                } else if mode == "oversize_frame" {
                    peer.write_all(&[3, 255, 255, 255, 255]).await.unwrap();
                } else {
                    write_frame(&mut peer, Kind::Stdout, b"x").await.unwrap();
                    if mode != "missing_outcome" {
                        write_frame(
                            &mut peer,
                            Kind::Outcome,
                            &serde_json::to_vec(&outcome).unwrap(),
                        )
                        .await
                        .unwrap();
                    }
                    if mode == "extra_bytes" {
                        peer.write_all(b"extra").await.unwrap();
                    }
                }
                peer.shutdown().await.unwrap();
            });
            let (_stdin, input) = tokio::io::duplex(64);
            let (_stdout, output) = tokio::io::duplex(64);
            let (_stderr, errors) = tokio::io::duplex(64);
            let result = tokio::time::timeout(
                Duration::from_secs(1),
                bridge(
                    socket,
                    &request,
                    input,
                    output,
                    errors,
                    None,
                    &CancellationToken::new(),
                ),
            )
            .await
            .unwrap();
            assert_eq!(
                result.is_ok(),
                matches!(mode, "valid" | "failed_exit"),
                "{mode}: {result:?}"
            );
            if mode == "failed_exit" {
                assert_eq!(result.unwrap().exit_code, 17);
            }
            server.await.unwrap();
        }
    }
    #[tokio::test]
    async fn file_stream_requires_requested_finalization_and_verified_payload() {
        use sha2::{Digest, Sha256};
        for case in [
            "natural",
            "finalized",
            "unsolicited",
            "missing",
            "hash",
            "truncated",
            "extra",
            "failed",
        ] {
            let (_root, transfer) = super::super::files::fixture();
            let request = guest::Command {
                version: guest::VERSION,
                id: uuid::Uuid::new_v4().to_string(),
                mode: guest::CommandMode::Stdio,
                argv: vec!["/bin/server".into()],
                environment: Default::default(),
                working_dir: "/tmp".into(),
                input_length: 0,
                input_as_file: false,
                files: Some(transfer.begin().unwrap()),
                snapshot: None,
                max_output_bytes: 1024,
                timeout_ms: 1000,
            };
            let mut outcome = guest::Outcome {
                version: guest::VERSION,
                id: request.id.clone(),
                exit_code: 0,
                stdout_length: 0,
                stderr_length: 0,
                stdout_truncated: false,
                stderr_truncated: false,
                timed_out: false,
                error: None,
                finalized: false,
                snapshot: None,
                file_output: Some(guest::files::Receipt {
                    path: "/tmp/result".into(),
                    length: 3,
                    sha256: format!("{:x}", Sha256::digest(b"x\0y")),
                }),
            };
            let finalize = CancellationToken::new();
            if matches!(case, "finalized" | "unsolicited") {
                outcome.finalized = true;
                outcome.exit_code = -9;
            }
            if case == "finalized" {
                finalize.cancel();
            }
            if case == "failed" {
                outcome.exit_code = 17;
                outcome.file_output = None;
            }
            if case == "missing" {
                outcome.file_output = None;
            }
            let (socket, mut peer) = tokio::net::UnixStream::pair().unwrap();
            let server = tokio::spawn(async move {
                if case == "finalized" {
                    let (kind, data) = read_frame(&mut peer).await.unwrap();
                    assert_eq!(kind, Kind::Finalize);
                    let control: stream::Started = serde_json::from_slice(&data).unwrap();
                    assert_eq!(control.id, outcome.id);
                }
                let _ = write_frame(
                    &mut peer,
                    Kind::Outcome,
                    &serde_json::to_vec(&outcome).unwrap(),
                )
                .await;
                if outcome.file_output.is_some() {
                    let _ = peer
                        .write_all(match case {
                            "hash" => b"bad",
                            "truncated" => b"x",
                            _ => b"x\0y",
                        })
                        .await;
                }
                if case == "extra" {
                    let _ = peer.write_all(b"extra").await;
                }
                let _ = peer.shutdown().await;
            });
            let (_stdin, input) = tokio::io::duplex(64);
            let (_stdout, output) = tokio::io::duplex(64);
            let (_stderr, errors) = tokio::io::duplex(64);
            let result = tokio::time::timeout(
                Duration::from_secs(1),
                bridge(
                    socket,
                    &request,
                    input,
                    output,
                    errors,
                    Some(&transfer),
                    &finalize,
                ),
            )
            .await
            .unwrap();
            assert_eq!(
                result.is_ok(),
                matches!(case, "natural" | "finalized" | "failed"),
                "{case}: {result:?}"
            );
            assert!(transfer.check_publication().is_err());
            if let Ok(exit) = result {
                assert_eq!(exit.finalized, case == "finalized");
                if exit.accepted() {
                    transfer.complete();
                    transfer.check_publication().unwrap();
                } else {
                    assert!(transfer.check_publication().is_err());
                }
            }
            server.await.unwrap();
        }
    }
}
