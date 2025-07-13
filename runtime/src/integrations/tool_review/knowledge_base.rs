//! Security Knowledge Base
//!
//! A comprehensive knowledge base containing vulnerability patterns, malicious code signatures,
//! and security rules for analyzing MCP tools.

use super::types::{SecurityCategory, SecuritySeverity, SecurityFinding};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use regex::Regex;

/// Security vulnerability pattern
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnerabilityPattern {
    /// Unique identifier for the pattern
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Description of the vulnerability
    pub description: String,
    /// Security category
    pub category: SecurityCategory,
    /// Severity level
    pub severity: SecuritySeverity,
    /// Pattern matching rules
    pub rules: Vec<PatternRule>,
    /// CVE references if applicable
    pub cve_references: Vec<String>,
    /// Remediation suggestions
    pub remediation: Option<String>,
    /// False positive indicators
    pub false_positive_indicators: Vec<String>,
}

/// Pattern matching rule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternRule {
    /// Rule type
    pub rule_type: RuleType,
    /// Pattern to match
    pub pattern: String,
    /// Fields to apply the pattern to
    pub target_fields: Vec<String>,
    /// Confidence weight (0.0 to 1.0)
    pub confidence_weight: f32,
    /// Whether this rule is required for a match
    pub required: bool,
}

/// Types of pattern matching rules
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RuleType {
    /// Regular expression match
    Regex,
    /// Exact string match
    Exact,
    /// Case-insensitive substring match
    Contains,
    /// JSON path match
    JsonPath,
    /// Schema structure validation
    SchemaStructure,
}

/// Malicious code signature
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaliciousSignature {
    /// Unique identifier
    pub id: String,
    /// Signature name
    pub name: String,
    /// Description of the malicious behavior
    pub description: String,
    /// Pattern to detect
    pub pattern: String,
    /// Confidence score (0.0 to 1.0)
    pub confidence: f32,
    /// Threat type
    pub threat_type: ThreatType,
    /// Indicators of compromise
    pub iocs: Vec<String>,
}

/// Types of threats
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ThreatType {
    /// Data exfiltration
    DataExfiltration,
    /// Remote code execution
    RemoteCodeExecution,
    /// Privilege escalation
    PrivilegeEscalation,
    /// Backdoor installation
    Backdoor,
    /// Information disclosure
    InformationDisclosure,
    /// Denial of service
    DenialOfService,
    /// Persistence mechanism
    Persistence,
}

/// Result of vulnerability pattern matching
#[derive(Debug, Clone)]
pub struct VulnerabilityMatch {
    pub pattern_id: String,
    pub confidence: f32,
    pub matched_rules: Vec<String>,
}

/// Result of malicious signature matching
#[derive(Debug, Clone)]
pub struct SignatureMatch {
    pub signature_id: String,
    pub confidence: f32,
    pub matched_content: String,
}

/// Security knowledge base
pub struct SecurityKnowledgeBase {
    /// Vulnerability patterns indexed by category
    vulnerability_patterns: HashMap<SecurityCategory, Vec<VulnerabilityPattern>>,
    /// Malicious code signatures
    malicious_signatures: Vec<MaliciousSignature>,
    /// Compiled regex patterns for performance
    compiled_patterns: HashMap<String, Regex>,
    /// Pattern statistics
    pattern_stats: PatternStats,
}

/// Statistics about pattern usage and effectiveness
#[derive(Debug, Clone, Default)]
pub struct PatternStats {
    /// Total patterns loaded
    pub total_patterns: usize,
    /// Patterns by severity
    pub patterns_by_severity: HashMap<SecuritySeverity, usize>,
    /// Pattern match rates
    pub match_rates: HashMap<String, f32>,
    /// False positive rates
    pub false_positive_rates: HashMap<String, f32>,
}

impl SecurityKnowledgeBase {
    /// Create a new security knowledge base
    pub fn new() -> Self {
        let mut kb = Self {
            vulnerability_patterns: HashMap::new(),
            malicious_signatures: Vec::new(),
            compiled_patterns: HashMap::new(),
            pattern_stats: PatternStats::default(),
        };

        // Load default patterns
        kb.load_default_patterns();
        kb
    }

    /// Load patterns from a JSON file
    pub fn load_from_file(&mut self, file_path: &str) -> Result<(), SecurityKnowledgeError> {
        let content = std::fs::read_to_string(file_path)
            .map_err(|e| SecurityKnowledgeError::FileError(e.to_string()))?;
        
        let patterns: Vec<VulnerabilityPattern> = serde_json::from_str(&content)
            .map_err(|e| SecurityKnowledgeError::ParseError(e.to_string()))?;

        for pattern in patterns {
            self.add_vulnerability_pattern(pattern)?;
        }

        Ok(())
    }

