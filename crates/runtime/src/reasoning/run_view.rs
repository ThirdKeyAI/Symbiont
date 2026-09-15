//! Bounded operator presentation of verified run evidence. No execution or
//! reconciliation authority is derived from this read-only view.

use super::{
    invocation::reconciliation::inspect_invocation,
    loop_types::{LoopEvent, TerminationReason},
    protected_journal::ProtectedJournal,
    recovery::classify,
    run_audit::RunAuditReference,
};
use crate::types::AgentId;
use serde_json::{json, Value};
use std::{collections::BTreeMap, os::unix::fs::MetadataExt, path::Path};
use uuid::Uuid;

const MAX_VIEW_RECORDS: usize = 256;
const MAX_VIEW_BYTES: usize = 2 * 1024 * 1024;
const MAX_VIEW_JOURNAL_BYTES: u64 = 16 * 1024 * 1024;

fn link(project: &Path, audit: &RunAuditReference, key: &[u8; 32]) -> Result<Value, String> {
    let filename = audit
        .path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("invalid linked audit filename")?;
    let (principal, run) = filename
        .strip_suffix(".jsonl")
        .and_then(|s| s.split_once('.'))
        .ok_or("invalid linked audit identity")?;
    let principal: AgentId = principal.parse().map_err(|_| "invalid linked principal")?;
    let run: Uuid = run.parse().map_err(|_| "invalid linked run")?;
    if run != audit.run_id
        || audit.public_key != hex::encode(key)
        || audit.path
            != project
                .join(".symbiont/governed")
                .join(format!("{principal}.{run}.jsonl"))
    {
        return Err("linked audit leaves the project or uses a different key".into());
    }
    Ok(json!({"agent_id":principal,"audit":audit}))
}

fn permission(call: &Value, sequence: u64, decision: &str) -> Value {
    let resolved = &call["resolved"];
    let mut grants = serde_json::Map::new();
    // Keep effective access and contract identity. Raw model arguments, prompt
    // text, executable argv and tool output do not belong in this summary.
    for field in [
        "command_boundary",
        "filesystem",
        "file_access",
        "execution_transport",
        "delegation_target",
        "network_policy",
    ] {
        if let Some(value) = resolved.get(field) {
            grants.insert(field.into(), value.clone());
        }
    }
    json!({"sequence":sequence,"decision":decision,"contract":call.get("contract"),
        "tool_name":call.pointer("/action/ToolCall/name"),
        "action_type":call.get("action").and_then(Value::as_object).and_then(|o| o.keys().next()),
        "target":call.pointer("/action/Delegate/target"),
        "fingerprint":call.get("fingerprint"),"source_policy":call.get("source_policy"),
        "reason":call.get("reason"),"grants":grants})
}

