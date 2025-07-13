# OpenRouter Git Agent Example

A comprehensive example demonstrating how to build intelligent AI agents using the Symbiont Agent Runtime System with OpenRouter API integration and Git repository analysis capabilities.

## Overview

This example showcases the integration of multiple Symbiont runtime components to create a sophisticated AI agent capable of:

- **Repository Analysis**: Clone and analyze Git repositories with intelligent code understanding
- **AI-Powered Insights**: Use OpenRouter's language models for code analysis, security review, and documentation generation
- **Persistent Context**: Maintain agent memory and knowledge across sessions
- **Vector Search**: Leverage Qdrant for semantic search and retrieval-augmented generation (RAG)
- **Security**: Implement cryptographic verification and policy-based access control
- **Multiple Scenarios**: Run different types of analysis workflows

## Features

### Core Capabilities
- 🧠 **Intelligent Code Analysis** - Deep understanding of codebases using AI
- 🔍 **Semantic Search** - Vector-based knowledge retrieval with <500ms performance
- 🛡️ **Security Review** - Automated vulnerability detection and policy enforcement
- 📚 **Documentation Generation** - AI-powered documentation creation
- 💾 **Persistent Context** - Agent memory that survives restarts
- 🔐 **Cryptographic Security** - SchemaPin verification and TOFU protection

### Test Scenarios
1. **Code Analysis Scenario** - Comprehensive codebase analysis and quality assessment
2. **Security Review Scenario** - Automated security vulnerability scanning
3. **Documentation Scenario** - Generate comprehensive project documentation

## Architecture

```
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│   OpenRouter    │    │   Git Tools     │    │  Symbiont       │
│   API Client    │    │   Integration   │    │  Runtime        │
└─────────────────┘    └─────────────────┘    └─────────────────┘
        │                       │                       │
        └───────────────────────┼───────────────────────┘
                                │
                    ┌─────────────────┐
                    │  Agent Core     │
                    │  - Context Mgr  │
                    │  - RAG Engine   │
                    │  - Vector DB    │
                    │  - Security     │
                    └─────────────────┘
```

## Prerequisites

