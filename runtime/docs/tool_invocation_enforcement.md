# Tool Invocation Enforcement

This document describes the tool invocation enforcement system that ensures only verified tools can be executed based on configurable policies.

## Overview

The Tool Invocation Enforcement system provides a security layer that validates tool verification status before allowing execution. It implements configurable policies to control how strictly verification requirements are enforced.

## Architecture

### Core Components

- **[`ToolInvocationEnforcer`](../src/integrations/tool_invocation.rs:82)**: Main trait defining enforcement operations
- **[`DefaultToolInvocationEnforcer`](../src/integrations/tool_invocation.rs:174)**: Default implementation with configurable policies
- **[`EnforcementPolicy`](../src/integrations/tool_invocation.rs:43)**: Defines enforcement strictness levels
- **[`InvocationEnforcementConfig`](../src/integrations/tool_invocation.rs:58)**: Configuration for enforcement behavior

### Enforcement Policies

#### Strict Mode (Default)
- **Purpose**: Maximum security - only verified tools are allowed
- **Behavior**: 
  - ✅ Allow: Verified tools only
  - ❌ Block: Failed, pending, and skipped verification
- **Use Case**: Production environments requiring high security

#### Permissive Mode
- **Purpose**: Balanced security with operational flexibility
- **Behavior**:
  - ✅ Allow: Verified tools
  - ⚠️ Allow with warnings: Pending verification (configurable)
  - ❌ Block: Failed verification (configurable)
  - ⚠️ Allow with warnings: Skipped verification
- **Use Case**: Development/staging environments

#### Development Mode
- **Purpose**: Maximum flexibility for development
- **Behavior**:
  - ✅ Allow: Verified tools
  - ⚠️ Allow with warnings: Pending verification
  - ❌ Block: Failed verification (configurable)
  - ⚠️ Allow with warnings: Skipped verification (if enabled)
- **Use Case**: Development environments with frequent tool changes

#### Disabled Mode
- **Purpose**: No enforcement (for testing/emergency scenarios)
- **Behavior**: Allow all tools regardless of verification status
- **Use Case**: Emergency scenarios or testing

## Configuration

### Default Configuration

```rust
InvocationEnforcementConfig {
    policy: EnforcementPolicy::Strict,
    block_failed_verification: true,
    block_pending_verification: true,
    allow_skipped_in_dev: false,
    verification_timeout: Duration::from_secs(5),
    max_warnings_before_escalation: 10,
}
```

### Configuration Options

| Field | Type | Description | Default |
|-------|------|-------------|---------|
| `policy` | `EnforcementPolicy` | Primary enforcement policy | `Strict` |
| `block_failed_verification` | `bool` | Whether to block tools with failed verification | `true` |
| `block_pending_verification` | `bool` | Whether to block tools with pending verification | `true` |
| `allow_skipped_in_dev` | `bool` | Allow skipped verification in development mode | `false` |
| `verification_timeout` | `Duration` | Timeout for verification checks | `5 seconds` |
| `max_warnings_before_escalation` | `usize` | Max warnings before escalation logging | `10` |

## Integration with MCP Client

The enforcement system is integrated with the MCP client through the [`invoke_tool`](../src/integrations/mcp/client.rs:307) method:

```rust
async fn invoke_tool(
    &self,
    tool_name: &str,
    arguments: serde_json::Value,
    context: InvocationContext,
) -> Result<InvocationResult, McpClientError>
```

### Integration Flow

1. **Tool Retrieval**: Get tool from MCP client registry
2. **Enforcement Check**: Validate against current enforcement policy
3. **Decision Processing**: 
   - Allow: Execute tool normally
   - Block: Return error with clear message
   - Allow with warnings: Execute with warning logging
4. **Result Processing**: Return execution result with any warnings

## Error Messages

The system provides clear, actionable error messages:

### Tool Invocation Blocked
```
Tool invocation blocked: {tool_name} - {reason}
```
- **When**: Tool execution is blocked by policy
- **Action**: Verify the tool or adjust enforcement policy

### Verification Required
```
Verification required but tool is not verified: {tool_name} (status: {status})
```
- **When**: Strict mode blocks unverified tool
- **Action**: Complete tool verification process