    /// Add a vulnerability pattern to the knowledge base
    pub fn add_vulnerability_pattern(&mut self, pattern: VulnerabilityPattern) -> Result<(), SecurityKnowledgeError> {
        // Compile regex patterns for performance
        for rule in &pattern.rules {
            if rule.rule_type == RuleType::Regex {
                let regex = Regex::new(&rule.pattern)
                    .map_err(|e| SecurityKnowledgeError::InvalidPattern(e.to_string()))?;
                self.compiled_patterns.insert(rule.pattern.clone(), regex);
            }
        }

        // Add to category index
        self.vulnerability_patterns
            .entry(pattern.category.clone())
            .or_insert_with(Vec::new)
            .push(pattern);

        self.update_stats();
        Ok(())
    }

    /// Add a malicious signature
    pub fn add_malicious_signature(&mut self, signature: MaliciousSignature) -> Result<(), SecurityKnowledgeError> {
        // Compile regex if needed
        if let Ok(regex) = Regex::new(&signature.pattern) {
            self.compiled_patterns.insert(signature.pattern.clone(), regex);
        }

        self.malicious_signatures.push(signature);
        Ok(())
    }

    /// Analyze a tool schema for vulnerabilities
    pub fn analyze_schema(&self, schema: &serde_json::Value) -> Vec<SecurityFinding> {
        let mut findings = Vec::new();

        // Check against vulnerability patterns
        for patterns in self.vulnerability_patterns.values() {
            for pattern in patterns {
                if let Some(finding) = self.check_pattern(schema, pattern) {
                    findings.push(finding);
                }
            }
        }

        // Check against malicious signatures
        for signature in &self.malicious_signatures {
            if let Some(finding) = self.check_signature(schema, signature) {
                findings.push(finding);
            }
        }

        findings
    }

    /// Check a specific vulnerability pattern against schema
    fn check_pattern(&self, schema: &serde_json::Value, pattern: &VulnerabilityPattern) -> Option<SecurityFinding> {
        let mut total_confidence = 0.0;
        let mut _matched_rules = 0;
        let mut required_rules_matched = 0;
        let required_rules_count = pattern.rules.iter().filter(|r| r.required).count();

        for rule in &pattern.rules {
            if self.check_rule(schema, rule) {
                total_confidence += rule.confidence_weight;
                _matched_rules += 1;
                if rule.required {
                    required_rules_matched += 1;
                }
            }
        }

        // Check if all required rules matched
        if required_rules_count > 0 && required_rules_matched < required_rules_count {
            return None;
        }

        // Calculate final confidence
        let confidence = if pattern.rules.is_empty() {
            0.0
        } else {
            total_confidence / pattern.rules.len() as f32
        };

        // Only return finding if confidence is above threshold
        if confidence >= 0.3 {
            Some(SecurityFinding {
                finding_id: format!("{}_{}", pattern.id, uuid::Uuid::new_v4()),
                severity: pattern.severity.clone(),
                category: pattern.category.clone(),
                title: pattern.name.clone(),
                description: pattern.description.clone(),
                location: Some("schema".to_string()),
                confidence,
                remediation_suggestion: pattern.remediation.clone(),
                cve_references: pattern.cve_references.clone(),
            })
        } else {
            None
        }
    }

    /// Check a rule against the schema
    fn check_rule(&self, schema: &serde_json::Value, rule: &PatternRule) -> bool {
        let schema_text = if rule.target_fields.is_empty() {
            // Search entire schema
            serde_json::to_string(schema).unwrap_or_default()
        } else {
            // Search specific fields
            rule.target_fields
                .iter()
                .filter_map(|field| self.extract_field_value(schema, field))
                .collect::<Vec<_>>()
                .join(" ")
        };

        match rule.rule_type {
            RuleType::Regex => {
                if let Some(regex) = self.compiled_patterns.get(&rule.pattern) {
                    regex.is_match(&schema_text)
                } else {
                    false
                }
            }
            RuleType::Exact => schema_text == rule.pattern,
            RuleType::Contains => schema_text.to_lowercase().contains(&rule.pattern.to_lowercase()),
            RuleType::JsonPath => self.check_json_path(schema, &rule.pattern),
            RuleType::SchemaStructure => self.check_schema_structure(schema, &rule.pattern),
        }
    }

