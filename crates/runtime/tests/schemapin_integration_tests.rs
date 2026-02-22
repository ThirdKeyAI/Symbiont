//! SchemaPin Integration Tests
//!
//! Tests for the SchemaPin native client integration

use std::collections::HashMap;

use symbi_runtime::integrations::schemapin::{
    KeyStoreConfig, KeyStoreError, LocalKeyStore, MockNativeSchemaPinClient, NativeSchemaPinClient,
    PinnedKey, SchemaPinClient, SchemaPinError, SignatureInfo, VerificationResult, VerifyArgs,
};

#[tokio::test]
async fn test_mock_schemapin_cli_success() {
    let cli = MockNativeSchemaPinClient::new_success();

    let args = VerifyArgs::new(
        "/tmp/test_schema.json".to_string(),
        "https://example.com/pubkey".to_string(),
    );

    let result = cli.verify_schema(args).await;
    assert!(result.is_ok());

    let verification = result.unwrap();
    assert!(verification.success);
    assert_eq!(verification.message, "Mock verification successful");
    assert_eq!(
        verification.schema_hash,
        Some("mock_native_hash_123".to_string())
    );
}

#[tokio::test]
async fn test_mock_schemapin_cli_failure() {
    let cli = MockNativeSchemaPinClient::new_failure();

    let args = VerifyArgs::new(
        "/tmp/test_schema.json".to_string(),
        "https://example.com/pubkey".to_string(),
    );

    let result = cli.verify_schema(args).await;
    assert!(result.is_err());

    match result.unwrap_err() {
        SchemaPinError::VerificationFailed { reason } => {
            assert_eq!(reason, "Mock verification failed");
        }
        _ => panic!("Expected VerificationFailed error"),
    }
}

#[tokio::test]
async fn test_mock_schemapin_cli_custom_result() {
    let custom_result = VerificationResult {
        success: true,
        message: "Custom verification result".to_string(),
        schema_hash: Some("custom_hash_456".to_string()),
        public_key_url: Some("https://custom.example.com/pubkey".to_string()),
        signature: Some(SignatureInfo {
            algorithm: "Ed25519".to_string(),
            signature: "mock_signature".to_string(),
            key_fingerprint: Some("mock_fingerprint".to_string()),
            valid: true,
        }),
        metadata: Some({
            let mut map = HashMap::new();
            map.insert(
                "version".to_string(),
                serde_json::Value::String("1.0.0".to_string()),
            );
            map.insert(
                "author".to_string(),
                serde_json::Value::String("test_author".to_string()),
            );
            map
        }),
        timestamp: Some("2024-12-07T14:30:00Z".to_string()),
    };

    let cli = MockNativeSchemaPinClient::with_result(custom_result.clone());

    let args = VerifyArgs::new(
        "/tmp/test_schema.json".to_string(),
        "https://example.com/pubkey".to_string(),
    );

    let result = cli.verify_schema(args).await.unwrap();
    assert_eq!(result.message, "Custom verification result");
    assert_eq!(result.schema_hash, Some("custom_hash_456".to_string()));
    assert!(result.signature.is_some());

    let signature = result.signature.unwrap();
    assert_eq!(signature.algorithm, "Ed25519");
    assert!(signature.valid);
}

#[tokio::test]
async fn test_mock_cli_version_and_binary_check() {
    let cli = MockNativeSchemaPinClient::new_success();

    // Test version
    let version = cli.get_version().await.unwrap();
    assert_eq!(version, "schemapin-cli v1.0.0 (mock)");

    // Test availability check
    let available = cli.check_available().await.unwrap();
    assert!(available);
}

#[tokio::test]
async fn test_verify_args_validation() {
    let cli = NativeSchemaPinClient::new();

    // Test empty schema path
    let args = VerifyArgs::new("".to_string(), "https://example.com/pubkey".to_string());
    let result = cli.verify_schema(args).await;
    assert!(matches!(
        result,
        Err(SchemaPinError::InvalidArguments { .. })
    ));

    // Test empty public key URL
    let args = VerifyArgs::new("/tmp/test_schema.json".to_string(), "".to_string());
    let result = cli.verify_schema(args).await;
    assert!(matches!(
        result,
        Err(SchemaPinError::InvalidArguments { .. })
    ));

    // Test invalid public key URL format
    let args = VerifyArgs::new(
        "/tmp/test_schema.json".to_string(),
        "invalid-url-format".to_string(),
    );
    let result = cli.verify_schema(args).await;
    assert!(matches!(
        result,
        Err(SchemaPinError::InvalidPublicKeyUrl { .. })
    ));

    // Test non-existent schema file
    let args = VerifyArgs::new(
        "/non/existent/file.json".to_string(),
        "https://example.com/pubkey".to_string(),
    );
    let result = cli.verify_schema(args).await;
    assert!(matches!(
        result,
        Err(SchemaPinError::SchemaFileNotFound { .. })
    ));
}

