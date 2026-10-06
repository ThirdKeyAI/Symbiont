#![allow(clippy::multiple_crate_versions)]

use clap::{Arg, ArgAction, Command};

mod commands;
mod mcp_server;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    #[cfg(target_os = "linux")]
    if std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new(
            symbi_runtime::sandbox::landlock::workspace::INTERNAL_COMMAND,
        ))
    {
        let arguments: Vec<_> = std::env::args_os().skip(2).collect();
        let result = if arguments.len() == 1 {
            symbi_runtime::sandbox::landlock::workspace::run(std::path::Path::new(&arguments[0]))
        } else {
            Err(anyhow::anyhow!("expected one native launch contract"))
        };
        if let Err(error) = result {
            eprintln!("native workspace setup failed: {error:#}");
        }
        std::process::exit(125);
    }
    if std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new(
            symbi_runtime::sandbox::supervisor::INTERNAL_COMMAND,
        ))
    {
        let arguments = std::env::args().skip(2).collect::<Vec<_>>();
        let result = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|error| error.to_string())
            .and_then(|runtime| {
                runtime
                    .block_on(symbi_runtime::sandbox::supervisor::run_service(&arguments))
                    .map_err(|e| e.to_string())
            });
        if let Err(error) = result {
            eprintln!("sandbox supervisor failed: {error}");
            std::process::exit(1);
        }
        return;
    }
    if let Err(error) = symbi_runtime::sandbox::supervisor::use_embedded_helper() {
        eprintln!("cannot locate sandbox supervisor executable: {error}");
        std::process::exit(1);
    }
    app_main();
}

/// Locate a `.env` without letting the search escape the project.
///
/// `dotenvy::dotenv()` walks to the filesystem root, so running symbi anywhere
/// picks up the nearest ancestor `.env` -- potentially one belonging to an
/// unrelated tree, silently supplying credentials the caller never chose. Bound
/// the search the way the runtime bounds everything else: to the project, named
/// by the `symbiont.toml` that `symbi init` writes. With no project above the
/// current directory, only that directory is considered.
fn project_env_file() -> Option<std::path::PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let root = cwd
        .ancestors()
        .find(|dir| dir.join("symbiont.toml").is_file());
    let mut search: Vec<&std::path::Path> = Vec::new();
    match root {
        Some(root) => {
            for dir in cwd.ancestors() {
                search.push(dir);
                if dir == root {
                    break;
                }
            }
        }
        None => search.push(cwd.as_path()),
    }
    search.into_iter().find_map(|dir| {
        let candidate = dir.join(".env");
        candidate.is_file().then_some(candidate)
    })
}