### Required Services
1. **OpenRouter API Key** - Get from [openrouter.ai](https://openrouter.ai/keys)
2. **Qdrant Vector Database** - For semantic search capabilities
3. **Git** - For repository cloning and analysis

### Optional Security Components
1. **SchemaPin CLI** - For cryptographic tool verification
2. **Security Policies** - YAML-based access control

## Setup

### 1. Install Dependencies

```bash
# Navigate to the example directory
cd runtime/examples/openrouter_git_agent

# Build the project
cargo build --release
```

### 2. Start Qdrant Vector Database

#### Using Docker:
```bash
docker run -p 6333:6333 qdrant/qdrant
```

#### Using Docker Compose:
```bash
# Create docker-compose.yml
cat > docker-compose.yml << EOF
version: '3.8'
services:
  qdrant:
    image: qdrant/qdrant
    ports:
      - "6333:6333"
    volumes:
      - ./qdrant_storage:/qdrant/storage
EOF

docker-compose up -d
```

### 3. Configure the Agent

Copy and customize the configuration file:

```bash
cp config.toml config.local.toml
```

Edit `config.local.toml` and set your OpenRouter API key:

```toml
[openrouter]
api_key = "your_actual_openrouter_api_key_here"
```

### 4. Optional: Enable Security Features

If you want to use SchemaPin cryptographic verification:

```bash
# Install SchemaPin CLI (example for Linux)
wget https://github.com/your-org/schemapin/releases/latest/download/schemapin-cli-linux
sudo mv schemapin-cli-linux /usr/local/bin/schemapin-cli
sudo chmod +x /usr/local/bin/schemapin-cli

# Enable in config
[security]
enable_schemapin = true
policy_file = "./security_policies.yaml"
```

## Usage

### Command Line Interface

The agent provides several subcommands for different operations:

```bash
# Analyze a repository with all scenarios
./target/release/openrouter_git_agent analyze-repo \
  --repo https://github.com/rust-lang/mdBook \
  --config config.local.toml

# Run individual scenarios
./target/release/openrouter_git_agent code-analysis \
  --repo https://github.com/rust-lang/mdBook \
  --config config.local.toml

./target/release/openrouter_git_agent security-review \
  --repo https://github.com/rust-lang/mdBook \
  --config config.local.toml

./target/release/openrouter_git_agent generate-docs \
  --repo https://github.com/rust-lang/mdBook \
  --config config.local.toml

# Interactive query mode
./target/release/openrouter_git_agent query \
  --repo https://github.com/rust-lang/mdBook \
  --config config.local.toml \
  --query "How does the routing system work?"

# Show agent status and capabilities
./target/release/openrouter_git_agent status \
  --config config.local.toml
```

### Configuration Options

#### OpenRouter Settings
```toml
[openrouter]
api_key = "your_key"                    # Required: Your OpenRouter API key
model = "anthropic/claude-3.5-sonnet"   # AI model to use
max_tokens = 4000                       # Maximum response tokens
timeout = 60                            # Request timeout in seconds
```

#### Git Repository Settings
```toml
[git]
clone_directory = "./temp_repos"        # Where to clone repositories
max_file_size = 1048576                 # Max file size to analyze (1MB)
include_extensions = ["rs", "py", "js"] # File types to include
exclude_directories = [".git", "node_modules"] # Dirs to exclude
```

#### Symbiont Runtime Settings
```toml
[symbiont]
context_storage_path = "./agent_storage" # Agent persistent storage
qdrant_url = "http://localhost:6333"     # Vector database URL
collection_name = "agent_knowledge"      # Vector collection name
vector_dimension = 1536                  # Embedding dimensions
enable_compression = true                # Compress stored context
max_context_size_mb = 100               # Max context size
```

#### Security Settings
```toml
[security]
enable_schemapin = true                  # Enable cryptographic verification
policy_file = "./security_policies.yaml" # Security policy configuration
```

## Example Scenarios

### 1. Code Analysis Scenario

Performs comprehensive analysis of a repository:

```bash
./target/release/openrouter_git_agent code-analysis \
  --repo https://github.com/tokio-rs/tokio \
  --config config.local.toml
```

**What it does:**
- Clones the repository and ingests all code files
- Runs predefined analysis queries about architecture, patterns, and design
- Performs code quality assessment on key files
- Generates metrics on analysis performance

**Sample Output:**
```
Code Analysis Results for: https://github.com/tokio-rs/tokio

Query 1: What is the main purpose and functionality of this codebase?
Answer: Tokio is an asynchronous runtime for Rust, providing the building blocks for writing 
reliable, asynchronous, and performant applications...

Code Quality Analysis:
File: src/lib.rs
Analysis: The main library file shows excellent modular organization with clear separation 
of concerns. The public API is well-structured and follows Rust naming conventions...

Metrics:
- Total queries: 5
- Successful queries: 5
- Average query time: 1,234ms
- Code files analyzed: 156
```

### 2. Security Review Scenario

Automated security analysis:

```bash
./target/release/openrouter_git_agent security-review \
  --repo https://github.com/actix/actix-web \
  --config config.local.toml
```

**What it does:**
- Scans for common security vulnerabilities
- Checks for hardcoded secrets and credentials
- Reviews authentication and authorization patterns
- Validates against security policies
- Generates security score and recommendations

### 3. Documentation Generation

AI-powered documentation creation:

```bash
./target/release/openrouter_git_agent generate-docs \
  --repo https://github.com/serde-rs/serde \
  --config config.local.toml
```

**What it does:**
- Analyzes codebase to understand structure and functionality
- Generates comprehensive README documentation
- Creates API documentation for public interfaces
- Documents architecture and design patterns
- Provides installation and usage instructions

### 4. Interactive Query Mode

Ask specific questions about a repository:

```bash
./target/release/openrouter_git_agent query \
  --repo https://github.com/clap-rs/clap \
  --config config.local.toml \
  --query "How does command-line argument parsing work in this library?"
```

## Performance Metrics

The agent tracks various performance metrics:

- **Query Response Time**: Average time for AI-powered queries
- **Repository Ingestion**: Time to clone and process repositories
- **Vector Search**: Semantic search performance (<500ms target)
- **Context Management**: Memory usage and persistence efficiency
- **Security Verification**: Cryptographic verification overhead

## Security Features

### Policy-Based Access Control

The `security_policies.yaml` file defines comprehensive access control:

```yaml
network:
  allowed_domains:
    - "github.com"
    - "gitlab.com"
    - "openrouter.ai"
    
filesystem:
  allowed_base_paths:
    - "./temp_repos"
    - "./agent_storage"
    
repository:
  max_repo_size: 104857600  # 100MB
  trusted_sources:
    - pattern: "https://github.com/*"
```

### Cryptographic Verification

When SchemaPin is enabled:
- All external tool schemas are cryptographically verified
- Trust-On-First-Use (TOFU) prevents man-in-the-middle attacks
- Local key store maintains verification state
- AI-driven tool review workflow for new schemas

## Troubleshooting

### Common Issues

**Qdrant Connection Failed**
```bash
# Check if Qdrant is running
curl http://localhost:6333/collections

# Start Qdrant if needed
docker run -p 6333:6333 qdrant/qdrant
```

**OpenRouter API Errors**
- Verify your API key is correct
- Check your account has sufficient credits
- Ensure the selected model is available

**Repository Clone Failures**
- Check internet connectivity
- Verify repository URL is correct and accessible
- Ensure sufficient disk space in clone directory

**Permission Errors**
- Check security policies allow access to the repository
- Verify file system permissions for storage directories
- Review audit logs for policy violations

### Debug Mode

Enable detailed logging:

```bash
RUST_LOG=debug ./target/release/openrouter_git_agent analyze-repo \
  --repo https://github.com/rust-lang/mdBook \
  --config config.local.toml
```

## Development

### Running Tests

```bash
# Run unit tests
cargo test

# Run integration tests (requires Qdrant running)
cargo test --test integration_tests

# Run with coverage
cargo tarpaulin --out html
```

### Adding New Scenarios

1. Implement the `TestScenario` trait in `src/scenarios.rs`
2. Add the scenario to the CLI in `src/main.rs`
3. Update configuration options if needed
4. Add tests and documentation

### Extending Security Policies

1. Modify `security_policies.yaml` with new rules
2. Update the policy engine implementation
3. Test policy enforcement with various scenarios

## Contributing

1. Fork the repository
2. Create a feature branch
3. Add tests for new functionality
4. Ensure all tests pass
5. Submit a pull request

## License

This example is part of the Symbiont Agent Runtime System and follows the same license terms.

## Related Documentation

- [Symbiont Runtime README](../../README.md) - Main runtime documentation
- [Context Management Guide](../../docs/context_management.md) - Agent memory and persistence
- [Vector Database Integration](../../docs/vector_database.md) - Semantic search setup
- [Security Architecture](../../docs/security.md) - SchemaPin and policy enforcement
- [RAG Engine Documentation](../../docs/rag_engine.md) - Retrieval-augmented generation

## Support

For questions and support:
- Create an issue in the main repository
- Check the troubleshooting section above
- Review the comprehensive documentation in the parent project