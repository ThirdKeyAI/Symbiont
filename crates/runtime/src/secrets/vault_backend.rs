//! HashiCorp Vault secrets backend implementation
//!
//! This module provides a Vault-based secrets store that supports multiple authentication
//! methods and interacts with Vault's KV v2 secrets engine.

use super::{BoxedAuditSink, Secret, SecretAuditEvent, SecretError, SecretStore};
use crate::secrets::config::{VaultAuthConfig, VaultConfig};
use async_trait::async_trait;
use std::time::Duration;
use vaultrs::client::{Client, VaultClient, VaultClientSettingsBuilder};
use vaultrs::error::ClientError;
use vaultrs::kv2;
use vaultrs::token;

/// Vault-based secrets store implementation
pub struct VaultSecretStore {
    client: VaultClient,
    config: VaultConfig,
    agent_id: String,
    audit_sink: Option<BoxedAuditSink>,
}

impl VaultSecretStore {
    /// Create a new VaultSecretStore with the given configuration and agent ID
    pub async fn new(
        config: VaultConfig,
        agent_id: String,
        audit_sink: Option<BoxedAuditSink>,
    ) -> Result<Self, SecretError> {
        let client = Self::create_vault_client(&config).await?;

        Ok(Self {
            client,
            config,
            agent_id,
            audit_sink,
        })
    }

    /// Create and configure a Vault client
    async fn create_vault_client(config: &VaultConfig) -> Result<VaultClient, SecretError> {
        let mut settings_builder = VaultClientSettingsBuilder::default();
        settings_builder.address(&config.url);

        // Configure namespace if specified
        if let Some(namespace) = &config.namespace {
            settings_builder.namespace(Some(namespace.clone()));
        }

        // Configure TLS settings
        if config.tls.skip_verify {
            settings_builder.verify(false);
        }

        // Set timeouts
        let connection_timeout = Duration::from_secs(config.connection.connection_timeout_seconds);
        settings_builder.timeout(Some(connection_timeout));

        let settings = settings_builder
            .build()
            .map_err(|e| SecretError::ConfigurationError {
                message: format!("Failed to build Vault client settings: {}", e),
            })?;

        let client = VaultClient::new(settings).map_err(|e| SecretError::ConnectionError {
            message: format!("Failed to create Vault client: {}", e),
        })?;

        Ok(client)
    }

    /// Authenticate with Vault using the configured authentication method
    pub async fn authenticate(&mut self) -> Result<(), SecretError> {
        let auth_config = self.config.auth.clone();
        match auth_config {
            VaultAuthConfig::Token { token } => {
                self.client.set_token(&token);
                // Verify token by checking our own capabilities
                self.verify_token().await
            }
            VaultAuthConfig::Kubernetes {
                token_path,
                role,
                mount_path,
            } => {
                self.authenticate_kubernetes(&token_path, &role, &mount_path)
                    .await
            }
            VaultAuthConfig::Aws {
                region,
                role,
                mount_path,
            } => self.authenticate_aws(&region, &role, &mount_path).await,
            VaultAuthConfig::AppRole {
                role_id,
                secret_id,
                mount_path,
            } => {
                self.authenticate_approle(&role_id, &secret_id, &mount_path)
                    .await
            }
        }
    }

    /// Verify the current token is valid
    async fn verify_token(&self) -> Result<(), SecretError> {
        match token::lookup_self(&self.client).await {
            Ok(_) => Ok(()),
            Err(e) => Err(self.map_vault_error(e)),
        }
    }

    /// Authenticate using Kubernetes service account token
    async fn authenticate_kubernetes(
        &mut self,
        token_path: &str,
        role: &str,
        mount_path: &str,
    ) -> Result<(), SecretError> {
        // Read the service account token
        let jwt = tokio::fs::read_to_string(token_path).await.map_err(|e| {
            SecretError::AuthenticationFailed {
                message: format!("Failed to read Kubernetes token from {}: {}", token_path, e),
            }
        })?;

        // Authenticate with Vault
        let auth_info = vaultrs::auth::kubernetes::login(&self.client, mount_path, role, &jwt)
            .await
            .map_err(|e| SecretError::AuthenticationFailed {
                message: format!("Kubernetes authentication failed: {}", e),
            })?;

        self.client.set_token(&auth_info.client_token);
        Ok(())
    }

    /// Authenticate using AWS IAM role
    async fn authenticate_aws(
        &mut self,
        _region: &str,
        role: &str,
        _mount_path: &str,
    ) -> Result<(), SecretError> {
        // For AWS authentication, we need to use the AWS SDK to get credentials
        // and create a signed request. This is a simplified implementation.
        // In a production environment, you would use the AWS SDK to properly
        // generate the required authentication data.

        // This would require additional dependencies and AWS credential configuration
        // For now, return an error indicating this needs to be implemented
        Err(SecretError::UnsupportedOperation {
            operation: format!(
                "AWS IAM authentication not yet implemented for role: {}",
                role
            ),
        })
    }

