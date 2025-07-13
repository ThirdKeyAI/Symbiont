# OpenRouter Git Agent

An AI-powered Git repository modification agent that understands natural language instructions and autonomously applies changes to code repositories.

## Overview

The OpenRouter Git Agent is a sophisticated AI agent that bridges the gap between natural language instructions and precise code modifications. Simply tell it what you want to accomplish, and the agent will analyze your repository, create an execution plan, and apply the changes safely.

**Key Capabilities:**
- 🗣️ **Natural Language Interface** - Describe what you want in plain English
- 🧠 **Intelligent Planning** - Converts prompts into structured execution plans  
- 🔄 **Autonomous Execution** - Applies changes with configurable autonomy levels
- 🛡️ **Safety-First** - Automatic backups, validation, and risk assessment
- 📋 **Dry Run Mode** - Preview changes before applying them
- 🔍 **Smart Analysis** - Understands repository context and dependencies

## Features

### Natural Language Processing
- **Free-form Prompts**: No rigid command syntax - just describe what you want
- **Context-Aware**: Understands your repository structure and existing code
- **Intent Classification**: Automatically categorizes requests (create, modify, refactor, etc.)
- **Ambiguity Resolution**: Asks clarifying questions when needed

### Autonomous Operation Modes
- **Ask Mode** (`--autonomy ask`): Requests confirmation before each change
- **Auto-Backup Mode** (`--autonomy auto-backup`): Creates backups and applies changes automatically
- **Auto-Commit Mode** (`--autonomy auto-commit`): Fully autonomous operation with automatic commits

### Safety & Validation
- **Automatic Backups**: Creates Git branches before making changes
- **Syntax Validation**: Checks code syntax before applying modifications
- **Impact Analysis**: Assesses the scope and risk of proposed changes
- **Safety Checks**: Prevents dangerous operations and protects sensitive files

## Quick Start

### 1. Setup

```bash
# Navigate to the agent directory
cd runtime/examples/openrouter_git_agent

# Build the agent
cargo build --release
```

### 2. Configure

Copy the configuration template and add your OpenRouter API key:

```bash
cp config.toml config.toml.local
```

Edit [`config.toml.local`](config.toml:7) and set your API key:

```toml
[openrouter]
api_key = "your_actual_openrouter_api_key_here"
```

