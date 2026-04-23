//! `symbi schemapin` — TOFU integrity pinning for MCP server configurations.
//!
//! This command pins the *server identity* (command + args + env block from
//! `.mcp.json`) so a SessionStart hook can detect tampering between runs.
//! It is the operational complement to the in-process `verify_schema` MCP tool,
//! which signs individual schema documents — both share the same trust model.
//!
//! Pin store layout: `~/.symbiont/schemapin/mcp/<server-name>.pin`
//!
//!   {
//!     "server_name": "...",
//!     "config_hash": "sha256:<hex>",
//!     "pinned_at":   "RFC-3339 timestamp"
//!   }
//!
//! Exit codes consumed by `symbi-claude-code`'s install-check.sh hook:
//!   0  verified (or freshly pinned)
//!   1  no signature pinned (stderr contains "no signature")
//!   2  tampered (stderr contains "verification failed")

use clap::ArgMatches;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct McpServersConfig {
    #[serde(rename = "mcpServers")]
    servers: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PinRecord {
    server_name: String,
    config_hash: String,
    pinned_at: String,
}

pub async fn run(matches: &ArgMatches) {
    match matches.subcommand() {
        Some(("verify", sub)) => cmd_verify(sub),
        Some(("pin", sub)) => cmd_pin(sub),
        Some(("list", _)) => cmd_list(),
        Some(("unpin", sub)) => cmd_unpin(sub),
        _ => {
            eprintln!("Usage: symbi schemapin <verify|pin|list|unpin> [--mcp-server NAME]");
            std::process::exit(2);
        }
    }
}

fn cmd_verify(matches: &ArgMatches) {
    let server_name = matches.get_one::<String>("mcp-server").cloned();
    let config_path = matches
        .get_one::<String>("config")
        .map(PathBuf::from)
        .unwrap_or_else(default_mcp_config_path);

    let servers = match load_mcp_servers(&config_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("verification failed: {}", e);
            std::process::exit(2);
        }
    };

    let names: Vec<String> = match server_name {
        Some(n) => vec![n],
        None => servers.servers.keys().cloned().collect(),
    };

    if names.is_empty() {
        eprintln!(
            "no signature: config '{}' lists no MCP servers",
            config_path.display()
        );
        std::process::exit(1);
    }

    let mut had_failure = false;
    for name in &names {
        let entry = match servers.servers.get(name) {
            Some(e) => e,
            None => {
                eprintln!(
                    "no signature: server '{}' not found in {}",
                    name,
                    config_path.display()
                );
                had_failure = true;
                continue;
            }
        };

        let live_hash = hash_server_entry(entry);
        let pin_path = pin_path_for(name);

        if !pin_path.exists() {
            eprintln!("no signature pinned for MCP server '{}'", name);
            had_failure = true;
            continue;
        }

        let record: PinRecord = match std::fs::read_to_string(&pin_path)
            .map_err(|e| e.to_string())
            .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
        {
            Ok(r) => r,
            Err(e) => {
                eprintln!("verification failed: cannot read pin for '{}': {}", name, e);
                had_failure = true;
                continue;
            }
        };

        if record.config_hash == live_hash {
            println!(
                "verified: {} ({})",
                name,
                &live_hash[..23.min(live_hash.len())]
            );
        } else {
            eprintln!(
                "verification failed: MCP server '{}' config changed since pinning ({} → {})",
                name,
                short(&record.config_hash),
                short(&live_hash),
            );
            had_failure = true;
        }
    }

    if had_failure {
        // Distinguish "no signature" vs "tampered" via stderr text already printed;
        // hooks branch on stderr content, so we just need a non-zero exit.
        std::process::exit(1);
    }
}

fn cmd_pin(matches: &ArgMatches) {
    let server_name = match matches.get_one::<String>("mcp-server") {
        Some(n) => n.clone(),
        None => {
            eprintln!("symbi schemapin pin requires --mcp-server <name>");
            std::process::exit(2);
        }
    };
    let config_path = matches
        .get_one::<String>("config")
        .map(PathBuf::from)
        .unwrap_or_else(default_mcp_config_path);
    let force = matches.get_flag("force");

    let servers = match load_mcp_servers(&config_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("pin failed: {}", e);
            std::process::exit(2);
        }
    };

    let entry = match servers.servers.get(&server_name) {
        Some(e) => e,
        None => {
            eprintln!(
                "pin failed: server '{}' not found in {}",
                server_name,
                config_path.display()
            );
            std::process::exit(2);
        }
    };

    let pin_path = pin_path_for(&server_name);
    if pin_path.exists() && !force {
        if let Ok(existing) = std::fs::read_to_string(&pin_path)
            .map_err(|e| e.to_string())
            .and_then(|s| serde_json::from_str::<PinRecord>(&s).map_err(|e| e.to_string()))
        {
            let live_hash = hash_server_entry(entry);
            if existing.config_hash == live_hash {
                println!(
                    "already pinned: {} ({})",
                    server_name,
                    short(&existing.config_hash)
                );
                return;
            }
            eprintln!(
                "pin already exists for '{}' with different hash ({} → {}). Use --force to overwrite.",
                server_name,
                short(&existing.config_hash),
                short(&live_hash),
            );
            std::process::exit(2);
        }
    }

    let live_hash = hash_server_entry(entry);
    let record = PinRecord {
        server_name: server_name.clone(),
        config_hash: live_hash.clone(),
        pinned_at: now_iso8601(),
    };

    if let Some(parent) = pin_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("pin failed: cannot create pin directory: {}", e);
            std::process::exit(2);
        }
    }
    let json = match serde_json::to_string_pretty(&record) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("pin failed: cannot serialize record: {}", e);
            std::process::exit(2);
        }
    };
    if let Err(e) = std::fs::write(&pin_path, json) {
        eprintln!("pin failed: cannot write pin file: {}", e);
        std::process::exit(2);
    }

    println!(
        "pinned: {} ({}) at {}",
        server_name,
        short(&live_hash),
        pin_path.display()
    );
}

