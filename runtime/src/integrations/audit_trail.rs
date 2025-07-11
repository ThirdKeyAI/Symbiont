//! Audit Trail Integration Interface
//! 
//! Provides interface for integrating with cryptographic audit trail systems

use std::collections::HashMap;
use std::time::SystemTime;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::types::*;

/// Audit trail trait for cryptographic logging and verification
#[async_trait]
pub trait AuditTrail: Send + Sync {
    /// Record an audit event
    async fn record_event(&self, event: AuditEvent) -> Result<AuditRecordId, AuditError>;
    
    /// Batch record multiple audit events
    async fn record_events(&self, events: Vec<AuditEvent>) -> Result<Vec<AuditRecordId>, AuditError>;
    
    /// Retrieve audit records by criteria
    async fn query_records(&self, query: AuditQuery) -> Result<Vec<AuditRecord>, AuditError>;
    
    /// Get a specific audit record by ID
    async fn get_record(&self, record_id: AuditRecordId) -> Result<AuditRecord, AuditError>;
    
    /// Verify the integrity of audit records
    async fn verify_integrity(&self, records: Vec<AuditRecordId>) -> Result<IntegrityReport, AuditError>;
    
    /// Create a tamper-evident seal for a set of records
    async fn create_seal(&self, records: Vec<AuditRecordId>) -> Result<AuditSeal, AuditError>;
    
    /// Verify an audit seal
    async fn verify_seal(&self, seal: AuditSeal) -> Result<SealVerification, AuditError>;
    
    /// Export audit records for compliance
    async fn export_records(&self, query: AuditQuery, format: ExportFormat) -> Result<Vec<u8>, AuditError>;
    
    /// Get audit statistics
    async fn get_statistics(&self) -> Result<AuditStatistics, AuditError>;
    
    /// Archive old audit records
    async fn archive_records(&self, before: SystemTime) -> Result<ArchiveResult, AuditError>;
    
    /// Search audit records with full-text search
    async fn search_records(&self, search: AuditSearch) -> Result<Vec<AuditRecord>, AuditError>;
    
    /// Create an audit trail snapshot
    async fn create_snapshot(&self, name: String) -> Result<SnapshotId, AuditError>;
    
    /// List available snapshots
    async fn list_snapshots(&self) -> Result<Vec<SnapshotInfo>, AuditError>;
}

/// Audit event to be recorded
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub event_type: AuditEventType,
    pub agent_id: Option<AgentId>,
    pub user_id: Option<String>,
    pub session_id: Option<String>,
    pub timestamp: SystemTime,
    pub severity: AuditSeverity,
    pub category: AuditCategory,
    pub action: String,
    pub resource: Option<String>,
    pub details: AuditDetails,
    pub context: AuditContext,
    pub tags: Vec<String>,
}

/// Types of audit events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuditEventType {
    AgentCreated,
    AgentStarted,
    AgentStopped,
    AgentTerminated,
    AgentError,
    ResourceAllocated,
    ResourceDeallocated,
    ResourceExhausted,
    PolicyEvaluated,
    PolicyViolation,
    SecurityEvent,
    NetworkAccess,
    FileAccess,
    CommandExecution,
    DataAccess,
    ConfigurationChange,
    UserAction,
    SystemEvent,
    Custom { event_name: String },
}

/// Audit event severity levels
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum AuditSeverity {
    Trace,
    Debug,
    Info,
    Warning,
    Error,
    Critical,
    Fatal,
}

/// Audit event categories
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AuditCategory {
    Security,
    Compliance,
    Performance,
    Operations,
    Configuration,
    Data,
    Network,
    System,
    Application,
    User,
}

/// Detailed audit event information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditDetails {
    pub description: String,
    pub outcome: AuditOutcome,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub request_id: Option<String>,
    pub correlation_id: Option<String>,
    pub duration: Option<std::time::Duration>,
    pub data_size: Option<u64>,
    pub metadata: HashMap<String, String>,
}

/// Audit event outcomes
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AuditOutcome {
    Success,
    Failure,
    Partial,
    Cancelled,
    Timeout,
    Unknown,
}

/// Audit event context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditContext {
    pub source_ip: Option<String>,
    pub user_agent: Option<String>,
    pub process_id: Option<u32>,
    pub thread_id: Option<u64>,
    pub hostname: Option<String>,
    pub environment: Option<String>,
    pub version: Option<String>,
    pub location: Option<GeoLocation>,
    pub additional: HashMap<String, String>,
}

