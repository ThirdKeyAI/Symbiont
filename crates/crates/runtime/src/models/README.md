# Symbiont Models Module

This module provides infrastructure for managing and executing Small Language Models (SLMs) within the Symbiont runtime environment. It implements a secure, sandboxed execution framework that respects resource constraints and security policies.

## Architecture Overview

The models module consists of three main components:

### 1. ModelCatalog (`catalog.rs`)

The `ModelCatalog` serves as a central registry for all available models in the system. It provides:

- **Model Discovery**: Efficient lookup of models by ID, capability, or agent assignment
- **Configuration Management**: Loads and validates model definitions from runtime configuration
- **Access Control**: Manages agent-to-model mappings and enforces access policies
- **Resource Planning**: Provides resource requirement information for deployment decisions

Key features:
- Thread-safe, immutable catalog design
- Support for agent-specific model restrictions
- Capability-based model filtering
- Resource-aware model selection algorithms

### 2. SlmRunner Trait (`runners.rs`)

The `SlmRunner` trait defines a standardized interface for executing models while respecting security constraints:

```rust
#[async_trait]
pub trait SlmRunner: Send + Sync {
    async fn execute(&self, prompt: &str, options: Option<ExecutionOptions>) -> Result<ExecutionResult, SlmRunnerError>;
    fn get_sandbox_profile(&self) -> &SandboxProfile;
    fn get_resource_requirements(&self) -> &ModelResourceRequirements;
    async fn health_check(&self) -> Result<(), SlmRunnerError>;
    fn get_info(&self) -> RunnerInfo;
}
```

### 3. LocalGgufRunner Implementation

A concrete implementation of `SlmRunner` for GGUF-quantized models using llama.cpp:

- **Security**: Enforces sandbox profile constraints (memory, CPU, filesystem, network)
- **Resource Management**: Respects configured resource limits
- **Process Isolation**: Uses system-level process controls for security
- **Monitoring**: Tracks execution metrics and resource usage

## Security Model

All model execution operates within strict security boundaries defined by `SandboxProfile`:

### Resource Constraints
- **Memory Limits**: Maximum memory allocation per execution
- **CPU Limits**: CPU core allocation and priority settings
- **Disk Limits**: Temporary storage restrictions
- **Execution Time**: Maximum runtime per request

### Filesystem Controls
- **Read Paths**: Restricted file system read access
- **Write Paths**: Limited write access to designated areas
- **Denied Paths**: Explicitly blocked system directories
- **Temporary Files**: Controlled temporary file creation

### Network Policy
- **Access Modes**: None, Restricted, or Full network access
- **Destination Filtering**: Allowed hosts and ports
- **Bandwidth Limits**: Network throughput restrictions

### Process Security
- **Syscall Filtering**: Limited system call access via seccomp
- **Child Processes**: Controlled subprocess creation
- **Priority Management**: Process scheduling constraints

## Configuration Integration

The models module integrates with Symbiont's configuration system through the `Slm` configuration struct:

```toml
[slm]
enabled = true
default_sandbox_profile = "secure"

# Global model definitions
[[slm.model_allow_lists.global_models]]
id = "llama2-7b"
name = "Llama 2 7B"
provider = { LocalFile = { file_path = "/models/llama2-7b.gguf" } }
capabilities = ["TextGeneration", "Reasoning"]

[slm.model_allow_lists.global_models.resource_requirements]
min_memory_mb = 8192
preferred_cpu_cores = 4.0

# Agent-specific model mappings
[slm.model_allow_lists.agent_model_maps]
"security_scanner" = ["llama2-7b"]
"code_generator" = ["codellama-7b"]

# Sandbox profiles
[slm.sandbox_profiles.secure]
[slm.sandbox_profiles.secure.resources]
max_memory_mb = 4096
max_cpu_cores = 2.0
```

## Usage Examples

### Basic Model Execution

