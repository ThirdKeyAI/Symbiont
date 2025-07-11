# Symbiont Agent Runtime API Reference

Complete API documentation for the Symbiont Agent Runtime System.

## Core Types

### Identifiers

```rust
// Unique identifiers for various entities
pub struct AgentId(Uuid);
pub struct TaskId(Uuid);
pub struct MessageId(Uuid);
pub struct RequestId(Uuid);
pub struct AuditId(Uuid);
pub struct SandboxId(Uuid);
pub struct SnapshotId(Uuid);

impl AgentId {
    pub fn new() -> Self;
}
// Similar for all ID types
```

### Agent Types

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentState {
    Created,
    Initializing,
    Ready,
    Running,
    Suspended,
    Waiting,
    Completed,
    Failed,
    Terminating,
    Terminated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionMode {
    Persistent,
    Ephemeral,
    Scheduled { interval: Duration },
    EventDriven,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Priority {
    Critical = 4,
    High = 3,
    Normal = 2,
    Low = 1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Capability {
    FileSystem,
    Network,
    Database,
    Custom(String),
}

pub struct AgentConfig {
    pub id: AgentId,
    pub name: String,
    pub dsl_source: String,
    pub execution_mode: ExecutionMode,
    pub security_tier: SecurityTier,
    pub resource_limits: ResourceLimits,
    pub capabilities: Vec<Capability>,
    pub policies: Vec<Policy>,
    pub metadata: HashMap<String, String>,
    pub priority: Priority,
}

pub struct AgentInstance {
    pub id: AgentId,
    pub config: AgentConfig,
    pub state: AgentState,
    pub created_at: SystemTime,
    pub last_updated: SystemTime,
    pub execution_count: u64,
    pub error_count: u32,
    pub restart_count: u32,
}
```

### Resource Types

```rust
pub struct ResourceLimits {
    pub memory_mb: u64,
    pub cpu_cores: f64,
    pub disk_io_mbps: u64,
    pub network_io_mbps: u64,
    pub execution_timeout: Duration,
    pub idle_timeout: Duration,
}

pub struct ResourceUsage {
    pub memory_used: u64,
    pub cpu_usage: f64,
    pub disk_io_rate: u64,
    pub network_io_rate: u64,
    pub uptime: Duration,
}

pub struct ResourceAllocation {
    pub agent_id: AgentId,
    pub allocated_at: SystemTime,
    pub limits: ResourceLimits,
    pub current_usage: ResourceUsage,
}
```

### Security Types

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SecurityTier {
    Tier1 = 1, // Docker
    Tier2 = 2, // gVisor
    Tier3 = 3, // Firecracker
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolationLevel {
    None,
    Low,
    Medium,
    High,
    Maximum,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncryptionAlgorithm {
    Aes256Gcm,
    ChaCha20Poly1305,
}

pub struct SecurityContext {
    pub tier: SecurityTier,
    pub isolation_level: IsolationLevel,
    pub encryption_algorithm: EncryptionAlgorithm,
    pub signing_key: Option<Vec<u8>>,
    pub encryption_key: Option<Vec<u8>>,
}
```

### Communication Types

```rust
pub struct Message {
    pub id: MessageId,
    pub from: AgentId,
    pub to: AgentId,
    pub topic: String,
    pub payload: Vec<u8>,
    pub priority: Priority,
    pub ttl: Duration,
}

pub struct SecureMessage {
    pub message: Message,
    pub signature: Vec<u8>,
    pub encrypted_payload: Vec<u8>,
    pub timestamp: SystemTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryStatus {
    Pending,
    Delivered,
    Failed,
    Expired,
}
```

### Error Types

```rust
#[derive(Debug, Clone)]
pub enum RuntimeError {
    Resource(ResourceError),
    Communication(CommunicationError),
    Security(SecurityError),
    Scheduler(SchedulerError),
    Lifecycle(LifecycleError),
    ErrorHandler(ErrorHandlerError),
    Configuration(ConfigurationError),
    Policy(PolicyError),
    Sandbox(SandboxError),
    Audit(AuditError),
    Internal(String),
}

#[derive(Debug, Clone)]
pub enum ResourceError {
    InsufficientResources { requirements: String },
    AllocationFailed { agent_id: AgentId },
    DeallocationFailed { agent_id: AgentId },
    UsageExceeded { agent_id: AgentId, resource: String },
    MonitoringFailed { reason: String },
}

#[derive(Debug, Clone)]
pub enum LifecycleError {
    AgentNotFound { agent_id: AgentId },
    InvalidStateTransition { from: AgentState, to: AgentState },
    InitializationFailed { agent_id: AgentId, reason: String },
    TerminationFailed { agent_id: AgentId, reason: String },
    ConfigurationInvalid { reason: String },
}

#[derive(Debug, Clone)]
pub enum CommunicationError {
    AgentNotRegistered { agent_id: AgentId },
    MessageTooLarge { size: usize, max_size: usize },
    DeliveryFailed { message_id: MessageId, reason: String },
    EncryptionFailed { reason: String },
    TopicNotFound { topic: String },
}
```

## Core Interfaces

### 1. Lifecycle Controller

```rust
#[async_trait]
pub trait LifecycleController {
    async fn create_agent(&self, config: AgentConfig) -> Result<AgentInstance, LifecycleError>;
    async fn initialize_agent(&self, agent_id: AgentId) -> Result<(), LifecycleError>;
    async fn start_agent(&self, agent_id: AgentId) -> Result<(), LifecycleError>;
    async fn stop_agent(&self, agent_id: AgentId) -> Result<(), LifecycleError>;
    async fn suspend_agent(&self, agent_id: AgentId) -> Result<(), LifecycleError>;
    async fn resume_agent(&self, agent_id: AgentId) -> Result<(), LifecycleError>;
    async fn terminate_agent(&self, agent_id: AgentId) -> Result<(), LifecycleError>;
    async fn get_agent_state(&self, agent_id: AgentId) -> Result<AgentState, LifecycleError>;
    async fn list_agents(&self) -> Vec<AgentInstance>;
    async fn get_agent(&self, agent_id: AgentId) -> Result<AgentInstance, LifecycleError>;
    async fn update_agent_config(&self, agent_id: AgentId, config: AgentConfig) -> Result<(), LifecycleError>;
    async fn restart_agent(&self, agent_id: AgentId) -> Result<(), LifecycleError>;
    async fn get_system_status(&self) -> SystemStatus;
    async fn shutdown(&self) -> Result<(), LifecycleError>;
}

pub struct LifecycleConfig {
    pub initialization_timeout: Duration,
    pub termination_timeout: Duration,
    pub state_check_interval: Duration,
    pub enable_auto_recovery: bool,
    pub max_restart_attempts: u32,
    pub max_agents: usize,
}
```

### 2. Resource Manager

```rust
#[async_trait]
pub trait ResourceManager {
    async fn allocate_resources(&self, agent_id: AgentId, limits: ResourceLimits) -> Result<ResourceAllocation, ResourceError>;
    async fn deallocate_resources(&self, agent_id: AgentId) -> Result<(), ResourceError>;
    async fn update_resource_limits(&self, agent_id: AgentId, limits: ResourceLimits) -> Result<(), ResourceError>;
    async fn get_resource_usage(&self, agent_id: AgentId) -> Result<ResourceUsage, ResourceError>;
    async fn get_system_resources(&self) -> SystemResourceStatus;
    async fn check_resource_violations(&self) -> Vec<ResourceViolation>;
    async fn set_resource_alerts(&self, agent_id: AgentId, thresholds: ResourceThresholds) -> Result<(), ResourceError>;
    async fn get_resource_history(&self, agent_id: AgentId, duration: Duration) -> Result<Vec<ResourceSnapshot>, ResourceError>;
    async fn shutdown(&self) -> Result<(), ResourceError>;
}

pub struct ResourceManagerConfig {
    pub total_memory: usize,
    pub total_cpu_cores: u32,
    pub total_disk_space: usize,
    pub total_network_bandwidth: usize,
    pub enforcement_enabled: bool,
    pub auto_scaling_enabled: bool,
    pub resource_reservation_percentage: f32,
    pub monitoring_interval: Duration,
}
```

### 3. Scheduler

```rust
#[async_trait]
pub trait Scheduler {
    async fn schedule_task(&self, task: ScheduledTask) -> Result<(), SchedulerError>;
    async fn cancel_task(&self, task_id: TaskId) -> Result<(), SchedulerError>;
    async fn get_task_status(&self, task_id: TaskId) -> Result<TaskStatus, SchedulerError>;
    async fn list_pending_tasks(&self) -> Vec<ScheduledTask>;
    async fn list_running_tasks(&self) -> Vec<RunningTask>;
    async fn get_scheduler_metrics(&self) -> SchedulerMetrics;
    async fn update_task_priority(&self, task_id: TaskId, priority: Priority) -> Result<(), SchedulerError>;
    async fn pause_scheduling(&self) -> Result<(), SchedulerError>;
    async fn resume_scheduling(&self) -> Result<(), SchedulerError>;
    async fn shutdown(&self) -> Result<(), SchedulerError>;
}

pub struct ScheduledTask {
    pub id: TaskId,
    pub agent_id: AgentId,
    pub priority: Priority,
    pub scheduled_time: SystemTime,
    pub timeout: Duration,
    pub retry_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadBalancingStrategy {
    RoundRobin,
    LeastConnections,
    ResourceBased,
    WeightedRoundRobin,
}

pub struct SchedulerConfig {
    pub max_concurrent_tasks: usize,
    pub task_timeout: Duration,
    pub retry_attempts: u32,
    pub load_balancing_strategy: LoadBalancingStrategy,
    pub enable_priority_scheduling: bool,
    pub task_queue_size: usize,
    pub worker_threads: usize,
    pub health_check_interval: Duration,
}
```

### 4. Communication Bus

```rust
#[async_trait]
pub trait CommunicationBus {
    async fn register_agent(&self, agent_id: AgentId, capabilities: Vec<Capability>) -> Result<(), CommunicationError>;
    async fn unregister_agent(&self, agent_id: AgentId) -> Result<(), CommunicationError>;
    async fn send_message(&self, message: Message) -> Result<MessageId, CommunicationError>;
    async fn receive_messages(&self, agent_id: AgentId) -> Result<Vec<SecureMessage>, CommunicationError>;
    async fn subscribe_to_topic(&self, agent_id: AgentId, topic: String) -> Result<(), CommunicationError>;
    async fn unsubscribe_from_topic(&self, agent_id: AgentId, topic: String) -> Result<(), CommunicationError>;
    async fn broadcast_message(&self, topic: String, message: Message) -> Result<Vec<MessageId>, CommunicationError>;
    async fn get_message_status(&self, message_id: MessageId) -> Result<DeliveryStatus, CommunicationError>;
    async fn get_agent_topics(&self, agent_id: AgentId) -> Result<Vec<String>, CommunicationError>;
    async fn shutdown(&self) -> Result<(), CommunicationError>;
}

pub struct CommunicationConfig {
    pub message_ttl: Duration,
    pub max_queue_size: usize,
    pub delivery_timeout: Duration,
    pub retry_attempts: u32,
    pub enable_encryption: bool,
    pub enable_compression: bool,
    pub max_message_size: usize,
    pub dead_letter_queue_size: usize,
}
```

### 5. Error Handler

```rust
#[async_trait]
pub trait ErrorHandler {
    async fn handle_error(&self, agent_id: AgentId, error: RuntimeError) -> Result<ErrorAction, ErrorHandlerError>;
    async fn register_strategy(&self, error_type: ErrorType, strategy: RecoveryStrategy) -> Result<(), ErrorHandlerError>;
    async fn get_error_stats(&self, agent_id: AgentId) -> Result<ErrorStatistics, ErrorHandlerError>;
    async fn get_system_error_stats(&self) -> SystemErrorStatistics;
    async fn set_error_thresholds(&self, agent_id: AgentId, thresholds: ErrorThresholds) -> Result<(), ErrorHandlerError>;
    async fn clear_error_history(&self, agent_id: AgentId) -> Result<(), ErrorHandlerError>;
    async fn shutdown(&self) -> Result<(), ErrorHandlerError>;
}

#[derive(Debug, Clone)]
pub enum ErrorAction {
    Retry { max_attempts: u32, backoff: Duration },
    Restart,
    Suspend,
    Terminate,
    Failover,
}

#[derive(Debug, Clone)]
pub enum RecoveryStrategy {
    Retry { max_attempts: u32, backoff: Duration },
    Restart { preserve_state: bool },
    Failover { backup_agent: Option<AgentId> },
    Terminate { cleanup: bool },
    Manual { reason: String },
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorType {
    ResourceExhaustion,
    NetworkError,
    SecurityViolation,
    PolicyViolation,
    SystemError,
    ValidationError,
}

pub struct ErrorHandlerConfig {
    pub max_error_history: usize,
    pub error_aggregation_window: Duration,
    pub escalation_threshold: u32,
    pub circuit_breaker_threshold: u32,
    pub circuit_breaker_timeout: Duration,
    pub enable_auto_recovery: bool,
    pub max_recovery_attempts: u32,
    pub recovery_backoff_multiplier: f32,
}
```

## External Integrations

### 1. Policy Engine

```rust
#[async_trait]
pub trait PolicyEngine {
    async fn validate_agent_config(&self, config: &AgentConfig) -> Result<PolicyValidationResult, PolicyError>;
    async fn check_operation_allowed(&self, agent_id: AgentId, operation: &str, context: &PolicyContext) -> Result<bool, PolicyError>;
    async fn get_agent_policies(&self, agent_id: AgentId) -> Result<Vec<Policy>, PolicyError>;
    async fn update_policy(&self, policy: Policy) -> Result<(), PolicyError>;
    async fn delete_policy(&self, policy_id: String) -> Result<(), PolicyError>;
    async fn evaluate_policy(&self, policy_id: String, context: &PolicyContext) -> Result<PolicyDecision, PolicyError>;
}

pub struct Policy {
    pub id: String,
    pub name: String,
    pub description: String,
    pub rules: Vec<PolicyRule>,
    pub priority: u32,
    pub enabled: bool,
}

pub struct PolicyContext {
    pub agent_id: AgentId,
    pub operation: String,
    pub resource_requirements: Option<ResourceLimits>,
    pub security_context: SecurityContext,
    pub metadata: HashMap<String, String>,
}
```

### 2. Sandbox Orchestrator

```rust
#[async_trait]
pub trait SandboxOrchestrator {
    async fn create_sandbox(&self, config: SandboxConfig) -> Result<SandboxId, SandboxError>;
    async fn start_sandbox(&self, sandbox_id: SandboxId) -> Result<(), SandboxError>;
    async fn stop_sandbox(&self, sandbox_id: SandboxId) -> Result<(), SandboxError>;
    async fn destroy_sandbox(&self, sandbox_id: SandboxId) -> Result<(), SandboxError>;
    async fn get_sandbox_status(&self, sandbox_id: SandboxId) -> Result<SandboxStatus, SandboxError>;
    async fn execute_command(&self, sandbox_id: SandboxId, command: &str, args: Vec<String>) -> Result<CommandResult, SandboxError>;
    async fn upload_file(&self, sandbox_id: SandboxId, local_path: &str, remote_path: &str) -> Result<(), SandboxError>;
    async fn download_file(&self, sandbox_id: SandboxId, remote_path: &str, local_path: &str) -> Result<(), SandboxError>;
}

pub struct SandboxConfig {
    pub agent_id: AgentId,
    pub security_tier: SecurityTier,
    pub resource_limits: ResourceLimits,
    pub network_config: NetworkConfig,
    pub filesystem_config: FilesystemConfig,
    pub environment_variables: HashMap<String, String>,
}
```

### 3. Audit Trail

```rust
#[async_trait]
pub trait AuditTrail {
    async fn record_event(&self, event: AuditEvent) -> Result<AuditId, AuditError>;
    async fn query_events(&self, query: AuditQuery) -> Result<Vec<AuditEvent>, AuditError>;
    async fn verify_integrity(&self, from_time: SystemTime, to_time: SystemTime) -> Result<IntegrityReport, AuditError>;
    async fn get_event(&self, audit_id: AuditId) -> Result<AuditEvent, AuditError>;
    async fn export_events(&self, query: AuditQuery, format: ExportFormat) -> Result<Vec<u8>, AuditError>;
}

pub struct AuditEvent {
    pub id: AuditId,
    pub timestamp: SystemTime,
    pub event_type: AuditEventType,
    pub agent_id: Option<AgentId>,
    pub details: String,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditEventType {
    AgentCreated,
    AgentStarted,
    AgentStopped,
    AgentTerminated,
    ResourceAllocated,
    ResourceDeallocated,
    MessageSent,
    MessageReceived,
    ErrorOccurred,
    PolicyViolation,
    SecurityEvent,
}
```

## Usage Examples

### Complete Agent Lifecycle

```rust
use symbiont_runtime::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize components
    let lifecycle_controller = DefaultLifecycleController::new(LifecycleConfig::default()).await?;
    let resource_manager = DefaultResourceManager::new(ResourceManagerConfig::default()).await?;
    let scheduler = DefaultScheduler::new(SchedulerConfig::default()).await?;
    let comm_bus = DefaultCommunicationBus::new(CommunicationConfig::default()).await?;
    let error_handler = DefaultErrorHandler::new(ErrorHandlerConfig::default()).await?;

    // Create agent configuration
    let agent_config = AgentConfig {
        id: AgentId::new(),
        name: "example_agent".to_string(),
        dsl_source: "agent logic".to_string(),
        execution_mode: ExecutionMode::Persistent,
        security_tier: SecurityTier::Tier2,
        resource_limits: ResourceLimits {
            memory_mb: 512,
            cpu_cores: 1.0,
            disk_io_mbps: 50,
            network_io_mbps: 10,
            execution_timeout: Duration::from_secs(3600),
            idle_timeout: Duration::from_secs(300),
        },
        capabilities: vec![Capability::FileSystem, Capability::Network],
        policies: vec![],
        metadata: HashMap::new(),
        priority: Priority::Normal,
    };

    // Create and manage agent
    let agent = lifecycle_controller.create_agent(agent_config.clone()).await?;
    println!("Created agent: {}", agent.id);

    // Allocate resources
    let allocation = resource_manager.allocate_resources(agent.id, agent_config.resource_limits).await?;
    println!("Allocated resources for agent: {}", agent.id);

    // Register with communication bus
    comm_bus.register_agent(agent.id, agent_config.capabilities).await?;
    println!("Registered agent with communication bus");

    // Initialize and start agent
    lifecycle_controller.initialize_agent(agent.id).await?;
    lifecycle_controller.start_agent(agent.id).await?;
    println!("Agent started successfully");

    // Schedule a task
    let task = ScheduledTask {
        id: TaskId::new(),
        agent_id: agent.id,
        priority: Priority::Normal,
        scheduled_time: SystemTime::now(),
        timeout: Duration::from_secs(60),
        retry_count: 0,
    };
    scheduler.schedule_task(task).await?;

    // Send a message
    let message = Message {
        id: MessageId::new(),
        from: agent.id,
        to: agent.id, // Self-message for demo
        topic: "test_topic".to_string(),
        payload: b"Hello, world!".to_vec(),
        priority: Priority::Normal,
        ttl: Duration::from_secs(300),
    };
    comm_bus.send_message(message).await?;

    // Monitor and cleanup
    tokio::time::sleep(Duration::from_secs(5)).await;
    
    let state = lifecycle_controller.get_agent_state(agent.id).await?;
    println!("Agent state: {:?}", state);
    
    let usage = resource_manager.get_resource_usage(agent.id).await?;
    println!("Resource usage: {:?}", usage);

    // Shutdown
    lifecycle_controller.terminate_agent(agent.id).await?;
    resource_manager.deallocate_resources(agent.id).await?;
    comm_bus.unregister_agent(agent.id).await?;

    Ok(())
}
```

This API reference provides complete type definitions and interface specifications for all components of the Symbiont Agent Runtime System.