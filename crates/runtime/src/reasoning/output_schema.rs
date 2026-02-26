//! Output schema registry and management
//!
//! Provides `OutputSchema` for declaring expected response formats and
//! `SchemaRegistry` for storing versioned, pre-compiled schema validators
//! that can be provided to LLM API calls.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Describes the expected output format for an inference call.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum OutputSchema {
    /// Free-form text, no validation.
    #[serde(rename = "text")]
    Text,

    /// JSON object, validated for well-formedness only.
    #[serde(rename = "json_object")]
    JsonObject,

    /// JSON conforming to an explicit JSON Schema.
    #[serde(rename = "json_schema")]
    JsonSchema {
        /// The raw JSON Schema value.
        schema: serde_json::Value,
        /// Human-readable name for logging and API calls.
        name: String,
        /// Optional description.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
}

impl OutputSchema {
    /// Create a JSON Schema output from a raw schema value.
    pub fn json_schema(name: impl Into<String>, schema: serde_json::Value) -> Self {
        OutputSchema::JsonSchema {
            schema,
            name: name.into(),
            description: None,
        }
    }

    /// Create a JSON Schema output with description.
    pub fn json_schema_with_description(
        name: impl Into<String>,
        schema: serde_json::Value,
        description: impl Into<String>,
    ) -> Self {
        OutputSchema::JsonSchema {
            schema,
            name: name.into(),
            description: Some(description.into()),
        }
    }

    /// Get the JSON Schema value if this is a schema variant.
    pub fn schema_value(&self) -> Option<&serde_json::Value> {
        match self {
            OutputSchema::JsonSchema { schema, .. } => Some(schema),
            _ => None,
        }
    }

    /// Convert to the InferenceOptions ResponseFormat.
    pub fn to_response_format(&self) -> crate::reasoning::inference::ResponseFormat {
        match self {
            OutputSchema::Text => crate::reasoning::inference::ResponseFormat::Text,
            OutputSchema::JsonObject => crate::reasoning::inference::ResponseFormat::JsonObject,
            OutputSchema::JsonSchema { schema, name, .. } => {
                crate::reasoning::inference::ResponseFormat::JsonSchema {
                    schema: schema.clone(),
                    name: Some(name.clone()),
                }
            }
        }
    }
}

/// A versioned entry in the schema registry.
#[derive(Debug, Clone)]
struct SchemaEntry {
    /// The raw schema.
    schema: serde_json::Value,
    /// Pre-compiled validator for fast validation.
    validator: Arc<jsonschema::Validator>,
    /// Human-readable name.
    name: String,
    /// Optional description.
    description: Option<String>,
}

/// Registry key combining name and version.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct SchemaKey {
    name: String,
    version: String,
}

/// Thread-safe registry of versioned, pre-compiled JSON Schema validators.
///
/// Schemas are registered once, compiled into validators, and reused across
/// many validation calls. This amortizes the cost of schema compilation
/// (typically 10-100μs) over the lifetime of the application.
#[derive(Clone)]
pub struct SchemaRegistry {
    schemas: Arc<RwLock<HashMap<SchemaKey, SchemaEntry>>>,
    /// Tracks the latest version for each schema name.
    latest_versions: Arc<RwLock<HashMap<String, String>>>,
}

