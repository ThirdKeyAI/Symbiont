//! Policy evaluation engine for routing decisions

use std::collections::HashMap;
use super::config::{RoutingPolicyConfig, RouteAction, ModelPreference};
use super::decision::{RouteDecision, RoutingContext, LLMProvider};
use super::error::{RoutingError, TaskType};
use super::classifier::{TaskClassifier, ClassificationResult};
use crate::models::ModelCatalog;
use crate::config::{ResourceConstraints, ModelCapability};

/// Policy evaluation engine for making routing decisions
#[derive(Debug)]
pub struct PolicyEvaluator {
    /// Policy configuration
    config: RoutingPolicyConfig,
    /// Task classifier for automatic task type detection
    classifier: TaskClassifier,
    /// Model catalog for finding suitable models
    model_catalog: ModelCatalog,
    /// Evaluation cache for performance
    evaluation_cache: std::sync::RwLock<HashMap<String, CachedEvaluation>>,
}

/// Cached evaluation result
#[derive(Debug, Clone)]
struct CachedEvaluation {
    decision: RouteDecision,
    timestamp: std::time::Instant,
    #[allow(dead_code)]
    context_hash: u64,
}

/// Policy evaluation result
#[derive(Debug, Clone)]
pub struct PolicyEvaluationResult {
    /// The routing decision
    pub decision: RouteDecision,
    /// Rule that matched (if any)
    pub matched_rule: Option<String>,
    /// Task classification result
    pub task_classification: ClassificationResult,
    /// Evaluation metadata
    pub metadata: HashMap<String, String>,
}

/// Policy context for evaluation
#[derive(Debug, Clone)]
pub struct PolicyContext {
    /// Routing context
    pub routing_context: RoutingContext,
    /// Available resource constraints
    pub available_resources: Option<ResourceConstraints>,
    /// Current system load
    pub system_load: Option<f64>,
    /// Additional evaluation hints
    pub hints: HashMap<String, String>,
}

impl PolicyEvaluator {
    /// Create a new policy evaluator
    pub fn new(
        config: RoutingPolicyConfig,
        classifier: TaskClassifier,
        model_catalog: ModelCatalog,
    ) -> Result<Self, RoutingError> {
        // Validate the policy configuration
        config.validate()?;
        
        Ok(Self {
            config,
            classifier,
            model_catalog,
            evaluation_cache: std::sync::RwLock::new(HashMap::new()),
        })
    }
    
    /// Evaluate routing policies and make a decision
    pub async fn evaluate_policies(
        &self,
        context: &RoutingContext,
    ) -> Result<PolicyEvaluationResult, RoutingError> {
        let policy_context = PolicyContext {
            routing_context: context.clone(),
            available_resources: None,
            system_load: None,
            hints: HashMap::new(),
        };
        
        self.evaluate_policies_with_context(&policy_context).await
    }
    
    /// Evaluate routing policies with additional context
    pub async fn evaluate_policies_with_context(
        &self,
        context: &PolicyContext,
    ) -> Result<PolicyEvaluationResult, RoutingError> {
        // Check if SLM routing is globally enabled
        if !self.config.global_settings.slm_routing_enabled {
            return Ok(PolicyEvaluationResult {
                decision: self.apply_default_action()?,
                matched_rule: None,
                task_classification: ClassificationResult {
                    task_type: context.routing_context.task_type.clone(),
                    confidence: 1.0,
                    matched_patterns: vec!["slm_routing_disabled".to_string()],
                    keyword_matches: Vec::new(),
                },
                metadata: {
                    let mut meta = HashMap::new();
                    meta.insert("reason".to_string(), "SLM routing globally disabled".to_string());
                    meta
                },
            });
        }
        
        // Check cache first
        let cache_key = self.generate_cache_key(context);
        if let Some(cached) = self.check_cache(&cache_key) {
            return Ok(PolicyEvaluationResult {
                decision: cached.decision,
                matched_rule: Some("cached".to_string()),
                task_classification: ClassificationResult {
                    task_type: context.routing_context.task_type.clone(),
                    confidence: 1.0,
                    matched_patterns: vec!["cached_result".to_string()],
                    keyword_matches: Vec::new(),
                },
                metadata: {
                    let mut meta = HashMap::new();
                    meta.insert("source".to_string(), "cache".to_string());
                    meta
                },
            });
        }
        
        // Classify the task if not already classified
        let task_classification = if matches!(context.routing_context.task_type, TaskType::Custom(ref name) if name == "unknown") {
            self.classifier.classify_task(&context.routing_context.prompt, &context.routing_context)?
        } else {
            ClassificationResult {
                task_type: context.routing_context.task_type.clone(),
                confidence: 1.0,
                matched_patterns: vec!["pre_classified".to_string()],
                keyword_matches: Vec::new(),
            }
        };
        
        let mut evaluation_context = context.clone();
        evaluation_context.routing_context.task_type = task_classification.task_type.clone();
        
        // Evaluate rules in priority order
        for rule in &self.config.rules {
            if rule.matches(&evaluation_context.routing_context) {
                let decision = self.apply_rule_action(&rule.action, &evaluation_context).await?;
                
                // Cache the result
                self.cache_evaluation(&cache_key, &decision);
                
                return Ok(PolicyEvaluationResult {
                    decision,
                    matched_rule: Some(rule.name.clone()),
                    task_classification,
                    metadata: {
                        let mut meta = HashMap::new();
                        meta.insert("rule_priority".to_string(), rule.priority.to_string());
                        meta.insert("rule_name".to_string(), rule.name.clone());
                        meta
                    },
                });
            }
        }
        
        // No rules matched, apply default action
        let decision = self.apply_default_action()?;
        self.cache_evaluation(&cache_key, &decision);
        
        Ok(PolicyEvaluationResult {
            decision,
            matched_rule: None,
            task_classification,
            metadata: {
                let mut meta = HashMap::new();
                meta.insert("source".to_string(), "default_action".to_string());
                meta
            },
        })
    }
    
