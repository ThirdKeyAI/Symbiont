//! Explicit local administration for optional governed workflow instructions.
use clap::{Arg, ArgMatches, Command};

pub fn command() -> Command {
    fn operation(name: &'static str, about: &'static str) -> Command {
        Command::new(name)
            .about(about)
            .arg(Arg::new("project").long("project").default_value("."))
            .arg(Arg::new("workflow").long("workflow").required(true))
    }
    fn arg(name: &'static str) -> Arg {
        Arg::new(name).long(name).required(true)
    }
    Command::new("improvement")
        .about("Opt in, evaluate and release governed workflow instructions")
        .subcommand_required(true)
        .subcommand(
            operation(
                "init",
                "Opt in one workflow with a frozen operator-owned acceptance suite",
            )
            .arg(arg("agent"))
            .arg(arg("suite")),
        )
        .subcommand(
            operation(
                "propose",
                "Store an instruction candidate without activating it",
            )
            .arg(arg("file")),
        )
        .subcommand(
            operation("inspect", "Inspect workflow state and its verification key")
                .arg(Arg::new("candidate").long("candidate"))
                .arg(Arg::new("evaluation").long("evaluation")),
        )
        .subcommand(
            operation(
                "evaluate",
                "Evaluate completed signed trials offline; execute no agent actions",
            )
            .arg(arg("candidate"))
            .arg(arg("trials")),
        )
        .subcommand(
            operation(
                "approve",
                "Approve one exact passing evaluation as the local operator",
            )
            .arg(arg("candidate"))
            .arg(arg("evaluation"))
            .arg(arg("rationale")),
        )
        .subcommand(
            operation(
                "promote",
                "Activate an approved candidate for future explicit selections",
            )
            .arg(arg("candidate"))
            .arg(arg("approval"))
            .arg(arg("expected-active").help("Current digest, or none for first activation")),
        )
        .subcommand(
            operation(
                "rollback",
                "Restore a previously activated approved version",
            )
            .arg(arg("candidate"))
            .arg(arg("approval"))
            .arg(arg("expected-active")),
        )
        .subcommand(operation(
            "disable",
            "Refuse new improvement selections; leave admitted runs unchanged",
        ))
        .subcommand(operation(
            "enable",
            "Explicitly re-enable an existing workflow",
        ))
        .subcommand(
            operation(
                "export",
                "Export one signed document for independent inspection",
            )
            .arg(arg("kind").value_parser(["state", "candidate", "evaluation", "approval"]))
            .arg(Arg::new("id").long("id")),
        )
        .subcommand(
            Command::new("verify")
                .about("Verify an exported document against an independently trusted public key")
                .arg(arg("file"))
                .arg(arg("public-key"))
                .arg(arg("kind").value_parser(["state", "candidate", "evaluation", "approval"])),
        )
}

pub fn run(matches: &ArgMatches) {
    match execute(matches) {
        Ok((value, code)) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&value).expect("JSON value")
            );
            std::process::exit(code);
        }
        Err(error) => {
            eprintln!("Improvement operation refused: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(unix)]
fn execute(matches: &ArgMatches) -> Result<(serde_json::Value, i32), String> {
    use std::path::Path;
    use symbi_runtime::improvement::{evaluation::ExactAnswerEvaluator, read_json, Store};
    let (operation, args) = matches.subcommand().ok_or("missing operation")?;
    let value = |name: &str| {
        args.get_one::<String>(name)
            .map(String::as_str)
            .ok_or_else(|| format!("missing {name}"))
    };
    if operation == "verify" {
        let document: serde_json::Value = read_json(Path::new(value("file")?))?;
        let key = hex::decode(value("public-key")?)
            .map_err(|e| e.to_string())?
            .try_into()
            .map_err(|_| "expected 32-byte public key")?;
        let payload = symbi_runtime::improvement::verify_document(
            &serde_json::to_vec(&document).map_err(|e| e.to_string())?,
            value("kind")?,
            &key,
        )?;
        return Ok((serde_json::json!({"verified":true,"payload":payload}), 0));
    }
    let project = Path::new(value("project")?)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let workflow = value("workflow")?;
    if operation == "init" {
        let source =
            std::fs::read_to_string(project.join(value("agent")?)).map_err(|e| e.to_string())?;
        let fallback = Path::new(value("agent")?)
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or("invalid agent path")?;
        let settings =
            dsl::resolve_execution_settings(&source, fallback).map_err(|e| e.to_string())?;
        let suite = read_json(Path::new(value("suite")?))?;
        let store = Store::initialize(
            &project,
            workflow,
            &settings.agent_name,
            &settings.agent_source,
            suite,
        )?;
        return Ok((
            serde_json::json!({"workflow":store.state(),"public_key":store.public_key()}),
            0,
        ));
    }
    let mut store = Store::open(&project, workflow)?;
    let result = match operation {
        "propose" => {
            serde_json::json!({"candidate":store.propose(read_json(Path::new(value("file")?))?)?})
        }
        "inspect" => {
            let mut result =
                serde_json::json!({"workflow":store.state(),"public_key":store.public_key()});
            if let Some(id) = args.get_one::<String>("candidate") {
                result["candidate"] =
                    serde_json::to_value(store.candidate(id)?).map_err(|e| e.to_string())?;
            }
            if let Some(id) = args.get_one::<String>("evaluation") {
                result["evaluation"] =
                    serde_json::to_value(store.evaluation(id)?).map_err(|e| e.to_string())?;
            }
            result
        }
        "evaluate" => {
            let references = read_json::<Vec<_>>(Path::new(value("trials")?))?;
            let (id, report) =
                store.evaluate(value("candidate")?, &references, &ExactAnswerEvaluator)?;
            let code = if report.accepted { 0 } else { 2 };
            return Ok((serde_json::json!({"evaluation":id,"report":report}), code));
        }
        "approve" => {
            serde_json::json!({"approval":store.approve(value("candidate")?,value("evaluation")?,value("rationale")?)?})
        }
        "promote" | "rollback" => {
            let expected = value("expected-active")?;
            store.activate(
                value("candidate")?,
                value("approval")?,
                if expected == "none" {
                    None
                } else {
                    Some(expected)
                },
                operation == "rollback",
            )?;
            serde_json::json!({"active":store.state().active,"workflow":workflow})
        }
        "enable" | "disable" => {
            store.set_enabled(operation == "enable")?;
            serde_json::json!({"enabled":store.state().enabled,"workflow":workflow})
        }
        "export" => store.export_document(
            value("kind")?,
            args.get_one::<String>("id").map(String::as_str),
        )?,
        _ => return Err("unsupported improvement operation".into()),
    };
    Ok((result, 0))
}

#[cfg(not(unix))]
fn execute(_: &ArgMatches) -> Result<(serde_json::Value, i32), String> {
    Err("protected improvement workflows require a Unix host".into())
}
