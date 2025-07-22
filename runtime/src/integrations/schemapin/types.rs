//! SchemaPin CLI Types
//! 
//! Defines Rust structs to represent the JSON output of the SchemaPin CLI verify command

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;
use std::path::PathBuf;

/// Result of schema verification from SchemaPin CLI
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VerificationResult {
    /// Whether the verification was successful
    pub success: bool,
    /// Verification message
    pub message: String,
    /// Schema hash if verification succeeded
    pub schema_hash: Option<String>,
    /// Public key URL used for verification
    pub public_key_url: Option<String>,
    /// Signature information
    pub signature: Option<SignatureInfo>,
    /// Additional metadata from verification
    pub metadata: Option<HashMap<String, serde_json::Value>>,
    /// Timestamp of verification
    pub timestamp: Option<String>,
}

/// Signature information from verification
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SignatureInfo {
    /// Algorithm used for signing
    pub algorithm: String,
    /// Signature value
    pub signature: String,
    /// Public key fingerprint
    pub key_fingerprint: Option<String>,
    /// Signature validity
    pub valid: bool,
}

/// SchemaPin CLI error types
#[derive(Error, Debug, Clone)]
pub enum SchemaPinError {
    #[error("CLI execution failed: {reason}")]
    ExecutionFailed { reason: String },
    
    #[error("Binary not found at path: {path}")]
    BinaryNotFound { path: String },
    
    #[error("Invalid arguments: {args:?}")]
    InvalidArguments { args: Vec<String> },
    
    #[error("JSON parsing failed: {reason}")]
    JsonParsingFailed { reason: String },
    
    #[error("Verification failed: {reason}")]
    VerificationFailed { reason: String },
    
    #[error("Timeout occurred after {seconds} seconds")]
    Timeout { seconds: u64 },
    
    #[error("IO error: {reason}")]
    IoError { reason: String },
    
    #[error("Schema file not found: {path}")]
    SchemaFileNotFound { path: String },
    
    #[error("Public key URL invalid: {url}")]
    InvalidPublicKeyUrl { url: String },
    
    #[error("Signing failed: {reason}")]
    SigningFailed { reason: String },
    
    #[error("Private key file not found: {path}")]
    PrivateKeyNotFound { path: String },
    
    #[error("Invalid private key format: {reason}")]
    InvalidPrivateKey { reason: String },
}

/// Configuration for SchemaPin CLI wrapper
#[derive(Debug, Clone)]
pub struct SchemaPinConfig {
    /// Path to the schemapin-cli binary
    pub binary_path: String,
    /// Default timeout for CLI operations in seconds
    pub timeout_seconds: u64,
    /// Whether to capture stderr output
    pub capture_stderr: bool,
    /// Environment variables to set for CLI execution
    pub environment: HashMap<String, String>,
}

impl Default for SchemaPinConfig {
    fn default() -> Self {
        Self {
            binary_path: "/home/jascha/Documents/repos/SchemaPin/go/bin/schemapin-cli".to_string(),
            timeout_seconds: 30,
            capture_stderr: true,
            environment: HashMap::new(),
        }
    }
}

/// Arguments for schema verification
#[derive(Debug, Clone)]
pub struct VerifyArgs {
    /// Path to the schema file to verify
    pub schema_path: String,
    /// URL to the public key for verification
    pub public_key_url: String,
    /// Optional additional arguments
    pub additional_args: Vec<String>,
}

/// Arguments for schema signing
#[derive(Debug, Clone)]
pub struct SignArgs {
    /// Path to the schema file to sign
    pub schema_path: String,
    /// Path to the private key file
    pub private_key_path: String,
    /// Output path for the signed schema
    pub output_path: Option<String>,
    /// Optional additional arguments
    pub additional_args: Vec<String>,
}

/// Result of schema signing from SchemaPin CLI
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SigningResult {
    /// Whether the signing was successful
    pub success: bool,
    /// Signing message
    pub message: String,
    /// Schema hash of the signed schema
    pub schema_hash: Option<String>,
    /// Path to the signed schema file
    pub signed_schema_path: Option<String>,
    /// Signature information
    pub signature: Option<SignatureInfo>,
    /// Additional metadata from signing
    pub metadata: Option<HashMap<String, serde_json::Value>>,
    /// Timestamp of signing
    pub timestamp: Option<String>,
}

