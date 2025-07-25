//! SchemaPin CLI Wrapper
//!
//! Rust wrapper for executing the SchemaPin Go CLI binary

use async_trait::async_trait;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command as TokioCommand;
use tokio::time::timeout;

use super::types::{
    SchemaPinConfig, SchemaPinError, SignArgs, SigningResult, VerificationResult, VerifyArgs,
};

/// Trait for SchemaPin CLI operations
#[async_trait]
pub trait SchemaPinCli: Send + Sync {
    /// Verify a schema using the SchemaPin CLI
    async fn verify_schema(&self, args: VerifyArgs) -> Result<VerificationResult, SchemaPinError>;

    /// Sign a schema using the SchemaPin CLI
    async fn sign_schema(&self, args: SignArgs) -> Result<SigningResult, SchemaPinError>;

    /// Check if the CLI binary is available
    async fn check_binary(&self) -> Result<bool, SchemaPinError>;

    /// Get CLI version information
    async fn get_version(&self) -> Result<String, SchemaPinError>;
}

/// SchemaPin CLI wrapper implementation
pub struct SchemaPinCliWrapper {
    pub config: SchemaPinConfig,
}

impl SchemaPinCliWrapper {
    /// Create a new CLI wrapper with default configuration
    pub fn new() -> Self {
        Self {
            config: SchemaPinConfig::default(),
        }
    }

    /// Create a new CLI wrapper with custom configuration
    pub fn with_config(config: SchemaPinConfig) -> Self {
        Self { config }
    }

    /// Execute a command with the CLI binary
    async fn execute_command(&self, args: Vec<String>) -> Result<String, SchemaPinError> {
        // Check if binary exists
        if !std::path::Path::new(&self.config.binary_path).exists() {
            return Err(SchemaPinError::BinaryNotFound {
                path: self.config.binary_path.clone(),
            });
        }

        // Build command
        let mut cmd = TokioCommand::new(&self.config.binary_path);
        cmd.args(&args)
            .stdout(Stdio::piped())
            .stderr(if self.config.capture_stderr {
                Stdio::piped()
            } else {
                Stdio::null()
            });

        // Set environment variables
        for (key, value) in &self.config.environment {
            cmd.env(key, value);
        }

        // Execute with timeout
        let timeout_duration = Duration::from_secs(self.config.timeout_seconds);
        let output = timeout(timeout_duration, cmd.output())
            .await
            .map_err(|_| SchemaPinError::Timeout {
                seconds: self.config.timeout_seconds,
            })?
            .map_err(|e| SchemaPinError::IoError {
                reason: e.to_string(),
            })?;

        // Check exit status
        if !output.status.success() {
            let stderr = if self.config.capture_stderr {
                String::from_utf8_lossy(&output.stderr).to_string()
            } else {
                "stderr not captured".to_string()
            };

            return Err(SchemaPinError::ExecutionFailed {
                reason: format!(
                    "Command failed with exit code {:?}. stderr: {}",
                    output.status.code(),
                    stderr
                ),
            });
        }

        // Return stdout
        String::from_utf8(output.stdout).map_err(|e| SchemaPinError::IoError {
            reason: format!("Failed to parse stdout as UTF-8: {}", e),
        })
    }

    /// Parse JSON output from CLI
    fn parse_verification_result(
        &self,
        json_output: &str,
    ) -> Result<VerificationResult, SchemaPinError> {
        serde_json::from_str(json_output).map_err(|e| SchemaPinError::JsonParsingFailed {
            reason: format!("Failed to parse JSON: {}. Output: {}", e, json_output),
        })
    }

    /// Parse signing result JSON output from CLI
    fn parse_signing_result(&self, json_output: &str) -> Result<SigningResult, SchemaPinError> {
        serde_json::from_str(json_output).map_err(|e| SchemaPinError::JsonParsingFailed {
            reason: format!(
                "Failed to parse signing JSON: {}. Output: {}",
                e, json_output
            ),
        })
    }
}

