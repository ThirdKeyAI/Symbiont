use super::CommandResult;
use crate::app::App;
use crate::deploy;

pub fn deploy(app: &mut App, args: &str) -> CommandResult {
    let parts: Vec<&str> = args.split_whitespace().collect();

    if parts.is_empty() {
        return CommandResult::Output(
            "Usage:\n\
             /deploy <@agent> local [--port 8080]                    Deploy locally via Docker\n\
             /deploy <@agent> cloudrun --project <p> [--region <r>]  Deploy to Google Cloud Run\n\
                 [--service-account <sa>] [--allow-unauthenticated]\n\
             /deploy <@agent> aws [--region <r>]                     Deploy to AWS App Runner\n\
                 [--access-role-arn <arn>] [--instance-role-arn <arn>]\n\
             /deploy list                                            List local deployments\n\
             /deploy logs <agent> [--tail 50]                        Show container logs\n\
             /deploy stop <agent>                                    Stop local deployment\n\n\
             Secrets from /secrets are injected into containers:\n\
               - local: env vars on the container\n\
               - cloudrun: GCP Secret Manager mapped via --set-secrets\n\
               - aws: AWS Secrets Manager mapped via App Runner RuntimeEnvironmentSecrets"
                .to_string(),
        );
    }

    match parts[0] {
        "list" => deploy_list(),
        "logs" => {
            let agent = parts.get(1).unwrap_or(&"");
            let tail = parts
                .iter()
                .position(|&p| p == "--tail")
                .and_then(|i| parts.get(i + 1))
                .and_then(|t| t.parse().ok())
                .unwrap_or(50);
            deploy_logs(agent, tail)
        }
        "stop" => {
            let agent = parts.get(1).unwrap_or(&"");
            if agent.is_empty() {
                return CommandResult::Error("Usage: /deploy stop <agent>".to_string());
            }
            deploy_stop(agent)
        }
        agent_ref => {
            // /deploy @agent local [--port 8080]
            let agent_name = agent_ref.strip_prefix('@').unwrap_or(agent_ref);
            let target = parts.get(1).unwrap_or(&"local");

            match *target {
                "local" => {
                    let port = parts
                        .iter()
                        .position(|&p| p == "--port")
                        .and_then(|i| parts.get(i + 1))
                        .and_then(|p| p.parse().ok())
                        .unwrap_or(8080u16);
                    deploy_local(app, agent_name, port)
                }
                "cloudrun" => deploy_cloudrun(agent_name, &parts),
                "aws" => deploy_aws(agent_name, &parts),
                _ => CommandResult::Error(format!(
                    "Unknown deploy target: {}. Use: local, cloudrun, aws",
                    target
                )),
            }
        }
    }
}

fn deploy_local(_app: &mut App, agent_name: &str, port: u16) -> CommandResult {
    // Discover agent bundle
    let bundle = match deploy::AgentBundle::discover(agent_name) {
        Ok(b) => b,
        Err(e) => return CommandResult::Error(e.to_string()),
    };

    let rt = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(_) => return CommandResult::Error("No async runtime".to_string()),
    };

    // Build image
    let image_name =
        match tokio::task::block_in_place(|| rt.block_on(deploy::build_image(&bundle, None))) {
            Ok(name) => name,
            Err(e) => return CommandResult::Error(format!("Build failed: {}", e)),
        };

    // Collect env vars for the container: shell env (LLM keys) + stored secrets
    let mut env_vars = std::collections::HashMap::new();
    for key in &["ANTHROPIC_API_KEY", "OPENAI_API_KEY", "OPENROUTER_API_KEY"] {
        if let Ok(val) = std::env::var(key) {
            env_vars.insert(key.to_string(), val);
        }
    }

    // Add stored secrets (encrypted in .symbi/secrets.enc)
    if let Ok(stored) = crate::secrets_store::all_as_env() {
        for (k, v) in stored {
            env_vars.insert(k, v);
        }
    }

    // Run container
    match tokio::task::block_in_place(|| {
        rt.block_on(deploy::run_container(&image_name, port, &env_vars))
    }) {
        Ok(container_id) => CommandResult::Output(format!(
            "Deployed {} locally\n\n\
             Container: {}\n\
             API:       http://localhost:{}\n\
             HTTP Input: http://localhost:{}\n\n\
             Commands:\n\
             /deploy logs {}    — view logs\n\
             /deploy stop {}    — stop and remove",
            agent_name,
            &container_id[..12.min(container_id.len())],
            port,
            port + 1,
            agent_name,
            agent_name,
        )),
        Err(e) => CommandResult::Error(format!("Failed to start: {}", e)),
    }
}

fn deploy_list() -> CommandResult {
    let rt = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(_) => return CommandResult::Error("No async runtime".to_string()),
    };

    match tokio::task::block_in_place(|| rt.block_on(deploy::list_deployments())) {
        Ok(deployments) if deployments.is_empty() => {
            CommandResult::Output("No deployed agents.".to_string())
        }
        Ok(deployments) => {
            let mut out = String::from("Deployed agents:\n\n");
            for (name, status, ports) in &deployments {
                out.push_str(&format!("  {} — {} ({})\n", name, status, ports));
            }
            CommandResult::Output(out)
        }
        Err(e) => CommandResult::Error(format!("Failed to list: {}", e)),
    }
}

