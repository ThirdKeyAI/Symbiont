//! Read-only native development setup using the existing managed tool boundary.

use clap::ArgMatches;
use serde_json::json;
use std::path::{Path, PathBuf};
use symbi_runtime::{
    cli_executor::inference_broker::InferenceBrokerConfig,
    sandbox::{command::CommandBoundary, landlock::LandlockProfile},
};

const AGENT: &str = r#"metadata {
    version = "1.0.0"
    description = "Read-only repository review through governed source tools"
    executor = "claude_code"
    allowed_tools = "read_file,list_files,grep_files"
}

// The project selects Landlock. Source access belongs to the tool broker.
agent dev() {
    capabilities = ["read", "analyze"]
    with timeout = 300.seconds {}
}
"#;

const POLICY: &str = r#"// Authorize managed CLI admission and only the three fixed source queries.
// Source roots in symbiont.toml bound all reads. No output roots or write tools
// are configured. Built-in CLI tools and automatic plugins remain disabled.
permit(principal, action == Action::"tool_call::claude_code", resource);
permit(principal, action == Action::"tool_call::read_file", resource);
permit(principal, action == Action::"tool_call::list_files", resource);
permit(principal, action == Action::"tool_call::grep_files", resource);
"#;

const TOOLS: &[(&str, &str)] = &[
    (
        "read_file",
        include_str!("../../../tools/read_file.clad.toml"),
    ),
    (
        "list_files",
        include_str!("../../../tools/list_files.clad.toml"),
    ),
    (
        "grep_files",
        include_str!("../../../tools/grep_files.clad.toml"),
    ),
];

pub(super) struct Settings {
    source: PathBuf,
    executable: PathBuf,
    inference: InferenceBrokerConfig,
}

impl Settings {
    pub(super) fn collect(
        matches: &ArgMatches,
        project: &Path,
        no_interact: bool,
    ) -> Result<Self, String> {
        // Never combine generated read-only grants with pre-existing tools or
        // policies. --force only overwrites the ordinary init configuration.
        if std::fs::read_dir(project)
            .map_err(|e| e.to_string())?
            .next()
            .is_some()
        {
            return Err("choose an empty --dir for the development control project; existing files are preserved even with --force".into());
        }
        if no_interact || !cfg!(feature = "interactive") {
            let missing: Vec<_> = super::DEVELOPER_OPTIONS
                .iter()
                .filter(|name| matches.get_one::<String>(name).is_none())
                .map(|name| format!("--{name}"))
                .collect();
            if !missing.is_empty() {
                return Err(format!(
                    "supply {} (credential variable NAME only, not its value). See docs/landlock-development.md",
                    missing.join(", ")
                ));
            }
        }
        if !no_interact {
            println!("\nRead-only review: select a source repository separate from this control project.");
            println!("The inference endpoint must support the Anthropic Messages API; credentials stay in the runtime.");
        }
        let source = setting(matches, "source", "Source repository path", no_interact)?;
        let executable = setting(
            matches,
            "managed-executable",
            "Installed Claude Code executable path",
            no_interact,
        )?;
        let base_url = setting(
            matches,
            "inference-url",
            "Inference base URL (Anthropic Messages-compatible)",
            no_interact,
        )?;
        let model = setting(matches, "inference-model", "Inference model", no_interact)?;
        let api_key_env = setting(
            matches,
            "inference-key-env",
            "Credential environment variable NAME (not the secret)",
            no_interact,
        )?;
        Self::validate(project, &source, &executable, base_url, model, api_key_env)
    }

