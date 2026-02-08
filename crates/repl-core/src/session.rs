use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
pub struct Session {
    pub variables: HashMap<String, String>,
    pub rng_seed: u64,
    // We will add clock state later
}

/// Session snapshot for state management
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub data: serde_json::Value,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Self {
        Self {
            variables: HashMap::new(),
            rng_seed: 0,
        }
    }

    pub fn snapshot(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    pub fn restore(snapshot: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(snapshot)
    }
}
