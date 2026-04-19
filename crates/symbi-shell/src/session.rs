//! Shell session persistence.
//!
//! Saves and restores conversation history, input history, and mode.
//! Sessions are stored as JSON files in `.symbi/sessions/`.

use crate::app::{EntrySource, OutputEntry};
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const SESSION_DIR: &str = ".symbi/sessions";

/// Serializable session state.
#[derive(Serialize, Deserialize)]
pub struct ShellSession {
    pub name: String,
    pub timestamp: String,
    pub mode: String,
    pub output: Vec<SerializedEntry>,
    pub input_history: Vec<String>,
    pub tokens_used: u64,
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
        let (source, source_name) = match &entry.source {
            EntrySource::User => ("user".to_string(), None),
            EntrySource::System => ("system".to_string(), None),
            EntrySource::Agent(name) => ("agent".to_string(), Some(name.clone())),
            EntrySource::Error => ("error".to_string(), None),
        };
        Self {
            source,
            source_name,
            content: entry.content.clone(),
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
            _ => EntrySource::System,
        };
        OutputEntry {
            source,
            content: self.content.clone(),
        }
    }
}

/// Save a session to disk.
pub fn save_session(name: &str, session: &ShellSession) -> Result<PathBuf> {
    let dir = Path::new(SESSION_DIR);
    std::fs::create_dir_all(dir)?;

    let filename = format!("{}.json", sanitize_name(name));
    let path = dir.join(&filename);

    let json = serde_json::to_string_pretty(session)?;
    std::fs::write(&path, json)?;

    Ok(path)
}

/// Load a session from disk.
pub fn load_session(name: &str) -> Result<ShellSession> {
    let dir = Path::new(SESSION_DIR);
    let filename = format!("{}.json", sanitize_name(name));
    let path = dir.join(&filename);

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
    let dir = Path::new(SESSION_DIR);
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut sessions = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            if let Some(stem) = path.file_stem() {
                sessions.push(stem.to_string_lossy().to_string());
            }
        }
    }
    sessions.sort();
    Ok(sessions)
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
            _ => "sys: ",
        };
        out.push_str(&format!("{}{}\n", prefix, entry.content));
    }

    out
}

fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
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
        assert_eq!(sanitize_name("my session!"), "my_session_");
        assert_eq!(sanitize_name("test-2024"), "test-2024");
    }

    #[test]
    fn test_export_session() {
        let session = ShellSession {
            name: "test".to_string(),
            timestamp: "2026-04-16".to_string(),
            mode: "orchestrator".to_string(),
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
        };
        let text = export_session(&session);
        assert!(text.contains("you: hello"));
        assert!(text.contains("orchestrator: hi there"));
    }
}
