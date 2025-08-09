use crate::error::{Result, ReplError};

pub fn parse_policy(policy: &str) -> Result<()> {
    if policy.is_empty() {
        return Err(ReplError::PolicyParsing("Empty policy".to_string()));
    }
    // For now, always succeed.
    Ok(())
}