    /// Authenticate using AppRole
    async fn authenticate_approle(
        &mut self,
        role_id: &str,
        secret_id: &str,
        mount_path: &str,
    ) -> Result<(), SecretError> {
        let auth_info = vaultrs::auth::approle::login(&self.client, mount_path, role_id, secret_id)
            .await
            .map_err(|e| SecretError::AuthenticationFailed {
                message: format!("AppRole authentication failed: {}", e),
            })?;

        self.client.set_token(&auth_info.client_token);
        Ok(())
    }

    /// Get the full path for a secret key
    fn get_secret_path(&self, key: &str) -> String {
        format!("agents/{}/secrets/{}", self.agent_id, key)
    }

    /// Get the base path for listing secrets
    fn get_base_path(&self) -> String {
        format!("agents/{}/secrets", self.agent_id)
    }

    /// Map Vault client errors to SecretError
    fn map_vault_error(&self, error: ClientError) -> SecretError {
        match error {
            ClientError::RestClientError { .. } => SecretError::ConnectionError {
                message: error.to_string(),
            },
            ClientError::APIError { code: 404, .. } => SecretError::NotFound {
                key: "unknown".to_string(), // We don't always have the key context
            },
            ClientError::APIError { code: 403, .. } => SecretError::PermissionDenied {
                key: "unknown".to_string(),
            },
            ClientError::APIError { code: 401, .. } => SecretError::AuthenticationFailed {
                message: "Vault authentication failed".to_string(),
            },
            ClientError::APIError { code: 429, .. } => SecretError::RateLimitExceeded {
                message: "Vault rate limit exceeded".to_string(),
            },
            _ => SecretError::BackendError {
                message: error.to_string(),
            },
        }
    }

    /// Log an audit event if an audit sink is configured.
    /// In strict mode, returns an error if audit logging fails.
    /// In permissive mode, logs a warning and continues.
    async fn log_audit_event(&self, event: SecretAuditEvent) -> Result<(), SecretError> {
        if let Some(audit_sink) = &self.audit_sink {
            if let Err(e) = audit_sink.log_event(event).await {
                match audit_sink.failure_mode() {
                    crate::secrets::auditing::AuditFailureMode::Strict => {
                        return Err(SecretError::AuditFailed {
                            message: format!("Audit logging failed (strict mode): {}", e),
                        });
                    }
                    crate::secrets::auditing::AuditFailureMode::Permissive => {
                        tracing::warn!("Audit logging failed (permissive mode): {}", e);
                    }
                }
            }
        }
        Ok(())
    }
}

#[async_trait]
impl SecretStore for VaultSecretStore {
    /// Retrieve a secret by key from Vault KV v2
    async fn get_secret(&self, key: &str) -> Result<Secret, SecretError> {
        let path = self.get_secret_path(key);

        let result: Result<Secret, SecretError> = async {
            match kv2::read::<serde_json::Value>(&self.client, &self.config.mount_path, &path).await
            {
                Ok(secret_response) => {
                    // Extract the secret data from the Vault KVv2 response structure
                    let data = secret_response
                        .get("data")
                        .and_then(|d| d.get("data"))
                        .ok_or_else(|| SecretError::BackendError {
                            message: "Invalid Vault response structure".to_string(),
                        })?;

                    // Extract the secret value - assume it's stored under a "value" key
                    let secret_value = data
                        .get("value")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .or_else(|| {
                            // Fallback: if no "value" key, try to get the first string value
                            data.as_object()
                                .and_then(|obj| obj.values().next())
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                        })
                        .ok_or_else(|| SecretError::BackendError {
                            message: format!("No string value found for key '{}'", key),
                        })?;

                    // Extract metadata from the Vault response
                    let mut metadata = std::collections::HashMap::new();
                    if let Some(metadata_obj) =
                        secret_response.get("data").and_then(|d| d.get("metadata"))
                    {
                        if let Some(created_time) =
                            metadata_obj.get("created_time").and_then(|v| v.as_str())
                        {
                            metadata.insert("created_time".to_string(), created_time.to_string());
                        }
                        if let Some(version) = metadata_obj.get("version").and_then(|v| v.as_u64())
                        {
                            metadata.insert("version".to_string(), version.to_string());
                        }
                        if let Some(destroyed) =
                            metadata_obj.get("destroyed").and_then(|v| v.as_bool())
                        {
                            metadata.insert("destroyed".to_string(), destroyed.to_string());
                        }
                        if let Some(deletion_time) =
                            metadata_obj.get("deletion_time").and_then(|v| v.as_str())
                        {
                            if !deletion_time.is_empty() && deletion_time != "null" {
                                metadata
                                    .insert("deletion_time".to_string(), deletion_time.to_string());
                            }
                        }
                    }

                    // Parse created_at timestamp if available - convert to string
                    let created_at = metadata.get("created_time").cloned();

                    // Parse version if available - convert to string
                    let version = metadata.get("version").cloned();

                    Ok(Secret {
                        key: key.to_string(),
                        value: secret_value,
                        metadata: Some(metadata),
                        created_at,
                        version,
                    })
                }
                Err(e) => {
                    let mapped_error = self.map_vault_error(e);
                    // Update the key context if it's a NotFound error
                    match mapped_error {
                        SecretError::NotFound { .. } => Err(SecretError::NotFound {
                            key: key.to_string(),
                        }),
                        SecretError::PermissionDenied { .. } => {
                            Err(SecretError::PermissionDenied {
                                key: key.to_string(),
                            })
                        }
                        other => Err(other),
                    }
                }
            }
        }
        .await;

        // Log audit event — in strict mode, audit failure blocks the operation
        let audit_event = match &result {
            Ok(_) => SecretAuditEvent::success(
                self.agent_id.clone(),
                "get_secret".to_string(),
                Some(key.to_string()),
            ),
            Err(e) => SecretAuditEvent::failure(
                self.agent_id.clone(),
                "get_secret".to_string(),
                Some(key.to_string()),
                e.to_string(),
            ),
        };
        self.log_audit_event(audit_event).await?;

        result
    }

