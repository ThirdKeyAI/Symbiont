//! Integration tests that assert the documented performance claims.
//!
//! These tests measure wall-clock time and fail if any claim is violated.
//! They are intentionally generous with thresholds to avoid flakiness in
//! CI while still catching order-of-magnitude regressions.
//!
//! Claims:
//!   1. Policy engine evaluates decisions in under 1 ms (10,000+ evals/sec).
//!   2. SchemaPin signature verification completes in under 5 ms per tool.
//!   3. Registration and bounded-queue operations remain responsive.
//!      These checks do not measure CPU overhead of executing agents.
//!
//! Run with:
//!   cargo test -p symbi-runtime --test performance_claims -- --nocapture

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime};

// ── Symbiont runtime types ────────────────────────────────────────────────────

use symbi_runtime::integrations::policy_engine::{
    AccessContext, AccessType, DefaultPolicyEnforcementPoint, PolicyEnforcementPoint,
    ResourceAccessConfig, ResourceAccessRequest, ResourceType, SourceInfo,
};
use symbi_runtime::scheduler::priority_queue::PriorityQueue;
use symbi_runtime::scheduler::{DefaultAgentScheduler, ScheduledTask, SchedulerConfig};
use symbi_runtime::types::agent::AgentMetadata;
use symbi_runtime::types::*;
use symbi_runtime::AgentScheduler;

// ── SchemaPin types ───────────────────────────────────────────────────────────

use schemapin::canonicalize::canonicalize_and_hash;
use schemapin::crypto::{generate_key_pair, sign_data, verify_signature};
use schemapin::discovery::build_well_known_response;
use schemapin::pinning::KeyPinStore;
use schemapin::verification::verify_schema_offline;

// ═══════════════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════════════

fn make_access_request(resource_id: &str) -> ResourceAccessRequest {
    ResourceAccessRequest {
        resource_type: ResourceType::File,
        resource_id: resource_id.to_string(),
        access_type: AccessType::Read,
        context: AccessContext {
            agent_metadata: AgentMetadata {
                version: "1.0.0".to_string(),
                author: "perf-test".to_string(),
                description: "Performance test agent".to_string(),
                capabilities: vec![],
                dependencies: vec![],
                resource_requirements: symbi_runtime::types::agent::ResourceRequirements::default(),
                security_requirements: symbi_runtime::types::agent::SecurityRequirements::default(),
                custom_fields: HashMap::new(),
            },
            security_level: SecurityTier::Tier1,
            access_history: Vec::new(),
            resource_usage: ResourceUsage::default(),
            environment: HashMap::new(),
            source_info: SourceInfo {
                ip_address: None,
                user_agent: None,
                session_id: None,
                request_id: "perf-test".to_string(),
            },
        },
        timestamp: SystemTime::now(),
    }
}

