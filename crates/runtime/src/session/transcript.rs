//! Hash-chained, Ed25519-signed transcript of session transitions.
//!
//! Every session state transition is recorded as an immutable entry,
//! with each entry's chain hash covering all prior entries.

use chrono::{DateTime, Utc};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Decision recorded for a session transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptDecision {
    Allowed,
    Denied,
}

/// A single entry in the session transcript chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptEntry {
    pub seq: u64,
    pub session_id: String,
    pub sender: String,
    pub recipient: String,
    pub label: String,
    pub decision: TranscriptDecision,
    pub reason: Option<String>,
    pub timestamp: DateTime<Utc>,
    /// SHA-256(prev_chain_hash || entry_data), hex-encoded.
    pub chain_hash: String,
    /// Ed25519 signature over `chain_hash.as_bytes()`, hex-encoded.
    pub signature: String,
}

/// Tamper-evident, Ed25519 hash-chained transcript of session transitions.
#[derive(Debug)]
pub struct SessionTranscript {
    entries: Vec<TranscriptEntry>,
    signing_key: SigningKey,
    last_chain_hash: String,
    seq: u64,
}

impl SessionTranscript {
    /// Create a new transcript with the given signing key.
    pub fn new(signing_key: SigningKey) -> Self {
        let genesis = sha256_hex(b"session-transcript-genesis");
        Self {
            entries: Vec::new(),
            signing_key,
            last_chain_hash: genesis,
            seq: 0,
        }
    }

    /// Create a new transcript with a freshly-generated ephemeral signing key.
    pub fn new_ephemeral() -> Self {
        let mut secret = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut secret);
        let signing_key = SigningKey::from_bytes(&secret);
        Self::new(signing_key)
    }

    /// The public verifying key for this transcript.
    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }

    /// Number of entries recorded.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the transcript is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Read-only view of all entries.
    pub fn entries(&self) -> &[TranscriptEntry] {
        &self.entries
    }

    /// Mutable access for tests (e.g., tampering checks).
    #[cfg(test)]
    pub fn entries_mut(&mut self) -> &mut Vec<TranscriptEntry> {
        &mut self.entries
    }

    /// Record a session transition and append it to the chain.
    ///
    /// Returns the new chain hash.
    pub fn record(
        &mut self,
        session_id: &str,
        sender: &str,
        recipient: &str,
        label: &str,
        decision: TranscriptDecision,
        reason: Option<String>,
    ) -> String {
        let seq = self.seq;
        self.seq += 1;
        let timestamp = Utc::now();

        let entry_data = EntryFields {
            seq,
            session_id,
            sender,
            recipient,
            label,
            decision,
            reason: reason.as_deref().unwrap_or(""),
            timestamp_rfc3339: &timestamp.to_rfc3339(),
        }
        .canonical();
        let chain_input = format!("{}{}", self.last_chain_hash, entry_data);
        let chain_hash = sha256_hex(chain_input.as_bytes());

        let signature_bytes = self.signing_key.sign(chain_hash.as_bytes());
        let signature = hex::encode(signature_bytes.to_bytes());

        let entry = TranscriptEntry {
            seq,
            session_id: session_id.to_string(),
            sender: sender.to_string(),
            recipient: recipient.to_string(),
            label: label.to_string(),
            decision,
            reason,
            timestamp,
            chain_hash: chain_hash.clone(),
            signature,
        };

        self.last_chain_hash = chain_hash.clone();
        self.entries.push(entry);
        chain_hash
    }

    /// Verify the full chain: hash continuity and Ed25519 signatures on every entry.
    ///
    /// Returns `true` iff the chain is intact.
    ///
    /// Note: this validates integrity against this transcript's own in-process key.
    /// Cross-node verification against an external or provisioned public key is
    /// deferred; a future API would accept an explicit `VerifyingKey` parameter.
    pub fn verify(&self) -> bool {
        let verifying_key = self.signing_key.verifying_key();
        let mut expected_prev_hash = sha256_hex(b"session-transcript-genesis");

        for entry in &self.entries {
            let entry_data = EntryFields {
                seq: entry.seq,
                session_id: &entry.session_id,
                sender: &entry.sender,
                recipient: &entry.recipient,
                label: &entry.label,
                decision: entry.decision,
                reason: entry.reason.as_deref().unwrap_or(""),
                timestamp_rfc3339: &entry.timestamp.to_rfc3339(),
            }
            .canonical();
            let chain_input = format!("{}{}", expected_prev_hash, entry_data);
            let expected_chain_hash = sha256_hex(chain_input.as_bytes());

            if entry.chain_hash != expected_chain_hash {
                return false;
            }

            // Verify signature — mirror critic_audit's exact pattern
            let Ok(sig_bytes) = hex::decode(&entry.signature) else {
                return false;
            };
            let Ok(sig_array) = <[u8; 64]>::try_from(sig_bytes.as_slice()) else {
                return false;
            };
            let signature = Signature::from_bytes(&sig_array);
            if verifying_key
                .verify(entry.chain_hash.as_bytes(), &signature)
                .is_err()
            {
                return false;
            }

            expected_prev_hash = entry.chain_hash.clone();
        }

        true
    }
}

