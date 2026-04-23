//! Shell session persistence.
//!
//! Saves and restores:
//! - the visible transcript (`output`)
//! - input history for up-arrow recall
//! - the orchestrator's full `Conversation` (so `/resume` actually
//!   restores the model's memory instead of just the displayed text)
//! - session metadata (mode, model, token count, timestamps)
//!
//! Sessions are stored as JSON files in
//! `$SYMBIONT_SESSION_DIR` (default: `$HOME/.symbi/sessions`), falling
//! back to `./.symbi/sessions` only when the home dir can't be
//! resolved. Writes are atomic (tempfile + rename) and, on Unix, the
//! file is chmodded to 0o600 so transcripts containing prompts /
//! responses aren't world-readable.
//!
//! Every shell run has a session UUID. On clean exit the shell
//! auto-saves to `<uuid>.json` and prints a `--resume` hint. Named
//! sessions (via `/snapshot foo`) live alongside the UUIDs in the same
//! directory.

use crate::app::{EntrySource, OutputEntry};
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

/// Current on-disk schema version. Bump when the `ShellSession`
/// structure changes in a non-backwards-compatible way.
pub const SESSION_SCHEMA_VERSION: u32 = 2;

/// Default name used by `/snapshot` with no argument — overwritten on
/// each call. Users pick their own names with `/snapshot foo`.
pub const DEFAULT_SNAPSHOT_NAME: &str = "main";

/// Env override for the sessions directory.
pub const SESSION_DIR_ENV: &str = "SYMBIONT_SESSION_DIR";

/// Fallback name used when the home directory can't be resolved.
const FALLBACK_SESSION_DIR: &str = ".symbi/sessions";

/// Serializable session state.
///
/// Older session files (`version` missing) still deserialise — the
/// `#[serde(default)]` attributes on new fields keep pre-v2 files
/// loadable, they just come back without orchestrator memory.
#[derive(Serialize, Deserialize)]
pub struct ShellSession {
    #[serde(default = "default_version")]
    pub version: u32,
    pub name: String,
    #[serde(default)]
    pub session_id: String,
    pub timestamp: String,
    pub mode: String,
    #[serde(default)]
    pub model_name: Option<String>,
    pub output: Vec<SerializedEntry>,
    pub input_history: Vec<String>,
    pub tokens_used: u64,
    /// Orchestrator's full conversation, serialised. When present,
    /// `/resume` restores the model's memory; when `None` (older files
    /// or sessions without an orchestrator), only the visible transcript
    /// is restored.
    #[serde(default)]
    pub conversation: Option<serde_json::Value>,
}

fn default_version() -> u32 {
    1
}

/// Serializable output entry.
#[derive(Serialize, Deserialize)]
pub struct SerializedEntry {
    pub source: String,
    pub source_name: Option<String>,
    pub content: String,
}

impl From<&OutputEntry> for SerializedEntry {
    fn from(entry: &OutputEntry) -> Self {
        let (source, source_name, content) = match &entry.source {
            EntrySource::User => ("user".to_string(), None, entry.content.clone()),
            EntrySource::System => ("system".to_string(), None, entry.content.clone()),
            EntrySource::Agent(name) => (
                "agent".to_string(),
                Some(name.clone()),
                entry.content.clone(),
            ),
            EntrySource::Error => ("error".to_string(), None, entry.content.clone()),
            EntrySource::Meta => ("meta".to_string(), None, entry.content.clone()),
            EntrySource::ToolCall(card) => {
                // Flatten to a compact text summary so saved transcripts
                // still convey which tools ran. Cards are never
                // reconstructed from a saved session — they're
                // derived from the live journal stream.
                let content = format!(
                    "{}({}) — {}{}",
                    card.name,
                    card.args_summary,
                    if card.is_error { "ERROR: " } else { "" },
                    card.output.lines().next().unwrap_or("")
                );
                ("tool".to_string(), Some(card.name.clone()), content)
            }
            EntrySource::Notice { kind, source_label } => {
                let kind_str = match kind {
                    crate::app::NoticeKind::Info => "notice-info",
                    crate::app::NoticeKind::Success => "notice-ok",
                    crate::app::NoticeKind::Warning => "notice-warn",
                    crate::app::NoticeKind::Error => "notice-err",
                };
                (
                    kind_str.to_string(),
                    Some(source_label.clone()),
                    entry.content.clone(),
                )
            }
        };
        Self {
            source,
            source_name,
            content,
        }
    }
}

