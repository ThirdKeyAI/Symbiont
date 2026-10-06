//! Runtime-reported dispatch attribution, never an authorization credential.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerOrigin {
    pub agent_id: uuid::Uuid,
    pub run_id: uuid::Uuid,
    pub public_key: String,
    pub dispatch_id: uuid::Uuid,
    pub call_fingerprint: String,
    pub tool_name: String,
    pub iteration: u32,
}

impl WorkerOrigin {
    pub fn validate(&self) -> anyhow::Result<()> {
        let hash = |text: &str| text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit());
        anyhow::ensure!(
            !self.agent_id.is_nil()
                && !self.run_id.is_nil()
                && !self.dispatch_id.is_nil()
                && hash(&self.public_key)
                && self
                    .call_fingerprint
                    .strip_prefix("sha256:")
                    .is_some_and(hash)
                && !self.tool_name.is_empty()
                && self.tool_name.len() <= 256
                && !self.tool_name.chars().any(char::is_control),
            "invalid worker dispatch attribution"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn origin_metadata_is_bounded_and_rejects_invalid_references() {
        let origin = WorkerOrigin {
            agent_id: uuid::Uuid::new_v4(),
            run_id: uuid::Uuid::new_v4(),
            public_key: "a".repeat(64),
            dispatch_id: uuid::Uuid::new_v4(),
            call_fingerprint: format!("sha256:{}", "b".repeat(64)),
            tool_name: "read_file".into(),
            iteration: 1,
        };
        origin.validate().unwrap();
        let mut bad = origin.clone();
        bad.run_id = uuid::Uuid::nil();
        assert!(bad.validate().is_err());
        let mut bad = origin.clone();
        bad.public_key = "x".repeat(64);
        assert!(bad.validate().is_err());
        let mut bad = origin.clone();
        bad.call_fingerprint = "b".repeat(64);
        assert!(bad.validate().is_err());
        let mut bad = origin.clone();
        bad.tool_name = "a".repeat(257);
        assert!(bad.validate().is_err());
        let mut bad = origin;
        bad.tool_name = "a\nb".into();
        assert!(bad.validate().is_err());
    }
}
