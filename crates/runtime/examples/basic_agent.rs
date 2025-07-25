//! Basic Agent Example
//!
//! Demonstrates creating, configuring, and managing a simple agent.

use std::collections::HashMap;
use std::time::Duration;
use symbiont_runtime::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    println!("=== Symbiont Agent Runtime - Basic Agent Example ===");

    // Create agent configuration
    let agent_config = AgentConfig {
        id: AgentId::new(),
        name: "basic_example_agent".to_string(),
        dsl_source: "
            // Simple agent logic
            fn main() {
                println!(\"Hello from agent!\");
                return \"success\";
            }
        "
        .to_string(),
        execution_mode: ExecutionMode::Ephemeral,
        security_tier: SecurityTier::Tier1,
        resource_limits: ResourceLimits {
            memory_mb: 256,
            cpu_cores: 0.5,
            disk_io_mbps: 25,
            network_io_mbps: 5,
            execution_timeout: Duration::from_secs(300),
            idle_timeout: Duration::from_secs(60),
        },
        capabilities: vec![
            Capability::FileSystem,
            Capability::Custom("logging".to_string()),
        ],
        policies: vec![],
        metadata: {
            let mut meta = HashMap::new();
            meta.insert("example".to_string(), "basic".to_string());
            meta.insert("version".to_string(), "1.0".to_string());
            meta
        },
        priority: Priority::Normal,
    };

    println!("Agent ID: {}", agent_config.id);
    println!("Agent Name: {}", agent_config.name);
    println!("Security Tier: {:?}", agent_config.security_tier);
    println!("Resource Limits: {:?}", agent_config.resource_limits);

    // Create agent instance
    let agent = AgentInstance::new(agent_config);

    println!("\n=== Agent Created ===");
    println!("State: {:?}", agent.state);
    println!("Created at: {:?}", agent.created_at);
    println!("Execution count: {}", agent.execution_count);

    // Demonstrate state transitions
    println!("\n=== Agent States ===");
    let states = vec![
        AgentState::Created,
        AgentState::Initializing,
        AgentState::Ready,
        AgentState::Running,
        AgentState::Completed,
    ];

    for state in states {
        println!("State: {:?}", state);
    }

    // Demonstrate priority ordering
    println!("\n=== Priority Levels ===");
    let priorities = vec![
        Priority::Low,
        Priority::Normal,
        Priority::High,
        Priority::Critical,
    ];
    for priority in priorities {
        println!("Priority: {:?} (value: {})", priority, priority as u8);
    }

    println!("\n=== Example Complete ===");
    Ok(())
}