/// IDs are parsed by the caller; no caller-supplied filesystem path is opened.
/// The supplied public key is a trust input and should be retained independently.
pub fn inspect(
    project: &Path,
    principal: AgentId,
    run: Uuid,
    key: &[u8; 32],
) -> Result<Value, String> {
    let directory = project.join(".symbiont/governed");
    // Reject redirecting directory links without creating or modifying storage.
    for (part, forbidden_mode) in [
        (project.join(".symbiont"), 0o022),
        (directory.clone(), 0o077),
    ] {
        let meta = std::fs::symlink_metadata(part).map_err(|e| e.to_string())?;
        // SAFETY: geteuid has no memory-safety preconditions.
        if !meta.is_dir()
            || meta.uid() != unsafe { libc::geteuid() }
            || meta.mode() & forbidden_mode != 0
        {
            return Err(
                "run storage must be runtime-owned directories without links or untrusted writers"
                    .into(),
            );
        }
    }
    let path = directory.join(format!("{principal}.{run}.jsonl"));
    let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    // SAFETY: geteuid has no memory-safety preconditions.
    if !meta.is_file()
        || meta.nlink() != 1
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o077 != 0
        || meta.len() > MAX_VIEW_JOURNAL_BYTES
    {
        return Err("run view requires a private, singly linked journal of at most 16 MiB; use offline inspection for larger evidence".into());
    }
    let prefix = ProtectedJournal::verify_run_prefix(&path, key, run).map_err(|e| e.to_string())?;
    if prefix
        .entries
        .first()
        .is_none_or(|entry| entry.agent_id != principal)
    {
        return Err("run principal does not match its reference".into());
    }
    let snapshot_sha256 = prefix.snapshot_sha256.clone();
    let mut parent = None;
    let mut parent_context = None;
    let mut children = BTreeMap::new();
    let mut permissions = Vec::new();
    let mut budget = None;
    let mut budget_root = None;
    let mut terminal = None;
    let mut invocation_identity = None;
    let mut limits = Value::Null;
    let mut source = Value::Null;
    let first_record_at = prefix.entries.first().map(|e| e.timestamp);
    let last_record_at = prefix.entries.last().map(|e| e.timestamp);
    for entry in &prefix.entries {
        match &entry.event {
            LoopEvent::Started {
                config,
                execution_context,
                ..
            } => {
                limits = json!({"max_total_tokens":config.max_total_tokens,"max_iterations":config.max_iterations,
                    "timeout_seconds":config.timeout.as_secs_f64(),"delegation_depth":config.delegation_depth,"max_delegation_depth":config.max_delegation_depth});
                source = execution_context
                    .get("agent_definition")
                    .cloned()
                    .unwrap_or(Value::Null);
                invocation_identity = execution_context.get("invocation").cloned();
                if let Some(context) = execution_context.get("delegation") {
                    parent_context = Some(
                        json!({"agent_id":context.get("parent_agent_id"),"run_key":context.get("run_key"),"call_id":context.get("call_id")}),
                    );
                    if let Some(reference) = context.get("parent_audit") {
                        let reference: RunAuditReference =
                            serde_json::from_value(reference.clone()).map_err(|e| e.to_string())?;
                        parent = Some(link(project, &reference, key)?);
                    }
                }
            }
            LoopEvent::DelegationStarted {
                call_id,
                target,
                child_agent_id,
                audit,
                ..
            } => {
                if children.len() >= MAX_VIEW_RECORDS {
                    return Err("run view has too many child links; use offline inspection".into());
                }
                let mut child = link(project, audit, key)?;
                if child["agent_id"] != json!(child_agent_id) {
                    return Err("child principal differs from its audit reference".into());
                }
                child["call_id"] = json!(call_id);
                child["target"] = json!(target);
                child["parent_recorded_outcome"] = Value::Null;
                children.insert(call_id.clone(), child);
            }
            LoopEvent::DelegationFinished {
                call_id, reason, ..
            } => {
                if let Some(child) = children.get_mut(call_id) {
                    child["parent_recorded_outcome"] = json!(reason);
                }
            }
            LoopEvent::PolicyEvaluated {
                approved_calls,
                denied_calls,
                ..
            } => {
                if permissions.len() + approved_calls.len() + denied_calls.len() > MAX_VIEW_RECORDS
                {
                    return Err(
                        "run view has too many permission decisions; use offline inspection".into(),
                    );
                }
                permissions.extend(
                    approved_calls
                        .iter()
                        .map(|call| permission(call, entry.sequence, "allowed")),
                );
                permissions.extend(
                    denied_calls
                        .iter()
                        .map(|call| permission(call, entry.sequence, "denied")),
                );
            }
            LoopEvent::BudgetUpdated { budget: snapshot } => {
                budget = Some(
                    json!({"recorded_at":entry.timestamp,"sequence":entry.sequence,"snapshot":snapshot}),
                );
            }
            LoopEvent::BudgetScopeLinked {
                root_audit: Some(audit),
                ..
            } => {
                budget_root = Some(link(project, audit, key)?);
            }
            LoopEvent::Terminated {
                reason,
                iterations,
                total_usage,
                duration,
            } => {
                terminal = Some(
                    json!({"reason":reason,"iterations":iterations,"usage":total_usage,"duration_seconds":duration.as_secs_f64()}),
                );
            }
            _ => {}
        }
    }
    let report = classify(prefix, run).map_err(|e| e.to_string())?;
    if report.effects.len() > MAX_VIEW_RECORDS {
        return Err("run view has too many tracked effects; use offline inspection".into());
    }
    let status = if !report.journal_complete {
        "incomplete"
    } else if report.requires_reconciliation {
        "unknown_effects"
    } else if matches!(report.terminal_reason, Some(TerminationReason::Completed)) {
        "completed"
    } else {
        "stopped"
    };
    let mut invocation = Value::Null;
    if let Some(identity) = invocation_identity {
        if let (Some(scope), Some(id)) = (
            identity["scope"].as_str(),
            identity["id"].as_str().and_then(|s| s.parse().ok()),
        ) {
            invocation = match inspect_invocation(project, scope, id) {
                Ok(view)
                    if view.snapshot.audit.run_id == run
                        && view.snapshot.journal_sha256 == snapshot_sha256 =>
                {
                    json!({"id":id,"scope":scope,"status":view.status,"resolution":view.resolution})
                }
                Ok(_) => {
                    json!({"id":id,"scope":scope,"status":"changed","error":"invocation evidence changed during inspection; refresh before relying on the view"})
                }
                Err(error) => {
                    json!({"id":id,"scope":scope,"status":if error.contains("still owned") {"in_progress"} else {"unavailable"},"error":error})
                }
            };
        }
    }
    let view = json!({"audit":{"run_id":run,"path":path,"public_key":hex::encode(key)},"agent_id":principal,
        "snapshot_sha256":snapshot_sha256,"status":status,"first_record_at":first_record_at,"last_record_at":last_record_at,
        "recovery":report,"parent":parent,"parent_context":parent_context,"children":children.into_values().collect::<Vec<_>>(),
        "permissions":permissions,"budget":budget,"budget_root":budget_root,"terminal":terminal,"limits":limits,"source":source,"invocation":invocation});
    if serde_json::to_vec(&view).map_err(|e| e.to_string())?.len() > MAX_VIEW_BYTES {
        return Err("run view exceeds 2 MiB; use offline inspection".into());
    }
    Ok(view)
}

