# Unit Testing Guide for SLM-First Features

This document provides comprehensive documentation for the unit test implementation of Symbiont's SLM-first architecture components.

## Overview

The unit test suite provides comprehensive coverage for all core SLM-first functionality, including configuration management, encrypted logging, model catalog operations, routing policy evaluation, confidence monitoring, and the routing engine. The tests follow Rust best practices and include extensive mock implementations for dependency isolation.

## Test Coverage by Module

### 1. Configuration Module (`crates/runtime/src/config.rs`)

**Coverage Areas:**
- SLM configuration validation and error handling
- Sandbox profile management and security settings
- Model configuration validation including duplicate ID detection
- Network policies and GPU requirements testing
- Environment variable handling and edge cases

**Key Test Functions:**
- `test_slm_config_validation_*` - Various configuration validation scenarios
- `test_sandbox_profile_*` - Sandbox security and resource management
- `test_model_configuration_*` - Model setup and validation
- `test_network_policy_*` - Network access control testing

**Mock Components:**
- Test configuration builders for various scenarios
- Mock GPU requirements and resource constraints

### 2. Encrypted Logging Module (`crates/runtime/src/logging.rs`)

**Coverage Areas:**
- PII/PHI masking for sensitive data (SSN, credit cards, API keys)
- AES-256-GCM encryption/decryption workflows
- Secret store integration and fallback mechanisms
- Error handling and logging workflow validation

**Key Test Functions:**
- `test_pii_masking_*` - Comprehensive PII pattern detection and masking
- `test_encryption_*` - Encryption/decryption roundtrip validation
- `test_secret_store_*` - Secret management and retrieval
- `test_error_scenarios_*` - Error handling and recovery

**Mock Components:**
- `MockSecretStore` - Isolated secret management testing
- Test encryption keys and sample sensitive data

### 3. Model Catalog Module (`crates/runtime/src/models/catalog.rs`)

**Coverage Areas:**
- Model management and agent-specific access control
- Capability filtering and resource constraint validation
- Model selection algorithms and "best model" logic
- Agent model mapping and runtime override testing

**Key Test Functions:**
- `test_model_management_*` - Model CRUD operations
- `test_agent_access_*` - Agent-specific model restrictions
- `test_capability_filtering_*` - Model capability matching
- `test_resource_constraints_*` - Resource requirement validation

**Mock Components:**
- Test model configurations with various capabilities
- Mock resource constraints and agent mappings

### 4. Routing Policy Evaluation (`crates/runtime/src/routing/policy.rs`)

**Coverage Areas:**
- Policy rule evaluation with different model preferences
- Agent restriction testing and resource constraint validation
- Rule priority ordering and fallback scenarios
- Task classification integration and error handling

**Key Test Functions:**
- `test_policy_evaluation_*` - Policy rule matching and evaluation
- `test_model_preference_*` - BestAvailable, Specialist, and Specific preferences
- `test_agent_restrictions_*` - Agent-specific routing rules
- `test_resource_constraints_*` - Resource-based routing decisions

**Mock Components:**
- Test routing policies and rules
- Mock task classifiers and model catalogs

### 5. Confidence Monitor (`crates/runtime/src/routing/confidence.rs`)

**Coverage Areas:**
- Multi-factor confidence evaluation (length, coherence, task validation)
- Task-specific quality assessment (code, extraction, translation, QA)
- Configurable heuristic thresholds and critical factor handling
- Recommendation engine testing and adaptive thresholds

**Key Test Functions:**
- `test_confidence_evaluation_*` - Comprehensive confidence scoring
- `test_task_specific_*` - Task-specific quality assessment
- `test_factor_aggregation_*` - Factor weighting and scoring
- `test_recommendation_*` - Action recommendation logic

**Mock Components:**
- Test model responses with various quality indicators
- Mock confidence configurations and thresholds

### 6. Routing Engine (`crates/runtime/src/routing/engine.rs`)

**Coverage Areas:**
- Complete routing workflow testing (SLM → LLM fallback)
- Confidence monitoring integration and statistics tracking
- Error handling and recovery scenarios
- Concurrent request testing and state consistency

**Key Test Functions:**
- `test_routing_engine_*` - End-to-end routing workflows
- `test_slm_execution_*` - SLM execution and monitoring
- `test_llm_fallback_*` - LLM fallback mechanisms
- `test_concurrent_*` - Concurrent request handling

