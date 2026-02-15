# Symbiont Agent Development Skills Guide

**Purpose**: This guide helps AI assistants quickly build secure, compliant Symbiont agents following best practices.

**For Full Documentation**: See [DSL Guide](https://github.com/thirdkeyai/symbiont/blob/main/docs/dsl-guide.md), [DSL Specification](https://github.com/thirdkeyai/symbiont/blob/main/docs/dsl-specification.md), and [Example Agents](https://github.com/thirdkeyai/symbiont/blob/main/agents/README.md)

## What Makes Symbiont Unique

- **Zero-Trust Security**: All inputs untrusted by default, explicit policies required
- **Policy-as-Code**: Declarative security rules enforced at runtime
- **Multi-Tier Sandboxing**: Docker → gVisor → Firecracker isolation
- **Enterprise Compliance**: HIPAA, SOC2, GDPR patterns built-in
- **Cryptographic Verification**: SchemaPin for MCP tools, Ed25519 signatures
- **Webhook DX**: Signature verification middleware with GitHub/Stripe/Slack presets
- **Persistent Memory**: Markdown-backed agent memory with retention and compaction

---

## Quick Start Template

### Minimal Viable Agent

```dsl
metadata {
    version = "1.0.0"
    author = "Your Name"
    description = "Brief description of what this agent does"
    tags = ["category", "use-case"]
}

agent my_agent(input: String) -> String {
    capabilities = ["process_data", "validate_input"]

    policy security_policy {
        // Allow specific operations
        allow: ["read_input", "write_output"] if input.length() < 10000

        // Deny dangerous operations
        deny: ["network_access", "file_system"] if true

        // Require conditions
        require: {
            input_validation: true,
            output_sanitization: true
        }

        // Audit important actions
        audit: {
            log_level: "info",
            include_input: false,  // Don't log sensitive data
            include_output: true
        }
    }

    with memory = "ephemeral", security = "high", timeout = 30000 {
        try {
            // Validate input
            if input.is_empty() {
                return error("Input cannot be empty");
            }

            // Process data
            let result = process(input);

            // Return result
            return result;

        } catch (error) {
            log("ERROR", "Processing failed: " + error.message);
            return error("Processing failed");
        }
    }
}

fn process(data: String) -> String {
    // Your processing logic here
    return data.to_uppercase();
}
```

---

## Security-First Policy Patterns

### 1. Data Processing Agent (Read/Transform/Write)

```dsl
policy data_processing_policy {
    // Allow data operations with size limits
    allow: [
        "read_data",
        "transform_data",
        "write_output"
    ] if request.data_size < 10_000_000  // 10MB limit

    // Deny dangerous operations
    deny: [
        "execute_code",
        "spawn_process",
        "network_access"
    ] if true

    // Require validation
    require: {
        input_validation: true,
        output_sanitization: true,
        rate_limiting: "100/minute"
    }

    // Audit with PII protection
    audit: {
        log_level: "info",
        include_input: false,      // Protect PII
        include_output: false,     // Protect PII
        include_metadata: true,
        retention_days: 90
    }
}
```

### 2. API Integration Agent (External Calls)

```dsl
policy api_integration_policy {
    // Allow specific endpoints only
    allow: [
        "https_request"
    ] if request.url.starts_with("https://api.trusted-service.com/")

    // Deny everything else
    deny: [
        "http_request",           // Only HTTPS
        "file_access",
        "database_access"
    ] if true

    // Require security measures
    require: {
        tls_verification: true,
        api_key_rotation: "30_days",
        rate_limiting: "1000/hour",
        timeout: "5000ms"
    }

    // Audit all API calls
    audit: {
        log_level: "info",
        include_request_headers: true,
        include_response_status: true,
        include_latency: true,
        alert_on_errors: true
    }
}
```

### 3. Security Scanning Agent (Audit/Compliance)

```dsl
policy security_scanner_policy {
    // Allow read-only scanning
    allow: [
        "read_files",
        "analyze_code",
        "check_dependencies",
        "validate_configs"
    ] if scan.depth <= 5  // Limit recursion

    // Deny modifications
    deny: [
        "write_files",
        "modify_permissions",
        "execute_code"
    ] if true

    // Require strict controls
    require: {
        signature_verification: true,
        checksum_validation: true,
        sandbox_tier: "Tier2",  // gVisor isolation
        max_scan_time: "300000ms"  // 5 minutes
    }

    // Audit findings
    audit: {
        log_level: "warning",
        include_findings: true,
        include_risk_scores: true,
        alert_on_critical: true,
        compliance_tags: ["HIPAA", "SOC2"]
    }
}
```

### 4. Workflow Orchestration Agent (Multi-Step)

```dsl
policy orchestration_policy {
    // Allow agent coordination
    allow: [
        "invoke_agent",
        "message_passing",
        "state_management"
    ] if orchestration.depth < 10  // Prevent infinite loops

    // Deny resource-intensive ops
    deny: [
        "spawn_unlimited_agents",
        "recursive_orchestration"
    ] if true

    // Require controls
    require: {
        max_concurrent_agents: 50,
        total_timeout: "600000ms",  // 10 minutes
        failure_recovery: "retry_with_backoff",
        circuit_breaker: true
    }

    // Audit workflow
    audit: {
        log_level: "info",
        include_workflow_graph: true,
        include_timing: true,
        include_dependencies: true,
        trace_id: true
    }
}
```

---

## Sandbox Tier Selection Guide

| Tier | Technology | Use Case | Performance | Security | Overhead |
|------|------------|----------|-------------|----------|----------|
| **Tier1** | Docker | General workloads | Fast | Good | Low (~100ms) |
| **Tier2** | gVisor | Untrusted code | Medium | High | Medium (~500ms) |
| **Tier3** | Firecracker | Multi-tenant isolation | Slower | Maximum | High (~2s) |
| **Native** | Process only | Development ONLY | Fastest | None | Minimal |

**Selection Guide**:
- **Tier1 (Docker)**: Default choice for most agents
- **Tier2 (gVisor)**: Processing external data, user-provided code
- **Tier3 (Firecracker)**: Highly sensitive, regulatory compliance
- **Native**: NEVER use in production (development/testing only)

---

## DSL Syntax Cheatsheet

### Type System

```dsl
// Primitives
let name: String = "value";
let count: Integer = 42;
let price: Float = 19.99;
let active: Boolean = true;
let data: Bytes = [0x01, 0x02, 0x03];

// Collections
let tags: Array<String> = ["tag1", "tag2"];
let config: Map<String, String> = {"key": "value"};
let unique: Set<Integer> = {1, 2, 3};

// Security-Aware Types
let sensitive: EncryptedData<String> = encrypt("secret");
let private_data: PrivateData<Integer> = private(123);
let verified: VerifiableResult<String> = sign("data");

// Optional Types
let optional: Optional<String> = Some("value");
let none_value: Optional<String> = None;
```

### Control Flow

```dsl
// If/Else
if condition {
    // true branch
} else if other_condition {
    // else if branch
} else {
    // false branch
}

// Match (Pattern Matching)
match value {
    Some(x) => process(x),
    None => default_value,
    Error(e) => handle_error(e)
}

// Loops
for item in collection {
    process(item);
}

while condition {
    do_work();
}

// Error Handling
try {
    risky_operation();
} catch (error) {
    log("ERROR", error.message);
    return error("Operation failed");
}
```

### Policy Language

```dsl
policy policy_name {
    // Allow operations with conditions
    allow: ["operation1", "operation2"] if condition
    allow: "operation3" if complex.condition && other.check

    // Deny operations
    deny: ["dangerous_op"] if true
    deny: "risky_op" if environment == "production"

    // Required conditions
    require: {
        authentication: true,
        authorization: "role:admin",
        encryption: "AES-256-GCM",
        rate_limit: "100/second"
    }

    // Audit specification
    audit: {
        log_level: "info" | "warning" | "error",
        include_input: boolean,
        include_output: boolean,
        retention_days: integer,
        compliance_tags: array<string>
    }
}
```

### With Block (Execution Context)

```dsl
with
    memory = "ephemeral" | "persistent",
    privacy = "strict" | "medium" | "low",
    security = "high" | "medium" | "low",
    sandbox = "Tier1" | "Tier2" | "Tier3",
    timeout = milliseconds,
    requires = ["clearance:level5", "approval:manager"]
{
    // Agent implementation
}
```

---

## Integration Patterns

### 1. Secrets Management (Vault/OpenBao)

```dsl
agent secure_api_caller(endpoint: String) -> String {
    policy secret_access {
        allow: ["read_secret"] if secret.path.starts_with("application/")
        deny: ["write_secret", "delete_secret"] if true

        require: {
            vault_auth: true,
            token_rotation: "1_hour"
        }

        audit: {
            log_level: "warning",
            include_secret_path: true,
            include_secret_value: false  // NEVER log secrets
        }
    }

    with memory = "ephemeral", security = "high" {
        // Reference secrets using vault:// protocol
        let api_key = vault://application/api/key;
        let api_secret = vault://application/api/secret;

        // Use secrets in API call
        let response = http.post(endpoint, {
            headers: {
                "Authorization": "Bearer " + api_key,
                "X-API-Secret": api_secret
            }
        });

        return response.body;
    }
}
```

### 2. MCP Tool Integration (Cryptographic Verification)

```dsl
agent mcp_tool_user(tool_name: String, input: String) -> String {
    capabilities = ["invoke_mcp_tool"]

    policy mcp_security {
        // Only allow verified tools
        allow: ["mcp_invoke"] if tool.verified == true
        deny: ["mcp_invoke"] if tool.signature_invalid

        require: {
            schema_pin_verification: true,  // ECDSA P-256
            tofu_trust_model: true,         // Trust-On-First-Use
            tool_review_required: false     // Auto for signed tools
        }

        audit: {
            log_level: "info",
            include_tool_signature: true,
            include_tool_schema: true
        }
    }

    with security = "high" {
        // Discover and invoke MCP tool
        let tool = mcp.discover(tool_name);

        // Verify cryptographic signature
        if !tool.verify_signature() {
            return error("Tool signature verification failed");
        }

        // Invoke tool
        let result = mcp.invoke(tool, input);
        return result;
    }
}
```

### 3. HTTP Webhook Processing

```dsl
agent webhook_processor(request: HttpRequest) -> HttpResponse {
    capabilities = ["process_webhook", "validate_signature"]

    policy webhook_policy {
        allow: ["parse_json", "validate_data"] if request.size < 1_000_000
        deny: ["execute_code", "file_access"] if true

        require: {
            signature_verification: true,
            rate_limiting: "1000/minute",
            timeout: "5000ms"
        }

        audit: {
            log_level: "info",
            include_request_id: true,
            include_source_ip: true,
            alert_on_invalid_signature: true
        }
    }

    with memory = "ephemeral", timeout = 5000 {
        // Verify webhook signature (e.g., GitHub, Stripe)
        let signature = request.headers["X-Webhook-Signature"];
        let secret = vault://webhooks/secret;

        if !verify_hmac_sha256(request.body, secret, signature) {
            return HttpResponse(401, "Invalid signature");
        }

        // Parse and process webhook
        let data = json.parse(request.body);
        let result = process_event(data);

        return HttpResponse(200, json.stringify(result));
    }
}
```

### 4. Scheduled Execution

```dsl
metadata {
    schedule = "0 */6 * * *"  // Every 6 hours (cron format)
}

agent scheduled_cleanup() -> String {
    capabilities = ["cleanup_data", "archival"]

    policy cleanup_policy {
        allow: ["read_old_data", "archive", "delete"] if data.age > 90_days
        deny: ["delete"] if data.age <= 90_days

        require: {
            backup_verification: true,
            retention_check: true
        }

        audit: {
            log_level: "warning",
            include_deleted_count: true,
            include_archived_count: true
        }
    }

    with memory = "persistent", timeout = 300000 {
        let old_data = query_old_data(90);
        let archived_count = archive_data(old_data);
        let deleted_count = delete_archived_data(old_data);

        return "Archived: " + archived_count + ", Deleted: " + deleted_count;
    }
}
```

### 5. Persistent Memory (DSL Configuration)

```dsl
// Top-level memory block — configures Markdown-backed agent memory
memory agent_memory {
    store    markdown           // Storage backend (markdown only for now)
    path     "data/agents"     // Root directory for memory files
    retention 90d              // How long daily logs are kept
    search {
        vector_weight  0.7     // Semantic similarity weight
        keyword_weight 0.3     // BM25 keyword match weight
    }
}
```

Memory files are human-readable Markdown stored at `data/agents/{agent_id}/memory.md` with sections for Facts, Procedures, and Learned Patterns. Daily interaction logs are appended to `logs/{date}.md` and compacted based on retention policy.

**REPL Commands**:
- `:memory inspect <agent-id>` — Display agent's memory.md
- `:memory compact <agent-id>` — Flush daily logs, remove expired entries
- `:memory purge <agent-id>` — Delete all memory for an agent

### 6. Webhook Endpoints (DSL Configuration)

```dsl
// Top-level webhook block — defines verified webhook endpoints
webhook github_events {
    path     "/hooks/github"
    provider github                              // Preset: github, stripe, slack, custom
    secret   "secret://vault/github-webhook-secret"  // HMAC secret (supports vault refs)
    agent    code_review_agent                   // Route to this agent
    filter {
        json_path "$.action"
        equals    "opened"                       // Only process "opened" events
    }
}
```

Provider presets configure signature verification automatically:
- **github**: `X-Hub-Signature-256` header, `sha256=` prefix, HMAC-SHA256
- **stripe**: `Stripe-Signature` header, HMAC-SHA256
- **slack**: `X-Slack-Signature` header, `v0=` prefix, HMAC-SHA256
- **custom**: `X-Signature` header, HMAC-SHA256

All signatures are verified using constant-time comparison before the request reaches the agent handler. Invalid signatures return HTTP 401.

**REPL Commands**:
- `:webhook list` — Show configured webhook definitions

### 7. Persistent Memory & RAG Engine

```dsl
agent knowledge_assistant(query: String) -> String {
    capabilities = ["semantic_search", "rag_retrieval", "synthesis"]

    policy knowledge_policy {
        allow: [
            "vector_search",
            "knowledge_retrieval",
            "context_synthesis"
        ] if query.length() < 1000

        deny: ["knowledge_modification"] if true

        require: {
            embedding_model: "all-MiniLM-L6-v2",
            similarity_threshold: 0.7,
            max_results: 10
        }

        audit: {
            log_level: "info",
            include_query: true,
            include_relevance_scores: true
        }
    }

    with memory = "persistent", security = "medium" {
        // Semantic search in vector database
        let context = rag.search(query, {
            top_k: 5,
            similarity_threshold: 0.7
        });

        // Synthesize response
        let response = synthesize(query, context);

        // Store interaction for future learning
        memory.store({
            query: query,
            response: response,
            timestamp: now()
        });

        return response;
    }
}
```

### 8. Inter-Agent Communication

```dsl
agent coordinator(task: String) -> String {
    capabilities = ["message_passing", "agent_coordination"]

    policy coordination_policy {
        allow: [
            "send_message",
            "receive_message",
            "invoke_agent"
        ] if coordination.depth < 5

        deny: ["broadcast"] if true  // Prevent message storms

        require: {
            message_encryption: true,  // AES-256-GCM
            message_signing: true,     // Ed25519
            max_concurrent_agents: 10
        }

        audit: {
            log_level: "info",
            include_message_flow: true,
            include_agent_graph: true
        }
    }

    with memory = "persistent" {
        // Invoke specialized agent
        let validator_response = agent.invoke("data_validator", {
            data: task
        });

        // Send encrypted message to another agent
        agent.send_message("processor_agent", {
            type: "process_request",
            payload: validator_response,
            priority: "high"
        });

        // Wait for response
        let result = agent.receive_message(timeout = 10000);

        return result.payload;
    }
}
```

---

## Common Agent Patterns

### Pattern 1: Data Validation Pipeline

```dsl
agent data_validator(data: String, schema: String) -> ValidationResult {
    capabilities = ["schema_validation", "data_quality_check"]

    policy validation_policy {
        allow: ["parse_schema", "validate_data", "quality_scoring"]
        deny: ["modify_data", "execute_code"]
        require: {
            max_data_size: "10MB",
            timeout: "5000ms"
        }
        audit: {
            log_level: "warning",
            include_validation_errors: true
        }
    }

    with memory = "ephemeral", security = "high" {
        try {
            // Parse schema
            let parsed_schema = json.parse(schema);

            // Validate against schema
            let validation = validate(data, parsed_schema);

            // Calculate quality score
            let quality_score = calculate_quality(data);

            return ValidationResult {
                valid: validation.success,
                errors: validation.errors,
                quality_score: quality_score,
                recommendations: generate_recommendations(validation)
            };

        } catch (error) {
            return ValidationResult {
                valid: false,
                errors: [error.message],
                quality_score: 0.0,
                recommendations: []
            };
        }
    }
}
```

### Pattern 2: Format Converter

```dsl
agent format_converter(data: String, from_format: String, to_format: String) -> String {
    capabilities = ["parse_format", "transform_data", "serialize_format"]

    policy conversion_policy {
        allow: ["parse", "transform", "serialize"] if data.size < 50_000_000
        deny: ["execute_code", "file_access"]
        require: {
            supported_formats: ["json", "xml", "yaml", "csv"],
            charset_validation: true
        }
        audit: {
            log_level: "info",
            include_conversion_stats: true
        }
    }

    with memory = "ephemeral", timeout = 10000 {
        // Validate formats
        if !is_supported(from_format) || !is_supported(to_format) {
            return error("Unsupported format");
        }

        // Parse source format
        let parsed = parse(data, from_format);

        // Transform to intermediate representation
        let transformed = normalize(parsed);

        // Serialize to target format
        let result = serialize(transformed, to_format);

        return result;
    }
}
```

### Pattern 3: API Aggregator

```dsl
agent api_aggregator(sources: Array<String>) -> AggregatedData {
    capabilities = ["parallel_requests", "data_normalization", "deduplication"]

    policy aggregation_policy {
        allow: ["https_request"] if url in sources
        deny: ["http_request", "file_access"]
        require: {
            tls_verification: true,
            concurrent_limit: 10,
            timeout_per_request: "3000ms",
            total_timeout: "15000ms"
        }
        audit: {
            log_level: "info",
            include_source_latencies: true,
            alert_on_source_failure: true
        }
    }

    with memory = "ephemeral", timeout = 15000 {
        let results = [];

        // Parallel fetch from all sources
        for source in sources {
            async {
                try {
                    let response = http.get(source, {
                        timeout: 3000,
                        verify_tls: true
                    });
                    results.push(response.json());
                } catch (error) {
                    log("WARNING", "Source failed: " + source);
                }
            }
        }

        // Wait for all requests
        await_all(results);

        // Normalize and deduplicate
        let normalized = normalize_data(results);
        let deduplicated = deduplicate(normalized);

        return AggregatedData {
            sources: sources.length,
            records: deduplicated.length,
            data: deduplicated
        };
    }
}
```

### Pattern 4: Security Scanner

```dsl
agent security_scanner(target: String, scan_type: String) -> ScanReport {
    capabilities = [
        "vulnerability_detection",
        "dependency_analysis",
        "compliance_check"
    ]

    policy scanner_policy {
        allow: [
            "read_files",
            "analyze_dependencies",
            "check_vulnerabilities"
        ] if scan.depth <= 10

        deny: [
            "write_files",
            "execute_code",
            "network_access"
        ]

        require: {
            sandbox_tier: "Tier2",  // gVisor isolation
            cvss_scoring: true,
            cwe_classification: true
        }

        audit: {
            log_level: "warning",
            include_findings: true,
            include_cvss_scores: true,
            compliance_tags: ["OWASP", "CWE"]
        }
    }

    with memory = "ephemeral", security = "high", sandbox = "Tier2" {
        let findings = [];

        // Scan based on type
        match scan_type {
            "dependencies" => {
                findings = scan_dependencies(target);
            },
            "vulnerabilities" => {
                findings = scan_vulnerabilities(target);
            },
            "compliance" => {
                findings = check_compliance(target, ["HIPAA", "SOC2"]);
            },
            _ => {
                return error("Unknown scan type");
            }
        }

        // Calculate risk score
        let risk_score = calculate_risk(findings);

        return ScanReport {
            target: target,
            scan_type: scan_type,
            findings_count: findings.length,
            critical_count: count_by_severity(findings, "CRITICAL"),
            high_count: count_by_severity(findings, "HIGH"),
            risk_score: risk_score,
            findings: findings,
            recommendations: generate_remediation(findings)
        };
    }
}
```

### Pattern 5: Notification Router

```dsl
agent notification_router(event: Event, routing_rules: RoutingRules) -> String {
    capabilities = ["event_filtering", "multi_channel_delivery", "retry_logic"]

    policy notification_policy {
        allow: [
            "send_email",
            "send_slack",
            "send_webhook"
        ] if event.priority != "spam"

        deny: ["send_sms"] if event.priority == "low"  // Cost control

        require: {
            rate_limiting: "100/minute",
            retry_attempts: 3,
            backoff_strategy: "exponential"
        }

        audit: {
            log_level: "info",
            include_delivery_status: true,
            include_retry_count: true
        }
    }

    with memory = "ephemeral" {
        // Filter event
        if !should_notify(event, routing_rules) {
            return "Event filtered";
        }

        // Determine channels
        let channels = select_channels(event, routing_rules);

        // Send notifications with retry
        let results = [];
        for channel in channels {
            let success = send_with_retry(channel, event, max_attempts = 3);
            results.push({channel: channel, success: success});
        }

        return format_results(results);
    }
}
```

### Pattern 6: Workflow Orchestrator

```dsl
agent workflow_orchestrator(workflow_spec: WorkflowSpec) -> WorkflowResult {
    capabilities = [
        "step_execution",
        "dependency_resolution",
        "failure_recovery"
    ]

    policy orchestration_policy {
        allow: [
            "invoke_agent",
            "manage_state",
            "handle_errors"
        ] if workflow.depth < 10

        deny: [
            "recursive_workflows",
            "unlimited_agents"
        ]

        require: {
            max_concurrent_steps: 20,
            total_timeout: "600000ms",  // 10 minutes
            checkpoint_enabled: true,
            circuit_breaker: true
        }

        audit: {
            log_level: "info",
            include_workflow_graph: true,
            include_step_timing: true,
            include_failure_trace: true
        }
    }

    with memory = "persistent", timeout = 600000 {
        let state = WorkflowState.new(workflow_spec);

        try {
            // Execute workflow steps
            for step in workflow_spec.steps {
                // Check dependencies
                if !dependencies_met(step, state) {
                    await_dependencies(step, state);
                }

                // Execute step
                let result = execute_step(step, state);

                // Update state with checkpoint
                state.complete_step(step.id, result);
                checkpoint(state);
            }

            return WorkflowResult {
                status: "completed",
                outputs: state.collect_outputs(),
                execution_time: state.elapsed_time()
            };

        } catch (error) {
            // Attempt recovery
            if can_recover(error, state) {
                let recovered_state = recover_workflow(state);
                return resume_workflow(recovered_state);
            }

            return WorkflowResult {
                status: "failed",
                error: error.message,
                completed_steps: state.completed_steps(),
                checkpoint: state.last_checkpoint()
            };
        }
    }
}
```

---

## Security Anti-Patterns to Avoid

### ❌ Anti-Pattern 1: Missing Policy Definitions

```dsl
// BAD: No policies defined
agent insecure_agent(input: String) -> String {
    with memory = "ephemeral" {
        return process(input);
    }
}
```

✅ **Fix**: Always define explicit policies

```dsl
agent secure_agent(input: String) -> String {
    policy security_policy {
        allow: ["process_data"] if input.length() < 10000
        deny: ["network_access", "file_access"]
        require: {input_validation: true}
        audit: {log_level: "info"}
    }

    with memory = "ephemeral" {
        return process(input);
    }
}
```

### ❌ Anti-Pattern 2: Overly Permissive Policies

```dsl
// BAD: Allows everything
policy bad_policy {
    allow: "*" if true
}
```

✅ **Fix**: Use principle of least privilege

```dsl
policy good_policy {
    // Only allow what's needed
    allow: ["read_data", "write_output"] if authorized
    // Explicitly deny risky operations
    deny: ["execute_code", "network_access", "file_system"]
    require: {authentication: true}
}
```

### ❌ Anti-Pattern 3: No Resource Limits

```dsl
// BAD: No timeout, unlimited memory
with memory = "persistent" {
    // Could run forever or consume unlimited memory
    while true {
        expensive_operation();
    }
}
```

✅ **Fix**: Always set resource limits

```dsl
with
    memory = "ephemeral",       // Use ephemeral when possible
    timeout = 30000,            // 30 second timeout
    max_memory_mb = 512,        // Memory limit
    max_cpu_cores = 1.0         // CPU limit
{
    for item in limited_dataset {
        process(item);
    }
}
```

### ❌ Anti-Pattern 4: Logging Sensitive Data

```dsl
// BAD: Logs passwords and secrets
audit: {
    log_level: "info",
    include_input: true,   // Will log passwords!
    include_output: true
}
```

✅ **Fix**: Never log sensitive data

```dsl
audit: {
    log_level: "info",
    include_input: false,      // Protect PII/secrets
    include_output: false,     // Protect PII/secrets
    include_metadata: true,    // OK to log metadata
    include_timing: true       // OK to log performance
}
```

### ❌ Anti-Pattern 5: Hardcoded Secrets

```dsl
// BAD: API key hardcoded
let api_key = "sk_live_abc123xyz789";
```

✅ **Fix**: Use Vault references

```dsl
// GOOD: Secret from Vault
let api_key = vault://application/api/key;
```

### ❌ Anti-Pattern 6: No Input Validation

```dsl
// BAD: No validation
agent bad_agent(input: String) -> String {
    return execute_command(input);  // Command injection risk!
}
```

✅ **Fix**: Always validate input

```dsl
agent good_agent(input: String) -> String {
    // Validate input
    if !is_valid_input(input) {
        return error("Invalid input");
    }

    // Sanitize before use
    let sanitized = sanitize(input);
    return safe_process(sanitized);
}
```

### ❌ Anti-Pattern 7: Wrong Sandbox Tier

```dsl
// BAD: Processing untrusted code in Tier1
agent code_runner(untrusted_code: String) -> String {
    with sandbox = "Tier1" {  // Not enough isolation!
        return eval(untrusted_code);
    }
}
```

✅ **Fix**: Use appropriate sandbox tier

```dsl
agent code_runner(untrusted_code: String) -> String {
    policy strict_isolation {
        deny: ["file_access", "network_access"]
        require: {sandbox_tier: "Tier3"}
    }

    with sandbox = "Tier3" {  // Firecracker microVM
        return safe_eval(untrusted_code);
    }
}
```

### ❌ Anti-Pattern 8: No Error Handling

```dsl
// BAD: Unhandled errors will crash agent
agent fragile_agent(url: String) -> String {
    let response = http.get(url);  // What if this fails?
    return response.body;
}
```

✅ **Fix**: Always handle errors

```dsl
agent robust_agent(url: String) -> String {
    try {
        let response = http.get(url, timeout = 5000);

        if response.status != 200 {
            return error("HTTP error: " + response.status);
        }

        return response.body;

    } catch (error) {
        log("ERROR", "Request failed: " + error.message);
        return error("Request failed");
    }
}
```

---

## Validation Checklist

Before deploying an agent, verify:

### Security Checklist
- [ ] **Policies defined** for all operations
- [ ] **Principle of least privilege** applied (deny by default)
- [ ] **Sandbox tier** appropriate for workload
- [ ] **Secrets** referenced via Vault (never hardcoded)
- [ ] **Input validation** present for all user inputs
- [ ] **Output sanitization** prevents injection attacks
- [ ] **No sensitive data** in audit logs

### Resource Management
- [ ] **Timeout** configured appropriately
- [ ] **Memory limits** set based on workload
- [ ] **CPU limits** defined
- [ ] **Concurrency limits** for agent invocations
- [ ] **Rate limiting** configured for external calls

### Error Handling
- [ ] **Try/catch blocks** around risky operations
- [ ] **Error messages** are informative but not leaking secrets
- [ ] **Retry logic** for transient failures
- [ ] **Circuit breakers** for cascading failures
- [ ] **Graceful degradation** when dependencies fail

### Compliance & Audit
- [ ] **Audit logging** configured
- [ ] **Compliance tags** added (HIPAA, SOC2, GDPR as needed)
- [ ] **Retention policies** set appropriately
- [ ] **PII handling** follows regulations
- [ ] **Data encryption** at rest and in transit

### Testing
- [ ] **Unit tests** for core functions
- [ ] **Integration tests** with dependencies
- [ ] **Security tests** (injection, overflow, etc.)
- [ ] **Performance tests** (within resource limits)
- [ ] **Chaos tests** (failure scenarios)

---

## Quick Reference

### Built-in Functions

| Category | Function | Purpose |
|----------|----------|---------|
| **String** | `len(s)` | String length |
| | `to_upper(s)` | Convert to uppercase |
| | `to_lower(s)` | Convert to lowercase |
| | `trim(s)` | Remove whitespace |
| | `split(s, delim)` | Split string |
| | `contains(s, substr)` | Check substring |
| **JSON** | `json.parse(s)` | Parse JSON string |
| | `json.stringify(obj)` | Convert to JSON |
| | `json.validate(s, schema)` | Validate against schema |
| **HTTP** | `http.get(url, opts)` | HTTP GET request |
| | `http.post(url, body, opts)` | HTTP POST request |
| | `verify_hmac_sha256(data, secret, sig)` | Verify HMAC signature |
| **Crypto** | `encrypt(data)` | AES-256-GCM encrypt |
| | `decrypt(data)` | AES-256-GCM decrypt |
| | `hash_sha256(data)` | SHA-256 hash |
| | `sign(data)` | Ed25519 signature |
| **Time** | `now()` | Current timestamp |
| | `sleep(ms)` | Sleep for milliseconds |
| | `format_time(ts, fmt)` | Format timestamp |
| **Logging** | `log(level, msg)` | Log message |
| | `debug(msg)` | Debug log |
| | `info(msg)` | Info log |
| | `warn(msg)` | Warning log |
| | `error(msg)` | Error log |
| **Validation** | `is_valid_email(s)` | Email validation |
| | `is_valid_url(s)` | URL validation |
| | `is_valid_json(s)` | JSON validation |
| **Arrays** | `push(arr, item)` | Add to array |
| | `pop(arr)` | Remove from array |
| | `map(arr, fn)` | Map function |
| | `filter(arr, fn)` | Filter array |
| | `reduce(arr, fn, init)` | Reduce array |

### Resource Limit Recommendations

| Workload Type | Memory | CPU | Timeout | Sandbox |
|---------------|--------|-----|---------|---------|
| **Data Validation** | 256MB | 0.5 | 5s | Tier1 |
| **Format Conversion** | 512MB | 1.0 | 10s | Tier1 |
| **API Integration** | 512MB | 1.0 | 15s | Tier1 |
| **Code Analysis** | 1GB | 2.0 | 30s | Tier2 |
| **Security Scan** | 2GB | 2.0 | 60s | Tier2 |
| **ML Inference** | 4GB | 4.0 | 120s | Tier2 |
| **Workflow Orchestration** | 1GB | 1.0 | 600s | Tier1 |
| **Untrusted Code** | 512MB | 1.0 | 10s | Tier3 |

### Common Error Codes

| Code | Meaning | Resolution |
|------|---------|------------|
| `POLICY_VIOLATION` | Operation denied by policy | Check policy allow/deny rules |
| `RESOURCE_EXCEEDED` | Resource limit reached | Increase limits or optimize code |
| `TIMEOUT` | Execution timeout | Increase timeout or optimize |
| `AUTH_FAILED` | Authentication failed | Check Vault credentials |
| `SIGNATURE_INVALID` | Crypto signature invalid | Verify tool signature |
| `SANDBOX_ERROR` | Sandbox isolation failed | Check sandbox tier compatibility |
| `VALIDATION_ERROR` | Input validation failed | Fix input data format |
| `NETWORK_ERROR` | Network request failed | Check endpoint and connectivity |

### Documentation Links

- **Full DSL Guide**: [docs/dsl-guide.md](https://github.com/thirdkeyai/symbiont/blob/main/docs/dsl-guide.md)
- **DSL Specification**: [docs/dsl-specification.md](https://github.com/thirdkeyai/symbiont/blob/main/docs/dsl-specification.md)
- **Example Agents**: [agents/README.md](https://github.com/thirdkeyai/symbiont/blob/main/agents/README.md) (8 production examples)
- **Runtime Architecture**: [docs/runtime-architecture.md](https://github.com/thirdkeyai/symbiont/blob/main/docs/runtime-architecture.md)
- **API Reference**: [docs/api-reference.md](https://github.com/thirdkeyai/symbiont/blob/main/docs/api-reference.md)
- **Tool Review Workflow**: [docs/tool_review_workflow.md](https://github.com/thirdkeyai/symbiont/blob/main/docs/tool_review_workflow.md)
- **Getting Started**: [docs/getting-started.md](https://github.com/thirdkeyai/symbiont/blob/main/docs/getting-started.md)

---

## Pro Tips for AI Assistants

1. **Always Start with Security**: Design policies before implementation
2. **Use Example Agents**: Adapt from the 8 production agents in `/agents/`
3. **Validate Early**: Add input validation at the beginning
4. **Handle Errors Gracefully**: Wrap risky operations in try/catch
5. **Log Thoughtfully**: Audit what matters, never log secrets
6. **Choose Right Sandbox**: Match tier to threat model
7. **Test Incrementally**: Start simple, add features with tests
8. **Document Assumptions**: Comment complex policy logic
9. **Monitor Resources**: Set realistic limits based on workload
10. **Review Before Deploy**: Use the validation checklist

---

**End of SKILLS.md**

*This guide prioritizes security, compliance, and best practices for building production-grade Symbiont agents.*