#[cfg(test)]
mod run_inspection_tests {
    use super::*;
    use crate::reasoning::{
        budget::SharedBudget,
        inference::Usage,
        loop_types::{JournalEntry, JournalWriter, LoopConfig},
        run_audit::open_run_journal,
    };
    use std::{collections::HashMap, io::Write, os::unix::fs::PermissionsExt};

    async fn event(writer: &dyn JournalWriter, agent: AgentId, event: LoopEvent) {
        writer
            .append(JournalEntry {
                sequence: writer.next_sequence().await,
                timestamp: chrono::Utc::now(),
                agent_id: agent,
                iteration: 0,
                event,
            })
            .await
            .unwrap();
    }
    async fn start(writer: &dyn JournalWriter, agent: AgentId, context: HashMap<String, Value>) {
        event(
            writer,
            agent,
            LoopEvent::Started {
                agent_id: agent,
                config: Box::new(LoopConfig::default()),
                execution_context: context,
            },
        )
        .await;
    }
    fn key(reference: &RunAuditReference) -> [u8; 32] {
        hex::decode(&reference.public_key)
            .unwrap()
            .try_into()
            .unwrap()
    }
    async fn finish(writer: &dyn JournalWriter, agent: AgentId) {
        event(
            writer,
            agent,
            LoopEvent::Terminated {
                reason: TerminationReason::Completed,
                iterations: 1,
                total_usage: Usage::default(),
                duration: std::time::Duration::from_secs(1),
            },
        )
        .await;
    }

