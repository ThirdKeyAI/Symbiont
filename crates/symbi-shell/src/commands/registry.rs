//! Static registry of slash commands.
//!
//! Single source of truth for command completion UI (popup shown above
//! the input line). Each entry carries a one-line summary and a broad
//! category tag that the UI displays next to the command name.
//!
//! The richer, example-bearing help text shown on `<cmd> help` lives in
//! `super::intercept_help` — that table is intentionally separate so we
//! can iterate on phrasing without retraining users' muscle memory about
//! what a command is called.

/// A slash-command entry in the completion registry.
#[derive(Debug, Clone, Copy)]
pub struct SlashCommand {
    /// Including the leading slash, e.g. `/behavior`.
    pub name: &'static str,
    /// One-line summary shown next to the name in the completion popup.
    pub summary: &'static str,
    /// Broad category (rendered as `(category)`).
    pub category: &'static str,
}

/// All registered slash commands. Order within a category reflects the
/// logical grouping; the completion UI sorts by fuzzy-match score, so
/// declaration order only matters as a tie-breaker for equal scores.
pub const REGISTRY: &[SlashCommand] = &[
    // ── session ──────────────────────────────────────────────────────
    SlashCommand {
        name: "/help",
        summary: "Show help for the shell",
        category: "session",
    },
    SlashCommand {
        name: "/clear",
        summary: "Clear the visible output buffer",
        category: "session",
    },
    SlashCommand {
        name: "/quit",
        summary: "Exit the shell",
        category: "session",
    },
    SlashCommand {
        name: "/exit",
        summary: "Exit the shell (or leave DSL mode)",
        category: "session",
    },
    SlashCommand {
        name: "/dsl",
        summary: "Toggle DSL input mode",
        category: "session",
    },
    SlashCommand {
        name: "/model",
        summary: "Show or switch the active inference model",
        category: "session",
    },
    SlashCommand {
        name: "/cost",
        summary: "Show token / API cost totals for this session",
        category: "session",
    },
    SlashCommand {
        name: "/status",
        summary: "Show runtime + session status",
        category: "session",
    },
    SlashCommand {
        name: "/context",
        summary: "Show current context window / token usage",
        category: "session",
    },
    SlashCommand {
        name: "/compact",
        summary: "Compact conversation history to fit a budget",
        category: "session",
    },
    SlashCommand {
        name: "/snapshot",
        summary: "Save a session snapshot",
        category: "session",
    },
    SlashCommand {
        name: "/resume",
        summary: "Restore a saved snapshot into this session",
        category: "session",
    },
    SlashCommand {
        name: "/export",
        summary: "Export the current transcript to disk",
        category: "session",
    },
    SlashCommand {
        name: "/new",
        summary: "Start a new session, discarding the current one",
        category: "session",
    },
    SlashCommand {
        name: "/branch",
        summary: "Branch the current session (planned)",
        category: "session",
    },
    SlashCommand {
        name: "/copy",
        summary: "Copy the current session (planned)",
        category: "session",
    },
    // ── agents ───────────────────────────────────────────────────────
    SlashCommand {
        name: "/agents",
        summary: "List active agents",
        category: "agents",
    },
    SlashCommand {
        name: "/ask",
        summary: "Send a message to an agent and wait for the reply",
        category: "agents",
    },
    SlashCommand {
        name: "/send",
        summary: "Send a message to an agent without waiting",
        category: "agents",
    },
    SlashCommand {
        name: "/memory",
        summary: "Query an agent's memory",
        category: "agents",
    },
    SlashCommand {
        name: "/debug",
        summary: "Inspect an agent's internal state",
        category: "agents",
    },
    SlashCommand {
        name: "/pause",
        summary: "Pause the given agent",
        category: "agents",
    },
    SlashCommand {
        name: "/resume-agent",
        summary: "Resume a paused agent",
        category: "agents",
    },
    SlashCommand {
        name: "/stop",
        summary: "Stop the given agent",
        category: "agents",
    },
    SlashCommand {
        name: "/destroy",
        summary: "Destroy an agent and its state",
        category: "agents",
    },
    // ── authoring ────────────────────────────────────────────────────
    SlashCommand {
        name: "/spawn",
        summary: "Generate a Symbiont DSL agent from a description",
        category: "authoring",
    },
    SlashCommand {
        name: "/policy",
        summary: "Generate a validated Cedar policy",
        category: "authoring",
    },
    SlashCommand {
        name: "/tool",
        summary: "Generate a ToolClad manifest (.clad.toml)",
        category: "authoring",
    },
    SlashCommand {
        name: "/behavior",
        summary: "Generate a DSL behavior definition",
        category: "authoring",
    },
    SlashCommand {
        name: "/init",
        summary: "Scaffold a Symbiont project",
        category: "authoring",
    },
    // ── orchestration ────────────────────────────────────────────────
    SlashCommand {
        name: "/run",
        summary: "Start or re-run an agent / workflow",
        category: "orchestration",
    },
    SlashCommand {
        name: "/chain",
        summary: "Pipe agent outputs through a sequence",
        category: "orchestration",
    },
    SlashCommand {
        name: "/debate",
        summary: "Multi-agent debate on a topic",
        category: "orchestration",
    },
    SlashCommand {
        name: "/parallel",
        summary: "Run agents in parallel and aggregate",
        category: "orchestration",
    },
    SlashCommand {
        name: "/race",
        summary: "First successful agent reply wins",
        category: "orchestration",
    },
    SlashCommand {
        name: "/exec",
        summary: "Execute a shell command in the dev agent",
        category: "orchestration",
    },
    // ── operations ───────────────────────────────────────────────────
    SlashCommand {
        name: "/monitor",
        summary: "Stream live agent status",
        category: "ops",
    },
    SlashCommand {
        name: "/logs",
        summary: "Show recent agent logs",
        category: "ops",
    },
    SlashCommand {
        name: "/doctor",
        summary: "Diagnose the local runtime environment",
        category: "ops",
    },
    SlashCommand {
        name: "/audit",
        summary: "Show recent audit trail entries",
        category: "ops",
    },
    // ── scheduling ───────────────────────────────────────────────────
    SlashCommand {
        name: "/cron",
        summary: "Manage cron-scheduled agent runs",
        category: "scheduling",
    },
    // ── tools ────────────────────────────────────────────────────────
    SlashCommand {
        name: "/tools",
        summary: "Manage ToolClad tools available to agents",
        category: "tools",
    },
    SlashCommand {
        name: "/skills",
        summary: "Manage skills available to agents",
        category: "tools",
    },
    SlashCommand {
        name: "/verify",
        summary: "Verify a SchemaPin-signed artifact",
        category: "tools",
    },
    // ── channels ─────────────────────────────────────────────────────
    SlashCommand {
        name: "/channels",
        summary: "List registered channel adapters",
        category: "channels",
    },
    SlashCommand {
        name: "/connect",
        summary: "Register a new channel adapter",
        category: "channels",
    },
    SlashCommand {
        name: "/disconnect",
        summary: "Remove a channel adapter",
        category: "channels",
    },
    // ── secrets ──────────────────────────────────────────────────────
    SlashCommand {
        name: "/secrets",
        summary: "Manage secrets exposed to the runtime",
        category: "secrets",
    },
    // ── deploy ───────────────────────────────────────────────────────
    SlashCommand {
        name: "/deploy",
        summary: "Deploy the agent stack (local / cloudrun / aws)",
        category: "deploy",
    },
    // ── remote ───────────────────────────────────────────────────────
    SlashCommand {
        name: "/attach",
        summary: "Attach to a remote runtime over HTTP",
        category: "remote",
    },
    SlashCommand {
        name: "/detach",
        summary: "Detach from the currently attached runtime",
        category: "remote",
    },
];