#[tokio::test]
async fn test_verify_args_construction() {
    let args = VerifyArgs::new(
        "/path/to/schema.json".to_string(),
        "https://example.com/pubkey".to_string(),
    )
    .with_arg("--verbose".to_string())
    .with_arg("--format=json".to_string());

    assert_eq!(args.schema_path, "/path/to/schema.json");
    assert_eq!(args.public_key_url, "https://example.com/pubkey");
    assert_eq!(args.additional_args.len(), 2);
    assert_eq!(args.additional_args[0], "--verbose");
    assert_eq!(args.additional_args[1], "--format=json");

    let cmd_args = args.to_args();
    let expected = vec![
        "verify",
        "--schema",
        "/path/to/schema.json",
        "--public-key-url",
        "https://example.com/pubkey",
        "--verbose",
        "--format=json",
    ];
    assert_eq!(cmd_args, expected);
}

#[tokio::test]
async fn test_native_client_no_binary_dependency() {
    // Native client doesn't depend on external binaries, so it should always be available
    let cli = NativeSchemaPinClient::new();

    // Test that availability check always succeeds
    let available = cli.check_available().await.unwrap();
    assert!(available);

    // Test that version can be retrieved
    let version = cli.get_version().await.unwrap();
    assert!(version.contains("schemapin-native"));
}

#[test]
fn test_error_types() {
    // Test error creation and formatting
    let error = SchemaPinError::ExecutionFailed {
        reason: "Command failed with exit code 1".to_string(),
    };
    assert_eq!(
        error.to_string(),
        "CLI execution failed: Command failed with exit code 1"
    );

    let error = SchemaPinError::BinaryNotFound {
        path: "/path/to/binary".to_string(),
    };
    assert_eq!(
        error.to_string(),
        "Binary not found at path: /path/to/binary"
    );

    let error = SchemaPinError::VerificationFailed {
        reason: "Signature verification failed".to_string(),
    };
    assert_eq!(
        error.to_string(),
        "Verification failed: Signature verification failed"
    );

    let error = SchemaPinError::Timeout { seconds: 30 };
    assert_eq!(error.to_string(), "Timeout occurred after 30 seconds");
}

#[test]
fn test_verification_result_serialization() {
    let result = VerificationResult {
        success: true,
        message: "Verification successful".to_string(),
        schema_hash: Some("abc123".to_string()),
        public_key_url: Some("https://example.com/pubkey".to_string()),
        signature: Some(SignatureInfo {
            algorithm: "Ed25519".to_string(),
            signature: "signature_data".to_string(),
            key_fingerprint: Some("fingerprint".to_string()),
            valid: true,
        }),
        metadata: None,
        timestamp: Some("2024-12-07T14:30:00Z".to_string()),
    };

    // Test serialization
    let json = serde_json::to_string(&result).unwrap();
    assert!(json.contains("\"success\":true"));
    assert!(json.contains("\"message\":\"Verification successful\""));

    // Test deserialization
    let deserialized: VerificationResult = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.success, result.success);
    assert_eq!(deserialized.message, result.message);
    assert_eq!(deserialized.schema_hash, result.schema_hash);
}

// ============================================================================
// Key Store Integration Tests
// ============================================================================

#[tokio::test]
async fn test_key_store_creation_and_basic_operations() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let store_path = temp_dir.path().join("test_keys.json");

    let config = KeyStoreConfig {
        store_path,
        create_if_missing: true,
        file_permissions: Some(0o600),
    };

    let store = LocalKeyStore::with_config(config).unwrap();

    // Test basic operations
    assert_eq!(store.list_identifiers().unwrap().len(), 0);
    assert!(!store.has_key("example.com").unwrap());

    let key = PinnedKey::new(
        "example.com".to_string(),
        "test_public_key".to_string(),
        "Ed25519".to_string(),
        "test_fingerprint".to_string(),
    );

    store.pin_key(key.clone()).unwrap();
    assert!(store.has_key("example.com").unwrap());

    let retrieved_key = store.get_key("example.com").unwrap();
    assert_eq!(retrieved_key.identifier, key.identifier);
    assert_eq!(retrieved_key.public_key, key.public_key);
}

#[tokio::test]
async fn test_key_store_tofu_mechanism() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let store_path = temp_dir.path().join("tofu_test.json");

    let config = KeyStoreConfig {
        store_path,
        create_if_missing: true,
        file_permissions: Some(0o600),
    };

    let store = LocalKeyStore::with_config(config).unwrap();

    let key1 = PinnedKey::new(
        "example.com".to_string(),
        "first_public_key".to_string(),
        "Ed25519".to_string(),
        "first_fingerprint".to_string(),
    );

    let key2 = PinnedKey::new(
        "example.com".to_string(),
        "different_public_key".to_string(),
        "Ed25519".to_string(),
        "different_fingerprint".to_string(),
    );

    // First key should pin successfully
    store.pin_key(key1.clone()).unwrap();

    // Second different key should fail with KeyMismatch
    let result = store.pin_key(key2);
    assert!(matches!(result, Err(KeyStoreError::KeyMismatch { .. })));

    // Same key should succeed
    store.pin_key(key1).unwrap();
}

