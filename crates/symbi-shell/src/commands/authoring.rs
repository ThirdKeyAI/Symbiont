use super::CommandResult;
use crate::app::App;

pub fn spawn(app: &mut App, args: &str) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Output(
            "Usage: /spawn <description of the agent to create>".to_string(),
        );
    }

    let prompt = format!(
        "Generate a Symbiont DSL agent definition for the following description:\n\n{}\n\n\
         Validate with validate_dsl; if it fails, fix and re-validate (cap: 2 attempts). \
         As soon as validation passes, call save_artifact to write the agent to \
         agents/<agent_name>.dsl — the user's description above IS the approval, \
         do not stop to ask again. After saving, tell the user the exact filename \
         and remind them they can launch the agent with `/run <agent_name>`.",
        args
    );

    if app.send_to_orchestrator(&prompt, "Generating agent...") {
        CommandResult::Handled
    } else {
        CommandResult::Error("No inference provider configured.".to_string())
    }
}

pub fn policy(app: &mut App, args: &str) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Output(
            "Usage: /policy <description of what the policy should do>".to_string(),
        );
    }

    let prompt = format!(
        "Generate a Cedar policy for the following requirement:\n\n{}\n\n\
         Validate with validate_cedar; if it fails, fix and re-validate (cap: 2 attempts). \
         As soon as validation passes, call save_artifact to write the policy to \
         policies/<descriptive_slug>.cedar — the user's requirement above IS the \
         approval. After saving, tell the user the exact filename.",
        args
    );

    if app.send_to_orchestrator(&prompt, "Generating policy...") {
        CommandResult::Handled
    } else {
        CommandResult::Error("No inference provider configured.".to_string())
    }
}

pub fn tool(app: &mut App, args: &str) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Output("Usage: /tool <description of the tool>".to_string());
    }

    let prompt = format!(
        "Generate a ToolClad TOML manifest (.clad.toml) for the following tool:\n\n{}\n\n\
         Validate with validate_toolclad; if it fails, fix and re-validate (cap: 2 attempts). \
         As soon as validation passes, call save_artifact to write the manifest to \
         tools/<tool_name>.clad.toml — the user's description above IS the approval. \
         After saving, tell the user the exact filename.",
        args
    );

    if app.send_to_orchestrator(&prompt, "Generating tool manifest...") {
        CommandResult::Handled
    } else {
        CommandResult::Error("No inference provider configured.".to_string())
    }
}

pub fn behavior(app: &mut App, args: &str) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Output("Usage: /behavior <description of the behavior>".to_string());
    }

    let prompt = format!(
        "Generate a Symbiont DSL behavior definition for the following:\n\n{}\n\n\
         Validate with validate_dsl; if it fails, fix and re-validate (cap: 2 attempts). \
         As soon as validation passes, call save_artifact to write the behavior to \
         agents/behaviors/<behavior_name>.dsl — the user's description above IS the \
         approval. After saving, tell the user the exact filename.",
        args
    );

    if app.send_to_orchestrator(&prompt, "Generating behavior...") {
        CommandResult::Handled
    } else {
        CommandResult::Error("No inference provider configured.".to_string())
    }
}

