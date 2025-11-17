//! Sandbox Orchestrator Integration Interface
//!
//! Provides interface for integrating with multi-tier sandboxing systems

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use crate::types::*;
use crate::sandbox::{SandboxTier, SandboxRunner, ExecutionResult};
use std::sync::Arc;

// Import audit trail types - conditional compilation for runtime vs enterprise
#[cfg(feature = "enterprise")]
use crate::integrations::audit_trail::{AuditTrail, AuditEvent, AuditEventType, AuditSeverity, AuditCategory, AuditDetails, AuditContext, AuditOutcome};

/// Sandbox orchestrator trait for managing agent sandboxes
#[async_trait]
pub trait SandboxOrchestrator: Send + Sync {
    /// Create a new sandbox for an agent
    async fn create_sandbox(&self, request: SandboxRequest) -> Result<SandboxInfo, SandboxError>;

    /// Start a sandbox
    async fn start_sandbox(&self, sandbox_id: SandboxId) -> Result<(), SandboxError>;

    /// Stop a sandbox
    async fn stop_sandbox(&self, sandbox_id: SandboxId) -> Result<(), SandboxError>;

    /// Destroy a sandbox and cleanup resources
    async fn destroy_sandbox(&self, sandbox_id: SandboxId) -> Result<(), SandboxError>;

    /// Get sandbox status and information
    async fn get_sandbox_info(&self, sandbox_id: SandboxId) -> Result<SandboxInfo, SandboxError>;

    /// List all sandboxes
    async fn list_sandboxes(&self) -> Result<Vec<SandboxInfo>, SandboxError>;

    /// Execute a command in a sandbox
    async fn execute_command(
        &self,
        sandbox_id: SandboxId,
        command: SandboxCommand,
    ) -> Result<CommandResult, SandboxError>;

    /// Upload files to a sandbox
    async fn upload_files(
        &self,
        sandbox_id: SandboxId,
        files: Vec<FileUpload>,
    ) -> Result<(), SandboxError>;

    /// Download files from a sandbox
    async fn download_files(
        &self,
        sandbox_id: SandboxId,
        paths: Vec<String>,
    ) -> Result<Vec<FileDownload>, SandboxError>;

    /// Get sandbox resource usage
    async fn get_resource_usage(
        &self,
        sandbox_id: SandboxId,
    ) -> Result<SandboxResourceUsage, SandboxError>;

    /// Update sandbox configuration
    async fn update_sandbox(
        &self,
        sandbox_id: SandboxId,
        config: SandboxConfig,
    ) -> Result<(), SandboxError>;

    /// Get sandbox logs
    async fn get_logs(
        &self,
        sandbox_id: SandboxId,
        options: LogOptions,
    ) -> Result<Vec<LogEntry>, SandboxError>;

    /// Create a snapshot of a sandbox
    async fn create_snapshot(
        &self,
        sandbox_id: SandboxId,
        name: String,
    ) -> Result<SnapshotId, SandboxError>;

    /// Restore sandbox from snapshot
    async fn restore_snapshot(
        &self,
        sandbox_id: SandboxId,
        snapshot_id: SnapshotId,
    ) -> Result<(), SandboxError>;

    /// Delete a snapshot
    async fn delete_snapshot(&self, snapshot_id: SnapshotId) -> Result<(), SandboxError>;

    /// Execute code using a specific sandbox tier
    async fn execute_code(
        &self,
        tier: SandboxTier,
        code: &str,
        env: HashMap<String, String>,
    ) -> Result<ExecutionResult, SandboxError>;

    /// Register a sandbox runner for a specific tier
    async fn register_sandbox_runner(
        &self,
        tier: SandboxTier,
        runner: Arc<dyn SandboxRunner>,
    ) -> Result<(), SandboxError>;
}

/// Sandbox creation request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxRequest {
    pub agent_id: AgentId,
    pub sandbox_type: SandboxType,
    pub config: SandboxConfig,
    pub security_level: SecurityTier,
    pub resource_limits: ResourceLimits,
    pub network_config: NetworkConfig,
    pub storage_config: StorageConfig,
    pub metadata: HashMap<String, String>,
}

