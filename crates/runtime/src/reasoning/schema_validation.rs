//! Schema-first validation pipeline
//!
//! Provides a layered validation pipeline for LLM output:
//! 1. Strip markdown fences
//! 2. Parse as JSON
//! 3. Validate against JSON Schema
//! 4. Deserialize into target Rust type
//!
//! Each layer produces actionable error messages that can be fed back to
//! the LLM as observations for self-correction.

use crate::reasoning::providers::slm::strip_markdown_fences;
use serde::de::DeserializeOwned;

/// Errors from the validation pipeline, ordered by severity.
///
/// Each variant contains an actionable message suitable for feeding
/// back to an LLM as an observation.
#[derive(Debug, thiserror::Error)]
pub enum SchemaValidationError {
    /// The raw text couldn't be parsed as JSON.
    #[error("JSON parse error at line {line}, column {column}: {message}. Raw text starts with: {raw_prefix:?}")]
    JsonParseError {
        message: String,
        line: usize,
        column: usize,
        raw_prefix: String,
    },

    /// The JSON is valid but doesn't conform to the expected schema.
    #[error("Schema validation failed: {errors:?}")]
    SchemaViolation { errors: Vec<String> },

    /// The JSON conforms to the schema but couldn't be deserialized into
    /// the target Rust type (usually a serde issue).
    #[error("Deserialization error: {message}")]
    DeserializationError { message: String },
}

impl SchemaValidationError {
    /// Format as a concise feedback message for the LLM.
    pub fn to_llm_feedback(&self) -> String {
        match self {
            SchemaValidationError::JsonParseError {
                message,
                line,
                column,
                ..
            } => {
                format!(
                    "Your response was not valid JSON. Error at line {}, column {}: {}. Please respond with a valid JSON object.",
                    line, column, message
                )
            }
            SchemaValidationError::SchemaViolation { errors } => {
                let error_list = errors.join("; ");
                format!(
                    "Your JSON response did not match the required schema. Issues: {}. Please fix these and try again.",
                    error_list
                )
            }
            SchemaValidationError::DeserializationError { message } => {
                format!(
                    "Your JSON had the right structure but contained invalid values: {}. Please correct the values.",
                    message
                )
            }
        }
    }
}

/// The validation pipeline: parses, validates, and deserializes LLM output.
///
/// Supports two modes:
/// - **Static (typed)**: `validate_and_parse::<T>()` for compile-time Rust types
/// - **Dynamic**: `validate_dynamic()` for runtime-defined schemas from the DSL
///
/// The dynamic path validates `serde_json::Value` against a JSON Schema without
/// requiring a Rust type, which is essential for user-defined output shapes
/// composed at runtime via the DSL.
pub struct ValidationPipeline;

impl ValidationPipeline {
    /// Run the full validation pipeline with static typing:
    /// strip fences → parse JSON → validate → deserialize into `T`.
    ///
    /// Use this when you have a compile-time Rust type for the output.
    pub fn validate_and_parse<T: DeserializeOwned>(
        raw_text: &str,
        schema: Option<&jsonschema::Validator>,
    ) -> Result<T, SchemaValidationError> {
        let json_value = Self::parse_and_validate(raw_text, schema)?;

        // Deserialize into target type
        serde_json::from_value(json_value).map_err(|e| {
            SchemaValidationError::DeserializationError {
                message: e.to_string(),
            }
        })
    }

    /// Run the validation pipeline for dynamic schemas:
    /// strip fences → parse JSON → validate against schema → return Value.
    ///
    /// Use this when output shapes are defined at runtime (e.g., from the DSL).
    /// The returned `serde_json::Value` is guaranteed to conform to the schema.
    pub fn validate_dynamic(
        raw_text: &str,
        schema: Option<&jsonschema::Validator>,
    ) -> Result<serde_json::Value, SchemaValidationError> {
        Self::parse_and_validate(raw_text, schema)
    }

    /// Common pipeline: strip fences → parse JSON → validate against schema.
    fn parse_and_validate(
        raw_text: &str,
        schema: Option<&jsonschema::Validator>,
    ) -> Result<serde_json::Value, SchemaValidationError> {
        // Step 1: Strip markdown fences
        let cleaned = strip_markdown_fences(raw_text);

        // Step 2: Parse as JSON
        let json_value: serde_json::Value = serde_json::from_str(&cleaned).map_err(|e| {
            let prefix = if cleaned.len() > 100 {
                format!("{}...", &cleaned[..100])
            } else {
                cleaned.clone()
            };
            SchemaValidationError::JsonParseError {
                message: e.to_string(),
                line: e.line(),
                column: e.column(),
                raw_prefix: prefix,
            }
        })?;

        // Step 3: Validate against schema if provided
        if let Some(validator) = schema {
            Self::check_schema_errors(&json_value, validator)?;
        }

        Ok(json_value)
    }

