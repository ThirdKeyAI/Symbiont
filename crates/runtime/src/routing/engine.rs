//! Core routing engine implementation

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use super::config::RoutingConfig;
use super::decision::{
    RouteDecision, RoutingContext, RoutingStatistics, ModelRequest, ModelResponse, 
    FinishReason, TokenUsage, LLMProvider
};
use super::error::RoutingError;
use super::policy::PolicyEvaluator;
use super::classifier::TaskClassifier;
use super::confidence::{ConfidenceMonitorTrait, NoOpConfidenceMonitor};
use crate::models::{ModelCatalog, SlmRunnerError};
use crate::logging::{ModelLogger, ModelInteractionType, RequestData, ResponseData, TokenUsage as LogTokenUsage};

/// Core routing engine trait for SLM-first architecture
#[async_trait]
pub trait RoutingEngine: Send + Sync {
    /// Route a model request based on configured policies
    async fn route_request(
        &self,
        context: &RoutingContext,
    ) -> Result<RouteDecision, RoutingError>;
    
    /// Execute the routing decision and handle fallbacks
    async fn execute_with_routing(
        &self,
        context: RoutingContext,
        request: ModelRequest,
    ) -> Result<ModelResponse, RoutingError>;
    
    /// Validate routing policies
    fn validate_policies(&self) -> Result<(), RoutingError>;
    
    /// Get routing statistics
    async fn get_routing_stats(&self) -> RoutingStatistics;
    
    /// Update routing configuration
    async fn update_config(&self, config: RoutingConfig) -> Result<(), RoutingError>;
}

/// Default implementation of the routing engine
pub struct DefaultRoutingEngine {
    /// Policy evaluator for making routing decisions
    policy_evaluator: Arc<RwLock<PolicyEvaluator>>,
    /// Model catalog for SLM information
    model_catalog: Arc<ModelCatalog>,
    /// Confidence monitor for evaluating SLM responses
    confidence_monitor: Arc<RwLock<Box<dyn ConfidenceMonitorTrait>>>,
    /// Optional model logger for audit trails
    model_logger: Option<Arc<ModelLogger>>,
    /// Routing statistics
    statistics: Arc<RwLock<RoutingStatistics>>,
    /// Configuration
    config: Arc<RwLock<RoutingConfig>>,
    /// LLM client pool for fallback
    llm_clients: Arc<LLMClientPool>,
}

/// Pool of LLM clients for different providers
struct LLMClientPool {
    clients: HashMap<String, Box<dyn LLMClient>>,
}

/// Trait for LLM clients
#[async_trait]
trait LLMClient: Send + Sync {
    async fn execute_request(
        &self,
        request: &ModelRequest,
        provider: &LLMProvider,
    ) -> Result<ModelResponse, RoutingError>;
}

/// Mock LLM client implementation
#[derive(Debug)]
struct MockLLMClient;

#[async_trait]
impl LLMClient for MockLLMClient {
    async fn execute_request(
        &self,
        request: &ModelRequest,
        provider: &LLMProvider,
    ) -> Result<ModelResponse, RoutingError> {
        // Mock implementation - in real system would call actual LLM APIs
        tokio::time::sleep(Duration::from_millis(100)).await;
        
        Ok(ModelResponse {
            content: format!("LLM response to: {}", request.prompt),
            finish_reason: FinishReason::Stop,
            token_usage: Some(TokenUsage {
                prompt_tokens: request.prompt.len() as u32 / 4,
                completion_tokens: 50,
                total_tokens: (request.prompt.len() as u32 / 4) + 50,
            }),
            metadata: {
                let mut meta = HashMap::new();
                meta.insert("provider".to_string(), serde_json::Value::String(provider.to_string()));
                meta.insert("mock".to_string(), serde_json::Value::Bool(true));
                meta
            },
            confidence_score: Some(0.95),
        })
    }
}

impl LLMClientPool {
    fn new() -> Self {
        let mut clients: HashMap<String, Box<dyn LLMClient>> = HashMap::new();
        clients.insert("openai".to_string(), Box::new(MockLLMClient));
        clients.insert("anthropic".to_string(), Box::new(MockLLMClient));
        clients.insert("custom".to_string(), Box::new(MockLLMClient));
        
        Self { clients }
    }
    