/// Sandbox types for different isolation levels
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SandboxType {
    /// Docker container sandbox
    Docker { image: String, tag: String },
    /// gVisor sandbox for enhanced security (requires enterprise feature)
    #[cfg(feature = "enterprise")]
    GVisor { runtime: String, platform: String },
    /// Firecracker microVM sandbox (requires enterprise feature)
    #[cfg(feature = "enterprise")]
    Firecracker {
        kernel_image: String,
        rootfs_image: String,
    },
    /// Process-level sandbox
    Process {
        executable: String,
        working_dir: PathBuf,
    },
    /// Custom sandbox implementation
    Custom {
        provider: String,
        config: HashMap<String, String>,
    },
}

/// Sandbox configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    pub name: String,
    pub description: String,
    pub environment_variables: HashMap<String, String>,
    pub working_directory: Option<PathBuf>,
    pub command: Option<Vec<String>>,
    pub entrypoint: Option<Vec<String>>,
    pub user: Option<String>,
    pub group: Option<String>,
    pub capabilities: Vec<String>,
    pub security_options: SecurityOptions,
    pub auto_remove: bool,
    pub restart_policy: RestartPolicy,
    pub health_check: Option<HealthCheck>,
}

/// Security options for sandbox
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityOptions {
    pub read_only_root: bool,
    pub no_new_privileges: bool,
    pub seccomp_profile: Option<String>,
    pub apparmor_profile: Option<String>,
    pub selinux_label: Option<String>,
    pub privileged: bool,
    pub drop_capabilities: Vec<String>,
    pub add_capabilities: Vec<String>,
}

/// Restart policy for sandbox
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RestartPolicy {
    Never,
    Always,
    OnFailure { max_retries: u32 },
    UnlessStopped,
}

/// Health check configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheck {
    pub command: Vec<String>,
    pub interval: Duration,
    pub timeout: Duration,
    pub retries: u32,
    pub start_period: Duration,
}

/// Network configuration for sandbox
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    pub mode: NetworkMode,
    pub ports: Vec<PortMapping>,
    pub dns_servers: Vec<String>,
    pub dns_search: Vec<String>,
    pub hostname: Option<String>,
    pub extra_hosts: HashMap<String, String>,
    pub network_aliases: Vec<String>,
}

/// Network modes for sandbox
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NetworkMode {
    Bridge,
    Host,
    None,
    Container { container_id: String },
    Custom { network_name: String },
}

/// Port mapping for network access
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortMapping {
    pub host_port: u16,
    pub container_port: u16,
    pub protocol: Protocol,
    pub host_ip: Option<String>,
}

/// Network protocols
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Protocol {
    TCP,
    UDP,
    SCTP,
}

/// Storage configuration for sandbox
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub volumes: Vec<VolumeMount>,
    pub tmpfs_mounts: Vec<TmpfsMount>,
    pub storage_driver: Option<String>,
    pub storage_options: HashMap<String, String>,
}

/// Volume mount configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeMount {
    pub source: String,
    pub target: String,
    pub mount_type: MountType,
    pub read_only: bool,
    pub options: Vec<String>,
}

/// Mount types for volumes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MountType {
    Bind,
    Volume,
    Tmpfs,
}

/// Tmpfs mount configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TmpfsMount {
    pub target: String,
    pub size: Option<u64>,
    pub mode: Option<u32>,
    pub options: Vec<String>,
}

/// Sandbox information and status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxInfo {
    pub id: SandboxId,
    pub agent_id: AgentId,
    pub sandbox_type: SandboxType,
    pub status: SandboxStatus,
    pub config: SandboxConfig,
    pub resource_usage: SandboxResourceUsage,
    pub network_info: NetworkInfo,
    pub created_at: SystemTime,
    pub started_at: Option<SystemTime>,
    pub stopped_at: Option<SystemTime>,
    pub metadata: HashMap<String, String>,
}