    #[tokio::test]
    async fn budget_and_permissions_preserve_uncertainty_without_raw_arguments() {
        let root = tempfile::tempdir().unwrap();
        let agent = AgentId::new();
        let (writer, audit) = open_run_journal(root.path(), agent).await.unwrap();
        assert_eq!(writer.audit_reference().unwrap().run_id, audit.run_id);
        start(writer.as_ref(), agent, HashMap::new()).await;
        let budget = SharedBudget::new(1000);
        let child = budget.child(500).unwrap();
        child
            .reserve(10, 50)
            .unwrap()
            .settle(&Usage {
                prompt_tokens: 10,
                completion_tokens: 20,
                total_tokens: 30,
            })
            .unwrap();
        let mut pending = child.reserve(10, 40).unwrap();
        pending.mark_dispatched();
        drop(pending);
        event(
            writer.as_ref(),
            agent,
            LoopEvent::BudgetUpdated {
                budget: budget.snapshot(),
            },
        )
        .await;
        event(writer.as_ref(), agent, LoopEvent::PolicyEvaluated { iteration:0, action_count:1, denied_count:0,
            approved_calls:vec![json!({"action":{"ToolCall":{"name":"read","arguments":"private-input"}},
                "fingerprint":"read-fingerprint","contract":{"name":"read"},"resolved":{"argv":["private-input"],
                    "command_boundary":{"filesystem":{"read":["/workspace/input.txt"]},"container":{"memory_limit":"128m"}}}})],
            denied_calls:vec![] }).await;
        finish(writer.as_ref(), agent).await;
        let view = inspect(root.path(), agent, audit.run_id, &key(&audit)).unwrap();
        assert_eq!(view["budget"]["snapshot"]["usage"]["total_tokens"], 30);
        assert_eq!(view["budget"]["snapshot"]["uncertain_tokens"], 50);
        assert_eq!(view["budget"]["snapshot"]["available_tokens"], 920);
        assert_eq!(
            view["permissions"][0]["grants"]["command_boundary"]["filesystem"]["read"][0],
            "/workspace/input.txt"
        );
        assert!(!view.to_string().contains("private-input"));
        // An approval with no effect receipt still requires reconciliation.
        assert_eq!(view["status"], "unknown_effects");
    }

    #[tokio::test]
    async fn child_links_require_separate_verification_and_reject_foreign_paths() {
        let root = tempfile::tempdir().unwrap();
        let agent = AgentId::new();
        let child = AgentId::new();
        let (writer, audit) = open_run_journal(root.path(), agent).await.unwrap();
        let (cw, ca) = open_run_journal(root.path(), child).await.unwrap();
        start(writer.as_ref(), agent, HashMap::new()).await;
        start(
            cw.as_ref(),
            child,
            HashMap::from([(
                "delegation".into(),
                json!({"parent_agent_id":agent,"parent_audit":audit}),
            )]),
        )
        .await;
        event(
            writer.as_ref(),
            agent,
            LoopEvent::DelegationStarted {
                run_key: "run".into(),
                call_id: "call".into(),
                call_fingerprint: "hash".into(),
                target: "reader".into(),
                child_agent_id: child,
                audit: ca.clone(),
                system_prompt_hash: "hash".into(),
                message_hash: "hash".into(),
            },
        )
        .await;
        let view = inspect(root.path(), agent, audit.run_id, &key(&audit)).unwrap();
        assert_eq!(view["children"][0]["audit"]["run_id"], json!(ca.run_id));
        assert!(view["children"][0]["parent_recorded_outcome"].is_null());
        assert_eq!(
            inspect(root.path(), child, ca.run_id, &key(&ca)).unwrap()["parent"]["audit"]["run_id"],
            json!(audit.run_id)
        );
        let mut foreign = ca.clone();
        foreign.path = root
            .path()
            .join("outside")
            .join(ca.path.file_name().unwrap());
        assert!(link(root.path(), &foreign, &key(&audit)).is_err());
        std::fs::write(&ca.path, "invalid\n").unwrap();
        assert!(inspect(root.path(), agent, audit.run_id, &key(&audit)).is_ok());
        assert!(inspect(root.path(), child, ca.run_id, &key(&ca)).is_err());
    }

