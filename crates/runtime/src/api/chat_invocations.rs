//! Durable chat IDs share one project scope; changing caller or content cannot
//! create another execution under an existing ID. Socket history is not resumed.
use super::{
    coordinator::CoordinatorState, invocations::AuthenticatedCaller, ws_types::ServerMessage,
};
use crate::reasoning::{
    invocation::{
        lookup_invocation, open_invocation, ExistingInvocation, Invocation, OpenInvocation,
    },
    loop_types::{LoopResult, TerminationReason},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use uuid::Uuid;

pub(super) const SCOPE: &str = "ws:chat:v1";

pub(super) struct AdmittedChat {
    pub id: Uuid,
    pub content: String,
    pub invocation: Box<Invocation>,
}

impl CoordinatorState {
    fn chat_identity(caller: &AuthenticatedCaller, content: &str) -> Result<Value, String> {
        if content.len() > 64 * 1024 {
            return Err("chat input exceeds 64 KiB".into());
        }
        // The ID identifies the first accepted message in its original socket
        // context. A retry is retrieval, never a request to rerun in new history.
        Ok(json!({"version":1,"caller":caller.fingerprint(),"content":content}))
    }

    pub(super) async fn lookup_chat(
        &self,
        caller: &AuthenticatedCaller,
        id: Uuid,
        content: &str,
    ) -> Result<Option<ExistingInvocation>, String> {
        lookup_invocation(
            self.audit_project.as_ref().map_err(Clone::clone)?,
            SCOPE,
            id,
            &Self::chat_identity(caller, content)?,
        )
        .await
    }

    pub(super) async fn admit_chat(
        &self,
        caller: &AuthenticatedCaller,
        id: Uuid,
        content: &str,
    ) -> Result<OpenInvocation, String> {
        if id.is_nil() {
            return Err("chat message ID must be a non-nil UUID".into());
        }
        open_invocation(
            self.audit_project.as_ref().map_err(Clone::clone)?,
            SCOPE,
            id,
            &Self::chat_identity(caller, content)?,
            self.knowledge_agent_id,
        )
        .await
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedChat {
    version: u32,
    completed: bool,
    output: String,
    termination_reason: TerminationReason,
}

pub(super) fn saved_result(result: &LoopResult) -> Value {
    let completed = matches!(result.termination_reason, TerminationReason::Completed);
    json!(SavedChat {
        version: 1,
        completed,
        output: if completed {
            result.output.clone()
        } else {
            String::new()
        },
        termination_reason: result.termination_reason.clone(),
    })
}

pub(super) async fn send(tx: &mpsc::Sender<ServerMessage>, message: ServerMessage) -> bool {
    matches!(
        tokio::time::timeout(std::time::Duration::from_secs(2), tx.send(message)).await,
        Ok(Ok(()))
    )
}

pub(super) async fn error(
    tx: &mpsc::Sender<ServerMessage>,
    id: Uuid,
    code: &str,
    message: &str,
) -> bool {
    send(
        tx,
        ServerMessage::Error {
            request_id: Some(id.to_string()),
            code: code.into(),
            message: message.into(),
        },
    )
    .await
}

pub(super) async fn admission_error(
    tx: &mpsc::Sender<ServerMessage>,
    id: Uuid,
    detail: &str,
) -> bool {
    let (code, message) = if detail == "invocation ID conflicts with a different request" {
        (
            "INVOCATION_CONFLICT",
            "This message ID belongs to different content or a different caller; no work repeated.",
        )
    } else {
        (
            "AUDIT_UNAVAILABLE",
            "Required invocation storage is unavailable; no new work admitted.",
        )
    };
    error(tx, id, code, message).await
}

/// Return stored evidence only. In particular, a cached reply must not add
/// another user/response pair to the live socket's conversation.
pub(super) async fn existing(
    tx: &mpsc::Sender<ServerMessage>,
    id: Uuid,
    receipt: ExistingInvocation,
    replayed: bool,
) -> bool {
    let audit = match &receipt {
        ExistingInvocation::InProgress => None,
        ExistingInvocation::Unresolved { audit } => audit.clone(),
        ExistingInvocation::Recorded { audit, .. }
        | ExistingInvocation::Reconciled { audit, .. } => Some(audit.clone()),
    };
    if replayed {
        if let Some(audit) = audit {
            if !send(
                tx,
                ServerMessage::AuditOpened {
                    request_id: id.to_string(),
                    audit,
                },
            )
            .await
            {
                return false;
            }
        }
    }
    match receipt {
        ExistingInvocation::InProgress => error(tx, id, "INVOCATION_IN_PROGRESS", "The original message is queued or executing; no work repeated.").await,
        ExistingInvocation::Unresolved { .. } => error(tx, id, "INVOCATION_UNRESOLVED", "The original message outcome is unresolved. Inspect its audit and reconcile its effects; no work repeated.").await,
        ExistingInvocation::Reconciled { .. } => error(tx, id, "INVOCATION_RECONCILED", "An operator assessed the original message. Inspect its signed assessment; this ID cannot execute again.").await,
        ExistingInvocation::Recorded { result, .. } => {
            let saved = serde_json::from_value::<SavedChat>(result);
            match saved {
                Ok(saved) if saved.version == 1 && saved.completed == matches!(saved.termination_reason, TerminationReason::Completed) => {
                    if saved.completed {
                        send(tx, ServerMessage::ChatChunk { request_id: id.to_string(), content: saved.output, done: true, replayed }).await
                    } else {
                        error(tx, id, "LOOP_ERROR", "The recorded run did not complete; inspect its protected audit. No work repeated.").await
                    }
                }
                _ => error(tx, id, "AUDIT_UNAVAILABLE", "The saved chat result is invalid; no work repeated.").await,
            }
        }
    }
}
