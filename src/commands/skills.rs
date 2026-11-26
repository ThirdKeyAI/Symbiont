use clap::ArgMatches;
use std::io::Write;
use std::path::{Path, PathBuf};
use symbi_runtime::skills::{ScanRule, SkillLoader, SkillScanner, SkillsConfig};

pub async fn run(matches: &ArgMatches) {
    match matches.subcommand() {
        Some(("list", _)) => cmd_list().await,
        Some(("scan", sub)) => cmd_scan(sub).await,
        Some(("rules", sub)) => {
            if let Err(error) = cmd_rules(sub).await {
                eprintln!("Rule feed error: {error:#}");
                std::process::exit(1);
            }
        }
        Some(("verify", sub)) => cmd_verify(sub).await,
        Some(("sign", sub)) => cmd_sign(sub).await,
        _ => {
            println!("Usage: symbi skills <list|scan|rules|verify|sign>");
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

    let json = matches.get_flag("json");
    let mut rules = Vec::new();
    if let Some(patterns) = matches.get_many::<String>("deny-pattern") {
        rules.extend(patterns.cloned().map(ScanRule::DenyContentPattern));
    }
    if let Some(patterns) = matches.get_many::<String>("warn-pattern") {
        rules.extend(patterns.cloned().map(ScanRule::WarnContentPattern));
    }
    // Compatibility constructor retains invalid policy identity in failed JSON results.
    let scanner = match matches.get_one::<String>("rules-dir") {
        Some(dir) => SkillScanner::with_rule_store(rules, Default::default(), Path::new(dir)),
        None => SkillScanner::with_custom_rules(rules),
    };
    let start = std::time::Instant::now();

    let result = scanner.scan_skill_with_progress(&dir, |progress| {
        if json {
            return;
        }
        // Overwrite the current line with progress status
        eprint!(
            "\r\x1b[K  Scanning [{}/{}] {}",
            progress.scanned, progress.total, progress.file,
        );
        let _ = std::io::stderr().flush();
    });

    if json {
        println!(
            "{}",
            serde_json::to_string(&result).expect("serializable scan result")
        );
        if !result.passed {
            std::process::exit(1);
        }
        return;
    }

    // Clear the progress line
    eprint!("\r\x1b[K");
    let _ = std::io::stderr().flush();

    let elapsed = start.elapsed();
    println!(
        "Scanner {} / semantics {} / ruleset {} / policy SHA-256 {}",
        result.identity.engine_version,
        result.identity.semantics_version,
        result.identity.ruleset_version,
        result.identity.policy_sha256
    );

    if result.findings.is_empty() {
        println!(
            "No findings under the selected scan policy. ({:.1}s)",
            elapsed.as_secs_f64()
        );
        return;
    }

    println!("Scan results for: {}\n", dir.display());

    for finding in &result.findings {
        let icon = match finding.severity {
            symbi_runtime::skills::ScanSeverity::Critical => "!!",
            symbi_runtime::skills::ScanSeverity::High => "!+",
            symbi_runtime::skills::ScanSeverity::Medium => "!~",
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
        "\n{} finding(s). {} ({:.1}s)",
        result.findings.len(),
        if result.passed { "PASSED" } else { "FAILED" },
        elapsed.as_secs_f64(),
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
    match symbi_runtime::skills::load_skill_signature(&dir) {
        Ok(sig) => {
            println!("Signature found:");
            println!("  Skill:   {}", sig.skill_name);
            println!("  Domain:  {}", sig.domain);
            println!(
                "  Claimed signing time (pending verification): {}",
                sig.signed_at
            );
            println!("  Signer:  {}", sig.signer_kid);
            println!("  Hash:    {}", sig.skill_hash);
            println!("  Files:   {} in manifest", sig.file_manifest.len());

            if sig.schemapin_version != symbi_runtime::skills::signature::FORMAT {
                eprintln!("Legacy or unsupported skill signature; re-sign with symbi skills sign");
                std::process::exit(1);
            }

            let expected = symbi_runtime::skills::loader::verification_domain(&domain);
            let claimed = symbi_runtime::skills::loader::verification_domain(&sig.domain);
            if !matches!((expected, claimed), (Ok(expected), Ok(claimed)) if expected == claimed) {
                eprintln!("Signature domain does not match requested authority");
                std::process::exit(1);
            }

            // Attempt to fetch discovery document for full verification
            println!("\nAttempting online verification against {}...", domain);
            match fetch_and_verify(&dir, &domain).await {
                Ok(status) => {
                    println!("Result: {}", status);
                    if !matches!(
                        status,
                        symbi_runtime::skills::SignatureStatus::Verified { .. }
                            | symbi_runtime::skills::SignatureStatus::Pinned { .. }
                    ) {
                        std::process::exit(1);
                    }
                }
                Err(e) => {
                    println!("Online verification failed: {}", e);
                    println!(
                        "Tip: ensure {0}/.well-known/schemapin.json is reachable",
                        domain
                    );
                    std::process::exit(1);
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
    let domain =
        symbi_runtime::skills::loader::verification_domain(domain).map_err(|e| e.to_string())?;
    let url = format!("https://{}/.well-known/schemapin.json", domain);
    symbi_runtime::net_guard::reject_ssrf_url(&url)?;
    let client =
        symbi_runtime::net_guard::build_ssrf_safe_client(std::time::Duration::from_secs(15))
            .map_err(|e| e.to_string())?;
    let mut response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if body.len().saturating_add(chunk.len()) > 1024 * 1024 {
            return Err("discovery document exceeds 1 MiB".into());
        }
        body.extend_from_slice(&chunk);
    }
    let discovery: schemapin::types::discovery::WellKnownResponse =
        serde_json::from_slice(&body).map_err(|e| e.to_string())?;

    let config = SkillsConfig {
        require_signed: true,
        auto_pin: true,
        ..SkillsConfig::default()
    };
    let mut loader = SkillLoader::new(config);
    Ok(loader.verify_skill_with_discovery(dir, &discovery, &domain))
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

    match symbi_runtime::skills::signature::sign_skill(&dir, &private_key_pem, &domain) {
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
        format!(
            "{}...",
            symbi_runtime::text_util::truncate_utf8(s, max_len.saturating_sub(3))
        )
    } else {
        s.to_string()
    }
}

fn rule_store(matches: &ArgMatches) -> anyhow::Result<PathBuf> {
    matches
        .get_one::<String>("store")
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join(".symbiont/clawhavoc"))
                .ok_or_else(|| anyhow::anyhow!("HOME is unavailable; supply --store"))
        })
}

async fn cmd_rules(matches: &ArgMatches) -> anyhow::Result<()> {
    use symbi_runtime::skills::rule_pack::{self, MAX_PACK_BYTES};
    let required = |sub: &ArgMatches, name: &str| -> anyhow::Result<String> {
        sub.get_one::<String>(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing {name}"))
    };
    match matches.subcommand() {
        Some(("init", sub)) => {
            let dir = rule_store(sub)?;
            let key = rule_pack::read_rule_file(Path::new(&required(sub, "public-key")?), 8192)?;
            rule_pack::initialize_rule_feed(
                &dir,
                &required(sub, "url")?,
                std::str::from_utf8(&key)?,
            )?;
            println!(
                "Pinned rule feed in {}. Run skills rules update to fetch it.",
                dir.display()
            );
        }
        Some(("update", sub)) => {
            let pack = rule_pack::update_rule_feed(&rule_store(sub)?).await?;
            println!(
                "Verified rule pack {} / SHA-256 {}",
                pack.version(),
                pack.digest()
            );
        }
        Some(("status", sub)) => {
            let pack = rule_pack::load_rule_pack(&rule_store(sub)?)?;
            println!(
                "Verified rule pack {} / expires {} / SHA-256 {}",
                pack.version(),
                pack.expires_at(),
                pack.digest()
            );
        }
        Some(("sign", sub)) => {
            let input =
                rule_pack::read_rule_file(Path::new(&required(sub, "input")?), MAX_PACK_BYTES)?;
            let key = rule_pack::read_rule_file(Path::new(&required(sub, "key")?), 8192)?;
            let signed = rule_pack::sign_rule_pack(
                serde_json::from_slice(&input)?,
                std::str::from_utf8(&key)?,
            )?;
            let output = required(sub, "output")?;
            // Never overwrite a publisher's existing artifact or follow an output link.
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&output)?;
            file.write_all(&signed)?;
            file.sync_all()?;
            println!("Signed rule pack written to {output}");
        }
        _ => anyhow::bail!("use skills rules <init|update|status|sign>"),
    }
    Ok(())
}
