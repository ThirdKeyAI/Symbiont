//! Built-in workspace capabilities backed by the common governed executor.

use std::path::{Component, Path, PathBuf};
use symbi_runtime::reasoning::{
    executor::ActionExecutor,
    inference::ToolDefinition,
    loop_types::{LoopConfig, ProposedAction},
    prepared::{AuthorizedAction, PreparedAction},
};
use symbi_runtime::sandbox::command::{CommandBoundary, CommandTier};
use symbi_runtime::sandbox::workspace::WorkspacePlan;
use symbi_runtime::toolclad::{executor::ToolCladExecutor, manifest::Manifest};

const WORKSPACE: &str = "/workspace";

struct WorkspaceSnapshot {
    owner: uuid::Uuid,
    plan: WorkspacePlan,
}

pub struct SandboxTools {
    pub executor: ToolCladExecutor,
    boundary: Result<CommandBoundary, String>,
    owner: uuid::Uuid,
}

impl SandboxTools {
    pub fn new(project: &Result<PathBuf, String>, allow_shell: bool) -> Self {
        let boundary = project
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|project| CommandBoundary::load(project));
        Self::with_boundary(boundary, allow_shell)
    }

    pub fn with_boundary(boundary: Result<CommandBoundary, String>, allow_shell: bool) -> Self {
        let mut executor = ToolCladExecutor::new(manifests(allow_shell));
        if let Ok(boundary) = &boundary {
            executor = executor.with_command_boundary(boundary.clone());
        }
        Self {
            executor,
            boundary,
            owner: uuid::Uuid::new_v4(),
        }
    }

    pub fn handles(name: &str) -> bool {
        matches!(
            name,
            "read_file" | "search" | "edit_file" | "save_artifact" | "shell"
        )
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.executor.tool_definitions()
    }

    pub fn validate_configuration(&self) -> Result<(), String> {
        self.boundary.as_ref().map_err(Clone::clone)?.validate()
    }

    pub fn prepare(
        &self,
        action: &ProposedAction,
        config: &LoopConfig,
    ) -> Result<PreparedAction, String> {
        let boundary = self.boundary.as_ref().map_err(Clone::clone)?;
        boundary.validate()?;
        let ProposedAction::ToolCall {
            call_id,
            name,
            arguments,
        } = action
        else {
            return Err("workspace executor requires a tool call".into());
        };
        if arguments.len() > 65536 {
            return Err("workspace arguments exceed 65536 bytes".into());
        }
        let mut args: serde_json::Value =
            serde_json::from_str(arguments).map_err(|e| e.to_string())?;
        let args = args
            .as_object_mut()
            .ok_or("workspace arguments must be an object")?;
        if name != "shell" {
            let key = if name == "save_artifact" {
                "filename"
            } else {
                "path"
            };
            if name == "search" && !args.contains_key(key) {
                args.insert(key.into(), serde_json::json!("."));
            }
            let path = args
                .get(key)
                .and_then(|v| v.as_str())
                .ok_or("missing workspace path")?;
            let normalized = normalize_path(path, name == "search")?;
            self.check_mount(
                &normalized,
                name == "search",
                matches!(name.as_str(), "edit_file" | "save_artifact"),
            )?;
            args.insert(key.into(), serde_json::json!(normalized));
        }
        for key in ["content", "query", "command"] {
            if args
                .get(key)
                .and_then(|value| value.as_str())
                .is_some_and(|text| text.len() > 32768 || text.contains('\0'))
            {
                return Err(format!("{key} exceeds 32768 bytes or contains NUL"));
            }
        }
        if name == "search" && args.get("query").and_then(|value| value.as_str()) == Some("") {
            return Err("query must not be empty".into());
        }
        let prepared = self.executor.prepare_action(
            &ProposedAction::ToolCall {
                call_id: call_id.clone(),
                name: name.clone(),
                arguments: serde_json::to_string(args).map_err(|error| error.to_string())?,
            },
            config,
        )?;
        if name == "shell" {
            return Ok(prepared);
        }
        let path_key = if name == "save_artifact" {
            "filename"
        } else {
            "path"
        };
        let plan = WorkspacePlan::prepare(
            boundary,
            name,
            args[path_key].as_str().ok_or("missing workspace path")?,
            args.get(if name == "search" { "query" } else { "content" })
                .and_then(|v| v.as_str()),
        )?;
        // Bind the retained snapshots and exact write capability into policy
        // and approval. Arbitrary programs never enter the fixed file broker.
        let mut resolved = prepared.policy_context()["resolved"].clone();
        resolved.as_object_mut().unwrap().remove("argv");
        resolved["workspace"] = serde_json::json!(WORKSPACE);
        resolved["workspace_operation"] = serde_json::json!(name);
        resolved["execution_transport"] = serde_json::json!("fixed_workspace_file_broker");
        resolved["file_access"] = plan.descriptor().clone();
        resolved["command_boundary"]["filesystem"] = plan.descriptor().clone();
        resolved["worker_program_hash"] = serde_json::json!(WorkspacePlan::implementation_hash());
        let mut contract = prepared
            .contract()
            .cloned()
            .ok_or("missing workspace tool contract")?;
        contract.digest = symbi_runtime::reasoning::prepared::digest_json(&serde_json::json!({
            "manifest": contract.digest, "fixed_file_broker": WorkspacePlan::implementation_hash()
        }))?;
        Ok(
            PreparedAction::new(prepared.action().clone(), Some(contract))?
                .with_resolved(resolved)?
                .with_backend(WorkspaceSnapshot {
                    owner: self.owner,
                    plan,
                }),
        )
    }

    pub async fn execute_authorized(
        &self,
        grants: Vec<AuthorizedAction>,
        config: &LoopConfig,
        breakers: &symbi_runtime::reasoning::circuit_breaker::CircuitBreakerRegistry,
    ) -> Vec<symbi_runtime::reasoning::loop_types::Observation> {
        use symbi_runtime::reasoning::loop_types::Observation;
        let mut observations = Vec::new();
        for grant in grants {
            let ProposedAction::ToolCall { call_id, name, .. } = grant.action().clone() else {
                continue;
            };
            if name == "shell" {
                observations.extend(
                    self.executor
                        .execute_authorized(vec![grant], config, breakers)
                        .await,
                );
                continue;
            }
            let result = async {
                grant.check_live()?;
                breakers
                    .check(&name)
                    .await
                    .map_err(|error| error.to_string())?;
                let deadline = grant.deadline();
                let prepared = grant.into_prepared()?;
                let snapshot = prepared
                    .backend::<WorkspaceSnapshot>()
                    .ok_or("missing workspace file capability")?;
                if snapshot.owner != self.owner {
                    return Err("workspace capability belongs to another executor".into());
                }
                self.boundary.as_ref().map_err(Clone::clone)?.validate()?;
                snapshot.plan.execute(deadline)
            }
            .await;
            observations.push(match result {
                Ok(value) => {
                    Observation::tool_result(format!("toolclad:{name}"), value.to_string())
                        .with_call_id(call_id)
                }
                Err(error) => {
                    Observation::tool_error(format!("toolclad:{name}"), error).with_call_id(call_id)
                }
            });
        }
        observations
    }

    fn check_mount(&self, relative: &str, search: bool, write: bool) -> Result<(), String> {
        let boundary = self.boundary.as_ref().map_err(Clone::clone)?;
        let config = match boundary.tier {
            CommandTier::Docker => &boundary.docker,
            CommandTier::GVisor => &boundary.gvisor.docker,
            _ => return Err("workspace file tools require a selected container boundary".into()),
        };
        let target = if relative == "." {
            PathBuf::from(WORKSPACE)
        } else {
            Path::new(WORKSPACE).join(relative)
        };
        let mounts: Vec<_> = config
            .volumes
            .iter()
            .filter_map(|mount| {
                let parts: Vec<_> = mount.split(':').collect();
                let destination = Path::new(*parts.get(1)?);
                destination
                    .starts_with(WORKSPACE)
                    .then_some((destination, parts.get(2) == Some(&"rw")))
            })
            .collect();
        let covering = mounts
            .iter()
            .filter(|(destination, _)| target.starts_with(destination))
            .max_by_key(|(destination, _)| destination.components().count());
        let allowed = covering.is_some_and(|(_, writable)| !write || *writable)
            || (!write
                && search
                && mounts
                    .iter()
                    .any(|(destination, _)| destination.starts_with(&target)));
        if !allowed {
            return Err(
                "workspace path has no explicit sandbox mount with the required access".into(),
            );
        }
        Ok(())
    }
}