fn deploy_stop(agent_name: &str) -> CommandResult {
    let rt = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(_) => return CommandResult::Error("No async runtime".to_string()),
    };

    match tokio::task::block_in_place(|| rt.block_on(deploy::stop_deployment(agent_name))) {
        Ok(name) => CommandResult::Output(format!("Stopped and removed: {}", name)),
        Err(e) => CommandResult::Error(format!("Failed to stop: {}", e)),
    }
}

fn deploy_logs(agent_name: &str, tail: usize) -> CommandResult {
    if agent_name.is_empty() {
        return CommandResult::Error("Usage: /deploy logs <agent>".to_string());
    }

    let rt = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(_) => return CommandResult::Error("No async runtime".to_string()),
    };

    match tokio::task::block_in_place(|| rt.block_on(deploy::get_logs(agent_name, tail))) {
        Ok(logs) if logs.is_empty() => {
            CommandResult::Output(format!("No logs for agent '{}'", agent_name))
        }
        Ok(logs) => CommandResult::Output(format!("Logs for {}:\n\n{}", agent_name, logs)),
        Err(e) => CommandResult::Error(format!("Failed to get logs: {}", e)),
    }
}

fn deploy_cloudrun(agent_name: &str, parts: &[&str]) -> CommandResult {
    // Parse required --project
    let project = match find_flag_value(parts, "--project") {
        Some(p) => p,
        None => {
            return CommandResult::Error(
                "Missing --project.\n\
                 Usage: /deploy @<agent> cloudrun --project <id> [--region <r>]"
                    .to_string(),
            )
        }
    };

    let region = find_flag_value(parts, "--region").unwrap_or("us-central1");
    let service_account = find_flag_value(parts, "--service-account");
    let allow_unauthenticated = parts.contains(&"--allow-unauthenticated");

    let bundle = match deploy::AgentBundle::discover(agent_name) {
        Ok(b) => b,
        Err(e) => return CommandResult::Error(e.to_string()),
    };

    let rt = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(_) => return CommandResult::Error("No async runtime".to_string()),
    };

    // Collect secrets (LLM keys + stored secrets)
    let mut secrets = std::collections::HashMap::new();
    for key in &["ANTHROPIC_API_KEY", "OPENAI_API_KEY", "OPENROUTER_API_KEY"] {
        if let Ok(val) = std::env::var(key) {
            secrets.insert(key.to_string(), val);
        }
    }
    if let Ok(stored) = crate::secrets_store::all_as_env() {
        for (k, v) in stored {
            secrets.insert(k, v);
        }
    }

    let mut options = deploy::CloudRunOptions::new(project, region);
    options.service_account = service_account;
    options.allow_unauthenticated = allow_unauthenticated;

    match tokio::task::block_in_place(|| {
        rt.block_on(deploy::deploy_cloudrun(&bundle, &options, &secrets))
    }) {
        Ok(url) => CommandResult::Output(format!(
            "Deployed {} to Cloud Run\n\n\
             URL:      {}\n\
             Project:  {}\n\
             Region:   {}\n\
             Secrets:  {} injected via Secret Manager\n\n\
             Commands:\n\
             /attach {}    — connect to manage the agent\n\
             gcloud run services logs tail symbiont-{} --region={}  # stream logs",
            agent_name,
            url,
            project,
            region,
            secrets.len(),
            url,
            agent_name,
            region,
        )),
        Err(e) => CommandResult::Error(format!("Cloud Run deployment failed:\n{}", e)),
    }
}

fn deploy_aws(agent_name: &str, parts: &[&str]) -> CommandResult {
    let region = find_flag_value(parts, "--region").unwrap_or("us-east-1");
    let access_role_arn = find_flag_value(parts, "--access-role-arn");
    let instance_role_arn = find_flag_value(parts, "--instance-role-arn");

    let bundle = match deploy::AgentBundle::discover(agent_name) {
        Ok(b) => b,
        Err(e) => return CommandResult::Error(e.to_string()),
    };

    let rt = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(_) => return CommandResult::Error("No async runtime".to_string()),
    };

    // Collect secrets (LLM keys + stored secrets)
    let mut secrets = std::collections::HashMap::new();
    for key in &["ANTHROPIC_API_KEY", "OPENAI_API_KEY", "OPENROUTER_API_KEY"] {
        if let Ok(val) = std::env::var(key) {
            secrets.insert(key.to_string(), val);
        }
    }
    if let Ok(stored) = crate::secrets_store::all_as_env() {
        for (k, v) in stored {
            secrets.insert(k, v);
        }
    }

    let mut options = deploy::AwsOptions::new(region);
    options.access_role_arn = access_role_arn;
    options.instance_role_arn = instance_role_arn;

    match tokio::task::block_in_place(|| {
        rt.block_on(deploy::deploy_aws(&bundle, &options, &secrets))
    }) {
        Ok(url) => CommandResult::Output(format!(
            "Deployed {} to AWS App Runner\n\n\
             URL:     {}\n\
             Region:  {}\n\
             Secrets: {} uploaded to Secrets Manager\n\n\
             Commands:\n\
             /attach {}    — connect to manage the agent\n\
             aws apprunner list-services --region {}  # verify deployment",
            agent_name,
            url,
            region,
            secrets.len(),
            url,
            region,
        )),
        Err(e) => CommandResult::Error(format!("AWS deployment failed:\n{}", e)),
    }
}

/// Find the value after a --flag argument.
fn find_flag_value<'a>(parts: &'a [&'a str], flag: &str) -> Option<&'a str> {
    parts
        .iter()
        .position(|&p| p == flag)
        .and_then(|i| parts.get(i + 1))
        .copied()
}
