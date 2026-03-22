//! SessionExecutor — PTY-based interactive CLI tool session manager.
//!
//! Spawns interactive CLI tools in pseudo-terminals, manages per-interaction
//! command validation and policy checking, and captures evidence transcripts.

#[cfg(feature = "toolclad-session")]
use pty_process::blocking::{Command as PtyCommand, Pty};

use std::collections::HashMap;
#[cfg(feature = "toolclad-session")]
use std::io::Read;
#[cfg(feature = "toolclad-session")]
use std::io::Write;
use std::sync::{Arc, Mutex};
#[cfg(feature = "toolclad-session")]
use std::time::{Duration, Instant};

use super::manifest::Manifest;
#[cfg(feature = "toolclad-session")]
use super::manifest::SessionDef;
use super::session_state::*;

/// Manages interactive CLI tool sessions via PTY.
pub struct SessionExecutor {
    sessions: Arc<Mutex<HashMap<SessionId, SessionHandle>>>,
    manifests: HashMap<String, Manifest>,
}

/// A live session handle.
struct SessionHandle {
    #[cfg(feature = "toolclad-session")]
    pty: Pty,
    #[cfg(feature = "toolclad-session")]
    child: std::process::Child,
    state: SessionState,
    transcript: SessionTranscript,
    #[allow(dead_code)]
    manifest_name: String,
}

