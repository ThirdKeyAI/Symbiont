//! CLI commands for managing cron-scheduled agent jobs.
//!
//! Provides `symbi cron list|add|remove|pause|resume|status|run|history`.

use clap::ArgMatches;

#[cfg(feature = "cron")]
use symbi_runtime::{CronJobDefinition, CronJobId, JobStore, SqliteJobStore};

/// Entry point for `symbi cron <subcommand>`.
pub async fn run(matches: &ArgMatches) {
    match matches.subcommand() {
        Some(("list", _sub)) => cmd_list().await,
        Some(("add", sub)) => cmd_add(sub).await,
        Some(("remove", sub)) => cmd_remove(sub).await,
        Some(("pause", sub)) => cmd_pause(sub).await,
        Some(("resume", sub)) => cmd_resume(sub).await,
        Some(("status", sub)) => cmd_status(sub).await,
        Some(("run", sub)) => cmd_run(sub).await,
        Some(("history", sub)) => cmd_history(sub).await,
        _ => {
            eprintln!("Unknown cron subcommand. Use --help for usage.");
            std::process::exit(1);
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

#[cfg(feature = "cron")]
async fn open_store() -> Result<SqliteJobStore, String> {
    let project = std::env::current_dir()
        .and_then(|p| p.canonicalize())
        .map_err(|e| e.to_string())?;
    let path = project.join(".symbiont/cron_jobs.db");
    let store =
        SqliteJobStore::open(&path).map_err(|e| format!("Failed to open job store: {e}"))?;
    store
        .bind_project(&project)
        .await
        .map_err(|e| e.to_string())?;
    Ok(store)
}

#[cfg(feature = "cron")]
fn selected_source(
    name: &str,
) -> Result<(String, String, symbi_runtime::types::SecurityTier), String> {
    use std::path::Path;
    if name.is_empty()
        || matches!(name, "." | "..")
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
    {
        return Err("agent must be a direct source filename without an extension".into());
    }
    let reader = symbi_runtime::integrations::mcp::project::ProjectReader::open(Path::new("."))?;
    let mut sources = Vec::new();
    for extension in [dsl::SYMBI_EXTENSION, dsl::LEGACY_DSL_EXTENSION] {
        let path = Path::new("agents").join(format!("{name}.{extension}"));
        match std::fs::symlink_metadata(&path) {
            Ok(_) => sources.push(reader.read_text(&path, 1024 * 1024)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    if sources.len() != 1 {
        return Err("agent must identify exactly one source file in agents/".into());
    }
    let source = sources.remove(0);
    let settings = dsl::resolve_execution_settings(&source, name)?;
    dsl::ExecutionPolicy::parse(&source, &settings.agent_name)?;
    let tier = super::up::resolve_security_tier(&source, &settings.agent_name)?;
    Ok((source, settings.agent_name, tier))
}

#[cfg(feature = "cron")]
fn parse_job_id(s: &str) -> Result<CronJobId, String> {
    s.parse::<CronJobId>()
        .map_err(|e| format!("Invalid job ID '{}': {}", s, e))
}

// ── Subcommands ──────────────────────────────────────────────────────────

async fn cmd_list() {
    #[cfg(feature = "cron")]
    {
        let store = match open_store().await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}", e);
                return;
            }
        };
        let jobs = match store.list_jobs(None).await {
            Ok(j) => j,
            Err(e) => {
                eprintln!("Failed to list jobs: {}", e);
                return;
            }
        };
        if jobs.is_empty() {
            println!("No scheduled jobs.");
            return;
        }
        println!(
            "{:<38} {:<20} {:<10} {:<20} {:<6}",
            "ID", "NAME", "STATUS", "NEXT RUN", "RUNS"
        );
        println!("{}", "-".repeat(100));
        for job in &jobs {
            let next = job
                .next_run
                .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_else(|| "—".to_string());
            println!(
                "{:<38} {:<20} {:<10} {:<20} {:<6}",
                job.job_id,
                truncate(&job.name, 18),
                format!("{:?}", job.status),
                next,
                job.run_count,
            );
        }
    }
    #[cfg(not(feature = "cron"))]
    {
        eprintln!("Cron feature is not enabled. Rebuild with --features cron");
    }
}

async fn cmd_add(matches: &ArgMatches) {
    #[cfg(feature = "cron")]
    {
        let name = matches.get_one::<String>("name").expect("name required");
        let cron_expr = matches.get_one::<String>("cron").expect("cron required");
        let tz = matches
            .get_one::<String>("tz")
            .map(|s| s.as_str())
            .unwrap_or("UTC");
        let agent_name = matches.get_one::<String>("agent").expect("agent required");
        let one_shot = matches.get_flag("one-shot");

        let (source, selected_name, security_tier) = match selected_source(agent_name) {
            Ok(selected) => selected,
            Err(error) => {
                eprintln!("Cannot select scheduled source: {error}");
                std::process::exit(1);
            }
        };
        let agent_config = symbi_runtime::types::AgentConfig {
            id: symbi_runtime::types::AgentId::new(),
            name: selected_name,
            dsl_source: source,
            execution_mode: symbi_runtime::types::ExecutionMode::Ephemeral,
            security_tier,
            resource_limits: symbi_runtime::types::ResourceLimits::default(),
            capabilities: vec![],
            policies: vec![],
            metadata: std::collections::HashMap::new(),
            priority: symbi_runtime::types::Priority::Normal,
        };

        let mut job = CronJobDefinition::new(
            name.clone(),
            cron_expr.clone(),
            tz.to_string(),
            agent_config,
        );
        job.one_shot = one_shot;

        if let Some(policy) = matches.get_one::<String>("policy") {
            job.policy_ids.push(policy.clone());
        }

        // Validate cron expression before saving.
        if let Err(e) = cron_expr.parse::<cron::Schedule>() {
            eprintln!("Invalid cron expression: {}", e);
            std::process::exit(1);
        }

        let store = match open_store().await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}", e);
                return;
            }
        };

        // Compute next_run.
        let tz_parsed: chrono_tz::Tz = match tz.parse() {
            Ok(t) => t,
            Err(_) => {
                eprintln!("Invalid timezone: {}", tz);
                std::process::exit(1);
            }
        };
        let now = chrono::Utc::now().with_timezone(&tz_parsed);
        if let Ok(schedule) = cron_expr.parse::<cron::Schedule>() {
            job.next_run = schedule
                .after(&now)
                .next()
                .map(|dt| dt.with_timezone(&chrono::Utc));
        }

        match store.save_job(&job).await {
            Ok(()) => {
                println!("Created job: {}", job.job_id);
                if let Some(nr) = job.next_run {
                    println!("  Next run: {}", nr.format("%Y-%m-%d %H:%M:%S UTC"));
                }
            }
            Err(e) => eprintln!("Failed to create job: {}", e),
        }
    }
    #[cfg(not(feature = "cron"))]
    {
        let _ = matches;
        eprintln!("Cron feature is not enabled. Rebuild with --features cron");
    }
}