impl Default for SchemaRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SchemaRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            schemas: Arc::new(RwLock::new(HashMap::new())),
            latest_versions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a schema with a name and version.
    ///
    /// The schema is compiled into a validator at registration time.
    /// Returns an error if the schema is invalid.
    pub async fn register(
        &self,
        name: impl Into<String>,
        version: impl Into<String>,
        schema: serde_json::Value,
        description: Option<String>,
    ) -> Result<(), SchemaRegistryError> {
        let name = name.into();
        let version = version.into();

        let validator =
            jsonschema::validator_for(&schema).map_err(|e| SchemaRegistryError::InvalidSchema {
                name: name.clone(),
                reason: e.to_string(),
            })?;

        let key = SchemaKey {
            name: name.clone(),
            version: version.clone(),
        };
        let entry = SchemaEntry {
            schema,
            validator: Arc::new(validator),
            name: name.clone(),
            description,
        };

        self.schemas.write().await.insert(key, entry);
        self.latest_versions.write().await.insert(name, version);

        Ok(())
    }

    /// Get the pre-compiled validator for a specific schema version.
    pub async fn get_validator(
        &self,
        name: &str,
        version: &str,
    ) -> Option<Arc<jsonschema::Validator>> {
        let key = SchemaKey {
            name: name.into(),
            version: version.into(),
        };
        self.schemas
            .read()
            .await
            .get(&key)
            .map(|e| Arc::clone(&e.validator))
    }

    /// Get the pre-compiled validator for the latest version of a schema.
    pub async fn get_latest_validator(&self, name: &str) -> Option<Arc<jsonschema::Validator>> {
        let version = self.latest_versions.read().await.get(name).cloned()?;
        self.get_validator(name, &version).await
    }

    /// Get the raw schema value for a specific version.
    pub async fn get_schema(&self, name: &str, version: &str) -> Option<serde_json::Value> {
        let key = SchemaKey {
            name: name.into(),
            version: version.into(),
        };
        self.schemas
            .read()
            .await
            .get(&key)
            .map(|e| e.schema.clone())
    }

    /// Get the schema as an OutputSchema for the latest version.
    pub async fn get_output_schema(&self, name: &str) -> Option<OutputSchema> {
        let version = self.latest_versions.read().await.get(name).cloned()?;
        let key = SchemaKey {
            name: name.into(),
            version,
        };
        let schemas = self.schemas.read().await;
        let entry = schemas.get(&key)?;
        Some(OutputSchema::JsonSchema {
            schema: entry.schema.clone(),
            name: entry.name.clone(),
            description: entry.description.clone(),
        })
    }

    /// List all registered schema names with their latest versions.
    pub async fn list_schemas(&self) -> Vec<(String, String)> {
        self.latest_versions
            .read()
            .await
            .iter()
            .map(|(name, version)| (name.clone(), version.clone()))
            .collect()
    }

    /// Remove a schema version from the registry.
    pub async fn remove(&self, name: &str, version: &str) -> bool {
        let key = SchemaKey {
            name: name.into(),
            version: version.into(),
        };
        let removed = self.schemas.write().await.remove(&key).is_some();
        if removed {
            // If this was the latest version, find the next latest
            let mut latest = self.latest_versions.write().await;
            if latest.get(name).is_some_and(|v| v == version) {
                // Find another version for this name
                let schemas = self.schemas.read().await;
                let next_version = schemas
                    .keys()
                    .filter(|k| k.name == name)
                    .map(|k| k.version.clone())
                    .max();
                match next_version {
                    Some(v) => {
                        latest.insert(name.into(), v);
                    }
                    None => {
                        latest.remove(name);
                    }
                }
            }
        }
        removed
    }
}