    async fn execute_request(
        &self,
        request: &ModelRequest,
        provider: &LLMProvider,
    ) -> Result<ModelResponse, RoutingError> {
        let client_key = match provider {
            LLMProvider::OpenAI { .. } => "openai",
            LLMProvider::Anthropic { .. } => "anthropic", 
            LLMProvider::Custom { .. } => "custom",
        };
        
        let client = self.clients.get(client_key)
            .ok_or_else(|| RoutingError::LLMFallbackFailed {
                provider: provider.to_string(),
                reason: "No client available for provider".to_string(),
            })?;
        
        client.execute_request(request, provider).await
    }
}

impl DefaultRoutingEngine {
    /// Create a new routing engine with the given configuration
    pub async fn new(
        config: RoutingConfig,
        model_catalog: ModelCatalog,
        model_logger: Option<Arc<ModelLogger>>,
    ) -> Result<Self, RoutingError> {
        Self::new_with_confidence_monitor(
            config,
            model_catalog,
            model_logger,
            Box::new(NoOpConfidenceMonitor),
        ).await
    }

    /// Create a new routing engine with a custom confidence monitor implementation
    /// This allows enterprise builds to inject their own confidence monitor
    pub async fn new_with_confidence_monitor(
        config: RoutingConfig,
        model_catalog: ModelCatalog,
        model_logger: Option<Arc<ModelLogger>>,
        confidence_monitor: Box<dyn ConfidenceMonitorTrait>,
    ) -> Result<Self, RoutingError> {
        // Create task classifier
        let classifier = TaskClassifier::new(config.classification.clone())?;
        
        // Create policy evaluator
        let policy_evaluator = PolicyEvaluator::new(
            config.policy.clone(),
            classifier,
            model_catalog.clone(),
        )?;
        
        // Create LLM client pool
        let llm_clients = Arc::new(LLMClientPool::new());
        
        let engine = Self {
            policy_evaluator: Arc::new(RwLock::new(policy_evaluator)),
            model_catalog: Arc::new(model_catalog),
            confidence_monitor: Arc::new(RwLock::new(confidence_monitor)),
            model_logger,
            statistics: Arc::new(RwLock::new(RoutingStatistics::default())),
            config: Arc::new(RwLock::new(config)),
            llm_clients,
        };
        
        Ok(engine)
    }
    
    /// Execute an SLM route with monitoring and fallback
    async fn execute_slm_route(
        &self,
        context: &RoutingContext,
        request: &ModelRequest,
        model_id: &str,
        monitoring_level: &super::decision::MonitoringLevel,
        fallback_on_failure: bool,
    ) -> Result<ModelResponse, RoutingError> {
        let _start_time = Instant::now();
        
        // Get the model from catalog
        let model = self.model_catalog.get_model(model_id)
            .ok_or_else(|| RoutingError::NoSuitableModel { 
                task_type: context.task_type.clone() 
            })?;
        
        // Execute the SLM (mock implementation)
        let slm_result = self.execute_slm_mock(request, model).await;
        
        match slm_result {
            Ok(response) => {
                // Evaluate confidence if monitoring is enabled
                let should_fallback = match monitoring_level {
                    super::decision::MonitoringLevel::None => false,
                    super::decision::MonitoringLevel::Basic => {
                        // Basic monitoring - check for obvious failures
                        response.finish_reason != FinishReason::Stop
                    }
                    super::decision::MonitoringLevel::Enhanced { confidence_threshold } => {
                        // For enhanced monitoring, use confidence score if available
                        // Enterprise builds can inject more sophisticated confidence monitors
                        let confidence_score = response.confidence_score.unwrap_or(0.5);
                        confidence_score < *confidence_threshold
                    }
                };
                
                if should_fallback && fallback_on_failure {
                    tracing::warn!(
                        "SLM response did not meet confidence threshold, falling back to LLM"
                    );
                    
                    // Update statistics for fallback
                    {
                        let mut stats = self.statistics.write().await;
                        stats.fallback_routes += 1;
                    }
                    
                    self.execute_llm_fallback(request, "Low confidence SLM response").await
                } else {
                    // Note: In enterprise mode, the ConfidenceMonitor would record evaluation results
                    // but the trait interface doesn't expose this method to keep OSS code clean
                    Ok(response)
                }
            }
            Err(e) => {
                if fallback_on_failure {
                    tracing::error!("SLM execution failed, falling back to LLM: {}", e);
                    self.execute_llm_fallback(request, &format!("SLM execution failed: {}", e)).await
                } else {
                    Err(RoutingError::ModelExecutionFailed {
                        model_id: model_id.to_string(),
                        reason: e.to_string(),
                    })
                }
            }
        }
    }
    