impl SerializedEntry {
    pub fn to_output_entry(&self) -> OutputEntry {
        let source = match self.source.as_str() {
            "user" => EntrySource::User,
            "agent" => EntrySource::Agent(
                self.source_name
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
            ),
            "error" => EntrySource::Error,
            "meta" => EntrySource::Meta,
            // Saved "tool" entries are flat text — render as System
            // lines on restore rather than reconstructing a ToolCall
            // card (the raw per-tool output isn't in the file).
            "tool" => EntrySource::System,
            // Notices keep their severity through a restore.
            s if s.starts_with("notice-") => {
                let kind = match s {
                    "notice-ok" => crate::app::NoticeKind::Success,
                    "notice-warn" => crate::app::NoticeKind::Warning,
                    "notice-err" => crate::app::NoticeKind::Error,
                    _ => crate::app::NoticeKind::Info,
                };
                EntrySource::Notice {
                    kind,
                    source_label: self
                        .source_name
                        .clone()
                        .unwrap_or_else(|| "runtime".to_string()),
                }
            }
            _ => EntrySource::System,
        };
        OutputEntry {
            source,
            content: self.content.clone(),
        }
    }
}

/// Resolve the sessions directory for the current run.
///
/// Precedence: `$SYMBIONT_SESSION_DIR` → `$HOME/.symbi/sessions` →
/// `./.symbi/sessions` (last-resort fallback for environments without a
/// home directory, e.g. some CI sandboxes).
pub fn session_dir() -> PathBuf {
    if let Ok(custom) = std::env::var(SESSION_DIR_ENV) {
        return PathBuf::from(custom);
    }
    if let Some(mut home) = dirs::home_dir() {
        home.push(".symbi");
        home.push("sessions");
        return home;
    }
    PathBuf::from(FALLBACK_SESSION_DIR)
}

/// Save a session to disk atomically.
///
/// Writes via tempfile-in-same-dir + `rename` so a crash mid-write
/// doesn't leave a truncated file. On Unix the resulting file is
/// chmodded to 0o600 — transcripts often contain prompts / responses
/// and should not be world-readable.
pub fn save_session(name: &str, session: &ShellSession) -> Result<PathBuf> {
    let dir = session_dir();
    std::fs::create_dir_all(&dir)?;

    let safe = sanitize_name(name)?;
    let path = dir.join(format!("{}.json", safe));

    let json = serde_json::to_string_pretty(session)?;
    let tmp = tempfile::NamedTempFile::new_in(&dir)?;
    {
        use std::io::Write;
        let mut file = tmp.as_file();
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
    }
    tmp.persist(&path)
        .map_err(|e| anyhow!("failed to persist session file: {}", e.error))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)) {
            tracing::warn!(
                "Failed to set 0o600 permissions on {}: {}",
                path.display(),
                e
            );
        }
    }

    Ok(path)
}

/// Load a session from disk by name OR by UUID — both look up the same
/// directory and the same extension.
pub fn load_session(name: &str) -> Result<ShellSession> {
    let safe = sanitize_name(name)?;
    let dir = session_dir();
    let path = dir.join(format!("{}.json", safe));

    if !path.exists() {
        return Err(anyhow!(
            "Session '{}' not found at {}",
            name,
            path.display()
        ));
    }

    let json = std::fs::read_to_string(&path)?;
    let session: ShellSession = serde_json::from_str(&json)?;
    Ok(session)
}

/// List available sessions.
pub fn list_sessions() -> Result<Vec<String>> {
    Ok(list_sessions_with_metadata()?
        .into_iter()
        .map(|s| s.name)
        .collect())
}

/// A session file on disk plus the metadata needed to filter it.
#[derive(Clone)]
pub struct SessionInfo {
    pub name: String,
    pub path: PathBuf,
    pub modified: SystemTime,
}

/// List sessions with each file's modified timestamp, sorted by name.
///
/// Returns an empty Vec when the sessions directory doesn't exist yet;
/// that's the normal "fresh install" state, not an error.
pub fn list_sessions_with_metadata() -> Result<Vec<SessionInfo>> {
    let dir = session_dir();
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut sessions = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            if let Some(stem) = path.file_stem() {
                let name = stem.to_string_lossy().to_string();
                let modified = entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                sessions.push(SessionInfo {
                    name,
                    path,
                    modified,
                });
            }
        }
    }
    sessions.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(sessions)
}

