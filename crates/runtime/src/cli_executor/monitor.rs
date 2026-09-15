//! One activity clock and bounded, fallible output collection for a CLI child.

use std::time::{Duration, Instant};
use tokio::{io::AsyncReadExt, process::Child};

use super::watchdog::LineSink;
use crate::sandbox::ExecutionResult;

pub(super) async fn monitor(
    child: &mut Child,
    deadline: Instant,
    idle: Duration,
    limit: usize,
    sink: Option<&LineSink>,
    started: Instant,
) -> anyhow::Result<ExecutionResult> {
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("CLI stdout unavailable"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("CLI stderr unavailable"))?;
    Settings {
        deadline,
        idle,
        limit,
        sink,
        started,
    }
    .collect(&mut stdout, &mut stderr, async {
        child
            .wait()
            .await
            .map(|status| status.code().unwrap_or(-1))
            .map_err(Into::into)
    })
    .await
}

pub(super) struct Settings<'a> {
    pub deadline: Instant,
    pub idle: Duration,
    pub limit: usize,
    pub sink: Option<&'a LineSink>,
    pub started: Instant,
}
impl Settings<'_> {
    pub async fn collect(
        self,
        stdout: &mut (impl tokio::io::AsyncRead + Unpin),
        stderr: &mut (impl tokio::io::AsyncRead + Unpin),
        exit: impl std::future::Future<Output = anyhow::Result<i32>>,
    ) -> anyhow::Result<ExecutionResult> {
        tokio::pin!(exit);
        let Self {
            deadline,
            idle,
            limit,
            sink,
            started,
        } = self;
        let mut out = Stream::default();
        let mut err = Stream::default();
        let mut out_buf = [0; 8192];
        let mut err_buf = [0; 8192];
        let mut status = None;
        let mut active = Instant::now();
        loop {
            if out.closed && err.closed && status.is_some() {
                break;
            }
            let idle_deadline = active.checked_add(idle).unwrap_or(deadline).min(deadline);
            tokio::select! {
                biased;
                _ = tokio::time::sleep_until(deadline.into()) => anyhow::bail!("CLI execution timed out"),
                _ = tokio::time::sleep_until(idle_deadline.into()) => anyhow::bail!("CLI idle timeout"),
                read = stdout.read(&mut out_buf), if !out.closed => {
                    let size = read.map_err(|e| anyhow::anyhow!("CLI stdout read failed: {e}"))?;
                    out.append(&out_buf[..size], limit, sink)?;
                    if size > 0 { active = Instant::now(); }
                }
                read = stderr.read(&mut err_buf), if !err.closed => {
                    let size = read.map_err(|e| anyhow::anyhow!("CLI stderr read failed: {e}"))?;
                    err.append(&err_buf[..size], limit, None)?;
                    if size > 0 { active = Instant::now(); }
                }
                result = &mut exit, if status.is_none() => {
                    status = Some(result.map_err(|e| anyhow::anyhow!("CLI process wait failed: {e}"))?);
                }
            }
        }
        let status = status.ok_or_else(|| anyhow::anyhow!("CLI exit status unavailable"))?;
        Ok(ExecutionResult {
            exit_code: status,
            success: status == 0,
            stdout: String::from_utf8_lossy(&out.bytes).into_owned(),
            stderr: String::from_utf8_lossy(&err.bytes).into_owned(),
            stdout_truncated: false,
            stderr_truncated: false,
            execution_time_ms: started.elapsed().as_millis() as u64,
        })
    }
}

#[derive(Default)]
struct Stream {
    bytes: Vec<u8>,
    emitted: usize,
    closed: bool,
}
impl Stream {
    fn append(
        &mut self,
        chunk: &[u8],
        limit: usize,
        sink: Option<&LineSink>,
    ) -> anyhow::Result<()> {
        if chunk.is_empty() {
            self.closed = true;
            if self.emitted < self.bytes.len() {
                if let Some(sink) = sink {
                    sink(&String::from_utf8_lossy(&self.bytes[self.emitted..]));
                }
            }
            return Ok(());
        }
        let remaining = limit.saturating_sub(self.bytes.len());
        self.bytes
            .extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        if let Some(sink) = sink {
            while let Some(offset) = self.bytes[self.emitted..]
                .iter()
                .position(|byte| *byte == b'\n')
            {
                let end = self.emitted + offset;
                sink(
                    String::from_utf8_lossy(&self.bytes[self.emitted..end]).trim_end_matches('\r'),
                );
                self.emitted = end + 1;
            }
        }
        if chunk.len() > remaining {
            anyhow::bail!("CLI output limit exceeded");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_limit_is_valid_but_one_more_byte_fails() {
        let mut stream = Stream::default();
        stream.append(b"abc", 3, None).unwrap();
        stream.append(b"", 3, None).unwrap();
        assert!(stream.closed);
        let mut overflow = Stream::default();
        overflow.append(b"abc", 3, None).unwrap();
        assert!(overflow.append(b"d", 3, None).is_err());
        assert_eq!(overflow.bytes, b"abc");
    }

    #[test]
    fn split_utf8_and_complete_lines_survive_overflow_without_torn_events() {
        let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let collected = lines.clone();
        let sink: LineSink =
            std::sync::Arc::new(move |line| collected.lock().unwrap().push(line.to_owned()));
        let mut stream = Stream::default();
        let bytes = "界\nok\ntorn".as_bytes();
        stream.append(&bytes[..2], 8, Some(&sink)).unwrap();
        assert!(stream.append(&bytes[2..], 8, Some(&sink)).is_err());
        assert_eq!(*lines.lock().unwrap(), vec!["界", "ok"]);
    }
}
