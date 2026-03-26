//! Integration tests for the Symbiont Agent Runtime System
//!
//! These tests verify that the core components can be instantiated and basic types work correctly.

use std::time::Duration;
use symbi_runtime::error_handler::ErrorType;
use symbi_runtime::*;

#[tokio::test]
async fn test_basic_id_generation() {
    // Test that all core ID types can be created and are unique
    let agent_id1 = AgentId::new();
    let agent_id2 = AgentId::new();
    let request_id1 = RequestId::new();
    let request_id2 = RequestId::new();
    let audit_id1 = AuditId::new();
    let audit_id2 = AuditId::new();

    // Verify IDs are unique
    assert_ne!(agent_id1, agent_id2);
    assert_ne!(request_id1, request_id2);
    assert_ne!(audit_id1, audit_id2);
}

#[tokio::test]
async fn test_agent_state_enum() {
    // Test agent state enum variants exist and are distinct
    let states = [
        AgentState::Created,
        AgentState::Initializing,
        AgentState::Ready,
        AgentState::Running,
        AgentState::Suspended,
        AgentState::Waiting,
        AgentState::Completed,
        AgentState::Failed,
        AgentState::Terminating,
        AgentState::Terminated,
    ];

    // Verify all states are distinct
    for (i, state1) in states.iter().enumerate() {
        for (j, state2) in states.iter().enumerate() {
            if i != j {
                assert_ne!(state1, state2);
            }
        }
    }
}

#[tokio::test]
async fn test_priority_ordering() {
    // Test priority enum ordering
    assert!(Priority::Critical > Priority::High);
    assert!(Priority::High > Priority::Normal);
    assert!(Priority::Normal > Priority::Low);
}

#[tokio::test]
async fn test_isolation_levels() {
    // Test isolation level variants from security module
    let levels = [
        IsolationLevel::None,
        IsolationLevel::Low,
        IsolationLevel::Medium,
        IsolationLevel::High,
        IsolationLevel::Maximum,
    ];

    // Verify all levels are distinct
    for (i, level1) in levels.iter().enumerate() {
        for (j, level2) in levels.iter().enumerate() {
            if i != j {
                assert_ne!(level1, level2);
            }
        }
    }
}

#[tokio::test]
async fn test_execution_modes() {
    // Test execution mode variants from agent module
    let modes = [
        ExecutionMode::Persistent,
        ExecutionMode::Ephemeral,
        ExecutionMode::Scheduled {
            interval: Duration::from_secs(60),
        },
        ExecutionMode::EventDriven,
    ];

    // Verify all modes are distinct
    for (i, mode1) in modes.iter().enumerate() {
        for (j, mode2) in modes.iter().enumerate() {
            if i != j {
                assert_ne!(mode1, mode2);
            }
        }
    }
}

#[tokio::test]
async fn test_capabilities() {
    // Test capability variants
    let capabilities = [
        Capability::FileSystem,
        Capability::Network,
        Capability::Database,
        Capability::Custom("test".to_string()),
    ];

    // Verify capabilities can be created
    assert_eq!(capabilities.len(), 4);

    // Test custom capability
    if let Capability::Custom(name) = &capabilities[3] {
        assert_eq!(name, "test");
    } else {
        panic!("Expected Custom capability");
    }
}

#[tokio::test]
async fn test_resource_limits() {
    // Test resource limits creation with correct field names
    let limits = ResourceLimits {
        memory_mb: 1024,
        cpu_cores: 2.0,
        disk_io_mbps: 100,
        network_io_mbps: 10,
        execution_timeout: Duration::from_secs(3600),
        idle_timeout: Duration::from_secs(300),
    };

    assert_eq!(limits.memory_mb, 1024);
    assert_eq!(limits.cpu_cores, 2.0);
    assert_eq!(limits.disk_io_mbps, 100);
    assert_eq!(limits.network_io_mbps, 10);
}