/// Initialize a Symbiont project.
///
/// - `/init` with no args: prints the profile menu shell-side (no LLM call).
///   The user then re-runs with `/init <profile>` or a free-form description.
/// - `/init <profile>`: deterministic scaffold. The 4 canonical profiles are
///   Rust constants, so output is identical every time and passes validation.
/// - `/init <free-form description>`: conversational init through the
///   orchestrator, which is instructed to generate + save without a second
///   approval round (the description itself is the approval).
pub fn init(app: &mut App, args: &str) -> CommandResult {
    let args = args.trim();

    if matches!(args, "minimal" | "assistant" | "dev-agent" | "multi-agent") {
        return init_deterministic(args);
    }

    if args.is_empty() {
        return CommandResult::Output(init_menu_text());
    }

    if app.send_to_orchestrator(
        &format!(
            "Initialize a new Symbiont project in the current directory. Requirements:\n\n{}\n\n\
             Generate symbiont.toml, a default Cedar policy at policies/default.cedar, \
             and any agent DSL files under agents/. Use validate_dsl / validate_cedar / \
             validate_toolclad to verify each artifact. The user's description above is \
             their approval — call save_artifact immediately for every validated file \
             without asking again. After saving, tell the user the /spawn command they \
             should run next.",
            args
        ),
        "Setting up project...",
    ) {
        CommandResult::Handled
    } else {
        CommandResult::Error(
            "No orchestrator configured for free-form /init. Use a known profile instead:\n\n\
             Known profiles: minimal, assistant, dev-agent, multi-agent"
                .to_string(),
        )
    }
}

fn init_menu_text() -> String {
    r#"Choose a project profile:

  minimal       Bare project structure — symbiont.toml, default policy, no agents.
  assistant     Single governed assistant agent (read/analyze/summarize).
  dev-agent     Development agent with CLI execution (read/write/execute).
  multi-agent   Coordinator + worker for parallel task decomposition.

Usage:
  /init <profile>            deterministic scaffold (recommended)
  /init <free-form need>     describe what you need; the orchestrator will scaffold

Example:
  /init assistant
  /init a pipeline with one cedar-gated scraper and one summarizer"#
        .to_string()
}

const DEFAULT_CEDAR_POLICY: &str = r#"// Default Symbiont policy — deny by default, allow explicitly.

// Allow all read operations
permit(
    principal,
    action == Action::"read",
    resource
);

// Allow tool invocations that have been verified by SchemaPin
permit(
    principal,
    action == Action::"invoke_tool",
    resource
) when {
    resource.schema_verified == true
};

// Gate write operations on approval
forbid(
    principal,
    action == Action::"write",
    resource
) unless {
    principal.approved == true
};

// Require audit on all state-changing operations
permit(
    principal,
    action == Action::"audit",
    resource
);
"#;

const DEFAULT_CONSTRAINTS: &str = r#"# Project constraints — enforced by symbi shell's validation pipeline.
# The orchestrator LLM cannot see or modify this file.

[constraints]
max_memory = "512MB"
max_cpu = 2.0
required_sandbox = "strict"
min_audit_level = "errors_only"
forbidden_capabilities = []

[constraints.cedar]
require_schema_verified = true
forbid_wildcard_principal = true

[constraints.toolclad]
require_scope_check = true
"#;

