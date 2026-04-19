use super::CommandResult;
use crate::app::App;
use crate::secrets_store;

pub fn secrets(_app: &mut App, args: &str) -> CommandResult {
    if args.is_empty() {
        return CommandResult::Output(
            "Usage:\n\
             /secrets list                   List all secret keys\n\
             /secrets set <key> <value>      Store a secret (encrypted locally)\n\
             /secrets get <key>              Retrieve a secret value\n\
             /secrets delete <key>           Remove a secret\n\n\
             Secrets are stored encrypted in .symbi/secrets.enc\n\
             They are injected as env vars during /deploy local\n\
             The master key comes from SYMBIONT_MASTER_KEY or a generated .symbi/master.key"
                .to_string(),
        );
    }

    let parts: Vec<&str> = args.splitn(3, ' ').collect();
    match parts[0] {
        "list" => match secrets_store::list_secrets() {
            Ok(keys) if keys.is_empty() => CommandResult::Output("No secrets stored.".to_string()),
            Ok(keys) => {
                let mut out = format!("Secrets ({}):\n\n", keys.len());
                for k in &keys {
                    out.push_str(&format!("  {}\n", k));
                }
                CommandResult::Output(out)
            }
            Err(e) => CommandResult::Error(format!("Failed to list: {}", e)),
        },
        "set" => {
            if parts.len() < 3 {
                return CommandResult::Error("Usage: /secrets set <key> <value>".to_string());
            }
            let key = parts[1];
            let value = parts[2];
            match secrets_store::set_secret(key, value) {
                Ok(()) => CommandResult::Output(format!("Stored secret '{}'", key)),
                Err(e) => CommandResult::Error(format!("Failed to set: {}", e)),
            }
        }
        "get" => {
            if parts.len() < 2 {
                return CommandResult::Error("Usage: /secrets get <key>".to_string());
            }
            let key = parts[1];
            match secrets_store::get_secret(key) {
                Ok(Some(value)) => CommandResult::Output(format!("{}: {}", key, value)),
                Ok(None) => CommandResult::Output(format!("Secret '{}' not found", key)),
                Err(e) => CommandResult::Error(format!("Failed to get: {}", e)),
            }
        }
        "delete" => {
            if parts.len() < 2 {
                return CommandResult::Error("Usage: /secrets delete <key>".to_string());
            }
            let key = parts[1];
            match secrets_store::delete_secret(key) {
                Ok(true) => CommandResult::Output(format!("Deleted secret '{}'", key)),
                Ok(false) => CommandResult::Output(format!("Secret '{}' did not exist", key)),
                Err(e) => CommandResult::Error(format!("Failed to delete: {}", e)),
            }
        }
        _ => CommandResult::Error(format!(
            "Unknown secrets subcommand: {}\n\
             Available: list, set, get, delete",
            parts[0]
        )),
    }
}