impl Default for SchemaPinCliWrapper {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SchemaPinCli for SchemaPinCliWrapper {
    async fn verify_schema(&self, args: VerifyArgs) -> Result<VerificationResult, SchemaPinError> {
        // Validate arguments
        if args.schema_path.is_empty() {
            return Err(SchemaPinError::InvalidArguments {
                args: vec!["schema_path cannot be empty".to_string()],
            });
        }

        if args.public_key_url.is_empty() {
            return Err(SchemaPinError::InvalidArguments {
                args: vec!["public_key_url cannot be empty".to_string()],
            });
        }

        // Validate public key URL format (basic check) - do this before file existence check
        if !args.public_key_url.starts_with("http://")
            && !args.public_key_url.starts_with("https://")
        {
            return Err(SchemaPinError::InvalidPublicKeyUrl {
                url: args.public_key_url.clone(),
            });
        }

        // Check if schema file exists
        if !std::path::Path::new(&args.schema_path).exists() {
            return Err(SchemaPinError::SchemaFileNotFound {
                path: args.schema_path.clone(),
            });
        }

        // Execute verification command
        let cmd_args = args.to_args();
        let output = self.execute_command(cmd_args).await?;

        // Parse result
        let result = self.parse_verification_result(&output)?;

        // Check if verification was successful
        if !result.success {
            return Err(SchemaPinError::VerificationFailed {
                reason: result.message.clone(),
            });
        }

        Ok(result)
    }

    async fn sign_schema(&self, args: SignArgs) -> Result<SigningResult, SchemaPinError> {
        // Validate arguments
        if args.schema_path.is_empty() {
            return Err(SchemaPinError::InvalidArguments {
                args: vec!["schema_path cannot be empty".to_string()],
            });
        }

        if args.private_key_path.is_empty() {
            return Err(SchemaPinError::InvalidArguments {
                args: vec!["private_key_path cannot be empty".to_string()],
            });
        }

        // Check if schema file exists
        if !std::path::Path::new(&args.schema_path).exists() {
            return Err(SchemaPinError::SchemaFileNotFound {
                path: args.schema_path.clone(),
            });
        }

        // Check if private key file exists
        if !std::path::Path::new(&args.private_key_path).exists() {
            return Err(SchemaPinError::PrivateKeyNotFound {
                path: args.private_key_path.clone(),
            });
        }

        // Execute signing command
        let cmd_args = args.to_args();
        let output = self.execute_command(cmd_args).await?;

        // Parse result
        let result = self.parse_signing_result(&output)?;

        // Check if signing was successful
        if !result.success {
            return Err(SchemaPinError::SigningFailed {
                reason: result.message.clone(),
            });
        }

        Ok(result)
    }

    async fn check_binary(&self) -> Result<bool, SchemaPinError> {
        // Check if file exists and is executable
        let path = std::path::Path::new(&self.config.binary_path);
        if !path.exists() {
            return Ok(false);
        }

        // Try to execute version command to verify it's working
        match self.execute_command(vec!["--version".to_string()]).await {
            Ok(_) => Ok(true),
            Err(SchemaPinError::BinaryNotFound { .. }) => Ok(false),
            Err(SchemaPinError::ExecutionFailed { .. }) => Ok(false),
            Err(e) => Err(e),
        }
    }

    async fn get_version(&self) -> Result<String, SchemaPinError> {
        let output = self.execute_command(vec!["--version".to_string()]).await?;
        Ok(output.trim().to_string())
    }
}

/// Mock implementation for testing
pub struct MockSchemaPinCli {
    should_succeed: bool,
    mock_result: Option<VerificationResult>,
}

impl MockSchemaPinCli {
    /// Create a new mock that always succeeds
    pub fn new_success() -> Self {
        Self {
            should_succeed: true,
            mock_result: None,
        }
    }

