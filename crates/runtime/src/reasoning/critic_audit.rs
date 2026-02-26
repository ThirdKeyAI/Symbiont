//! Cryptographic audit for director-critic exchanges
//!
//! Provides Merkle-chained, Ed25519-signed audit entries for every
//! director-critic interaction, enabling tamper-evident review trails.

use chrono::{DateTime, Utc};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// Identity of who produced an artifact in the audit chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AuditIdentity {
    /// An LLM model acted as critic.
    #[serde(rename = "llm")]
    Llm { model_id: String },
    /// A human acted as critic.
    #[serde(rename = "human")]
    Human { user_id: String, name: String },
}

/// Verdict of a critic evaluation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditVerdict {
    Approved,
    Rejected,
    NeedsRevision,
}

/// A single auditable director-critic exchange.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CriticAuditEntry {
    /// Unique ID for this entry.
    pub entry_id: String,
    /// SHA-256 hash of the director's output.
    pub director_output_hash: String,
    /// SHA-256 hash of the critic's assessment.
    pub critic_assessment_hash: String,
    /// The critic's verdict.
    pub verdict: AuditVerdict,
    /// Per-dimension scores (for rubric evaluations).
    pub dimension_scores: HashMap<String, f64>,
    /// Overall score (0.0 - 1.0).
    pub score: f64,
    /// Identity of the critic.
    pub critic_identity: AuditIdentity,
    /// Timestamp of the exchange.
    pub timestamp: DateTime<Utc>,
    /// Chain hash: SHA-256(previous_chain_hash || entry_data).
    pub chain_hash: String,
    /// Ed25519 signature over the chain hash (hex-encoded).
    pub signature: String,
    /// Iteration number in the director-critic loop.
    pub iteration: u32,
}

/// Parameters for recording a director-critic exchange.
pub struct RecordParams<'a> {
    /// The director's output text.
    pub director_output: &'a str,
    /// The critic's assessment text.
    pub critic_assessment: &'a str,
    /// The critic's verdict.
    pub verdict: AuditVerdict,
    /// Per-dimension scores (for rubric evaluations).
    pub dimension_scores: HashMap<String, f64>,
    /// Overall score (0.0 - 1.0).
    pub score: f64,
    /// Identity of the critic.
    pub critic_identity: AuditIdentity,
    /// Iteration number in the director-critic loop.
    pub iteration: u32,
}

/// Maintains a Merkle-chained, Ed25519-signed audit trail for director-critic exchanges.
pub struct AuditChain {
    entries: Vec<CriticAuditEntry>,
    signing_key: SigningKey,
    last_chain_hash: String,
}

impl AuditChain {
    /// Create a new audit chain with the given signing key.
    pub fn new(signing_key: SigningKey) -> Self {
        let genesis = sha256_hex(b"genesis");
        Self {
            entries: Vec::new(),
            signing_key,
            last_chain_hash: genesis,
        }
    }

    /// Record a director-critic exchange and append it to the chain.
    pub fn record(&mut self, params: RecordParams<'_>) -> CriticAuditEntry {
        let entry_id = uuid::Uuid::new_v4().to_string();
        let director_output_hash = sha256_hex(params.director_output.as_bytes());
        let critic_assessment_hash = sha256_hex(params.critic_assessment.as_bytes());
        let timestamp = Utc::now();

        // Compute chain hash: SHA-256(previous_chain_hash || entry_data)
        let entry_data = format!(
            "{}|{}|{}|{:?}|{}|{}|{}",
            entry_id,
            director_output_hash,
            critic_assessment_hash,
            params.verdict,
            params.score,
            timestamp.to_rfc3339(),
            params.iteration
        );
        let chain_input = format!("{}{}", self.last_chain_hash, entry_data);
        let chain_hash = sha256_hex(chain_input.as_bytes());

        // Sign the chain hash with Ed25519
        let signature_bytes = self.signing_key.sign(chain_hash.as_bytes());
        let signature = hex::encode(signature_bytes.to_bytes());

        let entry = CriticAuditEntry {
            entry_id,
            director_output_hash,
            critic_assessment_hash,
            verdict: params.verdict,
            dimension_scores: params.dimension_scores,
            score: params.score,
            critic_identity: params.critic_identity,
            timestamp,
            chain_hash: chain_hash.clone(),
            signature,
            iteration: params.iteration,
        };

        self.last_chain_hash = chain_hash;
        self.entries.push(entry.clone());
        entry
    }

    /// Get all entries in the chain.
    pub fn entries(&self) -> &[CriticAuditEntry] {
        &self.entries
    }

