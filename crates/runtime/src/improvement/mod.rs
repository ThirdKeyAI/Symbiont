//! Opt-in, operator-governed workflow instruction releases.
//!
//! Artifacts are data. They never grant tool, policy or deployment authority.
//! The local store and acceptance suite belong to the trusted runtime operator.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub mod evaluation;
#[cfg(unix)]
mod execution;
#[cfg(unix)]
mod store;
#[cfg(unix)]
pub use execution::PinnedImprovement;
#[cfg(unix)]
pub use store::{read_json, verify_document, Store};

pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_DOCUMENT_BYTES: u64 = 1024 * 1024;

pub fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Sorted JSON object keys give the public format a stable content identity.
pub fn digest<T: Serialize>(value: &T) -> Result<String, String> {
    let value = serde_json::to_value(value).map_err(|e| e.to_string())?;
    let canonical = crate::reasoning::prepared::digest_json(&value)?;
    Ok(canonical.trim_start_matches("sha256:").to_string())
}

pub(crate) fn identifier(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 80
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("identifier must contain 1–80 ASCII letters, digits, '-' or '_'".into());
    }
    Ok(())
}

pub(crate) fn hash_valid(value: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("expected a lowercase SHA-256 digest".into());
    }
    Ok(())
}

pub(crate) fn text(value: &str, max: usize) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > max || value.contains('\0') {
        return Err(format!(
            "text must be nonempty and at most {max} bytes, without NUL"
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceReference {
    pub reference: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    pub instructions: String,
    pub rationale: String,
    #[serde(default)]
    pub evidence: Vec<EvidenceReference>,
}

impl Proposal {
    pub fn validate(&self) -> Result<(), String> {
        text(&self.instructions, 32 * 1024)?;
        text(&self.rationale, 4096)?;
        if self.evidence.len() > 32 {
            return Err("at most 32 evidence references are supported".into());
        }
        for item in &self.evidence {
            text(&item.reference, 2048)?;
            hash_valid(&item.sha256)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub schema_version: u32,
    pub workflow: String,
    pub agent: String,
    pub source_sha256: String,
    pub deployment_sha256: String,
    pub suite_sha256: String,
    pub parent: Option<String>,
    pub proposal: Proposal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptanceCase {
    pub id: String,
    /// Exact CLI input bytes. The evaluator compares its hash to signed evidence.
    pub input: String,
    pub expected_output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptanceSuite {
    pub schema_version: u32,
    pub cases: Vec<AcceptanceCase>,
    pub minimum_passed: usize,
    pub maximum_tokens_per_case: u32,
    /// Denials are independent of answer quality; zero requires no denied calls.
    pub maximum_denied_calls: u32,
}

impl AcceptanceSuite {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != SCHEMA_VERSION
            || self.cases.is_empty()
            || self.cases.len() > 128
            || self.minimum_passed == 0
            || self.minimum_passed > self.cases.len()
            || self.maximum_tokens_per_case == 0
        {
            return Err("invalid acceptance suite version, case count or thresholds".into());
        }
        let mut ids = BTreeSet::new();
        let mut inputs = BTreeSet::new();
        for case in &self.cases {
            identifier(&case.id)?;
            text(&case.input, 64 * 1024)?;
            text(&case.expected_output, 64 * 1024)?;
            if !ids.insert(&case.id) || !inputs.insert(&case.input) {
                return Err("acceptance cases require unique IDs and inputs".into());
            }
        }
        if serde_json::to_vec(self).map_err(|e| e.to_string())?.len() as u64 > MAX_DOCUMENT_BYTES {
            return Err("acceptance suite exceeds document limit".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExecutionEnvironment {
    pub runtime_version: String,
    pub provider: String,
    pub model: String,
    pub provider_configuration: String,
    pub loop_config_sha256: String,
    pub system_prompt_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SelectionMode {
    Trial,
    Approved,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunBinding {
    pub schema_version: u32,
    pub workflow: String,
    pub candidate: String,
    pub approval: Option<String>,
    pub mode: SelectionMode,
    pub input_sha256: String,
    pub environment: ExecutionEnvironment,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CaseResult {
    pub id: String,
    pub run_id: uuid::Uuid,
    pub journal_sha256: String,
    pub answer_matches: bool,
    pub tokens: u32,
    pub denied_calls: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationReport {
    pub schema_version: u32,
    pub evaluator: String,
    pub candidate: String,
    pub suite_sha256: String,
    pub environment: ExecutionEnvironment,
    pub cases: Vec<CaseResult>,
    pub accepted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Approval {
    pub schema_version: u32,
    pub workflow: String,
    pub candidate: String,
    pub evaluation: String,
    pub rationale: String,
    pub operator_uid: u32,
    pub recorded_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Activation {
    pub candidate: String,
    pub approval: String,
    pub previous: Option<String>,
    pub rollback: bool,
    pub operator_uid: u32,
    pub recorded_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowState {
    pub schema_version: u32,
    pub workflow: String,
    pub enabled: bool,
    pub agent: String,
    pub source_sha256: String,
    /// Relative deployment files captured at initialization, including additions.
    pub deployment: BTreeMap<String, String>,
    pub suite: AcceptanceSuite,
    pub active: Option<String>,
    pub activations: Vec<Activation>,
    pub controls: Vec<ControlChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlChange {
    pub enabled: bool,
    pub operator_uid: u32,
    pub recorded_at: chrono::DateTime<chrono::Utc>,
}

#[cfg(test)]
mod tests;