#[tokio::test]
async fn test_load_balancing_strategies() {
    // Test load balancing strategy variants
    let strategies = [
        LoadBalancingStrategy::RoundRobin,
        LoadBalancingStrategy::LeastConnections,
        LoadBalancingStrategy::ResourceBased,
        LoadBalancingStrategy::WeightedRoundRobin,
    ];

    // Verify all strategies are distinct
    for (i, strategy1) in strategies.iter().enumerate() {
        for (j, strategy2) in strategies.iter().enumerate() {
            if i != j {
                assert_ne!(strategy1, strategy2);
            }
        }
    }
}

#[tokio::test]
async fn test_encryption_algorithms() {
    // Test encryption algorithm variants
    let algorithms = [
        EncryptionAlgorithm::Aes256Gcm,
        EncryptionAlgorithm::ChaCha20Poly1305,
    ];

    // Verify all algorithms are distinct
    for (i, alg1) in algorithms.iter().enumerate() {
        for (j, alg2) in algorithms.iter().enumerate() {
            if i != j {
                assert_ne!(alg1, alg2);
            }
        }
    }
}

#[tokio::test]
async fn test_error_types() {
    // Test that error types can be created with correct syntax
    let agent_id = AgentId::new();
    let lifecycle_error = LifecycleError::AgentNotFound { agent_id };

    let resource_error = ResourceError::InsufficientResources {
        requirements: "Need more memory".into(),
    };

    let comm_error = CommunicationError::MessageTooLarge {
        size: 2048,
        max_size: 1024,
    };

    // Verify errors can be created and match expected patterns
    assert!(matches!(
        lifecycle_error,
        LifecycleError::AgentNotFound { .. }
    ));
    assert!(matches!(
        resource_error,
        ResourceError::InsufficientResources { .. }
    ));
    assert!(matches!(
        comm_error,
        CommunicationError::MessageTooLarge { .. }
    ));
}

#[tokio::test]
async fn test_basic_configurations() {
    // Test that basic configuration structs can be created
    let lifecycle_config = LifecycleConfig {
        initialization_timeout: Duration::from_secs(60),
        termination_timeout: Duration::from_secs(30),
        state_check_interval: Duration::from_secs(5),
        enable_auto_recovery: true,
        max_restart_attempts: 3,
        max_agents: 100,
    };

    let resource_config = ResourceManagerConfig {
        total_memory: 8192 * 1024 * 1024, // 8GB
        total_cpu_cores: 8,
        total_disk_space: 1024 * 1024 * 1024 * 1024, // 1TB
        total_network_bandwidth: 1000 * 1024 * 1024, // 1GB/s
        enforcement_enabled: true,
        auto_scaling_enabled: false,
        resource_reservation_percentage: 10.0,
        monitoring_interval: Duration::from_secs(30),
        policy_enforcement_config: Default::default(),
    };

    let comm_config = CommunicationConfig {
        message_ttl: Duration::from_secs(300),
        max_queue_size: 1000,
        delivery_timeout: Duration::from_secs(30),
        retry_attempts: 3,
        enable_encryption: true,
        enable_compression: false,
        max_message_size: 1024 * 1024, // 1MB
        dead_letter_queue_size: 100,
    };

    let error_config = ErrorHandlerConfig {
        max_error_history: 1000,
        error_aggregation_window: Duration::from_secs(60),
        escalation_threshold: 5,
        circuit_breaker_threshold: 10,
        circuit_breaker_timeout: Duration::from_secs(60),
        enable_auto_recovery: true,
        max_recovery_attempts: 3,
        recovery_backoff_multiplier: 2.0,
    };

    // Verify configurations are valid
    assert!(lifecycle_config.enable_auto_recovery);
    assert_eq!(resource_config.total_cpu_cores, 8);
    assert!(comm_config.enable_encryption);
    assert_eq!(error_config.max_recovery_attempts, 3);
}

