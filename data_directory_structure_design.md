# Extended Data Directory Structure Design

## Overview

This document presents the design for an enhanced data directory structure that extends the current `FilePersistenceConfig` to support unified management of agent contexts, logs, prompts, and vector database storage under a single root directory.

## Current Implementation Analysis

### Existing `FilePersistenceConfig`
```rust
pub struct FilePersistenceConfig {
    /// Base directory for storing context files
    pub storage_path: PathBuf,
    /// Enable compression for stored files
    pub enable_compression: bool,
    /// Enable encryption for stored files
    pub enable_encryption: bool,
    /// Backup retention count
    pub backup_count: usize,
    /// Auto-save interval in seconds
    pub auto_save_interval: u64,
}
```

**Current Default**: `./data/contexts`

### Identified Patterns
- Logs currently use: `~/.symbiont/logs/mcp-cli.log`
- SchemaPin keys use: `~/.symbiont/schemapin_keys.json`
- Current context storage: `./data/contexts`

## Proposed Enhanced Structure

### Directory Layout
```
~/.symbiont/data/
├── state/           # Agent contexts and session state
│   ├── agents/      # Per-agent context files
│   └── sessions/    # Session-specific data
├── logs/            # System and agent logs
│   ├── system/      # System-level logs
│   ├── agents/      # Agent-specific logs
│   └── audit/       # Audit trail logs
├── prompts/         # Prompt templates and history
│   ├── templates/   # Reusable prompt templates
│   ├── history/     # Prompt execution history
│   └── cache/       # Cached prompt results
└── vector_db/       # Vector database storage
    ├── collections/ # Vector collections
    ├── indexes/     # Search indexes
    └── metadata/    # Collection metadata
```

### Enhanced `FilePersistenceConfig`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilePersistenceConfig {
    /// Root data directory (replaces storage_path)
    pub root_data_dir: PathBuf,
    
    /// Subdirectory paths (relative to root_data_dir)
    pub state_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub prompts_dir: PathBuf,
    pub vector_db_dir: PathBuf,
    
    /// Existing configuration options
    pub enable_compression: bool,
    pub enable_encryption: bool,
    pub backup_count: usize,
    pub auto_save_interval: u64,
    
    /// New configuration options
    pub auto_create_dirs: bool,
    pub dir_permissions: Option<u32>,
}

impl Default for FilePersistenceConfig {
    fn default() -> Self {
        let mut root_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        root_dir.push(".symbiont");
        root_dir.push("data");
        
        Self {
            root_data_dir: root_dir,
            state_dir: PathBuf::from("state"),
            logs_dir: PathBuf::from("logs"),
            prompts_dir: PathBuf::from("prompts"),
            vector_db_dir: PathBuf::from("vector_db"),
            enable_compression: true,
            enable_encryption: false,
            backup_count: 3,
            auto_save_interval: 300,
            auto_create_dirs: true,
            dir_permissions: Some(0o755),
        }
    }
}

impl FilePersistenceConfig {
    /// Get the full path for state storage
    pub fn state_path(&self) -> PathBuf {
        self.root_data_dir.join(&self.state_dir)
    }
    
    /// Get the full path for logs storage
    pub fn logs_path(&self) -> PathBuf {
        self.root_data_dir.join(&self.logs_dir)
    }
    
    /// Get the full path for prompts storage
    pub fn prompts_path(&self) -> PathBuf {
        self.root_data_dir.join(&self.prompts_dir)
    }
    
    /// Get the full path for vector database storage
    pub fn vector_db_path(&self) -> PathBuf {
        self.root_data_dir.join(&self.vector_db_dir)
    }
    
