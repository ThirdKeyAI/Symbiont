//! Bounded operations for trusted operator interfaces. Creation, proposal and
//! trial collection remain explicit local actions; APIs never auto-enable them.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Inspection {
    pub candidate: Option<String>,
    pub evaluation: Option<String>,
    pub approval: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Inspect {
        #[serde(flatten)]
        selection: Inspection,
    },
    Approve {
        receipt: Box<super::review::ReviewReceipt>,
    },
    Activate {
        candidate: String,
        approval: String,
        expected_active: Option<String>,
        rollback: bool,
    },
    SetEnabled {
        enabled: bool,
    },
}

#[cfg(unix)]
pub async fn execute(
    project: &std::path::Path,
    workflow: String,
    operation: Operation,
) -> Result<serde_json::Value, String> {
    use serde_json::json;
    static OPERATIONS: std::sync::OnceLock<std::sync::Arc<tokio::sync::Semaphore>> =
        std::sync::OnceLock::new();
    let permit = OPERATIONS
        .get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::new(2)))
        .clone()
        .try_acquire_owned()
        .map_err(|_| "improvement administration is busy")?;
    let project = project.to_owned();
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let mut store = super::Store::open(&project, &workflow)?;
        match operation {
            Operation::Inspect { selection } => {
                let mut value =
                    json!({"workflow": store.state(), "public_key": store.public_key()});
                if let Some(id) = selection.candidate {
                    value["candidate"] = json!(store.candidate(&id)?);
                }
                if let Some(id) = selection.evaluation {
                    value["evaluation"] = json!(store.evaluation(&id)?);
                }
                if let Some(id) = selection.approval {
                    value["approval"] = store.export_document("approval", Some(&id))?;
                }
                Ok(value)
            }
            Operation::Approve { receipt } => {
                Ok(json!({"approval": store.approve_receipt(*receipt)?}))
            }
            Operation::Activate {
                candidate,
                approval,
                expected_active,
                rollback,
            } => {
                store.activate(&candidate, &approval, expected_active.as_deref(), rollback)?;
                Ok(json!({"workflow": store.state()}))
            }
            Operation::SetEnabled { enabled } => {
                store.set_enabled(enabled)?;
                Ok(json!({"workflow": store.state()}))
            }
        }
    })
    .await
    .map_err(|e| e.to_string())?
}