#[tokio::main]
async fn app_main() {
    // Load a project-local `.env` (e.g. the one `symbi init` generates with
    // SYMBIONT_MASTER_KEY) before anything reads the environment. Variables
    // already set in the real environment take precedence -- from_path never
    // overrides them -- so this is additive. We announce it on stderr rather
    // than loading secrets silently.
    if let Some(path) = project_env_file() {
        match dotenvy::from_path(&path) {
            Ok(()) => eprintln!("Loaded environment from {}", path.display()),
            Err(error) => {
                eprintln!("Ignored unreadable {}: {error}", path.display());
            }
        }
    }

    let matches = Command::new("symbi")
        .version(VERSION)
        .about("Symbiont - AI Agent Runtime and DSL")
        .subcommand(
            Command::new("up")
                .about("Start the Symbiont runtime with auto-configuration")
                .arg(
                    Arg::new("port")
                        .short('p')
                        .long("port")
                        .value_name("PORT")
                        .help("Runtime API port")
                        .default_value("8080"),
                )
                .arg(
                    Arg::new("http-port")
                        .long("http-port")
                        .value_name("HTTP_PORT")
                        .help("HTTP Input port")
                        .default_value("8081"),
                )
                .arg(
                    Arg::new("http-bind")
                        .long("http-bind")
                        .value_name("ADDRESS")
                        .help("HTTP bind address (default: 127.0.0.1; use 0.0.0.0 for all interfaces)")
                        .default_value("127.0.0.1"),
                )
                .arg(
                    Arg::new("http-token")
                        .long("http.token")
                        .value_name("TOKEN")
                        .help("Bearer token for HTTP authentication (use 'env:VAR' for environment variable)"),
                )
                .arg(
                    Arg::new("http-cors-origins")
                        .long("http.cors-origins")
                        .value_name("ORIGINS")
                        .help("Comma-separated CORS origin allow-list (e.g. 'https://app.example.com,https://staging.example.com' or '*' for permissive)"),
                )
                .arg(
                    Arg::new("http-audit")
                        .long("http.audit")
                        .action(ArgAction::SetTrue)
                        .help("Log all HTTP requests to audit log"),
                )
                .arg(
                    Arg::new("preset")
                        .long("preset")
                        .value_name("PRESET")
                        .help("Use a configuration preset (e.g., dev-simple)"),
                )
                .arg(
                    Arg::new("slack-token")
                        .long("slack.token")
                        .value_name("TOKEN")
                        .help("Slack bot token (xoxb-...) — enables Slack adapter"),
                )
                .arg(
                    Arg::new("slack-signing-secret")
                        .long("slack.signing-secret")
                        .value_name("SECRET")
                        .help("Slack signing secret for webhook signature verification"),
                )
                .arg(
                    Arg::new("slack-port")
                        .long("slack.port")
                        .value_name("PORT")
                        .help("Slack webhook server port")
                        .default_value("3100"),
                )
                .arg(
                    Arg::new("slack-agent")
                        .long("slack.agent")
                        .value_name("AGENT")
                        .help("Default agent name for Slack messages"),
                )
                // Teams adapter flags (enterprise)
                .arg(
                    Arg::new("teams-tenant-id")
                        .long("teams.tenant-id")
                        .value_name("ID")
                        .help("Azure AD tenant ID — enables Teams adapter"),
                )
                .arg(
                    Arg::new("teams-client-id")
                        .long("teams.client-id")
                        .value_name("ID")
                        .help("Azure AD application (client) ID"),
                )
                .arg(
                    Arg::new("teams-client-secret")
                        .long("teams.client-secret")
                        .value_name("SECRET")
                        .help("Azure AD client secret"),
                )
                .arg(
                    Arg::new("teams-bot-id")
                        .long("teams.bot-id")
                        .value_name("ID")
                        .help("Bot Framework bot ID"),
                )
                .arg(
                    Arg::new("teams-port")
                        .long("teams.port")
                        .value_name("PORT")
                        .help("Teams webhook server port")
                        .default_value("3200"),
                )
                .arg(
                    Arg::new("teams-agent")
                        .long("teams.agent")
                        .value_name("AGENT")
                        .help("Default agent name for Teams messages"),
                )
                // Mattermost adapter flags (enterprise)
                .arg(
                    Arg::new("mm-server-url")
                        .long("mm.server-url")
                        .value_name("URL")
                        .help("Mattermost server URL — enables Mattermost adapter"),
                )
                .arg(
                    Arg::new("mm-token")
                        .long("mm.token")
                        .value_name("TOKEN")
                        .help("Mattermost bot token"),
                )
                .arg(
                    Arg::new("mm-webhook-secret")
                        .long("mm.webhook-secret")
                        .value_name("SECRET")
                        .help("Mattermost webhook secret for verification"),
                )
                .arg(
                    Arg::new("mm-port")
                        .long("mm.port")
                        .value_name("PORT")
                        .help("Mattermost webhook server port")
                        .default_value("3300"),
                )
                .arg(
                    Arg::new("mm-agent")
                        .long("mm.agent")
                        .value_name("AGENT")
                        .help("Default agent name for Mattermost messages"),
                )
                .arg(
                    Arg::new("serve-agents-md")
                        .long("serve-agents-md")
                        .action(ArgAction::SetTrue)
                        .help("Serve AGENTS.md at /agents.md and /.well-known/agents.md (auth-gated)"),
                )
                .arg(
                    Arg::new("insecure-allow-all")
                        .long("insecure-allow-all")
                        .action(ArgAction::SetTrue)
                        .help("DEV ONLY: switch the policy gate into permissive mode (allows every tool call and delegation). Also enabled by SYMBI_INSECURE_ALLOW_ALL=1. Do not use in production."),
                ),
        )
        .subcommand(
            Command::new("doctor")
                .about("Check system health and dependencies")
        )
        .subcommand(
            Command::new("audit")
                .about("Inspect protected run evidence")
                .subcommand_required(true)
                .subcommand(Command::new("inspect")
                    .about("Verify a run and report incomplete or unknown outcomes without replaying work")
                    .arg(Arg::new("journal").required(true).value_name("JOURNAL"))
                    .arg(Arg::new("public-key").long("public-key").required(true).value_name("HEX")
                        .help("Public audit key retained through a trusted channel"))
                    .arg(Arg::new("run-id").long("run-id").required(true).value_name("UUID")
                        .help("Expected invocation ID retained through a trusted channel"))),
        )
        .subcommand(commands::invocation::command())
        .subcommand(commands::improvement::command())
        .subcommand(
            Command::new("logs")
                .about("Show runtime logs")
                .arg(
                    Arg::new("follow")
                        .short('f')
                        .long("follow")
                        .action(ArgAction::SetTrue)
                        .help("Follow log output in real-time"),
                )
                .arg(
                    Arg::new("lines")
                        .short('n')
                        .long("lines")
                        .value_name("LINES")
                        .help("Number of recent log lines to show")
                        .default_value("50"),
                ),
        )
        .subcommand(
            Command::new("status")
                .about("Show running agents, routes, and I/O handlers")
        )
        .subcommand(
            Command::new("new")
                .about("Create a new project from a template")
                .arg(
                    Arg::new("template")
                        .value_name("TEMPLATE")
                        .help("Template name (webhook-min, webscraper-agent, slm-first, rag-lite)")
                        .required_unless_present("list"),
                )
                .arg(
                    Arg::new("name")
                        .value_name("PROJECT_NAME")
                        .help("Project name")
                        .required(false),
                )
                .arg(
                    Arg::new("list")
                        .long("list")
                        .action(ArgAction::SetTrue)
                        .help("List available templates"),
                ),
        )
        .subcommand(
            Command::new("init")
                .about("Create a new Symbiont project (symbiont.toml, agents/, policies/, docker-compose.yml, .env)")
                .display_order(0)
                .arg(Arg::new("profile").long("profile").value_name("PROFILE").help("Project profile (minimal, assistant, dev-agent, multi-agent)"))
                .arg(Arg::new("schemapin").long("schemapin").value_name("MODE").help("SchemaPin verification mode (tofu, strict, disabled)").default_value("tofu"))
                .arg(Arg::new("sandbox").long("sandbox").value_name("TIER").help("Sandbox isolation: landlock (Linux ABI 6+, systemd user delegation), tier0 (none, dev only), tier1 (Docker), tier2 (gVisor), tier3 (Firecracker)").default_value("tier1"))
                .arg(Arg::new("source").long("source").value_name("PATH").help("Repository to grant read-only access (--profile dev-agent --sandbox landlock; separate from --dir)"))
                .arg(Arg::new("managed-executable").long("managed-executable").value_name("PATH").help("Installed Claude Code executable for the Landlock dev-agent profile"))
                .arg(Arg::new("inference-url").long("inference-url").value_name("URL").help("Explicit Anthropic Messages-compatible base URL for the Landlock dev-agent profile"))
                .arg(Arg::new("inference-model").long("inference-model").value_name("MODEL").help("Model served by the managed inference endpoint"))
                .arg(Arg::new("inference-key-env").long("inference-key-env").value_name("NAME").help("Host credential variable name, never its value, for managed inference"))
                .arg(Arg::new("firecracker-kernel").long("firecracker-kernel").value_name("PATH").help("Path to a Firecracker-bootable vmlinux ELF (required when --sandbox=tier3). See docs/firecracker-setup.md."))
                .arg(Arg::new("firecracker-rootfs").long("firecracker-rootfs").value_name("PATH").help("Path to a Firecracker root filesystem image (ext4) implementing the in-VM init contract (required when --sandbox=tier3). See docs/firecracker-setup.md."))
                .arg(Arg::new("dir").long("dir").value_name("PATH").help("Target directory (default: current directory; useful inside Docker with -v $(pwd):/workspace --dir /workspace)"))
                .arg(Arg::new("force").long("force").action(ArgAction::SetTrue).help("Overwrite existing symbiont.toml"))
                .arg(Arg::new("no-interact").long("no-interact").action(ArgAction::SetTrue).help("Skip interactive prompts (use defaults or --flags)"))
                .arg(Arg::new("no-docker-compose").long("no-docker-compose").action(ArgAction::SetTrue).help("Skip generating docker-compose.yml"))
                .arg(
                    Arg::new("catalog")
                        .long("catalog")
                        .value_name("AGENTS")
                        .help("Comma-separated agent names to import, or 'list' to show available")
                )
        )
        .subcommand(
            Command::new("run")
                .about("Run a single agent and exit")
                .arg(Arg::new("improvement").long("improvement").value_name("WORKFLOW")
                    .help("Explicitly select an enabled, approved workflow instruction version"))
                .arg(Arg::new("improvement-trial").long("improvement-trial").value_name("SHA256")
                    .requires("improvement").help("Explicit evaluation run of this unpromoted candidate; normal execution controls still apply"))
                .arg(
                    Arg::new("approval-terminal")
                        .long("approval-terminal")
                        .action(ArgAction::SetTrue)
                        .help("Request mandatory tool approvals on the operator's controlling terminal"),
                )
                .arg(
                    Arg::new("approval-timeout")
                        .long("approval-timeout")
                        .value_name("SECONDS")
                        .value_parser(clap::value_parser!(u64).range(1..=3600))
                        .requires("approval-terminal")
                        .help("Time to display and answer each terminal approval (default: 120 seconds)"),
                )
                .arg(
                    Arg::new("agent")
                        .value_name("AGENT")
                        .help("Agent name or DSL file path (searches agents/ directory)")
                        .required(true),
                )
                .arg(
                    Arg::new("input")
                        .long("input")
                        .short('i')
                        .value_name("JSON")
                        .help("Input data as JSON string")
                        .default_value("{}"),
                )
                .arg(
                    Arg::new("max-iterations")
                        .long("max-iterations")
                        .value_name("N")
                        .help("Maximum ORGA loop iterations")
                        .default_value("10"),
                )
                .arg(
                    Arg::new("invocation-id")
                        .long("invocation-id")
                        .value_name("UUID")
                        .help("Ordinary ORGA run identity: reuse this ID to retrieve a result without repeating work"),
                )
                .arg(
                    Arg::new("target")
                        .long("target")
                        .value_name("DIR")
                        .help("Managed tool working directory: mapped host path for Docker/gVisor; absolute guest path for Firecracker (default: configured guest directory)"),
                )
                .arg(
                    Arg::new("max-turns")
                        .long("max-turns")
                        .value_name("N")
                        .help("Managed CLI: max agentic turns (primary bound, default 12)"),
                )
                .arg(
                    Arg::new("budget-timeout")
                        .long("budget-timeout")
                        .value_name("DURATION")
                        .help(
                            "Managed CLI: wall-clock budget backstop, e.g. 15m or 900s (default 15m)",
                        ),
                )
                .arg(
                    Arg::new("budget-tokens")
                        .long("budget-tokens")
                        .value_name("N")
                        .help(
                            "Managed CLI: total reserved inference output-token allowance (default 100000)",
                        ),
                )
                .arg(
                    Arg::new("plugin-dir")
                        .long("plugin-dir")
                        .value_name("DIR")
                        .help(
                            "Legacy managed CLI plugin option; brokered runs reject plugin loading",
                        ),
                ),
        )
        .subcommand(
            Command::new("tools")
                .about("Manage ToolClad tool manifests")
                .subcommand(Command::new("list").about("List all discovered tools"))
                .subcommand(
                    Command::new("validate")
                        .about("Validate tool manifest(s)")
                        .arg(Arg::new("file").value_name("FILE").help("Specific .clad.toml file (default: all in tools/)")),
                )
                .subcommand(
                    Command::new("test")
                        .about("Dry-run a tool invocation")
                        .arg(Arg::new("name").value_name("NAME").required(true).help("Tool name or manifest file"))
                        .arg(Arg::new("arg").long("arg").value_name("KEY=VALUE").action(ArgAction::Append).help("Tool argument")),
                )
                .subcommand(
                    Command::new("schema")
                        .about("Output MCP JSON Schema for a tool")
                        .arg(Arg::new("name").value_name("NAME").required(true).help("Tool name or manifest file")),
                )
                .subcommand(
                    Command::new("init")
                        .about("Create a starter .clad.toml manifest")
                        .arg(Arg::new("name").value_name("NAME").required(true).help("Tool name (creates tools/<name>.clad.toml)")),
                ),
        )
        .subcommand(
            Command::new("mcp")
                .about("Start MCP server (stdio transport) for AI assistant integration"),
        )
        .subcommand(
            Command::new("agents-md")
                .about("Generate AGENTS.md from agent DSL definitions")
                .subcommand(
                    Command::new("generate")
                        .about("Scan agents/*.dsl and generate AGENTS.md")
                        .arg(
                            Arg::new("dir")
                                .long("dir")
                                .value_name("PATH")
                                .help("Project directory containing agents/ folder")
                                .default_value("."),
                        )
                        .arg(
                            Arg::new("output")
                                .long("output")
                                .value_name("FILE")
                                .help("Output filename")
                                .default_value("AGENTS.md"),
                        ),
                ),
        )
        .subcommand(
            Command::new("dsl")
                .about("Parse and execute DSL")
                .arg(
                    Arg::new("file")
                        .short('f')
                        .long("file")
                        .value_name("FILE")
                        .help("DSL file to parse and execute"),
                )
                .arg(
                    Arg::new("content")
                        .short('c')
                        .long("content")
                        .value_name("CONTENT")
                        .help("DSL content to parse directly"),
                )
                .arg(
                    Arg::new("check")
                        .long("check")
                        .action(ArgAction::SetTrue)
                        .help("Validate only: print one line per file and set the exit code"),
                ),
        )
        .subcommand(
            Command::new("fmt")
                .about("Canonical-format Symbi (.symbi) source files")
                .arg(
                    Arg::new("check")
                        .long("check")
                        .action(ArgAction::SetTrue)
                        .help("Exit non-zero if any file needs formatting; do not write changes"),
                )
                .arg(
                    Arg::new("stdin")
                        .long("stdin")
                        .action(ArgAction::SetTrue)
                        .help("Read source from stdin, write formatted output to stdout"),
                )
                .arg(
                    Arg::new("files")
                        .help(".symbi files to format (rewritten in place by default)")
                        .num_args(0..)
                        .value_name("FILES"),
                ),
        )
        .subcommand(
            Command::new("chat")
                .about("Manage chat channel adapters (Slack, Teams, Mattermost)")
                .subcommand(
                    Command::new("connect")
                        .about("Connect a chat adapter")
                        .subcommand(
                            Command::new("slack")
                                .about("Connect to a Slack workspace")
                                .arg(
                                    Arg::new("token")
                                        .long("token")
                                        .value_name("TOKEN")
                                        .help("Slack bot token (xoxb-...)")
                                        .required(true),
                                )
                                .arg(
                                    Arg::new("port")
                                        .long("port")
                                        .value_name("PORT")
                                        .help("Webhook server port (default: 3100)")
                                        .default_value("3100"),
                                )
                                .arg(
                                    Arg::new("agent")
                                        .long("agent")
                                        .value_name("AGENT")
                                        .help("Default agent to invoke"),
                                ),
                        )
                        .subcommand(
                            Command::new("teams")
                                .about("Connect to Microsoft Teams (enterprise)")
                                .arg(
                                    Arg::new("tenant-id")
                                        .long("tenant-id")
                                        .value_name("ID")
                                        .help("Azure AD tenant ID")
                                        .required(true),
                                )
                                .arg(
                                    Arg::new("client-id")
                                        .long("client-id")
                                        .value_name("ID")
                                        .help("Azure AD application (client) ID")
                                        .required(true),
                                )
                                .arg(
                                    Arg::new("client-secret")
                                        .long("client-secret")
                                        .value_name("SECRET")
                                        .help("Azure AD client secret")
                                        .required(true),
                                )
                                .arg(
                                    Arg::new("bot-id")
                                        .long("bot-id")
                                        .value_name("ID")
                                        .help("Bot Framework bot ID"),
                                ),
                        )
                        .subcommand(
                            Command::new("mattermost")
                                .about("Connect to Mattermost (enterprise)")
                                .arg(
                                    Arg::new("server-url")
                                        .long("server-url")
                                        .value_name("URL")
                                        .help("Mattermost server URL")
                                        .required(true),
                                )
                                .arg(
                                    Arg::new("token")
                                        .long("token")
                                        .value_name("TOKEN")
                                        .help("Bot token")
                                        .required(true),
                                ),
                        ),
                )
                .subcommand(Command::new("status").about("Show connected adapters"))
                .subcommand(
                    Command::new("disconnect")
                        .about("Disconnect a chat adapter")
                        .subcommand(
                            Command::new("slack").about("Disconnect from Slack"),
                        )
                        .subcommand(
                            Command::new("teams").about("Disconnect from Teams"),
                        )
                        .subcommand(
                            Command::new("mattermost").about("Disconnect from Mattermost"),
                        ),
                ),
        )
        .subcommand(
            Command::new("skills")
                .about("Manage agent skills (load, verify, sign, scan)")
                .subcommand(Command::new("list").about("List loaded skills with verification status"))
                .subcommand(
                    Command::new("scan")
                        .about("Scan a skill directory for policy violations")
                        .arg(
                            Arg::new("dir")
                                .value_name("DIR")
                                .help("Skill directory to scan")
                                .required(true),
                        ),
                )
                .subcommand(
                    Command::new("verify")
                        .about("Verify a skill's cryptographic signature")
                        .arg(
                            Arg::new("dir")
                                .value_name("DIR")
                                .help("Skill directory to verify")
                                .required(true),
                        )
                        .arg(
                            Arg::new("domain")
                                .long("domain")
                                .value_name("DOMAIN")
                                .help("Expected signer domain")
                                .required(true),
                        ),
                )
                .subcommand(
                    Command::new("sign")
                        .about("Sign a skill folder with a private key")
                        .arg(
                            Arg::new("dir")
                                .value_name("DIR")
                                .help("Skill directory to sign")
                                .required(true),
                        )
                        .arg(
                            Arg::new("key")
                                .long("key")
                                .value_name("KEY_FILE")
                                .help("Path to PEM private key file")
                                .required(true),
                        )
                        .arg(
                            Arg::new("domain")
                                .long("domain")
                                .value_name("DOMAIN")
                                .help("Signer domain")
                                .required(true),
                        ),
                ),
        )
        .subcommand(
            Command::new("cron")
                .about("Manage cron-scheduled agent jobs")
                .subcommand(Command::new("list").about("List all scheduled jobs"))
                .subcommand(
                    Command::new("add")
                        .about("Create a new scheduled job")
                        .arg(
                            Arg::new("name")
                                .long("name")
                                .value_name("NAME")
                                .help("Job name")
                                .required(true),
                        )
                        .arg(
                            Arg::new("cron")
                                .long("cron")
                                .value_name("EXPR")
                                .help("Cron expression (e.g. \"0 */5 * * * * *\")")
                                .required(true),
                        )
                        .arg(
                            Arg::new("tz")
                                .long("tz")
                                .value_name("TIMEZONE")
                                .help("IANA timezone (default: UTC)")
                                .default_value("UTC"),
                        )
                        .arg(
                            Arg::new("agent")
                                .long("agent")
                                .value_name("AGENT")
                                .help("Agent name to execute")
                                .required(true),
                        )
                        .arg(
                            Arg::new("policy")
                                .long("policy")
                                .value_name("POLICY")
                                .help("Policy to apply"),
                        )
                        .arg(
                            Arg::new("one-shot")
                                .long("one-shot")
                                .action(ArgAction::SetTrue)
                                .help("Run once then disable"),
                        ),
                )
                .subcommand(
                    Command::new("remove")
                        .about("Delete a scheduled job")
                        .arg(Arg::new("job-id").required(true).help("Job UUID")),
                )
                .subcommand(
                    Command::new("pause")
                        .about("Pause a scheduled job")
                        .arg(Arg::new("job-id").required(true).help("Job UUID")),
                )
                .subcommand(
                    Command::new("resume")
                        .about("Resume a paused job")
                        .arg(Arg::new("job-id").required(true).help("Job UUID")),
                )
                .subcommand(
                    Command::new("status")
                        .about("Show job details and recent runs")
                        .arg(Arg::new("job-id").required(true).help("Job UUID")),
                )
                .subcommand(
                    Command::new("run")
                        .about("Force-trigger a job immediately")
                        .arg(Arg::new("job-id").required(true).help("Job UUID")),
                )
                .subcommand(
                    Command::new("history")
                        .about("Show run history")
                        .arg(
                            Arg::new("job")
                                .long("job")
                                .value_name("JOB_ID")
                                .help("Filter by job ID"),
                        )
                        .arg(
                            Arg::new("limit")
                                .long("limit")
                                .value_name("N")
                                .help("Max records to show")
                                .default_value("20"),
                        ),
                ),
        )
        .subcommand(
            Command::new("shell")
                .about("Interactive agent orchestration shell")
                // Forward trailing args verbatim to the symbi-shell binary
                // so flags like `--resume <id>` and `--list-sessions` work
                // via `symbi shell --resume …` as well as invoking the
                // binary directly.
                .trailing_var_arg(true)
                .arg(
                    clap::Arg::new("shell-args")
                        .num_args(0..)
                        .allow_hyphen_values(true)
                        .trailing_var_arg(true),
                ),
        )
        .subcommand(
            Command::new("repl")
                .about("Interactive DSL REPL (forwards to the repl-cli binary)")
                .trailing_var_arg(true)
                .arg(
                    clap::Arg::new("repl-args")
                        .num_args(0..)
                        .allow_hyphen_values(true)
                        .trailing_var_arg(true),
                ),
        )
        .subcommand(
            Command::new("schemapin")
                .about("TOFU integrity pinning for MCP server configs (used by SessionStart hooks)")
                .subcommand(
                    Command::new("verify")
                        .about("Verify the pinned hash for one or all MCP servers in .mcp.json")
                        .arg(
                            Arg::new("mcp-server")
                                .long("mcp-server")
                                .value_name("NAME")
                                .help("Server name from .mcp.json (omit to verify every server)"),
                        )
                        .arg(
                            Arg::new("config")
                                .long("config")
                                .value_name("PATH")
                                .help("Path to .mcp.json (default: ./.mcp.json)"),
                        ),
                )
                .subcommand(
                    Command::new("pin")
                        .about("Pin the current config hash for an MCP server")
                        .arg(
                            Arg::new("mcp-server")
                                .long("mcp-server")
                                .value_name("NAME")
                                .help("Server name from .mcp.json")
                                .required(true),
                        )
                        .arg(
                            Arg::new("config")
                                .long("config")
                                .value_name("PATH")
                                .help("Path to .mcp.json (default: ./.mcp.json)"),
                        )
                        .arg(
                            Arg::new("force")
                                .long("force")
                                .action(ArgAction::SetTrue)
                                .help("Overwrite an existing pin if the hash differs"),
                        ),
                )
                .subcommand(
                    Command::new("list")
                        .about("List all pinned MCP servers under ~/.symbiont/schemapin/mcp/"),
                )
                .subcommand(
                    Command::new("unpin")
                        .about("Remove the pin record for an MCP server")
                        .arg(
                            Arg::new("mcp-server")
                                .long("mcp-server")
                                .value_name("NAME")
                                .help("Server name to unpin")
                                .required(true),
                        ),
                ),
        )
        .subcommand(
            Command::new("policy")
                .about("Evaluate Cedar authorization policies against tool-call inputs")
                .subcommand(
                    Command::new("evaluate")
                        .about("Read a tool-call event and decide allow/deny against a policy directory")
                        .arg(
                            Arg::new("stdin")
                                .long("stdin")
                                .action(ArgAction::SetTrue)
                                .help("Read the JSON event from stdin"),
                        )
                        .arg(
                            Arg::new("input")
                                .long("input")
                                .value_name("FILE")
                                .help("Read the JSON event from FILE (alternative to --stdin)"),
                        )
                        .arg(
                            Arg::new("policies")
                                .long("policies")
                                .value_name("DIR")
                                .help("Directory containing .cedar policy files (default: ./policies)"),
                        )
                        .arg(
                            Arg::new("json")
                                .long("json")
                                .action(ArgAction::SetTrue)
                                .help("Emit only structured JSON to stdout (for programmatic use). Default: bare verdict on stdout, JSON on stderr."),
                        ),
                ),
        )
        .get_matches();

    match matches.subcommand() {
        Some(("up", sub_matches)) => {
            commands::up::run(sub_matches).await;
        }
        Some(("doctor", _sub_matches)) => {
            commands::doctor::run().await;
        }
        Some(("audit", sub_matches)) => {
            commands::audit::run(sub_matches);
        }
        Some(("invocation", sub_matches)) => commands::invocation::run(sub_matches).await,
        Some(("improvement", sub_matches)) => commands::improvement::run(sub_matches),
        Some(("logs", sub_matches)) => {
            commands::logs::run(sub_matches).await;
        }
        Some(("status", _sub_matches)) => {
            commands::status::run().await;
        }
        Some(("new", sub_matches)) => {
            commands::new::run(sub_matches).await;
        }
        Some(("init", sub_matches)) => {
            commands::init::run(sub_matches).await;
        }
        Some(("run", sub_matches)) => {
            commands::run::run(sub_matches).await;
        }
        Some(("mcp", _sub_matches)) => {
            mcp_server::start_mcp_server().await.unwrap_or_else(|e| {
                eprintln!("MCP server error: {}", e);
                std::process::exit(1);
            });
        }
        Some(("agents-md", sub_matches)) => {
            commands::agents_md::run(sub_matches);
        }
        Some(("fmt", sub_matches)) => {
            std::process::exit(commands::fmt::run(sub_matches));
        }
        Some(("dsl", sub_matches)) => {
            let source = if let Some(file) = sub_matches.get_one::<String>("file") {
                match std::fs::read_to_string(file) {
                    Ok(content) => (content, Some(file.as_str())),
                    Err(e) => {
                        eprintln!("Error reading file '{}': {}", file, e);
                        std::process::exit(1);
                    }
                }
            } else if let Some(content) = sub_matches.get_one::<String>("content") {
                (content.clone(), None)
            } else {
                eprintln!("Either --file or --content must be provided for DSL command");
                std::process::exit(1);
            };
            if sub_matches.get_flag("check") {
                std::process::exit(commands::dsl::check(&source.0, source.1));
            }
            commands::dsl::run(&source.0, source.1);
        }
        Some(("chat", sub_matches)) => {
            commands::chat::run(sub_matches).await;
        }
        Some(("skills", sub_matches)) => {
            commands::skills::run(sub_matches).await;
        }
        Some(("cron", sub_matches)) => {
            commands::cron::run(sub_matches).await;
        }
        Some(("tools", sub_matches)) => {
            commands::tools::run(sub_matches).await;
        }
        Some(("schemapin", sub_matches)) => {
            commands::schemapin::run(sub_matches).await;
        }
        Some(("policy", sub_matches)) => {
            commands::policy::run(sub_matches).await;
        }
        Some(("shell", sub_matches)) => {
            let exe_dir = std::env::current_exe()
                .expect("cannot determine executable path")
                .parent()
                .unwrap()
                .to_path_buf();
            let shell_bin = exe_dir.join("symbi-shell");
            if shell_bin.exists() {
                let forwarded: Vec<String> = sub_matches
                    .get_many::<String>("shell-args")
                    .map(|vals| vals.cloned().collect())
                    .unwrap_or_default();
                let status = std::process::Command::new(&shell_bin)
                    .args(forwarded)
                    .status()
                    .expect("failed to launch symbi-shell");
                std::process::exit(status.code().unwrap_or(1));
            } else {
                eprintln!("symbi-shell binary not found. Build with: cargo build -p symbi-shell");
                std::process::exit(1);
            }
        }
        Some(("repl", sub_matches)) => {
            let exe_dir = std::env::current_exe()
                .expect("cannot determine executable path")
                .parent()
                .unwrap()
                .to_path_buf();
            let repl_bin = exe_dir.join("repl-cli");
            if repl_bin.exists() {
                let forwarded: Vec<String> = sub_matches
                    .get_many::<String>("repl-args")
                    .map(|vals| vals.cloned().collect())
                    .unwrap_or_default();
                let status = std::process::Command::new(&repl_bin)
                    .args(forwarded)
                    .status()
                    .expect("failed to launch repl-cli");
                std::process::exit(status.code().unwrap_or(1));
            } else {
                eprintln!("repl-cli binary not found. Build with: cargo build -p repl-cli");
                std::process::exit(1);
            }
        }
        _ => {
            println!("Symbiont v{}", VERSION);
            println!("Use --help for available commands");
        }
    }
}