    /// Mock SLM execution (in real implementation, would use SlmRunner)
    async fn execute_slm_mock(
        &self,
        request: &ModelRequest,
        model: &crate::config::Model,
    ) -> Result<ModelResponse, SlmRunnerError> {
        // Simulate SLM execution time
        tokio::time::sleep(Duration::from_millis(200)).await;
        
        // Simulate potential failure for certain inputs
        if request.prompt.contains("error") {
            return Err(SlmRunnerError::ExecutionFailed {
                reason: "Simulated execution error".to_string(),
            });
        }
        
        Ok(ModelResponse {
            content: format!("SLM ({}) response: {}", model.name, request.prompt),
            finish_reason: FinishReason::Stop,
            token_usage: Some(TokenUsage {
                prompt_tokens: request.prompt.len() as u32 / 4,
                completion_tokens: 30,
                total_tokens: (request.prompt.len() as u32 / 4) + 30,
            }),
            metadata: {
                let mut meta = HashMap::new();
                meta.insert("model_id".to_string(), serde_json::Value::String(model.id.clone()));
                meta.insert("provider".to_string(), serde_json::Value::String(format!("{:?}", model.provider)));
                meta
            },
            confidence_score: Some(0.8 + (request.prompt.len() % 20) as f64 / 100.0), // Mock confidence
        })
    }
    
    /// Execute LLM fallback
    async fn execute_llm_fallback(
        &self,
        request: &ModelRequest,
        _reason: &str,
    ) -> Result<ModelResponse, RoutingError> {
        let config = self.config.read().await;
        let fallback_config = &config.policy.fallback_config;
        
        if !fallback_config.enabled {
            return Err(RoutingError::LLMFallbackFailed {
                provider: "disabled".to_string(),
                reason: "LLM fallback is disabled".to_string(),
            });
        }
        
        // Try primary provider first
        let provider = LLMProvider::OpenAI { model: None };
        
        match self.llm_clients.execute_request(request, &provider).await {
            Ok(response) => Ok(response),
            Err(e) => Err(RoutingError::LLMFallbackFailed {
                provider: provider.to_string(),
                reason: e.to_string(),
            }),
        }
    }
    
    /// Log routing decision and execution
    async fn log_routing_execution(
        &self,
        context: &RoutingContext,
        decision: &RouteDecision,
        request: &ModelRequest,
        response: &ModelResponse,
        execution_time: Duration,
        error: Option<&RoutingError>,
    ) {
        if let Some(ref logger) = self.model_logger {
            let model_name = match decision {
                RouteDecision::UseSLM { model_id, .. } => model_id.clone(),
                RouteDecision::UseLLM { provider, .. } => provider.to_string(),
                RouteDecision::Deny { .. } => "denied".to_string(),
            };
            
            let request_data = RequestData {
                prompt: request.prompt.clone(),
                tool_name: None,
                tool_arguments: None,
                parameters: {
                    let mut params = HashMap::new();
                    params.insert("routing_decision".to_string(), 
                        serde_json::Value::String(format!("{:?}", decision)));
                    params.insert("task_type".to_string(),
                        serde_json::Value::String(context.task_type.to_string()));
                    params
                },
            };
            
            let response_data = ResponseData {
                content: response.content.clone(),
                tool_result: None,
                confidence: response.confidence_score,
                metadata: response.metadata.clone(),
            };
            
            let metadata = {
                let mut meta = HashMap::new();
                meta.insert("routing_engine".to_string(), "default".to_string());
                meta.insert("agent_id".to_string(), context.agent_id.to_string());
                meta.insert("request_id".to_string(), context.request_id.clone());
                meta
            };
            
            if let Err(e) = logger.log_interaction(
                context.agent_id,
                ModelInteractionType::Completion,
                &model_name,
                request_data,
                response_data,
                execution_time,
                metadata,
                response.token_usage.as_ref().map(|t| LogTokenUsage {
                    input_tokens: t.prompt_tokens,
                    output_tokens: t.completion_tokens,
                    total_tokens: t.total_tokens,
                }),
                error.map(|e| e.to_string()),
            ).await {
                tracing::warn!("Failed to log routing execution: {}", e);
            }
        }
    }
}

#[async_trait]
impl RoutingEngine for DefaultRoutingEngine {
    async fn route_request(
        &self,
        context: &RoutingContext,
    ) -> Result<RouteDecision, RoutingError> {
        let policy_result = self.policy_evaluator
            .read()
            .await
            .evaluate_policies(context)
            .await?;
        
        tracing::debug!(
            "Routing decision for agent {}: {:?} (rule: {:?})",
            context.agent_id,
            policy_result.decision,
            policy_result.matched_rule
        );
        
        Ok(policy_result.decision)
    }
    
