//! Output watchdog for idle-timeout detection
//!
//! Provides `OutputWatchdog`, a per-execution helper that reads process
//! output while monitoring for idle periods. If the process stops
//! producing output for longer than `idle_timeout`, reading is aborted
//! and the caller is notified so it can kill the stalled process.

use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::time::Duration;

/// Callback invoked with each complete line as it arrives.
///
/// Used to observe a child's output while it still running rather than after
/// it exits — the accumulated buffer is discarded on a runtime timeout, so a
/// caller that needs a durable record of a long run cannot wait for the end.
pub type LineSink = Arc<dyn Fn(&str) + Send + Sync>;

/// Per-execution helper that wraps output reading with idle-timeout detection.
pub struct OutputWatchdog {
    /// Maximum time to wait for new output before declaring idle.
    idle_timeout: Duration,
    /// Maximum bytes to read before truncating.
    max_bytes: usize,
    /// Optional per-line observer, invoked as each newline arrives.
    line_sink: Option<LineSink>,
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
            line_sink: None,
        }
    }

    /// Observe each complete line as it arrives, in addition to accumulating.
    ///
    /// The sink runs inline on the read path, so it must not block: a slow
    /// sink stalls reading and can trip the idle timeout on a healthy child.
    pub fn with_line_sink(mut self, sink: LineSink) -> Self {
        self.line_sink = Some(sink);
        self
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
        // Byte offset up to which complete lines have been handed to the sink.
        let mut emitted = 0usize;
        let mut clean_eof = false;

        loop {
            match tokio::time::timeout(self.idle_timeout, reader.read(&mut buf[total..])).await {
                Ok(Ok(0)) => {
                    clean_eof = true;
                    break; // EOF
                }
                Ok(Ok(n)) => {
                    total += n;
                    if total > self.max_bytes {
                        total = self.max_bytes;
                        break;
                    }
                    if let Some(sink) = &self.line_sink {
                        // `\n` cannot appear inside a multi-byte UTF-8 sequence,
                        // so every slice between newlines is a whole line even
                        // when a read splits the stream mid-character.
                        while let Some(rel) = buf[emitted..total].iter().position(|b| *b == b'\n') {
                            let end = emitted + rel;
                            sink(
                                String::from_utf8_lossy(&buf[emitted..end]).trim_end_matches('\r'),
                            );
                            emitted = end + 1;
                        }
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

        // A final line with no trailing newline is only complete at clean EOF;
        // after truncation or an idle timeout the remainder is a partial line
        // and handing it over would emit a torn record.
        if clean_eof && emitted < total {
            if let Some(sink) = &self.line_sink {
                sink(String::from_utf8_lossy(&buf[emitted..total]).trim_end_matches('\r'));
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

    /// Collect every line the sink observes, in order.
    fn recording_sink() -> (LineSink, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let handle = seen.clone();
        let sink: LineSink = std::sync::Arc::new(move |line: &str| {
            handle.lock().unwrap().push(line.to_string());
        });
        (sink, seen)
    }

    /// A JSON event split across two reads must still arrive as one line.
    /// The child writes whenever it likes, so read boundaries fall mid-record
    /// routinely; framing on them would hand the journal torn JSON.
    #[tokio::test]
    async fn line_sink_reassembles_records_split_across_reads() {
        let (mut writer, mut reader) = duplex(1024);
        let (sink, seen) = recording_sink();
        let watchdog = OutputWatchdog::new(Duration::from_secs(5), 4096).with_line_sink(sink);

        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            writer.write_all(b"{\"type\":\"assis").await.unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
            writer
                .write_all(b"tant\"}\n{\"type\":\"result\"}\n")
                .await
                .unwrap();
            drop(writer);
        });

        let out = watchdog.read_with_idle_detection(&mut reader).await;
        assert!(!out.idle_timeout_triggered);
        let lines = seen.lock().unwrap().clone();
        assert_eq!(
            lines,
            vec![
                r#"{"type":"assistant"}"#.to_string(),
                r#"{"type":"result"}"#.to_string(),
            ],
            "a record split across reads must be reassembled, not framed on the read boundary"
        );
    }

    /// A final line with no trailing newline is complete at EOF, so it counts.
    #[tokio::test]
    async fn line_sink_emits_final_unterminated_line_at_eof() {
        let (mut writer, mut reader) = duplex(1024);
        let (sink, seen) = recording_sink();
        let watchdog = OutputWatchdog::new(Duration::from_secs(5), 4096).with_line_sink(sink);

        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            writer.write_all(b"first\nlast-no-newline").await.unwrap();
            drop(writer);
        });

        watchdog.read_with_idle_detection(&mut reader).await;
        assert_eq!(
            seen.lock().unwrap().clone(),
            vec!["first".to_string(), "last-no-newline".to_string()]
        );
    }

    /// Truncation cuts mid-record, so the remainder is a partial line. Handing
    /// it to the sink would append a torn record to the journal.
    #[tokio::test]
    async fn line_sink_drops_partial_line_on_truncation() {
        let (mut writer, mut reader) = duplex(1024);
        let (sink, seen) = recording_sink();
        // Cap below the second line so it is cut mid-record.
        let watchdog = OutputWatchdog::new(Duration::from_secs(5), 12).with_line_sink(sink);

        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            let _ = writer.write_all(b"ok\nthis-one-is-cut-off\n").await;
            drop(writer);
        });

        let out = watchdog.read_with_idle_detection(&mut reader).await;
        assert!(out.truncated);
        let lines = seen.lock().unwrap().clone();
        assert!(
            !lines.iter().any(|l| l.starts_with("this-one")),
            "a partial line must not reach the sink, got: {lines:?}"
        );
    }

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
