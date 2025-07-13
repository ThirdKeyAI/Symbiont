# Symbiont MCP Management CLI

The `symbiont-mcp` CLI tool provides a secure and user-friendly interface for managing Model Context Protocol (MCP) servers within the Symbiont runtime ecosystem.

## Features

- **GitHub Integration**: Automatically discover MCP servers from GitHub repositories
- **Security-First**: Built-in cryptographic verification using [`SecureMcpClient`](src/integrations/mcp/client.rs:56)
- **Flexible Sources**: Support for GitHub repos, direct URLs, and registry lookups
- **Persistent Configuration**: TOML-based configuration with automatic persistence
- **Schema Validation**: Tool schema verification and security analysis
- **TOFU Security**: Trust-On-First-Use key management with [`SchemaPin`](src/integrations/mcp/types.rs:44)

## Installation

Build the CLI tool from the runtime directory:

```bash
cd runtime
cargo build --bin symbiont-mcp --release
```

The binary will be available at `target/release/symbiont-mcp`.

## Configuration

The CLI uses a TOML configuration file located at `~/.symbiont/mcp-config.toml` by default.

### Configuration Structure

```toml
[settings]
enforce_verification = true
allow_unverified_in_dev = false
verification_timeout_seconds = 30
max_concurrent_verifications = 5
config_dir = "~/.symbiont"
github_token = "optional_github_token"

[servers]
# Server configurations are automatically managed
```

### Environment Variables

- `GITHUB_TOKEN`: GitHub personal access token for private repository access
- `SYMBIONT_CONFIG_DIR`: Override default configuration directory

## Commands

### Add Server

Add a new MCP server from various sources:

```bash
# Add from GitHub repository
symbiont-mcp add https://github.com/owner/repo

# Add with custom name
symbiont-mcp add https://github.com/owner/repo --name my-server

# Skip verification (not recommended)
symbiont-mcp add https://github.com/owner/repo --skip-verification

# Add from direct URL
symbiont-mcp add https://api.example.com/mcp

# Add from registry
symbiont-mcp add registry://server-name
```

### List Servers

Display registered MCP servers:

```bash
# List all servers
symbiont-mcp list

# Show detailed information
symbiont-mcp list --detailed

# Filter by status
symbiont-mcp list --status active
symbiont-mcp list --status error
```

### Check Status

Monitor server health and status:

```bash
# Check all servers
symbiont-mcp status

# Check specific server
symbiont-mcp status --name my-server

# Perform health check
symbiont-mcp status --name my-server --health-check
```

### Verify Server

Manually verify server tools and schemas:

```bash
# Verify specific server
symbiont-mcp verify my-server

# Force re-verification
symbiont-mcp verify my-server --force
```

### Update Server

Modify server configuration:

```bash
# Update server source
symbiont-mcp update my-server --source https://new-url.com/mcp
```

### Remove Server

Remove a registered server:

```bash
# Remove with confirmation
symbiont-mcp remove my-server

# Force removal without confirmation
symbiont-mcp remove my-server --force
```

## Security Model

### Verification Process

1. **Discovery**: Tools are discovered from the MCP server
2. **Schema Validation**: Tool schemas are validated against security policies
3. **Cryptographic Verification**: Tool signatures are verified using provider public keys
4. **Security Analysis**: Tools are analyzed for potential security risks
5. **Trust Decision**: Based on verification results and policy settings

### Trust Levels

- **Verified**: All security checks passed
- **Failed**: Security verification failed
- **Pending**: Verification in progress
- **Skipped**: Verification bypassed by user

### Security Policies

The CLI enforces configurable security policies:

- `enforce_verification`: Require successful verification before activation
- `allow_unverified_in_dev`: Allow unverified tools in development mode
- `verification_timeout_seconds`: Maximum time for verification process
- `max_concurrent_verifications`: Limit concurrent verification operations

## GitHub Integration

### Repository Discovery

The CLI automatically scans GitHub repositories for MCP servers by:

