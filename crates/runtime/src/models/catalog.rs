//! Model catalog for managing SLM definitions and capabilities
//!
//! The [`ModelCatalog`] acts as a central registry for all available Small Language Models
//! in the Symbiont runtime. It provides efficient lookup and management of model
//! definitions, their capabilities, and resource requirements.
//!
//! # Usage
//!
//! ```rust
//! use symbi_runtime::models::ModelCatalog;
//! use symbi_runtime::config::Slm;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let slm_config = Slm::default();
//! let catalog = ModelCatalog::new(slm_config)?;
//!
//! // Look up a model by ID
//! if let Some(model) = catalog.get_model("llama2-7b") {
//!     println!("Model: {} - Capabilities: {:?}", model.name, model.capabilities);
//! }
//!
//! // List all available models
//! let models = catalog.list_models();
//! println!("Available models: {}", models.len());
//!
//! // Get models for a specific agent
//! let agent_models = catalog.get_models_for_agent("security_scanner");
//! # Ok(())
//! # }
//! ```

use crate::config::{Model, ModelCapability, SandboxProfile, Slm};
use std::collections::HashMap;
use thiserror::Error;

/// Errors that can occur when working with the model catalog
#[derive(Debug, Error)]
pub enum ModelCatalogError {
    #[error("Model not found: {id}")]
    ModelNotFound { id: String },

    #[error("Invalid model configuration: {reason}")]
    InvalidConfig { reason: String },

    #[error("Sandbox profile not found: {profile}")]
    SandboxProfileNotFound { profile: String },

    #[error("Agent has no associated models: {agent_id}")]
    NoModelsForAgent { agent_id: String },
}

/// Central registry for model definitions and capabilities
///
/// The `ModelCatalog` manages all available Small Language Models in the system,
/// providing efficient lookup operations and maintaining the mapping between
/// agents and their allowed models.
#[derive(Debug, Clone)]
pub struct ModelCatalog {
    /// Map of model ID to model definition
    models: HashMap<String, Model>,
    /// Map of agent ID to allowed model IDs
    agent_model_maps: HashMap<String, Vec<String>>,
    /// Available sandbox profiles for model execution
    sandbox_profiles: HashMap<String, SandboxProfile>,
    /// Default sandbox profile name
    default_sandbox_profile: String,
    /// Whether runtime overrides are allowed
    allow_runtime_overrides: bool,
}

impl ModelCatalog {
    /// Create a new model catalog from SLM configuration
    ///
    /// # Errors
    ///
    /// Returns [`ModelCatalogError::InvalidConfig`] if the configuration contains
    /// invalid model definitions or references non-existent sandbox profiles.
    pub fn new(slm_config: Slm) -> Result<Self, ModelCatalogError> {
        // Validate that the default sandbox profile exists
        if !slm_config
            .sandbox_profiles
            .contains_key(&slm_config.default_sandbox_profile)
        {
            return Err(ModelCatalogError::SandboxProfileNotFound {
                profile: slm_config.default_sandbox_profile,
            });
        }

        // Build model lookup map
        let mut models = HashMap::new();
        for model in slm_config.model_allow_lists.global_models {
            if models.insert(model.id.clone(), model.clone()).is_some() {
                return Err(ModelCatalogError::InvalidConfig {
                    reason: format!("Duplicate model ID: {}", model.id),
                });
            }
        }

        // Validate agent model mappings reference existing models
        for (agent_id, model_ids) in &slm_config.model_allow_lists.agent_model_maps {
            for model_id in model_ids {
                if !models.contains_key(model_id) {
                    return Err(ModelCatalogError::InvalidConfig {
                        reason: format!(
                            "Agent '{}' references non-existent model: {}",
                            agent_id, model_id
                        ),
                    });
                }
            }
        }

        Ok(Self {
            models,
            agent_model_maps: slm_config.model_allow_lists.agent_model_maps,
            sandbox_profiles: slm_config.sandbox_profiles,
            default_sandbox_profile: slm_config.default_sandbox_profile,
            allow_runtime_overrides: slm_config.model_allow_lists.allow_runtime_overrides,
        })
    }

    /// Get a model by its ID
    ///
    /// Returns `None` if the model is not found in the catalog.
    pub fn get_model(&self, model_id: &str) -> Option<&Model> {
        self.models.get(model_id)
    }