    /// Extract field value from JSON schema
    fn extract_field_value(&self, schema: &serde_json::Value, field_path: &str) -> Option<String> {
        let parts: Vec<&str> = field_path.split('.').collect();
        let mut current = schema;

        for part in parts {
            match current {
                serde_json::Value::Object(obj) => {
                    current = obj.get(part)?;
                }
                serde_json::Value::Array(arr) => {
                    if let Ok(index) = part.parse::<usize>() {
                        current = arr.get(index)?;
                    } else {
                        return None;
                    }
                }
                _ => return None,
            }
        }

        Some(current.to_string())
    }

    /// Check JSON path pattern
    fn check_json_path(&self, _schema: &serde_json::Value, _path: &str) -> bool {
        // Simplified JSON path checking
        // In a real implementation, you'd use a proper JSON path library
        false
    }

    /// Check schema structure pattern
    fn check_schema_structure(&self, schema: &serde_json::Value, pattern: &str) -> bool {
        // Check for suspicious schema structures
        match pattern {
            "missing_input_validation" => self.check_missing_input_validation(schema),
            "overly_permissive_params" => self.check_overly_permissive_params(schema),
            "suspicious_properties" => self.check_suspicious_properties(schema),
            _ => false,
        }
    }

    /// Check for missing input validation
    fn check_missing_input_validation(&self, schema: &serde_json::Value) -> bool {
        if let Some(properties) = schema.get("properties") {
            if let serde_json::Value::Object(props) = properties {
                for (_, prop) in props {
                    if let serde_json::Value::Object(prop_obj) = prop {
                        // Check if string properties have no format, pattern, or enum constraints
                        if prop_obj.get("type") == Some(&serde_json::Value::String("string".to_string())) {
                            if !prop_obj.contains_key("format") 
                                && !prop_obj.contains_key("pattern") 
                                && !prop_obj.contains_key("enum") {
                                return true;
                            }
                        }
                    }
                }
            }
        }
        false
    }

    /// Check for overly permissive parameters
    fn check_overly_permissive_params(&self, schema: &serde_json::Value) -> bool {
        // Check for additionalProperties: true or missing required fields
        if let Some(additional) = schema.get("additionalProperties") {
            if additional == &serde_json::Value::Bool(true) {
                return true;
            }
        }

        // Check if required array is missing or empty
        if let Some(required) = schema.get("required") {
            if let serde_json::Value::Array(req_array) = required {
                if req_array.is_empty() {
                    return true;
                }
            }
        } else {
            return true; // No required fields at all
        }

        false
    }

    /// Check for suspicious property names
    fn check_suspicious_properties(&self, schema: &serde_json::Value) -> bool {
        let suspicious_names = [
            "eval", "exec", "system", "shell", "cmd", "command",
            "script", "code", "function", "callback", "handler",
            "file", "path", "dir", "directory", "url", "uri",
            "password", "secret", "key", "token", "auth",
        ];

        if let Some(properties) = schema.get("properties") {
            if let serde_json::Value::Object(props) = properties {
                for prop_name in props.keys() {
                    let prop_lower = prop_name.to_lowercase();
                    for suspicious in &suspicious_names {
                        if prop_lower.contains(suspicious) {
                            return true;
                        }
                    }
                }
            }
        }

        false
    }

    /// Check malicious signature
    fn check_signature(&self, schema: &serde_json::Value, signature: &MaliciousSignature) -> Option<SecurityFinding> {
        let schema_text = serde_json::to_string(schema).unwrap_or_default();
        
        if let Some(regex) = self.compiled_patterns.get(&signature.pattern) {
            if regex.is_match(&schema_text) {
                return Some(SecurityFinding {
                    finding_id: format!("sig_{}_{}", signature.id, uuid::Uuid::new_v4()),
                    severity: SecuritySeverity::Critical,
                    category: SecurityCategory::MaliciousCode,
                    title: format!("Malicious Signature: {}", signature.name),
                    description: signature.description.clone(),
                    location: Some("schema".to_string()),
                    confidence: signature.confidence,
                    remediation_suggestion: Some("Remove malicious code patterns".to_string()),
                    cve_references: Vec::new(),
                });
            }
        }

        None
    }

    /// Update pattern statistics
    fn update_stats(&mut self) {
        self.pattern_stats.total_patterns = self.vulnerability_patterns
            .values()
            .map(|patterns| patterns.len())
            .sum();

        self.pattern_stats.patterns_by_severity.clear();
        for patterns in self.vulnerability_patterns.values() {
            for pattern in patterns {
                *self.pattern_stats.patterns_by_severity
                    .entry(pattern.severity.clone())
                    .or_insert(0) += 1;
            }
        }
    }