1. Analyzing `package.json` for MCP-related keywords
2. Searching for MCP configuration files
3. Examining README files for MCP documentation
4. Detecting common MCP entry points

### Supported Patterns

- `https://github.com/owner/repo`
- `github.com/owner/repo`
- `owner/repo` (when context is clear)

### Authentication

For private repositories, provide a GitHub token:

```bash
export GITHUB_TOKEN=your_personal_access_token
symbiont-mcp add https://github.com/private/repo
```

## Error Handling

The CLI provides comprehensive error reporting:

- **Connection Errors**: Network connectivity issues
- **Authentication Errors**: Invalid credentials or permissions
- **Verification Errors**: Failed security checks
- **Configuration Errors**: Invalid TOML syntax or missing settings

## Integration with Symbiont Runtime

The CLI integrates seamlessly with the Symbiont runtime through:

1. **Shared Configuration**: Uses same TOML format as runtime
2. **Security Integration**: Leverages [`SecureMcpClient`](src/integrations/mcp/client.rs:56) for verification
3. **Schema Compatibility**: Compatible with runtime's tool invocation system
4. **Policy Enforcement**: Respects runtime security policies

## Development

### Building from Source

```bash
git clone https://github.com/your-org/symbiont
cd symbiont/runtime
cargo build --bin symbiont-mcp
```

### Running Tests

```bash
cargo test --bin symbiont-mcp
```

### Contributing

1. Follow Rust best practices and idioms
2. Ensure all security verification passes
3. Add tests for new functionality
4. Update documentation for API changes

## Architecture

### Components

- **CLI Parser**: [`src/bin/symbiont_mcp.rs`](src/bin/symbiont_mcp.rs) - Command-line interface
- **Command Handlers**: [`src/bin/commands.rs`](src/bin/commands.rs) - Business logic implementation
- **Configuration**: [`src/bin/config.rs`](src/bin/config.rs) - TOML configuration management
- **GitHub Client**: [`src/bin/github.rs`](src/bin/github.rs) - Repository discovery and API interaction
- **Registry**: [`src/bin/registry.rs`](src/bin/registry.rs) - Server persistence and metadata

### Dependencies

- `clap`: Command-line argument parsing
- `octocrab`: GitHub API client
- `tokio`: Async runtime
- `serde`: Serialization framework
- `toml`: Configuration file format
- `anyhow`: Error handling
- `tracing`: Structured logging

## Future Enhancements

- **Natural Language Interface**: Voice and text-based server management
- **Health Monitoring**: Continuous server health monitoring
- **Auto-Discovery**: Automatic MCP server detection in development environments
- **Plugin System**: Extensible verification and discovery plugins
- **Web Interface**: Browser-based management console

## Examples

### Basic Workflow

```bash
# Add a GitHub MCP server
symbiont-mcp add https://github.com/example/mcp-server

# List all servers
symbiont-mcp list --detailed

# Verify server security
symbiont-mcp verify example-mcp-server

# Check server status
symbiont-mcp status --name example-mcp-server --health-check

# Remove server when no longer needed
symbiont-mcp remove example-mcp-server
```

### Enterprise Setup

```bash
# Configure enterprise settings
export GITHUB_TOKEN=enterprise_token
export SYMBIONT_CONFIG_DIR=/etc/symbiont

# Add multiple servers
symbiont-mcp add https://github.com/company/auth-mcp
symbiont-mcp add https://github.com/company/data-mcp
symbiont-mcp add https://github.com/company/workflow-mcp

# Monitor all servers
symbiont-mcp status --health-check
```

## Troubleshooting

### Common Issues

1. **GitHub Rate Limiting**: Use authenticated requests with `GITHUB_TOKEN`
2. **Verification Failures**: Check network connectivity and tool signatures
3. **Configuration Errors**: Validate TOML syntax and file permissions
4. **Permission Denied**: Ensure proper file system permissions for config directory

### Debug Mode

Enable verbose logging for troubleshooting:

```bash
symbiont-mcp --verbose add https://github.com/example/repo
```

### Log Files

Logs are written to `~/.symbiont/logs/mcp-cli.log` by default.