#[tokio::test]
async fn test_security_tiers() {
    // Test security tier variants
    let tiers = [
        SecurityTier::Tier1,
        SecurityTier::Tier2,
        SecurityTier::Tier3,
    ];

    // Verify all tiers are distinct and ordered
    for (i, tier1) in tiers.iter().enumerate() {
        for (j, tier2) in tiers.iter().enumerate() {
            if i != j {
                assert_ne!(tier1, tier2);
            }
        }
    }

    // Test ordering
    assert!(SecurityTier::Tier3 > SecurityTier::Tier2);
    assert!(SecurityTier::Tier2 > SecurityTier::Tier1);
}

#[tokio::test]
async fn test_recovery_strategies() {
    // Test recovery strategy variants with correct syntax
    let strategies = [
        RecoveryStrategy::Retry {
            max_attempts: 3,
            backoff: Duration::from_secs(1),
        },
        RecoveryStrategy::Restart {
            preserve_state: true,
        },
        RecoveryStrategy::Failover {
            backup_agent: Some(AgentId::new()),
        },
        RecoveryStrategy::Terminate { cleanup: true },
        RecoveryStrategy::Manual {
            reason: "Manual intervention required".to_string(),
        },
        RecoveryStrategy::None,
    ];

    // Verify all strategies can be created
    assert_eq!(strategies.len(), 6);
}

#[tokio::test]
async fn test_error_handler_types() {
    // Test error handler specific types
    let error_types = [
        ErrorType::ResourceExhaustion,
        ErrorType::NetworkError,
        ErrorType::SecurityViolation,
        ErrorType::PolicyViolation,
        ErrorType::SystemError,
        ErrorType::ValidationError,
    ];

    // Verify all error types are distinct
    for (i, type1) in error_types.iter().enumerate() {
        for (j, type2) in error_types.iter().enumerate() {
            if i != j {
                assert_ne!(type1, type2);
            }
        }
    }
}

#[tokio::test]
async fn test_type_system_consistency() {
    // Test that the type system is internally consistent

    // Test that IDs can be used in collections
    use std::collections::HashMap;
    let mut agent_map = HashMap::new();
    let agent_id = AgentId::new();
    agent_map.insert(agent_id, "test_agent");
    assert_eq!(agent_map.len(), 1);

    // Test that priorities can be compared
    let high_priority = Priority::High;
    let low_priority = Priority::Low;
    assert!(high_priority > low_priority);

    // Test that states can be cloned and compared
    let state1 = AgentState::Running;
    let state2 = state1.clone();
    assert_eq!(state1, state2);

    // Test that durations work correctly
    let timeout = Duration::from_secs(60);
    assert_eq!(timeout.as_secs(), 60);

    // Test that security tiers can be compared
    let tier1 = SecurityTier::Tier1;
    let tier3 = SecurityTier::Tier3;
    assert!(tier3 > tier1);
}

#[tokio::test]
async fn test_agent_instance_creation() {
    // Test that AgentInstance can be created with proper config
    let agent_config = AgentConfig {
        id: AgentId::new(),
        name: "test_agent".to_string(),
        dsl_source: "test dsl".to_string(),
        execution_mode: ExecutionMode::Ephemeral,
        security_tier: SecurityTier::Tier1,
        resource_limits: ResourceLimits {
            memory_mb: 512,
            cpu_cores: 1.0,
            disk_io_mbps: 50,
            network_io_mbps: 10,
            execution_timeout: Duration::from_secs(300),
            idle_timeout: Duration::from_secs(60),
        },
        capabilities: vec![Capability::FileSystem, Capability::Network],
        policies: vec![],
        metadata: std::collections::HashMap::new(),
        priority: Priority::Normal,
    };

    let agent_instance = AgentInstance::new(agent_config.clone());

    assert_eq!(agent_instance.id, agent_config.id);
    assert_eq!(agent_instance.state, AgentState::Created);
    assert_eq!(agent_instance.execution_count, 0);
    assert_eq!(agent_instance.error_count, 0);
    assert_eq!(agent_instance.restart_count, 0);
}