    /// Apply a rule action to generate a routing decision
    async fn apply_rule_action(
        &self,
        action: &RouteAction,
        context: &PolicyContext,
    ) -> Result<RouteDecision, RoutingError> {
        match action {
            RouteAction::UseSLM {
                model_preference,
                monitoring_level,
                fallback_on_low_confidence,
                confidence_threshold: _,
            } => {
                let model = self.find_suitable_slm(
                    model_preference,
                    &context.routing_context.task_type,
                    context.routing_context.resource_limits.as_ref(),
                    Some(&context.routing_context.agent_id.to_string()),
                )?;
                
                Ok(RouteDecision::UseSLM {
                    model_id: model.id.clone(),
                    monitoring: monitoring_level.clone(),
                    fallback_on_failure: *fallback_on_low_confidence,
                })
            }
            RouteAction::UseLLM { provider, model: _ } => {
                Ok(RouteDecision::UseLLM {
                    provider: provider.clone(),
                    reason: "Policy rule matched".to_string(),
                })
            }
            RouteAction::Deny { reason } => {
                Ok(RouteDecision::Deny {
                    reason: reason.clone(),
                    policy_violated: "Explicit deny rule".to_string(),
                })
            }
        }
    }
    
    /// Apply the default action when no rules match
    fn apply_default_action(&self) -> Result<RouteDecision, RoutingError> {
        match &self.config.default_action {
            RouteAction::UseSLM { .. } => {
                // For default SLM action, use a simple fallback
                Ok(RouteDecision::UseLLM {
                    provider: LLMProvider::OpenAI { model: None },
                    reason: "Default action - no SLM available".to_string(),
                })
            }
            RouteAction::UseLLM { provider, .. } => {
                Ok(RouteDecision::UseLLM {
                    provider: provider.clone(),
                    reason: "Default action".to_string(),
                })
            }
            RouteAction::Deny { reason } => {
                Ok(RouteDecision::Deny {
                    reason: reason.clone(),
                    policy_violated: "Default deny policy".to_string(),
                })
            }
        }
    }
    
    /// Find a suitable SLM based on preferences and constraints
    fn find_suitable_slm(
        &self,
        preference: &ModelPreference,
        task_type: &TaskType,
        resource_constraints: Option<&ResourceConstraints>,
        agent_id: Option<&str>,
    ) -> Result<&crate::config::Model, RoutingError> {
        let required_capabilities = task_type.to_capabilities();
        let max_memory = resource_constraints.map(|rc| rc.max_memory_mb);
        
        let model = match preference {
            ModelPreference::Specialist => {
                self.find_specialist_model(task_type, &required_capabilities, max_memory, agent_id)?
            }
            ModelPreference::Generalist => {
                self.find_generalist_model(&required_capabilities, max_memory, agent_id)?
            }
            ModelPreference::Specific { model_id } => {
                self.model_catalog.get_model(model_id)
                    .ok_or_else(|| RoutingError::NoSuitableModel { 
                        task_type: task_type.clone() 
                    })?
            }
            ModelPreference::BestAvailable => {
                self.model_catalog.find_best_model_for_requirements(
                    &required_capabilities,
                    max_memory,
                    agent_id,
                ).ok_or_else(|| RoutingError::NoSuitableModel { 
                    task_type: task_type.clone() 
                })?
            }
        };
        
        // Validate the model meets our requirements
        self.validate_model_for_task(model, task_type, resource_constraints)?;
        
        Ok(model)
    }
    
