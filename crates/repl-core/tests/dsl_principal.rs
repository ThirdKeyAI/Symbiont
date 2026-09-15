//! Public parser/evaluator, communication policy and inference integration.
//! Synthetic inference records actual permitted calls; denied callers cannot reach it.
use repl_core::dsl::ast::*;
use repl_core::dsl::evaluator::{DslEvaluator, DslValue, ExecutionContext};
use repl_core::dsl::{lexer::Lexer, parser::Parser};
use repl_core::runtime_bridge::RuntimeBridge;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

struct PrincipalFixtureProvider {
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl symbi_runtime::reasoning::inference::InferenceProvider for PrincipalFixtureProvider {
    async fn complete(
        &self,
        _conversation: &symbi_runtime::reasoning::conversation::Conversation,
        _options: &symbi_runtime::reasoning::inference::InferenceOptions,
    ) -> std::result::Result<
        symbi_runtime::reasoning::inference::InferenceResponse,
        symbi_runtime::reasoning::inference::InferenceError,
    > {
        use symbi_runtime::reasoning::inference::{FinishReason, InferenceResponse, Usage};
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        // Keep the permitted call pending while other principals reach the gate.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        Ok(InferenceResponse {
            content: "permitted fixture response".into(),
            tool_calls: vec![],
            finish_reason: FinishReason::Stop,
            usage: Usage::default(),
            model: "fixture".into(),
        })
    }
    fn provider_name(&self) -> &str {
        "principal-fixture"
    }
    fn default_model(&self) -> &str {
        "fixture"
    }
    fn supports_native_tools(&self) -> bool {
        false
    }
    fn supports_structured_output(&self) -> bool {
        false
    }
}

fn parse_fixture(source: &str) -> Program {
    let tokens = Lexer::new(source).tokenize().unwrap();
    Parser::new(tokens).parse().unwrap()
}

async fn verify_builtin_callers(builtin: &str) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use symbi_runtime::communication::policy_gate::{
        CommunicationCondition, CommunicationEffect, CommunicationPolicyGate,
        CommunicationPolicyRule,
    };
    use symbi_runtime::types::AgentId;
    let caller = Uuid::new_v4();
    let sibling = Uuid::new_v4();
    let provider = Arc::new(PrincipalFixtureProvider {
        calls: AtomicUsize::new(0),
    });
    let project = tempfile::tempdir().unwrap();
    let bridge = Arc::new(
        RuntimeBridge::new_permissive_for_dev()
            .with_project_root(project.path())
            .unwrap(),
    );
    bridge.set_inference_provider(provider.clone());
    bridge
        .register_agent("receiver", "Synthetic fixture only", vec![])
        .await;
    bridge.set_comm_policy(Arc::new(CommunicationPolicyGate::new(vec![
        CommunicationPolicyRule {
            id: "one-caller".into(),
            name: "Permit only the selected caller".into(),
            condition: CommunicationCondition::SenderIs(AgentId(caller)),
            effect: CommunicationEffect::Allow,
            priority: 1,
        },
    ])));
    let evaluator = DslEvaluator::new(bridge.clone());
    let program = parse_fixture(&format!(
        r#"
        function relay() {{ return {builtin}("receiver", "fixture request") }}
        function outer() {{ return relay() }}
    "#
    ));
    let mut functions = HashMap::new();
    for declaration in program.declarations {
        if let Declaration::Function(function) = declaration {
            functions.insert(function.name.clone(), function);
        }
    }
    let expression = Expression::FunctionCall(FunctionCall {
        function: "outer".into(),
        arguments: vec![],
        span: program.span,
    });
    let mut permitted = ExecutionContext {
        agent_id: Some(caller),
        functions: functions.clone(),
        ..Default::default()
    };
    let mut denied = ExecutionContext {
        agent_id: Some(sibling),
        functions: functions.clone(),
        ..Default::default()
    };
    let mut anonymous = ExecutionContext {
        functions,
        ..Default::default()
    };
    let (allowed, sibling_result, anonymous_result) = tokio::join!(
        evaluator.evaluate_expression(&expression, &mut permitted),
        evaluator.evaluate_expression(&expression, &mut denied),
        evaluator.evaluate_expression(&expression, &mut anonymous)
    );
    assert_eq!(
        allowed.unwrap(),
        DslValue::String("permitted fixture response".into())
    );
    assert!(sibling_result
        .unwrap_err()
        .to_string()
        .contains("communication denied"));
    assert!(anonymous_result
        .unwrap_err()
        .to_string()
        .contains("communication denied"));
    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        1,
        "denied callers cannot reach inference"
    );
    assert_eq!(permitted.agent_id, Some(caller));
    assert_eq!(denied.agent_id, Some(sibling));
    assert_eq!(anonymous.agent_id, None);
    assert_eq!(
        bridge.reasoning_context().sender_agent_id,
        None,
        "invocation identity is not shared bridge state"
    );
}

#[tokio::test]
async fn nested_ask_preserves_the_caller_and_denies_other_principals() {
    verify_builtin_callers("ask").await;
}

#[tokio::test]
async fn nested_delegate_preserves_the_caller_and_denies_other_principals() {
    verify_builtin_callers("delegate").await;
}