```rust
use symbiont_runtime::models::{ModelCatalog, LocalGgufRunner, ExecutionOptions};
use symbiont_runtime::config::Slm;

// Initialize catalog from configuration
let slm_config = Slm::default();
let catalog = ModelCatalog::new(slm_config)?;

// Get model and create runner
let model = catalog.get_model("llama2-7b").unwrap();
let sandbox_profile = catalog.get_default_sandbox_profile().unwrap();

let runner = LocalGgufRunner::new(
    "/models/llama2-7b.gguf",
    sandbox_profile.clone(),
    model.resource_requirements.clone(),
).await?;

// Execute with custom options
let options = ExecutionOptions {
    timeout: Some(Duration::from_secs(30)),
    temperature: Some(0.7),
    max_tokens: Some(256),
    ..Default::default()
};

let result = runner.execute("Explain quantum computing", Some(options)).await?;
println!("Response: {}", result.response);
```

### Agent-Specific Model Access

```rust
// Get models allowed for a specific agent
let agent_models = catalog.get_models_for_agent("security_scanner");
for model in agent_models {
    println!("Agent can use: {} ({})", model.name, model.id);
}

// Validate model access
catalog.validate_model_access("llama2-7b", "security_scanner")?;
```

### Capability-Based Model Selection

```rust
use symbiont_runtime::config::ModelCapability;

// Find models with specific capabilities
let code_models = catalog.get_models_with_capability(&ModelCapability::CodeGeneration);

// Find best model for requirements
let best_model = catalog.find_best_model_for_requirements(
    &[ModelCapability::TextGeneration, ModelCapability::Reasoning],
    Some(4096), // Max 4GB memory
    Some("security_scanner"), // For specific agent
);
```

## Adding New Model Runners

To implement support for a new model format:

1. **Implement the SlmRunner trait**:
```rust
pub struct MyCustomRunner {
    model_path: PathBuf,
    sandbox_profile: SandboxProfile,
    resource_requirements: ModelResourceRequirements,
}

#[async_trait]
impl SlmRunner for MyCustomRunner {
    async fn execute(&self, prompt: &str, options: Option<ExecutionOptions>) -> Result<ExecutionResult, SlmRunnerError> {
        // Validate constraints
        self.validate_execution_constraints(prompt)?;
        
        // Apply sandbox restrictions
        // Execute model with security controls
        // Return results with metadata
    }
    
    // Implement other required methods...
}
```

2. **Ensure proper sandbox enforcement**:
   - Validate resource limits before execution
   - Apply filesystem and network restrictions
   - Monitor execution time and resource usage
   - Handle security violations appropriately

3. **Add comprehensive error handling**:
   - Resource limit violations
   - Sandbox policy violations
   - Model execution failures
   - I/O and timeout errors

4. **Include thorough testing**:
   - Unit tests for security constraint validation
   - Integration tests with various sandbox profiles
   - Error condition testing
   - Resource limit testing

## Error Handling

The module provides comprehensive error types:

- `ModelCatalogError`: Configuration and catalog operation errors
- `SlmRunnerError`: Model execution and security violation errors

All errors include detailed context for debugging and monitoring.

## Performance Considerations

- **Catalog Caching**: Model definitions are cached for efficient lookup
- **Resource Pooling**: Consider implementing model instance pooling for high-throughput scenarios
- **Lazy Loading**: Models are loaded on-demand to minimize startup time
- **Metrics Collection**: Built-in execution metadata for performance monitoring

## Security Best Practices

1. **Always validate inputs** before model execution
2. **Respect sandbox profiles** - never bypass security constraints
3. **Monitor resource usage** during execution
4. **Log security events** for audit trails
5. **Use least-privilege access** for model file permissions
6. **Validate model files** before loading
7. **Implement timeouts** for all operations
8. **Handle failures gracefully** without exposing system information

## Future Enhancements

- **Remote Model Support**: Integration with cloud-based model APIs
- **Model Versioning**: Support for model version management
- **Dynamic Scaling**: Auto-scaling based on demand and resources
- **Advanced Monitoring**: Detailed performance and security metrics
- **Model Caching**: Intelligent model loading and memory management
- **Distributed Execution**: Support for model execution across multiple nodes