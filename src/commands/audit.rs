use clap::ArgMatches;

pub fn run(matches: &ArgMatches) {
    let Some(("inspect", arguments)) = matches.subcommand() else {
        unreachable!("clap requires the audit inspect subcommand")
    };
    match inspect(arguments) {
        Ok((json, requires_reconciliation)) => {
            println!("{json}");
            if requires_reconciliation {
                std::process::exit(2);
            }
        }
        Err(error) => {
            eprintln!("Audit inspection failed: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(unix)]
fn inspect(arguments: &ArgMatches) -> Result<(String, bool), String> {
    let path = std::path::Path::new(arguments.get_one::<String>("journal").unwrap());
    let key = arguments.get_one::<String>("public-key").unwrap();
    let key: [u8; 32] = hex::decode(key)
        .map_err(|_| "public key must be 64 hexadecimal characters")?
        .try_into()
        .map_err(|_| "public key must be 32 bytes")?;
    let run_id = arguments
        .get_one::<String>("run-id")
        .unwrap()
        .parse()
        .map_err(|_| "run ID must be a UUID")?;
    let report = symbi_runtime::reasoning::recovery::inspect_run(path, &key, run_id)
        .map_err(|error| error.to_string())?;
    let json = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
    Ok((json, report.requires_reconciliation))
}

#[cfg(not(unix))]
fn inspect(_: &ArgMatches) -> Result<(String, bool), String> {
    Err("protected journal inspection requires a Unix host".into())
}
