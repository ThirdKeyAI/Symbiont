//! AI Security Analyzer
//! 
//! This module implements an AI-powered security analyzer that uses the RAG engine
//! to analyze MCP tool schemas for potential security vulnerabilities and malicious patterns.

use super::types::*;
use super::knowledge_base::SecurityKnowledgeBase;
use crate::integrations::mcp::McpTool;
use crate::rag::{RAGEngine, RAGRequest, QueryPreferences, QueryConstraints, ResponseLength, ResponseFormat, AccessLevel};
use crate::types::AgentId;
use async_trait::async_trait;
use std::sync::Arc;
use std::time::{Duration, SystemTime};


/// Trait for security analysis of MCP tools
#[async_trait]
pub trait SecurityAnalyzer: Send + Sync {
    /// Analyze a tool for security vulnerabilities
    async fn analyze_tool(&self, tool: &McpTool) -> ToolReviewResult<SecurityAnalysis>;
    
    /// Get analyzer configuration
    fn get_config(&self) -> &SecurityAnalyzerConfig;
    
    /// Update analyzer configuration
    async fn update_config(&mut self, config: SecurityAnalyzerConfig) -> ToolReviewResult<()>;
}

/// Configuration for the security analyzer
#[derive(Debug, Clone)]
pub struct SecurityAnalyzerConfig {
    /// Maximum analysis time in seconds
    pub max_analysis_time_seconds: u64,
    /// Confidence threshold for findings (0.0 to 1.0)
    pub confidence_threshold: f32,
    /// Whether to include low-severity findings
    pub include_low_severity: bool,
    /// Security knowledge sources to query
    pub knowledge_sources: Vec<String>,
    /// Maximum number of RAG queries per analysis
    pub max_rag_queries: u32,
}

impl Default for SecurityAnalyzerConfig {
    fn default() -> Self {
        Self {
            max_analysis_time_seconds: 120,
            confidence_threshold: 0.6,
            include_low_severity: false,
            knowledge_sources: vec![
                "vulnerability_patterns".to_string(),
                "malicious_code_signatures".to_string(),
                "security_best_practices".to_string(),
            ],
            max_rag_queries: 10,
        }
    }
}

/// AI-powered security analyzer implementation
pub struct AISecurityAnalyzer {
    rag_engine: Arc<dyn RAGEngine>,
    config: SecurityAnalyzerConfig,
    agent_id: AgentId,
    knowledge_base: SecurityKnowledgeBase,
}

impl AISecurityAnalyzer {
    /// Create a new AI security analyzer
    pub fn new(rag_engine: Arc<dyn RAGEngine>, agent_id: AgentId) -> Self {
        Self {
            rag_engine,
            config: SecurityAnalyzerConfig::default(),
            agent_id,
            knowledge_base: SecurityKnowledgeBase::new(),
        }
    }
    
    /// Create analyzer with custom configuration
    pub fn with_config(
        rag_engine: Arc<dyn RAGEngine>,
        agent_id: AgentId,
        config: SecurityAnalyzerConfig,
    ) -> Self {
        Self {
            rag_engine,
            config,
            agent_id,
            knowledge_base: SecurityKnowledgeBase::new(),
        }
    }
    
    /// Extract potential security patterns from tool schema using knowledge base
    fn extract_security_patterns(&self, tool: &McpTool) -> Vec<String> {
        let mut patterns = Vec::new();
        
        // Convert tool to JSON for pattern matching
        let tool_json = serde_json::to_string(tool).unwrap_or_default();
        
        // Use knowledge base for pattern matching
        let vulnerability_matches = self.knowledge_base.check_vulnerability_patterns(&tool_json);
        for pattern_match in vulnerability_matches {
            if pattern_match.confidence >= self.config.confidence_threshold {
                patterns.push(pattern_match.pattern_id);
            }
        }
        
        // Check for malicious signatures
        let malicious_matches = self.knowledge_base.check_malicious_signatures(&tool_json);
        for signature_match in malicious_matches {
            if signature_match.confidence >= self.config.confidence_threshold {
                patterns.push(format!("malicious_{}", signature_match.signature_id));
            }
        }
        
        // Additional manual pattern extraction as fallback
        let tool_name = tool.name.to_lowercase();
        if tool_name.contains("exec") || tool_name.contains("eval") || tool_name.contains("shell") {
            patterns.push("execution_related_name".to_string());
        }
        
        // Analyze schema for suspicious parameter patterns
        if let Some(properties) = tool.schema.get("properties").and_then(|p| p.as_object()) {
            for (param_name, param_def) in properties {
                let param_name_lower = param_name.to_lowercase();
                
                // Check for command injection patterns
                if param_name_lower.contains("command") || param_name_lower.contains("cmd") {
                    patterns.push("command_parameter".to_string());
                }
                
                // Check for file system access patterns
                if param_name_lower.contains("path") || param_name_lower.contains("file") {
                    patterns.push("filesystem_access".to_string());
                }
                
                // Check for network access patterns
                if param_name_lower.contains("url") || param_name_lower.contains("host") {
                    patterns.push("network_access".to_string());
                }
                
                // Check for unvalidated input patterns
                if let Some(param_type) = param_def.get("type").and_then(|t| t.as_str()) {
                    if param_type == "string" && !param_def.get("pattern").is_some() && !param_def.get("enum").is_some() {
                        patterns.push("unvalidated_string_input".to_string());
                    }
                }
            }
        }
        
        // Remove duplicates
        patterns.sort();
        patterns.dedup();
        patterns
    }
    