/// Outcome of a `cleanup_sessions` call.
pub struct CleanupReport {
    /// Session names that were removed (or would be removed, in dry-run).
    pub removed: Vec<String>,
    /// Number of sessions skipped because they were newer than the cutoff.
    pub kept: usize,
    /// Whether any disk writes actually happened.
    pub dry_run: bool,
}

/// Delete session files older than `older_than` from the sessions dir.
///
/// With `dry_run = true` the function reports what *would* be removed
/// without touching disk. A session's age is its file's modified time
/// (matching how the shell rewrites files on every save).
pub fn cleanup_sessions(older_than: Duration, dry_run: bool) -> Result<CleanupReport> {
    let now = SystemTime::now();
    let cutoff = now
        .checked_sub(older_than)
        .ok_or_else(|| anyhow!("cutoff underflow — duration too large"))?;

    let sessions = list_sessions_with_metadata()?;
    let mut removed = Vec::new();
    let mut kept = 0usize;
    for info in sessions {
        if info.modified <= cutoff {
            if !dry_run {
                std::fs::remove_file(&info.path)
                    .map_err(|e| anyhow!("failed to remove {}: {}", info.path.display(), e))?;
            }
            removed.push(info.name);
        } else {
            kept += 1;
        }
    }
    Ok(CleanupReport {
        removed,
        kept,
        dry_run,
    })
}

/// Parse a human-readable duration like `30d`, `12h`, `90m`, `45s`.
///
/// Accepts suffixes `s` (seconds), `m` (minutes), `h` (hours), `d` (days).
/// A bare integer with no suffix is rejected so `--older-than 7` doesn't
/// silently mean "7 seconds" when the user meant days.
pub fn parse_duration(s: &str) -> Result<Duration> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("empty duration"));
    }
    let (num_part, unit) = trimmed.split_at(trimmed.len() - 1);
    let unit_char = unit
        .chars()
        .next()
        .ok_or_else(|| anyhow!("duration missing unit"))?;
    let multiplier: u64 = match unit_char {
        's' => 1,
        'm' => 60,
        'h' => 60 * 60,
        'd' => 24 * 60 * 60,
        _ => {
            return Err(anyhow!(
                "duration '{}' missing unit suffix (expected one of s/m/h/d)",
                s
            ));
        }
    };
    let n: u64 = num_part
        .parse()
        .map_err(|_| anyhow!("duration '{}' has non-numeric amount", s))?;
    Ok(Duration::from_secs(n * multiplier))
}

/// Does a session with this name / UUID exist on disk? Used by CLI
/// argument validation paths; not a security check.
#[allow(dead_code)]
pub fn session_exists(name: &str) -> bool {
    match sanitize_name(name) {
        Ok(safe) => session_dir().join(format!("{}.json", safe)).exists(),
        Err(_) => false,
    }
}

/// Export session as plain text.
pub fn export_session(session: &ShellSession) -> String {
    let mut out = format!(
        "# symbi shell session: {}\n# {}\n# tokens: {}\n\n",
        session.name, session.timestamp, session.tokens_used
    );

    for entry in &session.output {
        let prefix = match entry.source.as_str() {
            "user" => "you: ",
            "agent" => {
                if let Some(ref name) = entry.source_name {
                    // Can't return a reference to a local, so push inline
                    out.push_str(&format!("{}: {}\n", name, entry.content));
                    continue;
                }
                "agent: "
            }
            "error" => "err: ",
            "meta" => "    ",
            _ => "sys: ",
        };
        out.push_str(&format!("{}{}\n", prefix, entry.content));
    }

    out
}

