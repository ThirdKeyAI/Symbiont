//! OpenTelemetry OTLP metrics exporter.
//!
//! Records scheduler metrics as OpenTelemetry gauge instruments and exports
//! them via gRPC or HTTP to any OTLP-compatible collector (e.g. Prometheus,
//! Grafana Alloy, Datadog, New Relic).

use super::{MetricsError, MetricsExporter, MetricsSnapshot, OtlpConfig, OtlpProtocol};
use async_trait::async_trait;
use opentelemetry::metrics::{Gauge, MeterProvider};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use std::time::Duration;

/// Holds all OpenTelemetry gauge instruments.
struct Instruments {
    // Scheduler
    running_agents: Gauge<f64>,
    queued_agents: Gauge<f64>,
    suspended_agents: Gauge<f64>,
    total_scheduled: Gauge<f64>,
    max_capacity: Gauge<f64>,
    load_factor: Gauge<f64>,
    uptime_seconds: Gauge<f64>,
    // Task manager
    tm_total_tasks: Gauge<f64>,
    tm_healthy_tasks: Gauge<f64>,
    tm_average_uptime: Gauge<f64>,
    tm_total_memory: Gauge<f64>,
    // Load balancer
    lb_total_allocations: Gauge<f64>,
    lb_active_allocations: Gauge<f64>,
    lb_memory_utilization: Gauge<f64>,
    lb_cpu_utilization: Gauge<f64>,
    lb_allocation_failures: Gauge<f64>,
    lb_avg_allocation_time: Gauge<f64>,
    // System
    system_memory_mb: Gauge<f64>,
    system_cpu_percent: Gauge<f64>,
}

/// Exports metrics via OpenTelemetry OTLP protocol.
pub struct OtlpExporter {
    provider: SdkMeterProvider,
    instruments: Instruments,
}

impl OtlpExporter {
    /// Create a new OTLP exporter from configuration.
    pub fn new(
        config: OtlpConfig,
        service_name: &str,
        service_namespace: &str,
        export_interval: Duration,
    ) -> Result<Self, MetricsError> {
        use opentelemetry::KeyValue;
        use opentelemetry_otlp::MetricExporter;
        use opentelemetry_sdk::metrics::PeriodicReader;
        use opentelemetry_sdk::Resource;

        let timeout = Duration::from_secs(config.timeout_seconds);

        let metric_exporter = match config.protocol {
            OtlpProtocol::Grpc => MetricExporter::builder()
                .with_tonic()
                .with_endpoint(&config.endpoint)
                .with_timeout(timeout)
                .build()
                .map_err(|e| {
                    MetricsError::ConfigError(format!("Failed to build gRPC OTLP exporter: {}", e))
                })?,
            OtlpProtocol::HttpBinary | OtlpProtocol::HttpJson => MetricExporter::builder()
                .with_http()
                .with_endpoint(&config.endpoint)
                .with_timeout(timeout)
                .build()
                .map_err(|e| {
                    MetricsError::ConfigError(format!("Failed to build HTTP OTLP exporter: {}", e))
                })?,
        };

        let reader = PeriodicReader::builder(metric_exporter)
            .with_interval(export_interval)
            .build();

        let resource = Resource::builder()
            .with_service_name(service_name.to_string())
            .with_attribute(KeyValue::new(
                "service.namespace",
                service_namespace.to_string(),
            ))
            .build();

        let provider = SdkMeterProvider::builder()
            .with_reader(reader)
            .with_resource(resource)
            .build();

        let meter = provider.meter("symbiont.scheduler");

        let instruments = Instruments {
            running_agents: meter
                .f64_gauge("scheduler.running_agents")
                .with_description("Number of currently running agents")
                .build(),
            queued_agents: meter
                .f64_gauge("scheduler.queued_agents")
                .with_description("Number of agents waiting in queue")
                .build(),
            suspended_agents: meter
                .f64_gauge("scheduler.suspended_agents")
                .with_description("Number of suspended agents")
                .build(),
            total_scheduled: meter
                .f64_gauge("scheduler.total_scheduled")
                .with_description("Total number of scheduled agents")
                .build(),
            max_capacity: meter
                .f64_gauge("scheduler.max_capacity")
                .with_description("Maximum concurrent agent capacity")
                .build(),
            load_factor: meter
                .f64_gauge("scheduler.load_factor")
                .with_description("Current load factor (0.0-1.0)")
                .build(),
            uptime_seconds: meter
                .f64_gauge("scheduler.uptime_seconds")
                .with_description("Scheduler uptime in seconds")
                .build(),
            tm_total_tasks: meter
                .f64_gauge("task_manager.total_tasks")
                .with_description("Total tasks tracked by task manager")
                .build(),
            tm_healthy_tasks: meter
                .f64_gauge("task_manager.healthy_tasks")
                .with_description("Number of healthy tasks")
                .build(),
            tm_average_uptime: meter
                .f64_gauge("task_manager.average_uptime_seconds")
                .with_description("Average task uptime in seconds")
                .build(),
            tm_total_memory: meter
                .f64_gauge("task_manager.total_memory_usage")
                .with_description("Total memory usage across all tasks")
                .build(),
            lb_total_allocations: meter
                .f64_gauge("load_balancer.total_allocations")
                .with_description("Total resource allocations made")
                .build(),
            lb_active_allocations: meter
                .f64_gauge("load_balancer.active_allocations")
                .with_description("Currently active resource allocations")
                .build(),
            lb_memory_utilization: meter
                .f64_gauge("load_balancer.memory_utilization")
                .with_description("Memory utilization ratio (0.0-1.0)")
                .build(),
            lb_cpu_utilization: meter
                .f64_gauge("load_balancer.cpu_utilization")
                .with_description("CPU utilization ratio (0.0-1.0)")
                .build(),
            lb_allocation_failures: meter
                .f64_gauge("load_balancer.allocation_failures")
                .with_description("Total resource allocation failures")
                .build(),
            lb_avg_allocation_time: meter
                .f64_gauge("load_balancer.average_allocation_time_ms")
                .with_description("Average resource allocation time in milliseconds")
                .build(),
            system_memory_mb: meter
                .f64_gauge("system.memory_usage_mb")
                .with_description("System memory usage in megabytes")
                .build(),
            system_cpu_percent: meter
                .f64_gauge("system.cpu_usage_percent")
                .with_description("System CPU usage percentage")
                .build(),
        };

        tracing::info!(
            "OTLP metrics exporter initialized: endpoint={}, protocol={:?}",
            config.endpoint,
            config.protocol
        );

        Ok(Self {
            provider,
            instruments,
        })
    }
}