#[tokio::test]
async fn test_key_store_persistence_across_instances() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let store_path = temp_dir.path().join("persistence_test.json");

    let config = KeyStoreConfig {
        store_path: store_path.clone(),
        create_if_missing: true,
        file_permissions: Some(0o600),
    };

    // Create first store instance and add keys
    {
        let store = LocalKeyStore::with_config(config.clone()).unwrap();

        let key1 = PinnedKey::new(
            "example.com".to_string(),
            "key1".to_string(),
            "Ed25519".to_string(),
            "fingerprint1".to_string(),
        );

        let key2 = PinnedKey::new(
            "test.org".to_string(),
            "key2".to_string(),
            "RSA".to_string(),
            "fingerprint2".to_string(),
        );

        store.pin_key(key1).unwrap();
        store.pin_key(key2).unwrap();

        assert_eq!(store.list_identifiers().unwrap().len(), 2);
    }

    // Create second store instance and verify persistence
    {
        let store = LocalKeyStore::with_config(config).unwrap();

        assert_eq!(store.list_identifiers().unwrap().len(), 2);
        assert!(store.has_key("example.com").unwrap());
        assert!(store.has_key("test.org").unwrap());

        let key1 = store.get_key("example.com").unwrap();
        assert_eq!(key1.public_key, "key1");
        assert_eq!(key1.algorithm, "Ed25519");

        let key2 = store.get_key("test.org").unwrap();
        assert_eq!(key2.public_key, "key2");
        assert_eq!(key2.algorithm, "RSA");
    }
}

#[tokio::test]
async fn test_key_store_concurrent_access() {
    use std::sync::Arc;
    use tokio::task;

    let temp_dir = tempfile::TempDir::new().unwrap();
    let store_path = temp_dir.path().join("concurrent_test.json");

    let config = KeyStoreConfig {
        store_path,
        create_if_missing: true,
        file_permissions: Some(0o600),
    };

    let store = Arc::new(LocalKeyStore::with_config(config).unwrap());

    // Spawn multiple tasks that try to pin keys concurrently
    let mut handles = vec![];

    for i in 0..10 {
        let store_clone = Arc::clone(&store);
        let handle = task::spawn(async move {
            let key = PinnedKey::new(
                format!("domain{}.com", i),
                format!("key_{}", i),
                "Ed25519".to_string(),
                format!("fingerprint_{}", i),
            );
            store_clone.pin_key(key).unwrap();
        });
        handles.push(handle);
    }

    // Wait for all tasks to complete
    for handle in handles {
        handle.await.unwrap();
    }

    // Verify all keys were pinned
    assert_eq!(store.list_identifiers().unwrap().len(), 10);

    for i in 0..10 {
        assert!(store.has_key(&format!("domain{}.com", i)).unwrap());
    }
}

#[tokio::test]
async fn test_key_store_verification_operations() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let store_path = temp_dir.path().join("verification_test.json");

    let config = KeyStoreConfig {
        store_path,
        create_if_missing: true,
        file_permissions: Some(0o600),
    };

    let store = LocalKeyStore::with_config(config).unwrap();

    let key = PinnedKey::new(
        "example.com".to_string(),
        "test_public_key".to_string(),
        "Ed25519".to_string(),
        "test_fingerprint".to_string(),
    );

    store.pin_key(key.clone()).unwrap();

    // Test successful verification
    assert!(store
        .verify_key("example.com", &key.public_key, &key.fingerprint)
        .unwrap());

    // Test failed verification with wrong key
    assert!(!store
        .verify_key("example.com", "wrong_key", &key.fingerprint)
        .unwrap());

    // Test failed verification with wrong fingerprint
    assert!(!store
        .verify_key("example.com", &key.public_key, "wrong_fingerprint")
        .unwrap());

    // Test verification for non-existent key
    let result = store.verify_key("nonexistent.com", &key.public_key, &key.fingerprint);
    assert!(matches!(result, Err(KeyStoreError::KeyNotFound { .. })));
}