    /// List all available models in the catalog
    pub fn list_models(&self) -> Vec<&Model> {
        self.models.values().collect()
    }

    /// Get models allowed for a specific agent
    ///
    /// If the agent has specific model mappings, returns those models.
    /// Otherwise, returns all global models.
    pub fn get_models_for_agent(&self, agent_id: &str) -> Vec<&Model> {
        if let Some(model_ids) = self.agent_model_maps.get(agent_id) {
            model_ids
                .iter()
                .filter_map(|id| self.models.get(id))
                .collect()
        } else {
            // Return all global models if no specific mapping exists
            self.list_models()
        }
    }

    /// Get models with specific capabilities
    pub fn get_models_with_capability(&self, capability: &ModelCapability) -> Vec<&Model> {
        self.models
            .values()
            .filter(|model| model.capabilities.contains(capability))
            .collect()
    }

    /// Get the default sandbox profile
    pub fn get_default_sandbox_profile(&self) -> Option<&SandboxProfile> {
        self.sandbox_profiles.get(&self.default_sandbox_profile)
    }

    /// Get a specific sandbox profile by name
    pub fn get_sandbox_profile(&self, profile_name: &str) -> Option<&SandboxProfile> {
        self.sandbox_profiles.get(profile_name)
    }

    /// List all available sandbox profiles
    pub fn list_sandbox_profiles(&self) -> Vec<(&String, &SandboxProfile)> {
        self.sandbox_profiles.iter().collect()
    }

    /// Check if runtime overrides are allowed
    pub fn allows_runtime_overrides(&self) -> bool {
        self.allow_runtime_overrides
    }

    /// Get resource requirements for a specific model
    pub fn get_model_requirements(
        &self,
        model_id: &str,
    ) -> Option<&crate::config::ModelResourceRequirements> {
        self.get_model(model_id)
            .map(|model| &model.resource_requirements)
    }

    /// Find the best model for given capabilities and resource constraints
    ///
    /// Returns the model that satisfies the required capabilities and has the
    /// lowest resource requirements (memory-first comparison).
    pub fn find_best_model_for_requirements(
        &self,
        required_capabilities: &[ModelCapability],
        max_memory_mb: Option<u64>,
        agent_id: Option<&str>,
    ) -> Option<&Model> {
        let candidate_models = if let Some(agent_id) = agent_id {
            self.get_models_for_agent(agent_id)
        } else {
            self.list_models()
        };

        candidate_models
            .into_iter()
            .filter(|model| {
                // Check if model has all required capabilities
                required_capabilities
                    .iter()
                    .all(|cap| model.capabilities.contains(cap))
            })
            .filter(|model| {
                // Check memory constraints if specified
                if let Some(max_memory) = max_memory_mb {
                    model.resource_requirements.min_memory_mb <= max_memory
                } else {
                    true
                }
            })
            .min_by_key(|model| model.resource_requirements.min_memory_mb)
    }

    /// Validate that a model exists and is accessible for an agent
    pub fn validate_model_access(
        &self,
        model_id: &str,
        agent_id: &str,
    ) -> Result<(), ModelCatalogError> {
        // Check if model exists
        if !self.models.contains_key(model_id) {
            return Err(ModelCatalogError::ModelNotFound {
                id: model_id.to_string(),
            });
        }

        // Check if agent has access to this model
        let agent_models = self.get_models_for_agent(agent_id);
        if !agent_models.iter().any(|model| model.id == model_id) {
            return Err(ModelCatalogError::InvalidConfig {
                reason: format!(
                    "Agent '{}' does not have access to model '{}'",
                    agent_id, model_id
                ),
            });
        }

        Ok(())
    }

    /// Get catalog statistics
    pub fn get_statistics(&self) -> CatalogStatistics {
        let total_models = self.models.len();
        let models_with_gpu = self
            .models
            .values()
            .filter(|model| model.resource_requirements.gpu_requirements.is_some())
            .count();

        let mut capability_counts = HashMap::new();
        for model in self.models.values() {
            for capability in &model.capabilities {
                *capability_counts.entry(capability.clone()).or_insert(0) += 1;
            }
        }

        CatalogStatistics {
            total_models,
            models_with_gpu,
            total_agents_with_mappings: self.agent_model_maps.len(),
            total_sandbox_profiles: self.sandbox_profiles.len(),
            capability_counts,
        }
    }
}