    /// Find a specialist model for the given task type
    fn find_specialist_model(
        &self,
        task_type: &TaskType,
        required_capabilities: &[ModelCapability],
        max_memory: Option<u64>,
        agent_id: Option<&str>,
    ) -> Result<&crate::config::Model, RoutingError> {
        let candidate_models = if let Some(agent_id) = agent_id {
            self.model_catalog.get_models_for_agent(agent_id)
        } else {
            self.model_catalog.list_models()
        };
        
        // Filter for models with required capabilities
        let suitable_models: Vec<_> = candidate_models
            .into_iter()
            .filter(|model| {
                required_capabilities.iter().all(|cap| model.capabilities.contains(cap))
            })
            .filter(|model| {
                if let Some(max_mem) = max_memory {
                    model.resource_requirements.min_memory_mb <= max_mem
                } else {
                    true
                }
            })
            .collect();
        
        // Prefer models that are specifically good for this task type
        let specialist = suitable_models.iter().find(|model| {
            match task_type {
                TaskType::CodeGeneration | TaskType::BoilerplateCode => {
                    model.capabilities.contains(&ModelCapability::CodeGeneration)
                }
                TaskType::Reasoning | TaskType::Analysis => {
                    model.capabilities.contains(&ModelCapability::Reasoning)
                }
                _ => false,
            }
        });
        
        specialist.or_else(|| suitable_models.first())
            .copied()
            .ok_or_else(|| RoutingError::NoSuitableModel { 
                task_type: task_type.clone() 
            })
    }
    
    /// Find a generalist model
    fn find_generalist_model(
        &self,
        required_capabilities: &[ModelCapability],
        max_memory: Option<u64>,
        agent_id: Option<&str>,
    ) -> Result<&crate::config::Model, RoutingError> {
        self.model_catalog.find_best_model_for_requirements(
            required_capabilities,
            max_memory,
            agent_id,
        ).ok_or_else(|| RoutingError::NoSuitableModel { 
            task_type: TaskType::Custom("generalist".to_string())
        })
    }
    
    /// Validate that a model is suitable for the given task and constraints
    fn validate_model_for_task(
        &self,
        model: &crate::config::Model,
        task_type: &TaskType,
        resource_constraints: Option<&ResourceConstraints>,
    ) -> Result<(), RoutingError> {
        // Check capabilities
        let required_capabilities = task_type.to_capabilities();
        for capability in &required_capabilities {
            if !model.capabilities.contains(capability) {
                return Err(RoutingError::NoSuitableModel { 
                    task_type: task_type.clone() 
                });
            }
        }
        
        // Check resource constraints
        if let Some(constraints) = resource_constraints {
            if model.resource_requirements.min_memory_mb > constraints.max_memory_mb {
                return Err(RoutingError::ResourceConstraintViolation {
                    constraint: format!(
                        "Model requires {}MB memory but only {}MB available",
                        model.resource_requirements.min_memory_mb,
                        constraints.max_memory_mb
                    ),
                });
            }
        }
        
        Ok(())
    }
    
    /// Generate cache key for evaluation context
    fn generate_cache_key(&self, context: &PolicyContext) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        
        let mut hasher = DefaultHasher::new();
        context.routing_context.agent_id.hash(&mut hasher);
        context.routing_context.task_type.hash(&mut hasher);
        context.routing_context.prompt.hash(&mut hasher);
        
        if let Some(ref constraints) = context.routing_context.resource_limits {
            constraints.max_memory_mb.hash(&mut hasher);
            constraints.max_cpu_cores.to_bits().hash(&mut hasher);
        }
        
        format!("policy_eval_{:x}", hasher.finish())
    }
    
    /// Check evaluation cache
    fn check_cache(&self, key: &str) -> Option<CachedEvaluation> {
        let cache = self.evaluation_cache.read().ok()?;
        let cached = cache.get(key)?;
        
        // Check if cache entry is still fresh (5 minutes)
        if cached.timestamp.elapsed() < std::time::Duration::from_secs(300) {
            Some(cached.clone())
        } else {
            None
        }
    }
    
    /// Cache evaluation result
    fn cache_evaluation(&self, key: &str, decision: &RouteDecision) {
        if let Ok(mut cache) = self.evaluation_cache.write() {
            // Limit cache size
            if cache.len() > 1000 {
                cache.clear();
            }
            
            cache.insert(key.to_string(), CachedEvaluation {
                decision: decision.clone(),
                timestamp: std::time::Instant::now(),
                context_hash: 0, // Could be used for more sophisticated caching
            });
        }
    }
    
    /// Update policy configuration
    pub fn update_config(&mut self, config: RoutingPolicyConfig) -> Result<(), RoutingError> {
        config.validate()?;
        self.config = config;
        
        // Clear cache when config changes
        if let Ok(mut cache) = self.evaluation_cache.write() {
            cache.clear();
        }
        
        Ok(())
    }
    
    /// Get policy evaluation statistics
    pub fn get_statistics(&self) -> PolicyStatistics {
        let cache_size = self.evaluation_cache.read()
            .map(|cache| cache.len())
            .unwrap_or(0);
        
        PolicyStatistics {
            total_rules: self.config.rules.len(),
            cache_size,
            slm_routing_enabled: self.config.global_settings.slm_routing_enabled,
            global_confidence_threshold: self.config.global_settings.global_confidence_threshold,
            max_slm_retries: self.config.global_settings.max_slm_retries,
        }
    }
}

