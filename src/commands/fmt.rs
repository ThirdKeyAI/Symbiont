//! `symbi fmt` subcommand — canonical-format `.symbi` files.

use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;

use clap::ArgMatches;

pub fn run(matches: &ArgMatches) -> i32 {
    let check = matches.get_flag("check");
    let stdin_mode = matches.get_flag("stdin");
    let files: Vec<&String> = matches
        .get_many::<String>("files")
        .map(|v| v.collect())
        .unwrap_or_default();

    if stdin_mode {
        if !files.is_empty() {
            eprintln!("symbi fmt: --stdin cannot be combined with file arguments");
            return 2;
        }
        return run_stdin(check);
    }

    if files.is_empty() {
        eprintln!("symbi fmt: no input files (pass --stdin to read from stdin)");
        return 2;
    }

    let mut had_change = false;
    let mut had_error = false;
    for file in files {
        match format_file(Path::new(file), check) {
            Ok(true) => had_change = true,
            Ok(false) => {}
            Err(e) => {
                eprintln!("symbi fmt: {}: {}", file, e);
                had_error = true;
            }
        }
    }

    if had_error {
        1
    } else if check && had_change {
        2
    } else {
        0
    }
}

fn run_stdin(check: bool) -> i32 {
    let mut input = String::new();
    if let Err(e) = io::stdin().read_to_string(&mut input) {
        eprintln!("symbi fmt: stdin: {}", e);
        return 1;
    }
    match dsl::format::format_source(&input) {
        Ok(out) => {
            if check {
                if out != input {
                    return 2;
                }
                0
            } else {
                if let Err(e) = io::stdout().write_all(out.as_bytes()) {
                    eprintln!("symbi fmt: stdout: {}", e);
                    return 1;
                }
                0
            }
        }
        Err(e) => {
            eprintln!("symbi fmt: stdin: {}", e);
            1
        }
    }
}

fn format_file(path: &Path, check: bool) -> Result<bool, String> {
    let input = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let out = dsl::format::format_source(&input).map_err(|e| e.to_string())?;
    if out == input {
        return Ok(false);
    }
    if check {
        eprintln!("would format {}", path.display());
        return Ok(true);
    }
    fs::write(path, out).map_err(|e| e.to_string())?;
    println!("formatted {}", path.display());
    Ok(true)
}