// ---------------------------------------------------------------------------
// Multi-Agent Scheduling Tests
// ---------------------------------------------------------------------------

#[test]
fn test_concurrent_agent_scheduling() {
    use symbi_runtime::scheduler::priority_queue::PriorityQueue;
    use symbi_runtime::scheduler::ScheduledTask;
    use symbi_runtime::types::*;

    let mut queue: PriorityQueue<ScheduledTask> = PriorityQueue::new();

    // Create 100 agents with rotating priorities: Low, Normal, High
    for i in 0..100 {
        let priority = match i % 3 {
            0 => Priority::Low,
            1 => Priority::Normal,
            _ => Priority::High,
        };
        let config = AgentConfig {
            id: AgentId::new(),
            name: format!("agent_{}", i),
            dsl_source: "test".to_string(),
            execution_mode: ExecutionMode::Ephemeral,
            security_tier: SecurityTier::Tier1,
            resource_limits: ResourceLimits::default(),
            capabilities: vec![],
            policies: vec![],
            metadata: std::collections::HashMap::new(),
            priority,
        };
        queue.push(ScheduledTask::new(config));
    }

    assert_eq!(queue.len(), 100);

    // Pop all and verify ordering: all High before Normal before Low
    let mut last_priority = Priority::Critical; // highest possible
    while let Some(task) = queue.pop() {
        assert!(
            task.priority <= last_priority,
            "Priority ordering violated: {:?} after {:?}",
            task.priority,
            last_priority
        );
        last_priority = task.priority;
    }

    assert!(queue.is_empty());
}

#[test]
fn test_agent_isolation() {
    // Verify that agent IDs are unique across 10,000 generations
    use std::collections::HashSet;
    use symbi_runtime::types::AgentId;

    let mut ids = HashSet::new();
    for _ in 0..10_000 {
        let id = AgentId::new();
        assert!(ids.insert(id), "AgentId collision detected!");
    }
    assert_eq!(ids.len(), 10_000);
}

#[test]
fn test_communication_error_types() {
    use symbi_runtime::types::error::*;
    use symbi_runtime::types::*;

    // Verify DeliveryFailed error carries the message ID and reason
    let msg_id = MessageId::new();
    let err = CommunicationError::DeliveryFailed {
        message_id: msg_id,
        reason: "recipient not found".into(),
    };
    let msg = format!("{err}");
    assert!(
        msg.contains("recipient not found"),
        "Error message should contain the reason, got: {msg}"
    );
    assert!(
        msg.contains(&msg_id.to_string()),
        "Error message should contain the message ID, got: {msg}"
    );

    // Verify MessageTooLarge error carries size info
    let err2 = CommunicationError::MessageTooLarge {
        size: 2048,
        max_size: 1024,
    };
    let msg2 = format!("{err2}");
    assert!(
        msg2.contains("2048"),
        "Error message should contain actual size, got: {msg2}"
    );
    assert!(
        msg2.contains("1024"),
        "Error message should contain max size, got: {msg2}"
    );

    // Verify AgentNotRegistered error
    let agent_id = AgentId::new();
    let err3 = CommunicationError::AgentNotRegistered { agent_id };
    let msg3 = format!("{err3}");
    assert!(
        msg3.contains(&agent_id.to_string()),
        "Error message should contain agent ID, got: {msg3}"
    );
}