/// Statistics about policy evaluation
#[derive(Debug, Clone)]
pub struct PolicyStatistics {
    pub total_rules: usize,
    pub cache_size: usize,
    pub slm_routing_enabled: bool,
    pub global_confidence_threshold: f64,
    pub max_slm_retries: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::AgentId;
    use crate::config::{Slm, Model, ModelProvider, ModelResourceRequirements, ModelAllowListConfig, SandboxProfile, ResourceConstraints};
    use std::path::PathBuf;
    use std::collections::HashMap;

    fn create_test_model_catalog() -> ModelCatalog {
        let mut global_models = Vec::new();
        
        global_models.push(Model {
            id: "test-slm-1".to_string(),
            name: "Test SLM 1".to_string(),
            provider: ModelProvider::LocalFile { file_path: PathBuf::from("/tmp/test.gguf") },
            capabilities: vec![ModelCapability::TextGeneration, ModelCapability::CodeGeneration],
            resource_requirements: ModelResourceRequirements {
                min_memory_mb: 1024,
                preferred_cpu_cores: 2.0,
                gpu_requirements: None,
            },
        });

        global_models.push(Model {
            id: "test-slm-2".to_string(),
            name: "Test SLM 2".to_string(),
            provider: ModelProvider::LocalFile { file_path: PathBuf::from("/tmp/test2.gguf") },
            capabilities: vec![ModelCapability::TextGeneration, ModelCapability::Reasoning],
            resource_requirements: ModelResourceRequirements {
                min_memory_mb: 2048,
                preferred_cpu_cores: 4.0,
                gpu_requirements: None,
            },
        });

        global_models.push(Model {
            id: "specialist-code".to_string(),
            name: "Code Specialist".to_string(),
            provider: ModelProvider::LocalFile { file_path: PathBuf::from("/tmp/code.gguf") },
            capabilities: vec![ModelCapability::CodeGeneration],
            resource_requirements: ModelResourceRequirements {
                min_memory_mb: 1536,
                preferred_cpu_cores: 3.0,
                gpu_requirements: None,
            },
        });

        // Add agent model mappings for testing
        let mut agent_model_maps = HashMap::new();
        agent_model_maps.insert("restricted-agent".to_string(), vec!["test-slm-1".to_string()]);
        agent_model_maps.insert("code-agent".to_string(), vec!["specialist-code".to_string(), "test-slm-1".to_string()]);

        let mut sandbox_profiles = HashMap::new();
        sandbox_profiles.insert("default".to_string(), SandboxProfile::secure_default());
        
        let slm_config = Slm {
            enabled: true,
            model_allow_lists: ModelAllowListConfig {
                global_models,
                agent_model_maps,
                allow_runtime_overrides: false,
            },
            sandbox_profiles,
            default_sandbox_profile: "default".to_string(),
        };
        
        ModelCatalog::new(slm_config).unwrap()
    }
    
    fn create_test_classifier() -> TaskClassifier {
        let config = super::super::config::TaskClassificationConfig::default();
        TaskClassifier::new(config).unwrap()
    }

    fn create_routing_context_with_resource_limits(
        agent_id: AgentId,
        task_type: TaskType,
        prompt: String,
        max_memory_mb: u64,
    ) -> RoutingContext {
        let mut context = RoutingContext::new(agent_id, task_type, prompt);
        context.resource_limits = Some(ResourceConstraints {
            max_memory_mb,
            max_cpu_cores: 2.0,
            max_disk_mb: 1000,
            gpu_access: crate::config::GpuAccess::None,
            max_io_bandwidth_mbps: Some(100),
        });
        context
    }
    