fn cmd_list() {
    let dir = pin_root_dir();
    if !dir.exists() {
        println!(
            "No MCP servers pinned. Use 'symbi schemapin pin --mcp-server <name>' to add one."
        );
        return;
    }
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("cannot list pin directory {}: {}", dir.display(), e);
            std::process::exit(2);
        }
    };
    let mut found = 0;
    println!("{:<32} {:<28} PINNED AT", "SERVER", "FINGERPRINT");
    println!("{}", "-".repeat(90));
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("pin") {
            continue;
        }
        let record: PinRecord = match std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
        {
            Ok(r) => r,
            Err(_) => continue,
        };
        println!(
            "{:<32} {:<28} {}",
            record.server_name,
            short(&record.config_hash),
            record.pinned_at
        );
        found += 1;
    }
    if found == 0 {
        println!("(no pin records found in {})", dir.display());
    }
}

fn cmd_unpin(matches: &ArgMatches) {
    let server_name = match matches.get_one::<String>("mcp-server") {
        Some(n) => n.clone(),
        None => {
            eprintln!("symbi schemapin unpin requires --mcp-server <name>");
            std::process::exit(2);
        }
    };
    let pin_path = pin_path_for(&server_name);
    if !pin_path.exists() {
        eprintln!("no pin found for MCP server '{}'", server_name);
        std::process::exit(1);
    }
    if let Err(e) = std::fs::remove_file(&pin_path) {
        eprintln!("unpin failed: {}", e);
        std::process::exit(2);
    }
    println!("unpinned: {}", server_name);
}

// ---------------------------------------------------------------------------
// helpers

fn default_mcp_config_path() -> PathBuf {
    PathBuf::from(".mcp.json")
}

fn load_mcp_servers(path: &Path) -> Result<McpServersConfig, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read MCP config '{}': {}", path.display(), e))?;
    serde_json::from_str(&raw)
        .map_err(|e| format!("cannot parse MCP config '{}': {}", path.display(), e))
}

/// Hash the server entry deterministically. We sort keys via serde_json's
/// `Value` round-trip so the hash is stable regardless of the source-file
/// key ordering — TOFU integrity should not flip on whitespace changes.
fn hash_server_entry(entry: &serde_json::Value) -> String {
    let canonical = canonicalize(entry);
    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    let digest = Sha256::digest(&bytes);
    format!("sha256:{}", hex::encode(digest))
}

fn canonicalize(v: &serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut out = serde_json::Map::with_capacity(map.len());
            for k in keys {
                out.insert(k.clone(), canonicalize(&map[k]));
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(canonicalize).collect())
        }
        other => other.clone(),
    }
}

fn pin_root_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".symbiont")
        .join("schemapin")
        .join("mcp")
}

fn pin_path_for(server_name: &str) -> PathBuf {
    pin_root_dir().join(format!("{}.pin", sanitize(server_name)))
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn short(hash: &str) -> String {
    let trimmed = hash.strip_prefix("sha256:").unwrap_or(hash);
    let take = trimmed.len().min(12);
    format!("sha256:{}", &trimmed[..take])
}

/// Format the current time as an ISO-8601 / RFC-3339 UTC string without
/// depending on `chrono` (which is feature-gated here under `cron`).
fn now_iso8601() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // Days from civil (Howard Hinnant's algorithm) to render YYYY-MM-DD.
    let days = secs.div_euclid(86_400);
    let secs_of_day = secs.rem_euclid(86_400);
    let (y, mo, d) = civil_from_days(days);
    let h = secs_of_day / 3600;
    let m = (secs_of_day / 60) % 60;
    let s = secs_of_day % 60;
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, mo, d, h, m, s)
}

fn civil_from_days(z_in: i64) -> (i32, u32, u32) {
    let z = z_in + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}