/// Geographic location information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeoLocation {
    pub country: String,
    pub region: String,
    pub city: String,
    pub latitude: f64,
    pub longitude: f64,
}

/// Stored audit record with cryptographic proof
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRecord {
    pub id: AuditRecordId,
    pub event: AuditEvent,
    pub recorded_at: SystemTime,
    pub sequence_number: u64,
    pub hash: String,
    pub previous_hash: Option<String>,
    pub signature: CryptographicSignature,
    pub merkle_proof: Option<MerkleProof>,
    pub blockchain_reference: Option<BlockchainReference>,
}

/// Cryptographic signature for audit records
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CryptographicSignature {
    pub algorithm: SignatureAlgorithm,
    pub signature: Vec<u8>,
    pub public_key: Vec<u8>,
    pub certificate: Option<Vec<u8>>,
    pub timestamp: SystemTime,
}

/// Signature algorithms
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SignatureAlgorithm {
    Ed25519,
    ECDSA,
    RSA,
    Custom { name: String },
}

/// Merkle tree proof for record integrity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerkleProof {
    pub root_hash: String,
    pub leaf_hash: String,
    pub proof_path: Vec<MerkleNode>,
    pub tree_size: u64,
}

/// Merkle tree node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerkleNode {
    pub hash: String,
    pub position: MerklePosition,
}

/// Position in merkle tree
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MerklePosition {
    Left,
    Right,
}

/// Blockchain reference for immutable storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockchainReference {
    pub blockchain: String,
    pub transaction_hash: String,
    pub block_number: u64,
    pub block_hash: String,
    pub confirmation_count: u32,
}

/// Query for retrieving audit records
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditQuery {
    pub start_time: Option<SystemTime>,
    pub end_time: Option<SystemTime>,
    pub agent_ids: Option<Vec<AgentId>>,
    pub user_ids: Option<Vec<String>>,
    pub event_types: Option<Vec<AuditEventType>>,
    pub severities: Option<Vec<AuditSeverity>>,
    pub categories: Option<Vec<AuditCategory>>,
    pub outcomes: Option<Vec<AuditOutcome>>,
    pub tags: Option<Vec<String>>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub sort_by: Option<SortField>,
    pub sort_order: Option<SortOrder>,
}

/// Fields to sort audit records by
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SortField {
    Timestamp,
    Severity,
    EventType,
    AgentId,
    UserId,
    SequenceNumber,
}

/// Sort order
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SortOrder {
    Ascending,
    Descending,
}

/// Integrity verification report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrityReport {
    pub verified_records: u32,
    pub failed_records: u32,
    pub missing_records: u32,
    pub tampered_records: Vec<AuditRecordId>,
    pub verification_time: std::time::Duration,
    pub overall_status: IntegrityStatus,
    pub details: Vec<IntegrityDetail>,
}

/// Integrity verification status
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntegrityStatus {
    Valid,
    Compromised,
    Incomplete,
    Unknown,
}

/// Detailed integrity verification information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrityDetail {
    pub record_id: AuditRecordId,
    pub status: IntegrityStatus,
    pub issue: Option<String>,
    pub recommendation: Option<String>,
}

/// Tamper-evident seal for audit records
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditSeal {
    pub id: SealId,
    pub record_ids: Vec<AuditRecordId>,
    pub created_at: SystemTime,
    pub merkle_root: String,
    pub signature: CryptographicSignature,
    pub metadata: HashMap<String, String>,
}

/// Seal verification result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealVerification {
    pub valid: bool,
    pub seal_id: SealId,
    pub verified_at: SystemTime,
    pub issues: Vec<String>,
    pub record_count: u32,
    pub verification_details: HashMap<String, String>,
}

/// Export formats for audit records
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExportFormat {
    JSON,
    CSV,
    XML,
    PDF,
    SIEM { format: String },
    Custom { format: String },
}