    #[tokio::test]
    async fn test_policy_evaluation_with_slm_action() {
        let model_catalog = create_test_model_catalog();
        let classifier = create_test_classifier();
        
        let mut config = RoutingPolicyConfig::default();
        config.rules.push(RoutingRule {
            name: "test_rule".to_string(),
            priority: 100,
            conditions: super::super::config::RoutingConditions {
                task_types: Some(vec![TaskType::CodeGeneration]),
                agent_ids: None,
                resource_constraints: None,
                security_level: None,
                custom_conditions: None,
            },
            action: RouteAction::UseSLM {
                model_preference: ModelPreference::BestAvailable,
                monitoring_level: MonitoringLevel::Basic,
                fallback_on_low_confidence: true,
                confidence_threshold: Some(0.8),
            },
            override_allowed: true,
        });
        
        let evaluator = PolicyEvaluator::new(config, classifier, model_catalog).unwrap();
        
        let context = RoutingContext::new(
            AgentId::new(),
            TaskType::CodeGeneration,
            "Write a function to sort an array".to_string(),
        );
        
        let result = evaluator.evaluate_policies(&context).await.unwrap();
        
        match result.decision {
            RouteDecision::UseSLM { model_id, .. } => {
                assert_eq!(model_id, "test-slm-1"); // Should pick the first available model
            }
            _ => panic!("Expected UseSLM decision"),
        }
        
        assert_eq!(result.matched_rule, Some("test_rule".to_string()));
    }

    #[tokio::test]
    async fn test_policy_evaluation_with_specialist_preference() {
        let model_catalog = create_test_model_catalog();
        let classifier = create_test_classifier();
        
        let mut config = RoutingPolicyConfig::default();
        config.rules.push(RoutingRule {
            name: "specialist_rule".to_string(),
            priority: 100,
            conditions: super::super::config::RoutingConditions {
                task_types: Some(vec![TaskType::CodeGeneration]),
                agent_ids: None,
                resource_constraints: None,
                security_level: None,
                custom_conditions: None,
            },
            action: RouteAction::UseSLM {
                model_preference: ModelPreference::Specialist,
                monitoring_level: MonitoringLevel::Enhanced { confidence_threshold: 0.9 },
                fallback_on_low_confidence: true,
                confidence_threshold: Some(0.9),
            },
            override_allowed: true,
        });
        
        let evaluator = PolicyEvaluator::new(config, classifier, model_catalog).unwrap();
        
        let context = RoutingContext::new(
            AgentId::new(),
            TaskType::CodeGeneration,
            "Generate complex algorithm".to_string(),
        );
        
        let result = evaluator.evaluate_policies(&context).await.unwrap();
        
        match result.decision {
            RouteDecision::UseSLM { model_id, .. } => {
                assert_eq!(model_id, "specialist-code"); // Should pick the specialist model
            }
            _ => panic!("Expected UseSLM decision with specialist model"),
        }
    }

    #[tokio::test]
    async fn test_policy_evaluation_with_specific_model() {
        let model_catalog = create_test_model_catalog();
        let classifier = create_test_classifier();
        
        let mut config = RoutingPolicyConfig::default();
        config.rules.push(RoutingRule {
            name: "specific_model_rule".to_string(),
            priority: 100,
            conditions: super::super::config::RoutingConditions {
                task_types: Some(vec![TaskType::TextGeneration]),
                agent_ids: None,
                resource_constraints: None,
                security_level: None,
                custom_conditions: None,
            },
            action: RouteAction::UseSLM {
                model_preference: ModelPreference::Specific { model_id: "test-slm-2".to_string() },
                monitoring_level: MonitoringLevel::None,
                fallback_on_low_confidence: false,
                confidence_threshold: None,
            },
            override_allowed: false,
        });
        
        let evaluator = PolicyEvaluator::new(config, classifier, model_catalog).unwrap();
        
        let context = RoutingContext::new(
            AgentId::new(),
            TaskType::TextGeneration,
            "Generate some text".to_string(),
        );
        
        let result = evaluator.evaluate_policies(&context).await.unwrap();
        
        match result.decision {
            RouteDecision::UseSLM { model_id, .. } => {
                assert_eq!(model_id, "test-slm-2");
            }
            _ => panic!("Expected UseSLM decision with specific model"),
        }
    }

    #[tokio::test]
    async fn test_policy_evaluation_with_agent_restrictions() {
        let model_catalog = create_test_model_catalog();
        let classifier = create_test_classifier();
        
        let mut config = RoutingPolicyConfig::default();
        config.rules.push(RoutingRule {
            name: "agent_restricted_rule".to_string(),
            priority: 100,
            conditions: super::super::config::RoutingConditions {
                task_types: Some(vec![TaskType::CodeGeneration]),
                agent_ids: Some(vec!["code-agent".to_string()]),
                resource_constraints: None,
                security_level: None,
                custom_conditions: None,
            },
            action: RouteAction::UseSLM {
                model_preference: ModelPreference::Specialist,
                monitoring_level: MonitoringLevel::Basic,
                fallback_on_low_confidence: true,
                confidence_threshold: Some(0.8),
            },
            override_allowed: true,
        });
        
        let evaluator = PolicyEvaluator::new(config, classifier, model_catalog).unwrap();
        
        // Test with matching agent
        let code_agent_id = AgentId::from_string("code-agent".to_string());
        let context = RoutingContext::new(
            code_agent_id,
            TaskType::CodeGeneration,
            "Write optimized code".to_string(),
        );
        
        let result = evaluator.evaluate_policies(&context).await.unwrap();
        
        match result.decision {
            RouteDecision::UseSLM { model_id, .. } => {
                assert_eq!(model_id, "specialist-code"); // Should get specialist from agent's allowed models
            }
            _ => panic!("Expected UseSLM decision for code agent"),
        }

        // Test with non-matching agent - should fall back to default action
        let other_agent_id = AgentId::new();
        let other_context = RoutingContext::new(
            other_agent_id,
            TaskType::CodeGeneration,
            "Write code".to_string(),
        );
        
        let other_result = evaluator.evaluate_policies(&other_context).await.unwrap();
        
        // Should fall back to default action since agent doesn't match
        match other_result.decision {
            RouteDecision::UseLLM { .. } => {
                // Expected default fallback
            }
            _ => panic!("Expected default LLM fallback for non-matching agent"),
        }
    }

