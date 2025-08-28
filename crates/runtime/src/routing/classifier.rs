//! Task classification system for routing decisions

use regex::Regex;
use std::collections::HashMap;
use super::config::{TaskClassificationConfig, ClassificationPattern};
use super::decision::RoutingContext;
use super::error::{TaskType, RoutingError};

/// Task classifier for determining task types from prompts
#[derive(Debug, Clone)]
pub struct TaskClassifier {
    /// Classification patterns for each task type
    patterns: HashMap<TaskType, ClassificationPattern>,
    /// Compiled regex patterns for efficiency
    compiled_patterns: HashMap<TaskType, Vec<Regex>>,
    /// Configuration settings
    config: TaskClassificationConfig,
}

/// Classification result with confidence score
#[derive(Debug, Clone)]
pub struct ClassificationResult {
    pub task_type: TaskType,
    pub confidence: f64,
    pub matched_patterns: Vec<String>,
    pub keyword_matches: Vec<String>,
}

impl TaskClassifier {
    /// Create a new task classifier with the given configuration
    pub fn new(config: TaskClassificationConfig) -> Result<Self, RoutingError> {
        let mut compiled_patterns = HashMap::new();
        
        // Compile regex patterns for efficiency
        for (task_type, pattern) in &config.patterns {
            let mut regexes = Vec::new();
            for pattern_str in &pattern.patterns {
                let regex = Regex::new(pattern_str)
                    .map_err(|e| RoutingError::ConfigurationError {
                        key: format!("classification.patterns.{}.patterns", task_type),
                        reason: format!("Invalid regex pattern '{}': {}", pattern_str, e),
                    })?;
                regexes.push(regex);
            }
            compiled_patterns.insert(task_type.clone(), regexes);
        }
        
        Ok(Self {
            patterns: config.patterns.clone(),
            compiled_patterns,
            config,
        })
    }
    
