//! Bounded host-process execution for oneshot tools.
//!
//! Process groups and output limits are lifecycle controls, not an OS sandbox.
//! Descendants that create their own session require an external worker boundary.

use std::io::{self, Read};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const MAX_OUTPUT_BYTES: usize = 10 * 1024 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(5);
const TOOL_ENV: &[&str] = &[
    "PATH",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TZ",
    "SystemRoot",
    "WINDIR",
];

struct ProcessGuard {
    child: Child,
    group_live: bool,
}

impl ProcessGuard {
    fn kill_group(&mut self) {
        if self.group_live {
            #[cfg(unix)]
            // SAFETY: the child was spawned as leader of its own process group.
            // The negative pid targets that group, never the caller's group.
            unsafe {
                libc::kill(-(self.child.id() as libc::pid_t), libc::SIGKILL);
            }
            self.group_live = false;
        }
    }
}

impl Drop for ProcessGuard {
    fn drop(&mut self) {
        self.kill_group();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(unix)]
fn nonblocking(pipe: &impl std::os::fd::AsRawFd) -> io::Result<()> {
    let fd = pipe.as_raw_fd();
    // SAFETY: the pipe owns a valid fd for both calls. Only its status flags change.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn capture(mut pipe: impl Read, cancelled: Arc<AtomicBool>, limit: usize) -> io::Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut chunk = [0u8; 8192];
    while !cancelled.load(Ordering::Relaxed) {
        match pipe.read(&mut chunk) {
            Ok(0) => return Ok(output),
            Ok(n) => {
                if n > limit.saturating_sub(output.len()) {
                    return Err(io::Error::other("tool output limit exceeded"));
                }
                output.extend_from_slice(&chunk[..n]);
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(POLL_INTERVAL),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(output)
}

type Reader = Option<JoinHandle<io::Result<Vec<u8>>>>;

fn collect(reader: &mut Reader, output: &mut Option<Vec<u8>>) -> Result<(), String> {
    if reader.as_ref().is_some_and(|r| r.is_finished()) {
        let result = reader
            .take()
            .expect("reader present")
            .join()
            .map_err(|_| "tool output reader panicked".to_string())?;
        *output = Some(result.map_err(|e| format!("Failed reading tool output: {e}"))?);
    }
    Ok(())
}

pub(crate) fn run_command(
    name: &str,
    program: &str,
    args: &[String],
    timeout: Duration,
) -> Result<Output, String> {
    run_with_limit(name, program, args, timeout, MAX_OUTPUT_BYTES)
}

fn run_with_limit(
    name: &str,
    program: &str,
    args: &[String],
    timeout: Duration,
    output_limit: usize,
) -> Result<Output, String> {
    if timeout.is_zero() {
        return Err(format!("Tool '{name}' timed out before execution"));
    }
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| "Tool timeout exceeds supported duration".to_string())?;
    let mut command = Command::new(program);
    command.args(args).env_clear();
    for key in TOOL_ENV {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let child = command
        .spawn()
        .map_err(|e| format!("Failed to execute '{program}': {e}"))?;
    let mut process = ProcessGuard {
        child,
        group_live: true,
    };
    let stdout = process.child.stdout.take().expect("stdout piped");
    let stderr = process.child.stderr.take().expect("stderr piped");
    #[cfg(unix)]
    {
        nonblocking(&stdout)
            .and_then(|_| nonblocking(&stderr))
            .map_err(|e| format!("Failed to configure tool pipes: {e}"))?;
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let mut stdout_reader = Some(std::thread::spawn(move || {
        capture(stdout, flag, output_limit)
    }));
    let flag = cancelled.clone();
    let mut stderr_reader = Some(std::thread::spawn(move || {
        capture(stderr, flag, output_limit)
    }));
    let result = (|| {
        let mut status = None;
        let mut stdout = None;
        let mut stderr = None;
        loop {
            collect(&mut stdout_reader, &mut stdout)?;
            collect(&mut stderr_reader, &mut stderr)?;
            if status.is_none() {
                status = process
                    .child
                    .try_wait()
                    .map_err(|e| format!("Failed waiting on '{program}': {e}"))?;
                if status.is_some() {
                    // Oneshot completion must not leave background workers behind.
                    process.kill_group();
                }
            }
            if let (Some(status), Some(stdout), Some(stderr)) =
                (status, stdout.as_mut(), stderr.as_mut())
            {
                return Ok(Output {
                    status,
                    stdout: std::mem::take(stdout),
                    stderr: std::mem::take(stderr),
                });
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "Tool '{name}' timed out after {timeout:?} and was killed"
                ));
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    })();
    drop(process);
    cancelled.store(true, Ordering::Relaxed);
    // Nonblocking Unix readers observe cancellation even when an escaped
    // descendant retains a pipe. Other platforms must not join a blocked read.
    for reader in [stdout_reader, stderr_reader].into_iter().flatten() {
        if cfg!(unix) || reader.is_finished() {
            let _ = reader.join();
        }
    }
    result
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn shell(code: &str, timeout: Duration, limit: usize) -> Result<Output, String> {
        run_with_limit(
            "fixture",
            "/bin/sh",
            &["-c".into(), code.into()],
            timeout,
            limit,
        )
    }

    #[test]
    fn exhausted_deadline_does_not_start_a_process() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("must-not-exist");
        let code = format!("printf started > '{}'", marker.display());
        let error = shell(&code, Duration::ZERO, 1024).unwrap_err();
        assert!(error.contains("timed out"), "{error}");
        assert!(!marker.exists());
    }

    #[test]
    fn output_limit_terminates_the_process() {
        let started = Instant::now();
        let error = shell(
            "while :; do printf 0123456789; done",
            Duration::from_secs(5),
            1024,
        )
        .unwrap_err();
        assert!(error.contains("output limit"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn stderr_limit_also_terminates_the_process() {
        let error = shell(
            "while :; do printf 0123456789 >&2; done",
            Duration::from_secs(5),
            1024,
        )
        .unwrap_err();
        assert!(error.contains("output limit"), "{error}");
    }

    #[test]
    fn timeout_covers_descendant_held_pipes() {
        let started = Instant::now();
        let error = shell("sleep 5 & wait", Duration::from_millis(100), 1024).unwrap_err();
        assert!(error.contains("timed out"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn normal_exit_cleans_up_background_workers() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("descendant-effect");
        let code = format!(
            "(sleep 1; printf escaped > '{}') & printf completed",
            marker.display()
        );
        let output = shell(&code, Duration::from_secs(3), 1024).unwrap();
        assert_eq!(output.stdout, b"completed");
        std::thread::sleep(Duration::from_millis(1200));
        assert!(
            !marker.exists(),
            "background worker survived oneshot completion"
        );
    }

    #[test]
    #[serial_test::serial]
    fn inherited_credentials_are_removed() {
        let key = "SYMBI_TOOL_PROCESS_TEST_SECRET";
        std::env::set_var(key, "synthetic-canary");
        let result = shell(
            "printf %s \"${SYMBI_TOOL_PROCESS_TEST_SECRET-unset}\"",
            Duration::from_secs(2),
            1024,
        );
        std::env::remove_var(key);
        assert_eq!(result.unwrap().stdout, b"unset");
    }
}