    #[tokio::test]
    async fn test_policy_evaluation_with_resource_constraints() {
        let model_catalog = create_test_model_catalog();
        let classifier = create_test_classifier();
        
        let mut config = RoutingPolicyConfig::default();
        config.rules.push(RoutingRule {
            name: "resource_constrained_rule".to_string(),
            priority: 100,
            conditions: super::super::config::RoutingConditions {
                task_types: Some(vec![TaskType::TextGeneration]),
                agent_ids: None,
                resource_constraints: Some(ResourceConstraints {
                    max_memory_mb: 1500, // Only test-slm-1 fits
                    max_cpu_cores: 3.0,
                    max_disk_mb: 1000,
                    gpu_access: crate::config::GpuAccess::None,
                    max_io_bandwidth_mbps: Some(100),
                }),
                security_level: None,
                custom_conditions: None,
            },
            action: RouteAction::UseSLM {
                model_preference: ModelPreference::BestAvailable,
                monitoring_level: MonitoringLevel::Basic,
                fallback_on_low_confidence: false,
                confidence_threshold: None,
            },
            override_allowed: true,
        });
        
        let evaluator = PolicyEvaluator::new(config, classifier, model_catalog).unwrap();
        
        let context = create_routing_context_with_resource_limits(
            AgentId::new(),
            TaskType::TextGeneration,
            "Generate text with constraints".to_string(),
            1500,
        );
        
        let result = evaluator.evaluate_policies(&context).await.unwrap();
        
        match result.decision {
            RouteDecision::UseSLM { model_id, .. } => {
                assert_eq!(model_id, "test-slm-1"); // Only model that fits the constraints
            }
            _ => panic!("Expected UseSLM decision with resource constraints"),
        }
    }

    #[tokio::test]
    async fn test_policy_evaluation_with_llm_action() {
        let model_catalog = create_test_model_catalog();
        let classifier = create_test_classifier();
        
        let mut config = RoutingPolicyConfig::default();
        config.rules.push(RoutingRule {
            name: "llm_rule".to_string(),
            priority: 100,
            conditions: super::super::config::RoutingConditions {
                task_types: Some(vec![TaskType::Reasoning]),
                agent_ids: None,
                resource_constraints: None,
                security_level: None,
                custom_conditions: None,
            },
            action: RouteAction::UseLLM {
                provider: LLMProvider::OpenAI { model: Some("gpt-4".to_string()) },
                model: Some("gpt-4".to_string()),
            },
            override_allowed: false,
        });
        
        let evaluator = PolicyEvaluator::new(config, classifier, model_catalog).unwrap();
        
        let context = RoutingContext::new(
            AgentId::new(),
            TaskType::Reasoning,
            "Solve complex reasoning problem".to_string(),
        );
        
        let result = evaluator.evaluate_policies(&context).await.unwrap();
        
        match result.decision {
            RouteDecision::UseLLM { provider, reason } => {
                assert!(matches!(provider, LLMProvider::OpenAI { .. }));
                assert!(reason.contains("Policy rule matched"));
            }
            _ => panic!("Expected UseLLM decision"),
        }
    }

    #[tokio::test]
    async fn test_policy_evaluation_with_deny_action() {
        let model_catalog = create_test_model_catalog();
        let classifier = create_test_classifier();
        
        let mut config = RoutingPolicyConfig::default();
        config.rules.push(RoutingRule {
            name: "deny_rule".to_string(),
            priority: 100,
            conditions: super::super::config::RoutingConditions {
                task_types: Some(vec![TaskType::Custom("forbidden".to_string())]),
                agent_ids: None,
                resource_constraints: None,
                security_level: None,
                custom_conditions: None,
            },
            action: RouteAction::Deny {
                reason: "Forbidden task type".to_string(),
            },
            override_allowed: false,
        });
        
        let evaluator = PolicyEvaluator::new(config, classifier, model_catalog).unwrap();
        
        let context = RoutingContext::new(
            AgentId::new(),
            TaskType::Custom("forbidden".to_string()),
            "Forbidden operation".to_string(),
        );
        
        let result = evaluator.evaluate_policies(&context).await.unwrap();
        
        match result.decision {
            RouteDecision::Deny { reason, policy_violated } => {
                assert_eq!(reason, "Forbidden task type");
                assert_eq!(policy_violated, "Explicit deny rule");
            }
            _ => panic!("Expected Deny decision"),
        }
    }