    /// Get the verifying (public) key for this chain.
    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }

    /// Verify the integrity of the entire chain (hashes + signatures).
    pub fn verify(&self, verifying_key: &VerifyingKey) -> Result<(), AuditError> {
        verify_chain(&self.entries, verifying_key)
    }

    /// Get the number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the chain is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Verify a chain of audit entries against a verifying key.
///
/// Checks both Merkle chain integrity and Ed25519 signatures on every entry.
pub fn verify_chain(
    entries: &[CriticAuditEntry],
    verifying_key: &VerifyingKey,
) -> Result<(), AuditError> {
    let mut expected_prev_hash = sha256_hex(b"genesis");

    for (i, entry) in entries.iter().enumerate() {
        // Recompute chain hash from entry data
        let entry_data = format!(
            "{}|{}|{}|{:?}|{}|{}|{}",
            entry.entry_id,
            entry.director_output_hash,
            entry.critic_assessment_hash,
            entry.verdict,
            entry.score,
            entry.timestamp.to_rfc3339(),
            entry.iteration
        );
        let chain_input = format!("{}{}", expected_prev_hash, entry_data);
        let expected_chain_hash = sha256_hex(chain_input.as_bytes());

        if entry.chain_hash != expected_chain_hash {
            return Err(AuditError::ChainIntegrity {
                entry_index: i,
                expected: expected_chain_hash,
                found: entry.chain_hash.clone(),
            });
        }

        // Verify Ed25519 signature
        let sig_bytes =
            hex::decode(&entry.signature).map_err(|e| AuditError::InvalidSignature {
                entry_index: i,
                message: format!("hex decode failed: {}", e),
            })?;

        let sig_array: [u8; 64] =
            sig_bytes
                .as_slice()
                .try_into()
                .map_err(|_| AuditError::InvalidSignature {
                    entry_index: i,
                    message: "signature must be 64 bytes".into(),
                })?;

        let signature = Signature::from_bytes(&sig_array);

        verifying_key
            .verify(entry.chain_hash.as_bytes(), &signature)
            .map_err(|e| AuditError::InvalidSignature {
                entry_index: i,
                message: e.to_string(),
            })?;

        expected_prev_hash = entry.chain_hash.clone();
    }

    Ok(())
}