    /// Create a new mock that always fails
    pub fn new_failure() -> Self {
        Self {
            should_succeed: false,
            mock_result: None,
        }
    }

    /// Create a new mock with custom result
    pub fn with_result(result: VerificationResult) -> Self {
        Self {
            should_succeed: result.success,
            mock_result: Some(result),
        }
    }
}

#[async_trait]
impl SchemaPinCli for MockSchemaPinCli {
    async fn verify_schema(&self, _args: VerifyArgs) -> Result<VerificationResult, SchemaPinError> {
        if let Some(ref result) = self.mock_result {
            if result.success {
                Ok(result.clone())
            } else {
                Err(SchemaPinError::VerificationFailed {
                    reason: result.message.clone(),
                })
            }
        } else if self.should_succeed {
            Ok(VerificationResult {
                success: true,
                message: "Mock verification successful".to_string(),
                schema_hash: Some("mock_hash_123".to_string()),
                public_key_url: Some("https://mock.example.com/pubkey".to_string()),
                signature: None,
                metadata: None,
                timestamp: Some("2024-01-01T00:00:00Z".to_string()),
            })
        } else {
            Err(SchemaPinError::VerificationFailed {
                reason: "Mock verification failed".to_string(),
            })
        }
    }

    async fn sign_schema(&self, _args: SignArgs) -> Result<SigningResult, SchemaPinError> {
        if self.should_succeed {
            Ok(SigningResult {
                success: true,
                message: "Mock signing successful".to_string(),
                schema_hash: Some("mock_signed_hash_456".to_string()),
                signed_schema_path: Some("/mock/path/signed_schema.json".to_string()),
                signature: Some(crate::integrations::schemapin::SignatureInfo {
                    algorithm: "Ed25519".to_string(),
                    signature: "mock_signature_data".to_string(),
                    key_fingerprint: Some("mock_fingerprint".to_string()),
                    valid: true,
                }),
                metadata: None,
                timestamp: Some("2024-01-01T00:00:00Z".to_string()),
            })
        } else {
            Err(SchemaPinError::SigningFailed {
                reason: "Mock signing failed".to_string(),
            })
        }
    }

    async fn check_binary(&self) -> Result<bool, SchemaPinError> {
        Ok(true) // Mock always reports binary as available
    }

