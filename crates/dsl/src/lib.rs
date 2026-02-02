//! Symbiont DSL Parser Library
//!
//! This library provides parsing capabilities for the Symbiont DSL using Tree-sitter.

use std::collections::HashMap;
use tree_sitter::{Language, Node, Parser, Tree};
use serde::{Deserialize, Serialize};

/// Sandbox tier enumeration representing different isolation levels
/// This mirrors the SandboxTier enum in the runtime crate to avoid circular dependencies
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SandboxTier {
    /// Docker container sandbox
    Docker,
    /// gVisor sandbox for enhanced security
    GVisor,
    /// Firecracker microVM sandbox
    Firecracker,
    /// E2B.dev cloud sandbox
    E2B,
}

impl std::fmt::Display for SandboxTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SandboxTier::Docker => write!(f, "docker"),
            SandboxTier::GVisor => write!(f, "gvisor"),
            SandboxTier::Firecracker => write!(f, "firecracker"),
            SandboxTier::E2B => write!(f, "e2b"),
        }
    }
}

// External function to get the language definition
extern "C" {
    fn tree_sitter_symbiont() -> Language;
}

/// Parse Symbiont DSL code and return the syntax tree
pub fn parse_dsl(source_code: &str) -> Result<Tree, Box<dyn std::error::Error>> {
    let language = unsafe { tree_sitter_symbiont() };
    let mut parser = Parser::new();
    parser.set_language(language)?;

    let tree = parser
        .parse(source_code, None)
        .ok_or("Failed to parse DSL code")?;

    Ok(tree)
}

/// Print the AST in a readable format
pub fn print_ast(node: Node, source: &str, depth: usize) {
    let indent = "  ".repeat(depth);
    let node_text = if node.child_count() == 0 {
        let start = node.start_byte();
        let end = node.end_byte();
        format!(" \"{}\"", &source[start..end].replace('\n', "\\n"))
    } else {
        String::new()
    };

    println!(
        "{}{}: {}{}",
        indent,
        node.kind(),
        node_text,
        if node.is_error() { " [ERROR]" } else { "" }
    );

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            print_ast(child, source, depth + 1);
        }
    }
}

/// WithBlock attribute structure
#[derive(Debug, Clone, PartialEq)]
pub struct WithAttribute {
    pub name: String,
    pub value: String,
}

/// WithBlock structure containing sandbox configuration
#[derive(Debug, Clone, PartialEq)]
pub struct WithBlock {
    pub attributes: Vec<WithAttribute>,
    pub sandbox_tier: Option<SandboxTier>,
    pub timeout: Option<u64>,
}

impl WithBlock {
    pub fn new() -> Self {
        Self {
            attributes: Vec::new(),
            sandbox_tier: None,
            timeout: None,
        }
    }

    /// Parse sandbox tier from string value, validating against known tiers
    pub fn parse_sandbox_tier(value: &str) -> Result<SandboxTier, String> {
        // Remove quotes if present
        let cleaned_value = value.trim_matches('"');
        match cleaned_value.to_lowercase().as_str() {
            "docker" => Ok(SandboxTier::Docker),
            "gvisor" => Ok(SandboxTier::GVisor),
            "firecracker" => Ok(SandboxTier::Firecracker),
            "e2b" => Ok(SandboxTier::E2B),
            _ => Err(format!("Invalid sandbox tier: {}. Valid options are: docker, gvisor, firecracker, e2b", value)),
        }
    }
}

impl Default for WithBlock {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract metadata from parsed AST
pub fn extract_metadata(tree: &Tree, source: &str) -> HashMap<String, String> {
    let mut metadata = HashMap::new();
    let root_node = tree.root_node();

    // Walk through the tree to find metadata blocks
    let _cursor = root_node.walk();

    fn traverse_for_metadata(node: Node, source: &str, metadata: &mut HashMap<String, String>) {
        if node.kind() == "metadata_block" {
            // Extract metadata key-value pairs
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if child.kind() == "metadata_pair" {
                        if let (Some(key_node), Some(value_node)) = (child.child(0), child.child(2))
                        {
                            let key =
                                source[key_node.start_byte()..key_node.end_byte()].to_string();
                            let value =
                                source[value_node.start_byte()..value_node.end_byte()].to_string();
                            metadata.insert(key, value);
                        }
                    }
                }
            }
        }

        // Recursively traverse children
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                traverse_for_metadata(child, source, metadata);
            }
        }
    }

    traverse_for_metadata(root_node, source, &mut metadata);
    metadata
}