    /// Generate security analysis queries for RAG engine
    fn generate_security_queries(&self, tool: &McpTool, patterns: &[String]) -> Vec<String> {
        let mut queries = Vec::new();
        
        // Base query about the tool
        queries.push(format!(
            "Security vulnerabilities in MCP tool named '{}' with description: {}",
            tool.name, tool.description
        ));
        
        // Pattern-specific queries
        for pattern in patterns {
            match pattern.as_str() {
                "execution_related_name" => {
                    queries.push("Security risks of tools with execution-related names like exec, eval, shell".to_string());
                }
                "command_parameter" => {
                    queries.push("Command injection vulnerabilities in tools with command parameters".to_string());
                }
                "filesystem_access" => {
                    queries.push("Path traversal and file system security vulnerabilities in tools".to_string());
                }
                "network_access" => {
                    queries.push("Network-based security vulnerabilities and SSRF attacks in tools".to_string());
                }
                "unvalidated_string_input" => {
                    queries.push("Security risks of unvalidated string inputs in tool parameters".to_string());
                }
                _ => {}
            }
        }
        
        // Limit queries to configured maximum
        queries.truncate(self.config.max_rag_queries as usize);
        queries
    }
    
    /// Analyze RAG responses for security findings
    fn analyze_rag_responses(&self, responses: &[String], patterns: &[String]) -> Vec<SecurityFinding> {
        let mut findings = Vec::new();
        let mut finding_counter = 0;
        
        for (query_idx, response) in responses.iter().enumerate() {
            // Simple pattern matching for demonstration
            // In a real implementation, this would use more sophisticated NLP analysis
            
            if response.to_lowercase().contains("injection") {
                finding_counter += 1;
                findings.push(SecurityFinding {
                    finding_id: format!("INJECTION_{}", finding_counter),
                    severity: SecuritySeverity::High,
                    category: SecurityCategory::SchemaInjection,
                    title: "Potential Injection Vulnerability".to_string(),
                    description: "Tool may be vulnerable to injection attacks based on schema analysis".to_string(),
                    location: Some(format!("Query {}: Pattern analysis", query_idx + 1)),
                    confidence: 0.8,
                    remediation_suggestion: Some("Implement input validation and sanitization".to_string()),
                    cve_references: vec![],
                });
            }
            
            if response.to_lowercase().contains("privilege") || response.to_lowercase().contains("escalation") {
                finding_counter += 1;
                findings.push(SecurityFinding {
                    finding_id: format!("PRIVESC_{}", finding_counter),
                    severity: SecuritySeverity::Critical,
                    category: SecurityCategory::PrivilegeEscalation,
                    title: "Potential Privilege Escalation".to_string(),
                    description: "Tool may allow privilege escalation based on security analysis".to_string(),
                    location: Some(format!("Query {}: Security pattern analysis", query_idx + 1)),
                    confidence: 0.9,
                    remediation_suggestion: Some("Implement proper access controls and privilege separation".to_string()),
                    cve_references: vec![],
                });
            }
            
            if response.to_lowercase().contains("malicious") || response.to_lowercase().contains("backdoor") {
                finding_counter += 1;
                findings.push(SecurityFinding {
                    finding_id: format!("MALICIOUS_{}", finding_counter),
                    severity: SecuritySeverity::Critical,
                    category: SecurityCategory::MaliciousCode,
                    title: "Potential Malicious Code".to_string(),
                    description: "Tool may contain malicious code patterns".to_string(),
                    location: Some(format!("Query {}: Malware analysis", query_idx + 1)),
                    confidence: 0.85,
                    remediation_suggestion: Some("Perform thorough code review and sandboxed testing".to_string()),
                    cve_references: vec![],
                });
            }
        }
        
        // Add pattern-based findings
        for pattern in patterns {
            match pattern.as_str() {
                "unvalidated_string_input" => {
                    finding_counter += 1;
                    findings.push(SecurityFinding {
                        finding_id: format!("UNVALIDATED_{}", finding_counter),
                        severity: SecuritySeverity::Medium,
                        category: SecurityCategory::UnvalidatedInput,
                        title: "Unvalidated Input Parameters".to_string(),
                        description: "Tool accepts string parameters without validation constraints".to_string(),
                        location: Some("Schema parameter analysis".to_string()),
                        confidence: 0.7,
                        remediation_suggestion: Some("Add input validation patterns or enums to string parameters".to_string()),
                        cve_references: vec![],
                    });
                }
                _ => {}
            }
        }
        
        // Filter by confidence threshold
        findings.retain(|f| f.confidence >= self.config.confidence_threshold);
        
        // Filter by severity if configured
        if !self.config.include_low_severity {
            findings.retain(|f| f.severity != SecuritySeverity::Low);
        }
        
        findings
    }
    
