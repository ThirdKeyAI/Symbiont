# Encrypted Model I/O Logging

## Overview

The Symbiont runtime includes a comprehensive encrypted logging system for all model interactions, including prompts, tool calls, outputs, and latency metrics. This system is designed with security-first principles, automatically encrypting sensitive data and providing PII detection and masking capabilities.

## Features

### Security Features
- **AES-256-GCM Encryption**: All sensitive log data is encrypted using industry-standard AES-256-GCM encryption
- **PII/PHI Detection**: Automatic detection and masking of personally identifiable information and protected health information
- **Secure Key Management**: Integration with the existing crypto utilities for secure key generation and storage
- **Data Minimization**: Only necessary data is logged, with configurable retention policies

### Logging Capabilities
- **Model Interactions**: Complete logging of prompts, responses, and metadata
- **Tool Calls**: Detailed logging of tool invocations with arguments and results
- **RAG Queries**: Comprehensive logging of retrieval-augmented generation pipeline
- **Agent Execution**: Tracking of agent task execution and model interactions
- **Performance Metrics**: Latency, token usage, and performance statistics

### Data Protection
- **Encryption at Rest**: All log files are encrypted before being written to disk
- **PII Masking**: Common patterns for SSNs, credit cards, emails, phone numbers, and API keys are automatically masked
- **Configurable Redaction**: Sensitive fields can be completely redacted or masked based on configuration
- **Secure Storage**: Log files are stored with restricted permissions (0o600)

## Configuration

### Basic Configuration

```rust
use symbi_runtime::logging::{LoggingConfig, ModelLogger};
use symbi_runtime::secrets;
use std::sync::Arc;

let config = LoggingConfig {
    enabled: true,
    log_file_path: "logs/model_io.encrypted.log".to_string(),
    encryption_key_name: "symbiont/logging/encryption_key".to_string(),
    encryption_key_env: Some("SYMBIONT_LOGGING_KEY".to_string()),
    max_entry_size: 1024 * 1024, // 1MB
    retention_days: 90,
    enable_pii_masking: true,
    batch_size: 100,
};

// Create secret store for key management
let secret_store = secrets::new_secret_store(&secrets_config, "symbiont-runtime").await?;
let logger = ModelLogger::new(config, Some(Arc::new(secret_store)))?;
```

### Configuration with Vault Backend

```toml
# symbiont.toml
[security.secrets]
type = "vault"
url = "https://vault.example.com"
namespace = "symbiont"
mount_path = "secret"

[security.secrets.auth]
method = "token"
token = "${VAULT_TOKEN}"

[logging]
enabled = true
encryption_key_name = "symbiont/logging/encryption_key"
log_file_path = "logs/model_io.encrypted.log"
enable_pii_masking = true
```

### Configuration with File Backend

```toml
# symbiont.toml
[security.secrets]
type = "file"
path = "/etc/symbiont/secrets.json"

[security.secrets.encryption]
enabled = true
algorithm = "AES-256-GCM"

[security.secrets.encryption.key]
provider = "os_keychain"
service = "symbiont"
account = "secrets"

[logging]
enabled = true
encryption_key_name = "symbiont/logging/encryption_key"
log_file_path = "logs/model_io.encrypted.log"
enable_pii_masking = true
```

### Secret Management Integration

The logging module is integrated with Symbiont's SecretStore system for secure key management:

#### Secret Store Keys

- `symbiont/logging/encryption_key`: Primary encryption key stored in SecretStore (Vault/file backend)
- `SYMBIONT_LOGGING_KEY`: Environment variable fallback for encryption key
- `SYMBIONT_MASTER_KEY`: Master key used by crypto utilities (final fallback)

#### Key Retrieval Priority

1. **SecretStore**: Retrieves key from configured backend (Vault or encrypted file)
2. **Environment Variable**: Falls back to `SYMBIONT_LOGGING_KEY` environment variable
3. **Keychain/Generated**: Uses OS keychain or generates new key as final fallback

### Runtime Integration

The logging system is automatically integrated into the runtime when enabled:

```rust
use symbi_runtime::{AgentRuntime, RuntimeConfig};

let mut runtime_config = RuntimeConfig::default();
runtime_config.logging.enabled = true;
runtime_config.logging.enable_pii_masking = true;

let runtime = AgentRuntime::new(runtime_config).await?;
```

## Usage Examples

### Manual Logging