    /// Validate a JSON value against a pre-compiled schema and collect errors.
    fn check_schema_errors(
        value: &serde_json::Value,
        validator: &jsonschema::Validator,
    ) -> Result<(), SchemaValidationError> {
        let errors: Vec<String> = validator
            .iter_errors(value)
            .map(|e| {
                let path = e.instance_path.to_string();
                if path.is_empty() {
                    e.to_string()
                } else {
                    format!("at '{}': {}", path, e)
                }
            })
            .collect();

        if errors.is_empty() {
            Ok(())
        } else {
            Err(SchemaValidationError::SchemaViolation { errors })
        }
    }

    /// Parse raw text as JSON without schema validation.
    pub fn parse_json(raw_text: &str) -> Result<serde_json::Value, SchemaValidationError> {
        let cleaned = strip_markdown_fences(raw_text);
        serde_json::from_str(&cleaned).map_err(|e| {
            let prefix = if cleaned.len() > 100 {
                format!("{}...", &cleaned[..100])
            } else {
                cleaned.clone()
            };
            SchemaValidationError::JsonParseError {
                message: e.to_string(),
                line: e.line(),
                column: e.column(),
                raw_prefix: prefix,
            }
        })
    }

    /// Validate a JSON value against a pre-compiled schema.
    pub fn validate_schema(
        value: &serde_json::Value,
        validator: &jsonschema::Validator,
    ) -> Result<(), SchemaValidationError> {
        Self::check_schema_errors(value, validator)
    }