    /// Calculate overall risk score based on findings
    fn calculate_risk_score(&self, findings: &[SecurityFinding]) -> f32 {
        if findings.is_empty() {
            return 0.0;
        }
        
        let mut total_score = 0.0;
        let mut weight_sum = 0.0;
        
        for finding in findings {
            let severity_weight = match finding.severity {
                SecuritySeverity::Critical => 4.0,
                SecuritySeverity::High => 3.0,
                SecuritySeverity::Medium => 2.0,
                SecuritySeverity::Low => 1.0,
            };
            
            total_score += severity_weight * finding.confidence;
            weight_sum += severity_weight;
        }
        
        if weight_sum > 0.0 {
            (total_score / weight_sum).min(1.0)
        } else {
            0.0
        }
    }
    
    /// Generate analysis recommendations
    fn generate_recommendations(&self, findings: &[SecurityFinding]) -> Vec<String> {
        let mut recommendations = Vec::new();
        
        if findings.is_empty() {
            recommendations.push("No significant security issues detected. Tool appears safe for use.".to_string());
            return recommendations;
        }
        
        let critical_count = findings.iter().filter(|f| f.severity == SecuritySeverity::Critical).count();
        let high_count = findings.iter().filter(|f| f.severity == SecuritySeverity::High).count();
        
        if critical_count > 0 {
            recommendations.push(format!(
                "CRITICAL: {} critical security issues found. Tool should be rejected.",
                critical_count
            ));
        }
        
        if high_count > 0 {
            recommendations.push(format!(
                "HIGH RISK: {} high-severity issues found. Requires thorough review.",
                high_count
            ));
        }
        
        // Add specific remediation suggestions
        let unique_suggestions: std::collections::HashSet<_> = findings
            .iter()
            .filter_map(|f| f.remediation_suggestion.as_ref())
            .collect();
        
        for suggestion in unique_suggestions {
            recommendations.push(format!("Remediation: {}", suggestion));
        }
        
        recommendations
    }
}

#[async_trait]
impl SecurityAnalyzer for AISecurityAnalyzer {
    async fn analyze_tool(&self, tool: &McpTool) -> ToolReviewResult<SecurityAnalysis> {
        let start_time = SystemTime::now();
        let analysis_id = AnalysisId::new();
        
        // Extract security patterns from the tool
        let patterns = self.extract_security_patterns(tool);
        
        // Generate queries for RAG engine
        let queries = self.generate_security_queries(tool, &patterns);
        
        // Query RAG engine for security knowledge
        let mut rag_responses = Vec::new();
        let mut rag_queries_performed = 0;
        
        for query in queries {
            if rag_queries_performed >= self.config.max_rag_queries {
                break;
            }
            
            let rag_request = RAGRequest {
                agent_id: self.agent_id,
                query: query.clone(),
                preferences: QueryPreferences {
                    response_length: ResponseLength::Standard,
                    include_citations: false,
                    preferred_sources: self.config.knowledge_sources.clone(),
                    response_format: ResponseFormat::Text,
                    language: "en".to_string(),
                },
                constraints: QueryConstraints {
                    max_documents: 5,
                    time_limit: Duration::from_secs(30),
                    security_level: AccessLevel::Public,
                    allowed_sources: self.config.knowledge_sources.clone(),
                    excluded_sources: vec![],
                },
            };
            
            match self.rag_engine.process_query(rag_request).await {
                Ok(response) => {
                    rag_responses.push(response.response.content);
                    rag_queries_performed += 1;
                }
                Err(_) => {
                    // Continue with other queries if one fails
                    continue;
                }
            }
        }
        
        // Analyze responses for security findings
        let findings = self.analyze_rag_responses(&rag_responses, &patterns);
        
        // Calculate risk score
        let risk_score = self.calculate_risk_score(&findings);
        
        // Generate recommendations
        let recommendations = self.generate_recommendations(&findings);
        
        // Calculate confidence score
        let confidence_score = if findings.is_empty() {
            0.8 // High confidence when no issues found
        } else {
            findings.iter().map(|f| f.confidence).sum::<f32>() / findings.len() as f32
        };
        
        let processing_time = start_time.elapsed().unwrap_or(Duration::from_secs(0));
        
        Ok(SecurityAnalysis {
            analysis_id,
            tool_id: tool.name.clone(),
            analyzed_at: SystemTime::now(),
            analyzer_version: "ai-security-analyzer-v1.0".to_string(),
            risk_score,
            findings,
            recommendations,
            confidence_score,
            analysis_metadata: AnalysisMetadata {
                processing_time_ms: processing_time.as_millis() as u64,
                rag_queries_performed,
                knowledge_sources_consulted: self.config.knowledge_sources.clone(),
                patterns_matched: patterns,
                false_positive_likelihood: 0.1, // Estimated false positive rate
            },
        })
    }
    
