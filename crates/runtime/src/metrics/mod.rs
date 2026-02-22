//! Metrics collection and export for the Symbiont scheduler.
//!
//! Supports multiple export backends:
//! - **File**: JSON snapshots written atomically to disk (always available)
//! - **OTLP**: OpenTelemetry Protocol export via gRPC or HTTP (requires `metrics` feature)
//!
//! Multiple backends can run simultaneously via [`CompositeExporter`].

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;

pub mod file;

#[cfg(feature = "metrics")]
pub mod otlp;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur during metrics operations.
#[derive(Debug, Error)]
pub enum MetricsError {
    #[error("metrics export failed: {0}")]
    ExportFailed(String),

    #[error("metrics configuration error: {0}")]
    ConfigError(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("metrics shutdown failed: {0}")]
    ShutdownFailed(String),
}

// ---------------------------------------------------------------------------
// Configuration types
// ---------------------------------------------------------------------------

/// OTLP transport protocol.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OtlpProtocol {
    /// gRPC (default port 4317).
    #[default]
    Grpc,
    /// HTTP with protobuf encoding (default port 4318).
    HttpBinary,
    /// HTTP with JSON encoding (default port 4318).
    HttpJson,
}

/// OTLP exporter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OtlpConfig {
    /// OTLP endpoint URL (e.g. `http://localhost:4317` for gRPC).
    pub endpoint: String,

    /// Transport protocol.
    #[serde(default)]
    pub protocol: OtlpProtocol,

    /// Export timeout in seconds.
    #[serde(default = "default_otlp_timeout")]
    pub timeout_seconds: u64,

    /// Additional headers sent with each export request.
    /// Applied to HTTP transport; for gRPC use `OTEL_EXPORTER_OTLP_HEADERS` env var.
    #[serde(default)]
    pub headers: std::collections::HashMap<String, String>,
}

fn default_otlp_timeout() -> u64 {
    10
}

/// File-based metrics exporter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMetricsConfig {
    /// Path to the output JSON file.
    pub path: PathBuf,

    /// Pretty-print JSON output.
    #[serde(default = "default_pretty_print")]
    pub pretty_print: bool,
}

fn default_pretty_print() -> bool {
    true
}

impl Default for FileMetricsConfig {
    fn default() -> Self {
        Self {
            path: std::env::temp_dir().join("symbiont_scheduler_metrics.json"),
            pretty_print: true,
        }
    }
}

/// Top-level metrics configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsConfig {
    /// Enable metrics collection and export.
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Export interval in seconds.
    #[serde(default = "default_export_interval")]
    pub export_interval_seconds: u64,

    /// Service name reported to backends.
    #[serde(default = "default_service_name")]
    pub service_name: String,

    /// Service namespace reported to backends.
    #[serde(default = "default_service_namespace")]
    pub service_namespace: String,

    /// OTLP exporter configuration (requires `metrics` feature).
    pub otlp: Option<OtlpConfig>,

    /// File exporter configuration.
    pub file: Option<FileMetricsConfig>,
}

fn default_enabled() -> bool {
    true
}

fn default_export_interval() -> u64 {
    60
}

fn default_service_name() -> String {
    "symbiont-scheduler".to_string()
}

fn default_service_namespace() -> String {
    "symbiont".to_string()
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            export_interval_seconds: 60,
            service_name: default_service_name(),
            service_namespace: default_service_namespace(),
            otlp: None,
            file: Some(FileMetricsConfig::default()),
        }
    }
}

// ---------------------------------------------------------------------------
// Snapshot types
// ---------------------------------------------------------------------------

/// Point-in-time snapshot of all scheduler metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    /// Unix timestamp (seconds) when snapshot was taken.
    pub timestamp: u64,
    /// Scheduler-level metrics.
    pub scheduler: SchedulerMetrics,
    /// Task manager metrics.
    pub task_manager: TaskManagerMetrics,
    /// Load balancer metrics.
    pub load_balancer: LoadBalancerMetrics,
    /// System resource metrics.
    pub system: SystemResourceMetrics,
    /// Context compaction metrics.
    pub compaction: Option<CompactionMetrics>,
}

/// Scheduler-level counters and gauges.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerMetrics {
    pub total_scheduled: usize,
    pub uptime_seconds: u64,
    pub running_agents: usize,
    pub queued_agents: usize,
    pub suspended_agents: usize,
    pub max_capacity: usize,
    pub load_factor: f64,
}