/// Fields needed for canonical encoding of a transcript entry.
struct EntryFields<'a> {
    seq: u64,
    session_id: &'a str,
    sender: &'a str,
    recipient: &'a str,
    label: &'a str,
    decision: TranscriptDecision,
    reason: &'a str,
    timestamp_rfc3339: &'a str,
}

impl EntryFields<'_> {
    /// Unambiguous canonical encoding. Length-prefixing every field makes
    /// delimiter injection impossible — two different field sets can never
    /// produce the same encoding.
    fn canonical(&self) -> String {
        fn f(s: &str) -> String {
            format!("{}:{}", s.len(), s)
        }
        let decision_str = match self.decision {
            TranscriptDecision::Allowed => "allowed",
            TranscriptDecision::Denied => "denied",
        };
        format!(
            "{}|{}{}{}{}{}{}{}",
            self.seq,
            f(self.session_id),
            f(self.sender),
            f(self.recipient),
            f(self.label),
            f(decision_str),
            f(self.reason),
            f(self.timestamp_rfc3339),
        )
    }
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_verifies_chain() {
        let mut t = SessionTranscript::new_ephemeral();
        t.record("s1", "A", "B", "task", TranscriptDecision::Allowed, None);
        t.record("s1", "B", "A", "ok", TranscriptDecision::Allowed, None);
        t.record(
            "s1",
            "A",
            "C",
            "task",
            TranscriptDecision::Denied,
            Some("illegal".into()),
        );
        assert_eq!(t.len(), 3);
        assert!(t.verify(), "intact chain must verify");
    }

    #[test]
    fn tampering_breaks_verification() {
        let mut t = SessionTranscript::new_ephemeral();
        t.record("s1", "A", "B", "task", TranscriptDecision::Allowed, None);
        t.record("s1", "B", "A", "ok", TranscriptDecision::Allowed, None);
        t.entries_mut()[0].label = "tampered".into();
        assert!(!t.verify(), "tampered chain must fail verification");
    }

    #[test]
    fn pipe_in_fields_is_not_forgeable() {
        let mut a = SessionTranscript::new_ephemeral();
        a.record("s", "A|B", "C", "task", TranscriptDecision::Allowed, None);
        assert!(a.verify());
        // Tamper: swap sender/recipient so the pipe falls on the other side.
        // Without length-prefixing, sender="A|B",recipient="C" and
        // sender="A",recipient="B|C" would produce the same naive delimiter
        // string and the swap would go undetected. With length-prefixing it
        // must be caught.
        let mut b = a;
        b.entries_mut()[0].sender = "A".into();
        b.entries_mut()[0].recipient = "B|C".into();
        assert!(
            !b.verify(),
            "length-prefixed fields must make the swap detectable"
        );
    }
}
