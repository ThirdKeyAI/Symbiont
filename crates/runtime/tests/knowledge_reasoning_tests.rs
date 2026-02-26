//! Integration tests for knowledge-reasoning bridge.
//!
//! Uses a mock `ContextManager` (the knowledge/context trait) with canned
//! responses to verify:
//! 1. Knowledge injection before reasoning
//! 2. `recall_knowledge` tool call handling
//! 3. `store_knowledge` tool call handling
//! 4. Backward compatibility without knowledge bridge
//! 5. Post-loop persistence of learnings

use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use tokio::sync::Mutex;

use symbi_runtime::context::types::*;
use symbi_runtime::context::ContextManager;
use symbi_runtime::reasoning::circuit_breaker::CircuitBreakerRegistry;
use symbi_runtime::reasoning::context_manager::DefaultContextManager;
use symbi_runtime::reasoning::conversation::{Conversation, ConversationMessage};
use symbi_runtime::reasoning::executor::DefaultActionExecutor;
use symbi_runtime::reasoning::inference::*;
use symbi_runtime::reasoning::knowledge_bridge::{KnowledgeBridge, KnowledgeConfig};
use symbi_runtime::reasoning::loop_types::{BufferedJournal, LoopConfig, TerminationReason};
use symbi_runtime::reasoning::policy_bridge::DefaultPolicyGate;
use symbi_runtime::reasoning::reasoning_loop::ReasoningLoopRunner;
use symbi_runtime::types::AgentId;

// ---------------------------------------------------------------------------
// Mock ContextManager
// ---------------------------------------------------------------------------

/// Tracks calls made to the mock and returns canned responses.
struct MockKnowledgeContextManager {
    /// Canned knowledge items returned by search_knowledge
    knowledge_items: Vec<KnowledgeItem>,
    /// Canned context items returned by query_context
    context_items: Vec<ContextItem>,
    /// Records calls to add_knowledge
    added_knowledge: Mutex<Vec<Knowledge>>,
    /// Records calls to update_memory
    memory_updates: Mutex<Vec<Vec<MemoryUpdate>>>,
}

impl MockKnowledgeContextManager {
    fn new() -> Self {
        Self {
            knowledge_items: vec![],
            context_items: vec![],
            added_knowledge: Mutex::new(vec![]),
            memory_updates: Mutex::new(vec![]),
        }
    }

    fn with_knowledge(mut self, items: Vec<KnowledgeItem>) -> Self {
        self.knowledge_items = items;
        self
    }

    fn with_context(mut self, items: Vec<ContextItem>) -> Self {
        self.context_items = items;
        self
    }
}

#[async_trait]
impl ContextManager for MockKnowledgeContextManager {
    async fn store_context(
        &self,
        _agent_id: AgentId,
        _context: AgentContext,
    ) -> Result<ContextId, ContextError> {
        Ok(ContextId::new())
    }

    async fn retrieve_context(
        &self,
        _agent_id: AgentId,
        _session_id: Option<SessionId>,
    ) -> Result<Option<AgentContext>, ContextError> {
        Ok(None)
    }

    async fn query_context(
        &self,
        _agent_id: AgentId,
        _query: ContextQuery,
    ) -> Result<Vec<ContextItem>, ContextError> {
        Ok(self.context_items.clone())
    }

    async fn update_memory(
        &self,
        _agent_id: AgentId,
        memory_updates: Vec<MemoryUpdate>,
    ) -> Result<(), ContextError> {
        self.memory_updates.lock().await.push(memory_updates);
        Ok(())
    }

    async fn add_knowledge(
        &self,
        _agent_id: AgentId,
        knowledge: Knowledge,
    ) -> Result<KnowledgeId, ContextError> {
        self.added_knowledge.lock().await.push(knowledge);
        Ok(KnowledgeId::new())
    }

    async fn search_knowledge(
        &self,
        _agent_id: AgentId,
        _query: &str,
        _limit: usize,
    ) -> Result<Vec<KnowledgeItem>, ContextError> {
        Ok(self.knowledge_items.clone())
    }