/// Task manager statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskManagerMetrics {
    pub total_tasks: usize,
    pub healthy_tasks: usize,
    pub average_uptime_seconds: f64,
    pub total_memory_usage: usize,
}

/// Load balancer statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadBalancerMetrics {
    pub total_allocations: usize,
    pub active_allocations: usize,
    pub memory_utilization: f64,
    pub cpu_utilization: f64,
    pub allocation_failures: usize,
    pub average_allocation_time_ms: f64,
}

/// System resource usage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemResourceMetrics {
    pub memory_usage_mb: f64,
    pub cpu_usage_percent: f64,
}

/// Compaction pipeline metrics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompactionMetrics {
    /// Total number of compaction runs.
    pub total_compactions: u64,
    /// Cumulative tokens reclaimed.
    pub total_tokens_saved: u64,
    /// Compactions by tier: (tier_name, count).
    pub compactions_by_tier: HashMap<String, u64>,
    /// Current context utilization ratio (0.0–1.0), if known.
    pub context_utilization_ratio: Option<f64>,
}

// ---------------------------------------------------------------------------
// Exporter trait
// ---------------------------------------------------------------------------

/// Trait for metrics export backends.
#[async_trait]
pub trait MetricsExporter: Send + Sync {
    /// Export a metrics snapshot to the backend.
    async fn export(&self, snapshot: &MetricsSnapshot) -> Result<(), MetricsError>;

    /// Flush pending data and release resources.
    async fn shutdown(&self) -> Result<(), MetricsError>;
}

// ---------------------------------------------------------------------------
// Composite exporter
// ---------------------------------------------------------------------------

/// Combines multiple exporters into a single exporter.
///
/// All backends are called on every export; individual failures are logged
/// but do not prevent other backends from running.
pub struct CompositeExporter {
    exporters: Vec<Arc<dyn MetricsExporter>>,
}

impl CompositeExporter {
    pub fn new(exporters: Vec<Arc<dyn MetricsExporter>>) -> Self {
        Self { exporters }
    }
}