/// Audit statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditStatistics {
    pub total_records: u64,
    pub records_by_type: HashMap<String, u64>,
    pub records_by_severity: HashMap<AuditSeverity, u64>,
    pub records_by_category: HashMap<AuditCategory, u64>,
    pub records_by_outcome: HashMap<AuditOutcome, u64>,
    pub storage_size: u64,
    pub oldest_record: Option<SystemTime>,
    pub newest_record: Option<SystemTime>,
    pub integrity_status: IntegrityStatus,
    pub last_verification: Option<SystemTime>,
}

/// Archive operation result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveResult {
    pub archived_records: u32,
    pub archive_size: u64,
    pub archive_location: String,
    pub archive_format: String,
    pub checksum: String,
    pub completed_at: SystemTime,
}

/// Full-text search for audit records
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditSearch {
    pub query: String,
    pub fields: Option<Vec<SearchField>>,
    pub filters: Option<AuditQuery>,
    pub highlight: bool,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

/// Searchable fields in audit records
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SearchField {
    Description,
    Action,
    Resource,
    ErrorMessage,
    Metadata,
    Tags,
    All,
}

/// Snapshot information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotInfo {
    pub id: SnapshotId,
    pub name: String,
    pub created_at: SystemTime,
    pub record_count: u64,
    pub size: u64,
    pub checksum: String,
    pub metadata: HashMap<String, String>,
}

/// Type aliases
pub type AuditRecordId = uuid::Uuid;
pub type SealId = uuid::Uuid;
pub type SnapshotId = uuid::Uuid;

/// Mock audit trail for testing and development
pub struct MockAuditTrail {
    records: std::sync::RwLock<HashMap<AuditRecordId, AuditRecord>>,
    seals: std::sync::RwLock<HashMap<SealId, AuditSeal>>,
    snapshots: std::sync::RwLock<HashMap<SnapshotId, SnapshotInfo>>,
    sequence_counter: std::sync::atomic::AtomicU64,
}

impl MockAuditTrail {
    pub fn new() -> Self {
        Self {
            records: std::sync::RwLock::new(HashMap::new()),
            seals: std::sync::RwLock::new(HashMap::new()),
            snapshots: std::sync::RwLock::new(HashMap::new()),
            sequence_counter: std::sync::atomic::AtomicU64::new(1),
        }
    }

    fn create_mock_signature() -> CryptographicSignature {
        CryptographicSignature {
            algorithm: SignatureAlgorithm::Ed25519,
            signature: vec![0u8; 64], // Mock signature
            public_key: vec![0u8; 32], // Mock public key
            certificate: None,
            timestamp: SystemTime::now(),
        }
    }

    fn calculate_hash(event: &AuditEvent, sequence: u64) -> String {
        // Mock hash calculation
        format!("hash_{}_{}_{:?}", sequence, event.timestamp.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs(), event.event_type)
    }
}

impl Default for MockAuditTrail {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AuditTrail for MockAuditTrail {
    async fn record_event(&self, event: AuditEvent) -> Result<AuditRecordId, AuditError> {
        let record_id = AuditRecordId::new_v4();
        let sequence = self.sequence_counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let now = SystemTime::now();
        
        let hash = Self::calculate_hash(&event, sequence);
        let previous_hash = if sequence > 1 {
            Some(format!("prev_hash_{}", sequence - 1))
        } else {
            None
        };

        let record = AuditRecord {
            id: record_id,
            event,
            recorded_at: now,
            sequence_number: sequence,
            hash,
            previous_hash,
            signature: Self::create_mock_signature(),
            merkle_proof: None,
            blockchain_reference: None,
        };

        self.records.write().unwrap().insert(record_id, record);
        Ok(record_id)
    }

    async fn record_events(&self, events: Vec<AuditEvent>) -> Result<Vec<AuditRecordId>, AuditError> {
        let mut record_ids = Vec::new();
        for event in events {
            let record_id = self.record_event(event).await?;
            record_ids.push(record_id);
        }
        Ok(record_ids)
    }