/// Errors from the schema registry.
#[derive(Debug, thiserror::Error)]
pub enum SchemaRegistryError {
    #[error("Invalid schema '{name}': {reason}")]
    InvalidSchema { name: String, reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_output_schema_text() {
        let schema = OutputSchema::Text;
        assert!(schema.schema_value().is_none());
    }

    #[test]
    fn test_output_schema_json_schema() {
        let schema = OutputSchema::json_schema("test", serde_json::json!({"type": "object"}));
        assert!(schema.schema_value().is_some());
    }

    #[test]
    fn test_output_schema_serde_roundtrip() {
        let schema = OutputSchema::json_schema_with_description(
            "Result",
            serde_json::json!({
                "type": "object",
                "properties": {"value": {"type": "string"}}
            }),
            "A simple result",
        );
        let json = serde_json::to_string(&schema).unwrap();
        let restored: OutputSchema = serde_json::from_str(&json).unwrap();
        assert!(restored.schema_value().is_some());
    }

    #[test]
    fn test_output_schema_to_response_format() {
        let text = OutputSchema::Text;
        assert!(matches!(
            text.to_response_format(),
            crate::reasoning::inference::ResponseFormat::Text
        ));

        let json_obj = OutputSchema::JsonObject;
        assert!(matches!(
            json_obj.to_response_format(),
            crate::reasoning::inference::ResponseFormat::JsonObject
        ));

        let schema = OutputSchema::json_schema("test", serde_json::json!({"type": "object"}));
        assert!(matches!(
            schema.to_response_format(),
            crate::reasoning::inference::ResponseFormat::JsonSchema { .. }
        ));
    }

    #[tokio::test]
    async fn test_schema_registry_register_and_get() {
        let registry = SchemaRegistry::new();

        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"}
            },
            "required": ["name"]
        });

        registry
            .register("test_schema", "1.0.0", schema.clone(), None)
            .await
            .unwrap();

        // Get specific version
        let validator = registry.get_validator("test_schema", "1.0.0").await;
        assert!(validator.is_some());

        // Get latest version
        let latest = registry.get_latest_validator("test_schema").await;
        assert!(latest.is_some());

        // Get raw schema
        let raw = registry.get_schema("test_schema", "1.0.0").await;
        assert!(raw.is_some());
        assert_eq!(raw.unwrap(), schema);
    }

    #[tokio::test]
    async fn test_schema_registry_versioning() {
        let registry = SchemaRegistry::new();

        let v1 = serde_json::json!({"type": "object", "properties": {"a": {"type": "string"}}});
        let v2 = serde_json::json!({"type": "object", "properties": {"a": {"type": "string"}, "b": {"type": "number"}}});

        registry
            .register("schema", "1.0.0", v1.clone(), None)
            .await
            .unwrap();
        registry
            .register("schema", "2.0.0", v2.clone(), None)
            .await
            .unwrap();

        // Latest should be v2
        let latest_schema = registry.get_schema("schema", "2.0.0").await;
        assert_eq!(latest_schema.unwrap(), v2);

        // Both versions accessible
        assert!(registry.get_validator("schema", "1.0.0").await.is_some());
        assert!(registry.get_validator("schema", "2.0.0").await.is_some());
    }

    #[tokio::test]
    async fn test_schema_registry_invalid_schema() {
        let registry = SchemaRegistry::new();

        // A schema with an invalid type should fail
        let invalid = serde_json::json!({"type": "not_a_real_type"});
        let result = registry.register("bad", "1.0.0", invalid, None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_schema_registry_list() {
        let registry = SchemaRegistry::new();

        registry
            .register("a", "1.0.0", serde_json::json!({"type": "object"}), None)
            .await
            .unwrap();
        registry
            .register("b", "1.0.0", serde_json::json!({"type": "string"}), None)
            .await
            .unwrap();

        let schemas = registry.list_schemas().await;
        assert_eq!(schemas.len(), 2);
    }

    #[tokio::test]
    async fn test_schema_registry_remove() {
        let registry = SchemaRegistry::new();

        registry
            .register(
                "rm_test",
                "1.0.0",
                serde_json::json!({"type": "object"}),
                None,
            )
            .await
            .unwrap();
        registry
            .register(
                "rm_test",
                "2.0.0",
                serde_json::json!({"type": "object"}),
                None,
            )
            .await
            .unwrap();

        assert!(registry.remove("rm_test", "2.0.0").await);
        // Latest should now fall back
        assert!(registry.get_validator("rm_test", "1.0.0").await.is_some());
        assert!(registry.get_validator("rm_test", "2.0.0").await.is_none());
    }

    #[tokio::test]
    async fn test_schema_registry_get_output_schema() {
        let registry = SchemaRegistry::new();

        registry
            .register(
                "output",
                "1.0.0",
                serde_json::json!({"type": "object"}),
                Some("Test output".into()),
            )
            .await
            .unwrap();

        let output = registry.get_output_schema("output").await;
        assert!(output.is_some());
        match output.unwrap() {
            OutputSchema::JsonSchema {
                name, description, ..
            } => {
                assert_eq!(name, "output");
                assert_eq!(description.as_deref(), Some("Test output"));
            }
            _ => panic!("Expected JsonSchema variant"),
        }
    }

    #[tokio::test]
    async fn test_schema_registry_nonexistent() {
        let registry = SchemaRegistry::new();
        assert!(registry.get_validator("nope", "1.0.0").await.is_none());
        assert!(registry.get_latest_validator("nope").await.is_none());
        assert!(registry.get_output_schema("nope").await.is_none());
    }
}
