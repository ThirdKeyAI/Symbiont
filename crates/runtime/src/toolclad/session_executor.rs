//! Interactive tools with a PTY allocated inside the selected sandbox.

use super::{manifest::Manifest, session_state::SessionTranscript};
use crate::sandbox::command::CommandBoundary;
use crate::sandbox::files::FileAccessPlan;
use std::{collections::HashMap, time::Instant};

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
    // Read only by the ungoverned fixture path.
    #[cfg_attr(not(any(test, feature = "toolclad-direct")), allow(dead_code))]
    boundary: Result<CommandBoundary, String>,
    owner: String,
    #[cfg_attr(not(any(test, feature = "toolclad-direct")), allow(dead_code))]
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
    #[cfg(any(test, feature = "toolclad-direct"))]
    pub fn execute_session_command(
        &self,
        tool: &str,
        args: &str,
    ) -> Result<serde_json::Value, String> {
        self.validate_direct(tool, args)?;
        Err("interactive sessions require execute_session_command_async".into())
    }

    /// Ungoverned fixture dispatch: no policy gate or approval relay. Compiled
    /// only for unit tests and the `toolclad-direct` feature, and refused in
    /// production.
    #[cfg(any(test, feature = "toolclad-direct"))]
    pub async fn execute_session_command_async(
        &self,
        tool: &str,
        args: &str,
    ) -> Result<serde_json::Value, String> {
        super::executor::refuse_direct_dispatch_in_production()?;
        let (manifest, values) = self.validate_direct(tool, args)?;
        let ceiling = self.boundary.as_ref().map_err(Clone::clone)?;
        let files = super::executor::ToolCladExecutor::prepare_files(manifest, ceiling, &values)?;
        let mut boundary = ceiling.without_host_mounts();
        if !manifest.tool.network_egress {
            boundary = boundary.without_network();
        }
        let contract = crate::reasoning::prepared::digest_json(&serde_json::json!({
            "manifest": manifest, "boundary": boundary.descriptor()?, "filesystem": files.descriptor(),
            "mount_ceiling": ceiling.descriptor()?
        }))?;
        let now = Instant::now();
        let deadline = now
            .checked_add(std::time::Duration::from_secs(
                manifest.tool.timeout_seconds,
            ))
            .ok_or("session command deadline exceeds supported range")?;
        let run_deadline = self.sdk_run_deadline(std::time::Duration::from_secs(
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

    #[cfg(any(test, feature = "toolclad-direct"))]
    pub(super) fn sdk_run_deadline(
        &self,
        lifetime: std::time::Duration,
    ) -> Result<Instant, String> {
        let candidate = Instant::now()
            .checked_add(lifetime)
            .ok_or("SDK session lifetime exceeds supported range")?;
        let deadline = *self.owner_deadline.get_or_init(|| candidate);
        if Instant::now() >= deadline {
            return Err("SDK execution run is closed; create a new session executor".into());
        }
        Ok(deadline)
    }

    #[cfg(any(test, feature = "toolclad-direct"))]
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
        validate_session_command(manifest, definition, input)?;
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

/// Console verbs refused in msfconsole-style sessions: each loads or
/// evaluates code, reads or writes host files, or drops to a shell.
pub const MSF_DENIED_VERBS: &[&str] = &[
    "irb",
    "pry",
    "load",
    "unload",
    "resource",
    "makerc",
    "edit",
    "spool",
    "save",
    "cd",
    "ruby",
    "sh",
    "shell",
    "bash",
    "exec",
    "loadpath",
    "reload_all",
    "debug",
];

/// Shell metacharacters refused when a session opts into `injection`.
const SESSION_SHELL_METACHARACTERS: &[char] = &[
    ';', '|', '&', '$', '`', '(', ')', '{', '}', '[', ']', '<', '>', '!',
];

/// The input policies that apply to a session: those inferred from the
/// startup binary (they cannot be switched off) plus the manifest's
/// `[session.interaction] input_sanitize` values.
pub fn session_sinks(manifest: &Manifest) -> Vec<&'static str> {
    let mut sinks = Vec::new();
    match manifest.session_binary().as_deref() {
        Some("psql") => sinks.push("psql_meta"),
        Some("msfconsole") => sinks.push("msf_meta"),
        _ => {}
    }
    if let Some(interaction) = manifest
        .session
        .as_ref()
        .and_then(|s| s.interaction.as_ref())
    {
        for value in &interaction.input_sanitize {
            if let Some(known) = super::manifest::SESSION_SANITIZERS
                .iter()
                .find(|k| **k == value.as_str())
            {
                if !sinks.contains(known) {
                    sinks.push(*known);
                }
            }
        }
    }
    sinks
}

/// The single validation function for session input, used by the governed
/// path and the direct fixture path alike (ToolClad S1 plus sink policy):
///
/// * the input is one line: no C0 control (tab, CR and LF included), DEL,
///   C1 control, or U+2028/U+2029; the executor appends the terminator;
/// * the declared pattern must match the whole input;
/// * `psql_meta`: no backslash anywhere, so `\gexec`, `\c host`, `\copy`,
///   `\lo_import`, `\o` and `\!` cannot be expressed, inside or after a query;
/// * `msf_meta`: no `;` command chaining and no denied console verb;
/// * `injection`: the shell-metacharacter blocklist, only when declared.
///
/// The global shell blocklist is deliberately not applied otherwise: SQL
/// such as `count(*)`, `a != b` or `$1` is legitimate session input.
pub fn validate_session_command(
    manifest: &Manifest,
    command: &super::manifest::SessionCommandDef,
    input: &str,
) -> Result<(), String> {
    if input.is_empty() || input.len() > 64 * 1024 {
        return Err("terminal command is empty or exceeds 64 KiB".into());
    }
    if input.chars().any(|c| {
        let cp = c as u32;
        cp < 0x20 || (0x7f..=0x9f).contains(&cp) || c == '\u{2028}' || c == '\u{2029}'
    }) {
        return Err("terminal command contains control characters".into());
    }
    let pattern = regex::Regex::new(&format!(r"\A(?:{})\z", command.pattern))
        .map_err(|e| format!("invalid session command pattern: {e}"))?;
    if !pattern.is_match(input) {
        return Err("terminal command does not fully match its declared pattern".into());
    }
    for sink in session_sinks(manifest) {
        match sink {
            "psql_meta" if input.contains('\\') => {
                return Err(
                    "psql meta-commands (backslash) are not permitted in session input".into(),
                )
            }
            "msf_meta" => {
                if input.contains(';') {
                    return Err("console command chaining (;) is not permitted".into());
                }
                let verb = input
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim_start_matches('!')
                    .to_ascii_lowercase();
                if input.trim_start().starts_with('!') || MSF_DENIED_VERBS.contains(&verb.as_str())
                {
                    return Err(format!("console verb '{verb}' is not permitted"));
                }
            }
            "injection" => {
                if let Some(c) = input
                    .chars()
                    .find(|c| SESSION_SHELL_METACHARACTERS.contains(c))
                {
                    return Err(format!(
                        "terminal command contains forbidden character '{c}'"
                    ));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Look up a session command and validate input against it.
#[cfg(any(test, feature = "toolclad-session"))]
pub(super) fn validate_named_session_command(
    manifest: &Manifest,
    command_name: &str,
    input: &str,
) -> Result<(), String> {
    let command = manifest
        .session
        .as_ref()
        .and_then(|s| s.commands.get(command_name))
        .ok_or("unknown session command")?;
    validate_session_command(manifest, command, input)
}

fn parse_session_tool_name(name: &str) -> Result<(String, String), String> {
    name.split_once('.')
        .filter(|(base, cmd)| !base.is_empty() && !cmd.is_empty())
        .map(|(base, cmd)| (base.to_owned(), cmd.to_owned()))
        .ok_or_else(|| format!("Invalid session tool name: '{name}' (expected 'session.command')"))
}

/// Remove every terminal control sequence the far side could use to
/// rewrite an operator's display: CSI (`ESC [` and 8-bit `0x9B`), OSC, DCS,
/// SOS, PM and APC strings (terminated by BEL or ST), two-byte escapes, and
/// every remaining C0/C1 control except newline and tab. `\r\n` becomes
/// `\n`; a lone `\r` is dropped.
#[cfg(any(feature = "toolclad-session", test))]
fn strip_ansi(input: &str) -> String {
    #[derive(PartialEq)]
    enum State {
        Text,
        Escape,
        Csi,
        String,
        StringEscape,
    }
    let mut out = String::with_capacity(input.len());
    let mut state = State::Text;
    for c in input.chars() {
        let cp = c as u32;
        state = match state {
            State::Text => match c {
                '\u{1b}' => State::Escape,
                '\u{9b}' => State::Csi,
                '\u{90}' | '\u{98}' | '\u{9d}' | '\u{9e}' | '\u{9f}' => State::String,
                '\n' | '\t' => {
                    out.push(c);
                    State::Text
                }
                _ if cp < 0x20 || (0x7f..=0x9f).contains(&cp) => State::Text,
                _ => {
                    out.push(c);
                    State::Text
                }
            },
            State::Escape => match c {
                '[' => State::Csi,
                ']' | 'P' | 'X' | '^' | '_' => State::String,
                // Intermediate bytes (e.g. `ESC ( B`) take one more byte.
                ' '..='/' => State::Escape,
                _ => State::Text,
            },
            State::Csi => {
                if ('\u{40}'..='\u{7e}').contains(&c) {
                    State::Text
                } else {
                    State::Csi
                }
            }
            State::String => match c {
                '\u{7}' | '\u{9c}' => State::Text,
                '\u{1b}' => State::StringEscape,
                _ => State::String,
            },
            State::StringEscape => {
                if c == '\\' {
                    State::Text
                } else {
                    State::String
                }
            }
        };
    }
    out
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
        // OSC title rewrite (BEL and ST terminated), DCS, 8-bit CSI, C0/C1.
        assert_eq!(strip_ansi("a\x1b]0;evil\x07b"), "ab");
        assert_eq!(strip_ansi("a\x1b]8;;http://x\x1b\\b"), "ab");
        assert_eq!(strip_ansi("a\x1bPq#0;1\x1b\\b"), "ab");
        assert_eq!(strip_ansi("a\u{9b}31mb"), "ab");
        assert_eq!(strip_ansi("a\x1b(Bb\r\nc\x08\u{85}d"), "ab\ncd");
    }

    fn session_manifest(binary: &str, pattern: &str, sanitize: &str) -> Manifest {
        crate::toolclad::manifest::parse_manifest(
            &format!(
                r#"
[tool]
name = "s"
version = "1"
description = "d"
mode = "session"
human_approval = true
[session]
startup_command = "{binary}"
ready_pattern = "db=> "
[session.interaction]
input_sanitize = [{sanitize}]
[session.commands.run]
pattern = "{pattern}"
description = "q"
[output]
format = "text"
"#
            ),
            &HashMap::new(),
        )
        .unwrap()
    }

    #[test]
    fn psql_meta_commands_are_refused_and_sql_is_not_over_blocked() {
        let m = session_manifest("psql -X", "SELECT .+", "");
        for bad in [
            "SELECT 1 \\gexec",
            "SELECT 1;\\c evil.example",
            "SELECT 1 \\copy t to '/tmp/x'",
            "SELECT \\lo_import('/etc/passwd')",
            "SELECT 1 \\o /tmp/out",
            "SELECT 1\tx",
            "SELECT 1\u{2028}DROP TABLE t",
        ] {
            assert!(
                validate_named_session_command(&m, "run", bad).is_err(),
                "{bad:?}"
            );
        }
        for good in [
            "SELECT count(*) FROM t",
            "SELECT * FROM t WHERE a != b AND c < 3",
            "SELECT $1",
        ] {
            assert!(
                validate_named_session_command(&m, "run", good).is_ok(),
                "{good}"
            );
        }
    }

    #[test]
    fn msf_console_verbs_and_chaining_are_refused() {
        let m = session_manifest("msfconsole -q", ".+", "");
        for bad in [
            "irb",
            "load evil",
            "resource /tmp/x.rc",
            "use x; irb",
            "!id",
            "makerc /tmp/x",
        ] {
            assert!(
                validate_named_session_command(&m, "run", bad).is_err(),
                "{bad}"
            );
        }
        assert!(validate_named_session_command(&m, "run", "use exploit/x").is_ok());
    }

    #[test]
    fn injection_sanitizer_is_opt_in() {
        let plain = session_manifest("cat", ".+", "");
        assert!(validate_named_session_command(&plain, "run", "a | b").is_ok());
        let strict = session_manifest("cat", ".+", "\"injection\"");
        assert!(validate_named_session_command(&strict, "run", "a | b").is_err());
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
        manifest.validate(&HashMap::new()).unwrap();
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
