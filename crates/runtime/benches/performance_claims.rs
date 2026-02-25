//! Performance benchmarks verifying documented performance claims.
//!
//! Claims under test:
//!   1. Policy engine evaluates decisions in under 1 millisecond,
//!      sustaining 10,000+ evaluations per second.
//!   2. SchemaPin signature verification completes in under 5 milliseconds
//!      per tool invocation.
//!   3. Runtime scheduling overhead is under 2% CPU for 10,000 concurrent agents.

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use std::collections::HashMap;
use std::time::{Duration, SystemTime};

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
                author: "bench".to_string(),
                description: "Benchmark agent".to_string(),
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
                request_id: "bench-request".to_string(),
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
// Claim 1: Policy engine < 1 ms per evaluation, 10 000+ evals/sec
// ═══════════════════════════════════════════════════════════════════════════════

fn bench_policy_evaluation_default(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    // Create the real DefaultPolicyEnforcementPoint (loads YAML policies).
    let enforcement_point = rt.block_on(async {
        let config = ResourceAccessConfig {
            default_deny: true,
            enable_caching: false, // Disable cache to measure raw evaluation cost
            cache_ttl_secs: 300,
            policy_path: None,
            enable_audit: false,
        };
        DefaultPolicyEnforcementPoint::new(config).await.unwrap()
    });

    let request = make_access_request("/tmp/test.txt");
    let agent_id = AgentId::new();

    c.bench_function("claim1: policy_eval_no_cache", |b| {
        b.to_async(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap(),
        )
        .iter(|| {
            let req = request.clone();
            let ep = &enforcement_point;
            async move {
                ep.check_resource_access(agent_id, &req).await.unwrap();
            }
        });
    });
}

fn bench_policy_evaluation_cached(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let enforcement_point = rt.block_on(async {
        let config = ResourceAccessConfig {
            default_deny: true,
            enable_caching: true,
            cache_ttl_secs: 300,
            policy_path: None,
            enable_audit: false,
        };
        DefaultPolicyEnforcementPoint::new(config).await.unwrap()
    });

    let request = make_access_request("/tmp/test.txt");
    let agent_id = AgentId::new();

    // Warm the cache
    rt.block_on(async {
        enforcement_point
            .check_resource_access(agent_id, &request)
            .await
            .unwrap();
    });

    c.bench_function("claim1: policy_eval_cached", |b| {
        b.to_async(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap(),
        )
        .iter(|| {
            let req = request.clone();
            let ep = &enforcement_point;
            async move {
                ep.check_resource_access(agent_id, &req).await.unwrap();
            }
        });
    });
}

fn bench_policy_throughput(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let enforcement_point = rt.block_on(async {
        let config = ResourceAccessConfig {
            default_deny: true,
            enable_caching: false,
            cache_ttl_secs: 300,
            policy_path: None,
            enable_audit: false,
        };
        DefaultPolicyEnforcementPoint::new(config).await.unwrap()
    });

    let agent_id = AgentId::new();

    // Benchmark a batch of 10,000 evaluations with varied inputs
    c.bench_function("claim1: policy_eval_10k_batch", |b| {
        b.to_async(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap(),
        )
        .iter(|| {
            let ep = &enforcement_point;
            async move {
                for i in 0..10_000u32 {
                    let req = make_access_request(&format!("/tmp/file_{}.txt", i % 100));
                    ep.check_resource_access(agent_id, &req).await.unwrap();
                }
            }
        });
    });
}

// ═══════════════════════════════════════════════════════════════════════════════
// Claim 2: SchemaPin verification < 5 ms per tool invocation
// ═══════════════════════════════════════════════════════════════════════════════

