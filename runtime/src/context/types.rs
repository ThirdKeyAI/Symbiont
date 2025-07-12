//! Core data structures for the Context & Knowledge Systems

use std::collections::HashMap;
use std::time::{Duration, SystemTime};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::types::AgentId;

/// Unique identifier for context sessions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub Uuid);

impl SessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

/// Unique identifier for context items
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContextId(pub Uuid);

impl ContextId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl std::fmt::Display for ContextId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Default for ContextId {
    fn default() -> Self {
        Self::new()
    }
}

/// Unique identifier for knowledge items
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KnowledgeId(pub Uuid);

impl KnowledgeId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl std::fmt::Display for KnowledgeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Default for KnowledgeId {
    fn default() -> Self {
        Self::new()
    }
}

/// Unique identifier for vectors
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VectorId(pub Uuid);

impl VectorId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for VectorId {
    fn default() -> Self {
        Self::new()
    }
}

/// Main agent context structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentContext {
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub memory: HierarchicalMemory,
    pub knowledge_base: KnowledgeBase,
    pub conversation_history: Vec<ConversationItem>,
    pub metadata: HashMap<String, String>,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
    pub retention_policy: RetentionPolicy,
}

/// Hierarchical memory structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HierarchicalMemory {
    pub working_memory: WorkingMemory,
    pub short_term: Vec<MemoryItem>,
    pub long_term: Vec<MemoryItem>,
    pub episodic_memory: Vec<Episode>,
    pub semantic_memory: Vec<SemanticMemoryItem>,
}

/// Working memory for immediate processing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkingMemory {
    pub variables: HashMap<String, Value>,
    pub active_goals: Vec<String>,
    pub current_context: Option<String>,
    pub attention_focus: Vec<String>,
}

/// Individual memory item
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryItem {
    pub id: ContextId,
    pub content: String,
    pub memory_type: MemoryType,
    pub importance: f32,
    pub access_count: u32,
    pub last_accessed: SystemTime,
    pub created_at: SystemTime,
    pub embedding: Option<Vec<f32>>,
    pub metadata: HashMap<String, String>,
}

/// Types of memory
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryType {
    Factual,
    Procedural,
    Episodic,
    Semantic,
    Working,
}

/// Semantic memory item for concepts and relationships
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticMemoryItem {
    pub id: ContextId,
    pub concept: String,
    pub relationships: Vec<ConceptRelationship>,
    pub properties: HashMap<String, Value>,
    pub confidence: f32,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}

/// Relationship between concepts
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConceptRelationship {
    pub relation_type: RelationType,
    pub target_concept: String,
    pub strength: f32,
    pub bidirectional: bool,
}

/// Types of concept relationships
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RelationType {
    IsA,
    PartOf,
    RelatedTo,
    Causes,
    Enables,
    Requires,
    Similar,
    Opposite,
    Custom(String),
}

/// Episodic memory for experiences
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Episode {
    pub id: ContextId,
    pub title: String,
    pub description: String,
    pub events: Vec<EpisodeEvent>,
    pub outcome: Option<String>,
    pub lessons_learned: Vec<String>,
    pub timestamp: SystemTime,
    pub importance: f32,
}

/// Individual event within an episode
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpisodeEvent {
    pub action: String,
    pub result: String,
    pub timestamp: SystemTime,
    pub context: HashMap<String, String>,
}

/// Agent knowledge base
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeBase {
    pub facts: Vec<KnowledgeFact>,
    pub procedures: Vec<Procedure>,
    pub learned_patterns: Vec<Pattern>,
    pub shared_knowledge: Vec<SharedKnowledgeRef>,
    pub domain_expertise: HashMap<String, ExpertiseLevel>,
}

/// Individual knowledge fact
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeFact {
    pub id: KnowledgeId,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub confidence: f32,
    pub source: KnowledgeSource,
    pub created_at: SystemTime,
    pub verified: bool,
}

/// Procedural knowledge
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Procedure {
    pub id: KnowledgeId,
    pub name: String,
    pub description: String,
    pub steps: Vec<ProcedureStep>,
    pub preconditions: Vec<String>,
    pub postconditions: Vec<String>,
    pub success_rate: f32,
}

/// Individual procedure step
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcedureStep {
    pub order: u32,
    pub action: String,
    pub expected_result: String,
    pub error_handling: Option<String>,
}

/// Learned patterns
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pattern {
    pub id: KnowledgeId,
    pub name: String,
    pub description: String,
    pub conditions: Vec<String>,
    pub outcomes: Vec<String>,
    pub confidence: f32,
    pub occurrences: u32,
}

/// Reference to shared knowledge
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedKnowledgeRef {
    pub knowledge_id: KnowledgeId,
    pub source_agent: AgentId,
    pub shared_at: SystemTime,
    pub access_level: AccessLevel,
    pub trust_score: f32,
}

/// Knowledge source tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KnowledgeSource {
    Experience,
    Learning,
    SharedFromAgent(AgentId),
    ExternalDocument(String),
    UserProvided,
}

/// Expertise levels
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExpertiseLevel {
    Novice,
    Intermediate,
    Advanced,
    Expert,
}

/// Access levels for knowledge sharing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AccessLevel {
    Public,
    Restricted,
    Confidential,
    Secret,
}

