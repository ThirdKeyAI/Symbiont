use clap::ArgMatches;
use std::path::{Path, PathBuf};
use symbi_runtime::skills::{SkillLoader, SkillScanner, SkillsConfig};

pub async fn run(matches: &ArgMatches) {
    match matches.subcommand() {
        Some(("list", _)) => cmd_list().await,
        Some(("scan", sub)) => cmd_scan(sub).await,
        Some(("verify", sub)) => cmd_verify(sub).await,
        Some(("sign", sub)) => cmd_sign(sub).await,
        _ => {
            println!("Usage: symbi skills <list|scan|verify|sign>");
            println!("Run 'symbi skills --help' for details.");
        }
    }
}

async fn cmd_list() {
    let config = SkillsConfig::default();
    let paths = config.load_paths.clone();
    let mut loader = SkillLoader::new(config);
    let skills = loader.load_all();

    if skills.is_empty() {
        println!("No skills found.");
        println!("\nSearch paths:");
        for p in &paths {
            println!("  {}", p.display());
        }
        return;
    }

    println!("{:<24} {:<40} {:<24} DOMAIN", "NAME", "PATH", "STATUS");
    println!("{}", "-".repeat(100));

    for skill in &skills {
        let (status, domain) = match &skill.signature_status {
            symbi_runtime::skills::SignatureStatus::Verified { domain, .. } => {
                ("Verified".to_string(), domain.clone())
            }
            symbi_runtime::skills::SignatureStatus::Pinned { domain, .. } => {
                ("Pinned".to_string(), domain.clone())
            }
            symbi_runtime::skills::SignatureStatus::Unsigned => {
                ("Unsigned".to_string(), "-".into())
            }
            symbi_runtime::skills::SignatureStatus::Invalid { reason } => {
                (format!("Invalid: {}", truncate(reason, 30)), "-".into())
            }
            symbi_runtime::skills::SignatureStatus::Revoked { reason } => {
                (format!("Revoked: {}", truncate(reason, 30)), "-".into())
            }
        };

        println!(
            "{:<24} {:<40} {:<24} {}",
            truncate(&skill.name, 22),
            truncate(&skill.path.display().to_string(), 38),
            status,
            domain,
        );
    }

    println!("\n{} skill(s) found.", skills.len());
}

async fn cmd_scan(matches: &ArgMatches) {
    let dir = match matches.get_one::<String>("dir") {
        Some(d) => PathBuf::from(d),
        None => {
            eprintln!("Error: <dir> argument is required");
            std::process::exit(1);
        }
    };

    if !dir.exists() {
        eprintln!("Error: directory '{}' does not exist", dir.display());
        std::process::exit(1);
    }

    let scanner = SkillScanner::new();
    let result = scanner.scan_skill(&dir);

    if result.findings.is_empty() {
        println!("No findings. Skill passed all security checks.");
        return;
    }

    println!("Scan results for: {}\n", dir.display());

    for finding in &result.findings {
        let icon = match finding.severity {
            symbi_runtime::skills::ScanSeverity::Critical => "!!",
            symbi_runtime::skills::ScanSeverity::Warning => "!",
            symbi_runtime::skills::ScanSeverity::Info => "i",
        };

        let location = match finding.line {
            Some(line) => format!("{}:{}", finding.file, line),
            None => finding.file.clone(),
        };

        println!(
            "[{}] {} — {} ({})",
            icon, finding.severity, finding.message, location
        );
    }

    println!(
        "\n{} finding(s). {}",
        result.findings.len(),
        if result.passed { "PASSED" } else { "FAILED" }
    );

    if !result.passed {
        std::process::exit(1);
    }
}