    /// Classify a task based on the prompt and context
    pub fn classify_task(&self, prompt: &str, context: &RoutingContext) -> Result<ClassificationResult, RoutingError> {
        if !self.config.enabled {
            return Ok(ClassificationResult {
                task_type: self.config.default_task_type.clone(),
                confidence: 1.0,
                matched_patterns: vec!["classification_disabled".to_string()],
                keyword_matches: Vec::new(),
            });
        }
        
        let prompt_lower = prompt.to_lowercase();
        let mut scores = HashMap::new();
        let mut all_matches = HashMap::new();
        
        // Score each task type based on pattern matching
        for (task_type, pattern) in &self.patterns {
            let mut score = 0.0;
            let mut matches = Vec::new();
            let mut keyword_matches = Vec::new();
            
            // Check keyword matches
            for keyword in &pattern.keywords {
                if prompt_lower.contains(&keyword.to_lowercase()) {
                    score += pattern.weight * 0.5; // Keywords get half weight
                    keyword_matches.push(keyword.clone());
                }
            }
            
            // Check regex pattern matches
            if let Some(regexes) = self.compiled_patterns.get(task_type) {
                for (i, regex) in regexes.iter().enumerate() {
                    if regex.is_match(&prompt_lower) {
                        score += pattern.weight; // Full weight for regex matches
                        matches.push(pattern.patterns[i].clone());
                    }
                }
            }
            
            if score > 0.0 {
                scores.insert(task_type.clone(), score);
                all_matches.insert(task_type.clone(), (matches, keyword_matches));
            }
        }
        
        // Apply context-based adjustments
        self.apply_context_adjustments(&mut scores, context);
        
        // Find the highest scoring task type
        let best_match = scores
            .iter()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal));
        
        let (task_type, raw_score) = match best_match {
            Some((task_type, score)) => (task_type.clone(), *score),
            None => {
                // No matches found, use default
                return Ok(ClassificationResult {
                    task_type: self.config.default_task_type.clone(),
                    confidence: 0.0,
                    matched_patterns: vec!["no_patterns_matched".to_string()],
                    keyword_matches: Vec::new(),
                });
            }
        };
        
        // Normalize confidence score
        let max_possible_score = self.calculate_max_possible_score(&task_type);
        let confidence = if max_possible_score > 0.0 {
            (raw_score / max_possible_score).min(1.0)
        } else {
            0.0
        };
        
        let (matched_patterns, keyword_matches) = all_matches
            .get(&task_type)
            .cloned()
            .unwrap_or((Vec::new(), Vec::new()));
        
        // Check if confidence meets threshold
        if confidence < self.config.confidence_threshold {
            return Ok(ClassificationResult {
                task_type: self.config.default_task_type.clone(),
                confidence,
                matched_patterns: vec!["confidence_below_threshold".to_string()],
                keyword_matches: Vec::new(),
            });
        }
        
        Ok(ClassificationResult {
            task_type,
            confidence,
            matched_patterns,
            keyword_matches,
        })
    }
    
    /// Apply context-based adjustments to scores
    fn apply_context_adjustments(&self, scores: &mut HashMap<TaskType, f64>, context: &RoutingContext) {
        // Adjust based on expected output type
        match context.expected_output_type {
            super::decision::OutputType::Code => {
                // Boost code-related task types
                if let Some(score) = scores.get_mut(&TaskType::CodeGeneration) {
                    *score *= 1.5;
                }
                if let Some(score) = scores.get_mut(&TaskType::BoilerplateCode) {
                    *score *= 1.3;
                }
            }
            super::decision::OutputType::Json | super::decision::OutputType::Structured => {
                // Boost extraction and analysis tasks
                if let Some(score) = scores.get_mut(&TaskType::Extract) {
                    *score *= 1.4;
                }
                if let Some(score) = scores.get_mut(&TaskType::Analysis) {
                    *score *= 1.2;
                }
            }
            _ => {}
        }
        
        // Adjust based on agent capabilities
        for capability in &context.agent_capabilities {
            match capability.as_str() {
                "code_generation" => {
                    if let Some(score) = scores.get_mut(&TaskType::CodeGeneration) {
                        *score *= 1.2;
                    }
                }
                "analysis" => {
                    if let Some(score) = scores.get_mut(&TaskType::Analysis) {
                        *score *= 1.2;
                    }
                    if let Some(score) = scores.get_mut(&TaskType::Reasoning) {
                        *score *= 1.1;
                    }
                }
                "translation" => {
                    if let Some(score) = scores.get_mut(&TaskType::Translation) {
                        *score *= 1.3;
                    }
                }
                _ => {}
            }
        }
        
        // Adjust based on security level (higher security might prefer certain task types)
        match context.agent_security_level {
            super::decision::SecurityLevel::Critical | super::decision::SecurityLevel::High => {
                // Prefer simpler, more predictable tasks for high security
                if let Some(score) = scores.get_mut(&TaskType::Intent) {
                    *score *= 1.1;
                }
                if let Some(score) = scores.get_mut(&TaskType::Extract) {
                    *score *= 1.1;
                }
                // Slightly penalize complex reasoning tasks
                if let Some(score) = scores.get_mut(&TaskType::Reasoning) {
                    *score *= 0.9;
                }
            }
            _ => {}
        }
    }
    
    /// Calculate the maximum possible score for a task type
    fn calculate_max_possible_score(&self, task_type: &TaskType) -> f64 {
        if let Some(pattern) = self.patterns.get(task_type) {
            let keyword_score = pattern.keywords.len() as f64 * pattern.weight * 0.5;
            let pattern_score = pattern.patterns.len() as f64 * pattern.weight;
            keyword_score + pattern_score
        } else {
            1.0
        }
    }
    
    /// Add or update a classification pattern
    pub fn add_pattern(&mut self, task_type: TaskType, pattern: ClassificationPattern) -> Result<(), RoutingError> {
        // Compile regex patterns
        let mut regexes = Vec::new();
        for pattern_str in &pattern.patterns {
            let regex = Regex::new(pattern_str)
                .map_err(|e| RoutingError::ConfigurationError {
                    key: format!("pattern.{}", task_type),
                    reason: format!("Invalid regex pattern '{}': {}", pattern_str, e),
                })?;
            regexes.push(regex);
        }
        
        self.compiled_patterns.insert(task_type.clone(), regexes);
        self.patterns.insert(task_type, pattern);
        Ok(())
    }
    
    /// Remove a classification pattern
    pub fn remove_pattern(&mut self, task_type: &TaskType) {
        self.patterns.remove(task_type);
        self.compiled_patterns.remove(task_type);
    }
    
    /// Get classification statistics
    pub fn get_statistics(&self) -> ClassificationStatistics {
        ClassificationStatistics {
            total_patterns: self.patterns.len(),
            task_type_coverage: self.patterns.keys().cloned().collect(),
            total_keywords: self.patterns.values().map(|p| p.keywords.len()).sum(),
            total_regex_patterns: self.patterns.values().map(|p| p.patterns.len()).sum(),
            confidence_threshold: self.config.confidence_threshold,
            default_task_type: self.config.default_task_type.clone(),
        }
    }
}

