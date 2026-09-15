//! Interactive tools with a PTY allocated inside the selected sandbox.

use super::{manifest::Manifest, session_state::SessionTranscript};
use crate::sandbox::command::CommandBoundary;
use crate::sandbox::files::FileAccessPlan;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

#[cfg(feature = "toolclad-session")]
#[path = "session_worker.rs"]
mod worker;

/// An executor-owned call. Model arguments cannot choose its session owner,
/// contract, sandbox, or deadlines.
pub(super) struct SessionCall<'a> {
    pub tool: &'a str,
    pub manifest: &'a Manifest,
    pub command: &'a str,
    pub boundary: &'a CommandBoundary,
    pub files: &'a FileAccessPlan,
    pub contract: &'a str,
    pub run: &'a str,
    pub binding: &'a str,
    pub run_deadline: Instant,
    pub deadline: Instant,
}

pub struct SessionExecutor {
    manifests: HashMap<String, Manifest>,
    boundary: Result<CommandBoundary, String>,
    owner: String,
    owner_deadline: std::sync::OnceLock<Instant>,
    #[cfg(feature = "toolclad-session")]
    worker: worker::SessionManager,
}

impl SessionExecutor {
    pub fn new(manifests: Vec<(String, Manifest)>) -> Self {
        Self {
            manifests: manifests
                .into_iter()
                .filter(|(_, m)| m.tool.mode == "session")
                .collect(),
            boundary: Ok(CommandBoundary::default()),
            owner: format!("sdk-session-{}", uuid::Uuid::new_v4()),
            owner_deadline: std::sync::OnceLock::new(),
            #[cfg(feature = "toolclad-session")]
            worker: worker::SessionManager::default(),
        }
    }

    pub fn with_command_boundary(mut self, boundary: CommandBoundary) -> Self {
        self.cleanup();
        #[cfg(feature = "toolclad-session")]
        {
            self.worker = worker::SessionManager::default();
        }
        self.owner = format!("sdk-session-{}", uuid::Uuid::new_v4());
        self.owner_deadline = std::sync::OnceLock::new();
        self.boundary = boundary.validate().map(|()| boundary);
        self
    }

    pub fn handles(&self, tool_name: &str) -> bool {
        parse_session_tool_name(tool_name)
            .ok()
            .is_some_and(|(base, command)| {
                self.manifests
                    .get(&base)
                    .and_then(|m| m.session.as_ref())
                    .is_some_and(|s| s.commands.contains_key(&command))
            })
    }

    /// Persistent sessions require the async API and a live Tokio runtime.
    pub fn execute_session_command(
        &self,
        tool: &str,
        args: &str,
    ) -> Result<serde_json::Value, String> {
        self.validate_direct(tool, args)?;
        Err("interactive sessions require execute_session_command_async".into())
    }

    pub async fn execute_session_command_async(
        &self,
        tool: &str,
        args: &str,
    ) -> Result<serde_json::Value, String> {
        let (manifest, values) = self.validate_direct(tool, args)?;
        let ceiling = self.boundary.as_ref().map_err(Clone::clone)?;
        let files = super::executor::ToolCladExecutor::prepare_files(manifest, ceiling, &values)?;
        let boundary = ceiling.without_host_mounts();
        let contract = crate::reasoning::prepared::digest_json(&serde_json::json!({
            "manifest": manifest, "boundary": boundary.descriptor()?, "filesystem": files.descriptor(),
            "mount_ceiling": ceiling.descriptor()?
        }))?;
        let now = Instant::now();
        let deadline = now
            .checked_add(Duration::from_secs(manifest.tool.timeout_seconds))
            .ok_or("session command deadline exceeds supported range")?;
        let run_deadline = self.sdk_run_deadline(Duration::from_secs(
            manifest.session.as_ref().unwrap().session_timeout_seconds,
        ))?;
        self.execute_prepared(SessionCall {
            tool,
            manifest,
            command: &values["command"],
            boundary: &boundary,
            files: &files,
            contract: &contract,
            run: &self.owner,
            binding: &self.owner,
            run_deadline,
            deadline,
        })
        .await
    }

    pub(super) fn sdk_run_deadline(&self, lifetime: Duration) -> Result<Instant, String> {
        let candidate = Instant::now()
            .checked_add(lifetime)
            .ok_or("SDK session lifetime exceeds supported range")?;
        let deadline = *self.owner_deadline.get_or_init(|| candidate);
        if Instant::now() >= deadline {
            return Err("SDK execution run is closed; create a new session executor".into());
        }
        Ok(deadline)
    }