async fn cmd_remove(matches: &ArgMatches) {
    #[cfg(feature = "cron")]
    {
        let id_str = matches
            .get_one::<String>("job-id")
            .expect("job-id required");
        let job_id = match parse_job_id(id_str) {
            Ok(id) => id,
            Err(e) => {
                eprintln!("{}", e);
                return;
            }
        };
        let store = match open_store().await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}", e);
                return;
            }
        };
        match store.delete_job(job_id).await {
            Ok(true) => println!("Removed job {}", job_id),
            Ok(false) => eprintln!("Job {} not found", job_id),
            Err(e) => eprintln!("Failed to remove job: {}", e),
        }
    }
    #[cfg(not(feature = "cron"))]
    {
        let _ = matches;
        eprintln!("Cron feature is not enabled. Rebuild with --features cron");
    }
}

async fn cmd_pause(matches: &ArgMatches) {
    #[cfg(feature = "cron")]
    {
        let id_str = matches
            .get_one::<String>("job-id")
            .expect("job-id required");
        let job_id = match parse_job_id(id_str) {
            Ok(id) => id,
            Err(e) => {
                eprintln!("{}", e);
                return;
            }
        };
        let store = match open_store().await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}", e);
                return;
            }
        };
        match store.get_job(job_id).await {
            Ok(Some(mut job)) => {
                job.status = symbi_runtime::CronJobStatus::Paused;
                job.enabled = false;
                job.updated_at = chrono::Utc::now();
                if let Err(e) = store.save_job(&job).await {
                    eprintln!("Failed to pause job: {}", e);
                } else {
                    println!("Paused job {}", job_id);
                }
            }
            Ok(None) => eprintln!("Job {} not found", job_id),
            Err(e) => eprintln!("Failed to get job: {}", e),
        }
    }
    #[cfg(not(feature = "cron"))]
    {
        let _ = matches;
        eprintln!("Cron feature is not enabled. Rebuild with --features cron");
    }
}