#[tokio::test]
async fn test_key_store_removal_and_clearing() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let store_path = temp_dir.path().join("removal_test.json");

    let config = KeyStoreConfig {
        store_path,
        create_if_missing: true,
        file_permissions: Some(0o600),
    };

    let store = LocalKeyStore::with_config(config).unwrap();

    // Add multiple keys
    for i in 0..5 {
        let key = PinnedKey::new(
            format!("domain{}.com", i),
            format!("key_{}", i),
            "Ed25519".to_string(),
            format!("fingerprint_{}", i),
        );
        store.pin_key(key).unwrap();
    }

    assert_eq!(store.list_identifiers().unwrap().len(), 5);

    // Remove one key
    let removed_key = store.remove_key("domain2.com").unwrap();
    assert!(removed_key.is_some());
    assert_eq!(removed_key.unwrap().identifier, "domain2.com");
    assert_eq!(store.list_identifiers().unwrap().len(), 4);
    assert!(!store.has_key("domain2.com").unwrap());

    // Try to remove non-existent key
    let removed_key = store.remove_key("nonexistent.com").unwrap();
    assert!(removed_key.is_none());

    // Clear all keys
    store.clear().unwrap();
    assert_eq!(store.list_identifiers().unwrap().len(), 0);
}

#[tokio::test]
async fn test_key_store_with_metadata() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let store_path = temp_dir.path().join("metadata_test.json");

    let config = KeyStoreConfig {
        store_path,
        create_if_missing: true,
        file_permissions: Some(0o600),
    };

    let store = LocalKeyStore::with_config(config).unwrap();

    let mut metadata = HashMap::new();
    metadata.insert(
        "version".to_string(),
        serde_json::Value::String("1.0.0".to_string()),
    );
    metadata.insert(
        "source".to_string(),
        serde_json::Value::String("test_suite".to_string()),
    );
    metadata.insert(
        "priority".to_string(),
        serde_json::Value::Number(serde_json::Number::from(10)),
    );

    let key = PinnedKey::with_metadata(
        "example.com".to_string(),
        "test_public_key".to_string(),
        "Ed25519".to_string(),
        "test_fingerprint".to_string(),
        metadata.clone(),
    );

    store.pin_key(key).unwrap();

    let retrieved_key = store.get_key("example.com").unwrap();
    assert!(retrieved_key.metadata.is_some());

    let retrieved_metadata = retrieved_key.metadata.unwrap();
    assert_eq!(retrieved_metadata.get("version"), metadata.get("version"));
    assert_eq!(retrieved_metadata.get("source"), metadata.get("source"));
    assert_eq!(retrieved_metadata.get("priority"), metadata.get("priority"));
}

#[tokio::test]
async fn test_key_store_error_conditions() {
    // Test with non-existent directory and create_if_missing = false
    let temp_dir = tempfile::TempDir::new().unwrap();
    let store_path = temp_dir.path().join("nonexistent").join("test_keys.json");

    let config = KeyStoreConfig {
        store_path: store_path.clone(),
        create_if_missing: false,
        file_permissions: Some(0o600),
    };

    let result = LocalKeyStore::with_config(config);
    assert!(matches!(
        result,
        Err(KeyStoreError::StoreFileNotFound { .. })
    ));

    // Test with create_if_missing = true (should succeed)
    let config = KeyStoreConfig {
        store_path,
        create_if_missing: true,
        file_permissions: Some(0o600),
    };

    let store = LocalKeyStore::with_config(config).unwrap();
    assert_eq!(store.list_identifiers().unwrap().len(), 0);
}

#[test]
fn test_key_store_config_default() {
    let config = KeyStoreConfig::default();
    assert!(config.create_if_missing);
    assert_eq!(config.file_permissions, Some(0o600));
    assert!(config.store_path.to_string_lossy().contains(".symbiont"));
    assert!(config
        .store_path
        .to_string_lossy()
        .contains("schemapin_keys.json"));
}

#[test]
fn test_pinned_key_creation() {
    let key = PinnedKey::new(
        "example.com".to_string(),
        "test_key".to_string(),
        "Ed25519".to_string(),
        "test_fingerprint".to_string(),
    );

    assert_eq!(key.identifier, "example.com");
    assert_eq!(key.public_key, "test_key");
    assert_eq!(key.algorithm, "Ed25519");
    assert_eq!(key.fingerprint, "test_fingerprint");
    assert!(key.metadata.is_none());
    assert!(!key.pinned_at.is_empty());
}

#[test]
fn test_key_store_error_display() {
    let error = KeyStoreError::KeyNotFound {
        identifier: "example.com".to_string(),
    };
    assert_eq!(
        error.to_string(),
        "Key not found for identifier: example.com"
    );

    let error = KeyStoreError::KeyMismatch {
        identifier: "example.com".to_string(),
    };
    assert_eq!(
        error.to_string(),
        "Key mismatch for identifier: example.com"
    );

    let error = KeyStoreError::KeyAlreadyPinned {
        identifier: "example.com".to_string(),
    };
    assert_eq!(
        error.to_string(),
        "Key already pinned for identifier: example.com"
    );
}