**Mock Components:**
- `MockLLMClient` - LLM provider simulation
- Mock routing configurations and model catalogs

## Testing Patterns and Best Practices

### 1. Mock Pattern Implementation

**Purpose:** Isolate units under test from external dependencies

**Examples:**
- `MockSecretStore` for encrypted logging tests
- `MockLLMClient` for routing engine tests
- Test model catalogs with predefined configurations

### 2. Error Scenario Testing

**Coverage:**
- Invalid input validation
- Network failure simulation
- Resource constraint violations
- Configuration validation errors

**Pattern:**
```rust
#[tokio::test]
async fn test_error_scenario() {
    let result = function_under_test(invalid_input).await;
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), ExpectedErrorType { .. }));
}
```

### 3. Edge Case Testing

**Areas:**
- Empty inputs and boundary values
- Maximum and minimum resource constraints
- Concurrent access patterns
- Configuration edge cases

### 4. Integration Testing

**Approach:**
- Cross-module interaction validation
- End-to-end workflow testing
- Component integration verification

## Running Tests

### Basic Test Execution
```bash
# Run all tests
cargo test

# Run specific module tests
cargo test config
cargo test logging
cargo test catalog
cargo test policy
cargo test confidence
cargo test engine

# Run with output
cargo test -- --nocapture
```

### Quality Assurance
```bash
# Run clippy for code quality
cargo clippy --all-targets --all-features -- -D warnings

# Compile tests without running
cargo test --no-run

# Run tests with coverage (requires cargo-tarpaulin)
cargo tarpaulin --out Html
```

### Test Configuration

**Environment Variables:**
- `RUST_LOG=trace` - Enable detailed logging during tests
- `TEST_THREADS=1` - Run tests sequentially if needed

**Test-Specific Configuration:**
- Mock configurations use smaller limits (e.g., 100 history entries vs 10000)
- Test timeouts are shorter for faster execution
- Mock delays simulate real-world scenarios

## Mock Implementation Details

### MockSecretStore
**Purpose:** Isolate secret management testing from external secret stores

**Features:**
- Predefined test secrets
- Configurable failure scenarios
- Encryption key simulation

**Usage:**
```rust
let mock_store = MockSecretStore::new();
mock_store.set_secret("encryption_key", "test_key_value");
```

### MockLLMClient
**Purpose:** Simulate LLM provider responses without external API calls

**Features:**
- Configurable response generation
- Provider-specific behavior simulation
- Error scenario injection

**Usage:**
```rust
let client = MockLLMClient::new();
let response = client.execute_request(&request, &provider).await?;
```

## Test Data Management

### Configuration Test Data
- Predefined model configurations for various scenarios
- Sandbox profiles with different security levels
- Network policies and resource constraints

### Sample Data Sets
- PII/PHI patterns for masking tests
- Model responses with varying quality indicators
- Routing contexts for different task types

## Continuous Integration

### Pre-commit Checks
1. `cargo clippy` - Code quality and style
2. `cargo test --no-run` - Compilation verification
3. `cargo test` - Full test suite execution

### Test Environment Setup
- No external dependencies required for unit tests
- All mocks are self-contained
- Tests can run in parallel safely

## Troubleshooting

### Common Issues

**Compilation Errors:**
- Ensure all dependencies are properly imported
- Check for missing test feature flags
- Verify mock implementations match trait signatures

**Test Failures:**
- Check for timing-dependent assertions
- Verify mock configurations match test expectations
- Ensure test isolation (no shared state)

**Performance Issues:**
- Use `cargo test --release` for faster execution
- Consider test parallelization limits
- Profile slow tests with `cargo flamegraph`

## Future Enhancements

### Planned Improvements
- Property-based testing integration
- Benchmark testing for performance validation
- Integration test suite for end-to-end scenarios
- Automated test coverage reporting

### Testing Framework Extensions
- Custom assertion macros for domain-specific validations
- Test fixture management for complex scenarios
- Automated mock generation from traits

## Related Documentation

- [SLM Configuration Design](slm_config_design.md)
- [Router Implementation Summary](router_implementation_summary.md)
- [Logging Documentation](../crates/runtime/docs/logging.md)
- [Model Catalog README](../crates/runtime/src/models/README.md)