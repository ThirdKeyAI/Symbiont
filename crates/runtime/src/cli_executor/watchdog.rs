//! Output watchdog for idle-timeout detection
//!
//! Provides `OutputWatchdog`, a per-execution helper that reads process
//! output while monitoring for idle periods. If the process stops
//! producing output for longer than `idle_timeout`, reading is aborted
//! and the caller is notified so it can kill the stalled process.

use tokio::io::AsyncReadExt;
use tokio::time::Duration;

/// Per-execution helper that wraps output reading with idle-timeout detection.
pub struct OutputWatchdog {
    /// Maximum time to wait for new output before declaring idle.
    idle_timeout: Duration,
    /// Maximum bytes to read before truncating.
    max_bytes: usize,
}

/// Result of reading output through the watchdog.
#[derive(Debug, Clone)]
pub struct WatchdogOutput {
    /// The data that was read.
    pub data: String,
    /// Whether the output was truncated at `max_bytes`.
    pub truncated: bool,
    /// Whether reading stopped because of an idle timeout.
    pub idle_timeout_triggered: bool,
    /// Total bytes read before any truncation marker.
    pub bytes_read: usize,
}

impl OutputWatchdog {
    /// Create a new watchdog with the given idle timeout and byte limit.
    pub fn new(idle_timeout: Duration, max_bytes: usize) -> Self {
        Self {
            idle_timeout,
            max_bytes,
        }
    }

    /// Read from `reader` with idle-timeout detection.
    ///
    /// Each successful read resets the idle deadline. If no data arrives
    /// within `idle_timeout`, the method returns with
    /// `idle_timeout_triggered = true`.
    ///
    /// Output is truncated at `max_bytes`; a marker is appended when this
    /// happens.
    pub async fn read_with_idle_detection<R: AsyncReadExt + Unpin>(
        &self,
        reader: &mut R,
    ) -> WatchdogOutput {
        let mut buf = vec![0u8; self.max_bytes + 1];
        let mut total = 0usize;
        let mut idle_triggered = false;

        loop {
            match tokio::time::timeout(self.idle_timeout, reader.read(&mut buf[total..])).await {
                Ok(Ok(0)) => break, // EOF
                Ok(Ok(n)) => {
                    total += n;
                    if total > self.max_bytes {
                        total = self.max_bytes;
                        break;
                    }
                }
                Ok(Err(_)) => break, // Read error
                Err(_) => {
                    // Idle timeout fired — no data within the window
                    idle_triggered = true;
                    break;
                }
            }
        }

        let truncated = total == self.max_bytes;
        let bytes_read = total;
        let output = String::from_utf8_lossy(&buf[..total]).to_string();

        let data = if truncated {
            format!(
                "{}\n... [output truncated at {} bytes]",
                output, self.max_bytes
            )
        } else {
            output
        };

        WatchdogOutput {
            data,
            truncated,
            idle_timeout_triggered: idle_triggered,
            bytes_read,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn test_continuous_output_no_idle_timeout() {
        let (mut writer, mut reader) = duplex(1024);

        let watchdog = OutputWatchdog::new(Duration::from_secs(5), 4096);

        // Write data then close the writer so the reader sees EOF
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            writer.write_all(b"hello world").await.unwrap();
            drop(writer);
        });

        let output = watchdog.read_with_idle_detection(&mut reader).await;

        assert!(!output.idle_timeout_triggered);
        assert!(!output.truncated);
        assert_eq!(output.data, "hello world");
        assert_eq!(output.bytes_read, 11);
    }

    #[tokio::test]
    async fn test_idle_timeout_triggers() {
        let (_writer, mut reader) = duplex(1024);

        // Writer stays open but never writes — idle timeout should fire
        let watchdog = OutputWatchdog::new(Duration::from_millis(50), 4096);

        let output = watchdog.read_with_idle_detection(&mut reader).await;

        assert!(output.idle_timeout_triggered);
        assert!(!output.truncated);
        assert_eq!(output.bytes_read, 0);
    }

    #[tokio::test]
    async fn test_truncation_with_watchdog() {
        let (mut writer, mut reader) = duplex(1024);

        let watchdog = OutputWatchdog::new(Duration::from_secs(5), 10);

        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            // Write more than max_bytes
            writer.write_all(b"abcdefghijklmnop").await.unwrap();
            drop(writer);
        });

        let output = watchdog.read_with_idle_detection(&mut reader).await;

        assert!(output.truncated);
        assert!(!output.idle_timeout_triggered);
        assert_eq!(output.bytes_read, 10);
        assert!(output.data.contains("[output truncated at 10 bytes]"));
    }
}