/// Sandbox status
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SandboxStatus {
    Creating,
    Created,
    Starting,
    Running,
    Stopping,
    Stopped,
    Paused,
    Error { message: String },
    Destroyed,
}

/// Network information for sandbox
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInfo {
    pub ip_address: Option<String>,
    pub mac_address: Option<String>,
    pub gateway: Option<String>,
    pub bridge: Option<String>,
    pub ports: Vec<PortMapping>,
}

/// Command to execute in sandbox
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxCommand {
    pub command: Vec<String>,
    pub working_dir: Option<PathBuf>,
    pub environment: HashMap<String, String>,
    pub user: Option<String>,
    pub timeout: Option<Duration>,
    pub stdin: Option<String>,
}

/// Command execution result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub execution_time: Duration,
    pub timed_out: bool,
}

/// File upload to sandbox
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileUpload {
    pub local_path: PathBuf,
    pub sandbox_path: String,
    pub permissions: Option<u32>,
    pub owner: Option<String>,
    pub group: Option<String>,
}

/// File download from sandbox
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDownload {
    pub sandbox_path: String,
    pub content: Vec<u8>,
    pub permissions: u32,
    pub size: u64,
    pub modified_at: SystemTime,
}

/// Sandbox resource usage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxResourceUsage {
    pub cpu_usage: CpuUsage,
    pub memory_usage: MemoryUsage,
    pub disk_usage: DiskUsage,
    pub network_usage: NetworkUsage,
    pub timestamp: SystemTime,
}

/// CPU usage statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuUsage {
    pub total_usage: Duration,
    pub user_usage: Duration,
    pub system_usage: Duration,
    pub cpu_percent: f64,
    pub throttled_time: Duration,
}

/// Memory usage statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryUsage {
    pub current: u64,
    pub peak: u64,
    pub limit: u64,
    pub cache: u64,
    pub swap: u64,
    pub percent: f64,
}

/// Disk usage statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskUsage {
    pub read_bytes: u64,
    pub write_bytes: u64,
    pub read_ops: u64,
    pub write_ops: u64,
    pub total_space: u64,
    pub used_space: u64,
}

/// Network usage statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkUsage {
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_packets: u64,
    pub tx_packets: u64,
    pub rx_errors: u64,
    pub tx_errors: u64,
}

/// Log options for retrieving sandbox logs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogOptions {
    pub since: Option<SystemTime>,
    pub until: Option<SystemTime>,
    pub tail: Option<u32>,
    pub follow: bool,
    pub timestamps: bool,
    pub details: bool,
}

/// Log entry from sandbox
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: SystemTime,
    pub level: LogLevel,
    pub source: LogSource,
    pub message: String,
    pub metadata: HashMap<String, String>,
}

/// Log levels
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warning,
    Error,
    Fatal,
}

/// Log sources
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LogSource {
    Stdout,
    Stderr,
    System,
    Application,
}

/// Sandbox identifier
pub type SandboxId = uuid::Uuid;

/// Snapshot identifier
pub type SnapshotId = uuid::Uuid;

/// Mock sandbox orchestrator for testing and development
pub struct MockSandboxOrchestrator {
    sandboxes: std::sync::RwLock<HashMap<SandboxId, SandboxInfo>>,
    snapshots: std::sync::RwLock<HashMap<SnapshotId, SandboxSnapshot>>,
    sandbox_runners: std::sync::RwLock<HashMap<SandboxTier, Arc<dyn SandboxRunner>>>,
}

/// Snapshot information
#[derive(Debug, Clone)]
struct SandboxSnapshot {
    id: SnapshotId,
    sandbox_id: SandboxId,
    name: String,
    created_at: SystemTime,
    size: u64,
}

impl SandboxSnapshot {
    fn new(id: SnapshotId, sandbox_id: SandboxId, name: String) -> Self {
        Self {
            id,
            sandbox_id,
            name,
            created_at: SystemTime::now(),
            size: 0,
        }
    }

    fn get_id(&self) -> SnapshotId {
        self.id
    }

    fn get_sandbox_id(&self) -> SandboxId {
        self.sandbox_id
    }

