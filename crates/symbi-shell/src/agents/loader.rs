//! Discovery + parsing of agent definitions into a unified `AgentSpec`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Where an ingested agent came from. A single-variant enum today; a later phase
/// adds `Symbi(PathBuf)` when real `.symbi` execution lands. Keeping it an enum
/// marks that extension point without implying `.symbi` is loadable now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentSource {
    Manifest(PathBuf),
}

/// The manifest's projected shape — what the orchestrator can delegate to.
#[derive(Debug, Clone)]
pub struct AgentSpec {
    pub name: String,
    pub description: String,
    pub system_prompt: String,
    pub tools: Vec<String>,
    // source retained for the deferred .symbi execution phase (set, not yet read)
    #[allow(dead_code)]
    pub source: AgentSource,
}

/// A synchronous summary of a loaded agent, used to build the `delegate` tool's
/// description without touching the async registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCard {
    pub name: String,
    pub description: String,
}

/// A single file that failed to load. One bad file never aborts the rest.
#[derive(Debug, Clone)]
pub struct LoadError {
    pub path: PathBuf,
    pub message: String,
}

#[derive(serde::Deserialize)]
struct Manifest {
    name: String,
    description: String,
    system_prompt: String,
    #[serde(default)]
    tools: Vec<String>,
}

/// Parse a single `*.toml` manifest into an `AgentSpec`.
pub fn parse_manifest(path: &Path) -> Result<AgentSpec, LoadError> {
    let err = |m: String| LoadError {
        path: path.to_path_buf(),
        message: m,
    };
    let text = std::fs::read_to_string(path).map_err(|e| err(e.to_string()))?;
    let m: Manifest = toml::from_str(&text).map_err(|e| err(e.to_string()))?;
    if m.name.trim().is_empty() {
        return Err(err("manifest 'name' is empty".into()));
    }
    if m.system_prompt.trim().is_empty() {
        return Err(err("manifest 'system_prompt' is empty".into()));
    }
    Ok(AgentSpec {
        name: m.name,
        description: m.description,
        system_prompt: m.system_prompt,
        tools: m.tools,
        source: AgentSource::Manifest(path.to_path_buf()),
    })
}

/// Scan a directory for `*.toml` manifests. Returns all successfully-parsed specs
/// plus a per-file error for each manifest that failed. `.symbi` files are
/// ignored (see `count_symbi`); a missing directory yields empty results.
pub fn scan_dir(dir: &Path) -> (Vec<AgentSpec>, Vec<LoadError>) {
    let mut specs = Vec::new();
    let mut errors = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return (specs, errors),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("toml") {
            match parse_manifest(&path) {
                Ok(s) => specs.push(s),
                Err(e) => errors.push(e),
            }
        }
    }
    (specs, errors)
}

/// Count `*.symbi` files in `dir`. These are deferred (full DSL-agent execution
/// is a later phase) and are NOT loaded; the count drives a one-line user notice
/// so a dropped `.symbi` file isn't met with silence. A missing directory → 0.
pub fn count_symbi(dir: &Path) -> usize {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    entries
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("symbi"))
        .count()
}

/// Outcome of a load pass, for surfacing to the user.
#[derive(Debug, Clone, Default)]
pub struct LoadReport {
    pub loaded: usize,
    pub errors: Vec<LoadError>,
    pub collisions: Vec<String>,
    /// Number of `.symbi` files detected but deferred (not loaded) in the MVP.
    pub deferred_symbi: usize,
}

