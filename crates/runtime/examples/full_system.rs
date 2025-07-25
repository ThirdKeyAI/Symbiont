//! Full System Example
//!
//! Demonstrates a complete agent runtime system with all components.

use std::collections::HashMap;
use std::time::Duration;
use symbiont_runtime::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Symbiont Agent Runtime - Full System Example ===");

    // Initialize all components with default configurations
    let lifecycle_config = LifecycleConfig::default();
    let resource_config = ResourceManagerConfig::default();
    let scheduler_config = SchedulerConfig::default();
    let comm_config = CommunicationConfig::default();
    let error_config = ErrorHandlerConfig::default();

    println!("Initializing runtime components...");

    // Create component instances (using mock implementations for demo)
    let lifecycle_controller = DefaultLifecycleController::new(lifecycle_config).await?;
    let resource_manager = DefaultResourceManager::new(resource_config).await?;
    let scheduler = DefaultAgentScheduler::new(scheduler_config).await?;
    let comm_bus = DefaultCommunicationBus::new(comm_config).await?;
    let error_handler = DefaultErrorHandler::new(error_config).await?;

    println!("✓ All components initialized");

    // Create multiple agents with different configurations
    let agents = vec![
        create_agent_config(
            "web_scraper",
            ExecutionMode::Scheduled {
                interval: Duration::from_secs(3600),
            },
            SecurityTier::Tier2,
            Priority::Normal,
        ),
        create_agent_config(
            "data_processor",
            ExecutionMode::Persistent,
            SecurityTier::Tier1,
            Priority::High,
        ),
        create_agent_config(
            "notification_service",
            ExecutionMode::EventDriven,
            SecurityTier::Tier1,
            Priority::Low,
        ),
    ];

    println!("\n=== Creating Agents ===");
    let mut agent_configs = Vec::new();

    for config in agents {
        println!("Creating agent: {}", config.name);

        // Allocate resources
        let requirements = ResourceRequirements {
            min_memory_mb: config.resource_limits.memory_mb / 2,
            max_memory_mb: config.resource_limits.memory_mb,
            min_cpu_cores: config.resource_limits.cpu_cores / 2.0,
            max_cpu_cores: config.resource_limits.cpu_cores,
            disk_space_mb: 1000,
            network_bandwidth_mbps: config.resource_limits.network_io_mbps,
        };
        let _allocation = resource_manager
            .allocate_resources(config.id, requirements)
            .await?;
        println!("  ✓ Resources allocated");

        // Register with communication bus
        comm_bus.register_agent(config.id).await?;
        println!("  ✓ Registered with communication bus");

        agent_configs.push(config);
    }

    println!("\n=== Starting Agents ===");
    let mut initialized_agent_ids = Vec::new();

    for config in &agent_configs {
        // Initialize agent (Created → Initializing)
        let initialized_id = lifecycle_controller
            .initialize_agent(config.clone())
            .await?;
        println!("  ✓ Agent {} initialized", config.name);

        // Wait a moment for initialization to process
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Check current state
        let current_state = lifecycle_controller.get_agent_state(initialized_id).await?;
        println!("  Current state: {:?}", current_state);

        // Note: In a real implementation, the agent would automatically transition to Ready
        // For this demo, we'll skip the start since we can't manually transition to Ready
        println!(
            "  ✓ Agent {} ready for operation (would start in real system)",
            config.name
        );

        initialized_agent_ids.push(initialized_id);
    }

    println!("\n=== Inter-Agent Communication ===");
    if initialized_agent_ids.len() >= 2 && agent_configs.len() >= 2 {
        let sender_id = initialized_agent_ids[0];
        let receiver_id = initialized_agent_ids[1];
        let sender_name = &agent_configs[0].name;
        let receiver_name = &agent_configs[1].name;

        // Subscribe receiver to a topic
        comm_bus
            .subscribe(receiver_id, "data_updates".to_string())
            .await?;
        println!("✓ {} subscribed to 'data_updates' topic", receiver_name);

        // Send message from sender to receiver
        let message = SecureMessage {
            id: MessageId::new(),
            sender: sender_id,
            recipient: Some(receiver_id),
            topic: Some("data_updates".to_string()),
            payload: EncryptedPayload {
                data: b"Hello from web scraper!".to_vec().into(),
                encryption_algorithm: EncryptionAlgorithm::None, // Demo - no encryption
                nonce: vec![0; 12],                              // Demo nonce
            },
            signature: MessageSignature {
                signature: vec![0; 64],              // Demo signature
                algorithm: SignatureAlgorithm::None, // Demo - no signature
                public_key: vec![0; 32],             // Demo public key
            },
            timestamp: std::time::SystemTime::now(),
            ttl: Duration::from_secs(300),
            message_type: MessageType::Direct(receiver_id),
        };

        let message_id = comm_bus.send_message(message).await?;
        println!("✓ Message sent from {} to {}", sender_name, receiver_name);
        println!("  Message ID: {}", message_id);
    }

    println!("\n=== Error Handling Demo ===");
    if let Some(&agent_id) = initialized_agent_ids.first() {
        let agent_name = &agent_configs[0].name;

        // Simulate an error
        let error = RuntimeError::Resource(ResourceError::InsufficientResources {
            requirements: "Need more memory".to_string(),
        });

        let action = error_handler.handle_error(agent_id, error).await?;
        println!("✓ Error handled for agent: {}", agent_name);
        println!("  Recommended action: {:?}", action);

        // Get error statistics
        tokio::time::sleep(Duration::from_millis(100)).await; // Allow processing
        let stats = error_handler.get_error_stats(agent_id).await?;
        println!(
            "  Error stats: {} total, {} recent",
            stats.total_errors, stats.recent_errors
        );
    }

    println!("\n=== System Status ===");

    // Get resource status
    let resource_status = resource_manager.get_system_status().await;
    println!("Total memory: {} MB", resource_status.total_memory);
    println!("Available memory: {} MB", resource_status.available_memory);
    println!("Total CPU cores: {}", resource_status.total_cpu_cores);
    println!(
        "Available CPU cores: {}",
        resource_status.available_cpu_cores
    );

    // Get system error stats
    let error_stats = error_handler.get_system_error_stats().await;
    println!(
        "System errors: {} total, {} recent",
        error_stats.total_errors, error_stats.recent_errors
    );

    println!("\n=== Cleanup ===");

    // Graceful shutdown
    for (i, &agent_id) in initialized_agent_ids.iter().enumerate() {
        let agent_name = &agent_configs[i].name;
        println!("Stopping agent: {}", agent_name);
        lifecycle_controller.terminate_agent(agent_id).await?;
        resource_manager.deallocate_resources(agent_id).await?;
        comm_bus.unregister_agent(agent_id).await?;
        println!("  ✓ Agent stopped and cleaned up");
    }

    // Shutdown components
    scheduler.shutdown().await?;
    comm_bus.shutdown().await?;
    error_handler.shutdown().await?;
    resource_manager.shutdown().await?;
    lifecycle_controller.shutdown().await?;

    println!("✓ All components shut down gracefully");
    println!("\n=== Example Complete ===");

    Ok(())
}

fn create_agent_config(
    name: &str,
    execution_mode: ExecutionMode,
    security_tier: SecurityTier,
    priority: Priority,
) -> AgentConfig {
    let mut metadata = HashMap::new();
    metadata.insert("example".to_string(), "full_system".to_string());
    metadata.insert("created_by".to_string(), "demo".to_string());

    AgentConfig {
        id: AgentId::new(),
        name: name.to_string(),
        dsl_source: format!("// Agent logic for {}", name),
        execution_mode,
        security_tier,
        resource_limits: ResourceLimits {
            memory_mb: match priority {
                Priority::Critical => 1024,
                Priority::High => 512,
                Priority::Normal => 256,
                Priority::Low => 128,
            },
            cpu_cores: match priority {
                Priority::Critical => 2.0,
                Priority::High => 1.0,
                Priority::Normal => 0.5,
                Priority::Low => 0.25,
            },
            disk_io_mbps: 50,
            network_io_mbps: 10,
            execution_timeout: Duration::from_secs(3600),
            idle_timeout: Duration::from_secs(300),
        },
        capabilities: vec![
            Capability::FileSystem,
            Capability::Network,
            Capability::Custom(format!("{}_specific", name)),
        ],
        policies: vec![],
        metadata,
        priority,
    }
}