    fn get_name(&self) -> &str {
        &self.name
    }

    fn get_age(&self) -> Duration {
        SystemTime::now()
            .duration_since(self.created_at)
            .unwrap_or_default()
    }

    fn get_size(&self) -> u64 {
        self.size
    }

    fn set_size(&mut self, size: u64) {
        self.size = size;
    }

    fn is_expired(&self, max_age: Duration) -> bool {
        self.get_age() > max_age
    }
}

impl MockSandboxOrchestrator {
    pub fn new() -> Self {
        Self {
            sandboxes: std::sync::RwLock::new(HashMap::new()),
            snapshots: std::sync::RwLock::new(HashMap::new()),
            sandbox_runners: std::sync::RwLock::new(HashMap::new()),
        }
    }

    fn create_mock_resource_usage() -> SandboxResourceUsage {
        SandboxResourceUsage {
            cpu_usage: CpuUsage {
                total_usage: Duration::from_secs(10),
                user_usage: Duration::from_secs(8),
                system_usage: Duration::from_secs(2),
                cpu_percent: 5.0,
                throttled_time: Duration::from_millis(0),
            },
            memory_usage: MemoryUsage {
                current: 64 * 1024 * 1024, // 64MB
                peak: 128 * 1024 * 1024,   // 128MB
                limit: 512 * 1024 * 1024,  // 512MB
                cache: 16 * 1024 * 1024,   // 16MB
                swap: 0,
                percent: 12.5,
            },
            disk_usage: DiskUsage {
                read_bytes: 1024 * 1024, // 1MB
                write_bytes: 512 * 1024, // 512KB
                read_ops: 100,
                write_ops: 50,
                total_space: 10 * 1024 * 1024 * 1024, // 10GB
                used_space: 1024 * 1024 * 1024,       // 1GB
            },
            network_usage: NetworkUsage {
                rx_bytes: 2048,
                tx_bytes: 1024,
                rx_packets: 20,
                tx_packets: 15,
                rx_errors: 0,
                tx_errors: 0,
            },
            timestamp: SystemTime::now(),
        }
    }
}

impl Default for MockSandboxOrchestrator {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SandboxOrchestrator for MockSandboxOrchestrator {
    async fn create_sandbox(&self, request: SandboxRequest) -> Result<SandboxInfo, SandboxError> {
        let sandbox_id = SandboxId::new_v4();
        let now = SystemTime::now();

        let sandbox_info = SandboxInfo {
            id: sandbox_id,
            agent_id: request.agent_id,
            sandbox_type: request.sandbox_type,
            status: SandboxStatus::Created,
            config: request.config,
            resource_usage: Self::create_mock_resource_usage(),
            network_info: NetworkInfo {
                ip_address: Some("172.17.0.2".to_string()),
                mac_address: Some("02:42:ac:11:00:02".to_string()),
                gateway: Some("172.17.0.1".to_string()),
                bridge: Some("docker0".to_string()),
                ports: request.network_config.ports,
            },
            created_at: now,
            started_at: None,
            stopped_at: None,
            metadata: request.metadata,
        };

        self.sandboxes
            .write()
            .unwrap()
            .insert(sandbox_id, sandbox_info.clone());
        Ok(sandbox_info)
    }

    async fn start_sandbox(&self, sandbox_id: SandboxId) -> Result<(), SandboxError> {
        let mut sandboxes = self.sandboxes.write().unwrap();
        if let Some(sandbox) = sandboxes.get_mut(&sandbox_id) {
            sandbox.status = SandboxStatus::Running;
            sandbox.started_at = Some(SystemTime::now());
            Ok(())
        } else {
            Err(SandboxError::SandboxNotFound {
                id: sandbox_id.to_string(),
            })
        }
    }

    async fn stop_sandbox(&self, sandbox_id: SandboxId) -> Result<(), SandboxError> {
        let mut sandboxes = self.sandboxes.write().unwrap();
        if let Some(sandbox) = sandboxes.get_mut(&sandbox_id) {
            sandbox.status = SandboxStatus::Stopped;
            sandbox.stopped_at = Some(SystemTime::now());
            Ok(())
        } else {
            Err(SandboxError::SandboxNotFound {
                id: sandbox_id.to_string(),
            })
        }
    }