async fn cmd_resume(matches: &ArgMatches) {
    #[cfg(feature = "cron")]
    {
        let id_str = matches
            .get_one::<String>("job-id")
            .expect("job-id required");
        let job_id = match parse_job_id(id_str) {
            Ok(id) => id,
            Err(e) => {
                eprintln!("{}", e);
                return;
            }
        };
        let store = match open_store().await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}", e);
                return;
            }
        };
        #[cfg(unix)]
        match store.job_has_unresolved_occurrence(job_id).await {
            Ok(false) => {}
            Ok(true) => {
                eprintln!("Unresolved occurrences require reconciliation before resume");
                std::process::exit(1);
            }
            Err(error) => {
                eprintln!("Cannot check unresolved history: {error}");
                std::process::exit(1);
            }
        }
        match store.get_job(job_id).await {
            Ok(Some(mut job)) => {
                job.status = symbi_runtime::CronJobStatus::Active;
                job.enabled = true;
                job.updated_at = chrono::Utc::now();
                if let Err(e) = store.save_job(&job).await {
                    eprintln!("Failed to resume job: {}", e);
                } else {
                    println!("Resumed job {}", job_id);
                }
            }
            Ok(None) => eprintln!("Job {} not found", job_id),
            Err(e) => eprintln!("Failed to get job: {}", e),
        }
    }
    #[cfg(not(feature = "cron"))]
    {
        let _ = matches;
        eprintln!("Cron feature is not enabled. Rebuild with --features cron");
    }
}

async fn cmd_status(matches: &ArgMatches) {
    #[cfg(feature = "cron")]
    {
        let id_str = matches
            .get_one::<String>("job-id")
            .expect("job-id required");
        let job_id = match parse_job_id(id_str) {
            Ok(id) => id,
            Err(e) => {
                eprintln!("{}", e);
                return;
            }
        };
        let store = match open_store().await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}", e);
                return;
            }
        };
        match store.get_job(job_id).await {
            Ok(Some(job)) => {
                println!("Job:        {}", job.job_id);
                println!("Name:       {}", job.name);
                println!("Cron:       {}", job.cron_expression);
                println!("Timezone:   {}", job.timezone);
                println!("Status:     {:?}", job.status);
                println!("Enabled:    {}", job.enabled);
                println!("One-shot:   {}", job.one_shot);
                println!("Runs:       {}", job.run_count);
                println!("Failures:   {}", job.failure_count);
                println!(
                    "Created:    {}",
                    job.created_at.format("%Y-%m-%d %H:%M:%S UTC")
                );
                println!(
                    "Updated:    {}",
                    job.updated_at.format("%Y-%m-%d %H:%M:%S UTC")
                );
                if let Some(lr) = job.last_run {
                    println!("Last run:   {}", lr.format("%Y-%m-%d %H:%M:%S UTC"));
                }
                if let Some(nr) = job.next_run {
                    println!("Next run:   {}", nr.format("%Y-%m-%d %H:%M:%S UTC"));
                }

                // Show recent history.
                match store.get_run_history(job_id, 5).await {
                    Ok(history) if !history.is_empty() => {
                        println!("\nRecent runs:");
                        for run in &history {
                            let duration = run
                                .execution_time_ms
                                .map(|ms| format!("{}ms", ms))
                                .unwrap_or_else(|| "—".to_string());
                            println!(
                                "  {} | {} | {} | {}",
                                run.started_at.format("%Y-%m-%d %H:%M:%S"),
                                run.status,
                                duration,
                                run.error.as_deref().unwrap_or(""),
                            );
                        }
                    }
                    _ => {}
                }
            }
            Ok(None) => eprintln!("Job {} not found", job_id),
            Err(e) => eprintln!("Failed to get job: {}", e),
        }
    }
    #[cfg(not(feature = "cron"))]
    {
        let _ = matches;
        eprintln!("Cron feature is not enabled. Rebuild with --features cron");
    }
}