// ---------------------------------------------------------------------------
// Sandbox Lifecycle Test
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_sandbox_full_lifecycle() {
    use symbi_runtime::integrations::sandbox_orchestrator::{
        MockSandboxOrchestrator, NetworkConfig, NetworkMode, RestartPolicy, SandboxCommand,
        SandboxConfig, SandboxOrchestrator, SandboxRequest, SandboxStatus, SandboxType,
        SecurityOptions, StorageConfig,
    };
    use symbi_runtime::types::{AgentId, ResourceLimits, SecurityTier};

    let orchestrator = MockSandboxOrchestrator::new();
    let agent_id = AgentId::new();

    // Create sandbox
    let request = SandboxRequest {
        agent_id,
        sandbox_type: SandboxType::Docker {
            image: "ubuntu".to_string(),
            tag: "22.04".to_string(),
        },
        config: SandboxConfig {
            name: "lifecycle-test-sandbox".to_string(),
            description: "Integration test sandbox".to_string(),
            environment_variables: std::collections::HashMap::new(),
            working_directory: None,
            command: None,
            entrypoint: None,
            user: None,
            group: None,
            capabilities: vec![],
            security_options: SecurityOptions {
                read_only_root: false,
                no_new_privileges: true,
                seccomp_profile: None,
                apparmor_profile: None,
                selinux_label: None,
                privileged: false,
                drop_capabilities: vec![],
                add_capabilities: vec![],
            },
            auto_remove: true,
            restart_policy: RestartPolicy::Never,
            health_check: None,
        },
        security_level: SecurityTier::Tier2,
        resource_limits: ResourceLimits {
            memory_mb: 256,
            cpu_cores: 1.0,
            disk_io_mbps: 50,
            network_io_mbps: 10,
            execution_timeout: Duration::from_secs(120),
            idle_timeout: Duration::from_secs(30),
        },
        network_config: NetworkConfig {
            mode: NetworkMode::Bridge,
            ports: vec![],
            dns_servers: vec![],
            dns_search: vec![],
            hostname: Some("test-host".to_string()),
            extra_hosts: std::collections::HashMap::new(),
            network_aliases: vec![],
        },
        storage_config: StorageConfig {
            volumes: vec![],
            tmpfs_mounts: vec![],
            storage_driver: None,
            storage_options: std::collections::HashMap::new(),
        },
        metadata: std::collections::HashMap::new(),
    };

    let info = orchestrator.create_sandbox(request).await.unwrap();
    assert_eq!(info.status, SandboxStatus::Created);
    assert_eq!(info.agent_id, agent_id);

    // Start sandbox
    orchestrator.start_sandbox(info.id).await.unwrap();
    let running_info = orchestrator.get_sandbox_info(info.id).await.unwrap();
    assert_eq!(running_info.status, SandboxStatus::Running);
    assert!(running_info.started_at.is_some());

    // Execute command
    let command = SandboxCommand {
        command: vec!["echo".to_string(), "hello".to_string()],
        working_dir: None,
        environment: std::collections::HashMap::new(),
        user: None,
        timeout: None,
        stdin: None,
    };
    let result = orchestrator
        .execute_command(info.id, command)
        .await
        .unwrap();
    assert_eq!(result.exit_code, 0);
    assert!(!result.timed_out);
    assert!(!result.stdout.is_empty());

    // Stop sandbox
    orchestrator.stop_sandbox(info.id).await.unwrap();
    let stopped_info = orchestrator.get_sandbox_info(info.id).await.unwrap();
    assert_eq!(stopped_info.status, SandboxStatus::Stopped);
    assert!(stopped_info.stopped_at.is_some());

    // Destroy sandbox
    orchestrator.destroy_sandbox(info.id).await.unwrap();
    let destroyed_info = orchestrator.get_sandbox_info(info.id).await.unwrap();
    assert_eq!(destroyed_info.status, SandboxStatus::Destroyed);

    // Operations on a non-existent sandbox should fail
    let fake_id = uuid::Uuid::new_v4();
    assert!(orchestrator.start_sandbox(fake_id).await.is_err());
    assert!(orchestrator.stop_sandbox(fake_id).await.is_err());
    assert!(orchestrator.destroy_sandbox(fake_id).await.is_err());
    assert!(orchestrator.get_sandbox_info(fake_id).await.is_err());
}
