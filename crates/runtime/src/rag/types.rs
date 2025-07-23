//! RAG Engine Data Structures and Types
//!
//! This module contains all the data structures, enums, and types used by the RAG engine.

use crate::types::{AgentId, PolicyId};
use serde::{Deserialize, Serialize};

use std::time::{Duration, SystemTime};
use uuid::Uuid;

/// Errors that can occur during RAG operations
#[derive(Debug, thiserror::Error)]
pub enum RAGError {
    #[error("Query analysis failed: {0}")]
    QueryAnalysisFailed(String),

    #[error("Document retrieval failed: {0}")]
    DocumentRetrievalFailed(String),

    #[error("Ranking failed: {0}")]
    RankingFailed(String),

    #[error("Context augmentation failed: {0}")]
    ContextAugmentationFailed(String),

    #[error("Response generation failed: {0}")]
    ResponseGenerationFailed(String),

    #[error("Validation failed: {0}")]
    ValidationFailed(String),

    #[error("Configuration error: {0}")]
    ConfigurationError(String),

    #[error("Vector database error: {0}")]
    VectorDatabaseError(String),

    #[error("Context manager error: {0}")]
    ContextManagerError(String),

    #[error("Policy violation: {0}")]
    PolicyViolation(String),

    #[error("Insufficient permissions: {0}")]
    InsufficientPermissions(String),

    #[error("Timeout error: {0}")]
    Timeout(String),
}

/// Unique identifier for documents
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DocumentId(pub Uuid);

impl Default for DocumentId {
    fn default() -> Self {
        Self::new()
    }
}

impl DocumentId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

/// RAG request containing query and context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RAGRequest {
    pub agent_id: AgentId,
    pub query: String,
    pub preferences: QueryPreferences,
    pub constraints: QueryConstraints,
}

/// Query preferences for response generation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryPreferences {
    pub response_length: ResponseLength,
    pub include_citations: bool,
    pub preferred_sources: Vec<String>,
    pub response_format: ResponseFormat,
    pub language: String,
}

/// Response length preferences
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ResponseLength {
    Brief,
    Standard,
    Detailed,
    Comprehensive,
}

/// Response format options
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ResponseFormat {
    Text,
    Markdown,
    Structured,
    Code,
}

/// Query constraints and limitations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryConstraints {
    pub max_documents: usize,
    pub time_limit: Duration,
    pub security_level: AccessLevel,
    pub allowed_sources: Vec<String>,
    pub excluded_sources: Vec<String>,
}

/// Access levels for security
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AccessLevel {
    Public,
    Restricted,
    Confidential,
    Secret,
}

/// Analyzed query with expanded terms and metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyzedQuery {
    pub original_query: String,
    pub expanded_terms: Vec<String>,
    pub intent: QueryIntent,
    pub entities: Vec<Entity>,
    pub keywords: Vec<String>,
    pub embeddings: Vec<f32>,
    pub context_keywords: Vec<String>,
}

/// Query intent classification
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum QueryIntent {
    Factual,
    Procedural,
    Analytical,
    Creative,
    Comparative,
    Troubleshooting,
}

/// Named entities extracted from query
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    pub text: String,
    pub entity_type: EntityType,
    pub confidence: f32,
}

/// Types of entities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EntityType {
    Person,
    Organization,
    Location,
    Technology,
    Concept,
    Date,
    Number,
}

/// Document for retrieval and processing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: DocumentId,
    pub title: String,
    pub content: String,
    pub metadata: DocumentMetadata,
    pub embeddings: Vec<f32>,
    pub chunks: Vec<DocumentChunk>,
}

/// Document metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentMetadata {
    pub document_type: DocumentType,
    pub author: Option<String>,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
    pub language: String,
    pub domain: String,
    pub access_level: AccessLevel,
    pub tags: Vec<String>,
    pub source_url: Option<String>,
    pub file_path: Option<String>,
}

/// Types of documents
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DocumentType {
    Text,
    Code,
    Structured,
    Manual,
    API,
    Research,
}

/// Document chunk for processing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentChunk {
    pub chunk_id: String,
    pub content: String,
    pub start_index: usize,
    pub end_index: usize,
    pub embeddings: Vec<f32>,
}

/// Ranked document with relevance scoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankedDocument {
    pub document: Document,
    pub relevance_score: f32,
    pub ranking_factors: RankingFactors,
    pub selected_chunks: Vec<DocumentChunk>,
}

