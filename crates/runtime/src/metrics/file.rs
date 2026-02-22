//! File-based metrics exporter.
//!
//! Writes JSON snapshots atomically using `tempfile` + rename to prevent
//! partial reads by monitoring tools.

use super::{FileMetricsConfig, MetricsError, MetricsExporter, MetricsSnapshot};
use async_trait::async_trait;
use std::path::PathBuf;

/// Exports metrics snapshots as JSON files using atomic writes.
pub struct FileExporter {
    path: PathBuf,
    pretty_print: bool,
}

impl FileExporter {
    /// Create a new file exporter, ensuring the parent directory exists.
    pub fn new(config: FileMetricsConfig) -> Result<Self, MetricsError> {
        if let Some(parent) = config.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                MetricsError::ConfigError(format!(
                    "Failed to create metrics directory {}: {}",
                    parent.display(),
                    e
                ))
            })?;
        }
        Ok(Self {
            path: config.path,
            pretty_print: config.pretty_print,
        })
    }
}

#[async_trait]
impl MetricsExporter for FileExporter {
    async fn export(&self, snapshot: &MetricsSnapshot) -> Result<(), MetricsError> {
        let json = if self.pretty_print {
            serde_json::to_string_pretty(snapshot)?
        } else {
            serde_json::to_string(snapshot)?
        };

        let path = self.path.clone();

        // Perform the atomic write on a blocking thread to avoid blocking the runtime.
        tokio::task::spawn_blocking(move || -> Result<(), MetricsError> {
            use std::io::Write;

            let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
            let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
            tmp.write_all(json.as_bytes())?;
            tmp.flush()?;
            tmp.persist(&path).map_err(|e| {
                MetricsError::ExportFailed(format!(
                    "Failed to persist metrics file {}: {}",
                    path.display(),
                    e
                ))
            })?;
            Ok(())
        })
        .await
        .map_err(|e| MetricsError::ExportFailed(format!("Blocking task panicked: {}", e)))??;

        tracing::debug!("Metrics snapshot written to {}", self.path.display());
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), MetricsError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::{
        LoadBalancerMetrics, SchedulerMetrics, SystemResourceMetrics, TaskManagerMetrics,
    };

    fn sample_snapshot() -> MetricsSnapshot {
        MetricsSnapshot {
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
        }
    }

    #[tokio::test]
    async fn test_file_exporter_write_and_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("metrics.json");

        let exporter = FileExporter::new(FileMetricsConfig {
            path: path.clone(),
            pretty_print: true,
        })
        .unwrap();

        let snapshot = sample_snapshot();
        exporter.export(&snapshot).await.unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let loaded: MetricsSnapshot = serde_json::from_str(&content).unwrap();
        assert_eq!(loaded.timestamp, 1700000000);
        assert_eq!(loaded.scheduler.running_agents, 5);
    }

    #[tokio::test]
    async fn test_file_exporter_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("deep").join("metrics.json");

        let exporter = FileExporter::new(FileMetricsConfig {
            path: path.clone(),
            pretty_print: false,
        })
        .unwrap();

        let snapshot = sample_snapshot();
        exporter.export(&snapshot).await.unwrap();
        assert!(path.exists());
    }

    #[tokio::test]
    async fn test_file_exporter_compact_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("compact.json");

        let exporter = FileExporter::new(FileMetricsConfig {
            path: path.clone(),
            pretty_print: false,
        })
        .unwrap();

        let snapshot = sample_snapshot();
        exporter.export(&snapshot).await.unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        // Compact JSON contains no newlines.
        assert!(!content.trim().contains('\n'));
    }

    #[tokio::test]
    async fn test_file_exporter_shutdown() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shutdown.json");

        let exporter = FileExporter::new(FileMetricsConfig {
            path,
            pretty_print: true,
        })
        .unwrap();

        assert!(exporter.shutdown().await.is_ok());
    }

    #[tokio::test]
    async fn test_file_exporter_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("overwrite.json");

        let exporter = FileExporter::new(FileMetricsConfig {
            path: path.clone(),
            pretty_print: false,
        })
        .unwrap();

        let mut snapshot = sample_snapshot();
        exporter.export(&snapshot).await.unwrap();

        // Overwrite with different data.
        snapshot.timestamp = 1700000001;
        snapshot.scheduler.running_agents = 42;
        exporter.export(&snapshot).await.unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let loaded: MetricsSnapshot = serde_json::from_str(&content).unwrap();
        assert_eq!(loaded.timestamp, 1700000001);
        assert_eq!(loaded.scheduler.running_agents, 42);
    }
}
