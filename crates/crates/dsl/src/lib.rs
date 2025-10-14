//! Symbiont DSL Parser Library
//!
//! This library provides parsing capabilities for the Symbiont DSL using Tree-sitter.

use std::collections::HashMap;
use tree_sitter::{Language, Node, Parser, Tree};

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
}