/// Sanitise a session name into a safe filename stem.
///
/// Only alphanumerics, `-`, and `_` survive; everything else becomes
/// `_`. Empty / whitespace-only names are rejected outright rather than
/// silently producing a `.json` hidden file. The maximum length is
/// capped so no caller can create a filename that blows past filesystem
/// limits.
fn sanitize_name(name: &str) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("session name cannot be empty"));
    }
    if trimmed == "." || trimmed == ".." {
        return Err(anyhow!("session name '{}' is reserved", trimmed));
    }
    let cleaned: String = trimmed
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    // Belt-and-braces: sanitised output must still be non-empty after
    // stripping the surviving separators.
    if cleaned.chars().all(|c| c == '_' || c == '-') {
        return Err(anyhow!(
            "session name '{}' contains no filename-safe characters",
            name
        ));
    }
    if cleaned.len() > 128 {
        return Err(anyhow!(
            "session name too long (>128 chars after sanitisation)"
        ));
    }
    Ok(cleaned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialized_entry_roundtrip() {
        let entry = OutputEntry {
            source: EntrySource::Agent("orchestrator".to_string()),
            content: "Hello world".to_string(),
        };
        let serialized = SerializedEntry::from(&entry);
        let restored = serialized.to_output_entry();
        assert_eq!(restored.content, "Hello world");
        assert_eq!(
            restored.source,
            EntrySource::Agent("orchestrator".to_string())
        );
    }

    #[test]
    fn test_sanitize_name() {
        assert_eq!(sanitize_name("my session!").unwrap(), "my_session_");
        assert_eq!(sanitize_name("test-2024").unwrap(), "test-2024");
    }

    #[test]
    fn test_sanitize_name_rejects_empty() {
        assert!(sanitize_name("").is_err());
        assert!(sanitize_name("   ").is_err());
    }

    #[test]
    fn test_sanitize_name_rejects_traversal_stems() {
        assert!(sanitize_name("..").is_err());
        assert!(sanitize_name(".").is_err());
    }

    #[test]
    fn test_sanitize_name_rejects_all_separators() {
        // A name that sanitises to nothing-but-separators would yield
        // a filename like `____.json` — we refuse rather than silently
        // creating a meaningless file.
        assert!(sanitize_name("!!!").is_err());
    }

    #[test]
    fn test_sanitize_name_rejects_overlong() {
        assert!(sanitize_name(&"a".repeat(200)).is_err());
    }

    #[test]
    fn test_export_session() {
        let session = ShellSession {
            version: SESSION_SCHEMA_VERSION,
            name: "test".to_string(),
            session_id: "test-id".to_string(),
            timestamp: "2026-04-16".to_string(),
            mode: "orchestrator".to_string(),
            model_name: None,
            output: vec![
                SerializedEntry {
                    source: "user".to_string(),
                    source_name: None,
                    content: "hello".to_string(),
                },
                SerializedEntry {
                    source: "agent".to_string(),
                    source_name: Some("orchestrator".to_string()),
                    content: "hi there".to_string(),
                },
            ],
            input_history: vec![],
            tokens_used: 100,
            conversation: None,
        };
        let text = export_session(&session);
        assert!(text.contains("you: hello"));
        assert!(text.contains("orchestrator: hi there"));
    }

    #[test]
    #[serial_test::serial(session_env)]
    fn test_save_load_round_trip_under_custom_dir() {
        let td = tempfile::tempdir().unwrap();
        std::env::set_var(SESSION_DIR_ENV, td.path());

        let session = ShellSession {
            version: SESSION_SCHEMA_VERSION,
            name: "rt".to_string(),
            session_id: "uuid-placeholder".to_string(),
            timestamp: "2026-04-16".to_string(),
            mode: "Orchestrator".to_string(),
            model_name: Some("claude-test".to_string()),
            output: vec![SerializedEntry {
                source: "user".to_string(),
                source_name: None,
                content: "hello".to_string(),
            }],
            input_history: vec!["hello".to_string()],
            tokens_used: 42,
            conversation: Some(serde_json::json!({"messages": []})),
        };

        let path = save_session("rt", &session).unwrap();
        assert!(path.exists(), "saved file must exist");

        let loaded = load_session("rt").unwrap();
        assert_eq!(loaded.name, "rt");
        assert_eq!(loaded.model_name.as_deref(), Some("claude-test"));
        assert_eq!(loaded.tokens_used, 42);
        assert!(loaded.conversation.is_some());
        assert_eq!(loaded.input_history, vec!["hello".to_string()]);

        std::env::remove_var(SESSION_DIR_ENV);
    }

    #[test]
    #[serial_test::serial(session_env)]
    fn test_save_load_by_uuid_name() {
        let td = tempfile::tempdir().unwrap();
        std::env::set_var(SESSION_DIR_ENV, td.path());
        let id = uuid::Uuid::new_v4().to_string();

        let session = ShellSession {
            version: SESSION_SCHEMA_VERSION,
            name: id.clone(),
            session_id: id.clone(),
            timestamp: "2026-04-16".to_string(),
            mode: "Orchestrator".to_string(),
            model_name: None,
            output: vec![],
            input_history: vec![],
            tokens_used: 0,
            conversation: None,
        };
        save_session(&id, &session).unwrap();
        assert!(session_exists(&id));

        let loaded = load_session(&id).unwrap();
        assert_eq!(loaded.session_id, id);
        std::env::remove_var(SESSION_DIR_ENV);
    }

    #[test]
    fn test_parse_duration_units() {
        assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(5 * 60));
        assert_eq!(
            parse_duration("2h").unwrap(),
            Duration::from_secs(2 * 60 * 60)
        );
        assert_eq!(
            parse_duration("7d").unwrap(),
            Duration::from_secs(7 * 24 * 60 * 60)
        );
    }

    #[test]
    fn test_parse_duration_rejects_no_suffix() {
        assert!(parse_duration("30").is_err());
        assert!(parse_duration("").is_err());
        assert!(parse_duration("abc").is_err());
    }

    #[test]
    #[serial_test::serial(session_env)]
    fn test_cleanup_dry_run_does_not_delete() {
        let td = tempfile::tempdir().unwrap();
        std::env::set_var(SESSION_DIR_ENV, td.path());
        // Create a session file and age it by backdating its mtime via
        // writing and then re-stamping through filetime would be ideal,
        // but we can instead set the cutoff at 0s so ANY file older than
        // "now minus 0" (i.e. all existing files) qualifies as stale.
        std::fs::write(td.path().join("old.json"), "{}").unwrap();

        let report = cleanup_sessions(Duration::from_secs(0), /*dry_run*/ true).unwrap();
        assert!(report.dry_run);
        assert_eq!(report.removed.len(), 1);
        assert!(
            td.path().join("old.json").exists(),
            "dry-run must not delete"
        );
        std::env::remove_var(SESSION_DIR_ENV);
    }

    #[test]
    #[serial_test::serial(session_env)]
    fn test_cleanup_removes_stale_files() {
        let td = tempfile::tempdir().unwrap();
        std::env::set_var(SESSION_DIR_ENV, td.path());
        std::fs::write(td.path().join("stale.json"), "{}").unwrap();

        let report = cleanup_sessions(Duration::from_secs(0), /*dry_run*/ false).unwrap();
        assert!(!report.dry_run);
        assert_eq!(report.removed, vec!["stale".to_string()]);
        assert!(!td.path().join("stale.json").exists());
        std::env::remove_var(SESSION_DIR_ENV);
    }

    #[test]
    #[serial_test::serial(session_env)]
    fn test_cleanup_keeps_fresh_files() {
        let td = tempfile::tempdir().unwrap();
        std::env::set_var(SESSION_DIR_ENV, td.path());
        std::fs::write(td.path().join("fresh.json"), "{}").unwrap();

        // Cutoff 1 hour back — file we just created is fresher than that.
        let report = cleanup_sessions(Duration::from_secs(3600), false).unwrap();
        assert_eq!(report.removed, Vec::<String>::new());
        assert_eq!(report.kept, 1);
        assert!(td.path().join("fresh.json").exists());
        std::env::remove_var(SESSION_DIR_ENV);
    }

    #[test]
    #[serial_test::serial(session_env)]
    fn test_load_old_schema_file_without_conversation() {
        // A session file produced by an older version of the shell has
        // no `version`, `session_id`, `model_name`, or `conversation`
        // field. It must still load (with defaults) so users don't lose
        // access to saved transcripts after an upgrade.
        let td = tempfile::tempdir().unwrap();
        std::env::set_var(SESSION_DIR_ENV, td.path());
        let legacy = serde_json::json!({
            "name": "legacy",
            "timestamp": "2026-04-16",
            "mode": "Orchestrator",
            "output": [],
            "input_history": [],
            "tokens_used": 0
        });
        std::fs::write(td.path().join("legacy.json"), legacy.to_string()).unwrap();

        let loaded = load_session("legacy").unwrap();
        assert_eq!(loaded.version, 1);
        assert!(loaded.session_id.is_empty());
        assert!(loaded.conversation.is_none());
        std::env::remove_var(SESSION_DIR_ENV);
    }
}