    fn validate(
        project: &Path,
        source: &str,
        executable: &str,
        base_url: String,
        model: String,
        api_key_env: String,
    ) -> Result<Self, String> {
        let project = project.canonicalize().map_err(|e| e.to_string())?;
        if project
            .to_str()
            .ok_or("control --dir must be UTF-8")?
            .chars()
            .any(char::is_control)
        {
            return Err("control --dir cannot contain control characters".into());
        }
        let source = Path::new(source)
            .canonicalize()
            .map_err(|e| format!("cannot resolve --source: {e}"))?;
        validate_source(&project, &source)?;
        let executable = Path::new(executable)
            .canonicalize()
            .map_err(|e| format!("cannot resolve --managed-executable: {e}"))?;
        let mut profile = LandlockProfile::default();
        profile.allow_executable(&executable)?;
        if !api_key_env
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_uppercase() || b == b'_')
        {
            return Err(
                "--inference-key-env must be an uppercase shell variable name, not a credential"
                    .into(),
            );
        }
        if matches!(api_key_env.as_str(), "SYMBIONT_MASTER_KEY" | "RUST_LOG") {
            return Err("--inference-key-env must name a separate provider credential, not a generated runtime setting".into());
        }
        let inference = InferenceBrokerConfig {
            base_url,
            model,
            api_key_env,
            max_requests: 32,
            max_output_tokens_per_request: 4096,
            request_timeout_seconds: 60,
            beta_headers: vec![],
        };
        inference.validate()?;
        let settings = Self {
            source,
            executable,
            inference,
        };
        // Reuse the runtime's protected-root and kernel checks before any
        // project files are emitted. Doctor separately checks worker startup.
        let boundary: CommandBoundary = serde_json::from_value(json!({
            "tier": "landlock", "roots": settings.roots(),
        }))
        .map_err(|e| e.to_string())?;
        boundary.validate()?;
        Ok(settings)
    }

    fn roots(&self) -> serde_json::Value {
        json!({ "source_roots": [format!("{}:/workspace:ro", self.source.display())], "output_roots": [] })
    }

    pub(super) fn write_configuration(&self, project: &Path) -> Result<(), String> {
        let path = project.join("symbiont.toml");
        let mut config = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let additions = json!({
            "sandbox": {"roots": self.roots()},
            "managed_cli": {"executable": self.executable, "inference": self.inference},
        });
        config.push('\n');
        config.push_str(&toml::to_string_pretty(&additions).map_err(|e| e.to_string())?);
        std::fs::write(path, config).map_err(|e| e.to_string())?;

        let path = project.join(".env.example");
        let mut example = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let name = &self.inference.api_key_env;
        if !example
            .lines()
            .any(|line| line.starts_with(&format!("{name}=")))
        {
            example.push_str(&format!(
                "\n# Managed inference credential; read only by the runtime.\n{name}=\n"
            ));
            std::fs::write(path, example).map_err(|e| e.to_string())?;
        }
        std::fs::write(project.join("DEVELOPMENT.md"), self.guide(project))
            .map_err(|e| e.to_string())?;
        println!("\u{2713} Configured read-only source access and managed inference");
        println!("\u{2713} Created DEVELOPMENT.md");
        Ok(())
    }

    fn guide(&self, project: &Path) -> String {
        format!(
            "# Repository review\n\n\
             This control project grants read-only access to `{source}` through\n\
             `read_file`, `list_files` and `grep_files`. The contained CLI sees\n\
             private scratch, with no direct source mount or host networking.\n\
             No write tools, output roots, command tools or shell policies are enabled.\n\n\
             1. Set `{key}` in your shell or this project's private `.env`.\n\
                Use the credential required by your configured Messages-compatible\n\
                endpoint. It stays in the runtime. Never commit `.env`.\n\
             2. Run the following commands from this control project:\n\n\
             ```sh\ncd {project}\nsymbi doctor\n{run}\n```\n\n\
             `doctor` checks isolation, cleanup and CLI startup without contacting\n\
             the provider. The review sends governed source results to the configured\n\
             provider. A valid endpoint, model and credential are needed for that step.\n\n\
             Linux needs Landlock ABI 6+, unprivileged user/mount/network namespaces,\n\
             cgroup v2 delegation, a systemd user manager and Python 3 at\n\
             `/usr/bin/python3` with `os.pidfd_open`. The runtime starts supervision\n\
             automatically. No Docker or VM is required.\n\n\
             Keep this control project outside the source tree. Paths were resolved\n\
             at initialization; moving either project requires updating configuration\n\
             and the run target. Additional tools or writes require explicit\n\
             configuration and policy; see the Linux development guide.\n\n\
             Each review prints its signed journal and public key. Preserve the key\n\
             through a trusted channel for verification.\n",
            source = self.source.display(),
            key = self.inference.api_key_env,
            project = shell_quote(
                &project
                    .canonicalize()
                    .unwrap_or_else(|_| project.into())
                    .to_string_lossy()
            ),
            run = self.run_command(),
        )
    }

    fn run_command(&self) -> String {
        format!(
            "symbi run dev --target {} --input 'Review this repository using the source tools. Report findings without changing files.'",
            shell_quote(&self.source.to_string_lossy())
        )
    }

    pub(super) fn print_next_steps(&self, project: &Path) {
        let project = project.canonicalize().unwrap_or_else(|_| project.into());
        println!("\nNext steps:");
        println!("  cd {}", shell_quote(&project.to_string_lossy()));
        println!(
            "  Set {} in your shell or .env (credential stays in the runtime)",
            self.inference.api_key_env
        );
        println!("  symbi dsl --check -f agents/dev.symbi");
        println!("  symbi doctor");
        println!("  {}", self.run_command());
        println!("  Guide: DEVELOPMENT.md");
    }
}

fn validate_source(project: &Path, source: &Path) -> Result<(), String> {
    if !source.is_dir() {
        return Err("--source must be an existing directory".into());
    }
    if source.starts_with(project) || project.starts_with(source) {
        return Err(
            "--source and the control --dir must be separate, non-overlapping directories".into(),
        );
    }
    let text = source.to_str().ok_or("--source must be UTF-8")?;
    if text.contains(':') || text.chars().any(char::is_control) {
        return Err("--source cannot contain colons or control characters".into());
    }
    Ok(())
}

