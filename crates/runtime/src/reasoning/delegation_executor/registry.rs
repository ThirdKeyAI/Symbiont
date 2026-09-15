//! Frozen, unambiguous conversational sources for coordinator delegation.
use super::*;
use crate::reasoning::{
    loop_types::{Observation, ProposedAction},
    prepared::{digest_json, AuthorizedAction, PreparedAction},
};
use serde_json::{json, Value};
use std::{collections::HashSet, path::Path};

pub(super) struct RegisteredTarget {
    pub agent: dsl::ConversationalAgent,
    pub prompt: String,
    pub metadata: Value,
}

/// Load once from operator-selected project files. Aliases share one declaration,
/// policy principal and immutable source snapshot; conflicts are never resolved
/// by directory order. Unsupported definitions remain unavailable.
pub struct RegisteredDelegationRegistry {
    targets: HashMap<String, Arc<RegisteredTarget>>,
    rejected: HashMap<String, String>,
}

impl RegisteredDelegationRegistry {
    pub fn load(project: &Path) -> Result<Self, String> {
        let reader = crate::integrations::mcp::project::ProjectReader::open(project)?;
        let entries = match std::fs::read_dir(reader.path().join("agents")) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Self::from_sources(Vec::new());
            }
            Err(error) => return Err(error.to_string()),
        };
        let mut sources = Vec::new();
        let mut bytes = 0;
        let mut unavailable = HashMap::new();
        for (index, entry) in entries.enumerate() {
            if index >= 1024 {
                return Err("delegation registry exceeds 1024 directory entries".into());
            }
            let entry = entry.map_err(|error| error.to_string())?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| "agent filename is not UTF-8")?;
            let Some(stem) = dsl::strip_symbi_extension(&name) else {
                continue;
            };
            match reader.read_text(&Path::new("agents").join(&name), 1024 * 1024) {
                Ok(source) => {
                    bytes += source.len();
                    if bytes > 16 * 1024 * 1024 {
                        return Err("delegation registry exceeds 16 MiB".into());
                    }
                    sources.push((name, source));
                }
                Err(error) => {
                    unavailable.insert(stem.to_owned(), error);
                }
            }
        }
        let mut registry = Self::from_sources(sources)?;
        for (name, error) in unavailable {
            registry.reject_identity(&name, &format!("unavailable delegation source: {error}"));
        }
        Ok(registry)
    }

    pub fn from_sources(sources: Vec<(String, String)>) -> Result<Self, String> {
        if sources.len() > 1024
            || sources
                .iter()
                .map(|(_, source)| source.len())
                .sum::<usize>()
                > 16 * 1024 * 1024
        {
            return Err("delegation registry exceeds its source count or byte limit".into());
        }
        let mut registry = Self {
            targets: HashMap::new(),
            rejected: HashMap::new(),
        };
        let mut candidates: HashMap<String, Vec<Arc<RegisteredTarget>>> = HashMap::new();
        let mut selected_bytes = 0;
        let mut selected_count = 0;
        for (file, source) in sources {
            let stem = dsl::strip_symbi_extension(&file)
                .ok_or("delegation source must use .symbi or .dsl")?;
            let names = match dsl::conversational_agent_names(&source) {
                Ok(names) => names,
                Err(error) => {
                    registry.rejected.insert(stem.into(), error);
                    continue;
                }
            };
            for name in &names {
                selected_count += 1;
                selected_bytes += source.len();
                if selected_count > 1024 || selected_bytes > 16 * 1024 * 1024 {
                    return Err("delegation registry exceeds retained declaration limits".into());
                }
                let mut aliases = vec![name.as_str()];
                if names.len() == 1 && stem != name {
                    aliases.push(stem);
                }
                match dsl::ConversationalAgent::parse(&source, name) {
                    Ok(agent) => {
                        let metadata = json!({"name":name, "principal":delegated_agent_id(name),
                            "source_hash":digest_json(&json!(agent.source()))?,
                            "declaration_hash":digest_json(&json!(agent.settings().agent_source))?,
                            "timeout_seconds":agent.settings().timeout_seconds,
                            "sandbox_tier":agent.settings().sandbox_tier,
                            "mode":"coordinator_monitoring", "policy_names":agent.policy().names()});
                        let prompt = format!("You are agent '{name}'. Answer the delegated task using the available monitoring tools.\n\n--- Selected agent definition ---\n{}\n--- End definition ---", agent.settings().agent_source);
                        let target = Arc::new(RegisteredTarget {
                            agent,
                            prompt,
                            metadata,
                        });
                        for alias in aliases {
                            candidates
                                .entry(alias.into())
                                .or_default()
                                .push(target.clone());
                        }
                    }
                    Err(error) => {
                        for alias in aliases {
                            registry.rejected.insert(alias.into(), error.clone());
                        }
                    }
                }
            }
            if names.len() > 1 && !names.iter().any(|name| name == stem) {
                registry.rejected.insert(
                    stem.into(),
                    "filename alias selects multiple declarations".into(),
                );
            }
        }
        // A collision invalidates every alias of the affected declarations,
        // otherwise an alias could still select a conflicting policy principal.
        let mut conflicts = HashSet::new();
        for (alias, targets) in &candidates {
            if targets.len() != 1 || registry.rejected.contains_key(alias) {
                conflicts.extend(
                    targets
                        .iter()
                        .map(|target| target.agent.settings().agent_name.clone()),
                );
            }
        }
        for (alias, targets) in candidates {
            if targets.len() == 1 && !conflicts.contains(&targets[0].agent.settings().agent_name) {
                registry.targets.insert(alias, targets[0].clone());
            } else {
                registry
                    .rejected
                    .insert(alias, "ambiguous delegation source identity".into());
            }
        }
        Ok(registry)
    }

    fn reject_identity(&mut self, alias: &str, error: &str) {
        if let Some(target) = self.targets.get(alias).cloned() {
            let aliases: Vec<_> = self
                .targets
                .iter()
                .filter(|(_, candidate)| Arc::ptr_eq(candidate, &target))
                .map(|(name, _)| name.clone())
                .collect();
            for name in aliases {
                self.targets.remove(&name);
                self.rejected.insert(name, error.into());
            }
        }
        self.rejected.insert(alias.into(), error.into());
    }

    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<_> = self.targets.keys().cloned().collect();
        names.sort();
        names
    }

    pub(super) fn resolve(&self, name: &str) -> Result<Arc<RegisteredTarget>, String> {
        self.targets.get(name).cloned().ok_or_else(|| {
            self.rejected
                .get(name)
                .cloned()
                .unwrap_or_else(|| format!("unknown delegation target '{name}'"))
        })
    }

    pub(crate) fn wrap(
        self: &Arc<Self>,
        inner: Arc<dyn ActionExecutor>,
    ) -> Arc<dyn ActionExecutor> {
        Arc::new(RegistryExecutor {
            inner,
            registry: self.clone(),
        })
    }
}