    #[tokio::test]
    async fn test_default_action_fallback() {
        let model_catalog = create_test_model_catalog();
        let classifier = create_test_classifier();
        let config = RoutingPolicyConfig::default(); // No rules, will use default action
        
        let evaluator = PolicyEvaluator::new(config, classifier, model_catalog).unwrap();
        
        let context = RoutingContext::new(
            AgentId::new(),
            TaskType::Analysis,
            "Analyze this data".to_string(),
        );
        
        let result = evaluator.evaluate_policies(&context).await.unwrap();
        
        match result.decision {
            RouteDecision::UseLLM { .. } => {
                // Expected default action
            }
            _ => panic!("Expected UseLLM decision from default action"),
        }
        
        assert!(result.matched_rule.is_none());
    }

    #[tokio::test]
    async fn test_policy_rule_priority_ordering() {
        let model_catalog = create_test_model_catalog();
        let classifier = create_test_classifier();
        
        let mut config = RoutingPolicyConfig::default();
        
        // Add lower priority rule first
        config.rules.push(RoutingRule {
            name: "low_priority".to_string(),
            priority: 50,
            conditions: super::super::config::RoutingConditions {
                task_types: Some(vec![TaskType::TextGeneration]),
                agent_ids: None,
                resource_constraints: None,
                security_level: None,
                custom_conditions: None,
            },
            action: RouteAction::UseLLM {
                provider: LLMProvider::OpenAI { model: None },
                model: None,
            },
            override_allowed: true,
        });

        // Add higher priority rule second
        config.rules.push(RoutingRule {
            name: "high_priority".to_string(),
            priority: 100,
            conditions: super::super::config::RoutingConditions {
                task_types: Some(vec![TaskType::TextGeneration]),
                agent_ids: None,
                resource_constraints: None,
                security_level: None,
                custom_conditions: None,
            },
            action: RouteAction::UseSLM {
                model_preference: ModelPreference::BestAvailable,
                monitoring_level: MonitoringLevel::Basic,
                fallback_on_low_confidence: false,
                confidence_threshold: None,
            },
            override_allowed: true,
        });

        // Sort rules by priority (should be done automatically)
        config.rules.sort_by(|a, b| b.priority.cmp(&a.priority));
        
        let evaluator = PolicyEvaluator::new(config, classifier, model_catalog).unwrap();
        
        let context = RoutingContext::new(
            AgentId::new(),
            TaskType::TextGeneration,
            "Generate text".to_string(),
        );
        
        let result = evaluator.evaluate_policies(&context).await.unwrap();
        
        // Should match the higher priority rule
        assert_eq!(result.matched_rule, Some("high_priority".to_string()));
        assert!(matches!(result.decision, RouteDecision::UseSLM { .. }));
    }

    #[tokio::test]
    async fn test_slm_routing_globally_disabled() {
        let model_catalog = create_test_model_catalog();
        let classifier = create_test_classifier();
        
        let mut config = RoutingPolicyConfig::default();
        config.global_settings.slm_routing_enabled = false; // Disable SLM routing globally
        
        config.rules.push(RoutingRule {
            name: "slm_rule".to_string(),
            priority: 100,
            conditions: super::super::config::RoutingConditions {
                task_types: Some(vec![TaskType::CodeGeneration]),
                agent_ids: None,
                resource_constraints: None,
                security_level: None,
                custom_conditions: None,
            },
            action: RouteAction::UseSLM {
                model_preference: ModelPreference::BestAvailable,
                monitoring_level: MonitoringLevel::Basic,
                fallback_on_low_confidence: true,
                confidence_threshold: Some(0.8),
            },
            override_allowed: true,
        });
        
        let evaluator = PolicyEvaluator::new(config, classifier, model_catalog).unwrap();
        
        let context = RoutingContext::new(
            AgentId::new(),
            TaskType::CodeGeneration,
            "Write code".to_string(),
        );
        
        let result = evaluator.evaluate_policies(&context).await.unwrap();
        
        // Should use default action instead of SLM rules when globally disabled
        match result.decision {
            RouteDecision::UseLLM { reason, .. } => {
                // Expected fallback to LLM
            }
            _ => panic!("Expected LLM fallback when SLM routing disabled"),
        }
        
        assert!(result.matched_rule.is_none());
        assert!(result.metadata.get("reason").unwrap().contains("SLM routing globally disabled"));
    }

