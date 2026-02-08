use super::*;
use crate::dsl::ast::{AgentDefinition, AgentMetadata, SourceLocation, Span};
use crate::dsl::evaluator::AgentState;
use crate::dsl::evaluator::{
    builtin_len, builtin_lower, builtin_upper, DslEvaluator, DslValue, ExecutionContext,
    ExecutionResult,
};
use crate::dsl::lexer::{Lexer, TokenType};
use crate::dsl::parser::Parser;
use crate::runtime_bridge::RuntimeBridge;
use std::sync::Arc;

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_evaluator() -> DslEvaluator {
        let runtime_bridge = Arc::new(RuntimeBridge::new());
        DslEvaluator::new(runtime_bridge)
    }

    #[tokio::test]
    async fn test_lexer_basic_tokens() {
        let input = "agent test_agent {}";
        let mut lexer = Lexer::new(input);

        let tokens = lexer.tokenize().unwrap();
        assert!(!tokens.is_empty());

        if let Some(first_token) = tokens.get(0) {
            assert_eq!(
                first_token.token_type,
                TokenType::Keyword(crate::dsl::lexer::Keyword::Agent)
            );
        }
    }

    #[tokio::test]
    async fn test_parser_simple_agent() {
        let input = r#"
            agent test_agent {
            }
        "#;

        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize().unwrap();
        let mut parser = Parser::new(tokens);

        let program = parser.parse();
        if let Err(ref e) = program {
            println!("Parser error: {:?}", e);
        }
        assert!(program.is_ok());

        let program = program.unwrap();
        assert_eq!(program.declarations.len(), 1);

        if let Declaration::Agent(agent) = &program.declarations[0] {
            assert_eq!(agent.name, "test_agent");
        } else {
            panic!("Expected agent declaration");
        }
    }

    #[tokio::test]
    async fn test_builtin_functions() {
        // Test len function
        let result = builtin_len(&[DslValue::String("hello".to_string())]);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), DslValue::Integer(5));

        // Test upper function
        let result = builtin_upper(&[DslValue::String("hello".to_string())]);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), DslValue::String("HELLO".to_string()));

        // Test lower function
        let result = builtin_lower(&[DslValue::String("HELLO".to_string())]);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), DslValue::String("hello".to_string()));
    }

    #[tokio::test]
    async fn test_dsl_value_json_conversion() {
        let val = DslValue::String("test".to_string());
        let json = val.to_json();
        assert_eq!(json, serde_json::json!("test"));

        let val = DslValue::Number(42.0);
        let json = val.to_json();
        assert_eq!(json, serde_json::json!(42.0));

        let val = DslValue::Boolean(true);
        let json = val.to_json();
        assert_eq!(json, serde_json::json!(true));
    }

    #[tokio::test]
    async fn test_dsl_value_truthiness() {
        assert!(DslValue::Boolean(true).is_truthy());
        assert!(!DslValue::Boolean(false).is_truthy());
        assert!(!DslValue::Null.is_truthy());
        assert!(DslValue::String("hello".to_string()).is_truthy());
        assert!(!DslValue::String("".to_string()).is_truthy());
        assert!(DslValue::Number(1.0).is_truthy());
        assert!(!DslValue::Number(0.0).is_truthy());
    }

    #[tokio::test]
    async fn test_agent_lifecycle() {
        let evaluator = create_test_evaluator();

        // Create a simple agent definition
        let agent_def = AgentDefinition {
            name: "test_agent".to_string(),
            metadata: AgentMetadata {
                name: Some("test_agent".to_string()),
                description: None,
                version: None,
                author: None,
            },
            resources: None,
            security: None,
            policies: None,
            span: Span {
                start: SourceLocation {
                    line: 1,
                    column: 1,
                    offset: 0,
                },
                end: SourceLocation {
                    line: 1,
                    column: 1,
                    offset: 0,
                },
            },
        };

        let mut context = ExecutionContext::default();
        let result = evaluator.create_agent(agent_def, &mut context).await;
        assert!(result.is_ok());

        if let Ok(ExecutionResult::Value(DslValue::Agent(agent))) = result {
            assert_eq!(agent.definition.name, "test_agent");
            assert_eq!(agent.state, AgentState::Created);

            // Test agent state transitions
            let agent_id = agent.id;
            let start_result = evaluator.start_agent(agent_id).await;
            assert!(start_result.is_ok());

            let stop_result = evaluator.stop_agent(agent_id).await;
            assert!(stop_result.is_ok());
        }
    }

    #[tokio::test]
    async fn test_execution_context() {
        let mut context = ExecutionContext::default();

        // Test variable storage
        context
            .variables
            .insert("test_var".to_string(), DslValue::Number(42.0));
        let retrieved = context.variables.get("test_var");
        assert!(retrieved.is_some());
        assert_eq!(*retrieved.unwrap(), DslValue::Number(42.0));

        // Test depth tracking
        assert_eq!(context.depth, 0);
        assert_eq!(context.max_depth, 100);
    }

    #[tokio::test]
    async fn test_literal_evaluation() {
        let evaluator = create_test_evaluator();

        // Test string literal
        let literal = Literal::String("hello".to_string());
        let result = evaluator.evaluate_literal(&literal);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), DslValue::String("hello".to_string()));

        // Test number literal
        let literal = Literal::Number(42.0);
        let result = evaluator.evaluate_literal(&literal);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), DslValue::Number(42.0));

        // Test boolean literal
        let literal = Literal::Boolean(true);
        let result = evaluator.evaluate_literal(&literal);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), DslValue::Boolean(true));
    }

    #[tokio::test]
    async fn test_snapshot_creation() {
        let evaluator = create_test_evaluator();
        let snapshot = evaluator.create_snapshot().await;

        assert!(snapshot.data.is_object());
        assert!(snapshot.data.get("agents").is_some());
        assert!(snapshot.data.get("context").is_some());
    }
}