    /// Get pattern statistics
    pub fn get_stats(&self) -> &PatternStats {
        &self.pattern_stats
    }

    /// Check vulnerability patterns against content
    pub fn check_vulnerability_patterns(&self, content: &str) -> Vec<VulnerabilityMatch> {
        let mut matches = Vec::new();

        for patterns in self.vulnerability_patterns.values() {
            for pattern in patterns {
                if let Some(pattern_match) = self.check_vulnerability_pattern(content, pattern) {
                    matches.push(pattern_match);
                }
            }
        }

        matches
    }

    /// Check malicious signatures against content
    pub fn check_malicious_signatures(&self, content: &str) -> Vec<SignatureMatch> {
        let mut matches = Vec::new();

        for signature in &self.malicious_signatures {
            if let Some(signature_match) = self.check_malicious_signature(content, signature) {
                matches.push(signature_match);
            }
        }

        matches
    }

    /// Get vulnerability patterns
    pub fn get_vulnerability_patterns(&self) -> &HashMap<SecurityCategory, Vec<VulnerabilityPattern>> {
        &self.vulnerability_patterns
    }

    /// Check a single vulnerability pattern
    fn check_vulnerability_pattern(&self, content: &str, pattern: &VulnerabilityPattern) -> Option<VulnerabilityMatch> {
        let mut total_confidence = 0.0;
        let mut matched_rules = Vec::new();
        let mut required_rules_matched = 0;
        let required_rules_count = pattern.rules.iter().filter(|r| r.required).count();

        for rule in &pattern.rules {
            if self.check_rule_against_content(content, rule) {
                total_confidence += rule.confidence_weight;
                matched_rules.push(rule.pattern.clone());
                if rule.required {
                    required_rules_matched += 1;
                }
            }
        }

        // Check if all required rules matched
        if required_rules_count > 0 && required_rules_matched < required_rules_count {
            return None;
        }

        // Calculate final confidence
        let confidence = if pattern.rules.is_empty() {
            0.0
        } else {
            total_confidence / pattern.rules.len() as f32
        };

        // Only return match if confidence is above threshold
        if confidence >= 0.3 {
            Some(VulnerabilityMatch {
                pattern_id: pattern.id.clone(),
                confidence,
                matched_rules,
            })
        } else {
            None
        }
    }

    /// Check a single malicious signature
    fn check_malicious_signature(&self, content: &str, signature: &MaliciousSignature) -> Option<SignatureMatch> {
        if let Some(regex) = self.compiled_patterns.get(&signature.pattern) {
            if let Some(matched) = regex.find(content) {
                return Some(SignatureMatch {
                    signature_id: signature.id.clone(),
                    confidence: signature.confidence,
                    matched_content: matched.as_str().to_string(),
                });
            }
        }

        None
    }

    /// Check a rule against content string
    fn check_rule_against_content(&self, content: &str, rule: &PatternRule) -> bool {
        match rule.rule_type {
            RuleType::Regex => {
                if let Some(regex) = self.compiled_patterns.get(&rule.pattern) {
                    regex.is_match(content)
                } else {
                    false
                }
            }
            RuleType::Exact => content == rule.pattern,
            RuleType::Contains => content.to_lowercase().contains(&rule.pattern.to_lowercase()),
            RuleType::JsonPath => false, // Not implemented for string content
            RuleType::SchemaStructure => false, // Not applicable for string content
        }
    }