    async fn query_records(&self, query: AuditQuery) -> Result<Vec<AuditRecord>, AuditError> {
        let records = self.records.read().unwrap();
        let mut filtered_records: Vec<_> = records.values()
            .filter(|record| {
                // Apply filters
                if let Some(start_time) = query.start_time {
                    if record.event.timestamp < start_time {
                        return false;
                    }
                }
                if let Some(end_time) = query.end_time {
                    if record.event.timestamp > end_time {
                        return false;
                    }
                }
                if let Some(ref agent_ids) = query.agent_ids {
                    if let Some(agent_id) = record.event.agent_id {
                        if !agent_ids.contains(&agent_id) {
                            return false;
                        }
                    } else {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();

        // Apply sorting
        if let Some(sort_field) = query.sort_by {
            let ascending = query.sort_order.as_ref().map_or(true, |o| matches!(o, SortOrder::Ascending));
            match sort_field {
                SortField::Timestamp => {
                    filtered_records.sort_by(|a, b| {
                        if ascending {
                            a.event.timestamp.cmp(&b.event.timestamp)
                        } else {
                            b.event.timestamp.cmp(&a.event.timestamp)
                        }
                    });
                }
                SortField::Severity => {
                    filtered_records.sort_by(|a, b| {
                        if ascending {
                            a.event.severity.cmp(&b.event.severity)
                        } else {
                            b.event.severity.cmp(&a.event.severity)
                        }
                    });
                }
                SortField::SequenceNumber => {
                    filtered_records.sort_by(|a, b| {
                        if ascending {
                            a.sequence_number.cmp(&b.sequence_number)
                        } else {
                            b.sequence_number.cmp(&a.sequence_number)
                        }
                    });
                }
                _ => {} // Other sort fields not implemented in mock
            }
        }

        // Apply pagination
        if let Some(offset) = query.offset {
            if offset as usize >= filtered_records.len() {
                return Ok(vec![]);
            }
            filtered_records = filtered_records.into_iter().skip(offset as usize).collect();
        }
        if let Some(limit) = query.limit {
            filtered_records.truncate(limit as usize);
        }

        Ok(filtered_records)
    }

    async fn get_record(&self, record_id: AuditRecordId) -> Result<AuditRecord, AuditError> {
        let records = self.records.read().unwrap();
        records.get(&record_id)
            .cloned()
            .ok_or(AuditError::RecordNotFound { id: record_id.to_string() })
    }

    async fn verify_integrity(&self, record_ids: Vec<AuditRecordId>) -> Result<IntegrityReport, AuditError> {
        let records = self.records.read().unwrap();
        let mut verified = 0;
        let mut failed = 0;
        let mut missing = 0;
        let mut details = Vec::new();

        for record_id in &record_ids {
            if let Some(record) = records.get(record_id) {
                // Mock verification - always pass
                let expected_hash = Self::calculate_hash(&record.event, record.sequence_number);
                if record.hash == expected_hash {
                    verified += 1;
                    details.push(IntegrityDetail {
                        record_id: *record_id,
                        status: IntegrityStatus::Valid,
                        issue: None,
                        recommendation: None,
                    });
                } else {
                    failed += 1;
                    details.push(IntegrityDetail {
                        record_id: *record_id,
                        status: IntegrityStatus::Compromised,
                        issue: Some("Hash mismatch".to_string()),
                        recommendation: Some("Investigate potential tampering".to_string()),
                    });
                }
            } else {
                missing += 1;
                details.push(IntegrityDetail {
                    record_id: *record_id,
                    status: IntegrityStatus::Incomplete,
                    issue: Some("Record not found".to_string()),
                    recommendation: Some("Check for data loss".to_string()),
                });
            }
        }

        let overall_status = if failed > 0 || missing > 0 {
            IntegrityStatus::Compromised
        } else {
            IntegrityStatus::Valid
        };

        Ok(IntegrityReport {
            verified_records: verified,
            failed_records: failed,
            missing_records: missing,
            tampered_records: vec![],
            verification_time: std::time::Duration::from_millis(100),
            overall_status,
            details,
        })
    }

    async fn create_seal(&self, record_ids: Vec<AuditRecordId>) -> Result<AuditSeal, AuditError> {
        let seal_id = SealId::new_v4();
        let seal = AuditSeal {
            id: seal_id,
            record_ids,
            created_at: SystemTime::now(),
            merkle_root: "mock_merkle_root".to_string(),
            signature: Self::create_mock_signature(),
            metadata: HashMap::new(),
        };

        self.seals.write().unwrap().insert(seal_id, seal.clone());
        Ok(seal)
    }

    async fn verify_seal(&self, seal: AuditSeal) -> Result<SealVerification, AuditError> {
        Ok(SealVerification {
            valid: true, // Mock always valid
            seal_id: seal.id,
            verified_at: SystemTime::now(),
            issues: vec![],
            record_count: seal.record_ids.len() as u32,
            verification_details: HashMap::new(),
        })
    }

    async fn export_records(&self, query: AuditQuery, format: ExportFormat) -> Result<Vec<u8>, AuditError> {
        let records = self.query_records(query).await?;
        
        match format {
            ExportFormat::JSON => {
                let json = serde_json::to_string_pretty(&records)
                    .map_err(|e| AuditError::ExportFailed { reason: e.to_string() })?;
                Ok(json.into_bytes())
            }
            ExportFormat::CSV => {
                let mut csv = String::from("id,timestamp,event_type,severity,action\n");
                for record in records {
                    csv.push_str(&format!(
                        "{},{:?},{:?},{:?},{}\n",
                        record.id,
                        record.event.timestamp,
                        record.event.event_type,
                        record.event.severity,
                        record.event.action
                    ));
                }
                Ok(csv.into_bytes())
            }
            _ => Err(AuditError::UnsupportedFormat { format: "Unsupported format".to_string() }),
        }
    }

    async fn get_statistics(&self) -> Result<AuditStatistics, AuditError> {
        let records = self.records.read().unwrap();
        let total_records = records.len() as u64;
        
        let mut records_by_severity = HashMap::new();
        let mut records_by_category = HashMap::new();
        let mut records_by_outcome = HashMap::new();
        let mut oldest_record = None;
        let mut newest_record = None;

        for record in records.values() {
            *records_by_severity.entry(record.event.severity.clone()).or_insert(0) += 1;
            *records_by_category.entry(record.event.category.clone()).or_insert(0) += 1;
            *records_by_outcome.entry(record.event.details.outcome.clone()).or_insert(0) += 1;

            if oldest_record.is_none() || record.event.timestamp < oldest_record.unwrap() {
                oldest_record = Some(record.event.timestamp);
            }
            if newest_record.is_none() || record.event.timestamp > newest_record.unwrap() {
                newest_record = Some(record.event.timestamp);
            }
        }

        Ok(AuditStatistics {
            total_records,
            records_by_type: HashMap::new(),
            records_by_severity,
            records_by_category,
            records_by_outcome,
            storage_size: total_records * 1024, // Mock size
            oldest_record,
            newest_record,
            integrity_status: IntegrityStatus::Valid,
            last_verification: Some(SystemTime::now()),
        })
    }

    async fn archive_records(&self, before: SystemTime) -> Result<ArchiveResult, AuditError> {
        let records = self.records.read().unwrap();
        let archived_count = records.values()
            .filter(|record| record.event.timestamp < before)
            .count() as u32;

        Ok(ArchiveResult {
            archived_records: archived_count,
            archive_size: archived_count as u64 * 1024,
            archive_location: "/tmp/audit_archive.tar.gz".to_string(),
            archive_format: "tar.gz".to_string(),
            checksum: "mock_checksum".to_string(),
            completed_at: SystemTime::now(),
        })
    }

    async fn search_records(&self, search: AuditSearch) -> Result<Vec<AuditRecord>, AuditError> {
        let records = self.records.read().unwrap();
        let filtered_records: Vec<_> = records.values()
            .filter(|record| {
                // Simple text search in description and action
                record.event.details.description.contains(&search.query) ||
                record.event.action.contains(&search.query)
            })
            .cloned()
            .collect();

        Ok(filtered_records)
    }

    async fn create_snapshot(&self, name: String) -> Result<SnapshotId, AuditError> {
        let snapshot_id = SnapshotId::new_v4();
        let records = self.records.read().unwrap();
        
        let snapshot_info = SnapshotInfo {
            id: snapshot_id,
            name,
            created_at: SystemTime::now(),
            record_count: records.len() as u64,
            size: records.len() as u64 * 1024,
            checksum: "mock_snapshot_checksum".to_string(),
            metadata: HashMap::new(),
        };

        self.snapshots.write().unwrap().insert(snapshot_id, snapshot_info);
        Ok(snapshot_id)
    }

    async fn list_snapshots(&self) -> Result<Vec<SnapshotInfo>, AuditError> {
        let snapshots = self.snapshots.read().unwrap();
        Ok(snapshots.values().cloned().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_audit_event_recording() {
        let audit_trail = MockAuditTrail::new();
        
        let event = AuditEvent {
            event_type: AuditEventType::AgentCreated,
            agent_id: Some(AgentId::new()),
            user_id: Some("test_user".to_string()),
            session_id: Some("session_123".to_string()),
            timestamp: SystemTime::now(),
            severity: AuditSeverity::Info,
            category: AuditCategory::Operations,
            action: "create_agent".to_string(),
            resource: Some("agent_resource".to_string()),
            details: AuditDetails {
                description: "Agent created successfully".to_string(),
                outcome: AuditOutcome::Success,
                error_code: None,
                error_message: None,
                request_id: Some("req_123".to_string()),
                correlation_id: None,
                duration: Some(std::time::Duration::from_millis(100)),
                data_size: None,
                metadata: HashMap::new(),
            },
            context: AuditContext {
                source_ip: Some("192.168.1.100".to_string()),
                user_agent: None,
                process_id: Some(1234),
                thread_id: Some(5678),
                hostname: Some("test-host".to_string()),
                environment: Some("test".to_string()),
                version: Some("1.0.0".to_string()),
                location: None,
                additional: HashMap::new(),
            },
            tags: vec!["test".to_string(), "agent".to_string()],
        };

        let record_id = audit_trail.record_event(event).await.unwrap();
        let retrieved_record = audit_trail.get_record(record_id).await.unwrap();
        
        assert_eq!(retrieved_record.id, record_id);
        assert_eq!(retrieved_record.event.action, "create_agent");
    }

    #[tokio::test]
    async fn test_audit_query() {
        let audit_trail = MockAuditTrail::new();
        let agent_id = AgentId::new();

        // Record multiple events
        for i in 0..5 {
            let event = AuditEvent {
                event_type: AuditEventType::AgentStarted,
                agent_id: Some(agent_id),
                user_id: Some(format!("user_{}", i)),
                session_id: None,
                timestamp: SystemTime::now(),
                severity: AuditSeverity::Info,
                category: AuditCategory::Operations,
                action: format!("action_{}", i),
                resource: None,
                details: AuditDetails {
                    description: format!("Test event {}", i),
                    outcome: AuditOutcome::Success,
                    error_code: None,
                    error_message: None,
                    request_id: None,
                    correlation_id: None,
                    duration: None,
                    data_size: None,
                    metadata: HashMap::new(),
                },
                context: AuditContext {
                    source_ip: None,
                    user_agent: None,
                    process_id: None,
                    thread_id: None,
                    hostname: None,
                    environment: None,
                    version: None,
                    location: None,
                    additional: HashMap::new(),
                },
                tags: vec![],
            };
            audit_trail.record_event(event).await.unwrap();
        }

        let query = AuditQuery {
            start_time: None,
            end_time: None,
            agent_ids: Some(vec![agent_id]),
            user_ids: None,
            event_types: None,
            severities: None,
            categories: None,
            outcomes: None,
            tags: None,
            limit: None,
            offset: None,
            sort_by: None,
            sort_order: None,
        };

        let records = audit_trail.query_records(query).await.unwrap();
        assert_eq!(records.len(), 5);
    }

    #[tokio::test]
    async fn test_integrity_verification() {
        let audit_trail = MockAuditTrail::new();
        
        let event = AuditEvent {
            event_type: AuditEventType::SecurityEvent,
            agent_id: Some(AgentId::new()),
            user_id: None,
            session_id: None,
            timestamp: SystemTime::now(),
            severity: AuditSeverity::Warning,
            category: AuditCategory::Security,
            action: "security_check".to_string(),
            resource: None,
            details: AuditDetails {
                description: "Security event occurred".to_string(),
                outcome: AuditOutcome::Success,
                error_code: None,
                error_message: None,
                request_id: None,
                correlation_id: None,
                duration: None,
                data_size: None,
                metadata: HashMap::new(),
            },
            context: AuditContext {
                source_ip: None,
                user_agent: None,
                process_id: None,
                thread_id: None,
                hostname: None,
                environment: None,
                version: None,
                location: None,
                additional: HashMap::new(),
            },
            tags: vec![],
        };

        let record_id = audit_trail.record_event(event).await.unwrap();
        let report = audit_trail.verify_integrity(vec![record_id]).await.unwrap();
        
        assert_eq!(report.overall_status, IntegrityStatus::Valid);
        assert_eq!(report.verified_records, 1);
    }
}
            