fn bench_schemapin_verify_offline(c: &mut Criterion) {
    let kp = generate_key_pair().unwrap();
    let schema = serde_json::json!({
        "name": "calculate_sum",
        "description": "Calculates the sum of two numbers",
        "parameters": { "a": "integer", "b": "integer" }
    });
    let hash = canonicalize_and_hash(&schema);
    let signature = sign_data(&kp.private_key_pem, &hash).unwrap();
    let discovery =
        build_well_known_response(&kp.public_key_pem, Some("Bench Developer"), vec![], "1.2");

    c.bench_function("claim2: schemapin_verify_offline", |b| {
        b.iter_batched(
            || KeyPinStore::new(),
            |mut pin_store| {
                verify_schema_offline(
                    &schema,
                    &signature,
                    "example.com",
                    "calculate_sum",
                    &discovery,
                    None,
                    &mut pin_store,
                )
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_schemapin_verify_pinned(c: &mut Criterion) {
    let kp = generate_key_pair().unwrap();
    let schema = serde_json::json!({
        "name": "calculate_sum",
        "description": "Calculates the sum of two numbers",
        "parameters": { "a": "integer", "b": "integer" }
    });
    let hash = canonicalize_and_hash(&schema);
    let signature = sign_data(&kp.private_key_pem, &hash).unwrap();
    let discovery =
        build_well_known_response(&kp.public_key_pem, Some("Bench Developer"), vec![], "1.2");

    // Pre-pin the key
    let mut pin_store = KeyPinStore::new();
    verify_schema_offline(
        &schema,
        &signature,
        "example.com",
        "calculate_sum",
        &discovery,
        None,
        &mut pin_store,
    );

    c.bench_function("claim2: schemapin_verify_pinned", |b| {
        b.iter(|| {
            verify_schema_offline(
                &schema,
                &signature,
                "example.com",
                "calculate_sum",
                &discovery,
                None,
                &mut pin_store,
            )
        });
    });
}

fn bench_schemapin_crypto_only(c: &mut Criterion) {
    let kp = generate_key_pair().unwrap();
    let data = b"benchmark data for signature verification";
    let signature = sign_data(&kp.private_key_pem, data).unwrap();

    c.bench_function("claim2: ecdsa_p256_verify", |b| {
        b.iter(|| {
            verify_signature(&kp.public_key_pem, data, &signature).unwrap();
        });
    });
}

fn bench_schemapin_canonicalize_and_hash(c: &mut Criterion) {
    let schema = serde_json::json!({
        "name": "calculate_sum",
        "description": "Calculates the sum of two numbers",
        "parameters": {
            "a": { "type": "integer", "description": "First number" },
            "b": { "type": "integer", "description": "Second number" }
        },
        "returns": { "type": "integer", "description": "Sum of a and b" }
    });

    c.bench_function("claim2: canonicalize_and_hash", |b| {
        b.iter(|| {
            canonicalize_and_hash(&schema);
        });
    });
}

// ═══════════════════════════════════════════════════════════════════════════════
// Claim 3: Scheduling overhead < 2% CPU for 10,000 concurrent agents
// ═══════════════════════════════════════════════════════════════════════════════

fn bench_scheduler_agent_scheduling(c: &mut Criterion) {
    c.bench_function("claim3: schedule_single_agent", |b| {
        b.to_async(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap(),
        )
        .iter_batched(
            || {
                let rt = tokio::runtime::Handle::current();
                let scheduler = rt.block_on(async {
                    DefaultAgentScheduler::new(SchedulerConfig {
                        max_concurrent_agents: 20_000,
                        ..Default::default()
                    })
                    .await
                    .unwrap()
                });
                scheduler
            },
            |scheduler: DefaultAgentScheduler| async move {
                let config = make_agent_config("bench-agent");
                scheduler.schedule_agent(config).await.unwrap();
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_priority_queue_enqueue(c: &mut Criterion) {
    // Measures enqueue throughput for 10k tasks. Excludes pop since the
    // scheduler only pops one item per 100ms tick.
    c.bench_function("claim3: priority_queue_enqueue_10k", |b| {
        b.iter_batched(
            || {
                (0..10_000u32)
                    .map(|i| {
                        let config = make_agent_config(&format!("agent-{}", i));
                        ScheduledTask::new(config)
                    })
                    .collect::<Vec<_>>()
            },
            |tasks| {
                let mut pq = PriorityQueue::<ScheduledTask>::new();
                for t in tasks {
                    pq.push(t);
                }
                assert_eq!(pq.len(), 10_000);
            },
            BatchSize::LargeInput,
        );
    });
}

fn bench_scheduler_10k_agents(c: &mut Criterion) {
    // Measures wall-clock time for scheduling 10,000 agents through the
    // full DefaultAgentScheduler. The CPU overhead claim is verified by
    // comparing scheduling time to a 1-second window.
    c.bench_function("claim3: schedule_10k_agents", |b| {
        b.to_async(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap(),
        )
        .iter_batched(
            || {
                let rt = tokio::runtime::Handle::current();
                rt.block_on(async {
                    DefaultAgentScheduler::new(SchedulerConfig {
                        max_concurrent_agents: 20_000,
                        ..Default::default()
                    })
                    .await
                    .unwrap()
                })
            },
            |scheduler: DefaultAgentScheduler| async move {
                for i in 0..10_000u32 {
                    let config = make_agent_config(&format!("agent-{}", i));
                    scheduler.schedule_agent(config).await.unwrap();
                }
            },
            BatchSize::LargeInput,
        );
    });
}

// ═══════════════════════════════════════════════════════════════════════════════
// Criterion groups
// ═══════════════════════════════════════════════════════════════════════════════

criterion_group! {
    name = policy_engine;
    config = Criterion::default()
        .sample_size(100)
        .measurement_time(Duration::from_secs(5));
    targets =
        bench_policy_evaluation_default,
        bench_policy_evaluation_cached,
        bench_policy_throughput,
}

criterion_group! {
    name = schemapin;
    config = Criterion::default()
        .sample_size(200)
        .measurement_time(Duration::from_secs(5));
    targets =
        bench_schemapin_verify_offline,
        bench_schemapin_verify_pinned,
        bench_schemapin_crypto_only,
        bench_schemapin_canonicalize_and_hash,
}

criterion_group! {
    name = scheduler;
    config = Criterion::default()
        .sample_size(20)
        .measurement_time(Duration::from_secs(10));
    targets =
        bench_scheduler_agent_scheduling,
        bench_priority_queue_enqueue,
        bench_scheduler_10k_agents,
}

criterion_main!(policy_engine, schemapin, scheduler);
