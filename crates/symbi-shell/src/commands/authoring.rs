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
         Use the validate_dsl tool to check the generated DSL against project constraints. \
         If validation fails, fix the issues and re-validate. \
         Present the final validated DSL to me for review.",
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
         Use the validate_cedar tool to check the generated policy against project constraints. \
         If validation fails, fix the issues and re-validate. \
         Present the final validated Cedar policy to me for review.",
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
         Use the validate_toolclad tool to check the generated manifest against project constraints. \
         If validation fails, fix the issues and re-validate. \
         Present the final validated ToolClad manifest to me for review.",
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
         Use the validate_dsl tool to check the generated behavior against project constraints. \
         If validation fails, fix the issues and re-validate. \
         Present the final validated behavior definition to me for review.",
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
/// - `/init` with no args and an orchestrator: conversational project setup
/// - `/init` with no args and no orchestrator: defaults to "minimal" profile
/// - `/init <profile>`: deterministic scaffold (minimal, assistant, dev-agent, multi-agent)
pub fn init(app: &mut App, args: &str) -> CommandResult {
    let args = args.trim();

    // Known profiles get deterministic scaffolding
    if matches!(args, "minimal" | "assistant" | "dev-agent" | "multi-agent") {
        return init_deterministic(args);
    }

    // No args: try conversational, fall back to minimal
    if args.is_empty() {
        if app.send_to_orchestrator(
            "I want to initialize a new Symbiont project in the current directory. \
             Ask me what kind of project I want to build, then help me set it up. \
             The available profiles are:\n\
             - minimal: bare project structure, no agents\n\
             - assistant: single governed assistant agent\n\
             - dev-agent: development agent with CLI execution\n\
             - multi-agent: coordinator + worker for parallel task decomposition\n\n\
             After I choose, generate the appropriate symbiont.toml, default Cedar policy, \
             and agent DSL files. Use the validation tools to check each artifact. \
             Present everything for my review before saving.",
            "Setting up project...",
        ) {
            return CommandResult::Handled;
        }
        // No orchestrator — fall back to deterministic minimal
        return init_deterministic("minimal");
    }

    // Unknown profile name — treat as description for conversational init
    if app.send_to_orchestrator(
        &format!(
            "I want to initialize a new Symbiont project. Here's what I need:\n\n{}\n\n\
             Help me choose the right profile and set up the project structure. \
             Generate symbiont.toml, Cedar policies, and agent DSL as needed. \
             Validate everything before presenting for review.",
            args
        ),
        "Setting up project...",
    ) {
        CommandResult::Handled
    } else {
        CommandResult::Error(
            "Unknown profile and no orchestrator available.\nKnown profiles: minimal, assistant, dev-agent, multi-agent".to_string(),
        )
    }
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
    out.push_str("\nNext: use /spawn, /policy, /tool to add more, or just describe what you need.");

    CommandResult::Output(out)
}