**Get an API key from [OpenRouter](https://openrouter.ai/keys)**

### 3. Use the Agent

```bash
./target/release/openrouter_git_agent --repo https://github.com/user/repo "Add error handling to the main function"
```

## Usage

### Basic Command Structure

```bash
openrouter_git_agent --repo <REPOSITORY_URL> "<NATURAL_LANGUAGE_PROMPT>"
```

### Command Line Options

| Option | Description | Default |
|--------|-------------|---------|
| [`--repo`](src/main.rs:22) | Repository URL or local path | *Required* |
| [`--autonomy`](src/main.rs:29) | Autonomy level: `ask`, `auto-backup`, `auto-commit` | `auto-backup` |
| [`--dry-run`](src/main.rs:33) | Generate plan only, don't apply changes | `false` |
| [`--config`](src/main.rs:37) | Configuration file path | `config.toml` |
| [`--skip-clarification`](src/main.rs:41) | Skip clarification questions | `false` |

### Examples

#### Refactoring Code
```bash
openrouter_git_agent --repo https://github.com/user/project \
  "Refactor the authentication module to use async/await instead of callbacks"
```

#### Adding Features
```bash
openrouter_git_agent --repo ./my-project \
  "Add input validation to all user-facing functions and return proper error messages"
```

#### Bug Fixes
```bash
openrouter_git_agent --repo https://github.com/user/project \
  "Fix the memory leak in the connection pool by properly closing unused connections"
```

#### Testing
```bash
openrouter_git_agent --repo ./my-project \
  "Add comprehensive unit tests for the user service module"
```

#### Documentation
```bash
openrouter_git_agent --repo https://github.com/user/project \
  "Update the README with installation instructions and usage examples"
```

#### Dry Run (Preview Only)
```bash
openrouter_git_agent --repo ./my-project --dry-run \
  "Optimize database queries in the analytics module"
```

#### Full Autonomy
```bash
openrouter_git_agent --repo ./my-project --autonomy auto-commit \
  "Remove unused imports and dead code throughout the project"
```

#### Manual Confirmation
```bash
openrouter_git_agent --repo ./my-project --autonomy ask \
  "Migrate the database schema to add user preferences table"
```

## How It Works

The agent follows a **prompt-to-plan-to-execution** workflow:

```
Natural Language → Planner → Execution Plan → Modifier → Git Changes
                      ↓           ↓              ↓
                  Repository   Validator    Backup Branch
                   Context      
```

### Architecture Components

1. **[Prompt Planner](src/planner.rs)**: Analyzes natural language prompts and repository context to generate structured execution plans
2. **[File Modifier](src/modifier.rs)**: Safely applies changes to files with backup and validation support
3. **[Change Validator](src/validator.rs)**: Validates changes for syntax, safety, and impact
4. **[Workflow Orchestrator](src/workflow.rs)**: Coordinates the entire process and handles user interaction

### Execution Flow

1. **Context Gathering**: Analyzes repository structure and existing code
2. **Plan Generation**: Uses AI to create step-by-step execution plan
3. **Risk Assessment**: Evaluates potential impact and safety concerns
4. **User Confirmation**: Requests approval based on autonomy level
5. **Backup Creation**: Creates Git branch for safety
6. **Change Application**: Applies modifications with validation
7. **Commit & Summary**: Commits changes and provides results summary

## Configuration

### OpenRouter Settings

```toml
[openrouter]
api_key = "your_key"                    # Required: Get from openrouter.ai
model = "anthropic/claude-3.5-sonnet"   # AI model for processing
max_tokens = 4000                       # Maximum response length
temperature = 0.1                       # Response creativity (0.0-1.0)
timeout_seconds = 60                    # Request timeout
```

### Git Repository Settings

```toml
[git]
clone_base_path = "./temp_repos"        # Where to clone remote repos
max_file_size_mb = 1                    # Max file size to analyze
allowed_extensions = ["rs", "py", "js"] # File types to process
ignore_patterns = [".git", "node_modules"] # Directories to skip
max_files_per_repo = 1000              # Limit files per repository
```

### Workflow Settings

```toml
[workflow]
default_autonomy_level = "auto-backup"  # Default autonomy mode
enable_backups = true                   # Create automatic backups
backup_directory = "./backups"          # Backup storage location
confirmation_timeout_seconds = 30       # User confirmation timeout
max_retry_attempts = 3                  # Retry failed operations
```

### Safety Settings

```toml
[safety]
enable_safety_checks = true            # Enable safety validation
risk_threshold = "medium"               # Risk tolerance level
protected_patterns = ["*.env", "*.key"] # Files to protect
dangerous_operations = ["rm -rf"]       # Operations to restrict
require_confirmation_for_high_risk = true # Confirm risky changes
```

### Security Settings (Optional)

```toml
[security]
enable_schemapin = false               # Cryptographic verification
enable_sandbox = false                 # Sandbox execution
trusted_domains = ["github.com"]       # Allowed repository sources
```

## Advanced Usage

### Custom Configuration

Create environment-specific configurations:

```bash
# Development
openrouter_git_agent --config dev.toml --repo ./project "Add debug logging"

# Production
openrouter_git_agent --config prod.toml --repo ./project "Optimize performance"
```

### Error Handling

The agent provides detailed error information and recovery options:

```
❌ Error in file modification: syntax error in main.rs:42
How would you like to proceed?
1. Retry
2. Skip this step  
3. Abort
4. Manual intervention required
```

### Backup Management

Automatic backups are created as Git branches:

```bash
# List backup branches
git branch -a | grep backup-

# Restore from backup
git checkout backup-1704067200
git checkout -b recovery-branch
```

## Troubleshooting

### Common Issues

**OpenRouter API Connection Failed**
- Verify your API key is correct in [`config.toml`](config.toml:7)
- Check your internet connection
- Ensure you have sufficient API credits

**Repository Access Denied**
- Verify the repository URL is correct and accessible
- For private repositories, ensure proper authentication
- Check that the repository domain is in [`trusted_domains`](config.toml:78)

**File Modification Failed**
- Ensure you have write permissions to the repository
- Check that target files are not locked or read-only
- Verify the repository is not in a conflicted state

**Validation Errors**
- Review the execution plan in dry-run mode first
- Check that your prompt is clear and specific
- Ensure the target files exist and are in a valid state

### Debug Mode

Enable detailed logging for troubleshooting:

```bash
RUST_LOG=debug ./target/release/openrouter_git_agent \
  --repo ./project "Your prompt here"
```

### Support

For additional help:
1. Check the [configuration examples](config.toml) 
2. Run with [`--dry-run`](src/main.rs:33) to preview changes
3. Use [`--autonomy ask`](src/main.rs:29) for step-by-step control
4. Review the generated execution plans for insights

## Development

### Building from Source

```bash
git clone <repository-url>
cd runtime/examples/openrouter_git_agent
cargo build --release
```

### Running Tests

```bash
cargo test
cargo test --test integration_tests
```

### Architecture Overview

The codebase is organized into focused modules:

- [`main.rs`](src/main.rs) - CLI interface and application entry point
- [`planner.rs`](src/planner.rs) - Natural language to execution plan conversion
- [`modifier.rs`](src/modifier.rs) - File modification and backup handling  
- [`validator.rs`](src/validator.rs) - Change validation and safety checks
- [`workflow.rs`](src/workflow.rs) - End-to-end workflow orchestration
- [`git_tools.rs`](src/git_tools.rs) - Git repository operations
- [`config.rs`](src/config.rs) - Configuration management

## License

This project is part of the Symbiont Agent Runtime System. See the main repository for license information.

## Related Documentation

- [Symbiont Runtime](../../README.md) - Main runtime documentation
- [API Reference](../../API_REFERENCE.md) - Core runtime APIs
- [Sandbox Communication](../../SANDBOX_COMMUNICATION.md) - Security architecture