fn make_agent_config(name: &str) -> AgentConfig {
    AgentConfig {
        id: AgentId::new(),
        name: name.to_string(),
        dsl_source: String::new(),
        execution_mode: ExecutionMode::Ephemeral,
        security_tier: SecurityTier::Tier1,
        resource_limits: ResourceLimits::default(),
        capabilities: vec![Capability::Computation],
        policies: vec![],
        metadata: HashMap::new(),
        priority: Priority::Normal,
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Claim 1 — Policy engine: < 1 ms per evaluation, 10 000+ evals/sec
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn claim1_policy_evaluation_under_1ms() {
    let config = ResourceAccessConfig {
        default_deny: true,
        enable_caching: false, // measure raw evaluation, not cache hits
        cache_ttl_secs: 300,
        policy_path: None,
        enable_audit: false,
    };
    let ep = DefaultPolicyEnforcementPoint::new(config).await.unwrap();
    let agent_id = AgentId::new();

    // Warm up
    for _ in 0..100 {
        let req = make_access_request("/tmp/warmup.txt");
        ep.check_resource_access(agent_id, &req).await.unwrap();
    }

    // Measure 1,000 individual evaluations
    let iterations = 1_000u32;
    let mut max_us = 0u128;
    let mut total_us = 0u128;

    for i in 0..iterations {
        let req = make_access_request(&format!("/tmp/file_{}.txt", i % 50));
        let start = Instant::now();
        ep.check_resource_access(agent_id, &req).await.unwrap();
        let elapsed = start.elapsed().as_micros();
        total_us += elapsed;
        if elapsed > max_us {
            max_us = elapsed;
        }
    }

    let avg_us = total_us / iterations as u128;
    let avg_ms = avg_us as f64 / 1_000.0;
    let max_ms = max_us as f64 / 1_000.0;

    println!("Policy evaluation (no cache):");
    println!("  Average: {avg_us} µs ({avg_ms:.3} ms)");
    println!("  Max:     {max_us} µs ({max_ms:.3} ms)");
    println!("  Total:   {total_us} µs for {iterations} iterations");

    assert!(
        avg_ms < 1.0,
        "CLAIM VIOLATED: average policy evaluation {avg_ms:.3} ms >= 1 ms threshold"
    );
}

#[tokio::test]
async fn claim1_policy_10k_evaluations_per_second() {
    let config = ResourceAccessConfig {
        default_deny: true,
        enable_caching: false,
        cache_ttl_secs: 300,
        policy_path: None,
        enable_audit: false,
    };
    let ep = DefaultPolicyEnforcementPoint::new(config).await.unwrap();
    let agent_id = AgentId::new();

    // Warm up
    for _ in 0..100 {
        let req = make_access_request("/tmp/warmup.txt");
        ep.check_resource_access(agent_id, &req).await.unwrap();
    }

    let iterations = 10_000u32;
    let start = Instant::now();
    for i in 0..iterations {
        let req = make_access_request(&format!("/tmp/file_{}.txt", i % 100));
        ep.check_resource_access(agent_id, &req).await.unwrap();
    }
    let elapsed = start.elapsed();
    let evals_per_sec = iterations as f64 / elapsed.as_secs_f64();

    println!("Policy throughput:");
    println!("  {iterations} evaluations in {elapsed:.2?}");
    println!("  Throughput: {evals_per_sec:.0} evals/sec");

    assert!(
        evals_per_sec >= 10_000.0,
        "CLAIM VIOLATED: throughput {evals_per_sec:.0} evals/sec < 10,000 threshold"
    );
}

#[tokio::test]
async fn claim1_policy_evaluation_cached_under_1ms() {
    let config = ResourceAccessConfig {
        default_deny: true,
        enable_caching: true,
        cache_ttl_secs: 300,
        policy_path: None,
        enable_audit: false,
    };
    let ep = DefaultPolicyEnforcementPoint::new(config).await.unwrap();
    let agent_id = AgentId::new();

    let request = make_access_request("/tmp/cached_test.txt");

    // Warm the cache
    ep.check_resource_access(agent_id, &request).await.unwrap();

    // Measure cached lookups
    let iterations = 10_000u32;
    let start = Instant::now();
    for _ in 0..iterations {
        ep.check_resource_access(agent_id, &request).await.unwrap();
    }
    let elapsed = start.elapsed();
    let avg_us = elapsed.as_micros() / iterations as u128;
    let avg_ms = avg_us as f64 / 1_000.0;
    let evals_per_sec = iterations as f64 / elapsed.as_secs_f64();

    println!("Policy evaluation (cached):");
    println!("  Average: {avg_us} µs ({avg_ms:.3} ms)");
    println!("  Throughput: {evals_per_sec:.0} evals/sec");

    assert!(
        avg_ms < 1.0,
        "CLAIM VIOLATED: cached evaluation {avg_ms:.3} ms >= 1 ms threshold"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// Claim 2 — SchemaPin signature verification: < 5 ms per tool invocation
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn claim2_schemapin_full_verification_under_5ms() {
    let kp = generate_key_pair().unwrap();
    let schema = serde_json::json!({
        "name": "calculate_sum",
        "description": "Calculates the sum of two numbers",
        "parameters": { "a": "integer", "b": "integer" }
    });
    let hash = canonicalize_and_hash(&schema);
    let signature = sign_data(&kp.private_key_pem, &hash).unwrap();
    let discovery =
        build_well_known_response(&kp.public_key_pem, Some("Test Developer"), vec![], "1.2");

    // Warm up
    for _ in 0..50 {
        let mut ps = KeyPinStore::new();
        let r = verify_schema_offline(
            &schema,
            &signature,
            "example.com",
            "calculate_sum",
            &discovery,
            None,
            &mut ps,
        );
        assert!(r.valid);
    }

    // Measure: each iteration uses a fresh pin store (first-use path)
    let iterations = 500u32;
    let mut max_us = 0u128;
    let mut total_us = 0u128;

    for _ in 0..iterations {
        let mut pin_store = KeyPinStore::new();
        let start = Instant::now();
        let result = verify_schema_offline(
            &schema,
            &signature,
            "example.com",
            "calculate_sum",
            &discovery,
            None,
            &mut pin_store,
        );
        let elapsed = start.elapsed().as_micros();
        assert!(result.valid);
        total_us += elapsed;
        if elapsed > max_us {
            max_us = elapsed;
        }
    }

    let avg_us = total_us / iterations as u128;
    let avg_ms = avg_us as f64 / 1_000.0;
    let max_ms = max_us as f64 / 1_000.0;

    println!("SchemaPin full verification (first-use):");
    println!("  Average: {avg_us} µs ({avg_ms:.3} ms)");
    println!("  Max:     {max_us} µs ({max_ms:.3} ms)");

    // Debug builds are ~2× slower due to unoptimized crypto; use relaxed
    // threshold in CI (debug) while keeping the real claim for release.
    let threshold_ms = if cfg!(debug_assertions) { 10.0 } else { 5.0 };
    assert!(
        avg_ms < threshold_ms,
        "CLAIM VIOLATED: average SchemaPin verification {avg_ms:.3} ms >= {threshold_ms} ms threshold"
    );
}

#[test]
fn claim2_schemapin_verification_pinned_under_5ms() {
    let kp = generate_key_pair().unwrap();
    let schema = serde_json::json!({
        "name": "calculate_sum",
        "description": "Calculates the sum of two numbers",
        "parameters": { "a": "integer", "b": "integer" }
    });
    let hash = canonicalize_and_hash(&schema);
    let signature = sign_data(&kp.private_key_pem, &hash).unwrap();
    let discovery =
        build_well_known_response(&kp.public_key_pem, Some("Test Developer"), vec![], "1.2");

    // Pre-pin the key
    let mut pin_store = KeyPinStore::new();
    let r = verify_schema_offline(
        &schema,
        &signature,
        "example.com",
        "calculate_sum",
        &discovery,
        None,
        &mut pin_store,
    );
    assert!(r.valid);

    // Measure subsequent verifications (pinned key path)
    let iterations = 500u32;
    let mut max_us = 0u128;
    let mut total_us = 0u128;

    for _ in 0..iterations {
        let start = Instant::now();
        let result = verify_schema_offline(
            &schema,
            &signature,
            "example.com",
            "calculate_sum",
            &discovery,
            None,
            &mut pin_store,
        );
        let elapsed = start.elapsed().as_micros();
        assert!(result.valid);
        total_us += elapsed;
        if elapsed > max_us {
            max_us = elapsed;
        }
    }

    let avg_us = total_us / iterations as u128;
    let avg_ms = avg_us as f64 / 1_000.0;
    let max_ms = max_us as f64 / 1_000.0;

    println!("SchemaPin verification (pinned key):");
    println!("  Average: {avg_us} µs ({avg_ms:.3} ms)");
    println!("  Max:     {max_us} µs ({max_ms:.3} ms)");

    // Debug builds are ~2× slower due to unoptimized crypto; use relaxed
    // threshold in CI (debug) while keeping the real claim for release.
    let threshold_ms = if cfg!(debug_assertions) { 10.0 } else { 5.0 };
    assert!(
        avg_ms < threshold_ms,
        "CLAIM VIOLATED: pinned-key verification {avg_ms:.3} ms >= {threshold_ms} ms threshold"
    );
}

#[test]
fn claim2_ecdsa_p256_verify_under_5ms() {
    let kp = generate_key_pair().unwrap();
    let schema = serde_json::json!({
        "name": "calculate_sum",
        "description": "Calculates the sum of two numbers",
        "parameters": { "a": "integer", "b": "integer" }
    });
    let hash = canonicalize_and_hash(&schema);
    let signature = sign_data(&kp.private_key_pem, &hash).unwrap();

    // Warm up
    for _ in 0..100 {
        verify_signature(&kp.public_key_pem, &hash, &signature).unwrap();
    }

    let iterations = 1_000u32;
    let start = Instant::now();
    for _ in 0..iterations {
        assert!(verify_signature(&kp.public_key_pem, &hash, &signature).unwrap());
    }
    let elapsed = start.elapsed();
    let avg_us = elapsed.as_micros() / iterations as u128;
    let avg_ms = avg_us as f64 / 1_000.0;

    println!("ECDSA P-256 verify only:");
    println!("  Average: {avg_us} µs ({avg_ms:.3} ms)");

    // Debug builds are ~2× slower due to unoptimized crypto; use relaxed
    // threshold in CI (debug) while keeping the real claim for release.
    let threshold_ms = if cfg!(debug_assertions) { 10.0 } else { 5.0 };
    assert!(
        avg_ms < threshold_ms,
        "CLAIM VIOLATED: ECDSA verify {avg_ms:.3} ms >= {threshold_ms} ms threshold"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// Registry and queue latency; executing-agent CPU overhead is not established
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn claim3_priority_queue_10k_enqueue() {
    // Measure the heap independently of the production admission bound.
    // This establishes data-structure latency, not live worker scalability.

    // Pre-build tasks outside the timed region
    let tasks: Vec<ScheduledTask> = (0..10_000u32)
        .map(|i| {
            let config = make_agent_config(&format!("agent-{}", i));
            ScheduledTask::new(config)
        })
        .collect();

    // Measure enqueue time (the hot path for schedule_agent)
    let start = Instant::now();
    let mut pq = PriorityQueue::<ScheduledTask>::new();
    for t in tasks {
        pq.push(t);
    }
    let push_elapsed = start.elapsed();
    assert_eq!(pq.len(), 10_000);

    // Measure a single pop from a full 10k-deep queue (the per-tick cost)
    let pop_start = Instant::now();
    let item = pq.pop();
    let pop_elapsed = pop_start.elapsed();
    assert!(item.is_some());

    println!("Priority queue operations:");
    println!("  10k enqueue: {push_elapsed:.2?}");
    println!("  Single pop (depth 10k): {pop_elapsed:.2?}");
    println!(
        "  Per-push: {:.1} µs",
        push_elapsed.as_micros() as f64 / 10_000.0
    );

    // Enqueue 10k should take < 200 ms even in debug mode.
    assert!(
        push_elapsed < Duration::from_millis(200),
        "CLAIM VIOLATED: enqueuing 10k tasks took {push_elapsed:.2?}"
    );

    // A single heap pop should take < 50 ms
    // even in debug mode with 10k items.
    assert!(
        pop_elapsed < Duration::from_millis(50),
        "Single pop from 10k queue took {pop_elapsed:.2?} — too slow"
    );
}

#[tokio::test]
async fn registry_10k_agents_registration_overhead() {
    let scheduler = DefaultAgentScheduler::new(SchedulerConfig::default())
        .await
        .unwrap();
    let configs: Vec<AgentConfig> = (0..10_000)
        .map(|i| make_agent_config(&format!("agent-{i}")))
        .collect();
    let start = Instant::now();
    for config in configs {
        scheduler.register_agent(config).await.unwrap();
    }
    let elapsed = start.elapsed();
    println!("Registering 10k agent configurations: {elapsed:.2?}");
    assert!(
        elapsed < Duration::from_millis(500),
        "registration latency regressed: {elapsed:.2?}"
    );
    assert_eq!(scheduler.list_agents().await.len(), 10_000);
    assert_eq!(scheduler.get_system_status().await.running_agents, 0);
    scheduler.shutdown().await.unwrap();
}

#[tokio::test]
async fn bounded_scheduler_queue_remains_responsive_and_refuses_overflow() {
    // Reserve no worker slots to hold the queue at its admission limit.
    // Real execution and cleanup are covered by scheduler_execution.rs.
    let scheduler = DefaultAgentScheduler::new(SchedulerConfig {
        max_concurrent_agents: 0,
        ..Default::default()
    })
    .await
    .unwrap();
    let mut handles = Vec::new();
    for i in 0..2048 {
        handles.push(
            scheduler
                .schedule_invocation(
                    make_agent_config(&format!("pending-{i}")),
                    serde_json::Value::Null,
                )
                .await
                .unwrap(),
        );
    }
    assert!(scheduler
        .schedule_agent(make_agent_config("overflow"))
        .await
        .is_err());
    let start = Instant::now();
    for _ in 0..5 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(scheduler.get_system_status().await.running_agents, 0);
    }
    let elapsed = start.elapsed();
    println!("Five status checks with a full pending queue: {elapsed:.2?}");
    assert!(
        elapsed < Duration::from_secs(1),
        "scheduler became unresponsive: {elapsed:.2?}"
    );
    scheduler.shutdown().await.unwrap();
    for handle in handles {
        assert_eq!(
            handle.wait().await.status,
            symbi_runtime::scheduler::task_manager::TaskStatus::Terminated
        );
    }
}