### Verification Failed
```
Tool verification failed: {tool_name} - {reason}
```
- **When**: Tool verification explicitly failed
- **Action**: Review and fix tool verification issues

## Usage Examples

### Basic Usage

```rust
use symbiont_runtime::integrations::{
    DefaultToolInvocationEnforcer, EnforcementPolicy, 
    InvocationEnforcementConfig, InvocationContext
};

// Create enforcer with strict policy
let config = InvocationEnforcementConfig {
    policy: EnforcementPolicy::Strict,
    ..Default::default()
};
let enforcer = DefaultToolInvocationEnforcer::with_config(config);

// Check if tool invocation is allowed
let context = InvocationContext {
    agent_id: AgentId::new(),
    tool_name: "example_tool".to_string(),
    arguments: serde_json::json!({"param": "value"}),
    timestamp: chrono::Utc::now(),
    metadata: HashMap::new(),
};

let decision = enforcer.check_invocation_allowed(&tool, &context).await?;
```

### Custom Enforcement Policy

```rust
// Development environment configuration
let dev_config = InvocationEnforcementConfig {
    policy: EnforcementPolicy::Development,
    allow_skipped_in_dev: true,
    block_failed_verification: false,
    max_warnings_before_escalation: 5,
    ..Default::default()
};

let enforcer = DefaultToolInvocationEnforcer::with_config(dev_config);
```

### MCP Client Integration

```rust
use symbiont_runtime::integrations::{SecureMcpClient, InvocationContext};

let client = SecureMcpClient::with_defaults(config)?;

// Tool invocation with automatic enforcement
let context = InvocationContext { /* ... */ };
let result = client.invoke_tool(
    "verified_tool", 
    serde_json::json!({"input": "data"}),
    context
).await?;
```

## Monitoring and Observability

### Warning System

The enforcement system includes a warning escalation mechanism:

1. **Warning Tracking**: Counts warnings per tool
2. **Escalation**: When warning threshold is exceeded, escalated logging occurs
3. **Reset**: Warning count resets after escalation

### Logging

All enforcement decisions are logged with appropriate levels:
- **INFO**: Successful tool invocations
- **WARN**: Tools allowed with warnings
- **ERROR**: Blocked tool invocations

## Security Considerations

### Trust Model

The enforcement system operates on a "Trust but Verify" model:
- Tools must be explicitly verified to be trusted
- Verification status is checked at invocation time
- Failed verification is treated as untrusted

### Attack Mitigation

The system helps mitigate several attack vectors:
- **Malicious Tools**: Only verified tools can execute in strict mode
- **Tool Substitution**: Verification prevents unauthorized tool replacement
- **Privilege Escalation**: Unverified tools cannot execute sensitive operations

### Configuration Security

- Default configuration favors security (strict mode)
- Permissive modes require explicit configuration
- Disabled mode should only be used in controlled environments

## Best Practices

### Production Environments
- Use **Strict** enforcement policy
- Enable all blocking options
- Set appropriate timeout values
- Monitor warning escalations

### Development Environments
- Use **Development** or **Permissive** mode
- Allow skipped verification for internal tools
- Lower warning escalation thresholds
- Regular verification audits

### Emergency Procedures
- **Disabled** mode for emergency tool access
- Document all policy changes
- Restore strict mode after emergency resolution

## Testing

Comprehensive tests cover all enforcement scenarios:
- All policy modes (Strict, Permissive, Development, Disabled)
- All verification states (Verified, Failed, Pending, Skipped)
- Error message clarity and accuracy
- Warning escalation behavior
- MCP client integration

See [`tool_invocation_tests.rs`](../tests/tool_invocation_tests.rs) for detailed test cases.

## Future Enhancements

### Planned Features
- **Time-based Policies**: Verification expiration and renewal
- **Risk-based Enforcement**: Different policies based on tool risk level
- **Audit Integration**: Enhanced logging and audit trail integration
- **Dynamic Policy Updates**: Runtime policy configuration changes

### Extension Points
- Custom enforcement policies via trait implementation
- Pluggable warning handlers
- Integration with external policy engines
- Tool-specific enforcement overrides