/// Compute SHA-256 and return hex-encoded string.
fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// Errors from the audit system.
#[derive(Debug, thiserror::Error)]
pub enum AuditError {
    #[error(
        "Chain integrity violation at entry {entry_index}: expected {expected}, found {found}"
    )]
    ChainIntegrity {
        entry_index: usize,
        expected: String,
        found: String,
    },

    #[error("Invalid signature at entry {entry_index}: {message}")]
    InvalidSignature { entry_index: usize, message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_signing_key() -> SigningKey {
        use rand::RngCore;
        let mut secret = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut secret);
        SigningKey::from_bytes(&secret)
    }

    #[test]
    fn test_record_and_verify() {
        let key = test_signing_key();
        let mut chain = AuditChain::new(key);

        chain.record(RecordParams {
            director_output: "The analysis shows...",
            critic_assessment: "Good analysis, approved.",
            verdict: AuditVerdict::Approved,
            dimension_scores: HashMap::new(),
            score: 0.9,
            critic_identity: AuditIdentity::Llm {
                model_id: "claude-sonnet".into(),
            },
            iteration: 1,
        });

        assert_eq!(chain.len(), 1);
        assert!(chain.verify(&chain.verifying_key()).is_ok());
    }

    #[test]
    fn test_multi_entry_chain() {
        let key = test_signing_key();
        let mut chain = AuditChain::new(key);

        for i in 0..5 {
            chain.record(RecordParams {
                director_output: &format!("Director output {}", i),
                critic_assessment: &format!("Critic review {}", i),
                verdict: if i < 4 {
                    AuditVerdict::NeedsRevision
                } else {
                    AuditVerdict::Approved
                },
                dimension_scores: {
                    let mut scores = HashMap::new();
                    scores.insert("accuracy".into(), 0.5 + (i as f64) * 0.1);
                    scores
                },
                score: 0.5 + (i as f64) * 0.1,
                critic_identity: AuditIdentity::Llm {
                    model_id: "claude-sonnet".into(),
                },
                iteration: i as u32 + 1,
            });
        }

        assert_eq!(chain.len(), 5);
        assert!(chain.verify(&chain.verifying_key()).is_ok());
    }

    #[test]
    fn test_tampered_chain_hash_detected() {
        let key = test_signing_key();
        let verifying_key = key.verifying_key();
        let mut chain = AuditChain::new(key);

        chain.record(RecordParams {
            director_output: "output 1",
            critic_assessment: "review 1",
            verdict: AuditVerdict::Approved,
            dimension_scores: HashMap::new(),
            score: 0.8,
            critic_identity: AuditIdentity::Llm {
                model_id: "test".into(),
            },
            iteration: 1,
        });
        chain.record(RecordParams {
            director_output: "output 2",
            critic_assessment: "review 2",
            verdict: AuditVerdict::Approved,
            dimension_scores: HashMap::new(),
            score: 0.9,
            critic_identity: AuditIdentity::Llm {
                model_id: "test".into(),
            },
            iteration: 2,
        });

        // Tamper with first entry's chain hash
        let mut tampered = chain.entries().to_vec();
        tampered[0].chain_hash = sha256_hex(b"tampered");

        let result = verify_chain(&tampered, &verifying_key);
        assert!(result.is_err());
        match result.unwrap_err() {
            AuditError::ChainIntegrity { entry_index, .. } => assert_eq!(entry_index, 0),
            other => panic!("Expected ChainIntegrity, got {:?}", other),
        }
    }

    #[test]
    fn test_wrong_key_rejected() {
        let key = test_signing_key();
        let wrong_key = test_signing_key();
        let mut chain = AuditChain::new(key);

        chain.record(RecordParams {
            director_output: "output",
            critic_assessment: "review",
            verdict: AuditVerdict::Approved,
            dimension_scores: HashMap::new(),
            score: 0.9,
            critic_identity: AuditIdentity::Human {
                user_id: "user-1".into(),
                name: "Alice".into(),
            },
            iteration: 1,
        });

        let result = verify_chain(chain.entries(), &wrong_key.verifying_key());
        assert!(result.is_err());
        match result.unwrap_err() {
            AuditError::InvalidSignature { entry_index, .. } => assert_eq!(entry_index, 0),
            other => panic!("Expected InvalidSignature, got {:?}", other),
        }
    }

    #[test]
    fn test_entry_serialization() {
        let key = test_signing_key();
        let mut chain = AuditChain::new(key);

        let entry = chain.record(RecordParams {
            director_output: "test output",
            critic_assessment: "test review",
            verdict: AuditVerdict::NeedsRevision,
            dimension_scores: {
                let mut m = HashMap::new();
                m.insert("accuracy".into(), 0.7);
                m.insert("completeness".into(), 0.8);
                m
            },
            score: 0.75,
            critic_identity: AuditIdentity::Llm {
                model_id: "claude-sonnet".into(),
            },
            iteration: 1,
        });

        let json = serde_json::to_string(&entry).unwrap();
        let restored: CriticAuditEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.entry_id, entry.entry_id);
        assert_eq!(restored.verdict, AuditVerdict::NeedsRevision);
        assert_eq!(restored.dimension_scores.len(), 2);
    }

    #[test]
    fn test_empty_chain_verifies() {
        let key = test_signing_key();
        let chain = AuditChain::new(key);
        assert!(chain.is_empty());
        assert!(chain.verify(&chain.verifying_key()).is_ok());
    }

    #[test]
    fn test_sha256_deterministic() {
        let hash1 = sha256_hex(b"hello world");
        let hash2 = sha256_hex(b"hello world");
        assert_eq!(hash1, hash2);
        assert_ne!(hash1, sha256_hex(b"different input"));
    }

    #[test]
    fn test_chain_order_matters() {
        let key = test_signing_key();
        let verifying_key = key.verifying_key();
        let mut chain = AuditChain::new(key);

        chain.record(RecordParams {
            director_output: "first",
            critic_assessment: "review first",
            verdict: AuditVerdict::NeedsRevision,
            dimension_scores: HashMap::new(),
            score: 0.5,
            critic_identity: AuditIdentity::Llm {
                model_id: "test".into(),
            },
            iteration: 1,
        });
        chain.record(RecordParams {
            director_output: "second",
            critic_assessment: "review second",
            verdict: AuditVerdict::Approved,
            dimension_scores: HashMap::new(),
            score: 0.9,
            critic_identity: AuditIdentity::Llm {
                model_id: "test".into(),
            },
            iteration: 2,
        });

        // Swap entries — should fail chain verification
        let mut swapped = chain.entries().to_vec();
        swapped.swap(0, 1);

        let result = verify_chain(&swapped, &verifying_key);
        assert!(result.is_err());
    }
}