impl VerifyArgs {
    /// Create new verification arguments
    pub fn new(schema_path: String, public_key_url: String) -> Self {
        Self {
            schema_path,
            public_key_url,
            additional_args: Vec::new(),
        }
    }
    
    /// Add additional argument
    pub fn with_arg(mut self, arg: String) -> Self {
        self.additional_args.push(arg);
        self
    }
    
    /// Convert to command line arguments
    pub fn to_args(&self) -> Vec<String> {
        let mut args = vec![
            "verify".to_string(),
            "--schema".to_string(),
            self.schema_path.clone(),
            "--public-key-url".to_string(),
            self.public_key_url.clone(),
        ];
        args.extend(self.additional_args.clone());
        args
    }
}

impl SignArgs {
    /// Create new signing arguments
    pub fn new(schema_path: String, private_key_path: String) -> Self {
        Self {
            schema_path,
            private_key_path,
            output_path: None,
            additional_args: Vec::new(),
        }
    }
    
    /// Set output path for signed schema
    pub fn with_output_path(mut self, output_path: String) -> Self {
        self.output_path = Some(output_path);
        self
    }
    
    /// Add additional argument
    pub fn with_arg(mut self, arg: String) -> Self {
        self.additional_args.push(arg);
        self
    }
    
    /// Convert to command line arguments
    pub fn to_args(&self) -> Vec<String> {
        let mut args = vec![
            "sign".to_string(),
            "--schema".to_string(),
            self.schema_path.clone(),
            "--private-key".to_string(),
            self.private_key_path.clone(),
        ];
        
        if let Some(ref output_path) = self.output_path {
            args.push("--output".to_string());
            args.push(output_path.clone());
        }
        
        args.extend(self.additional_args.clone());
        args
    }
}