/// Extract with blocks from parsed AST
pub fn extract_with_blocks(tree: &Tree, source: &str) -> Result<Vec<WithBlock>, String> {
    let mut with_blocks = Vec::new();
    let root_node = tree.root_node();

    fn traverse_for_with_blocks(node: Node, source: &str, with_blocks: &mut Vec<WithBlock>) -> Result<(), String> {
        if node.kind() == "with_block" {
            let mut with_block = WithBlock::new();
            
            // Extract with attributes
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if child.kind() == "with_attribute" {
                        if let (Some(name_node), Some(value_node)) = (child.child(0), child.child(2)) {
                            let name = source[name_node.start_byte()..name_node.end_byte()].to_string();
                            let value = source[value_node.start_byte()..value_node.end_byte()].to_string();
                            
                            let attribute = WithAttribute { name: name.clone(), value: value.clone() };
                            with_block.attributes.push(attribute);
                            
                            // Parse specific attributes
                            match name.as_str() {
                                "sandbox" => {
                                    match WithBlock::parse_sandbox_tier(&value) {
                                        Ok(tier) => with_block.sandbox_tier = Some(tier),
                                        Err(e) => return Err(e),
                                    }
                                }
                                "timeout" => {
                                    // Parse timeout (assuming it's in seconds)
                                    let timeout_str = value.trim_matches('"');
                                    if let Some(timeout_value) = timeout_str.strip_suffix(".seconds") {
                                        match timeout_value.parse::<u64>() {
                                            Ok(seconds) => with_block.timeout = Some(seconds),
                                            Err(_) => return Err(format!("Invalid timeout value: {}", value)),
                                        }
                                    } else {
                                        match timeout_str.parse::<u64>() {
                                            Ok(seconds) => with_block.timeout = Some(seconds),
                                            Err(_) => return Err(format!("Invalid timeout value: {}", value)),
                                        }
                                    }
                                }
                                _ => {} // Other attributes are stored but not specially parsed
                            }
                        }
                    }
                }
            }
            
            with_blocks.push(with_block);
        }

        // Recursively traverse children
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                traverse_for_with_blocks(child, source, with_blocks)?;
            }
        }
        
        Ok(())
    }

    traverse_for_with_blocks(root_node, source, &mut with_blocks)?;
    Ok(with_blocks)
}

/// Find and report errors in the AST
pub fn find_errors(node: Node, source: &str, depth: usize) {
    if node.kind() == "ERROR" {
        let start = node.start_position();
        let end = node.end_position();
        let text = &source[node.start_byte()..node.end_byte()];
        println!(
            "{}ERROR at {}:{}-{}:{}: '{}'",
            "  ".repeat(depth),
            start.row + 1,
            start.column + 1,
            end.row + 1,
            end.column + 1,
            text
        );
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            find_errors(child, source, depth + 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_parsing() {
        let simple_dsl = r#"
        agent TestAgent {
            capabilities: [test]
        }
        "#;

        let result = parse_dsl(simple_dsl);
        assert!(result.is_ok(), "Basic DSL parsing should succeed");
    }

    #[test]
    fn test_metadata_extraction() {
        let dsl_with_metadata = r#"
        metadata {
            version: "1.0",
            author: "Test"
        }
        "#;

        if let Ok(tree) = parse_dsl(dsl_with_metadata) {
            let metadata = extract_metadata(&tree, dsl_with_metadata);
            assert!(!metadata.is_empty(), "Should extract metadata");
        }
    }

    #[test]
    fn test_with_block_parsing() {
        let agent_with_sandbox = r#"
        agent code_runner(script: String) -> Output {
            with sandbox = "e2b", timeout = 60.seconds {
                return execute(script);
            }
        }
        "#;

        if let Ok(tree) = parse_dsl(agent_with_sandbox) {
            let with_blocks = extract_with_blocks(&tree, agent_with_sandbox).unwrap();
            assert_eq!(with_blocks.len(), 1, "Should extract one with block");
            
            let with_block = &with_blocks[0];
            assert_eq!(with_block.sandbox_tier, Some(SandboxTier::E2B));
            assert_eq!(with_block.timeout, Some(60));
        }
    }

    #[test]
    fn test_sandbox_tier_validation() {
        assert_eq!(WithBlock::parse_sandbox_tier("docker"), Ok(SandboxTier::Docker));
        assert_eq!(WithBlock::parse_sandbox_tier("gvisor"), Ok(SandboxTier::GVisor));
        assert_eq!(WithBlock::parse_sandbox_tier("firecracker"), Ok(SandboxTier::Firecracker));
        assert_eq!(WithBlock::parse_sandbox_tier("e2b"), Ok(SandboxTier::E2B));
        
        // Test with quotes
        assert_eq!(WithBlock::parse_sandbox_tier("\"docker\""), Ok(SandboxTier::Docker));
        
        // Test invalid tier
        assert!(WithBlock::parse_sandbox_tier("invalid").is_err());
    }

    #[test]
    fn test_with_block_attributes() {
        let agent_with_multiple_attrs = r#"
        agent test_agent {
            with sandbox = "docker", timeout = 30.seconds {
                return success();
            }
        }
        "#;

        if let Ok(tree) = parse_dsl(agent_with_multiple_attrs) {
            let with_blocks = extract_with_blocks(&tree, agent_with_multiple_attrs).unwrap();
            assert_eq!(with_blocks.len(), 1);
            
            let with_block = &with_blocks[0];
            assert_eq!(with_block.attributes.len(), 2);
            assert_eq!(with_block.sandbox_tier, Some(SandboxTier::Docker));
            assert_eq!(with_block.timeout, Some(30));
        }
    }
}