    async fn get_version(&self) -> Result<String, SchemaPinError> {
        Ok("schemapin-cli v1.0.0 (mock)".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_mock_cli_success() {
        let cli = MockSchemaPinCli::new_success();
        let args = VerifyArgs::new(
            "/path/to/schema.json".to_string(),
            "https://example.com/pubkey".to_string(),
        );

        let result = cli.verify_schema(args).await.unwrap();
        assert!(result.success);
        assert_eq!(result.message, "Mock verification successful");
    }

    #[tokio::test]
    async fn test_mock_cli_failure() {
        let cli = MockSchemaPinCli::new_failure();
        let args = VerifyArgs::new(
            "/path/to/schema.json".to_string(),
            "https://example.com/pubkey".to_string(),
        );

        let result = cli.verify_schema(args).await;
        assert!(result.is_err());

        if let Err(SchemaPinError::VerificationFailed { reason }) = result {
            assert_eq!(reason, "Mock verification failed");
        } else {
            panic!("Expected VerificationFailed error");
        }
    }

    #[tokio::test]
    async fn test_mock_cli_custom_result() {
        let custom_result = VerificationResult {
            success: true,
            message: "Custom mock result".to_string(),
            schema_hash: Some("custom_hash".to_string()),
            public_key_url: Some("https://custom.example.com/pubkey".to_string()),
            signature: None,
            metadata: Some({
                let mut map = HashMap::new();
                map.insert(
                    "test".to_string(),
                    serde_json::Value::String("value".to_string()),
                );
                map
            }),
            timestamp: Some("2024-12-07T00:00:00Z".to_string()),
        };

        let cli = MockSchemaPinCli::with_result(custom_result.clone());
        let args = VerifyArgs::new(
            "/path/to/schema.json".to_string(),
            "https://example.com/pubkey".to_string(),
        );

        let result = cli.verify_schema(args).await.unwrap();
        assert_eq!(result.message, "Custom mock result");
        assert_eq!(result.schema_hash, Some("custom_hash".to_string()));
    }

    #[tokio::test]
    async fn test_verify_args_validation() {
        let cli = SchemaPinCliWrapper::new();

        // Test empty schema path
        let args = VerifyArgs::new("".to_string(), "https://example.com/pubkey".to_string());
        let result = cli.verify_schema(args).await;
        assert!(matches!(
            result,
            Err(SchemaPinError::InvalidArguments { .. })
        ));

        // Test empty public key URL
        let args = VerifyArgs::new("/path/to/schema.json".to_string(), "".to_string());
        let result = cli.verify_schema(args).await;
        assert!(matches!(
            result,
            Err(SchemaPinError::InvalidArguments { .. })
        ));

        // Test invalid public key URL
        let args = VerifyArgs::new(
            "/path/to/schema.json".to_string(),
            "invalid-url".to_string(),
        );
        let result = cli.verify_schema(args).await;
        assert!(matches!(
            result,
            Err(SchemaPinError::InvalidPublicKeyUrl { .. })
        ));
    }

    #[test]
    fn test_config_creation() {
        let config = SchemaPinConfig {
            binary_path: "/custom/path/schemapin-cli".to_string(),
            timeout_seconds: 60,
            capture_stderr: false,
            environment: {
                let mut env = HashMap::new();
                env.insert("TEST_VAR".to_string(), "test_value".to_string());
                env
            },
        };

        let cli = SchemaPinCliWrapper::with_config(config.clone());
        assert_eq!(cli.config.binary_path, "/custom/path/schemapin-cli");
        assert_eq!(cli.config.timeout_seconds, 60);
        assert!(!cli.config.capture_stderr);
        assert_eq!(
            cli.config.environment.get("TEST_VAR"),
            Some(&"test_value".to_string())
        );
    }

    #[tokio::test]
    async fn test_mock_cli_signing_success() {
        let cli = MockSchemaPinCli::new_success();
        let args = SignArgs::new(
            "/path/to/schema.json".to_string(),
            "/path/to/private.key".to_string(),
        );

        let result = cli.sign_schema(args).await.unwrap();
        assert!(result.success);
        assert_eq!(result.message, "Mock signing successful");
        assert_eq!(result.schema_hash, Some("mock_signed_hash_456".to_string()));
        assert!(result.signature.is_some());
    }

    #[tokio::test]
    async fn test_mock_cli_signing_failure() {
        let cli = MockSchemaPinCli::new_failure();
        let args = SignArgs::new(
            "/path/to/schema.json".to_string(),
            "/path/to/private.key".to_string(),
        );

        let result = cli.sign_schema(args).await;
        assert!(result.is_err());

        if let Err(SchemaPinError::SigningFailed { reason }) = result {
            assert_eq!(reason, "Mock signing failed");
        } else {
            panic!("Expected SigningFailed error");
        }
    }

    #[tokio::test]
    async fn test_sign_args_validation() {
        let cli = SchemaPinCliWrapper::new();

        // Test empty schema path
        let args = SignArgs::new("".to_string(), "/path/to/private.key".to_string());
        let result = cli.sign_schema(args).await;
        assert!(matches!(
            result,
            Err(SchemaPinError::InvalidArguments { .. })
        ));

        // Test empty private key path
        let args = SignArgs::new("/path/to/schema.json".to_string(), "".to_string());
        let result = cli.sign_schema(args).await;
        assert!(matches!(
            result,
            Err(SchemaPinError::InvalidArguments { .. })
        ));
    }
}
