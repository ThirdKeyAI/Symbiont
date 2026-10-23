//! Tool executor builder
//!
//! Picks the right [`ActionExecutor`] for a runner based on whether the
//! project has ToolClad manifests: [`crate::toolclad::executor::ToolCladExecutor`]
//! (real command/HTTP/MCP-proxy/session/browser backends) when `tools/`
//! contains `.clad.toml` manifests, otherwise
//! [`UnavailableToolExecutor`], the honest no-backend executor that never
//! fabricates tool-call success.
//!
//! Centralizing this choice means entry points (`symbi run`, the DSL
//! `reason()`/`tool_call()` builtins) all get real tool execution the
//! moment a project adds manifests under `tools/`, with no separate wiring
//! per call site.

use crate::reasoning::executor::{ActionExecutor, UnavailableToolExecutor};
use crate::toolclad::executor::ToolCladExecutor;
use crate::toolclad::manifest::{load_custom_types, load_manifests_from_dir_with_types};
use std::path::Path;
use std::sync::Arc;

/// Build the tool executor for a runner: [`ToolCladExecutor`] (with its real
/// shell/HTTP/MCP-proxy/session/browser backends) when `tools_dir` has
/// `.clad.toml` manifests, otherwise [`UnavailableToolExecutor`] — the
/// honest executor that advertises no tools and never fabricates a
/// tool-call success.
pub fn build_tool_executor(tools_dir: &Path) -> Arc<dyn ActionExecutor> {
    let project = project_for_tools(tools_dir);
    match load_project_manifests(tools_dir, project) {
        Ok((manifests, _)) if manifests.is_empty() => Arc::new(UnavailableToolExecutor),
        Ok((manifests, custom_types)) => Arc::new(
            ToolCladExecutor::with_custom_types(manifests, custom_types)
                .with_project_scope(project)
                .with_project_sandbox(project),
        ),
        Err(error) => Arc::new(ToolCladExecutor::failed(error)),
    }
}

/// Load the project's custom types and manifests. A manifest that fails to
/// load is an error for the whole set: the caller gets an executor whose
/// `validate_configuration` fails, so the run stops at startup.
#[allow(clippy::type_complexity)]
fn load_project_manifests(
    tools_dir: &Path,
    project: &Path,
) -> Result<
    (
        Vec<(String, crate::toolclad::Manifest)>,
        std::collections::HashMap<String, crate::toolclad::manifest::ArgDef>,
    ),
    String,
> {
    let custom_types = load_custom_types(project)?;
    let manifests = load_manifests_from_dir_with_types(tools_dir, &custom_types)?;
    Ok((manifests, custom_types))
}

/// Build a production executor using the selected agent's frozen boundary.
/// Validation precedes inference even when no tools are registered.
pub fn build_agent_tool_executor(
    tools_dir: &Path,
    settings: &dsl::AgentExecutionSettings,
) -> Result<Arc<dyn ActionExecutor>, String> {
    let project = project_for_tools(tools_dir);
    let boundary = crate::sandbox::command::CommandBoundary::load_for_agent(project, settings)?;
    Ok(build_tool_executor_with_boundary(tools_dir, boundary))
}

/// Consume the already selected profile without rereading sandbox configuration.
pub(crate) fn build_tool_executor_with_boundary(
    tools_dir: &Path,
    boundary: crate::sandbox::command::CommandBoundary,
) -> Arc<dyn ActionExecutor> {
    let project = project_for_tools(tools_dir);
    match load_project_manifests(tools_dir, project) {
        Ok((manifests, _)) if manifests.is_empty() => Arc::new(UnavailableToolExecutor),
        Ok((manifests, custom_types)) => Arc::new(
            ToolCladExecutor::with_custom_types(manifests, custom_types)
                .with_project_scope(project)
                .with_command_boundary(boundary),
        ),
        Err(error) => Arc::new(ToolCladExecutor::failed(error)),
    }
}

fn project_for_tools(tools_dir: &Path) -> &Path {
    tools_dir
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_selection_is_required_even_without_tools() {
        let project = tempfile::tempdir().unwrap();
        let settings = dsl::resolve_execution_settings(
            r#"agent a() { with sandbox = "firecracker" {} }"#,
            "a",
        )
        .unwrap();
        assert!(build_agent_tool_executor(&project.path().join("tools"), &settings).is_err());
    }

    #[test]
    fn returns_unavailable_when_no_manifests() {
        let dir = tempfile::tempdir().unwrap();
        let exec = build_tool_executor(dir.path());
        // Unavailable advertises no tools.
        assert!(exec.tool_definitions().is_empty());
    }

    #[test]
    fn returns_unavailable_when_dir_missing() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        let exec = build_tool_executor(&missing);
        assert!(exec.tool_definitions().is_empty());
    }

    #[test]
    fn returns_toolclad_when_manifests_present() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("greet.clad.toml"),
            r#"
[tool]
name = "greet"
version = "1.0.0"
binary = "echo"
description = "g"

[args.message]
position = 1
required = true
type = "string"
description = "message to echo"

[command]
template = "echo {message}"

[output]
format = "text"

[output.schema]
type = "object"
"#,
        )
        .unwrap();

        let exec = build_tool_executor(dir.path());
        assert!(exec.tool_definitions().iter().any(|d| d.name == "greet"));
    }

    #[test]
    fn a_manifest_load_failure_fails_configuration_loudly() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("typo.clad.toml"),
            "[tool]\nname = \"t\"\nversion = \"1\"\nbinary = \"echo\"\ndescription = \"d\"\ntiemout_seconds = 5\n[command]\nexec = [\"echo\"]\n[output]\nformat = \"text\"\n",
        )
        .unwrap();
        let exec = build_tool_executor(dir.path());
        let err = exec.validate_configuration().unwrap_err();
        assert!(err.contains("typo.clad.toml"), "{err}");
    }
}
