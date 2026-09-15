//! Required per-invocation audit storage for production reasoning entry points.

use super::loop_types::JournalWriter;
use crate::types::AgentId;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RunAuditReference {
    pub run_id: uuid::Uuid,
    pub path: PathBuf,
    pub public_key: String,
}

/// Open protected storage before inference or tool effects. The project root
/// comes from trusted startup configuration, never from the request payload.
pub async fn open_run_journal(
    project: &Path,
    agent_id: AgentId,
) -> Result<(Arc<dyn JournalWriter>, RunAuditReference), String> {
    #[cfg(unix)]
    {
        let parent = project.join(".symbiont/governed");
        tokio::task::spawn_blocking(move || {
            let run_id = uuid::Uuid::new_v4();
            let journal =
                super::protected_journal::ProtectedJournal::create_run(&parent, agent_id, run_id)
                    .map_err(|error| error.to_string())?;
            let reference = RunAuditReference {
                run_id,
                path: journal.path().to_owned(),
                public_key: hex::encode(journal.public_key()),
            };
            Ok((Arc::new(journal) as Arc<dyn JournalWriter>, reference))
        })
        .await
        .map_err(|error| error.to_string())?
    }
    #[cfg(not(unix))]
    {
        let _ = (project, agent_id);
        Err("protected run journals require Unix".into())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[tokio::test]
    async fn each_invocation_gets_exclusive_private_storage_and_the_same_project_key() {
        let project = tempfile::tempdir().unwrap();
        let id = AgentId::new();
        let (_, first) = open_run_journal(project.path(), id).await.unwrap();
        let (_, second) = open_run_journal(project.path(), id).await.unwrap();
        assert_ne!(first.run_id, second.run_id);
        assert_ne!(first.path, second.path);
        assert_eq!(first.public_key, second.public_key);
        assert_eq!(
            std::fs::metadata(&first.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(first.path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    #[tokio::test]
    async fn unsafe_audit_directory_is_an_error() {
        let project = tempfile::tempdir().unwrap();
        let audit = project.path().join(".symbiont/governed");
        std::fs::create_dir_all(&audit).unwrap();
        std::fs::set_permissions(&audit, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(open_run_journal(project.path(), AgentId::new())
            .await
            .is_err());
        assert_eq!(std::fs::read_dir(audit).unwrap().count(), 0);
    }
}