/// Scan `dir`, register every manifest spec into the bridge's registry (last-wins
/// on name collision), rebuild the synchronous `cards` mirror, and count deferred
/// `.symbi` files. Per-file parse errors are collected, never fatal.
pub async fn load_agents_into(
    dir: &Path,
    bridge: &Arc<repl_core::RuntimeBridge>,
    cards: &Arc<RwLock<Vec<AgentCard>>>,
) -> LoadReport {
    let (specs, errors) = scan_dir(dir);
    let deferred_symbi = count_symbi(dir);

    // De-dupe by name, last wins; record which names collided.
    let mut by_name: std::collections::HashMap<String, AgentSpec> =
        std::collections::HashMap::new();
    let mut collisions = Vec::new();
    for spec in specs {
        if by_name.insert(spec.name.clone(), spec.clone()).is_some()
            && !collisions.contains(&spec.name)
        {
            collisions.push(spec.name.clone());
        }
    }

    let mut new_cards = Vec::new();
    for spec in by_name.values() {
        bridge
            .register_agent(&spec.name, &spec.system_prompt, spec.tools.clone())
            .await;
        new_cards.push(AgentCard {
            name: spec.name.clone(),
            description: spec.description.clone(),
        });
    }
    new_cards.sort_by(|a, b| a.name.cmp(&b.name));
    let loaded = new_cards.len();
    *cards.write().await = new_cards;

    LoadReport {
        loaded,
        errors,
        collisions,
        deferred_symbi,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        p
    }

    #[test]
    fn parses_valid_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(
            dir.path(),
            "researcher.toml",
            "name = \"researcher\"\ndescription = \"finds sources\"\nsystem_prompt = \"You research.\"\ntools = [\"search\"]\n",
        );
        let spec = parse_manifest(&p).unwrap();
        assert_eq!(spec.name, "researcher");
        assert_eq!(spec.description, "finds sources");
        assert_eq!(spec.system_prompt, "You research.");
        assert_eq!(spec.tools, vec!["search".to_string()]);
        assert_eq!(spec.source, AgentSource::Manifest(p));
    }

    #[test]
    fn missing_required_field_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(dir.path(), "bad.toml", "name = \"x\"\n");
        assert!(parse_manifest(&p).is_err());
    }

    #[test]
    fn malformed_toml_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(dir.path(), "bad.toml", "this is = = not toml");
        assert!(parse_manifest(&p).is_err());
    }

    #[test]
    fn scan_dir_loads_manifests_isolates_bad_and_ignores_symbi() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "a.toml",
            "name=\"a\"\ndescription=\"d\"\nsystem_prompt=\"p\"\n",
        );
        write(dir.path(), "broken.toml", "= = =");
        // a .symbi file must NOT be loaded as an agent
        write(
            dir.path(),
            "b.symbi",
            "agent b {\n  security { capabilities = [] }\n}\n",
        );
        let (specs, errors) = scan_dir(dir.path());
        let names: Vec<_> = specs.iter().map(|s| s.name.clone()).collect();
        assert_eq!(names, vec!["a".to_string()], "only the manifest loads");
        assert!(
            !names.contains(&"b".to_string()),
            ".symbi is never registered"
        );
        assert_eq!(errors.len(), 1, "the broken manifest is the only error");
    }

    #[test]
    fn count_symbi_counts_deferred_files() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "a.toml",
            "name=\"a\"\ndescription=\"d\"\nsystem_prompt=\"p\"\n",
        );
        write(dir.path(), "x.symbi", "agent x {}\n");
        write(dir.path(), "y.symbi", "agent y {}\n");
        assert_eq!(count_symbi(dir.path()), 2);
    }

    #[tokio::test]
    async fn load_registers_agents_rebuilds_cards_and_dedupes() {
        use std::sync::Arc;
        use tokio::sync::RwLock;
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "a.toml",
            "name=\"a\"\ndescription=\"first\"\nsystem_prompt=\"p\"\n",
        );
        write(
            dir.path(),
            "b.toml",
            "name=\"b\"\ndescription=\"second\"\nsystem_prompt=\"p\"\n",
        );
        // collision: a second 'a' — last loaded wins
        write(
            dir.path(),
            "a2.toml",
            "name=\"a\"\ndescription=\"override\"\nsystem_prompt=\"p\"\n",
        );
        // a .symbi file is counted as deferred, never loaded
        write(dir.path(), "z.symbi", "agent z {}\n");

        let bridge = Arc::new(repl_core::RuntimeBridge::new_permissive_for_dev());
        let cards = Arc::new(RwLock::new(Vec::<AgentCard>::new()));
        let report = load_agents_into(dir.path(), &bridge, &cards).await;

        assert_eq!(report.loaded, 2, "a (deduped) + b");
        assert_eq!(report.collisions, vec!["a".to_string()]);
        assert_eq!(report.deferred_symbi, 1, ".symbi counted, not loaded");
        let names: Vec<_> = cards.read().await.iter().map(|c| c.name.clone()).collect();
        assert!(names.contains(&"a".to_string()) && names.contains(&"b".to_string()));
        assert!(!names.contains(&"z".to_string()), ".symbi never registered");
    }

    #[test]
    fn scan_missing_dir_is_empty_not_error() {
        let (specs, errors) = scan_dir(std::path::Path::new("/no/such/dir/here"));
        assert!(specs.is_empty());
        assert!(errors.is_empty());
        assert_eq!(count_symbi(std::path::Path::new("/no/such/dir/here")), 0);
    }
}