```rust
use symbi_runtime::logging::{ModelLogger, ModelInteractionType, RequestData, ResponseData, TokenUsage};
use symbi_runtime::secrets;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

// Create logger with SecretStore integration
let secret_store = secrets::new_secret_store(&secrets_config, "symbiont-runtime").await?;
let logger = ModelLogger::new(LoggingConfig::default(), Some(Arc::new(secret_store)))?;

// Or use defaults (no SecretStore, falls back to environment variables)
let logger = ModelLogger::with_defaults()?;

// Log a complete interaction
let request_data = RequestData {
    prompt: "What is the weather like?".to_string(),
    tool_name: None,
    tool_arguments: None,
    parameters: HashMap::new(),
};

let response_data = ResponseData {
    content: "The weather is sunny today.".to_string(),
    tool_result: None,
    confidence: Some(0.95),
    metadata: HashMap::new(),
};

let token_usage = TokenUsage {
    input_tokens: 10,
    output_tokens: 15,
    total_tokens: 25,
};

logger.log_interaction(
    agent_id,
    ModelInteractionType::Completion,
    "gpt-4",
    request_data,
    response_data,
    Duration::from_millis(150),
    HashMap::new(),
    Some(token_usage),
    None,
).await?;
```

### Asynchronous Request/Response Logging

```rust
// Log request first
let entry_id = logger.log_request(
    agent_id,
    ModelInteractionType::ToolCall,
    "calculator-tool",
    request_data,
    metadata,
).await?;

// ... perform actual model call ...

// Log response when available
logger.log_response(
    &entry_id,
    response_data,
    latency,
    Some(token_usage),
    None, // No error
).await?;
```

## Log Format