    #[tokio::test]
    async fn interrupted_effect_and_fragment_remain_unknown_without_modifying_evidence() {
        let root = tempfile::tempdir().unwrap();
        let agent = AgentId::new();
        let (writer, audit) = open_run_journal(root.path(), agent).await.unwrap();
        start(writer.as_ref(), agent, HashMap::new()).await;
        event(
            writer.as_ref(),
            agent,
            LoopEvent::ToolDispatchStarted {
                dispatch_id: Uuid::new_v4(),
                run_key: "run".into(),
                call_id: "call".into(),
                call_fingerprint: "hash".into(),
                tool_name: "write".into(),
            },
        )
        .await;
        drop(writer);
        std::fs::OpenOptions::new()
            .append(true)
            .open(&audit.path)
            .unwrap()
            .write_all(b"fragment")
            .unwrap();
        let before = std::fs::read(&audit.path).unwrap();
        let view = inspect(root.path(), agent, audit.run_id, &key(&audit)).unwrap();
        assert_eq!(view["status"], "incomplete");
        assert_eq!(view["recovery"]["unverified_tail_bytes"], 8);
        assert_eq!(view["recovery"]["effects"][0]["outcome"], "unknown");
        assert_eq!(std::fs::read(&audit.path).unwrap(), before);
    }

    #[tokio::test]
    async fn invalid_keys_principals_links_and_oversized_evidence_are_refused() {
        let root = tempfile::tempdir().unwrap();
        let agent = AgentId::new();
        let (writer, audit) = open_run_journal(root.path(), agent).await.unwrap();
        start(writer.as_ref(), agent, HashMap::new()).await;
        finish(writer.as_ref(), agent).await;
        assert_eq!(
            inspect(root.path(), agent, audit.run_id, &key(&audit)).unwrap()["status"],
            "completed"
        );
        assert!(inspect(root.path(), agent, audit.run_id, &[0; 32]).is_err());
        let other = AgentId::new();
        let copied = audit
            .path
            .with_file_name(format!("{other}.{}.jsonl", audit.run_id));
        std::fs::copy(&audit.path, &copied).unwrap();
        assert!(inspect(root.path(), other, audit.run_id, &key(&audit)).is_err());
        std::fs::remove_file(&copied).unwrap();
        std::os::unix::fs::symlink(&audit.path, &copied).unwrap();
        assert!(inspect(root.path(), other, audit.run_id, &key(&audit)).is_err());
        std::fs::set_permissions(&audit.path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(inspect(root.path(), agent, audit.run_id, &key(&audit)).is_err());
        std::fs::set_permissions(&audit.path, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&audit.path)
            .unwrap()
            .set_len(MAX_VIEW_JOURNAL_BYTES + 1)
            .unwrap();
        assert!(inspect(root.path(), agent, audit.run_id, &key(&audit))
            .unwrap_err()
            .contains("16 MiB"));
    }
    #[tokio::test]
    async fn operator_resolution_is_separate_from_original_incomplete_outcome() {
        use crate::reasoning::invocation::{open_invocation, reconciliation::*, OpenInvocation};
        let root = tempfile::tempdir().unwrap();
        let agent = AgentId::new();
        let id = Uuid::new_v4();
        let OpenInvocation::Fresh(owner) = open_invocation(
            root.path(),
            "fixture",
            id,
            &json!({"task":"fixture"}),
            agent,
        )
        .await
        .unwrap() else {
            panic!("expected fresh invocation")
        };
        start(owner.journal().as_ref(), agent, HashMap::new()).await;
        let audit = owner.audit().clone();
        drop(owner);
        let inspection = inspect_invocation(root.path(), "fixture", id).unwrap();
        reconcile_invocation(root.path(),"fixture",id,ResolutionReview {
            snapshot_hash:inspection.snapshot_hash,outcome:ResolutionOutcome::Failed,
            rationale:"Operator confirmed the worker stopped and reviewed the external effect.".into(),
            evidence:vec![ResolutionEvidence {reference:"operator/effect.json".into(),sha256:"a".repeat(64)}],
            effects_stopped:true,
        }).unwrap();
        let view = inspect(root.path(), agent, audit.run_id, &key(&audit)).unwrap();
        assert_eq!(view["status"], "incomplete");
        assert_eq!(view["invocation"]["status"], "reconciled");
        assert!(view["invocation"]["resolution"].is_object());
        assert_eq!(view["recovery"]["requires_reconciliation"], true);
    }
}
