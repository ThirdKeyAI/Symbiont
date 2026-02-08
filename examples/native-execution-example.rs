//! Example demonstrating native (non-isolated) execution mode
//!
//! ⚠️ WARNING: This example shows how to run code without Docker/container
//! isolation. Use ONLY in trusted development environments!

use std::collections::HashMap;
use symbi_runtime::sandbox::{NativeConfig, NativeRunner, SandboxRunner};
use tokio::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    println!("=== Symbiont Native Execution Example ===\n");
    println!("⚠️  WARNING: Running code without isolation!\n");

    // Example 1: Basic Python execution
    example_python_execution().await?;

    // Example 2: Bash script execution
    example_bash_execution().await?;

    // Example 3: With environment variables
    example_with_env_vars().await?;

    // Example 4: With resource limits
    example_with_limits().await?;

    // Example 5: Timeout handling
    example_timeout().await?;

    Ok(())
}

async fn example_python_execution() -> Result<(), Box<dyn std::error::Error>> {
    println!("Example 1: Python Execution");
    println!("----------------------------");

    let mut config = NativeConfig::default();
    config.executable = "python3".to_string();

    let runner = NativeRunner::new(config)?;

    let code = r#"
import sys
print("Hello from native Python execution!")
print(f"Python version: {sys.version}")
print("This is running directly on your host system.")
"#;

    match runner.execute(code, HashMap::new()).await {
        Ok(result) => {
            println!("✓ Success!");
            println!("Output:\n{}", result.stdout);
            if !result.stderr.is_empty() {
                println!("Stderr:\n{}", result.stderr);
            }
        }
        Err(e) => {
            println!("✗ Error: {}", e);
        }
    }

    println!();
    Ok(())
}

async fn example_bash_execution() -> Result<(), Box<dyn std::error::Error>> {
    println!("Example 2: Bash Script Execution");
    println!("---------------------------------");

    let config = NativeConfig::default(); // Uses bash by default
    let runner = NativeRunner::new(config)?;

    let code = r#"
echo "Hello from native Bash execution!"
echo "Current directory: $(pwd)"
echo "User: $(whoami)"
echo "Date: $(date)"
"#;

    match runner.execute(code, HashMap::new()).await {
        Ok(result) => {
            println!("✓ Success!");
            println!("Output:\n{}", result.stdout);
        }
        Err(e) => {
            println!("✗ Error: {}", e);
        }
    }

    println!();
    Ok(())
}

async fn example_with_env_vars() -> Result<(), Box<dyn std::error::Error>> {
    println!("Example 3: Execution with Environment Variables");
    println!("-----------------------------------------------");

    let mut config = NativeConfig::default();
    config.executable = "bash".to_string();

    let runner = NativeRunner::new(config)?;

    let mut env = HashMap::new();
    env.insert("GREETING".to_string(), "Hello".to_string());
    env.insert("NAME".to_string(), "Symbiont User".to_string());
    env.insert("API_KEY".to_string(), "secret_key_123".to_string());

    let code = r#"
echo "$GREETING, $NAME!"
echo "API Key length: ${#API_KEY}"
"#;

    match runner.execute(code, env).await {
        Ok(result) => {
            println!("✓ Success!");
            println!("Output:\n{}", result.stdout);
        }
        Err(e) => {
            println!("✗ Error: {}", e);
        }
    }

    println!();
    Ok(())
}

async fn example_with_limits() -> Result<(), Box<dyn std::error::Error>> {
    println!("Example 4: Execution with Resource Limits");
    println!("-----------------------------------------");

    let mut config = NativeConfig::default();
    config.enforce_resource_limits = true;
    config.max_memory_mb = Some(512);
    config.max_cpu_seconds = Some(10);

    let runner = NativeRunner::new(config)?;

    let code = r#"
echo "This execution has resource limits applied"
echo "Memory limit: 512MB"
echo "CPU time limit: 10 seconds"
"#;

    match runner.execute(code, HashMap::new()).await {
        Ok(result) => {
            println!("✓ Success!");
            println!("Output:\n{}", result.stdout);
        }
        Err(e) => {
            println!("✗ Error: {}", e);
        }
    }

    println!();
    Ok(())
}

async fn example_timeout() -> Result<(), Box<dyn std::error::Error>> {
    println!("Example 5: Timeout Handling");
    println!("---------------------------");

    let mut config = NativeConfig::default();
    config.max_execution_time = Duration::from_secs(2);

    let runner = NativeRunner::new(config)?;

    // This will timeout
    let code = "sleep 5; echo 'This should not print'";

    println!("Attempting to sleep for 5 seconds with 2-second timeout...");

    match runner.execute(code, HashMap::new()).await {
        Ok(result) => {
            println!("✓ Completed (unexpected)");
            println!("Output:\n{}", result.stdout);
        }
        Err(e) => {
            println!("✗ Timed out as expected: {}", e);
        }
    }

    println!();
    Ok(())
}