### Encrypted Log Entry Structure

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "agent_id": "agent_123",
  "interaction_type": "Completion",
  "timestamp": "2024-01-15T10:30:00Z",
  "latency_ms": 150,
  "model_identifier": "gpt-4",
  "request_data": {
    "ciphertext": "encrypted_base64_data",
    "nonce": "random_nonce_base64",
    "salt": "random_salt_base64",
    "algorithm": "AES-256-GCM",
    "kdf": "Argon2"
  },
  "response_data": {
    "ciphertext": "encrypted_base64_data",
    "nonce": "random_nonce_base64",
    "salt": "random_salt_base64",
    "algorithm": "AES-256-GCM",
    "kdf": "Argon2"
  },
  "metadata": {
    "rag_pipeline": "generate_response",
    "documents_retrieved": "3"
  },
  "error": null,
  "token_usage": {
    "input_tokens": 10,
    "output_tokens": 15,
    "total_tokens": 25
  }
}
```

### Raw Data Structure (Decrypted)

#### Request Data
```json
{
  "prompt": "What is the weather like?",
  "tool_name": "weather_api",
  "tool_arguments": {
    "location": "San Francisco",
    "units": "metric"
  },
  "parameters": {
    "temperature": 0.7,
    "max_tokens": 100
  }
}
```

#### Response Data
```json
{
  "content": "The weather in San Francisco is currently 18°C and sunny.",
  "tool_result": {
    "temperature": 18,
    "condition": "sunny",
    "humidity": 65
  },
  "confidence": 0.95,
  "metadata": {
    "model_version": "gpt-4-turbo",
    "processing_time_ms": 150
  }
}
```

## PII Detection and Masking

### Supported Patterns

The logging system automatically detects and masks the following patterns:

- **Social Security Numbers**: `123-45-6789` → `***-**-****`
- **Credit Card Numbers**: `4532-1234-5678-9012` → `****-****-****-****`
- **Email Addresses**: `user@example.com` → `***@***.***`
- **Phone Numbers**: `555-123-4567` → `***-***-****`
- **API Keys**: `API_KEY=abcd1234...` → `API_KEY=***`
- **Tokens**: `TOKEN=xyz789...` → `TOKEN=***`

### Sensitive Key Detection

Fields with the following names are automatically masked:
- `password`, `token`, `key`, `secret`, `credential`
- `api_key`, `auth`, `authorization`
- `ssn`, `social_security`, `credit_card`, `card_number`, `cvv`, `pin`

### Custom PII Masking

```rust
impl ModelLogger {
    // Custom masking can be implemented by extending the mask_sensitive_patterns method
    fn mask_sensitive_patterns(&self, text: &str) -> String {
        // Add custom patterns here
        // ...
    }
}
```

## Security Considerations

### Encryption Key Management

1. **SecretStore Integration**: Primary key storage uses Symbiont's SecretStore system
   - **Vault Backend**: Keys stored in HashiCorp Vault with proper access controls
   - **File Backend**: Keys stored in encrypted files with OS keychain integration
2. **Key Generation**: Uses cryptographically secure random number generation
3. **Multi-tier Fallback**: SecretStore → Environment Variable → OS Keychain → Generated
4. **Key Rotation**: Can be implemented through SecretStore rotation policies

### Access Control

1. **File Permissions**: Log files are created with 0o600 permissions (owner read/write only)
2. **Process Isolation**: Logging runs in the same process as the runtime for performance
3. **Memory Safety**: Rust's memory safety guarantees protect against buffer overflows

### Data Retention

1. **Configurable Retention**: Set `retention_days` in configuration
2. **Manual Cleanup**: Implement log rotation and cleanup based on retention policy
3. **Secure Deletion**: Ensure old log files are securely deleted

## Integration Points

### RAG Engine Integration

```rust
// Automatic logging in RAG pipeline
impl StandardRAGEngine {
    pub fn with_logger(context_manager: Arc<dyn ContextManager>, logger: Arc<ModelLogger>) -> Self {
        // Logger is automatically used in generate_response method
    }
}
```

### Tool Invocation Integration

```rust
// Automatic logging in tool invocation enforcement
impl DefaultToolInvocationEnforcer {
    pub fn with_logger(config: InvocationEnforcementConfig, logger: Arc<ModelLogger>) -> Self {
        // Logger is automatically used in execute_tool_with_enforcement method
    }
}
```

### Runtime Integration

The logger is automatically initialized and shared across all components when enabled in the runtime configuration.

## Performance Considerations

### Batching

- Log entries can be batched for improved I/O performance
- Configure `batch_size` in `LoggingConfig` to control batching behavior

### Async Operations

- All logging operations are asynchronous to avoid blocking the main execution path
- Failed logging operations are logged as warnings but don't fail the main operation

### Memory Usage

- Log entries are encrypted immediately and not kept in memory
- Large payloads are handled efficiently with streaming encryption

## Troubleshooting

### Common Issues

1. **Encryption Key Not Found**
   ```
   Error: KeyManagementError { message: "Failed to retrieve logging encryption key from SecretStore" }
   ```
   Solution: Ensure the encryption key exists in SecretStore at the configured path

2. **SecretStore Connection Failed**
   ```
   Warning: Failed to initialize secret store for logging: ConnectionError { message: "..." }
   ```
   Solution: Check SecretStore configuration (Vault URL, authentication, etc.)

3. **Permission Denied**
   ```
   Error: IoError { source: "Permission denied (os error 13)" }
   ```
   Solution: Ensure the log directory is writable and has correct permissions

4. **Logging Disabled**
   ```
   Warning: Model logging is disabled
   ```
   Solution: Set `logging.enabled = true` in runtime configuration

### Debug Mode

Enable debug logging to troubleshoot issues:

```rust
use tracing::{info, warn, debug, error};

// Debug logs will show:
debug!("Logged model request {} for agent {}", entry_id, agent_id);
debug!("Logged model response for entry {}", entry_id);
warn!("Failed to log tool invocation: {}", error);
```

## Future Enhancements

### Planned Features

1. **Log Analytics**: Built-in analytics for model usage patterns
2. **Real-time Monitoring**: Integration with monitoring systems
3. **Audit Trails**: Enhanced audit capabilities for compliance
4. **Key Rotation**: Automated encryption key rotation
5. **Compression**: Log compression for long-term storage
6. **Search Capabilities**: Encrypted search over historical logs

### Extensibility

The logging system is designed to be extensible:

- Custom PII detection patterns can be added
- Additional metadata fields can be included
- Storage backends can be plugged in (currently file-based)
- Encryption algorithms can be upgraded (currently AES-256-GCM)

## Compliance and Auditing

### Audit Capabilities

- All model interactions are logged with timestamps and agent identifiers
- Immutable log entries provide audit trails
- Encrypted storage ensures data integrity and confidentiality

### Compliance Considerations

- **GDPR**: PII masking helps with data minimization requirements
- **HIPAA**: PHI detection and encryption support healthcare compliance
- **SOC 2**: Comprehensive logging supports security and availability controls
- **ISO 27001**: Encryption and access controls align with information security standards

### Data Subject Rights

For GDPR compliance, consider implementing:
- Log entry identification by data subject
- Secure deletion of specific log entries upon request
- Data export capabilities for transparency