    async fn destroy_sandbox(&self, sandbox_id: SandboxId) -> Result<(), SandboxError> {
        let mut sandboxes = self.sandboxes.write().unwrap();
        if let Some(sandbox) = sandboxes.get_mut(&sandbox_id) {
            sandbox.status = SandboxStatus::Destroyed;
            Ok(())
        } else {
            Err(SandboxError::SandboxNotFound {
                id: sandbox_id.to_string(),
            })
        }
    }

    async fn get_sandbox_info(&self, sandbox_id: SandboxId) -> Result<SandboxInfo, SandboxError> {
        let sandboxes = self.sandboxes.read().unwrap();
        sandboxes
            .get(&sandbox_id)
            .cloned()
            .ok_or(SandboxError::SandboxNotFound {
                id: sandbox_id.to_string(),
            })
    }

    async fn list_sandboxes(&self) -> Result<Vec<SandboxInfo>, SandboxError> {
        let sandboxes = self.sandboxes.read().unwrap();
        Ok(sandboxes.values().cloned().collect())
    }

    async fn execute_command(
        &self,
        sandbox_id: SandboxId,
        command: SandboxCommand,
    ) -> Result<CommandResult, SandboxError> {
        let sandboxes = self.sandboxes.read().unwrap();
        if sandboxes.contains_key(&sandbox_id) {
            // Mock command execution
            Ok(CommandResult {
                exit_code: 0,
                stdout: format!("Mock output for command: {:?}", command.command),
                stderr: String::new(),
                execution_time: Duration::from_millis(100),
                timed_out: false,
            })
        } else {
            Err(SandboxError::SandboxNotFound {
                id: sandbox_id.to_string(),
            })
        }
    }

    async fn upload_files(
        &self,
        sandbox_id: SandboxId,
        _files: Vec<FileUpload>,
    ) -> Result<(), SandboxError> {
        let sandboxes = self.sandboxes.read().unwrap();
        if sandboxes.contains_key(&sandbox_id) {
            Ok(())
        } else {
            Err(SandboxError::SandboxNotFound {
                id: sandbox_id.to_string(),
            })
        }
    }

    async fn download_files(
        &self,
        sandbox_id: SandboxId,
        paths: Vec<String>,
    ) -> Result<Vec<FileDownload>, SandboxError> {
        let sandboxes = self.sandboxes.read().unwrap();
        if sandboxes.contains_key(&sandbox_id) {
            let downloads = paths
                .into_iter()
                .map(|path| FileDownload {
                    sandbox_path: path,
                    content: b"mock file content".to_vec(),
                    permissions: 0o644,
                    size: 18,
                    modified_at: SystemTime::now(),
                })
                .collect();
            Ok(downloads)
        } else {
            Err(SandboxError::SandboxNotFound {
                id: sandbox_id.to_string(),
            })
        }
    }

    async fn get_resource_usage(
        &self,
        sandbox_id: SandboxId,
    ) -> Result<SandboxResourceUsage, SandboxError> {
        let sandboxes = self.sandboxes.read().unwrap();
        if sandboxes.contains_key(&sandbox_id) {
            Ok(Self::create_mock_resource_usage())
        } else {
            Err(SandboxError::SandboxNotFound {
                id: sandbox_id.to_string(),
            })
        }
    }

    async fn update_sandbox(
        &self,
        sandbox_id: SandboxId,
        config: SandboxConfig,
    ) -> Result<(), SandboxError> {
        let mut sandboxes = self.sandboxes.write().unwrap();
        if let Some(sandbox) = sandboxes.get_mut(&sandbox_id) {
            sandbox.config = config;
            Ok(())
        } else {
            Err(SandboxError::SandboxNotFound {
                id: sandbox_id.to_string(),
            })
        }
    }