    async fn share_knowledge(
        &self,
        _from_agent: AgentId,
        _to_agent: AgentId,
        _knowledge_id: KnowledgeId,
        _access_level: AccessLevel,
    ) -> Result<(), ContextError> {
        Ok(())
    }

    async fn get_shared_knowledge(
        &self,
        _agent_id: AgentId,
    ) -> Result<Vec<SharedKnowledgeRef>, ContextError> {
        Ok(vec![])
    }

    async fn archive_context(
        &self,
        _agent_id: AgentId,
        _before: SystemTime,
    ) -> Result<u32, ContextError> {
        Ok(0)
    }

    async fn get_context_stats(&self, _agent_id: AgentId) -> Result<ContextStats, ContextError> {
        Ok(ContextStats {
            total_memory_items: 0,
            total_knowledge_items: 0,
            total_conversations: 0,
            total_episodes: 0,
            memory_size_bytes: 0,
            last_activity: SystemTime::now(),
            retention_status: RetentionStatus {
                items_to_archive: 0,
                items_to_delete: 0,
                next_cleanup: SystemTime::now(),
            },
        })
    }

    async fn shutdown(&self) -> Result<(), ContextError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Mock Inference Provider
// ---------------------------------------------------------------------------

struct MockProvider {
    responses: std::sync::Mutex<Vec<InferenceResponse>>,
}

impl MockProvider {
    fn new(responses: Vec<InferenceResponse>) -> Self {
        Self {
            responses: std::sync::Mutex::new(responses),
        }
    }
}

#[async_trait]
impl InferenceProvider for MockProvider {
    async fn complete(
        &self,
        _conversation: &Conversation,
        _options: &InferenceOptions,
    ) -> Result<InferenceResponse, InferenceError> {
        let mut responses = self.responses.lock().unwrap();
        if responses.is_empty() {
            Ok(InferenceResponse {
                content: "Done.".into(),
                tool_calls: vec![],
                finish_reason: FinishReason::Stop,
                usage: Usage::default(),
                model: "mock".into(),
            })
        } else {
            Ok(responses.remove(0))
        }
    }

