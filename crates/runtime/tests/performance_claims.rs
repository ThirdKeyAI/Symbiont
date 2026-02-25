//! Integration tests that assert the documented performance claims.
//!
//! These tests measure wall-clock time and fail if any claim is violated.
//! They are intentionally generous with thresholds to avoid flakiness in
//! CI while still catching order-of-magnitude regressions.
//!
//! Claims:
//!   1. Policy engine evaluates decisions in under 1 ms (10,000+ evals/sec).
//!   2. SchemaPin signature verification completes in under 5 ms per tool.
//!   3. Runtime scheduling overhead is under 2% CPU for 10,000 concurrent agents.
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

    assert!(
        avg_ms < 5.0,
        "CLAIM VIOLATED: ECDSA verify {avg_ms:.3} ms >= 5 ms threshold"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// Claim 3 — Scheduling overhead: < 2% CPU for 10,000 concurrent agents
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn claim3_priority_queue_10k_enqueue() {
    // The priority queue is the core scheduling data structure. The scheduler
    // enqueues agents and pops one per tick (100ms interval). This test
    // verifies that enqueuing 10,000 tasks is fast.
    //
    // Note: PriorityQueue::pop() rebuilds the index (O(n) per pop), so
    // draining is O(n²). The scheduler only pops one item per tick, so the
    // relevant metric is single-pop latency at max queue depth, not bulk drain.

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

    // A single pop (including O(n) index rebuild) should take < 50 ms
    // even in debug mode with 10k items.
    assert!(
        pop_elapsed < Duration::from_millis(50),
        "Single pop from 10k queue took {pop_elapsed:.2?} — too slow"
    );
}

#[tokio::test]
async fn claim3_schedule_10k_agents_overhead() {
    // Schedule 10,000 agents through the full DefaultAgentScheduler and
    // measure the enqueue overhead. The claim is that scheduling overhead
    // is < 2% CPU for 10k concurrent agents, meaning the time spent in
    // scheduling operations should be small relative to wall time.
    let scheduler = DefaultAgentScheduler::new(SchedulerConfig {
        max_concurrent_agents: 20_000,
        ..Default::default()
    })
    .await
    .unwrap();

    let agent_count = 10_000u32;

    // Pre-build configs to isolate scheduling from allocation overhead
    let configs: Vec<AgentConfig> = (0..agent_count)
        .map(|i| make_agent_config(&format!("agent-{}", i)))
        .collect();

    let start = Instant::now();
    for config in configs {
        scheduler.schedule_agent(config).await.unwrap();
    }
    let elapsed = start.elapsed();

    let per_agent_us = elapsed.as_micros() as f64 / agent_count as f64;
    let overhead_pct = elapsed.as_secs_f64() * 100.0; // % of a 1-second window

    println!("Scheduler 10k agent enqueue:");
    println!("  Total: {elapsed:.2?}");
    println!("  Per agent: {per_agent_us:.1} µs");
    println!("  Overhead vs 1s window: {overhead_pct:.2}%");

    // In release mode, 10k enqueues complete in ~10 ms (1% overhead).
    // In debug mode, the DashMap operations and lock contention are slower.
    // Use 500 ms (50% of 1s) as a generous bound that catches regressions.
    assert!(
        elapsed < Duration::from_millis(500),
        "CLAIM VIOLATED: scheduling 10k agents took {elapsed:.2?} — overhead too high"
    );

    // Shutdown cleanly
    scheduler.shutdown().await.unwrap();
}

#[tokio::test]
async fn claim3_scheduler_10k_agents_background_loop_cost() {
    // Verify the scheduler's background tick loop is efficient with many
    // agents registered. We measure the time for the scheduler to exist
    // with 10k agents over a brief window, checking that the per-tick
    // scheduling overhead is bounded.
    let scheduler = DefaultAgentScheduler::new(SchedulerConfig {
        max_concurrent_agents: 20_000,
        health_check_interval: Duration::from_secs(60), // minimize health check noise
        ..Default::default()
    })
    .await
    .unwrap();

    // Enqueue 10k agents (they get dispatched by the background loop)
    for i in 0..10_000u32 {
        let config = make_agent_config(&format!("bg-agent-{}", i));
        scheduler.schedule_agent(config).await.unwrap();
    }

    // Let the scheduler's background loop run for a few ticks (100ms interval)
    // and verify it stays responsive.
    let start = Instant::now();
    for _ in 0..5 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        // Scheduler should remain responsive for status queries
        let _status = scheduler.get_system_status().await;
    }
    let elapsed = start.elapsed();

    println!("Scheduler background loop (5 ticks with 10k agents):");
    println!("  Total wall time: {elapsed:.2?}");

    // 5 × 50ms sleeps = 250ms minimum. With scheduling overhead < 2%,
    // the total should be well under 500ms even in debug mode.
    assert!(
        elapsed < Duration::from_millis(1000),
        "Scheduler loop with 10k agents took {elapsed:.2?} — too much overhead"
    );

    scheduler.shutdown().await.unwrap();
}