async fn cmd_run(matches: &ArgMatches) {
    #[cfg(feature = "cron")]
    {
        let id_str = matches
            .get_one::<String>("job-id")
            .expect("job-id required");
        let job_id = match parse_job_id(id_str) {
            Ok(id) => id,
            Err(e) => {
                eprintln!("{}", e);
                return;
            }
        };
        // Force-triggering requires a running CronScheduler (with an AgentScheduler).
        // For offline use, we just validate the job exists.
        let store = match open_store().await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}", e);
                return;
            }
        };
        match store.get_job(job_id).await {
            Ok(Some(_)) => {
                println!(
                    "Job {} exists. Force-trigger requires a running runtime (symbi up).",
                    job_id
                );
                println!(
                    "Connect to the runtime API: POST /api/v1/schedules/{}/trigger with an Idempotency-Key UUID retained for retries",
                    job_id
                );
            }
            Ok(None) => eprintln!("Job {} not found", job_id),
            Err(e) => eprintln!("Failed to get job: {}", e),
        }
    }
    #[cfg(not(feature = "cron"))]
    {
        let _ = matches;
        eprintln!("Cron feature is not enabled. Rebuild with --features cron");
    }
}

async fn cmd_history(matches: &ArgMatches) {
    #[cfg(feature = "cron")]
    {
        let limit = matches
            .get_one::<String>("limit")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(20);

        let store = match open_store().await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}", e);
                return;
            }
        };

        if let Some(id_str) = matches.get_one::<String>("job") {
            // History for a specific job.
            let job_id = match parse_job_id(id_str) {
                Ok(id) => id,
                Err(e) => {
                    eprintln!("{}", e);
                    return;
                }
            };
            match store.get_run_history(job_id, limit).await {
                Ok(history) => print_history(&history),
                Err(e) => eprintln!("Failed to get history: {}", e),
            }
        } else {
            // History across all jobs — list each job's recent history.
            match store.list_jobs(None).await {
                Ok(jobs) => {
                    for job in &jobs {
                        match store.get_run_history(job.job_id, limit).await {
                            Ok(history) if !history.is_empty() => {
                                println!("=== {} ({}) ===", job.name, job.job_id);
                                print_history(&history);
                                println!();
                            }
                            _ => {}
                        }
                    }
                }
                Err(e) => eprintln!("Failed to list jobs: {}", e),
            }
        }
    }
    #[cfg(not(feature = "cron"))]
    {
        let _ = matches;
        eprintln!("Cron feature is not enabled. Rebuild with --features cron");
    }
}

#[cfg(feature = "cron")]
fn print_history(history: &[symbi_runtime::JobRunRecord]) {
    if history.is_empty() {
        println!("No run history.");
        return;
    }
    println!(
        "{:<20} {:<38} {:<12} {:<10} ERROR",
        "STARTED", "RUN ID", "STATUS", "DURATION"
    );
    for run in history {
        let duration = run
            .execution_time_ms
            .map(|ms| format!("{}ms", ms))
            .unwrap_or_else(|| "—".to_string());
        println!(
            "{:<20} {:<38} {:<12} {:<10} {}",
            run.started_at.format("%Y-%m-%d %H:%M:%S"),
            run.run_id,
            run.status.to_string(),
            duration,
            run.error.as_deref().unwrap_or(""),
        );
        if let Some(audit) = &run.admission_audit {
            println!("  Audit run: {}", audit.run_id);
            println!("  Journal: {}", audit.path.display());
            println!("  Public key: {}", audit.public_key);
        }
        if let Some(resolution) = &run.resolution {
            println!("  Operator resolution: {}", resolution["review"]["outcome"]);
            println!("  Rationale: {}", resolution["review"]["rationale"]);
            println!("  Evidence: {}", resolution["review"]["evidence"]);
        }
    }
}

#[cfg(feature = "cron")]
fn truncate(s: &str, max: usize) -> String {
    if s.len() > max {
        format!(
            "{}…",
            symbi_runtime::text_util::truncate_utf8(s, max.saturating_sub(1))
        )
    } else {
        s.to_string()
    }
}