    /// Load default vulnerability patterns
    fn load_default_patterns(&mut self) {
        let default_patterns = vec![
            // Schema injection patterns
            VulnerabilityPattern {
                id: "schema_injection_01".to_string(),
                name: "Unvalidated String Parameters".to_string(),
                description: "String parameters without validation constraints".to_string(),
                category: SecurityCategory::UnvalidatedInput,
                severity: SecuritySeverity::Medium,
                rules: vec![
                    PatternRule {
                        rule_type: RuleType::SchemaStructure,
                        pattern: "missing_input_validation".to_string(),
                        target_fields: vec!["properties".to_string()],
                        confidence_weight: 0.7,
                        required: true,
                    }
                ],
                cve_references: Vec::new(),
                remediation: Some("Add format, pattern, or enum constraints to string parameters".to_string()),
                false_positive_indicators: vec!["description".to_string(), "title".to_string()],
            },

            // Privilege escalation patterns
            VulnerabilityPattern {
                id: "privesc_01".to_string(),
                name: "System Command Execution".to_string(),
                description: "Parameters that could enable system command execution".to_string(),
                category: SecurityCategory::PrivilegeEscalation,
                severity: SecuritySeverity::High,
                rules: vec![
                    PatternRule {
                        rule_type: RuleType::Contains,
                        pattern: "command".to_string(),
                        target_fields: vec!["properties".to_string()],
                        confidence_weight: 0.6,
                        required: false,
                    },
                    PatternRule {
                        rule_type: RuleType::Contains,
                        pattern: "exec".to_string(),
                        target_fields: vec!["properties".to_string()],
                        confidence_weight: 0.8,
                        required: false,
                    },
                ],
                cve_references: Vec::new(),
                remediation: Some("Avoid parameters that allow arbitrary command execution".to_string()),
                false_positive_indicators: Vec::new(),
            },

            // Data exfiltration patterns
            VulnerabilityPattern {
                id: "data_exfil_01".to_string(),
                name: "File Path Parameters".to_string(),
                description: "Parameters that could enable file system access".to_string(),
                category: SecurityCategory::DataExfiltration,
                severity: SecuritySeverity::Medium,
                rules: vec![
                    PatternRule {
                        rule_type: RuleType::Regex,
                        pattern: r"(file|path|dir|directory)".to_string(),
                        target_fields: vec!["properties".to_string()],
                        confidence_weight: 0.5,
                        required: true,
                    }
                ],
                cve_references: Vec::new(),
                remediation: Some("Validate file paths and restrict to safe directories".to_string()),
                false_positive_indicators: Vec::new(),
            },

            // Overly permissive schema
            VulnerabilityPattern {
                id: "permissive_01".to_string(),
                name: "Overly Permissive Schema".to_string(),
                description: "Schema allows additional properties without validation".to_string(),
                category: SecurityCategory::InsecureDefaults,
                severity: SecuritySeverity::Low,
                rules: vec![
                    PatternRule {
                        rule_type: RuleType::SchemaStructure,
                        pattern: "overly_permissive_params".to_string(),
                        target_fields: Vec::new(),
                        confidence_weight: 0.8,
                        required: true,
                    }
                ],
                cve_references: Vec::new(),
                remediation: Some("Set additionalProperties to false and define required fields".to_string()),
                false_positive_indicators: Vec::new(),
            },
        ];

        for pattern in default_patterns {
            let _ = self.add_vulnerability_pattern(pattern);
        }

        // Add default malicious signatures
        let default_signatures = vec![
            MaliciousSignature {
                id: "malicious_01".to_string(),
                name: "Base64 Encoded Payload".to_string(),
                description: "Suspicious base64 encoded content that may contain malicious code".to_string(),
                pattern: r"[A-Za-z0-9+/]{100,}={0,2}".to_string(),
                confidence: 0.6,
                threat_type: ThreatType::RemoteCodeExecution,
                iocs: Vec::new(),
            },
            MaliciousSignature {
                id: "malicious_02".to_string(),
                name: "Suspicious URL Pattern".to_string(),
                description: "URLs with suspicious characteristics".to_string(),
                pattern: r"https?://[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}".to_string(),
                confidence: 0.4,
                threat_type: ThreatType::DataExfiltration,
                iocs: Vec::new(),
            },
        ];

        for signature in default_signatures {
            let _ = self.add_malicious_signature(signature);
        }
    }
}

/// Errors that can occur with the security knowledge base
#[derive(Debug, thiserror::Error)]
pub enum SecurityKnowledgeError {
    #[error("File error: {0}")]
    FileError(String),

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Invalid pattern: {0}")]
    InvalidPattern(String),

    #[error("Pattern not found: {0}")]
    PatternNotFound(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_knowledge_base_creation() {
        let kb = SecurityKnowledgeBase::new();
        assert!(kb.get_stats().total_patterns > 0);
    }

    #[test]
    fn test_pattern_matching() {
        let kb = SecurityKnowledgeBase::new();
        
        // Test schema with unvalidated string parameter
        let test_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string"
                }
            }
        });

        let findings = kb.analyze_schema(&test_schema);
        assert!(!findings.is_empty());
        
        // Should detect unvalidated input
        assert!(findings.iter().any(|f| f.category == SecurityCategory::UnvalidatedInput));
    }

    #[test]
    fn test_malicious_signature_detection() {
        let kb = SecurityKnowledgeBase::new();
        
        // Test schema with suspicious IP address
        let test_schema = serde_json::json!({
            "properties": {
                "url": {
                    "default": "http://192.168.1.1/malicious"
                }
            }
        });

        let findings = kb.analyze_schema(&test_schema);
        assert!(findings.iter().any(|f| f.category == SecurityCategory::MaliciousCode));
    }
}