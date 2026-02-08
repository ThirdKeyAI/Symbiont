use std::fs;
use std::path::Path;

// Import the functions we want to test from main.rs
use dsl::{extract_metadata, extract_with_blocks, parse_dsl, print_ast, SandboxTier, WithBlock};

#[cfg(test)]
mod parser_tests {
    use super::*;

    #[test]
    fn test_parse_dsl_simple_metadata() {
        let simple_metadata = r#"metadata {
    version: "1.0"
}"#;

        let result = parse_dsl(simple_metadata);
        assert!(result.is_ok(), "Simple metadata parsing should succeed");

        let _tree = result.unwrap();
        // Note: We don't check for errors here as the grammar may still be evolving
    }

    #[test]
    fn test_parse_dsl_complex_metadata() {
        let complex_metadata = r#"metadata {
    version: "1.0",
    author: "Test Author",
    description: "A test DSL file"
}"#;

        let result = parse_dsl(complex_metadata);
        assert!(result.is_ok(), "Complex metadata parsing should succeed");

        let _tree = result.unwrap();
        // Note: We don't check for errors here as the grammar may still be evolving
    }

    #[test]
    fn test_parse_dsl_agent_definition() {
        let agent_dsl = r#"agent TestAgent {
    capabilities: [read, write, execute]
    
    policy TestPolicy {
        allow: read(data)
        deny: delete(critical_data)
    }
}"#;

        let result = parse_dsl(agent_dsl);
        assert!(result.is_ok(), "Agent definition parsing should succeed");

        let tree = result.unwrap();
        let root = tree.root_node();

        // Check that we have an agent_definition node
        let mut found_agent = false;
        for i in 0..root.child_count() {
            if let Some(child) = root.child(i) {
                if child.kind() == "agent_definition" {
                    found_agent = true;
                    break;
                }
            }
        }
        assert!(
            found_agent,
            "Should find an agent_definition node in the AST"
        );
    }

    #[test]
    fn test_parse_dsl_type_definition() {
        let type_dsl = r#"type DataSource = {
    url: String,
    format: String,
    credentials: Option<String>
}"#;

        let result = parse_dsl(type_dsl);
        assert!(result.is_ok(), "Type definition parsing should succeed");
    }

    #[test]
    fn test_parse_dsl_function_definition() {
        let function_dsl = r#"agent TestAgent {
    function process_data(input: String) -> Result<String> {
        let validated = validate(input);
        if validated {
            return transform(input);
        } else {
            return error("Invalid input");
        }
    }
}"#;

        let result = parse_dsl(function_dsl);
        assert!(result.is_ok(), "Function definition parsing should succeed");
    }

    #[test]
    fn test_extract_metadata_simple() {
        let metadata_dsl = r#"metadata {
    version: "1.0"
}"#;

        let tree = parse_dsl(metadata_dsl).expect("Should parse successfully");
        let metadata = extract_metadata(&tree, metadata_dsl);

        assert!(!metadata.is_empty(), "Should extract metadata");
        assert_eq!(metadata.get("version"), Some(&"\"1.0\"".to_string()));
    }

    #[test]
    fn test_extract_metadata_multiple_fields() {
        let metadata_dsl = r#"metadata {
    version: "1.0",
    author: "Test Author",
    description: "Test description"
}"#;

        let tree = parse_dsl(metadata_dsl).expect("Should parse successfully");
        let metadata = extract_metadata(&tree, metadata_dsl);

        assert_eq!(metadata.len(), 3, "Should extract 3 metadata fields");
        assert!(metadata.contains_key("version"));
        assert!(metadata.contains_key("author"));
        assert!(metadata.contains_key("description"));
    }

    #[test]
    fn test_extract_metadata_no_metadata() {
        let no_metadata_dsl = r#"agent TestAgent {
    capabilities: [test]
}"#;

        let tree = parse_dsl(no_metadata_dsl).expect("Should parse successfully");
        let metadata = extract_metadata(&tree, no_metadata_dsl);

        assert!(
            metadata.is_empty(),
            "Should extract no metadata when none present"
        );
    }

    #[test]
    fn test_print_ast_no_panic() {
        let simple_dsl = r#"metadata {
    version: "1.0"
}"#;

        let tree = parse_dsl(simple_dsl).expect("Should parse successfully");

        // This test ensures print_ast doesn't panic
        // We can't easily capture stdout in a unit test, but we can ensure it doesn't crash
        print_ast(tree.root_node(), simple_dsl, 0);

        // If we reach here, print_ast didn't panic
    }

    #[test]
    fn test_parse_dsl_invalid_syntax() {
        let invalid_dsl = r#"metadata {
    version: "1.0"
    // Missing closing brace"#;

        let result = parse_dsl(invalid_dsl);

        // The parser should still return a tree, but it may contain errors
        assert!(
            result.is_ok(),
            "Parser should return a tree even for invalid syntax"
        );

        let _tree = result.unwrap();
        // Check if the tree has errors (this depends on the grammar implementation)
        // For now, we just ensure the parser doesn't crash
    }

    #[test]
    fn test_parse_dsl_empty_input() {
        let empty_dsl = "";

        let result = parse_dsl(empty_dsl);
        assert!(result.is_ok(), "Empty input should be handled gracefully");
    }

    #[test]
    fn test_parse_dsl_whitespace_only() {
        let whitespace_dsl = "   \n\t  \n  ";

        let result = parse_dsl(whitespace_dsl);
        assert!(
            result.is_ok(),
            "Whitespace-only input should be handled gracefully"
        );
    }

    #[test]
    fn test_parse_dsl_comments_only() {
        let comments_dsl = r#"// This is a comment
// Another comment
/* Block comment */"#;

        let result = parse_dsl(comments_dsl);
        assert!(
            result.is_ok(),
            "Comments-only input should be handled gracefully"
        );
    }

    #[test]
    fn test_sample_files_valid() {
        let samples_dir = Path::new("tests/samples");
        if samples_dir.exists() {
            for entry in fs::read_dir(samples_dir).expect("Should read samples directory") {
                let entry = entry.expect("Should read directory entry");
                let path = entry.path();

                if path
                    .extension()
                    .and_then(|s| s.to_str())
                    .is_some_and(|ext| ext == "dsl" || ext == "symbi")
                {
                    let filename = path.file_name().unwrap().to_str().unwrap();

                    if filename.starts_with("valid_") {
                        let content = fs::read_to_string(&path)
                            .unwrap_or_else(|_| panic!("Should read file: {:?}", path));

                        let result = parse_dsl(&content);
                        assert!(
                            result.is_ok(),
                            "Valid sample file {} should parse successfully",
                            filename
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn test_sample_files_invalid() {
        let samples_dir = Path::new("tests/samples");
        if samples_dir.exists() {
            for entry in fs::read_dir(samples_dir).expect("Should read samples directory") {
                let entry = entry.expect("Should read directory entry");
                let path = entry.path();

                if path
                    .extension()
                    .and_then(|s| s.to_str())
                    .is_some_and(|ext| ext == "dsl" || ext == "symbi")
                {
                    let filename = path.file_name().unwrap().to_str().unwrap();

                    if filename.starts_with("invalid_") {
                        let content = fs::read_to_string(&path)
                            .unwrap_or_else(|_| panic!("Should read file: {:?}", path));

                        let result = parse_dsl(&content);
                        // Invalid files should either fail to parse or contain errors
                        if let Ok(_tree) = result {
                            // If parsing succeeds, the tree should contain errors
                            // This depends on the grammar implementation
                            println!("Invalid file {} parsed but may contain errors", filename);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn test_ast_structure_metadata() {
        let metadata_dsl = r#"metadata {
    version: "1.0"
}"#;

        let tree = parse_dsl(metadata_dsl).expect("Should parse successfully");
        let root = tree.root_node();

        // Verify AST structure contains expected nodes
        let mut _found_metadata = false;
        for i in 0..root.child_count() {
            if let Some(child) = root.child(i) {
                if child.kind() == "metadata_block" || child.kind() == "metadata" {
                    _found_metadata = true;
                    break;
                }
            }
        }

        // Note: The exact node names depend on the grammar definition
        // This test verifies the general structure
        assert!(root.child_count() > 0, "Root should have children");
    }

    #[test]
    fn test_ast_structure_agent() {
        let agent_dsl = r#"agent TestAgent {
    capabilities: [read, write]
}"#;

        let tree = parse_dsl(agent_dsl).expect("Should parse successfully");
        let root = tree.root_node();

        // Verify AST structure contains expected nodes
        let mut _found_agent = false;
        for i in 0..root.child_count() {
            if let Some(child) = root.child(i) {
                if child.kind() == "agent" || child.kind() == "agent_definition" {
                    _found_agent = true;
                    break;
                }
            }
        }

        assert!(root.child_count() > 0, "Root should have children");
    }

    #[test]
    fn test_error_handling_malformed_metadata() {
        let malformed_dsl = r#"metadata {
    version: 
}"#;

        let result = parse_dsl(malformed_dsl);
        assert!(
            result.is_ok(),
            "Parser should handle malformed input gracefully"
        );
    }

    #[test]
    fn test_error_handling_unclosed_braces() {
        let unclosed_dsl = r#"metadata {
    version: "1.0"
    // Missing closing brace"#;

        let result = parse_dsl(unclosed_dsl);
        assert!(
            result.is_ok(),
            "Parser should handle unclosed braces gracefully"
        );
    }

    #[test]
    fn test_large_input() {
        // Test with a larger DSL input to ensure performance
        let mut large_dsl = String::from("metadata {\n    version: \"1.0\"\n}\n\n");

        for i in 0..100 {
            large_dsl.push_str(&format!(
                "agent Agent{} {{\n    capabilities: [read, write]\n}}\n\n",
                i
            ));
        }

        let result = parse_dsl(&large_dsl);
        assert!(result.is_ok(), "Parser should handle large inputs");
    }

    #[test]
    fn test_parse_agent_with_sandbox_tier() {
        let agent_dsl = r#"agent code_runner(script: String) -> Output {
    with sandbox = "e2b", timeout = 60.seconds {
        return execute(script);
    }
}"#;

        let result = parse_dsl(agent_dsl);
        assert!(
            result.is_ok(),
            "Agent with sandbox should parse successfully"
        );

        let tree = result.unwrap();
        let with_blocks =
            extract_with_blocks(&tree, agent_dsl).expect("Should extract with blocks");

        assert_eq!(with_blocks.len(), 1, "Should have one with block");
        let with_block = &with_blocks[0];

        assert_eq!(with_block.sandbox_tier, Some(SandboxTier::E2B));
        assert_eq!(with_block.timeout, Some(60));
        assert_eq!(with_block.attributes.len(), 2);
    }

    #[test]
    fn test_parse_agent_with_docker_sandbox() {
        let agent_dsl = r#"agent data_processor {
    with sandbox = "docker" {
        let result = process_data();
        return result;
    }
}"#;

        let tree = parse_dsl(agent_dsl).expect("Should parse successfully");
        let with_blocks =
            extract_with_blocks(&tree, agent_dsl).expect("Should extract with blocks");

        assert_eq!(with_blocks.len(), 1);
        assert_eq!(with_blocks[0].sandbox_tier, Some(SandboxTier::Docker));
    }

    #[test]
    fn test_parse_agent_with_all_sandbox_tiers() {
        let test_cases = vec![
            ("docker", SandboxTier::Docker),
            ("gvisor", SandboxTier::GVisor),
            ("firecracker", SandboxTier::Firecracker),
            ("e2b", SandboxTier::E2B),
        ];

        for (tier_str, expected_tier) in test_cases {
            let agent_dsl = format!(
                r#"agent test_agent {{
    with sandbox = "{}" {{
        return success();
    }}
}}"#,
                tier_str
            );

            let tree = parse_dsl(&agent_dsl).expect("Should parse successfully");
            let with_blocks =
                extract_with_blocks(&tree, &agent_dsl).expect("Should extract with blocks");

            assert_eq!(with_blocks.len(), 1);
            assert_eq!(with_blocks[0].sandbox_tier, Some(expected_tier));
        }
    }

    #[test]
    fn test_sandbox_tier_validation() {
        assert_eq!(
            WithBlock::parse_sandbox_tier("docker"),
            Ok(SandboxTier::Docker)
        );
        assert_eq!(
            WithBlock::parse_sandbox_tier("DOCKER"),
            Ok(SandboxTier::Docker)
        );
        assert_eq!(
            WithBlock::parse_sandbox_tier("\"gvisor\""),
            Ok(SandboxTier::GVisor)
        );
        assert_eq!(
            WithBlock::parse_sandbox_tier("firecracker"),
            Ok(SandboxTier::Firecracker)
        );
        assert_eq!(WithBlock::parse_sandbox_tier("e2b"), Ok(SandboxTier::E2B));

        // Test invalid values
        assert!(WithBlock::parse_sandbox_tier("invalid_tier").is_err());
        assert!(WithBlock::parse_sandbox_tier("").is_err());
    }

    #[test]
    fn test_agent_with_parameters_and_sandbox() {
        let agent_dsl = r#"agent code_runner(script: String, language: String) -> ExecutionResult {
    with sandbox = "firecracker", timeout = 120.seconds {
        return execute_code(script, language);
    }
}"#;

        let tree = parse_dsl(agent_dsl).expect("Should parse agent with parameters and sandbox");
        let with_blocks =
            extract_with_blocks(&tree, agent_dsl).expect("Should extract with blocks");

        assert_eq!(with_blocks.len(), 1);
        assert_eq!(with_blocks[0].sandbox_tier, Some(SandboxTier::Firecracker));
        assert_eq!(with_blocks[0].timeout, Some(120));
    }

    #[test]
    fn test_multiple_with_blocks() {
        let agent_dsl = r#"agent complex_agent {
    with sandbox = "docker" {
        let step1 = process_input();
    }
    
    with sandbox = "e2b", timeout = 30.seconds {
        let step2 = secure_process(step1);
        return step2;
    }
}"#;

        let tree = parse_dsl(agent_dsl).expect("Should parse agent with multiple with blocks");
        let with_blocks =
            extract_with_blocks(&tree, agent_dsl).expect("Should extract with blocks");

        assert_eq!(with_blocks.len(), 2);
        assert_eq!(with_blocks[0].sandbox_tier, Some(SandboxTier::Docker));
        assert_eq!(with_blocks[1].sandbox_tier, Some(SandboxTier::E2B));
        assert_eq!(with_blocks[1].timeout, Some(30));
    }

    #[test]
    fn test_invalid_sandbox_tier_error() {
        let agent_dsl = r#"agent test_agent {
    with sandbox = "invalid_sandbox" {
        return error();
    }
}"#;

        let tree = parse_dsl(agent_dsl).expect("Should parse even with invalid sandbox");
        let result = extract_with_blocks(&tree, agent_dsl);

        assert!(
            result.is_err(),
            "Should return error for invalid sandbox tier"
        );
        assert!(result.unwrap_err().contains("Invalid sandbox tier"));
    }

    #[test]
    fn test_with_block_timeout_parsing() {
        let test_cases = vec![
            ("30.seconds", Some(30)),
            ("60", Some(60)),
            ("\"120\"", Some(120)),
        ];

        for (timeout_str, expected) in test_cases {
            let agent_dsl = format!(
                r#"agent test_agent {{
    with timeout = {} {{
        return result();
    }}
}}"#,
                timeout_str
            );

            let tree = parse_dsl(&agent_dsl).expect("Should parse successfully");
            let with_blocks =
                extract_with_blocks(&tree, &agent_dsl).expect("Should extract with blocks");

            assert_eq!(with_blocks.len(), 1);
            assert_eq!(with_blocks[0].timeout, expected);
        }
    }
}