/// Conversation history item
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationItem {
    pub id: ContextId,
    pub role: ConversationRole,
    pub content: String,
    pub timestamp: SystemTime,
    pub context_used: Vec<ContextId>,
    pub knowledge_used: Vec<KnowledgeId>,
}

/// Conversation roles
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConversationRole {
    User,
    Agent,
    System,
    Tool,
}

/// Context retention policies
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionPolicy {
    pub session_retention: Duration,
    pub memory_retention: Duration,
    pub knowledge_retention: Duration,
    pub auto_archive: bool,
    pub encryption_required: bool,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            session_retention: Duration::from_secs(86400), // 24 hours
            memory_retention: Duration::from_secs(604800), // 7 days
            knowledge_retention: Duration::from_secs(2592000), // 30 days
            auto_archive: true,
            encryption_required: true,
        }
    }
}

/// Context query parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextQuery {
    pub query_type: QueryType,
    pub search_terms: Vec<String>,
    pub time_range: Option<TimeRange>,
    pub memory_types: Vec<MemoryType>,
    pub relevance_threshold: f32,
    pub max_results: usize,
    pub include_embeddings: bool,
}

/// Query types for context search
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum QueryType {
    Semantic,
    Keyword,
    Temporal,
    Similarity,
    Hybrid,
}

/// Time range for queries
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: SystemTime,
    pub end: SystemTime,
}

/// Context query result item
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextItem {
    pub id: ContextId,
    pub content: String,
    pub item_type: ContextItemType,
    pub relevance_score: f32,
    pub timestamp: SystemTime,
    pub metadata: HashMap<String, String>,
}

/// Types of context items
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContextItemType {
    Memory(MemoryType),
    Knowledge(KnowledgeType),
    Conversation,
    Episode,
}

/// Knowledge types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KnowledgeType {
    Fact,
    Procedure,
    Pattern,
    Shared,
}

/// Memory update operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryUpdate {
    pub operation: UpdateOperation,
    pub target: MemoryTarget,
    pub data: Value,
}

/// Update operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UpdateOperation {
    Add,
    Update,
    Delete,
    Increment,
}

/// Memory update targets
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MemoryTarget {
    ShortTerm(ContextId),
    LongTerm(ContextId),
    Working(String),
    Episodic(ContextId),
    Semantic(ContextId),
}

/// Knowledge item for search results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeItem {
    pub id: KnowledgeId,
    pub content: String,
    pub knowledge_type: KnowledgeType,
    pub confidence: f32,
    pub relevance_score: f32,
    pub source: KnowledgeSource,
    pub created_at: SystemTime,
}

/// Knowledge for adding to knowledge base
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Knowledge {
    Fact(KnowledgeFact),
    Procedure(Procedure),
    Pattern(Pattern),
}

/// Context statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextStats {
    pub total_memory_items: usize,
    pub total_knowledge_items: usize,
    pub total_conversations: usize,
    pub total_episodes: usize,
    pub memory_size_bytes: usize,
    pub last_activity: SystemTime,
    pub retention_status: RetentionStatus,
}

/// Retention status information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionStatus {
    pub items_to_archive: usize,
    pub items_to_delete: usize,
    pub next_cleanup: SystemTime,
}

/// Context-related errors
#[derive(Debug, Clone, thiserror::Error)]
pub enum ContextError {
    #[error("Context not found: {id}")]
    NotFound { id: ContextId },
    
    #[error("Knowledge not found: {id}")]
    KnowledgeNotFound { id: KnowledgeId },
    
    #[error("Session not found: {id}")]
    SessionNotFound { id: SessionId },
    
    #[error("Storage error: {reason}")]
    StorageError { reason: String },
    
    #[error("Serialization error: {reason}")]
    SerializationError { reason: String },
    
    #[error("Query error: {reason}")]
    QueryError { reason: String },
    
    #[error("Policy violation: {reason}")]
    PolicyViolation { reason: String },
    
    #[error("Access denied: {reason}")]
    AccessDenied { reason: String },
    
    #[error("Invalid operation: {reason}")]
    InvalidOperation { reason: String },
    
    #[error("System error: {reason}")]
    SystemError { reason: String },
}

impl Default for HierarchicalMemory {
    fn default() -> Self {
        Self {
            working_memory: WorkingMemory::default(),
            short_term: Vec::new(),
            long_term: Vec::new(),
            episodic_memory: Vec::new(),
            semantic_memory: Vec::new(),
        }
    }
}

impl Default for WorkingMemory {
    fn default() -> Self {
        Self {
            variables: HashMap::new(),
            active_goals: Vec::new(),
            current_context: None,
            attention_focus: Vec::new(),
        }
    }
}

impl Default for KnowledgeBase {
    fn default() -> Self {
        Self {
            facts: Vec::new(),
            procedures: Vec::new(),
            learned_patterns: Vec::new(),
            shared_knowledge: Vec::new(),
            domain_expertise: HashMap::new(),
        }
    }
}

impl Default for ContextQuery {
    fn default() -> Self {
        Self {
            query_type: QueryType::Semantic,
            search_terms: Vec::new(),
            time_range: None,
            memory_types: Vec::new(),
            relevance_threshold: 0.7,
            max_results: 10,
            include_embeddings: false,
        }
    }
}