    async fn get_logs(
        &self,
        sandbox_id: SandboxId,
        _options: LogOptions,
    ) -> Result<Vec<LogEntry>, SandboxError> {
        let sandboxes = self.sandboxes.read().unwrap();
        if sandboxes.contains_key(&sandbox_id) {
            Ok(vec![LogEntry {
                timestamp: SystemTime::now(),
                level: LogLevel::Info,
                source: LogSource::Stdout,
                message: "Mock log entry".to_string(),
                metadata: HashMap::new(),
            }])
        } else {
            Err(SandboxError::SandboxNotFound {
                id: sandbox_id.to_string(),
            })
        }
    }

    async fn create_snapshot(
        &self,
        sandbox_id: SandboxId,
        name: String,
    ) -> Result<SnapshotId, SandboxError> {
        let sandboxes = self.sandboxes.read().unwrap();
        if sandboxes.contains_key(&sandbox_id) {
            let snapshot_id = SnapshotId::new_v4();
            let mut snapshot = SandboxSnapshot::new(snapshot_id, sandbox_id, name);
            snapshot.set_size(1024 * 1024 * 100); // 100MB

            tracing::info!(
                "Created snapshot {} for sandbox {} with size {} bytes",
                snapshot.get_id(),
                snapshot.get_sandbox_id(),
                snapshot.get_size()
            );

            self.snapshots
                .write()
                .unwrap()
                .insert(snapshot_id, snapshot);
            Ok(snapshot_id)
        } else {
            Err(SandboxError::SandboxNotFound {
                id: sandbox_id.to_string(),
            })
        }
    }

    async fn restore_snapshot(
        &self,
        sandbox_id: SandboxId,
        snapshot_id: SnapshotId,
    ) -> Result<(), SandboxError> {
        let sandboxes = self.sandboxes.read().unwrap();
        let snapshots = self.snapshots.read().unwrap();

        if !sandboxes.contains_key(&sandbox_id) {
            return Err(SandboxError::SandboxNotFound {
                id: sandbox_id.to_string(),
            });
        }

        if !snapshots.contains_key(&snapshot_id) {
            return Err(SandboxError::SnapshotNotFound {
                id: snapshot_id.to_string(),
            });
        }

        Ok(())
    }

    async fn delete_snapshot(&self, snapshot_id: SnapshotId) -> Result<(), SandboxError> {
        let mut snapshots = self.snapshots.write().unwrap();
        if let Some(snapshot) = snapshots.get(&snapshot_id) {
            tracing::info!(
                "Deleting snapshot '{}' (age: {:?}s, size: {} bytes)",
                snapshot.get_name(),
                snapshot.get_age().as_secs(),
                snapshot.get_size()
            );
        }

        if snapshots.remove(&snapshot_id).is_some() {
            Ok(())
        } else {
            Err(SandboxError::SnapshotNotFound {
                id: snapshot_id.to_string(),
            })
        }
    }

    async fn execute_code(
        &self,
        tier: SandboxTier,
        code: &str,
        env: HashMap<String, String>,
    ) -> Result<ExecutionResult, SandboxError> {
        let runner = {
            let runners = self.sandbox_runners.read().unwrap();
            runners.get(&tier).cloned()
        };
        
        if let Some(runner) = runner {
            runner
                .execute(code, env)
                .await
                .map_err(|e| SandboxError::ExecutionFailed(format!("Code execution failed: {}", e)))
        } else {
            Err(SandboxError::UnsupportedTier(format!("{:?}", tier)))
        }
    }