    /// Create all configured directories if they don't exist
    pub async fn ensure_directories(&self) -> Result<(), std::io::Error> {
        if self.auto_create_dirs {
            tokio::fs::create_dir_all(self.state_path()).await?;
            tokio::fs::create_dir_all(self.logs_path()).await?;
            tokio::fs::create_dir_all(self.prompts_path()).await?;
            tokio::fs::create_dir_all(self.vector_db_path()).await?;
            
            // Create subdirectories
            tokio::fs::create_dir_all(self.state_path().join("agents")).await?;
            tokio::fs::create_dir_all(self.state_path().join("sessions")).await?;
            tokio::fs::create_dir_all(self.logs_path().join("system")).await?;
            tokio::fs::create_dir_all(self.logs_path().join("agents")).await?;
            tokio::fs::create_dir_all(self.logs_path().join("audit")).await?;
            tokio::fs::create_dir_all(self.prompts_path().join("templates")).await?;
            tokio::fs::create_dir_all(self.prompts_path().join("history")).await?;
            tokio::fs::create_dir_all(self.prompts_path().join("cache")).await?;
            tokio::fs::create_dir_all(self.vector_db_path().join("collections")).await?;
            tokio::fs::create_dir_all(self.vector_db_path().join("indexes")).await?;
            tokio::fs::create_dir_all(self.vector_db_path().join("metadata")).await?;
        }
        Ok(())
    }
}
```

## Directory Usage Specifications

### State Directory (`state/`)
- **Purpose**: Store agent contexts, session data, and persistent state
- **Structure**:
  - `agents/{agent_id}.json` - Individual agent context files
  - `sessions/{session_id}.json` - Session-specific data
- **Features**: Supports compression, encryption, and backup rotation

### Logs Directory (`logs/`)
- **Purpose**: Centralized logging for system, agents, and audit trails
- **Structure**:
  - `system/symbiont.log` - System-level logs
  - `agents/{agent_id}.log` - Agent-specific logs
  - `audit/security.log` - Security and audit events
- **Features**: Log rotation, compression, retention policies

### Prompts Directory (`prompts/`)
- **Purpose**: Store prompt templates, execution history, and cached results
- **Structure**:
  - `templates/` - Reusable prompt templates
  - `history/{date}/` - Daily prompt execution logs
  - `cache/{hash}.json` - Cached prompt results
- **Features**: Template versioning, execution tracking, cache management

### Vector Database Directory (`vector_db/`)
- **Purpose**: Local vector database storage for embeddings and search indexes
- **Structure**:
  - `collections/{collection_name}/` - Vector collections
  - `indexes/{index_name}/` - Search indexes
  - `metadata/` - Collection and index metadata
- **Features**: Collection isolation, index optimization, metadata tracking

## Backward Compatibility Strategy

### Migration Path
1. **Detect existing `storage_path`** in current configurations
2. **Create migration utility** to move existing contexts to new structure
3. **Maintain compatibility layer** during transition period
4. **Provide configuration migration tool**

### Migration Implementation
```rust
impl FilePersistenceConfig {
    /// Migrate from legacy storage_path to new structure
    pub async fn migrate_from_legacy(legacy_path: PathBuf) -> Result<Self, MigrationError> {
        let mut config = Self::default();
        
        // Copy existing context files to new state directory
        if legacy_path.exists() {
            let state_path = config.state_path().join("agents");
            config.ensure_directories().await?;
            
            // Move existing context files
            let mut entries = tokio::fs::read_dir(&legacy_path).await?;
            while let Some(entry) = entries.next_entry().await? {
                let file_path = entry.path();
                if file_path.extension().map_or(false, |ext| ext == "json") {
                    let dest_path = state_path.join(entry.file_name());
                    tokio::fs::copy(&file_path, &dest_path).await?;
                }
            }
        }
        
        Ok(config)
    }
}
```

## Benefits and Rationale

### Organizational Benefits
- **Unified Structure**: All data under single root directory
- **Clear Separation**: Logical separation of different data types
- **Scalability**: Easy to add new data categories
- **Consistency**: Follows established `~/.symbiont/` pattern

### Operational Benefits
- **Backup Simplicity**: Single directory to backup
- **Security**: Unified permission and encryption management
- **Monitoring**: Centralized disk usage and health monitoring
- **Maintenance**: Simplified cleanup and archival processes

### Development Benefits
- **Type Safety**: Dedicated path methods prevent misuse
- **Configuration**: Flexible directory structure configuration
- **Testing**: Isolated test environments
- **Documentation**: Clear data organization for developers

## Implementation Considerations

### Performance
- **Directory Creation**: Lazy creation with `ensure_directories()`
- **Path Resolution**: Cached path computations
- **Concurrent Access**: Thread-safe directory operations

### Security
- **Permissions**: Configurable directory permissions (default 0o755)
- **Encryption**: Consistent encryption across all data types
- **Isolation**: Separate directories prevent data leakage

### Maintenance
- **Cleanup**: Automated cleanup based on retention policies
- **Monitoring**: Directory size and health monitoring
- **Backup**: Simplified backup strategies

## Next Steps

1. **Implementation**: Update `FilePersistenceConfig` in `crates/runtime/src/context/types.rs`
2. **Migration**: Create migration utilities for existing installations
3. **Integration**: Update all persistence-related code to use new paths
4. **Testing**: Comprehensive tests for new directory structure
5. **Documentation**: Update API documentation and user guides

---

This design provides a robust, scalable foundation for data persistence while maintaining backward compatibility and following established patterns in the Symbiont codebase.