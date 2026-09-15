use clap::{Arg, ArgMatches, Command};

pub fn command() -> Command {
    fn target(name: &'static str, about: &'static str) -> Command {
        Command::new(name)
            .about(about)
            .arg(
                Arg::new("project")
                    .long("project")
                    .default_value(".")
                    .value_name("DIRECTORY"),
            )
            .arg(
                Arg::new("scope")
                    .long("scope")
                    .required(true)
                    .value_name("SCOPE"),
            )
            .arg(Arg::new("id").long("id").required(true).value_name("UUID"))
    }
    Command::new("invocation").about("Inspect and reconcile retained invocation outcomes")
        .subcommand_required(true)
        .subcommand(target("inspect", "Inspect an inactive invocation and obtain the exact evidence snapshot hash"))
        .subcommand(target("file-inspect", "Inspect one signed file publication and its current candidate or output")
            .arg(Arg::new("publication").long("publication").required(true).value_name("UUID")))
        .subcommand(target("file-recover", "Finish only the exact recorded file publication; never rerun the tool")
            .arg(Arg::new("publication").long("publication").required(true).value_name("UUID"))
            .arg(Arg::new("snapshot-hash").long("snapshot-hash").required(true).value_name("SHA256")))
        .subcommand(target("reconcile", "Record a signed operator assessment without granting replay")
            .arg(Arg::new("review").long("review").required(true).value_name("JSON_FILE"))
            .arg(Arg::new("cron-store").long("cron-store").value_name("DATABASE")
                .help("Reconcile matching cron history in this project-bound store; defaults to the existing project store")))
}

pub async fn run(matches: &ArgMatches) {
    match execute(matches).await {
        Ok((output, code)) => {
            println!("{output}");
            std::process::exit(code);
        }
        Err(error) => {
            eprintln!("Invocation inspection/reconciliation failed: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(unix)]
async fn execute(matches: &ArgMatches) -> Result<(String, i32), String> {
    use std::path::Path;
    use symbi_runtime::reasoning::invocation::reconciliation::{
        inspect_invocation, read_review, reconcile_invocation,
    };
    let (name, arguments) = matches.subcommand().unwrap();
    let project = Path::new(arguments.get_one::<String>("project").unwrap())
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let scope = arguments.get_one::<String>("scope").unwrap();
    let id = arguments
        .get_one::<String>("id")
        .unwrap()
        .parse()
        .map_err(|_| "invocation ID must be a UUID")?;
    if name == "inspect" {
        let report = inspect_invocation(&project, scope, id)?;
        let code = if report.status == "unresolved" { 2 } else { 0 };
        return Ok((
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?,
            code,
        ));
    }
    if matches!(name, "file-inspect" | "file-recover") {
        use symbi_runtime::reasoning::invocation::file_recovery::{
            inspect_file_publication, recover_file_publication,
        };
        let publication = arguments
            .get_one::<String>("publication")
            .unwrap()
            .parse()
            .map_err(|_| "publication ID must be a UUID")?;
        if name == "file-inspect" {
            let report = inspect_file_publication(&project, scope, id, publication)?;
            let code = if report.state == symbi_runtime::sandbox::files::PublicationState::Published
            {
                0
            } else {
                2
            };
            return Ok((
                serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?,
                code,
            ));
        }
        let receipt = recover_file_publication(
            &project,
            scope,
            id,
            publication,
            arguments.get_one::<String>("snapshot-hash").unwrap(),
        )?;
        let output = serde_json::json!({"status":"file_publication_recovered", "recovery":receipt,
            "tool_repeated":false, "invocation_completed":false});
        return Ok((
            serde_json::to_string_pretty(&output).map_err(|e| e.to_string())?,
            0,
        ));
    }
    if let Some(database) = arguments.get_one::<String>("cron-store") {
        if scope != "scheduler:v1" {
            return Err("--cron-store requires scheduler:v1 scope".into());
        }
        if !Path::new(database)
            .try_exists()
            .map_err(|e| e.to_string())?
        {
            return Err("explicit cron store does not exist".into());
        }
        #[cfg(not(feature = "cron"))]
        return Err("--cron-store requires the cron feature".into());
    }
    let review = read_review(Path::new(arguments.get_one::<String>("review").unwrap()))?;
    let receipt = reconcile_invocation(&project, scope, id, review)?;
    let mut output =
        serde_json::json!({"status":"reconciled","resolution":receipt,"work_repeated":false});
    #[cfg(feature = "cron")]
    if scope == "scheduler:v1" {
        output["cron_history_updated"] = serde_json::json!(false);
        let database = arguments
            .get_one::<String>("cron-store")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| project.join(".symbiont/cron_jobs.db"));
        if database.try_exists().map_err(|e| e.to_string())? {
            let store = symbi_runtime::SqliteJobStore::open(&database)
                .map_err(|e| format!("Resolution saved; failed to open cron history: {e}"))?;
            store
                .bind_project(&project)
                .await
                .map_err(|e| format!("Resolution saved; cron store ownership check failed: {e}"))?;
            output["cron_history_updated"] = serde_json::json!(store
                .reconcile_occurrence(&project, id)
                .await
                .map_err(|e| format!(
                    "Resolution saved; cron history update failed, repeat this same review: {e}"
                ))?);
        }
    }
    Ok((
        serde_json::to_string_pretty(&output).map_err(|e| e.to_string())?,
        0,
    ))
}

#[cfg(not(unix))]
async fn execute(_: &ArgMatches) -> Result<(String, i32), String> {
    Err("protected invocation reconciliation requires a Unix host".into())
}