#[async_trait]
impl MetricsExporter for CompositeExporter {
    async fn export(&self, snapshot: &MetricsSnapshot) -> Result<(), MetricsError> {
        let mut last_error: Option<MetricsError> = None;
        for exporter in &self.exporters {
            if let Err(e) = exporter.export(snapshot).await {
                tracing::warn!("Metrics exporter failed: {}", e);
                last_error = Some(e);
            }
        }
        // Propagate error only when a single exporter is configured and it failed.
        if self.exporters.len() == 1 {
            if let Some(e) = last_error {
                return Err(e);
            }
        }
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), MetricsError> {
        let mut last_error: Option<MetricsError> = None;
        for exporter in &self.exporters {
            if let Err(e) = exporter.shutdown().await {
                tracing::warn!("Metrics exporter shutdown failed: {}", e);
                last_error = Some(e);
            }
        }
        if self.exporters.len() == 1 {
            if let Some(e) = last_error {
                return Err(e);
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Build an exporter (or composite) from configuration.
pub fn create_exporter(config: &MetricsConfig) -> Result<Arc<dyn MetricsExporter>, MetricsError> {
    let mut exporters: Vec<Arc<dyn MetricsExporter>> = Vec::new();

    // File exporter (always available).
    if let Some(ref file_cfg) = config.file {
        let file_exporter = file::FileExporter::new(file_cfg.clone())?;
        exporters.push(Arc::new(file_exporter));
    }

    // OTLP exporter (requires `metrics` feature).
    #[cfg(feature = "metrics")]
    if let Some(ref otlp_cfg) = config.otlp {
        let export_interval = std::time::Duration::from_secs(config.export_interval_seconds);
        let otlp_exporter = otlp::OtlpExporter::new(
            otlp_cfg.clone(),
            &config.service_name,
            &config.service_namespace,
            export_interval,
        )?;
        exporters.push(Arc::new(otlp_exporter));
    }

    #[cfg(not(feature = "metrics"))]
    if config.otlp.is_some() {
        tracing::warn!(
            "OTLP metrics configuration provided but the `metrics` feature is not enabled; \
             OTLP exporter will not be created"
        );
    }

    if exporters.is_empty() {
        return Err(MetricsError::ConfigError(
            "No metrics exporters configured (enable at least `file` or `otlp`)".to_string(),
        ));
    }

    if exporters.len() == 1 {
        Ok(exporters.remove(0))
    } else {
        Ok(Arc::new(CompositeExporter::new(exporters)))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_metrics_config() {
        let cfg = MetricsConfig::default();
        assert!(cfg.enabled);
        assert_eq!(cfg.export_interval_seconds, 60);
        assert_eq!(cfg.service_name, "symbiont-scheduler");
        assert!(cfg.file.is_some());
        assert!(cfg.otlp.is_none());
    }

    #[test]
    fn test_metrics_snapshot_serialization_roundtrip() {
        let snapshot = MetricsSnapshot {
            timestamp: 1700000000,
            scheduler: SchedulerMetrics {
                total_scheduled: 10,
                uptime_seconds: 3600,
                running_agents: 5,
                queued_agents: 3,
                suspended_agents: 2,
                max_capacity: 1000,
                load_factor: 0.005,
            },
            task_manager: TaskManagerMetrics {
                total_tasks: 5,
                healthy_tasks: 4,
                average_uptime_seconds: 1800.0,
                total_memory_usage: 1024,
            },
            load_balancer: LoadBalancerMetrics {
                total_allocations: 100,
                active_allocations: 5,
                memory_utilization: 0.45,
                cpu_utilization: 0.30,
                allocation_failures: 2,
                average_allocation_time_ms: 1.5,
            },
            system: SystemResourceMetrics {
                memory_usage_mb: 512.0,
                cpu_usage_percent: 30.0,
            },
            compaction: None,
        };

        let json = serde_json::to_string(&snapshot).unwrap();
        let deser: MetricsSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.timestamp, 1700000000);
        assert_eq!(deser.scheduler.running_agents, 5);
        assert_eq!(deser.task_manager.healthy_tasks, 4);
        assert_eq!(deser.load_balancer.allocation_failures, 2);
    }

    #[test]
    fn test_create_exporter_no_backends() {
        let cfg = MetricsConfig {
            enabled: true,
            export_interval_seconds: 60,
            service_name: "test".to_string(),
            service_namespace: "test".to_string(),
            otlp: None,
            file: None,
        };
        assert!(create_exporter(&cfg).is_err());
    }

    #[test]
    fn test_create_exporter_file_only() {
        let cfg = MetricsConfig {
            enabled: true,
            export_interval_seconds: 60,
            service_name: "test".to_string(),
            service_namespace: "test".to_string(),
            otlp: None,
            file: Some(FileMetricsConfig {
                path: std::env::temp_dir().join("test_metrics_create.json"),
                pretty_print: true,
            }),
        };
        assert!(create_exporter(&cfg).is_ok());
    }

    #[test]
    fn test_otlp_protocol_default() {
        let proto = OtlpProtocol::default();
        assert!(matches!(proto, OtlpProtocol::Grpc));
    }

    #[tokio::test]
    async fn test_composite_exporter_lifecycle() {
        let file_cfg = FileMetricsConfig {
            path: std::env::temp_dir().join("test_composite_lifecycle.json"),
            pretty_print: false,
        };
        let file_exp =
            Arc::new(file::FileExporter::new(file_cfg).unwrap()) as Arc<dyn MetricsExporter>;
        let composite = CompositeExporter::new(vec![file_exp]);

        let snapshot = MetricsSnapshot {
            timestamp: 1,
            scheduler: SchedulerMetrics {
                total_scheduled: 0,
                uptime_seconds: 0,
                running_agents: 0,
                queued_agents: 0,
                suspended_agents: 0,
                max_capacity: 100,
                load_factor: 0.0,
            },
            task_manager: TaskManagerMetrics {
                total_tasks: 0,
                healthy_tasks: 0,
                average_uptime_seconds: 0.0,
                total_memory_usage: 0,
            },
            load_balancer: LoadBalancerMetrics {
                total_allocations: 0,
                active_allocations: 0,
                memory_utilization: 0.0,
                cpu_utilization: 0.0,
                allocation_failures: 0,
                average_allocation_time_ms: 0.0,
            },
            system: SystemResourceMetrics {
                memory_usage_mb: 0.0,
                cpu_usage_percent: 0.0,
            },
            compaction: None,
        };

        assert!(composite.export(&snapshot).await.is_ok());
        assert!(composite.shutdown().await.is_ok());
    }

    #[test]
    fn compaction_metrics_default() {
        let m = CompactionMetrics::default();
        assert_eq!(m.total_compactions, 0);
        assert_eq!(m.total_tokens_saved, 0);
        assert!(m.context_utilization_ratio.is_none());
    }
}