/// Represents a pinned public key in the TOFU key store
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PinnedKey {
    /// The identifier for this key (e.g., domain name)
    pub identifier: String,
    /// The public key data
    pub public_key: String,
    /// Algorithm used for the key
    pub algorithm: String,
    /// Key fingerprint for verification
    pub fingerprint: String,
    /// Timestamp when the key was first pinned
    pub pinned_at: String,
    /// Optional metadata about the key
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

impl PinnedKey {
    /// Create a new pinned key
    pub fn new(
        identifier: String,
        public_key: String,
        algorithm: String,
        fingerprint: String,
    ) -> Self {
        Self {
            identifier,
            public_key,
            algorithm,
            fingerprint,
            pinned_at: chrono::Utc::now().to_rfc3339(),
            metadata: None,
        }
    }

    /// Create a new pinned key with metadata
    pub fn with_metadata(
        identifier: String,
        public_key: String,
        algorithm: String,
        fingerprint: String,
        metadata: HashMap<String, serde_json::Value>,
    ) -> Self {
        Self {
            identifier,
            public_key,
            algorithm,
            fingerprint,
            pinned_at: chrono::Utc::now().to_rfc3339(),
            metadata: Some(metadata),
        }
    }
}

/// Configuration for the key store
#[derive(Debug, Clone)]
pub struct KeyStoreConfig {
    /// Path to the key store file
    pub store_path: PathBuf,
    /// Whether to create the store file if it doesn't exist
    pub create_if_missing: bool,
    /// File permissions for the store file (Unix only)
    pub file_permissions: Option<u32>,
}

impl Default for KeyStoreConfig {
    fn default() -> Self {
        let mut store_path = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        store_path.push(".symbiont");
        store_path.push("schemapin_keys.json");
        
        Self {
            store_path,
            create_if_missing: true,
            file_permissions: Some(0o600), // Read/write for owner only
        }
    }
}

/// Key store specific errors
#[derive(Error, Debug, Clone)]
pub enum KeyStoreError {
    #[error("Key store file not found: {path}")]
    StoreFileNotFound { path: String },
    
    #[error("Failed to read key store: {reason}")]
    ReadFailed { reason: String },
    
    #[error("Failed to write key store: {reason}")]
    WriteFailed { reason: String },
    
    #[error("Key not found for identifier: {identifier}")]
    KeyNotFound { identifier: String },
    
    #[error("Key already pinned for identifier: {identifier}")]
    KeyAlreadyPinned { identifier: String },
    
    #[error("Key mismatch for identifier: {identifier}")]
    KeyMismatch { identifier: String },
    
    #[error("Invalid key format: {reason}")]
    InvalidKeyFormat { reason: String },
    
    #[error("Serialization failed: {reason}")]
    SerializationFailed { reason: String },
    
    #[error("IO error: {reason}")]
    IoError { reason: String },
    
    #[error("Permission denied: {reason}")]
    PermissionDenied { reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_verify_args_creation() {
        let args = VerifyArgs::new(
            "/path/to/schema.json".to_string(),
            "https://example.com/pubkey".to_string(),
        );
        
        assert_eq!(args.schema_path, "/path/to/schema.json");
        assert_eq!(args.public_key_url, "https://example.com/pubkey");
        assert!(args.additional_args.is_empty());
    }
    
    #[test]
    fn test_verify_args_with_additional() {
        let args = VerifyArgs::new(
            "/path/to/schema.json".to_string(),
            "https://example.com/pubkey".to_string(),
        )
        .with_arg("--verbose".to_string())
        .with_arg("--format=json".to_string());
        
        assert_eq!(args.additional_args.len(), 2);
        assert_eq!(args.additional_args[0], "--verbose");
        assert_eq!(args.additional_args[1], "--format=json");
    }
    
    #[test]
    fn test_verify_args_to_args() {
        let args = VerifyArgs::new(
            "/path/to/schema.json".to_string(),
            "https://example.com/pubkey".to_string(),
        )
        .with_arg("--verbose".to_string());
        
        let cmd_args = args.to_args();
        assert_eq!(cmd_args, vec![
            "verify",
            "--schema",
            "/path/to/schema.json",
            "--public-key-url",
            "https://example.com/pubkey",
            "--verbose"
        ]);
    }
    
    #[test]
    fn test_default_config() {
        let config = SchemaPinConfig::default();
        assert_eq!(config.binary_path, "/home/jascha/Documents/repos/SchemaPin/go/bin/schemapin-cli");
        assert_eq!(config.timeout_seconds, 30);
        assert!(config.capture_stderr);
        assert!(config.environment.is_empty());
    }
    
    #[test]
    fn test_sign_args_creation() {
        let args = SignArgs::new(
            "/path/to/schema.json".to_string(),
            "/path/to/private.key".to_string(),
        );
        
        assert_eq!(args.schema_path, "/path/to/schema.json");
        assert_eq!(args.private_key_path, "/path/to/private.key");
        assert!(args.output_path.is_none());
        assert!(args.additional_args.is_empty());
    }
    
    #[test]
    fn test_sign_args_with_output() {
        let args = SignArgs::new(
            "/path/to/schema.json".to_string(),
            "/path/to/private.key".to_string(),
        )
        .with_output_path("/path/to/signed_schema.json".to_string())
        .with_arg("--verbose".to_string());
        
        assert_eq!(args.output_path, Some("/path/to/signed_schema.json".to_string()));
        assert_eq!(args.additional_args.len(), 1);
        assert_eq!(args.additional_args[0], "--verbose");
    }
    
    #[test]
    fn test_sign_args_to_args() {
        let args = SignArgs::new(
            "/path/to/schema.json".to_string(),
            "/path/to/private.key".to_string(),
        )
        .with_output_path("/path/to/signed.json".to_string())
        .with_arg("--format=json".to_string());
        
        let cmd_args = args.to_args();
        assert_eq!(cmd_args, vec![
            "sign",
            "--schema",
            "/path/to/schema.json",
            "--private-key",
            "/path/to/private.key",
            "--output",
            "/path/to/signed.json",
            "--format=json"
        ]);
    }
}