    fn provider_name(&self) -> &str {
        "mock"
    }
    fn default_model(&self) -> &str {
        "mock-model"
    }
    fn supports_native_tools(&self) -> bool {
        true
    }
    fn supports_structured_output(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_runner(
    provider: Arc<dyn InferenceProvider>,
    knowledge_bridge: Option<Arc<KnowledgeBridge>>,
) -> ReasoningLoopRunner {
    ReasoningLoopRunner {
        provider,
        policy_gate: Arc::new(DefaultPolicyGate::permissive()),
        executor: Arc::new(DefaultActionExecutor::default()),
        context_manager: Arc::new(DefaultContextManager::default()),
        circuit_breakers: Arc::new(CircuitBreakerRegistry::default()),
        journal: Arc::new(BufferedJournal::new(1000)),
        knowledge_bridge,
    }
}

fn make_knowledge_item(content: &str, knowledge_type: KnowledgeType) -> KnowledgeItem {
    KnowledgeItem {
        id: KnowledgeId::new(),
        content: content.to_string(),
        knowledge_type,
        confidence: 0.9,
        relevance_score: 0.8,
        source: KnowledgeSource::Experience,
        created_at: SystemTime::now(),
    }
}

fn simple_response(content: &str) -> InferenceResponse {
    InferenceResponse {
        content: content.into(),
        tool_calls: vec![],
        finish_reason: FinishReason::Stop,
        usage: Usage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
        },
        model: "mock".into(),
    }
}

fn tool_call_response(call_id: &str, name: &str, arguments: &str) -> InferenceResponse {
    InferenceResponse {
        content: String::new(),
        tool_calls: vec![ToolCallRequest {
            id: call_id.into(),
            name: name.into(),
            arguments: arguments.into(),
        }],
        finish_reason: FinishReason::ToolCalls,
        usage: Usage {
            prompt_tokens: 10,
            completion_tokens: 10,
            total_tokens: 20,
        },
        model: "mock".into(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Test 1: Knowledge is injected as a system message before reasoning.
#[tokio::test]
async fn test_knowledge_injection() {
    let mock_cm = Arc::new(
        MockKnowledgeContextManager::new()
            .with_knowledge(vec![make_knowledge_item(
                "Rust is a systems programming language",
                KnowledgeType::Fact,
            )])
            .with_context(vec![ContextItem {
                id: ContextId::new(),
                content: "User prefers concise answers".to_string(),
                item_type: ContextItemType::Memory(MemoryType::Working),
                relevance_score: 0.85,
                timestamp: SystemTime::now(),
                metadata: HashMap::new(),
            }]),
    );

    let bridge = Arc::new(KnowledgeBridge::new(
        mock_cm,
        KnowledgeConfig {
            auto_persist: false,
            ..Default::default()
        },
    ));

    // The LLM should see the injected knowledge context.
    // We capture the conversation by having the mock just respond.
    let provider = Arc::new(MockProvider::new(vec![simple_response(
        "Rust is indeed a systems language.",
    )]));

    let runner = make_runner(provider, Some(bridge));

    let mut conv = Conversation::with_system("You are a test agent.");
    conv.push(ConversationMessage::user(
        "Tell me about Rust programming language",
    ));

    let result = runner
        .run(AgentId::new(), conv, LoopConfig::default())
        .await;

    assert!(matches!(
        result.termination_reason,
        TerminationReason::Completed
    ));
    assert_eq!(result.output, "Rust is indeed a systems language.");

    // Verify the knowledge context was injected into the conversation
    let has_knowledge_msg = result
        .conversation
        .messages()
        .iter()
        .any(|m| m.content.contains("[KNOWLEDGE_CONTEXT]"));
    assert!(
        has_knowledge_msg,
        "Knowledge context message should be present in conversation"
    );
}

/// Test 2: LLM calls `recall_knowledge` and gets results from mock context manager.
#[tokio::test]
async fn test_recall_tool_call() {
    let mock_cm =
        Arc::new(
            MockKnowledgeContextManager::new().with_knowledge(vec![make_knowledge_item(
                "The capital of France is Paris",
                KnowledgeType::Fact,
            )]),
        );

    let bridge = Arc::new(KnowledgeBridge::new(
        mock_cm,
        KnowledgeConfig {
            auto_persist: false,
            ..Default::default()
        },
    ));

    let provider = Arc::new(MockProvider::new(vec![
        // First: LLM calls recall_knowledge
        tool_call_response(
            "call_1",
            "recall_knowledge",
            r#"{"query": "capital of France"}"#,
        ),
        // Second: LLM responds with the answer
        simple_response("The capital of France is Paris."),
    ]));

    let runner = make_runner(provider, Some(bridge));

    let mut conv = Conversation::with_system("You are a geography expert.");
    conv.push(ConversationMessage::user("What is the capital of France?"));

    let result = runner
        .run(AgentId::new(), conv, LoopConfig::default())
        .await;

    assert!(matches!(
        result.termination_reason,
        TerminationReason::Completed
    ));
    assert_eq!(result.output, "The capital of France is Paris.");
    assert_eq!(result.iterations, 2);

    // Verify a tool result message was added for the recall
    // The recall_knowledge tool was called and the knowledge executor intercepted it.
    // Check that the conversation has tool results from call_1.
    let has_call_1_result = result
        .conversation
        .messages()
        .iter()
        .any(|m| m.tool_call_id.as_deref() == Some("call_1"));
    assert!(
        has_call_1_result,
        "Should have a tool result for call_1 (recall_knowledge)"
    );
}

/// Test 3: LLM calls `store_knowledge` and mock context manager receives the data.
#[tokio::test]
async fn test_store_tool_call() {
    let mock_cm = Arc::new(MockKnowledgeContextManager::new());
    let mock_cm_ref = mock_cm.clone();

    let bridge = Arc::new(KnowledgeBridge::new(
        mock_cm,
        KnowledgeConfig {
            auto_persist: false,
            ..Default::default()
        },
    ));

    let provider = Arc::new(MockProvider::new(vec![
        // First: LLM calls store_knowledge
        tool_call_response(
            "call_1",
            "store_knowledge",
            r#"{"subject": "Earth", "predicate": "has", "object": "one moon", "confidence": 0.95}"#,
        ),
        // Second: LLM responds
        simple_response("I've stored that fact."),
    ]));

    let runner = make_runner(provider, Some(bridge));

    let mut conv = Conversation::with_system("You are a science agent.");
    conv.push(ConversationMessage::user(
        "Remember that Earth has one moon",
    ));

    let result = runner
        .run(AgentId::new(), conv, LoopConfig::default())
        .await;

    assert!(matches!(
        result.termination_reason,
        TerminationReason::Completed
    ));
    assert_eq!(result.output, "I've stored that fact.");

    // Verify the knowledge was stored in the mock context manager
    let added = mock_cm_ref.added_knowledge.lock().await;
    assert_eq!(added.len(), 1, "One knowledge item should have been stored");
    match &added[0] {
        Knowledge::Fact(fact) => {
            assert_eq!(fact.subject, "Earth");
            assert_eq!(fact.predicate, "has");
            assert_eq!(fact.object, "one moon");
            assert!((fact.confidence - 0.95).abs() < f32::EPSILON);
        }
        _ => panic!("Expected a Fact knowledge item"),
    }
}

/// Test 4: Runner without knowledge bridge works identically to before.
#[tokio::test]
async fn test_backward_compat() {
    let provider = Arc::new(MockProvider::new(vec![simple_response(
        "No knowledge needed.",
    )]));

    let runner = make_runner(provider, None);

    let mut conv = Conversation::with_system("You are a test agent.");
    conv.push(ConversationMessage::user("Hello"));

    let result = runner
        .run(AgentId::new(), conv, LoopConfig::default())
        .await;

    assert!(matches!(
        result.termination_reason,
        TerminationReason::Completed
    ));
    assert_eq!(result.output, "No knowledge needed.");
    assert_eq!(result.iterations, 1);

    // No knowledge context message should be present
    let has_knowledge_msg = result
        .conversation
        .messages()
        .iter()
        .any(|m| m.content.contains("[KNOWLEDGE_CONTEXT]"));
    assert!(
        !has_knowledge_msg,
        "No knowledge context should be injected without a bridge"
    );
}

/// Test 5: After loop completion, episodic memory is persisted.
#[tokio::test]
async fn test_persist_learnings() {
    let mock_cm = Arc::new(MockKnowledgeContextManager::new());
    let mock_cm_ref = mock_cm.clone();

    let bridge = Arc::new(KnowledgeBridge::new(
        mock_cm,
        KnowledgeConfig {
            auto_persist: true,
            ..Default::default()
        },
    ));

    let provider = Arc::new(MockProvider::new(vec![simple_response(
        "The answer to everything is 42.",
    )]));

    let runner = make_runner(provider, Some(bridge));

    let mut conv = Conversation::with_system("You are a philosopher.");
    conv.push(ConversationMessage::user(
        "What is the answer to everything?",
    ));

    let result = runner
        .run(AgentId::new(), conv, LoopConfig::default())
        .await;

    assert!(matches!(
        result.termination_reason,
        TerminationReason::Completed
    ));

    // Verify that persist_learnings was called (update_memory was invoked)
    let updates = mock_cm_ref.memory_updates.lock().await;
    assert!(
        !updates.is_empty(),
        "Memory updates should have been persisted after loop completion"
    );

    // The persisted memory should contain the assistant's response
    let update = &updates[0][0];
    match &update.target {
        MemoryTarget::Working(key) => {
            assert_eq!(key, "last_conversation_summary");
        }
        other => panic!("Expected Working memory target, got {:?}", other),
    }
    assert!(
        matches!(update.operation, UpdateOperation::Add),
        "Expected Add operation"
    );

    // The data should contain the assistant's message
    let data_str = update.data.as_str().unwrap();
    assert!(
        data_str.contains("42"),
        "Persisted data should contain the assistant's response"
    );
}