    async fn execute_with_routing(
        &self,
        context: RoutingContext,
        request: ModelRequest,
    ) -> Result<ModelResponse, RoutingError> {
        let start_time = Instant::now();
        let route_decision = self.route_request(&context).await?;
        
        let result = match &route_decision {
            RouteDecision::UseSLM { model_id, monitoring, fallback_on_failure } => {
                self.execute_slm_route(
                    &context,
                    &request,
                    model_id,
                    monitoring,
                    *fallback_on_failure,
                ).await
            }
            RouteDecision::UseLLM { provider, reason } => {
                tracing::info!("Routing to LLM: {}", reason);
                self.llm_clients.execute_request(&request, provider).await
            }
            RouteDecision::Deny { reason, policy_violated } => {
                return Err(RoutingError::RoutingDenied {
                    policy: policy_violated.clone(),
                    reason: reason.clone(),
                });
            }
        };
        
        let execution_time = start_time.elapsed();
        
        // Update statistics
        {
            let mut stats = self.statistics.write().await;
            stats.update(
                &route_decision,
                execution_time,
                result.is_ok(),
            );
            
            if let Ok(ref response) = result {
                if let Some(confidence) = response.confidence_score {
                    stats.add_confidence_score(confidence);
                }
            }
        }
        
        // Log the execution
        match &result {
            Ok(response) => {
                self.log_routing_execution(
                    &context,
                    &route_decision,
                    &request,
                    response,
                    execution_time,
                    None,
                ).await;
            }
            Err(error) => {
                // Create a dummy response for logging
                let dummy_response = ModelResponse {
                    content: "Error occurred".to_string(),
                    finish_reason: FinishReason::Error,
                    token_usage: None,
                    metadata: HashMap::new(),
                    confidence_score: Some(0.0),
                };
                
                self.log_routing_execution(
                    &context,
                    &route_decision,
                    &request,
                    &dummy_response,
                    execution_time,
                    Some(error),
                ).await;
            }
        }
        
        result
    }
    
    fn validate_policies(&self) -> Result<(), RoutingError> {
        // Validation is done during PolicyEvaluator creation
        Ok(())
    }
    
    async fn get_routing_stats(&self) -> RoutingStatistics {
        self.statistics.read().await.clone()
    }
    