struct RegistryExecutor {
    inner: Arc<dyn ActionExecutor>,
    registry: Arc<RegisteredDelegationRegistry>,
}

#[async_trait]
impl ActionExecutor for RegistryExecutor {
    fn execution_context(&self) -> HashMap<String, Value> {
        self.inner.execution_context()
    }
    fn validate_configuration(&self) -> Result<(), String> {
        self.inner.validate_configuration()
    }
    fn tool_definitions(&self) -> Vec<crate::reasoning::inference::ToolDefinition> {
        self.inner.tool_definitions()
    }
    fn prepare_action(
        &self,
        action: &ProposedAction,
        config: &LoopConfig,
    ) -> Result<PreparedAction, String> {
        let ProposedAction::Delegate {
            call_id,
            target,
            message,
        } = action
        else {
            return self.inner.prepare_action(action, config);
        };
        if message.len() > 64 * 1024 {
            return Err("delegated task exceeds 64 KiB".into());
        }
        let selected = self.registry.resolve(target)?;
        PreparedAction::new(
            ProposedAction::Delegate {
                call_id: call_id.clone(),
                target: selected.agent.settings().agent_name.clone(),
                message: message.clone(),
            },
            None,
        )?
        .with_resolved(json!({"delegation_target":selected.metadata, "requested_name":target}))
        .map(|prepared| prepared.with_backend(selected))
    }
    fn cancel_run(&self, run: &str, deadline: std::time::Instant) {
        self.inner.cancel_run(run, deadline);
    }
    async fn close_run(&self, run: &str, deadline: std::time::Instant) -> Result<(), String> {
        self.inner.close_run(run, deadline).await
    }
    async fn execute_actions(
        &self,
        actions: &[ProposedAction],
        config: &LoopConfig,
        breakers: &CircuitBreakerRegistry,
    ) -> Vec<Observation> {
        self.inner.execute_actions(actions, config, breakers).await
    }
    async fn execute_authorized(
        &self,
        actions: Vec<AuthorizedAction>,
        config: &LoopConfig,
        breakers: &CircuitBreakerRegistry,
    ) -> Vec<Observation> {
        self.inner
            .execute_authorized(actions, config, breakers)
            .await
    }
}

#[cfg(test)]
mod tests;