    async fn register_sandbox_runner(
        &self,
        tier: SandboxTier,
        runner: Arc<dyn SandboxRunner>,
    ) -> Result<(), SandboxError> {
        let mut runners = self.sandbox_runners.write().unwrap();
        runners.insert(tier, runner);
        Ok(())
    }
}

impl MockSandboxOrchestrator {
    /// Clean up expired snapshots
    pub async fn cleanup_expired_snapshots(&self, max_age: Duration) -> u32 {
        let mut snapshots = self.snapshots.write().unwrap();
        let mut expired_count = 0;
        let expired_ids: Vec<SnapshotId> = snapshots
            .iter()
            .filter_map(|(id, snapshot)| {
                if snapshot.is_expired(max_age) {
                    tracing::info!(
                        "Snapshot '{}' expired (age: {:?})",
                        snapshot.get_name(),
                        snapshot.get_age()
                    );
                    expired_count += 1;
                    Some(*id)
                } else {
                    None
                }
            })
            .collect();

        for id in expired_ids {
            snapshots.remove(&id);
        }

        expired_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_sandbox_lifecycle() {
        let orchestrator = MockSandboxOrchestrator::new();
        let agent_id = AgentId::new();

        // Create sandbox
        let request = SandboxRequest {
            agent_id,
            sandbox_type: SandboxType::Docker {
                image: "ubuntu".to_string(),
                tag: "latest".to_string(),
            },
            config: SandboxConfig {
                name: "test-sandbox".to_string(),
                description: "Test sandbox".to_string(),
                environment_variables: HashMap::new(),
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
                memory_mb: 512,
                cpu_cores: 2.0,
                disk_io_mbps: 100,
                network_io_mbps: 10,
                execution_timeout: std::time::Duration::from_secs(300),
                idle_timeout: std::time::Duration::from_secs(60),
            },
            network_config: NetworkConfig {
                mode: NetworkMode::Bridge,
                ports: vec![],
                dns_servers: vec![],
                dns_search: vec![],
                hostname: None,
                extra_hosts: HashMap::new(),
                network_aliases: vec![],
            },
            storage_config: StorageConfig {
                volumes: vec![],
                tmpfs_mounts: vec![],
                storage_driver: None,
                storage_options: HashMap::new(),
            },
            metadata: HashMap::new(),
        };

        let sandbox_info = orchestrator.create_sandbox(request).await.unwrap();
        assert_eq!(sandbox_info.status, SandboxStatus::Created);

        // Start sandbox
        orchestrator.start_sandbox(sandbox_info.id).await.unwrap();
        let updated_info = orchestrator
            .get_sandbox_info(sandbox_info.id)
            .await
            .unwrap();
        assert_eq!(updated_info.status, SandboxStatus::Running);

        // Stop sandbox
        orchestrator.stop_sandbox(sandbox_info.id).await.unwrap();
        let stopped_info = orchestrator
            .get_sandbox_info(sandbox_info.id)
            .await
            .unwrap();
        assert_eq!(stopped_info.status, SandboxStatus::Stopped);
    }

    #[tokio::test]
    async fn test_command_execution() {
        let orchestrator = MockSandboxOrchestrator::new();
        let agent_id = AgentId::new();

        let request = SandboxRequest {
            agent_id,
            sandbox_type: SandboxType::Docker {
                image: "ubuntu".to_string(),
                tag: "latest".to_string(),
            },
            config: SandboxConfig {
                name: "test-sandbox".to_string(),
                description: "Test sandbox".to_string(),
                environment_variables: HashMap::new(),
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
                memory_mb: 512,
                cpu_cores: 2.0,
                disk_io_mbps: 100,
                network_io_mbps: 10,
                execution_timeout: std::time::Duration::from_secs(300),
                idle_timeout: std::time::Duration::from_secs(60),
            },
            network_config: NetworkConfig {
                mode: NetworkMode::Bridge,
                ports: vec![],
                dns_servers: vec![],
                dns_search: vec![],
                hostname: None,
                extra_hosts: HashMap::new(),
                network_aliases: vec![],
            },
            storage_config: StorageConfig {
                volumes: vec![],
                tmpfs_mounts: vec![],
                storage_driver: None,
                storage_options: HashMap::new(),
            },
            metadata: HashMap::new(),
        };

        let sandbox_info = orchestrator.create_sandbox(request).await.unwrap();

        let command = SandboxCommand {
            command: vec!["echo".to_string(), "hello".to_string()],
            working_dir: None,
            environment: HashMap::new(),
            user: None,
            timeout: None,
            stdin: None,
        };

        let result = orchestrator
            .execute_command(sandbox_info.id, command)
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(!result.timed_out);
    }
}