/// Breakdown of ranking factors
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingFactors {
    pub semantic_similarity: f32,
    pub keyword_match: f32,
    pub recency_score: f32,
    pub authority_score: f32,
    pub diversity_score: f32,
}

/// Augmented context for response generation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AugmentedContext {
    pub original_query: String,
    pub analyzed_query: AnalyzedQuery,
    pub retrieved_documents: Vec<RankedDocument>,
    pub context_summary: String,
    pub citations: Vec<Citation>,
}

/// Citation information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Citation {
    pub document_id: DocumentId,
    pub title: String,
    pub author: Option<String>,
    pub url: Option<String>,
    pub relevance_score: f32,
}

/// Generated response with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedResponse {
    pub content: String,
    pub confidence: f32,
    pub citations: Vec<Citation>,
    pub metadata: ResponseMetadata,
    pub validation_status: ValidationStatus,
}

/// Response generation metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseMetadata {
    pub generation_time: Duration,
    pub tokens_used: usize,
    pub sources_consulted: usize,
    pub model_version: String,
}

/// Validation status for responses
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ValidationStatus {
    Pending,
    Approved,
    Rejected(String),
    RequiresReview,
}

/// Validation result with details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    pub is_valid: bool,
    pub policy_violations: Vec<PolicyViolation>,
    pub content_issues: Vec<ContentIssue>,
    pub confidence_score: f32,
    pub recommendations: Vec<String>,
}

/// Policy violation details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyViolation {
    pub policy_id: PolicyId,
    pub violation_type: ViolationType,
    pub description: String,
    pub severity: Severity,
}

/// Types of policy violations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ViolationType {
    AccessControl,
    DataClassification,
    ContentFilter,
    SecurityLevel,
}

/// Severity levels
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

/// Content issues in responses
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentIssue {
    pub issue_type: ContentIssueType,
    pub description: String,
    pub confidence: f32,
}

/// Types of content issues
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContentIssueType {
    Factual,
    Bias,
    Toxicity,
    Misinformation,
    Inconsistency,
}

/// Final RAG response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RAGResponse {
    pub response: GeneratedResponse,
    pub processing_time: Duration,
    pub sources_used: Vec<Citation>,
    pub confidence_score: f32,
    pub follow_up_suggestions: Vec<String>,
}

/// Document input for ingestion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentInput {
    pub title: String,
    pub content: String,
    pub metadata: DocumentMetadata,
    pub chunking_strategy: ChunkingStrategy,
}

/// Chunking strategies for documents
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChunkingStrategy {
    FixedSize { size: usize, overlap: usize },
    Semantic { min_size: usize, max_size: usize },
    Paragraph,
    Sentence,
    Custom(String),
}

/// RAG engine statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RAGStats {
    pub total_documents: usize,
    pub total_queries: usize,
    pub avg_response_time: Duration,
    pub cache_hit_rate: f32,
    pub validation_pass_rate: f32,
    pub top_query_types: Vec<(QueryIntent, usize)>,
}

/// RAG engine configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RAGConfig {
    pub embedding_model: EmbeddingModelConfig,
    pub retrieval_config: RetrievalConfig,
    pub ranking_config: RankingConfig,
    pub generation_config: GenerationConfig,
    pub validation_config: ValidationConfig,
}

/// Embedding model configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingModelConfig {
    pub model_name: String,
    pub model_type: EmbeddingModelType,
    pub dimension: usize,
    pub max_tokens: usize,
    pub batch_size: usize,
}

/// Types of embedding models
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EmbeddingModelType {
    OpenAI,
    HuggingFace,
    Local,
    Custom,
}

/// Retrieval configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalConfig {
    pub max_documents: usize,
    pub similarity_threshold: f32,
    pub context_window: usize,
    pub enable_hybrid_search: bool,
    pub reranking_enabled: bool,
}

/// Ranking configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingConfig {
    pub ranking_algorithm: RankingAlgorithm,
    pub relevance_weight: f32,
    pub recency_weight: f32,
    pub authority_weight: f32,
    pub diversity_weight: f32,
}

/// Ranking algorithms
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RankingAlgorithm {
    CosineSimilarity,
    BM25,
    Hybrid,
    LearningToRank,
}

/// Generation configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationConfig {
    pub max_response_length: usize,
    pub temperature: f32,
    pub top_p: f32,
    pub enable_citations: bool,
    pub response_format: ResponseFormat,
}

/// Validation configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationConfig {
    pub enable_policy_check: bool,
    pub enable_content_filter: bool,
    pub enable_fact_check: bool,
    pub confidence_threshold: f32,
}