async fn cmd_verify(matches: &ArgMatches) {
    let dir = match matches.get_one::<String>("dir") {
        Some(d) => PathBuf::from(d),
        None => {
            eprintln!("Error: <dir> argument is required");
            std::process::exit(1);
        }
    };

    let domain = match matches.get_one::<String>("domain") {
        Some(d) => d.clone(),
        None => {
            eprintln!("Error: --domain is required for verification");
            std::process::exit(1);
        }
    };

    if !dir.exists() {
        eprintln!("Error: directory '{}' does not exist", dir.display());
        std::process::exit(1);
    }

    // Try to load the signature first
    match schemapin::skill::load_signature(&dir) {
        Ok(sig) => {
            println!("Signature found:");
            println!("  Skill:   {}", sig.skill_name);
            println!("  Domain:  {}", sig.domain);
            println!("  Signed:  {}", sig.signed_at);
            println!("  Signer:  {}", sig.signer_kid);
            println!("  Hash:    {}", sig.skill_hash);
            println!("  Files:   {} in manifest", sig.file_manifest.len());

            if sig.domain != domain {
                eprintln!(
                    "\nWarning: signature domain '{}' does not match requested domain '{}'",
                    sig.domain, domain
                );
            }

            // Attempt to fetch discovery document for full verification
            println!("\nAttempting online verification against {}...", domain);
            match fetch_and_verify(&dir, &domain).await {
                Ok(status) => println!("Result: {}", status),
                Err(e) => {
                    println!("Online verification failed: {}", e);
                    println!(
                        "Tip: ensure {0}/.well-known/schemapin.json is reachable",
                        domain
                    );
                }
            }
        }
        Err(e) => {
            eprintln!("No valid signature found: {}", e);
            std::process::exit(1);
        }
    }
}

async fn fetch_and_verify(
    dir: &Path,
    domain: &str,
) -> Result<symbi_runtime::skills::SignatureStatus, String> {
    // Try to fetch the discovery document
    let url = format!("https://{}/.well-known/schemapin.json", domain);
    let response = reqwest::get(&url).await.map_err(|e| e.to_string())?;
    let discovery: schemapin::types::discovery::WellKnownResponse =
        response.json().await.map_err(|e| e.to_string())?;

    let config = SkillsConfig {
        require_signed: true,
        auto_pin: true,
        ..SkillsConfig::default()
    };
    let mut loader = SkillLoader::new(config);
    Ok(loader.verify_skill_with_discovery(dir, &discovery))
}

async fn cmd_sign(matches: &ArgMatches) {
    let dir = match matches.get_one::<String>("dir") {
        Some(d) => PathBuf::from(d),
        None => {
            eprintln!("Error: <dir> argument is required");
            std::process::exit(1);
        }
    };

    let key_path = match matches.get_one::<String>("key") {
        Some(k) => PathBuf::from(k),
        None => {
            eprintln!("Error: --key is required for signing");
            std::process::exit(1);
        }
    };

    let domain = match matches.get_one::<String>("domain") {
        Some(d) => d.clone(),
        None => {
            eprintln!("Error: --domain is required for signing");
            std::process::exit(1);
        }
    };

    if !dir.exists() {
        eprintln!("Error: directory '{}' does not exist", dir.display());
        std::process::exit(1);
    }

    let private_key_pem = match std::fs::read_to_string(&key_path) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("Error reading key file '{}': {}", key_path.display(), e);
            std::process::exit(1);
        }
    };

    match schemapin::skill::sign_skill(&dir, &private_key_pem, &domain, None, None) {
        Ok(sig) => {
            println!("Skill signed successfully.");
            println!("  Skill:      {}", sig.skill_name);
            println!("  Domain:     {}", sig.domain);
            println!("  Signed at:  {}", sig.signed_at);
            println!("  Signer KID: {}", sig.signer_kid);
            println!("  Hash:       {}", sig.skill_hash);
            println!("  Files:      {} in manifest", sig.file_manifest.len());
            println!(
                "\nSignature written to: {}",
                dir.join(".schemapin.sig").display()
            );
        }
        Err(e) => {
            eprintln!("Signing failed: {}", e);
            std::process::exit(1);
        }
    }
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() > max_len {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    } else {
        s.to_string()
    }
}