    /// Create a validator from a raw JSON Schema value.
    ///
    /// This is the primary way to create validators for dynamic schemas
    /// defined at runtime (e.g., from DSL configurations or TOML pipelines).
    pub fn compile_schema(
        schema: &serde_json::Value,
    ) -> Result<jsonschema::Validator, SchemaValidationError> {
        jsonschema::validator_for(schema).map_err(|e| SchemaValidationError::SchemaViolation {
            errors: vec![format!("Invalid schema: {}", e)],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq)]
    struct TestOutput {
        answer: String,
        confidence: f64,
    }

    fn make_validator(schema: &serde_json::Value) -> jsonschema::Validator {
        jsonschema::validator_for(schema).expect("valid schema")
    }

    #[test]
    fn test_validate_and_parse_valid() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "answer": {"type": "string"},
                "confidence": {"type": "number", "minimum": 0.0, "maximum": 1.0}
            },
            "required": ["answer", "confidence"]
        });
        let validator = make_validator(&schema);

        let raw = r#"{"answer": "42", "confidence": 0.95}"#;
        let result: TestOutput =
            ValidationPipeline::validate_and_parse(raw, Some(&validator)).unwrap();
        assert_eq!(result.answer, "42");
        assert!((result.confidence - 0.95).abs() < f64::EPSILON);
    }

    #[test]
    fn test_validate_and_parse_markdown_fenced() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "answer": {"type": "string"},
                "confidence": {"type": "number"}
            },
            "required": ["answer", "confidence"]
        });
        let validator = make_validator(&schema);

        let raw = "```json\n{\"answer\": \"hello\", \"confidence\": 0.8}\n```";
        let result: TestOutput =
            ValidationPipeline::validate_and_parse(raw, Some(&validator)).unwrap();
        assert_eq!(result.answer, "hello");
    }

    #[test]
    fn test_validate_and_parse_invalid_json() {
        let raw = "This is not JSON at all";
        let result = ValidationPipeline::validate_and_parse::<TestOutput>(raw, None);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, SchemaValidationError::JsonParseError { .. }));

        let feedback = err.to_llm_feedback();
        assert!(feedback.contains("not valid JSON"));
    }

    #[test]
    fn test_validate_and_parse_schema_violation() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "answer": {"type": "string"},
                "confidence": {"type": "number", "minimum": 0.0, "maximum": 1.0}
            },
            "required": ["answer", "confidence"]
        });
        let validator = make_validator(&schema);

        // Missing required field "confidence"
        let raw = r#"{"answer": "42"}"#;
        let result = ValidationPipeline::validate_and_parse::<TestOutput>(raw, Some(&validator));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, SchemaValidationError::SchemaViolation { .. }));

        let feedback = err.to_llm_feedback();
        assert!(feedback.contains("did not match the required schema"));
    }

    #[test]
    fn test_validate_and_parse_out_of_range() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "answer": {"type": "string"},
                "confidence": {"type": "number", "minimum": 0.0, "maximum": 1.0}
            },
            "required": ["answer", "confidence"]
        });
        let validator = make_validator(&schema);

        // confidence out of range
        let raw = r#"{"answer": "42", "confidence": 1.5}"#;
        let result = ValidationPipeline::validate_and_parse::<TestOutput>(raw, Some(&validator));
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SchemaValidationError::SchemaViolation { .. }
        ));
    }

    #[test]
    fn test_validate_and_parse_no_schema() {
        let raw = r#"{"answer": "hello", "confidence": 0.5}"#;
        let result: TestOutput = ValidationPipeline::validate_and_parse(raw, None).unwrap();
        assert_eq!(result.answer, "hello");
    }

    #[test]
    fn test_parse_json_standalone() {
        let raw = "```json\n{\"key\": \"value\"}\n```";
        let value = ValidationPipeline::parse_json(raw).unwrap();
        assert_eq!(value["key"], "value");
    }

    #[test]
    fn test_validate_schema_standalone() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["name"]
        });
        let validator = make_validator(&schema);

        let valid = serde_json::json!({"name": "test"});
        assert!(ValidationPipeline::validate_schema(&valid, &validator).is_ok());

        let invalid = serde_json::json!({"other": "field"});
        assert!(ValidationPipeline::validate_schema(&invalid, &validator).is_err());
    }

    #[test]
    fn test_error_feedback_messages() {
        let json_err = SchemaValidationError::JsonParseError {
            message: "expected value".into(),
            line: 1,
            column: 1,
            raw_prefix: "bad input".into(),
        };
        let feedback = json_err.to_llm_feedback();
        assert!(feedback.contains("not valid JSON"));
        assert!(feedback.contains("line 1"));

        let schema_err = SchemaValidationError::SchemaViolation {
            errors: vec!["missing field 'name'".into()],
        };
        let feedback = schema_err.to_llm_feedback();
        assert!(feedback.contains("missing field 'name'"));

        let deser_err = SchemaValidationError::DeserializationError {
            message: "invalid type: string, expected f64".into(),
        };
        let feedback = deser_err.to_llm_feedback();
        assert!(feedback.contains("invalid values"));
    }

    #[test]
    fn test_validate_dynamic_valid() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "result": {"type": "string"},
                "score": {"type": "number"}
            },
            "required": ["result"]
        });
        let validator = make_validator(&schema);

        let raw = r#"{"result": "success", "score": 95.5}"#;
        let value = ValidationPipeline::validate_dynamic(raw, Some(&validator)).unwrap();
        assert_eq!(value["result"], "success");
        assert_eq!(value["score"], 95.5);
    }

    #[test]
    fn test_validate_dynamic_invalid() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"}
            },
            "required": ["name"]
        });
        let validator = make_validator(&schema);

        let raw = r#"{"other": "field"}"#;
        let result = ValidationPipeline::validate_dynamic(raw, Some(&validator));
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_dynamic_arbitrary_shape() {
        // Simulate a DSL-defined output schema at runtime
        let user_defined_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "tasks": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {"type": "integer"},
                            "description": {"type": "string"},
                            "priority": {"type": "string", "enum": ["low", "medium", "high"]}
                        },
                        "required": ["id", "description"]
                    }
                },
                "summary": {"type": "string"}
            },
            "required": ["tasks", "summary"]
        });
        let validator = make_validator(&user_defined_schema);

        let raw = r#"{"tasks": [{"id": 1, "description": "Do thing", "priority": "high"}], "summary": "One task"}"#;
        let value = ValidationPipeline::validate_dynamic(raw, Some(&validator)).unwrap();
        assert_eq!(value["tasks"][0]["priority"], "high");
        assert_eq!(value["summary"], "One task");

        // Invalid: wrong priority enum value
        let bad = r#"{"tasks": [{"id": 1, "description": "Do thing", "priority": "urgent"}], "summary": "x"}"#;
        let result = ValidationPipeline::validate_dynamic(bad, Some(&validator));
        assert!(result.is_err());
    }

    #[test]
    fn test_compile_schema_valid() {
        let schema = serde_json::json!({"type": "object"});
        assert!(ValidationPipeline::compile_schema(&schema).is_ok());
    }

    #[test]
    fn test_compile_schema_invalid() {
        let schema = serde_json::json!({"type": "not_a_type"});
        assert!(ValidationPipeline::compile_schema(&schema).is_err());
    }

    #[test]
    fn test_validator_performance() {
        // Verify that pre-compiled validators are fast (<100μs for typical schemas)
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "name": {"type": "string", "maxLength": 100},
                "score": {"type": "number", "minimum": 0, "maximum": 100},
                "tags": {"type": "array", "items": {"type": "string"}},
                "metadata": {
                    "type": "object",
                    "properties": {
                        "source": {"type": "string"},
                        "timestamp": {"type": "string"}
                    }
                }
            },
            "required": ["name", "score"]
        });
        let validator = make_validator(&schema);

        let valid_input = serde_json::json!({
            "name": "test agent output",
            "score": 85.5,
            "tags": ["analysis", "research"],
            "metadata": {"source": "web", "timestamp": "2024-01-01T00:00:00Z"}
        });

        let start = std::time::Instant::now();
        for _ in 0..1000 {
            let _ = ValidationPipeline::validate_schema(&valid_input, &validator);
        }
        let elapsed = start.elapsed();
        let per_validation = elapsed / 1000;

        // Pre-compiled validator should be well under 100μs per validation
        assert!(
            per_validation.as_micros() < 100,
            "Validation took {}μs, expected <100μs",
            per_validation.as_micros()
        );
    }
}