pub fn normalize_path(value: &str, allow_directory: bool) -> Result<String, String> {
    if value.is_empty()
        || value.len() > 4096
        || value.contains(['\0', '\\'])
        || Path::new(value).is_absolute()
    {
        return Err("invalid workspace-relative path".into());
    }
    let mut parts = Vec::new();
    for part in Path::new(value).components() {
        match part {
            Component::Normal(part) => parts.push(part.to_str().ok_or("invalid path encoding")?),
            Component::CurDir => {}
            _ => return Err("workspace paths cannot traverse parents".into()),
        }
    }
    if parts.is_empty() {
        return if allow_directory {
            Ok(".".into())
        } else {
            Err("expected a file path".into())
        };
    }
    if parts
        .iter()
        .any(|part| matches!(*part, ".git" | ".symbiont"))
    {
        return Err("workspace control directories are unavailable".into());
    }
    Ok(parts.join("/"))
}

fn manifests(allow_shell: bool) -> Vec<(String, Manifest)> {
    // Fixed operations use these contracts only for validation and approval.
    // Accidental generic command dispatch must fail instead of claiming success.
    let mut definitions = vec![
        ("read_file", "Read a bounded snapshot of one authorized workspace text file (up to 32768 bytes)", false,
            serde_json::json!({"path": argument(1, true)}), "false".to_owned()),
        ("search", "Search authorized workspace snapshots with explicit byte, file and depth limits", false,
            serde_json::json!({"path": {"position":1,"required":false,"type":"literal_text","default":"."}, "query": argument(2, true)}), "false".to_owned()),
        ("edit_file", "Write one authorized workspace file; checks its prior snapshot and requires exact approval", true,
            serde_json::json!({"path": argument(1, true), "content": argument(2, true)}), "false".to_owned()),
        ("save_artifact", "Save one validated artifact with exact file approval, creating missing parent directories", true,
            serde_json::json!({"filename": argument(1, true), "content": argument(2, true), "artifact_type": {"position":3,"type":"enum","required":true,"allowed":["dsl","cedar","toolclad"]}}), "false".to_owned()),
    ];
    if allow_shell {
        definitions.push((
            "shell",
            "Run a command in private sandbox scratch space without host files; requires exact approval",
            true,
            serde_json::json!({"command": argument(1, true)}),
            "sh -c {command}".into(),
        ));
    }
    definitions.into_iter().map(|(name, description, approval, args, template)| {
        let manifest = serde_json::from_value(serde_json::json!({
            "tool": {"name":name,"version":"2","binary":if name == "shell" {"sh"} else {"false"},"description":description,"human_approval":approval,"timeout_seconds":30},
            "args":args,"command":{"template":template},"output":{"format":"text"}
        })).expect("fixed workspace tool manifest");
        (name.into(), manifest)
    }).collect()
}

fn argument(position: u32, required: bool) -> serde_json::Value {
    serde_json::json!({"position":position,"required":required,"type":"literal_text"})
}