fn setting(
    matches: &ArgMatches,
    name: &str,
    label: &str,
    no_interact: bool,
) -> Result<String, String> {
    if let Some(value) = matches.get_one::<String>(name) {
        return Ok(value.clone());
    }
    #[cfg(feature = "interactive")]
    if !no_interact {
        return dialoguer::Input::<String>::new()
            .with_prompt(label)
            .interact_text()
            .map_err(|e| format!("cannot read --{name}: {e}; pass it as a flag"));
    }
    let _ = (label, no_interact);
    Err(format!("supply --{name}"))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

pub(super) fn write_templates(project: &Path) -> Result<(), String> {
    std::fs::create_dir_all(project.join("policies/managed-cli")).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(project.join("tools")).map_err(|e| e.to_string())?;
    std::fs::write(project.join("agents/dev.symbi"), AGENT).map_err(|e| e.to_string())?;
    std::fs::write(project.join("policies/managed-cli/dev-agent.cedar"), POLICY)
        .map_err(|e| e.to_string())?;
    for (name, content) in TOOLS {
        std::fs::write(project.join(format!("tools/{name}.clad.toml")), content)
            .map_err(|e| e.to_string())?;
    }
    println!("\u{2713} Created dev agent, read/list/search manifests and managed CLI policy");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_and_control_cannot_overlap() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("control");
        let source = root.path().join("source");
        std::fs::create_dir(&project).unwrap();
        std::fs::create_dir(&source).unwrap();
        assert!(validate_source(&project, &source).is_ok());
        assert!(validate_source(&project, &project).is_err());
        assert!(validate_source(&project, root.path()).is_err());
        assert!(validate_source(root.path(), &source).is_err());
    }

    #[test]
    fn configuration_and_command_preserve_literal_paths() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source '$`\" directory");
        let project = root.path().join("control");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(&project).unwrap();
        super::super::init_project(&project, "dev-agent", "tofu", "landlock", None);
        super::super::write_env_files(&project);
        let settings = Settings {
            source: source.clone(),
            executable: "/usr/bin/python3".into(),
            inference: InferenceBrokerConfig {
                base_url: "http://localhost:8000".into(),
                model: "model\"quoted".into(),
                api_key_env: "REVIEW_PROVIDER_KEY".into(),
                max_requests: 32,
                max_output_tokens_per_request: 4096,
                request_timeout_seconds: 60,
                beta_headers: vec![],
            },
        };
        settings.write_configuration(&project).unwrap();
        let text = std::fs::read_to_string(project.join("symbiont.toml")).unwrap();
        let config: toml::Value = toml::from_str(&text).unwrap();
        assert_eq!(
            config["sandbox"]["roots"]["source_roots"][0]
                .as_str()
                .unwrap(),
            format!("{}:/workspace:ro", source.display())
        );
        assert_eq!(
            config["sandbox"]["roots"]["output_roots"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
        assert_eq!(
            InferenceBrokerConfig::from_project(&project).unwrap().model,
            "model\"quoted"
        );
        // Exercise actual shell parsing, with a fixed printf and no evaluation
        // of the quoted path's command-substitution or expansion characters.
        let output = std::process::Command::new("/bin/sh")
            .args([
                "-c",
                &format!("printf '%s' {}", shell_quote(source.to_str().unwrap())),
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, source.to_str().unwrap().as_bytes());
        let tree = dsl::parse_dsl(AGENT).unwrap();
        assert!(!tree.root_node().has_error());
        assert!(dsl::resolve_execution_settings(AGENT, "dev").is_ok());
    }

    #[cfg(feature = "cedar")]
    #[test]
    fn development_policy_denies_effects_outside_source_queries() {
        use cedar_policy::{Authorizer, Context, Decision, Entities, PolicySet, Request};
        let policies: PolicySet = POLICY.parse().unwrap();
        for (action, permitted) in [
            ("claude_code", true),
            ("read_file", true),
            ("list_files", true),
            ("grep_files", true),
            ("write_file", false),
            ("edit_file", false),
            ("Bash", false),
            ("git_diff", false),
        ] {
            let request = Request::new(
                "Agent::\"dev\"".parse().unwrap(),
                format!("Action::\"tool_call::{action}\"").parse().unwrap(),
                "Resource::\"source\"".parse().unwrap(),
                Context::empty(),
                None,
            )
            .unwrap();
            assert_eq!(
                Authorizer::new()
                    .is_authorized(&request, &policies, &Entities::empty())
                    .decision()
                    == Decision::Allow,
                permitted,
                "{action}"
            );
        }
    }
}