impl SessionExecutor {
    pub fn new(manifests: Vec<(String, Manifest)>) -> Self {
        let session_manifests: HashMap<String, Manifest> = manifests
            .into_iter()
            .filter(|(_, m)| m.tool.mode == "session")
            .collect();
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            manifests: session_manifests,
        }
    }

    pub fn handles(&self, tool_name: &str) -> bool {
        // Check for "toolname.command" pattern
        if let Some(base) = tool_name.split('.').next() {
            if let Some(m) = self.manifests.get(base) {
                if let Some(session) = &m.session {
                    let cmd = tool_name
                        .strip_prefix(base)
                        .unwrap_or("")
                        .trim_start_matches('.');
                    return !cmd.is_empty() && session.commands.contains_key(cmd);
                }
            }
        }
        false
    }

    /// Execute a session command. Creates the session if it doesn't exist.
    pub fn execute_session_command(
        &self,
        tool_name: &str,
        args_json: &str,
    ) -> Result<serde_json::Value, String> {
        let (manifest_name, command_name) = parse_session_tool_name(tool_name)?;

        let manifest = self
            .manifests
            .get(&manifest_name)
            .ok_or_else(|| format!("No session manifest for '{}'", manifest_name))?;
        let session_def = manifest
            .session
            .as_ref()
            .ok_or("Manifest has no [session] section")?;
        let cmd_def = session_def
            .commands
            .get(&command_name)
            .ok_or_else(|| format!("Unknown session command: {}", command_name))?;

        // Parse and validate arguments
        let args: HashMap<String, serde_json::Value> =
            serde_json::from_str(args_json).map_err(|e| format!("Invalid arguments: {}", e))?;

        let command_str = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or("Session command requires 'command' argument")?;

        // Validate command against pattern
        let re = regex::Regex::new(&cmd_def.pattern)
            .map_err(|e| format!("Invalid command pattern: {}", e))?;
        if !re.is_match(command_str) {
            return Err(format!(
                "Command '{}' does not match pattern '{}' for {}",
                command_str, cmd_def.pattern, command_name
            ));
        }

        // Check max interactions
        {
            let sessions = self.sessions.lock().map_err(|e| e.to_string())?;
            if let Some(handle) = sessions.get(&manifest_name) {
                if handle.state.interaction_count >= session_def.max_interactions {
                    return Err(format!(
                        "Session '{}' exceeded max interactions ({})",
                        manifest_name, session_def.max_interactions
                    ));
                }
            }
        }

        // Ensure session exists (spawn if needed)
        #[cfg(feature = "toolclad-session")]
        {
            self.ensure_session(&manifest_name, manifest, session_def)?;
        }

        // Send command and get response
        #[cfg(feature = "toolclad-session")]
        {
            let mut sessions = self.sessions.lock().map_err(|e| e.to_string())?;
            let handle = sessions
                .get_mut(&manifest_name)
                .ok_or("Session not found after ensure")?;

            let start = Instant::now();

            // Write command to PTY
            handle
                .pty
                .write_all(format!("{}\n", command_str).as_bytes())
                .map_err(|e| format!("Failed to write to PTY: {}", e))?;
            handle
                .pty
                .flush()
                .map_err(|e| format!("Flush failed: {}", e))?;

            // Log command
            handle.transcript.append(
                TranscriptDirection::Command,
                command_str,
                Some(&command_name),
            );

            // Read until prompt
            let output_wait = session_def
                .interaction
                .as_ref()
                .map(|i| i.output_wait_ms)
                .unwrap_or(2000);
            let max_bytes = session_def
                .interaction
                .as_ref()
                .map(|i| i.output_max_bytes)
                .unwrap_or(1_048_576) as usize;

            let output = read_until_prompt_blocking(
                &mut handle.pty,
                &session_def.ready_pattern,
                Duration::from_millis(output_wait * 5), // give 5x the wait time
                max_bytes,
            )?;

            let duration_ms = start.elapsed().as_millis() as u64;

            // Strip ANSI and extract meaningful output
            let clean_output = strip_ansi(&output.0);
            let prompt = output.1.clone();

            // Update state
            handle.state.interaction_count += 1;
            handle.state.last_interaction_at = Instant::now();
            handle.state.prompt = prompt.clone();
            handle.state.inferred_state = infer_state(&prompt);

            // Log response
            handle.transcript.append(
                TranscriptDirection::Response,
                &clean_output,
                Some(&command_name),
            );

            // Build envelope
            let scan_id = format!(
                "{}-{}",
                chrono::Utc::now().timestamp(),
                uuid::Uuid::new_v4().as_fields().0
            );
            return Ok(serde_json::json!({
                "status": "success",
                "scan_id": scan_id,
                "tool": tool_name,
                "session_id": handle.state.session_id,
                "duration_ms": duration_ms,
                "timestamp": chrono::Utc::now().to_rfc3339(),
                "exit_code": 0,
                "stderr": "",
                "results": {
                    "output": clean_output,
                    "prompt": prompt,
                    "session_state": handle.state.inferred_state,
                    "interaction_count": handle.state.interaction_count,
                }
            }));
        }

        #[cfg(not(feature = "toolclad-session"))]
        Err("Session mode requires the 'toolclad-session' feature".to_string())
    }

    #[cfg(feature = "toolclad-session")]
    fn ensure_session(
        &self,
        name: &str,
        _manifest: &Manifest,
        session_def: &SessionDef,
    ) -> Result<(), String> {
        let mut sessions = self.sessions.lock().map_err(|e| e.to_string())?;
        if sessions.contains_key(name) {
            return Ok(());
        }

        // Spawn PTY
        let pty = Pty::new().map_err(|e| format!("Failed to create PTY: {}", e))?;
        let pts = pty.pts().map_err(|e| format!("Failed to get PTS: {}", e))?;

        let child = PtyCommand::new("sh")
            .arg("-c")
            .arg(&session_def.startup_command)
            .spawn(&pts)
            .map_err(|e| format!("Failed to spawn '{}': {}", session_def.startup_command, e))?;

        let session_id = format!("session-{}-{}", name, uuid::Uuid::new_v4().as_fields().0);

        let handle = SessionHandle {
            pty,
            child,
            state: SessionState {
                status: SessionStatus::Spawning,
                prompt: String::new(),
                inferred_state: "spawning".to_string(),
                interaction_count: 0,
                started_at: Instant::now(),
                last_interaction_at: Instant::now(),
                session_id,
            },
            transcript: SessionTranscript::default(),
            manifest_name: name.to_string(),
        };

        sessions.insert(name.to_string(), handle);

        // Wait for ready pattern
        let handle = sessions.get_mut(name).unwrap();
        let timeout = Duration::from_secs(session_def.startup_timeout_seconds);
        let output = read_until_prompt_blocking(
            &mut handle.pty,
            &session_def.ready_pattern,
            timeout,
            1_048_576,
        )
        .map_err(|e| format!("Session startup failed: {}", e))?;

        handle.state.status = SessionStatus::Ready;
        handle.state.prompt = output.1;
        handle.state.inferred_state = "ready".to_string();
        handle
            .transcript
            .append(TranscriptDirection::System, "Session started", None);

        Ok(())
    }

    /// Get session transcript for evidence.
    pub fn get_transcript(&self, manifest_name: &str) -> Option<SessionTranscript> {
        let sessions = self.sessions.lock().ok()?;
        sessions.get(manifest_name).map(|h| h.transcript.clone())
    }

    /// Cleanup all sessions.
    pub fn cleanup(&self) {
        if let Ok(mut sessions) = self.sessions.lock() {
            for (_name, handle) in sessions.drain() {
                #[cfg(feature = "toolclad-session")]
                {
                    let mut child = handle.child;
                    let _ = child.kill();
                }
                #[cfg(not(feature = "toolclad-session"))]
                {
                    let _ = handle;
                }
            }
        }
    }
}