/// Statistics about the model catalog
#[derive(Debug, Clone)]
pub struct CatalogStatistics {
    pub total_models: usize,
    pub models_with_gpu: usize,
    pub total_agents_with_mappings: usize,
    pub total_sandbox_profiles: usize,
    pub capability_counts: HashMap<ModelCapability, usize>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        GpuRequirements, Model, ModelAllowListConfig, ModelCapability, ModelProvider,
        ModelResourceRequirements, SandboxProfile,
    };
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn create_test_model(id: &str, capabilities: Vec<ModelCapability>) -> Model {
        Model {
            id: id.to_string(),
            name: format!("Test Model {}", id),
            provider: ModelProvider::LocalFile {
                file_path: PathBuf::from("/tmp/test.gguf"),
            },
            capabilities,
            resource_requirements: ModelResourceRequirements {
                min_memory_mb: 1024,
                preferred_cpu_cores: 2.0,
                gpu_requirements: None,
            },
        }
    }

    fn create_test_model_with_memory(
        id: &str,
        capabilities: Vec<ModelCapability>,
        memory_mb: u64,
    ) -> Model {
        Model {
            id: id.to_string(),
            name: format!("Test Model {}", id),
            provider: ModelProvider::LocalFile {
                file_path: PathBuf::from("/tmp/test.gguf"),
            },
            capabilities,
            resource_requirements: ModelResourceRequirements {
                min_memory_mb: memory_mb,
                preferred_cpu_cores: 2.0,
                gpu_requirements: None,
            },
        }
    }

    fn create_test_model_with_gpu(
        id: &str,
        capabilities: Vec<ModelCapability>,
        gpu_vram_mb: u64,
    ) -> Model {
        Model {
            id: id.to_string(),
            name: format!("Test Model {}", id),
            provider: ModelProvider::LocalFile {
                file_path: PathBuf::from("/tmp/test.gguf"),
            },
            capabilities,
            resource_requirements: ModelResourceRequirements {
                min_memory_mb: 1024,
                preferred_cpu_cores: 2.0,
                gpu_requirements: Some(GpuRequirements {
                    min_vram_mb: gpu_vram_mb,
                    compute_capability: "7.0".to_string(),
                }),
            },
        }
    }

    fn create_test_slm_config() -> Slm {
        let mut sandbox_profiles = HashMap::new();
        sandbox_profiles.insert("secure".to_string(), SandboxProfile::secure_default());
        sandbox_profiles.insert("standard".to_string(), SandboxProfile::standard_default());

        let models = vec![
            create_test_model("model1", vec![ModelCapability::TextGeneration]),
            create_test_model("model2", vec![ModelCapability::CodeGeneration]),
        ];

        let mut agent_model_maps = HashMap::new();
        agent_model_maps.insert("agent1".to_string(), vec!["model1".to_string()]);

        Slm {
            enabled: true,
            model_allow_lists: ModelAllowListConfig {
                global_models: models,
                agent_model_maps,
                allow_runtime_overrides: false,
            },
            sandbox_profiles,
            default_sandbox_profile: "secure".to_string(),
        }
    }

    fn create_complex_slm_config() -> Slm {
        let mut sandbox_profiles = HashMap::new();
        sandbox_profiles.insert("secure".to_string(), SandboxProfile::secure_default());
        sandbox_profiles.insert("standard".to_string(), SandboxProfile::standard_default());

        let models = vec![
            create_test_model_with_memory(
                "small_model",
                vec![ModelCapability::TextGeneration],
                512,
            ),
            create_test_model_with_memory(
                "medium_model",
                vec![ModelCapability::TextGeneration, ModelCapability::Reasoning],
                1024,
            ),
            create_test_model_with_memory(
                "large_model",
                vec![
                    ModelCapability::TextGeneration,
                    ModelCapability::CodeGeneration,
                ],
                2048,
            ),
            create_test_model_with_gpu(
                "gpu_model",
                vec![ModelCapability::TextGeneration, ModelCapability::Embeddings],
                4096,
            ),
            create_test_model(
                "multi_cap_model",
                vec![
                    ModelCapability::TextGeneration,
                    ModelCapability::CodeGeneration,
                    ModelCapability::Reasoning,
                    ModelCapability::ToolUse,
                ],
            ),
        ];

        let mut agent_model_maps = HashMap::new();
        agent_model_maps.insert(
            "text_agent".to_string(),
            vec!["small_model".to_string(), "medium_model".to_string()],
        );
        agent_model_maps.insert(
            "code_agent".to_string(),
            vec!["large_model".to_string(), "multi_cap_model".to_string()],
        );
        agent_model_maps.insert(
            "restricted_agent".to_string(),
            vec!["small_model".to_string()],
        );

        Slm {
            enabled: true,
            model_allow_lists: ModelAllowListConfig {
                global_models: models,
                agent_model_maps,
                allow_runtime_overrides: true,
            },
            sandbox_profiles,
            default_sandbox_profile: "secure".to_string(),
        }
    }

    #[test]
    fn test_catalog_creation() {
        let config = create_test_slm_config();
        let catalog = ModelCatalog::new(config).unwrap();

        assert_eq!(catalog.list_models().len(), 2);
        assert!(catalog.get_model("model1").is_some());
        assert!(catalog.get_model("model2").is_some());
        assert!(catalog.get_model("nonexistent").is_none());
    }

    #[test]
    fn test_catalog_creation_with_complex_config() {
        let config = create_complex_slm_config();
        let catalog = ModelCatalog::new(config).unwrap();

        assert_eq!(catalog.list_models().len(), 5);
        assert!(catalog.allows_runtime_overrides());
    }

    #[test]
    fn test_agent_model_access() {
        let config = create_test_slm_config();
        let catalog = ModelCatalog::new(config).unwrap();

        let agent1_models = catalog.get_models_for_agent("agent1");
        assert_eq!(agent1_models.len(), 1);
        assert_eq!(agent1_models[0].id, "model1");

        // Agent without specific mapping should get all models
        let agent2_models = catalog.get_models_for_agent("agent2");
        assert_eq!(agent2_models.len(), 2);
    }

    #[test]
    fn test_agent_model_access_complex() {
        let config = create_complex_slm_config();
        let catalog = ModelCatalog::new(config).unwrap();

        // Text agent should get specific models
        let text_agent_models = catalog.get_models_for_agent("text_agent");
        assert_eq!(text_agent_models.len(), 2);
        let model_ids: Vec<&str> = text_agent_models.iter().map(|m| m.id.as_str()).collect();
        assert!(model_ids.contains(&"small_model"));
        assert!(model_ids.contains(&"medium_model"));

        // Code agent should get different models
        let code_agent_models = catalog.get_models_for_agent("code_agent");
        assert_eq!(code_agent_models.len(), 2);
        let code_model_ids: Vec<&str> = code_agent_models.iter().map(|m| m.id.as_str()).collect();
        assert!(code_model_ids.contains(&"large_model"));
        assert!(code_model_ids.contains(&"multi_cap_model"));

        // Restricted agent should only get one model
        let restricted_models = catalog.get_models_for_agent("restricted_agent");
        assert_eq!(restricted_models.len(), 1);
        assert_eq!(restricted_models[0].id, "small_model");

        // Unmapped agent should get all models
        let unmapped_models = catalog.get_models_for_agent("unmapped_agent");
        assert_eq!(unmapped_models.len(), 5);
    }

    #[test]
    fn test_capability_filtering() {
        let config = create_test_slm_config();
        let catalog = ModelCatalog::new(config).unwrap();

        let text_models = catalog.get_models_with_capability(&ModelCapability::TextGeneration);
        assert_eq!(text_models.len(), 1);
        assert_eq!(text_models[0].id, "model1");

        let code_models = catalog.get_models_with_capability(&ModelCapability::CodeGeneration);
        assert_eq!(code_models.len(), 1);
        assert_eq!(code_models[0].id, "model2");

        // Test non-existent capability
        let embedding_models = catalog.get_models_with_capability(&ModelCapability::Embeddings);
        assert_eq!(embedding_models.len(), 0);
    }

    #[test]
    fn test_capability_filtering_complex() {
        let config = create_complex_slm_config();
        let catalog = ModelCatalog::new(config).unwrap();

        // Text generation should be available in multiple models
        let text_models = catalog.get_models_with_capability(&ModelCapability::TextGeneration);
        assert_eq!(text_models.len(), 5); // All models have text generation

        // Code generation should be in fewer models
        let code_models = catalog.get_models_with_capability(&ModelCapability::CodeGeneration);
        assert_eq!(code_models.len(), 2); // large_model and multi_cap_model

        // Reasoning capability
        let reasoning_models = catalog.get_models_with_capability(&ModelCapability::Reasoning);
        assert_eq!(reasoning_models.len(), 2); // medium_model and multi_cap_model

        // Tool use capability
        let tool_models = catalog.get_models_with_capability(&ModelCapability::ToolUse);
        assert_eq!(tool_models.len(), 1); // Only multi_cap_model

        // Embeddings capability
        let embedding_models = catalog.get_models_with_capability(&ModelCapability::Embeddings);
        assert_eq!(embedding_models.len(), 1); // Only gpu_model
    }

    #[test]
    fn test_sandbox_profile_access() {
        let config = create_test_slm_config();
        let catalog = ModelCatalog::new(config).unwrap();

        // Test default sandbox profile
        let default_profile = catalog.get_default_sandbox_profile();
        assert!(default_profile.is_some());

        // Test specific sandbox profile access
        let secure_profile = catalog.get_sandbox_profile("secure");
        assert!(secure_profile.is_some());

        let standard_profile = catalog.get_sandbox_profile("standard");
        assert!(standard_profile.is_some());

        let nonexistent_profile = catalog.get_sandbox_profile("nonexistent");
        assert!(nonexistent_profile.is_none());

        // Test listing all profiles
        let all_profiles = catalog.list_sandbox_profiles();
        assert_eq!(all_profiles.len(), 2);
    }

    #[test]
    fn test_model_requirements_access() {
        let config = create_complex_slm_config();
        let catalog = ModelCatalog::new(config).unwrap();

        // Test getting requirements for existing model
        let small_model_req = catalog.get_model_requirements("small_model");
        assert!(small_model_req.is_some());
        assert_eq!(small_model_req.unwrap().min_memory_mb, 512);

        let gpu_model_req = catalog.get_model_requirements("gpu_model");
        assert!(gpu_model_req.is_some());
        assert!(gpu_model_req.unwrap().gpu_requirements.is_some());

        // Test getting requirements for non-existent model
        let nonexistent_req = catalog.get_model_requirements("nonexistent");
        assert!(nonexistent_req.is_none());
    }

    #[test]
    fn test_find_best_model_for_requirements() {
        let config = create_complex_slm_config();
        let catalog = ModelCatalog::new(config).unwrap();

        // Test finding model with text generation capability
        let text_model = catalog.find_best_model_for_requirements(
            &[ModelCapability::TextGeneration],
            None,
            None,
        );
        assert!(text_model.is_some());
        // Should return the model with lowest memory requirement
        assert_eq!(text_model.unwrap().id, "small_model");

        // Test finding model with code generation capability
        let code_model = catalog.find_best_model_for_requirements(
            &[ModelCapability::CodeGeneration],
            None,
            None,
        );
        assert!(code_model.is_some());
        // Should return multi_cap_model (lower memory: 1024MB vs large_model's 2048MB)
        assert_eq!(code_model.unwrap().id, "multi_cap_model");

        // Test finding model with multiple capabilities
        let multi_cap_model = catalog.find_best_model_for_requirements(
            &[
                ModelCapability::TextGeneration,
                ModelCapability::Reasoning,
                ModelCapability::ToolUse,
            ],
            None,
            None,
        );
        assert!(multi_cap_model.is_some());
        // Only multi_cap_model has all these capabilities
        assert_eq!(multi_cap_model.unwrap().id, "multi_cap_model");

        // Test with memory constraint
        let constrained_model = catalog.find_best_model_for_requirements(
            &[ModelCapability::TextGeneration],
            Some(1000), // Only small_model fits
            None,
        );
        assert!(constrained_model.is_some());
        assert_eq!(constrained_model.unwrap().id, "small_model");

        // Test with very restrictive memory constraint
        let no_model = catalog.find_best_model_for_requirements(
            &[ModelCapability::TextGeneration],
            Some(100), // No model fits
            None,
        );
        assert!(no_model.is_none());
    }

    #[test]
    fn test_find_best_model_for_agent() {
        let config = create_complex_slm_config();
        let catalog = ModelCatalog::new(config).unwrap();

        // Test finding model for text agent
        let text_agent_model = catalog.find_best_model_for_requirements(
            &[ModelCapability::TextGeneration],
            None,
            Some("text_agent"),
        );
        assert!(text_agent_model.is_some());
        // Should get small_model as it has lower memory requirement
        assert_eq!(text_agent_model.unwrap().id, "small_model");

        // Test finding model for code agent with specific capabilities
        let code_agent_model = catalog.find_best_model_for_requirements(
            &[ModelCapability::CodeGeneration],
            None,
            Some("code_agent"),
        );
        assert!(code_agent_model.is_some());
        // Both large_model and multi_cap_model have code generation; multi_cap_model has lower memory
        assert_eq!(code_agent_model.unwrap().id, "multi_cap_model");

        // Test finding model for restricted agent
        let restricted_model = catalog.find_best_model_for_requirements(
            &[ModelCapability::TextGeneration],
            None,
            Some("restricted_agent"),
        );
        assert!(restricted_model.is_some());
        assert_eq!(restricted_model.unwrap().id, "small_model");

        // Test finding model with capability not available to agent
        let impossible_model = catalog.find_best_model_for_requirements(
            &[ModelCapability::CodeGeneration],
            None,
            Some("restricted_agent"), // Only has access to small_model which doesn't have code generation
        );
        assert!(impossible_model.is_none());
    }

    #[test]
    fn test_validate_model_access() {
        let config = create_complex_slm_config();
        let catalog = ModelCatalog::new(config).unwrap();

        // Test valid model access
        let valid_access = catalog.validate_model_access("small_model", "text_agent");
        assert!(valid_access.is_ok());

        // Test access to model not in agent's list
        let invalid_access = catalog.validate_model_access("large_model", "text_agent");
        assert!(invalid_access.is_err());
        if let Err(ModelCatalogError::InvalidConfig { reason }) = invalid_access {
            assert!(reason.contains("does not have access"));
        }

        // Test access to non-existent model
        let nonexistent_access = catalog.validate_model_access("nonexistent_model", "text_agent");
        assert!(nonexistent_access.is_err());
        if let Err(ModelCatalogError::ModelNotFound { id }) = nonexistent_access {
            assert_eq!(id, "nonexistent_model");
        }

        // Test unmapped agent should have access to all models
        let unmapped_access = catalog.validate_model_access("large_model", "unmapped_agent");
        assert!(unmapped_access.is_ok());
    }

    #[test]
    fn test_catalog_statistics() {
        let config = create_complex_slm_config();
        let catalog = ModelCatalog::new(config).unwrap();

        let stats = catalog.get_statistics();

        assert_eq!(stats.total_models, 5);
        assert_eq!(stats.models_with_gpu, 1); // Only gpu_model has GPU requirements
        assert_eq!(stats.total_agents_with_mappings, 3); // text_agent, code_agent, restricted_agent
        assert_eq!(stats.total_sandbox_profiles, 2);

        // Check capability counts
        assert_eq!(stats.capability_counts[&ModelCapability::TextGeneration], 5); // All models
        assert_eq!(stats.capability_counts[&ModelCapability::CodeGeneration], 2); // large_model, multi_cap_model
        assert_eq!(stats.capability_counts[&ModelCapability::Reasoning], 2); // medium_model, multi_cap_model
        assert_eq!(stats.capability_counts[&ModelCapability::ToolUse], 1); // multi_cap_model
        assert_eq!(stats.capability_counts[&ModelCapability::Embeddings], 1); // gpu_model
    }

    #[test]
    fn test_validation_errors() {
        let mut config = create_test_slm_config();

        // Test invalid default sandbox profile
        config.default_sandbox_profile = "nonexistent".to_string();
        let result = ModelCatalog::new(config);
        assert!(matches!(
            result,
            Err(ModelCatalogError::SandboxProfileNotFound { .. })
        ));
    }

    #[test]
    fn test_validation_duplicate_model_ids() {
        let mut config = create_test_slm_config();

        // Add duplicate model ID
        config
            .model_allow_lists
            .global_models
            .push(create_test_model(
                "model1",
                vec![ModelCapability::Reasoning],
            ));

        let result = ModelCatalog::new(config);
        assert!(matches!(
            result,
            Err(ModelCatalogError::InvalidConfig { .. })
        ));
    }

    #[test]
    fn test_validation_invalid_agent_model_mapping() {
        let mut config = create_test_slm_config();

        // Add agent mapping to non-existent model
        config.model_allow_lists.agent_model_maps.insert(
            "invalid_agent".to_string(),
            vec!["nonexistent_model".to_string()],
        );

        let result = ModelCatalog::new(config);
        assert!(matches!(
            result,
            Err(ModelCatalogError::InvalidConfig { .. })
        ));
    }

    #[test]
    fn test_empty_catalog() {
        let mut config = create_test_slm_config();
        config.model_allow_lists.global_models.clear();
        config.model_allow_lists.agent_model_maps.clear();

        let catalog = ModelCatalog::new(config).unwrap();

        assert_eq!(catalog.list_models().len(), 0);
        assert_eq!(catalog.get_models_for_agent("any_agent").len(), 0);
        assert_eq!(
            catalog
                .get_models_with_capability(&ModelCapability::TextGeneration)
                .len(),
            0
        );

        let stats = catalog.get_statistics();
        assert_eq!(stats.total_models, 0);
        assert_eq!(stats.models_with_gpu, 0);
        assert_eq!(stats.total_agents_with_mappings, 0);
    }

    #[test]
    fn test_model_provider_types() {
        let local_model = Model {
            id: "local".to_string(),
            name: "Local Model".to_string(),
            provider: ModelProvider::LocalFile {
                file_path: PathBuf::from("/models/local.gguf"),
            },
            capabilities: vec![ModelCapability::TextGeneration],
            resource_requirements: ModelResourceRequirements {
                min_memory_mb: 1024,
                preferred_cpu_cores: 2.0,
                gpu_requirements: None,
            },
        };

        let hf_model = Model {
            id: "huggingface".to_string(),
            name: "HuggingFace Model".to_string(),
            provider: ModelProvider::HuggingFace {
                model_path: "microsoft/DialoGPT-medium".to_string(),
            },
            capabilities: vec![ModelCapability::TextGeneration],
            resource_requirements: ModelResourceRequirements {
                min_memory_mb: 2048,
                preferred_cpu_cores: 4.0,
                gpu_requirements: Some(GpuRequirements {
                    min_vram_mb: 4096,
                    compute_capability: "7.0".to_string(),
                }),
            },
        };

        let openai_model = Model {
            id: "openai".to_string(),
            name: "OpenAI Model".to_string(),
            provider: ModelProvider::OpenAI {
                model_name: "gpt-3.5-turbo".to_string(),
            },
            capabilities: vec![ModelCapability::TextGeneration, ModelCapability::Reasoning],
            resource_requirements: ModelResourceRequirements {
                min_memory_mb: 0, // Cloud model
                preferred_cpu_cores: 0.0,
                gpu_requirements: None,
            },
        };

        let mut config = create_test_slm_config();
        config.model_allow_lists.global_models = vec![local_model, hf_model, openai_model];
        config.model_allow_lists.agent_model_maps.clear();

        let catalog = ModelCatalog::new(config).unwrap();
        assert_eq!(catalog.list_models().len(), 3);

        // Test that all provider types are accessible
        assert!(catalog.get_model("local").is_some());
        assert!(catalog.get_model("huggingface").is_some());
        assert!(catalog.get_model("openai").is_some());
    }

    #[test]
    fn test_runtime_overrides_setting() {
        let mut config = create_test_slm_config();
        config.model_allow_lists.allow_runtime_overrides = true;

        let catalog = ModelCatalog::new(config).unwrap();
        assert!(catalog.allows_runtime_overrides());

        let mut config_no_overrides = create_test_slm_config();
        config_no_overrides
            .model_allow_lists
            .allow_runtime_overrides = false;

        let catalog_no_overrides = ModelCatalog::new(config_no_overrides).unwrap();
        assert!(!catalog_no_overrides.allows_runtime_overrides());
    }
}