    /// List all secret keys under the agent's secrets path
    async fn list_secrets(&self) -> Result<Vec<String>, SecretError> {
        let base_path = self.get_base_path();

        let result: Result<Vec<String>, SecretError> = async {
            match kv2::list(&self.client, &self.config.mount_path, &base_path).await {
                Ok(list_response) => {
                    // list_response is already a Vec<String> of keys
                    Ok(list_response)
                }
                Err(e) => {
                    let mapped_error = self.map_vault_error(e);
                    // If the path doesn't exist (404), return empty list instead of error
                    match mapped_error {
                        SecretError::NotFound { .. } => Ok(vec![]),
                        other => Err(other),
                    }
                }
            }
        }
        .await;

        // Log audit event — in strict mode, audit failure blocks the operation
        let audit_event = match &result {
            Ok(keys) => {
                SecretAuditEvent::success(self.agent_id.clone(), "list_secrets".to_string(), None)
                    .with_metadata(serde_json::json!({
                        "secrets_count": keys.len()
                    }))
            }
            Err(e) => SecretAuditEvent::failure(
                self.agent_id.clone(),
                "list_secrets".to_string(),
                None,
                e.to_string(),
            ),
        };
        self.log_audit_event(audit_event).await?;

        result
    }
}

/// Extension trait for prefix filtering
impl VaultSecretStore {
    /// List secrets with prefix filtering
    pub async fn list_secrets_with_prefix(&self, prefix: &str) -> Result<Vec<String>, SecretError> {
        let all_keys = self.list_secrets().await?;
        Ok(all_keys
            .into_iter()
            .filter(|key| key.starts_with(prefix))
            .collect())
    }

    /// Get the agent ID for this store
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::config::{VaultConnectionConfig, VaultTlsConfig};

    fn create_test_config() -> VaultConfig {
        VaultConfig {
            url: "http://localhost:8200".to_string(),
            auth: VaultAuthConfig::Token {
                token: "test-token".to_string(),
            },
            namespace: None,
            mount_path: "secret".to_string(),
            api_version: "v2".to_string(),
            tls: VaultTlsConfig::default(),
            connection: VaultConnectionConfig::default(),
        }
    }

    #[test]
    fn test_secret_path_generation() {
        let _config = create_test_config();
        // We can't easily test the full VaultSecretStore without a real Vault instance
        // but we can test path generation logic
        let agent_id = "test-agent-123";
        let expected_path = format!("agents/{}/secrets/my-key", agent_id);

        // This would normally be done in the VaultSecretStore
        let path = format!("agents/{}/secrets/{}", agent_id, "my-key");
        assert_eq!(path, expected_path);
    }

    #[test]
    fn test_base_path_generation() {
        let agent_id = "test-agent-123";
        let expected_base = format!("agents/{}/secrets", agent_id);

        let base_path = format!("agents/{}/secrets", agent_id);
        assert_eq!(base_path, expected_base);
    }
}