    async fn update_config(&self, config: RoutingConfig) -> Result<(), RoutingError> {
        // Update configuration
        *self.config.write().await = config.clone();
        
        // Update policy evaluator
        self.policy_evaluator
            .write()
            .await
            .update_config(config.policy)?;
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::AgentId;
    use crate::config::{Slm, Model, ModelProvider, ModelResourceRequirements, ModelAllowListConfig, SandboxProfile};
    use std::path::PathBuf;
    use std::collections::HashMap;

    async fn create_test_engine() -> DefaultRoutingEngine {
        let mut global_models = Vec::new();
        
        global_models.push(Model {
            id: "test-slm".to_string(),
            name: "Test SLM".to_string(),
            provider: ModelProvider::LocalFile { file_path: PathBuf::from("/tmp/test.gguf") },
            capabilities: vec![
                crate::config::ModelCapability::TextGeneration,
                crate::config::ModelCapability::CodeGeneration,
            ],
            resource_requirements: ModelResourceRequirements {
                min_memory_mb: 1024,
                preferred_cpu_cores: 2.0,
                gpu_requirements: None,
            },
        });

        global_models.push(Model {
            id: "error-slm".to_string(),
            name: "Error SLM".to_string(),
            provider: ModelProvider::LocalFile { file_path: PathBuf::from("/tmp/error.gguf") },
            capabilities: vec![crate::config::ModelCapability::TextGeneration],
            resource_requirements: ModelResourceRequirements {
                min_memory_mb: 512,
                preferred_cpu_cores: 1.0,
                gpu_requirements: None,
            },
        });

        let mut sandbox_profiles = HashMap::new();
        sandbox_profiles.insert("default".to_string(), SandboxProfile::secure_default());
        
        let slm_config = Slm {
            enabled: true,
            model_allow_lists: ModelAllowListConfig {
                global_models,
                agent_model_maps: HashMap::new(),
                allow_runtime_overrides: false,
            },
            sandbox_profiles,
            default_sandbox_profile: "default".to_string(),
        };
        
        let model_catalog = ModelCatalog::new(slm_config).unwrap();
        let config = RoutingConfig::default();
        
        DefaultRoutingEngine::new(config, model_catalog, None).await.unwrap()
    }

    async fn create_test_engine_with_logger() -> DefaultRoutingEngine {
        let logger = ModelLogger::new(super::super::super::logging::LoggingConfig::default()).await.unwrap();
        
        let mut global_models = Vec::new();
        global_models.push(Model {
            id: "test-slm".to_string(),
            name: "Test SLM".to_string(),
            provider: ModelProvider::LocalFile { file_path: PathBuf::from("/tmp/test.gguf") },
            capabilities: vec![crate::config::ModelCapability::TextGeneration],
            resource_requirements: ModelResourceRequirements {
                min_memory_mb: 1024,
                preferred_cpu_cores: 2.0,
                gpu_requirements: None,
            },
        });

        let mut sandbox_profiles = HashMap::new();
        sandbox_profiles.insert("default".to_string(), SandboxProfile::secure_default());
        
        let slm_config = Slm {
            enabled: true,
            model_allow_lists: ModelAllowListConfig {
                global_models,
                agent_model_maps: HashMap::new(),
                allow_runtime_overrides: false,
            },
            sandbox_profiles,
            default_sandbox_profile: "default".to_string(),
        };
        
        let model_catalog = ModelCatalog::new(slm_config).unwrap();
        let config = RoutingConfig::default();
        
        DefaultRoutingEngine::new(config, model_catalog, Some(Arc::new(logger))).await.unwrap()
    }

    fn create_test_request(prompt: &str) -> ModelRequest {
        ModelRequest::from_task(prompt.to_string())
    }

    fn create_test_context(prompt: &str, task_type: super::super::error::TaskType) -> RoutingContext {
        RoutingContext::new(AgentId::new(), task_type, prompt.to_string())
    }
    
    #[tokio::test]
    async fn test_routing_engine_creation() {
        let engine = create_test_engine().await;
        
        // Verify engine was created successfully
        let stats = engine.get_routing_stats().await;
        assert_eq!(stats.total_requests, 0);
        
        // Verify policies can be validated
        assert!(engine.validate_policies().is_ok());
    }

    #[tokio::test]
    async fn test_routing_engine_with_logger() {
        let engine = create_test_engine_with_logger().await;
        
        // Should have logger configured
        assert!(engine.model_logger.is_some());
        
        let stats = engine.get_routing_stats().await;
        assert_eq!(stats.total_requests, 0);
    }
    
    #[tokio::test]
    async fn test_routing_engine_basic_flow() {
        let engine = create_test_engine().await;
        
        let context = create_test_context(
            "Write a hello world function",
            super::super::error::TaskType::CodeGeneration
        );
        
        let decision = engine.route_request(&context).await.unwrap();
        
        // Should get some kind of routing decision
        match decision {
            RouteDecision::UseSLM { .. } | RouteDecision::UseLLM { .. } => {
                // Expected outcomes
            }
            RouteDecision::Deny { .. } => {
                panic!("Should not deny basic request");
            }
        }
    }
    
    #[tokio::test]
    async fn test_execute_with_routing_slm_success() {
        let engine = create_test_engine().await;
        
        let context = create_test_context(
            "Write a hello world function",
            super::super::error::TaskType::CodeGeneration
        );
        
        let request = create_test_request("Write a hello world function");
        
        let response = engine.execute_with_routing(context, request).await.unwrap();
        
        assert!(!response.content.is_empty());
        assert!(response.confidence_score.is_some());
        assert_eq!(response.finish_reason, FinishReason::Stop);
        
        // Check that statistics were updated
        let stats = engine.get_routing_stats().await;
        assert!(stats.total_requests > 0);
    }

    #[tokio::test]
    async fn test_execute_with_routing_slm_error_fallback() {
        let engine = create_test_engine().await;
        
        let context = create_test_context(
            "This should trigger an error in SLM",
            super::super::error::TaskType::TextGeneration
        );
        
        let request = create_test_request("error trigger");
        
        let response = engine.execute_with_routing(context, request).await.unwrap();
        
        // Should get LLM fallback response
        assert!(!response.content.is_empty());
        assert!(response.content.contains("LLM response"));
        
        let stats = engine.get_routing_stats().await;
        assert!(stats.fallback_routes > 0);
    }

    #[tokio::test]
    async fn test_slm_execution_success() {
        let engine = create_test_engine().await;
        
        let context = create_test_context(
            "Test prompt",
            super::super::error::TaskType::TextGeneration
        );
        
        let request = create_test_request("Test prompt");
        
        let response = engine.execute_slm_route(
            &context,
            &request,
            "test-slm",
            &super::decision::MonitoringLevel::Basic,
            true,
        ).await.unwrap();
        
        assert!(!response.content.is_empty());
        assert!(response.content.contains("Test SLM"));
        assert!(response.confidence_score.is_some());
    }

    #[tokio::test]
    async fn test_slm_execution_with_enhanced_monitoring() {
        let engine = create_test_engine().await;
        
        let context = create_test_context(
            "Test prompt with monitoring",
            super::super::error::TaskType::TextGeneration
        );
        
        let request = create_test_request("Test prompt with monitoring");
        
        let response = engine.execute_slm_route(
            &context,
            &request,
            "test-slm",
            &super::decision::MonitoringLevel::Enhanced { confidence_threshold: 0.9 },
            true,
        ).await.unwrap();
        
        // Should either get SLM response or LLM fallback
        assert!(!response.content.is_empty());
    }

    #[tokio::test]
    async fn test_slm_execution_no_monitoring() {
        let engine = create_test_engine().await;
        
        let context = create_test_context(
            "Test prompt no monitoring",
            super::super::error::TaskType::TextGeneration
        );
        
        let request = create_test_request("Test prompt no monitoring");
        
        let response = engine.execute_slm_route(
            &context,
            &request,
            "test-slm",
            &super::decision::MonitoringLevel::None,
            true,
        ).await.unwrap();
        
        // Should get SLM response without monitoring
        assert!(!response.content.is_empty());
        assert!(response.content.contains("Test SLM"));
    }

    #[tokio::test]
    async fn test_slm_execution_error_no_fallback() {
        let engine = create_test_engine().await;
        
        let context = create_test_context(
            "error trigger",
            super::super::error::TaskType::TextGeneration
        );
        
        let request = create_test_request("error trigger");
        
        let result = engine.execute_slm_route(
            &context,
            &request,
            "test-slm",
            &super::decision::MonitoringLevel::Basic,
            false, // No fallback
        ).await;
        
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), RoutingError::ModelExecutionFailed { .. }));
    }

    #[tokio::test]
    async fn test_slm_execution_nonexistent_model() {
        let engine = create_test_engine().await;
        
        let context = create_test_context(
            "Test prompt",
            super::super::error::TaskType::TextGeneration
        );
        
        let request = create_test_request("Test prompt");
        
        let result = engine.execute_slm_route(
            &context,
            &request,
            "nonexistent-model",
            &super::decision::MonitoringLevel::Basic,
            true,
        ).await;
        
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), RoutingError::NoSuitableModel { .. }));
    }

    #[tokio::test]
    async fn test_llm_fallback_execution() {
        let engine = create_test_engine().await;
        
        let request = create_test_request("Test LLM fallback");
        
        let response = engine.execute_llm_fallback(&request, "Test reason").await.unwrap();
        
        assert!(!response.content.is_empty());
        assert!(response.content.contains("LLM response"));
        assert_eq!(response.finish_reason, FinishReason::Stop);
        assert!(response.confidence_score.is_some());
    }

    #[tokio::test]
    async fn test_llm_fallback_disabled() {
        let engine = create_test_engine().await;
        
        // Disable fallback in config
        {
            let mut config = engine.config.write().await;
            config.policy.fallback_config.enabled = false;
        }
        
        let request = create_test_request("Test disabled fallback");
        
        let result = engine.execute_llm_fallback(&request, "Test reason").await;
        
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), RoutingError::LLMFallbackFailed { .. }));
    }

    #[tokio::test]
    async fn test_llm_client_pool() {
        let pool = LLMClientPool::new();
        
        let request = create_test_request("Test LLM client");
        
        // Test OpenAI provider
        let openai_response = pool.execute_request(
            &request,
            &LLMProvider::OpenAI { model: Some("gpt-3.5-turbo".to_string()) }
        ).await.unwrap();
        
        assert!(!openai_response.content.is_empty());
        assert!(openai_response.metadata.contains_key("provider"));
        
        // Test Anthropic provider
        let anthropic_response = pool.execute_request(
            &request,
            &LLMProvider::Anthropic { model: Some("claude-3".to_string()) }
        ).await.unwrap();
        
        assert!(!anthropic_response.content.is_empty());
        assert!(anthropic_response.metadata.contains_key("provider"));
        
        // Test Custom provider
        let custom_response = pool.execute_request(
            &request,
            &LLMProvider::Custom { endpoint: "http://localhost:8080".to_string(), model: None }
        ).await.unwrap();
        
        assert!(!custom_response.content.is_empty());
    }

    #[tokio::test]
    async fn test_mock_slm_execution() {
        let engine = create_test_engine().await;
        
        let request = create_test_request("Test SLM execution");
        let model = engine.model_catalog.get_model("test-slm").unwrap();
        
        let response = engine.execute_slm_mock(&request, model).await.unwrap();
        
        assert!(!response.content.is_empty());
        assert!(response.content.contains("Test SLM"));
        assert!(response.content.contains("Test SLM execution"));
        assert_eq!(response.finish_reason, FinishReason::Stop);
        assert!(response.confidence_score.is_some());
        assert!(response.token_usage.is_some());
    }

    #[tokio::test]
    async fn test_mock_slm_execution_error() {
        let engine = create_test_engine().await;
        
        let request = create_test_request("This should error out");
        let model = engine.model_catalog.get_model("test-slm").unwrap();
        
        let result = engine.execute_slm_mock(&request, model).await;
        
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), SlmRunnerError::ExecutionFailed { .. }));
    }
    
    #[tokio::test]
    async fn test_routing_statistics_tracking() {
        let engine = create_test_engine().await;
        
        // Execute a few requests to track statistics
        let context1 = create_test_context("Test 1", super::super::error::TaskType::TextGeneration);
        let request1 = create_test_request("Test 1");
        
        let _response1 = engine.execute_with_routing(context1, request1).await.unwrap();
        
        let context2 = create_test_context("error trigger", super::super::error::TaskType::TextGeneration);
        let request2 = create_test_request("error trigger");
        
        let _response2 = engine.execute_with_routing(context2, request2).await.unwrap();
        
        let stats = engine.get_routing_stats().await;
        
        assert!(stats.total_requests > 0);
        assert!(stats.fallback_routes > 0); // Second request should trigger fallback
        assert!(stats.average_response_time > Duration::from_millis(0));
    }

    #[tokio::test]
    async fn test_config_update() {
        let engine = create_test_engine().await;
        
        let mut new_config = RoutingConfig::default();
        new_config.policy.fallback_config.enabled = false;
        
        let result = engine.update_config(new_config.clone()).await;
        assert!(result.is_ok());
        
        // Verify config was updated
        let updated_config = engine.config.read().await;
        assert!(!updated_config.policy.fallback_config.enabled);
    }

    #[tokio::test]
    async fn test_routing_with_deny_decision() {
        let engine = create_test_engine().await;
        
        // Create a routing context that would trigger a deny decision
        // (This would need specific policy configuration to work in practice)
        let context = create_test_context(
            "forbidden operation",
            super::super::error::TaskType::Custom("forbidden".to_string())
        );
        
        let request = create_test_request("forbidden operation");
        
        // This might not trigger a deny in the default config, but test the error handling
        let result = engine.execute_with_routing(context, request).await;
        
        // Should either succeed with a response or fail with specific error
        match result {
            Ok(response) => {
                assert!(!response.content.is_empty());
            }
            Err(RoutingError::RoutingDenied { .. }) => {
                // Expected for deny decision
            }
            Err(e) => {
                panic!("Unexpected error: {:?}", e);
            }
        }
    }

    #[tokio::test]
    async fn test_logging_integration() {
        let engine = create_test_engine_with_logger().await;
        
        let context = create_test_context(
            "Test logging integration",
            super::super::error::TaskType::TextGeneration
        );
        
        let request = create_test_request("Test logging integration");
        
        let response = engine.execute_with_routing(context, request).await.unwrap();
        
        assert!(!response.content.is_empty());
        // Logging should happen in the background without affecting the response
    }

    #[tokio::test]
    async fn test_confidence_monitoring_integration() {
        let engine = create_test_engine().await;
        
        let context = create_test_context(
            "Test confidence monitoring",
            super::super::error::TaskType::CodeGeneration
        );
        
        let request = create_test_request("Test confidence monitoring");
        
        let response = engine.execute_with_routing(context, request).await.unwrap();
        
        assert!(!response.content.is_empty());
        assert!(response.confidence_score.is_some());
        
        // Note: Confidence monitoring statistics are only available in enterprise mode
        // The trait interface doesn't expose statistics to keep OSS code clean
    }

    #[tokio::test]
    async fn test_policy_evaluation_integration() {
        let engine = create_test_engine().await;
        
        // Test different task types to ensure policy evaluation works
        let task_types = vec![
            super::super::error::TaskType::TextGeneration,
            super::super::error::TaskType::CodeGeneration,
            super::super::error::TaskType::Analysis,
            super::super::error::TaskType::Reasoning,
        ];
        
        for task_type in task_types {
            let context = create_test_context("Test policy evaluation", task_type.clone());
            
            let decision = engine.route_request(&context).await.unwrap();
            
            // Should get a valid routing decision for each task type
            match decision {
                RouteDecision::UseSLM { .. } | RouteDecision::UseLLM { .. } | RouteDecision::Deny { .. } => {
                    // All are valid outcomes
                }
            }
        }
    }

    #[tokio::test]
    async fn test_concurrent_routing_requests() {
        let engine = Arc::new(create_test_engine().await);
        
        let mut handles = Vec::new();
        
        // Spawn multiple concurrent routing requests
        for i in 0..10 {
            let engine_clone = Arc::clone(&engine);
            let handle = tokio::spawn(async move {
                let context = create_test_context(
                    &format!("Concurrent request {}", i),
                    super::super::error::TaskType::TextGeneration
                );
                
                let request = create_test_request(&format!("Concurrent request {}", i));
                
                engine_clone.execute_with_routing(context, request).await
            });
            handles.push(handle);
        }
        
        // Wait for all requests to complete
        let results = futures::future::join_all(handles).await;
        
        // All requests should succeed
        for result in results {
            let response = result.unwrap().unwrap();
            assert!(!response.content.is_empty());
        }
        
        // Check that statistics reflect all requests
        let stats = engine.get_routing_stats().await;
        assert_eq!(stats.total_requests, 10);
    }

    #[tokio::test]
    async fn test_error_handling_and_recovery() {
        let engine = create_test_engine().await;
        
        // Test various error scenarios
        let error_scenarios = vec![
            ("error trigger", "Should trigger SLM execution error"),
            ("", "Empty prompt"),
            ("   ", "Whitespace-only prompt"),
        ];
        
        for (prompt, description) in error_scenarios {
            let context = create_test_context(prompt, super::super::error::TaskType::TextGeneration);
            let request = create_test_request(prompt);
            
            let result = engine.execute_with_routing(context, request).await;
            
            match result {
                Ok(response) => {
                    // Should get a response (likely from LLM fallback)
                    assert!(!response.content.is_empty(), "Failed for: {}", description);
                }
                Err(e) => {
                    // Some errors are expected, but should be handled gracefully
                    tracing::info!("Expected error for '{}': {:?}", description, e);
                }
            }
        }
    }

    #[tokio::test]
    async fn test_model_metadata_and_token_usage() {
        let engine = create_test_engine().await;
        
        let context = create_test_context(
            "Test metadata and token usage",
            super::super::error::TaskType::TextGeneration
        );
        
        let request = create_test_request("Test metadata and token usage");
        
        let response = engine.execute_with_routing(context, request).await.unwrap();
        
        // Verify response structure
        assert!(!response.content.is_empty());
        assert!(response.token_usage.is_some());
        assert!(!response.metadata.is_empty());
        
        let token_usage = response.token_usage.unwrap();
        assert!(token_usage.prompt_tokens > 0);
        assert!(token_usage.completion_tokens > 0);
        assert_eq!(token_usage.total_tokens, token_usage.prompt_tokens + token_usage.completion_tokens);
    }

    #[tokio::test]
    async fn test_validate_policies() {
        let engine = create_test_engine().await;
        
        let result = engine.validate_policies();
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_engine_state_consistency() {
        let engine = create_test_engine().await;
        
        // Verify initial state
        let initial_stats = engine.get_routing_stats().await;
        assert_eq!(initial_stats.total_requests, 0);
        
        // Execute some requests
        for i in 0..5 {
            let context = create_test_context(
                &format!("Test request {}", i),
                super::super::error::TaskType::TextGeneration
            );
            let request = create_test_request(&format!("Test request {}", i));
            
            let _response = engine.execute_with_routing(context, request).await.unwrap();
        }
        
        // Verify state was updated consistently
        let final_stats = engine.get_routing_stats().await;
        assert_eq!(final_stats.total_requests, 5);
        assert!(final_stats.average_response_time > Duration::from_millis(0));
        
        // Note: Confidence monitoring statistics are only available in enterprise mode
        // The trait interface doesn't expose statistics to keep OSS code clean
    }
}