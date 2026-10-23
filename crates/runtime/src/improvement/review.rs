//! Portable Console review receipts. Trust keys come only from runtime operator
//! configuration, never from the receipt or an authenticated API caller.

use super::{digest, hash_valid, identifier, text};
use chrono::{DateTime, Utc};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReviewRequest {
    pub workspace: String,
    pub runtime: String,
    pub workflow: String,
    pub candidate: String,
    pub evaluation: String,
}
impl ReviewRequest {
    pub fn validate(&self) -> Result<(), String> {
        text(&self.workspace, 256)?;
        identifier(&self.runtime)?;
        identifier(&self.workflow)?;
        hash_valid(&self.candidate)?;
        hash_valid(&self.evaluation)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewDecision {
    pub schema_version: u32,
    pub request: ReviewRequest,
    pub approval_id: String,
    pub reviewer: String,
    pub decided_at: DateTime<Utc>,
    pub rationale: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewReceipt {
    pub decision: ReviewDecision,
    pub key_id: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewTrust {
    pub workspace: String,
    pub runtime: String,
    pub public_keys: Vec<String>,
}

fn signing_bytes(decision: &ReviewDecision) -> Result<Vec<u8>, String> {
    Ok(format!(
        "symbiont:improvement:console-approval:v1\n{}",
        digest(decision)?
    )
    .into_bytes())
}

impl ReviewReceipt {
    pub fn sign(decision: ReviewDecision, key: &SigningKey) -> Result<Self, String> {
        decision.validate()?;
        Ok(Self {
            signature: hex::encode(key.sign(&signing_bytes(&decision)?).to_bytes()),
            key_id: super::sha256(key.verifying_key().as_bytes()),
            decision,
        })
    }

    /// Verify at admission. Accepted decisions remain in the protected local
    /// approval history; transport expiry is not an instruction-version TTL.
    pub fn verify(
        &self,
        trust: &ReviewTrust,
        workflow: &str,
        now: DateTime<Utc>,
    ) -> Result<(), String> {
        self.decision.validate()?;
        if trust.public_keys.is_empty()
            || trust.public_keys.len() > 16
            || self.decision.request.workspace != trust.workspace
            || self.decision.request.runtime != trust.runtime
            || self.decision.request.workflow != workflow
        {
            return Err("Console approval scope is not trusted by this runtime".into());
        }
        if self.decision.issued_at > now + chrono::Duration::seconds(30)
            || self.decision.expires_at <= now
        {
            return Err("Console approval receipt is expired or not yet valid".into());
        }
        let mut selected = None;
        for encoded in &trust.public_keys {
            let bytes: [u8; 32] = hex::decode(encoded)
                .map_err(|_| "invalid trusted Console key")?
                .try_into()
                .map_err(|_| "invalid trusted Console key length")?;
            let key =
                VerifyingKey::from_bytes(&bytes).map_err(|_| "invalid trusted Console key")?;
            if super::sha256(&bytes) == self.key_id {
                selected = Some(key);
            }
        }
        let key = selected.ok_or("Console approval key is not explicitly trusted")?;
        let signature = Signature::from_slice(
            &hex::decode(&self.signature).map_err(|_| "invalid Console approval signature")?,
        )
        .map_err(|_| "invalid Console approval signature")?;
        key.verify_strict(&signing_bytes(&self.decision)?, &signature)
            .map_err(|_| "Console approval signature verification failed".into())
    }
}

impl ReviewDecision {
    fn validate(&self) -> Result<(), String> {
        self.request.validate()?;
        identifier(&self.approval_id)?;
        text(&self.reviewer, 512)?;
        text(&self.rationale, 4096)?;
        if self.schema_version != 1
            || self.decided_at > self.issued_at
            || self.expires_at <= self.issued_at
            || self.expires_at - self.issued_at > chrono::Duration::minutes(15)
        {
            return Err("invalid Console approval receipt validity".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn receipts_bind_identity_scope_and_exact_evidence() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let now = Utc::now();
        let receipt = ReviewReceipt::sign(
            ReviewDecision {
                schema_version: 1,
                request: ReviewRequest {
                    workspace: "regulated".into(),
                    runtime: "intake".into(),
                    workflow: "claims".into(),
                    candidate: "a".repeat(64),
                    evaluation: "b".repeat(64),
                },
                approval_id: "review123".into(),
                reviewer: "authenticated-user".into(),
                decided_at: now,
                rationale: "Acceptance evidence reviewed".into(),
                issued_at: now,
                expires_at: now + chrono::Duration::minutes(15),
            },
            &key,
        )
        .unwrap();
        let trust = ReviewTrust {
            workspace: "regulated".into(),
            runtime: "intake".into(),
            public_keys: vec![hex::encode(key.verifying_key().as_bytes())],
        };
        receipt.verify(&trust, "claims", now).unwrap();
        for field in [
            "reviewer",
            "candidate",
            "evaluation",
            "runtime",
            "workspace",
            "workflow",
        ] {
            let mut changed = receipt.clone();
            match field {
                "reviewer" => changed.decision.reviewer = "another-reviewer".into(),
                "candidate" => changed.decision.request.candidate = "c".repeat(64),
                "evaluation" => changed.decision.request.evaluation = "d".repeat(64),
                "runtime" => changed.decision.request.runtime = "other".into(),
                "workspace" => changed.decision.request.workspace = "other".into(),
                _ => changed.decision.request.workflow = "other".into(),
            }
            assert!(changed.verify(&trust, "claims", now).is_err(), "{field}");
        }
        let mut untrusted = trust.clone();
        untrusted.public_keys = vec![hex::encode(
            SigningKey::from_bytes(&[8; 32]).verifying_key().as_bytes(),
        )];
        assert!(receipt.verify(&untrusted, "claims", now).is_err());
        assert!(receipt
            .verify(&trust, "claims", now + chrono::Duration::minutes(15))
            .is_err());
        assert!(receipt
            .verify(&trust, "claims", now - chrono::Duration::minutes(1))
            .is_err());
    }
}