#[async_trait]
impl MetricsExporter for OtlpExporter {
    async fn export(&self, snapshot: &MetricsSnapshot) -> Result<(), MetricsError> {
        let i = &self.instruments;

        // Scheduler
        i.running_agents
            .record(snapshot.scheduler.running_agents as f64, &[]);
        i.queued_agents
            .record(snapshot.scheduler.queued_agents as f64, &[]);
        i.suspended_agents
            .record(snapshot.scheduler.suspended_agents as f64, &[]);
        i.total_scheduled
            .record(snapshot.scheduler.total_scheduled as f64, &[]);
        i.max_capacity
            .record(snapshot.scheduler.max_capacity as f64, &[]);
        i.load_factor.record(snapshot.scheduler.load_factor, &[]);
        i.uptime_seconds
            .record(snapshot.scheduler.uptime_seconds as f64, &[]);

        // Task manager
        i.tm_total_tasks
            .record(snapshot.task_manager.total_tasks as f64, &[]);
        i.tm_healthy_tasks
            .record(snapshot.task_manager.healthy_tasks as f64, &[]);
        i.tm_average_uptime
            .record(snapshot.task_manager.average_uptime_seconds, &[]);
        i.tm_total_memory
            .record(snapshot.task_manager.total_memory_usage as f64, &[]);

        // Load balancer
        i.lb_total_allocations
            .record(snapshot.load_balancer.total_allocations as f64, &[]);
        i.lb_active_allocations
            .record(snapshot.load_balancer.active_allocations as f64, &[]);
        i.lb_memory_utilization
            .record(snapshot.load_balancer.memory_utilization, &[]);
        i.lb_cpu_utilization
            .record(snapshot.load_balancer.cpu_utilization, &[]);
        i.lb_allocation_failures
            .record(snapshot.load_balancer.allocation_failures as f64, &[]);
        i.lb_avg_allocation_time
            .record(snapshot.load_balancer.average_allocation_time_ms, &[]);

        // System
        i.system_memory_mb
            .record(snapshot.system.memory_usage_mb, &[]);
        i.system_cpu_percent
            .record(snapshot.system.cpu_usage_percent, &[]);

        tracing::trace!("Recorded metrics snapshot to OTLP gauges");
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), MetricsError> {
        self.provider.shutdown().map_err(|e| {
            MetricsError::ShutdownFailed(format!("OTLP meter provider shutdown failed: {}", e))
        })
    }
}