/// Statistics about the task classifier
#[derive(Debug, Clone)]
pub struct ClassificationStatistics {
    pub total_patterns: usize,
    pub task_type_coverage: Vec<TaskType>,
    pub total_keywords: usize,
    pub total_regex_patterns: usize,
    pub confidence_threshold: f64,
    pub default_task_type: TaskType,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::AgentId;
    use super::super::decision::{RoutingContext, OutputType};

    fn create_test_config() -> TaskClassificationConfig {
        let mut patterns = HashMap::new();
        
        patterns.insert(TaskType::CodeGeneration, ClassificationPattern {
            keywords: vec!["code".to_string(), "function".to_string(), "implement".to_string()],
            patterns: vec![r"write.*code".to_string(), r"implement.*function".to_string()],
            weight: 1.0,
        });
        
        patterns.insert(TaskType::Analysis, ClassificationPattern {
            keywords: vec!["analyze".to_string(), "analysis".to_string(), "examine".to_string()],
            patterns: vec![r"analyze.*data".to_string(), r"perform.*analysis".to_string()],
            weight: 1.0,
        });
        
        TaskClassificationConfig {
            enabled: true,
            patterns,
            confidence_threshold: 0.5,
            default_task_type: TaskType::Custom("unknown".to_string()),
        }
    }
    
    fn create_test_context() -> RoutingContext {
        RoutingContext::new(
            AgentId::new(),
            TaskType::Custom("unknown".to_string()),
            "test prompt".to_string(),
        )
    }

    #[test]
    fn test_code_generation_classification() {
        let config = create_test_config();
        let classifier = TaskClassifier::new(config).unwrap();
        let context = create_test_context();
        
        let result = classifier.classify_task("Please write code to implement a sorting function", &context).unwrap();
        
        assert_eq!(result.task_type, TaskType::CodeGeneration);
        assert!(result.confidence > 0.5);
        assert!(!result.keyword_matches.is_empty());
    }
    
    #[test]
    fn test_analysis_classification() {
        let config = create_test_config();
        let classifier = TaskClassifier::new(config).unwrap();
        let context = create_test_context();
        
        let result = classifier.classify_task("Please analyze the data trends", &context).unwrap();
        
        assert_eq!(result.task_type, TaskType::Analysis);
        assert!(result.confidence > 0.5);
    }
    
    #[test]
    fn test_no_match_fallback() {
        let config = create_test_config();
        let classifier = TaskClassifier::new(config).unwrap();
        let context = create_test_context();
        
        let result = classifier.classify_task("Hello world", &context).unwrap();
        
        assert_eq!(result.task_type, TaskType::Custom("unknown".to_string()));
        assert_eq!(result.confidence, 0.0);
    }
    
    #[test]
    fn test_context_adjustments() {
        let config = create_test_config();
        let classifier = TaskClassifier::new(config).unwrap();
        let mut context = create_test_context();
        context.expected_output_type = OutputType::Code;
        context.agent_capabilities = vec!["code_generation".to_string()];
        
        let result = classifier.classify_task("Please write some code", &context).unwrap();
        
        assert_eq!(result.task_type, TaskType::CodeGeneration);
        // Should have higher confidence due to context adjustments
        assert!(result.confidence > 0.5);
    }
}