    fn get_config(&self) -> &SecurityAnalyzerConfig {
        &self.config
    }
    
    async fn update_config(&mut self, config: SecurityAnalyzerConfig) -> ToolReviewResult<()> {
        self.config = config;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::mcp::{McpTool, ToolProvider, VerificationStatus};
    use crate::rag::StandardRAGEngine;
    use crate::context::manager::MockContextManager;
    use crate::types::AgentId;
    use std::collections::HashMap;
    
    fn create_test_tool(name: &str, has_command_param: bool) -> McpTool {
        let mut schema = serde_json::json!({
            "type": "object",
            "properties": {}
        });
        
        if has_command_param {
            schema["properties"]["command"] = serde_json::json!({
                "type": "string",
                "description": "Command to execute"
            });
        }
        
        McpTool {
            name: name.to_string(),
            description: "Test tool for security analysis".to_string(),
            schema,
            provider: ToolProvider {
                name: "test-provider".to_string(),
                public_key_url: "https://test.example.com/pubkey".to_string(),
            },
            verification_status: VerificationStatus::Pending,
            metadata: Some(HashMap::new()),
        }
    }
    
    #[tokio::test]
    async fn test_security_analyzer_creation() {
        let context_manager = Arc::new(MockContextManager::new());
        let rag_engine = Arc::new(StandardRAGEngine::new(context_manager));
        let agent_id = AgentId::new();
        
        let analyzer = AISecurityAnalyzer::new(rag_engine, agent_id);
        assert_eq!(analyzer.config.max_analysis_time_seconds, 120);
        assert_eq!(analyzer.config.confidence_threshold, 0.6);
        assert!(!analyzer.knowledge_base.get_vulnerability_patterns().is_empty());
    }
    
    #[tokio::test]
    async fn test_pattern_extraction() {
        let context_manager = Arc::new(MockContextManager::new());
        let rag_engine = Arc::new(StandardRAGEngine::new(context_manager));
        let agent_id = AgentId::new();
        
        let analyzer = AISecurityAnalyzer::new(rag_engine, agent_id);
        let tool = create_test_tool("exec_command", true);
        
        let patterns = analyzer.extract_security_patterns(&tool);
        assert!(patterns.contains(&"execution_related_name".to_string()));
        assert!(patterns.contains(&"command_parameter".to_string()));
    }
    
    #[tokio::test]
    async fn test_risk_score_calculation() {
        let context_manager = Arc::new(MockContextManager::new());
        let rag_engine = Arc::new(StandardRAGEngine::new(context_manager));
        let agent_id = AgentId::new();
        
        let analyzer = AISecurityAnalyzer::new(rag_engine, agent_id);
        
        let findings = vec![
            SecurityFinding {
                finding_id: "TEST_1".to_string(),
                severity: SecuritySeverity::Critical,
                category: SecurityCategory::MaliciousCode,
                title: "Test Finding".to_string(),
                description: "Test".to_string(),
                location: None,
                confidence: 0.9,
                remediation_suggestion: None,
                cve_references: vec![],
            }
        ];
        
        let risk_score = analyzer.calculate_risk_score(&findings);
        assert!(risk_score > 0.8); // Should be high for critical finding
    }
}