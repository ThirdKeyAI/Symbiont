//! AgentPin integration tests covering the wired call sites (AP-1..AP-4).
//!
//! Exercises the AgentRuntime helpers directly — no HTTP server is booted,
//! so these tests are fast and deterministic. They verify:
//! - Runtime with no AgentPin config accepts absent + present JWTs.
//! - Runtime with a MockAgentPinVerifier installed refuses missing JWT.
//! - Runtime with success-mock + matching subject accepts.
//! - Runtime with subject mismatch rejects.
//! - Runtime with failure-mock rejects.

#![cfg(feature = "http-api")]

use std::sync::Arc;

use symbi_runtime::integrations::MockAgentPinVerifier;
use symbi_runtime::{AgentId, AgentRuntime, RuntimeConfig, RuntimeError};

async fn make_runtime() -> AgentRuntime {
    AgentRuntime::new(RuntimeConfig::default())
        .await
        .expect("runtime construction")
}

fn replace_verifier(
    runtime: &mut AgentRuntime,
    verifier: Arc<dyn symbi_runtime::integrations::AgentPinVerifier>,
) {
    runtime.agentpin_verifier = Some(verifier);
}

#[tokio::test]
async fn agentpin_disabled_accepts_without_jwt() {
    let runtime = make_runtime().await;
    let agent = AgentId::new();
    assert!(runtime
        .verify_agentpin_for_agent(None, agent)
        .await
        .is_ok());
}

#[tokio::test]
async fn agentpin_disabled_accepts_jwt_but_logs_warning() {
    let runtime = make_runtime().await;
    let agent = AgentId::new();
    // When the verifier is disabled we accept any JWT (and log a warn).
    assert!(runtime
        .verify_agentpin_for_agent(Some("any.jwt"), agent)
        .await
        .is_ok());
}

#[tokio::test]
async fn agentpin_enabled_rejects_missing_jwt() {
    let mut runtime = make_runtime().await;
    replace_verifier(
        &mut runtime,
        Arc::new(MockAgentPinVerifier::new_success()),
    );
    let agent = AgentId::new();
    let err = runtime
        .verify_agentpin_for_agent(None, agent)
        .await
        .expect_err("must reject");
    assert!(matches!(err, RuntimeError::Authentication(_)));
}

#[tokio::test]
async fn agentpin_enabled_rejects_when_verifier_fails() {
    let mut runtime = make_runtime().await;
    replace_verifier(
        &mut runtime,
        Arc::new(MockAgentPinVerifier::new_failure()),
    );
    let agent = AgentId::new();
    let err = runtime
        .verify_agentpin_for_agent(Some("any.jwt"), agent)
        .await
        .expect_err("must reject");
    assert!(matches!(err, RuntimeError::Authentication(_)));
}

#[tokio::test]
async fn agentpin_enabled_rejects_subject_mismatch() {
    let mut runtime = make_runtime().await;
    replace_verifier(
        &mut runtime,
        Arc::new(MockAgentPinVerifier::with_identity(
            "someone-else".to_string(),
            "test.example.com".to_string(),
            vec![],
        )),
    );
    let agent = AgentId::new();
    let err = runtime
        .verify_agentpin_for_agent(Some("any.jwt"), agent)
        .await
        .expect_err("must reject");
    let msg = err.to_string();
    assert!(msg.contains("does not cover"), "got: {}", msg);
}

#[tokio::test]
async fn agentpin_enabled_accepts_when_subject_matches() {
    let mut runtime = make_runtime().await;
    let agent = AgentId::new();
    let expected_sub = agent.0.to_string();
    replace_verifier(
        &mut runtime,
        Arc::new(MockAgentPinVerifier::with_identity(
            expected_sub,
            "test.example.com".to_string(),
            vec![],
        )),
    );
    runtime
        .verify_agentpin_for_agent(Some("any.jwt"), agent)
        .await
        .expect("valid subject must pass");
}