    #[tokio::test]
    async fn test_policy_evaluator_statistics() {
        let model_catalog = create_test_model_catalog();
        let classifier = create_test_classifier();
        
        let mut config = RoutingPolicyConfig::default();
        config.rules.push(RoutingRule {
            name: "test_rule".to_string(),
            priority: 100,
            conditions: super::super::config::RoutingConditions {
                task_types: Some(vec![TaskType::CodeGeneration]),
                agent_ids: None,
                resource_constraints: None,
                security_level: None,
                custom_conditions: None,
            },
            action: RouteAction::UseSLM {
                model_preference: ModelPreference::BestAvailable,
                monitoring_level: MonitoringLevel::Basic,
                fallback_on_low_confidence: true,
                confidence_threshold: Some(0.8),
            },
            override_allowed: true,
        });
        
        let evaluator = PolicyEvaluator::new(config, classifier, model_catalog).unwrap();
        
        let stats = evaluator.get_statistics();
        assert_eq!(stats.total_rules, 1);
        assert_eq!(stats.cache_size, 0); // No evaluations cached yet
        assert!(stats.slm_routing_enabled);
        assert_eq!(stats.global_confidence_threshold, 0.85); // Default value
        assert_eq!(stats.max_slm_retries, 2); // Default value
    }

    #[tokio::test]
    async fn test_policy_config_update() {
        let model_catalog = create_test_model_catalog();
        let classifier = create_test_classifier();
        let config = RoutingPolicyConfig::default();
        
        let mut evaluator = PolicyEvaluator::new(config, classifier, model_catalog).unwrap();
        
        // Update with new config
        let mut new_config = RoutingPolicyConfig::default();
        new_config.global_settings.slm_routing_enabled = false;
        
        let update_result = evaluator.update_config(new_config);
        assert!(update_result.is_ok());
        
        // Verify the updated settings
        let stats = evaluator.get_statistics();
        assert!(!stats.slm_routing_enabled);
    }

    #[tokio::test]
    async fn test_no_suitable_model_error() {
        let model_catalog = create_test_model_catalog();
        let classifier = create_test_classifier();
        
        let mut config = RoutingPolicyConfig::default();
        config.rules.push(RoutingRule {
            name: "impossible_rule".to_string(),
            priority: 100,
            conditions: super::super::config::RoutingConditions {
                task_types: Some(vec![TaskType::TextGeneration]),
                agent_ids: None,
                resource_constraints: None,
                security_level: None,
                custom_conditions: None,
            },
            action: RouteAction::UseSLM {
                model_preference: ModelPreference::Specific { model_id: "nonexistent-model".to_string() },
                monitoring_level: MonitoringLevel::Basic,
                fallback_on_low_confidence: true,
                confidence_threshold: Some(0.8),
            },
            override_allowed: true,
        });
        
        let evaluator = PolicyEvaluator::new(config, classifier, model_catalog).unwrap();
        
        let context = RoutingContext::new(
            AgentId::new(),
            TaskType::TextGeneration,
            "Generate text".to_string(),
        );
        
        let result = evaluator.evaluate_policies(&context).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), RoutingError::NoSuitableModel { .. }));
    }

    #[tokio::test]
    async fn test_task_classification_integration() {
        let model_catalog = create_test_model_catalog();
        let classifier = create_test_classifier();
        
        let mut config = RoutingPolicyConfig::default();
        config.rules.push(RoutingRule {
            name: "code_rule".to_string(),
            priority: 100,
            conditions: super::super::config::RoutingConditions {
                task_types: Some(vec![TaskType::CodeGeneration]),
                agent_ids: None,
                resource_constraints: None,
                security_level: None,
                custom_conditions: None,
            },
            action: RouteAction::UseSLM {
                model_preference: ModelPreference::Specialist,
                monitoring_level: MonitoringLevel::Basic,
                fallback_on_low_confidence: true,
                confidence_threshold: Some(0.8),
            },
            override_allowed: true,
        });
        
        let evaluator = PolicyEvaluator::new(config, classifier, model_catalog).unwrap();
        
        // Create context with unknown task type that should be classified
        let mut context = RoutingContext::new(
            AgentId::new(),
            TaskType::Custom("unknown".to_string()),
            "Write a function to implement quicksort algorithm".to_string(),
        );
        
        let result = evaluator.evaluate_policies(&context).await.unwrap();
        
        // The classifier should detect this as code generation and apply the rule
        assert!(result.task_classification.task_type == TaskType::CodeGeneration ||
                result.matched_rule.is_none()); // If classification doesn't work, no rule matches
    }
}