fn parse_session_tool_name(name: &str) -> Result<(String, String), String> {
    let parts: Vec<&str> = name.splitn(2, '.').collect();
    if parts.len() != 2 {
        return Err(format!(
            "Invalid session tool name: '{}' (expected 'session.command')",
            name
        ));
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

#[cfg(feature = "toolclad-session")]
fn read_until_prompt_blocking(
    pty: &mut Pty,
    pattern: &str,
    timeout: Duration,
    max_bytes: usize,
) -> Result<(String, String), String> {
    let re = regex::Regex::new(pattern)
        .map_err(|e| format!("Invalid ready pattern '{}': {}", pattern, e))?;

    let start = Instant::now();
    let mut buffer = Vec::new();
    let mut byte = [0u8; 1024];

    loop {
        if start.elapsed() > timeout {
            let partial = String::from_utf8_lossy(&buffer).to_string();
            return Err(format!(
                "Timeout waiting for prompt pattern '{}'. Got: {}",
                pattern,
                &partial[..partial.len().min(200)]
            ));
        }
        if buffer.len() > max_bytes {
            return Err("Output exceeded max bytes".to_string());
        }

        match pty.read(&mut byte) {
            Ok(0) => break,
            Ok(n) => {
                buffer.extend_from_slice(&byte[..n]);
                let text = String::from_utf8_lossy(&buffer);
                // Check if prompt pattern appears at the end
                for line in text.lines().rev().take(3) {
                    if re.is_match(line.trim()) {
                        let output = text.to_string();
                        let prompt = line.trim().to_string();
                        return Ok((output, prompt));
                    }
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(format!("PTY read error: {}", e)),
        }
    }

    let text = String::from_utf8_lossy(&buffer).to_string();
    Err(format!(
        "PTY closed before prompt. Got: {}",
        &text[..text.len().min(200)]
    ))
}

/// Strip ANSI escape sequences.
#[cfg(any(feature = "toolclad-session", test))]
fn strip_ansi(input: &str) -> String {
    let re = regex::Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]").unwrap();
    re.replace_all(input, "").to_string()
}

/// Infer session state from prompt text.
#[cfg(any(feature = "toolclad-session", test))]
fn infer_state(prompt: &str) -> String {
    let lower = prompt.to_lowercase();
    if lower.contains("error") {
        "error".to_string()
    } else {
        "ready".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_session_tool_name() {
        let (base, cmd) = parse_session_tool_name("psql_session.select").unwrap();
        assert_eq!(base, "psql_session");
        assert_eq!(cmd, "select");
    }

    #[test]
    fn test_parse_session_tool_name_invalid() {
        assert!(parse_session_tool_name("no_dot").is_err());
    }

    #[test]
    fn test_strip_ansi() {
        assert_eq!(strip_ansi("\x1b[32mhello\x1b[0m"), "hello");
        assert_eq!(strip_ansi("no escapes"), "no escapes");
    }

    #[test]
    fn test_infer_state() {
        assert_eq!(infer_state("dbname=> "), "ready");
        assert_eq!(infer_state("ERROR: "), "error");
    }

    #[test]
    fn test_session_executor_handles() {
        let manifest_toml = r#"
[tool]
name = "test_session"
mode = "session"
version = "1.0.0"
description = "Test"

[session]
startup_command = "cat"
ready_pattern = "^$"

[session.commands.echo]
pattern = "^echo .+$"
description = "Echo text"

[output]
format = "text"

[output.schema]
type = "object"
"#;
        let manifest: Manifest = toml::from_str(manifest_toml).unwrap();
        let executor = SessionExecutor::new(vec![("test_session".to_string(), manifest)]);

        assert!(executor.handles("test_session.echo"));
        assert!(!executor.handles("test_session.unknown"));
        assert!(!executor.handles("other_tool"));
    }

    #[test]
    fn test_command_pattern_validation() {
        let re = regex::Regex::new("^SELECT .+$").unwrap();
        assert!(re.is_match("SELECT * FROM users"));
        assert!(!re.is_match("DROP TABLE users"));
    }

    #[test]
    fn test_transcript() {
        let mut t = SessionTranscript::default();
        t.append(TranscriptDirection::Command, "SELECT 1", Some("select"));
        t.append(TranscriptDirection::Response, "1\n(1 row)", Some("select"));
        assert_eq!(t.entries.len(), 2);
        assert!(matches!(
            t.entries[0].direction,
            TranscriptDirection::Command
        ));
    }
}