fn init_deterministic(profile: &str) -> CommandResult {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            return CommandResult::Error(format!("Cannot determine working directory: {}", e))
        }
    };

    // Check if already initialized
    if cwd.join("symbiont.toml").exists() {
        return CommandResult::Error(
            "symbiont.toml already exists. Project is already initialized.".to_string(),
        );
    }

    let mut created = Vec::new();

    // symbiont.toml
    let toml_content = format!(
        r#"# Symbiont project configuration — generated by symbi shell
# profile: {}

[project]
name = ""
version = "0.1.0"

[schemapin]
mode = "tofu"

[sandbox]
tier = "tier1"
mode = "dev"

[governance]
policy_dir = "policies"
agents_dir = "agents"
tools_dir = "tools"
"#,
        profile
    );
    if let Err(e) = std::fs::write(cwd.join("symbiont.toml"), &toml_content) {
        return CommandResult::Error(format!("Failed to write symbiont.toml: {}", e));
    }
    created.push("symbiont.toml");

    // Directories
    for dir in &[
        "policies",
        "agents",
        "tools",
        ".symbi/sessions",
        ".symbiont/audit",
    ] {
        let _ = std::fs::create_dir_all(cwd.join(dir));
    }
    created.push("policies/");
    created.push("agents/");
    created.push("tools/");

    // Default Cedar policy
    if let Err(e) = std::fs::write(cwd.join("policies/default.cedar"), DEFAULT_CEDAR_POLICY) {
        return CommandResult::Error(format!("Failed to write default.cedar: {}", e));
    }
    created.push("policies/default.cedar");

    // Constraints file
    let constraints_path = cwd.join(".symbi/constraints.toml");
    if !constraints_path.exists() {
        let _ = std::fs::create_dir_all(cwd.join(".symbi"));
        if let Err(e) = std::fs::write(&constraints_path, DEFAULT_CONSTRAINTS) {
            return CommandResult::Error(format!("Failed to write constraints.toml: {}", e));
        }
        created.push(".symbi/constraints.toml");
    }

    // Profile-specific agent files
    match profile {
        "assistant" => {
            let dsl = r#"metadata {
    version = "1.0.0"
    description = "Default governed assistant"
}

agent assistant(input: Query) -> Response {
    capabilities: [read, analyze, summarize]

    policy default_access {
        allow: read
        deny: write if not approved
        audit: all_operations
    }

    with sandbox = "tier1" {
        result = process(input)
        return result
    }
}
"#;
            let _ = std::fs::write(cwd.join("agents/assistant.dsl"), dsl);
            created.push("agents/assistant.dsl");
        }
        "dev-agent" => {
            let dsl = r#"metadata {
    version = "1.0.0"
    description = "Development agent with CLI execution"
}

agent dev(input: Task) -> Result {
    capabilities: [read, write, execute, analyze]

    policy dev_access {
        allow: read
        allow: execute if schema_verified
        deny: write if not approved
        audit: all_operations
    }

    with sandbox = "tier1", memory = "session" {
        result = execute_task(input)
        return result
    }
}
"#;
            let _ = std::fs::write(cwd.join("agents/dev.dsl"), dsl);
            created.push("agents/dev.dsl");
        }
        "multi-agent" => {
            let coordinator = r#"metadata {
    version = "1.0.0"
    description = "Task coordinator for parallel decomposition"
}

agent coordinator(input: Task) -> Result {
    capabilities: [read, delegate, analyze]

    policy coordinator_access {
        allow: read
        allow: delegate
        deny: write
        audit: all_operations
    }
}
"#;
            let worker = r#"metadata {
    version = "1.0.0"
    description = "Worker agent for task execution"
}

agent worker(input: SubTask) -> Result {
    capabilities: [read, write, execute]

    policy worker_access {
        allow: read
        allow: execute if schema_verified
        deny: delegate
        audit: all_operations
    }
}
"#;
            let _ = std::fs::write(cwd.join("agents/coordinator.dsl"), coordinator);
            let _ = std::fs::write(cwd.join("agents/worker.dsl"), worker);
            created.push("agents/coordinator.dsl");
            created.push("agents/worker.dsl");
        }
        _ => {} // minimal — no agent files
    }

    let mut out = format!(
        "Initialized Symbiont project (profile: {})\n\nCreated:\n",
        profile
    );
    for item in &created {
        out.push_str(&format!("  {}\n", item));
    }
    out.push_str(next_steps_hint(profile));

    CommandResult::Output(out)
}

fn next_steps_hint(profile: &str) -> &'static str {
    match profile {
        "assistant" => {
            "\nNext:\n  /spawn assistant    run the scaffolded agent\n  /policy, /tool      add more governance or tools"
        }
        "dev-agent" => {
            "\nNext:\n  /spawn dev          run the scaffolded agent\n  /policy, /tool      add more governance or tools"
        }
        "multi-agent" => {
            "\nNext:\n  /spawn coordinator  run the coordinator\n  /spawn worker       run the worker\n  /policy, /tool      add more governance or tools"
        }
        _ => {
            "\nNext: /spawn <agent-name>, /policy, /tool, or just describe what you need."
        }
    }
}