    fn validate_direct(
        &self,
        tool: &str,
        args: &str,
    ) -> Result<(&Manifest, HashMap<String, String>), String> {
        let (base, command) = parse_session_tool_name(tool)?;
        let manifest = self
            .manifests
            .get(&base)
            .ok_or("unknown session manifest")?;
        let definition = manifest
            .session
            .as_ref()
            .and_then(|s| s.commands.get(&command))
            .ok_or("unknown session command")?;
        if manifest.tool.human_approval || definition.human_approval {
            return Err("command requires an authorized exact-call approval".into());
        }
        if definition.extract_target {
            return Err("scoped session commands require governed ToolClad dispatch".into());
        }
        if args.len() > 128 * 1024 {
            return Err("session arguments exceed input limit".into());
        }
        let values: HashMap<String, serde_json::Value> =
            serde_json::from_str(args).map_err(|e| e.to_string())?;
        if values
            .keys()
            .any(|k| k != "command" && !definition.args.contains_key(k))
        {
            return Err("unknown session argument".into());
        }
        let mut normalized = HashMap::new();
        for (name, def) in &definition.args {
            if name == "command" {
                continue;
            }
            if def.required && !values.contains_key(name) {
                return Err(format!("missing required argument: {name}"));
            }
            if let Some(text) = values
                .get(name)
                .map(|value| {
                    value
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| value.to_string())
                })
                .or_else(|| {
                    def.default.as_ref().map(|value| {
                        value
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| value.to_string())
                    })
                })
            {
                if super::validator::requires_scope(def, None)? {
                    return Err("scoped arguments require governed ToolClad dispatch".into());
                }
                let value =
                    super::validator::validate_arg(def, &text).map_err(|e| e.to_string())?;
                normalized.insert(name.clone(), value);
            } else if def.required {
                return Err(format!("missing required argument: {name}"));
            }
        }
        let input = values
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or("session command requires a string 'command' argument")?;
        validate_terminal_input(input, &definition.pattern)?;
        normalized.insert("command".into(), input.to_owned());
        Ok((manifest, normalized))
    }

    pub(super) async fn execute_prepared(
        &self,
        call: SessionCall<'_>,
    ) -> Result<serde_json::Value, String> {
        #[cfg(feature = "toolclad-session")]
        {
            self.worker.execute(call).await
        }
        #[cfg(not(feature = "toolclad-session"))]
        {
            let SessionCall {
                tool,
                manifest,
                command,
                boundary,
                files,
                contract,
                run,
                binding,
                run_deadline,
                deadline,
            } = call;
            let _ = (
                tool,
                manifest,
                command,
                boundary,
                files,
                contract,
                run,
                binding,
                run_deadline,
                deadline,
            );
            Err("Session mode requires the 'toolclad-session' feature".into())
        }
    }

    pub(super) fn cancel_run(&self, run: &str, deadline: Instant) {
        #[cfg(feature = "toolclad-session")]
        self.worker.cancel_run(run, deadline);
        #[cfg(not(feature = "toolclad-session"))]
        let _ = (run, deadline);
    }

    pub(super) async fn close_run(&self, run: &str, deadline: Instant) -> Result<(), String> {
        #[cfg(feature = "toolclad-session")]
        {
            self.worker.close_run(run, deadline).await
        }
        #[cfg(not(feature = "toolclad-session"))]
        {
            let _ = (run, deadline);
            Ok(())
        }
    }

    pub fn get_transcript(&self, manifest: &str) -> Option<SessionTranscript> {
        #[cfg(feature = "toolclad-session")]
        {
            self.worker.transcript(&self.owner, manifest)
        }
        #[cfg(not(feature = "toolclad-session"))]
        {
            let _ = manifest;
            None
        }
    }

    /// Signal every worker immediately. Drop has the same bounded cleanup path.
    pub fn cleanup(&self) {
        #[cfg(feature = "toolclad-session")]
        self.worker.cancel_all();
    }

    pub async fn cleanup_async(&self) -> Result<(), String> {
        #[cfg(feature = "toolclad-session")]
        {
            self.worker.close_all().await
        }
        #[cfg(not(feature = "toolclad-session"))]
        {
            Ok(())
        }
    }
}

pub(super) fn validate_terminal_input(command: &str, pattern: &str) -> Result<(), String> {
    if command.is_empty() || command.len() > 64 * 1024 || command.chars().any(char::is_control) {
        return Err(
            "terminal command is empty, contains control characters, or exceeds 64 KiB".into(),
        );
    }
    let pattern = regex::Regex::new(&format!(r"\A(?:{pattern})\z"))
        .map_err(|e| format!("invalid session command pattern: {e}"))?;
    if !pattern.is_match(command) {
        return Err("terminal command does not fully match its declared pattern".into());
    }
    Ok(())
}

fn parse_session_tool_name(name: &str) -> Result<(String, String), String> {
    name.split_once('.')
        .filter(|(base, cmd)| !base.is_empty() && !cmd.is_empty())
        .map(|(base, cmd)| (base.to_owned(), cmd.to_owned()))
        .ok_or_else(|| format!("Invalid session tool name: '{name}' (expected 'session.command')"))
}

#[cfg(any(feature = "toolclad-session", test))]
fn strip_ansi(input: &str) -> String {
    let re = regex::Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]").unwrap();
    re.replace_all(input, "").to_string()
}

#[cfg(any(feature = "toolclad-session", test))]
fn infer_state(prompt: &str) -> String {
    if prompt.to_lowercase().contains("error") {
        "error".into()
    } else {
        "ready".into()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolclad::session_state::TranscriptDirection;

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
