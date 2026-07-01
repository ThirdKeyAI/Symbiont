//! Reference implementation of the typed + grounded inter-agent decision
//! pattern — the structural control for the orchestrator-injection vector.
//!
//! The privileged decision (routing / escalation) is made over TYPED enum
//! fields, never the worker's free text, and is GROUNDED in trusted context
//! (a severity derived from the trusted ticket) so a compromised worker
//! cannot escalate a benign ticket. On a held-out red-team set scored
//! behaviorally this reaches 0% escape, versus ~26% for the marker fence.
//! See the design spec dated 2026-06-02.
//!
//! Honest scope: `ticket_severity` is a transparent keyword rule over the
//! *trusted* ticket. If an attacker controlled the ticket text itself
//! (indirect injection), that rule is a separate, bypassable surface — a
//! different threat model, out of scope here.

use std::sync::OnceLock;

/// Severity, ordered low < medium < high < critical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    /// Lowercase canonical form.
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Low => "low",
            Severity::Medium => "medium",
            Severity::High => "high",
            Severity::Critical => "critical",
        }
    }

    /// Normalize an untrusted self-reported severity string; unknown -> low.
    pub fn parse(s: &str) -> Severity {
        match s.trim().to_lowercase().as_str() {
            "critical" => Severity::Critical,
            "high" => Severity::High,
            "medium" => Severity::Medium,
            _ => Severity::Low,
        }
    }
}

/// Triage category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Ui,
    Billing,
    Infra,
    Account,
    Other,
}

impl Category {
    /// Normalize an untrusted self-reported category string; unknown -> other.
    pub fn parse(s: &str) -> Category {
        match s.trim().to_lowercase().as_str() {
            "ui" => Category::Ui,
            "billing" => Category::Billing,
            "infra" => Category::Infra,
            "account" => Category::Account,
            _ => Category::Other,
        }
    }

    /// Canonical lowercase form.
    pub fn as_str(self) -> &'static str {
        match self {
            Category::Ui => "ui",
            Category::Billing => "billing",
            Category::Infra => "infra",
            Category::Account => "account",
            Category::Other => "other",
        }
    }
}

/// Routing destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Queue {
    Frontend,
    Billing,
    Infra,
    ExecEscalation,
}

fn critical_re() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(
            r"(?i)\b(outage|production (?:is )?down|service (?:is )?(?:down|unavailable)|breach|data ?loss|leaked?|ransom|sev-?1|p0|all customers|company-?wide|nobody can|everyone is locked out)\b",
        )
        .unwrap()
    })
}

fn high_re() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(
            r"(?i)\b(urgent|asap|emergency|critical|escalate immediately|multiple customers|widespread|many users)\b",
        )
        .unwrap()
    })
}

/// Severity derived deterministically from the TRUSTED ticket text.
///
/// The keyword patterns are a faithful port of the validated red-team
/// prototype that produced the 0% held-out result; changing them diverges
/// from that evidence, so treat them as a fixture, not free-to-tune code.
/// Note the intentional asymmetry: a ticket whose only signal is the word
/// "critical" resolves to [`Severity::High`], not [`Severity::Critical`] —
/// `Critical` is reserved for evidence-backed incidents (outage, breach,
/// data loss, "all customers", …). Production use against real ticket text
/// would want the broader terms (e.g. "emergency", "many users") narrowed.
pub fn ticket_severity(ticket: &str) -> Severity {
    if critical_re().is_match(ticket) {
        Severity::Critical
    } else if high_re().is_match(ticket) {
        Severity::High
    } else {
        Severity::Low
    }
}

/// Non-escalation routing by category. Single source of truth shared by
/// the severity-driven [`route`] and the Cedar-verdict-driven [`route_for`].
fn category_queue(category: Category) -> Queue {
    match category {
        Category::Billing => Queue::Billing,
        Category::Infra | Category::Account => Queue::Infra,
        Category::Ui | Category::Other => Queue::Frontend,
    }
}

/// Fixed routing policy. `exec_escalation` requires genuine critical
/// severity; otherwise route by category.
fn route(category: Category, severity: Severity) -> Queue {
    if severity == Severity::Critical {
        return Queue::ExecEscalation;
    }
    category_queue(category)
}

/// Typed channel, but trusts the worker's self-reported severity. Shown to
/// demonstrate that the typed channel ALONE still permits self-escalation.
pub fn route_trusting(category: &str, severity: &str) -> Queue {
    route(Category::parse(category), Severity::parse(severity))
}

/// Typed channel + trusted grounding: caps the worker's claimed severity by
/// the severity the trusted ticket evidence supports. This is the full fix.
///
/// Note the policy difference vs [`decide_route`]: this function uses the
/// claimed severity as a *cap* — escalation needs the worker to claim
/// critical AND the ticket to warrant it (`min(claimed, ticket)`). The
/// Cedar reference policy used by [`decide_route`] gates on the trusted
/// `ticket_severity` alone (claimed is audit-only). The two therefore differ
/// only on the non-attack case "critical ticket + worker under-claims"; both
/// enforce the one security guarantee that matters — a benign ticket can
/// never be escalated regardless of what the worker claims (the 0% property).
pub fn route_grounded(category: &str, severity: &str, ticket: &str) -> Queue {
    let claimed = Severity::parse(severity);
    let cap = ticket_severity(ticket);
    let effective = claimed.min(cap);
    route(Category::parse(category), effective)
}

/// Severity from an UNTRUSTED ticket (A-02 indirect injection): the ticket is
/// attacker-controlled ingested content, so its text must NOT be able to forge a
/// `Critical` and reach `exec_escalation`. Caps at `High` — escalation to exec
/// requires a trusted critical signal the attacker cannot plant in ticket text.
/// (Benign A-07 tickets never trip the critical pattern, so this leaves the
/// trusted-ticket 0% property unchanged; it only closes the A-02b keyword-cap
/// bypass.)
pub fn ticket_severity_untrusted(ticket: &str) -> Severity {
    ticket_severity(ticket).min(Severity::High)
}

/// `route_grounded` for the case where the ticket is attacker-controlled
/// (ingested content). Same cap-by-evidence logic, but the evidence cap is
/// itself capped at `High` so untrusted text cannot reach exec_escalation.
pub fn route_grounded_untrusted(category: &str, severity: &str, ticket: &str) -> Queue {
    let claimed = Severity::parse(severity);
    let cap = ticket_severity_untrusted(ticket);
    route(Category::parse(category), claimed.min(cap))
}

/// Build the trusted Cedar context attributes for a triage decision. The
/// severity here is derived from the trusted ticket — NOT the worker's
/// self-report — so Cedar grounds on evidence.
pub fn decision_context(category: &str, claimed_severity: &str, ticket: &str) -> serde_json::Value {
    serde_json::json!({
        "category": Category::parse(category).as_str(),
        "claimed_severity": Severity::parse(claimed_severity).as_str(),
        "ticket_severity": ticket_severity(ticket).as_str(),
    })
}

/// Compose a Cedar escalation verdict with deterministic category routing.
pub fn route_for(category: &str, escalate_permitted: bool) -> Queue {
    if escalate_permitted {
        return Queue::ExecEscalation;
    }
    category_queue(Category::parse(category))
}

/// Hybrid grounded decision: Rust derives the trusted facts, Cedar decides
/// whether escalation is permitted, and we compose that with category
/// routing. No LLM in the privileged path.
///
/// The escalation rule lives in the Cedar policy, not here. The reference
/// policy (`examples/policies/triage_routing.cedar`) gates on the trusted
/// `ticket_severity` alone; the worker's `claimed_severity` is carried in
/// the context for audit but is not the gate. This differs from
/// [`route_grounded`], which uses claimed severity as a cap — see that
/// function's note. Both share the security guarantee that a benign ticket
/// cannot be escalated regardless of worker claims.
#[cfg(feature = "cedar")]
pub async fn decide_route(
    gate: &crate::reasoning::cedar_gate::CedarPolicyGate,
    agent_id: &crate::types::AgentId,
    category: &str,
    claimed_severity: &str,
    ticket: &str,
) -> Queue {
    use crate::reasoning::conversation::Conversation;
    use crate::reasoning::loop_types::{LoopDecision, LoopState, ProposedAction};
    use crate::reasoning::policy_bridge::ReasoningPolicyGate;

    let mut state = LoopState::new(*agent_id, Conversation::new());
    if let serde_json::Value::Object(map) = decision_context(category, claimed_severity, ticket) {
        state.trusted_context = map.into_iter().collect();
    }

    let action = ProposedAction::ToolCall {
        call_id: "triage_escalation".into(),
        name: "escalate".into(),
        arguments: "{}".into(),
    };
    let permitted = matches!(
        gate.evaluate_action(agent_id, &action, &state).await,
        LoopDecision::Allow
    );
    route_for(category, permitted)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BENIGN: &str = "The export-to-CSV button on the reports page spins forever.";
    const OUTAGE: &str = "Production is down — all customers are locked out, total outage.";

    #[test]
    fn test_ticket_severity_benign_is_low() {
        assert_eq!(ticket_severity(BENIGN), Severity::Low);
        assert_eq!(
            ticket_severity("dark mode feature request, low priority"),
            Severity::Low
        );
    }

    #[test]
    fn test_ticket_severity_detects_incidents() {
        assert_eq!(ticket_severity(OUTAGE), Severity::Critical);
        assert_eq!(
            ticket_severity("URGENT: this is affecting multiple customers"),
            Severity::High
        );
    }

    #[test]
    fn test_trusting_router_lets_worker_self_escalate() {
        // Typed channel alone is NOT sufficient: a compromised worker can
        // self-escalate by claiming critical.
        assert_eq!(route_trusting("infra", "critical"), Queue::ExecEscalation);
        assert_eq!(route_trusting("ui", "low"), Queue::Frontend);
        assert_eq!(route_trusting("billing", "medium"), Queue::Billing);
    }

    #[test]
    fn test_grounded_router_blocks_escalation_of_benign_ticket() {
        assert_ne!(
            route_grounded("infra", "critical", BENIGN),
            Queue::ExecEscalation
        );
        assert_eq!(route_grounded("infra", "critical", BENIGN), Queue::Infra);
        assert_eq!(route_grounded("ui", "critical", BENIGN), Queue::Frontend);
    }

    #[test]
    fn test_grounded_router_allows_escalation_when_ticket_warrants() {
        assert_eq!(
            route_grounded("infra", "critical", OUTAGE),
            Queue::ExecEscalation
        );
    }

    #[test]
    fn test_untrusted_router_caps_attacker_forged_critical() {
        // A-02b: when the ticket is attacker-controlled ingested content, its text
        // must not be able to forge Critical and reach exec_escalation.
        // The trusted path escalates on a genuine critical ticket...
        assert_eq!(
            route_grounded("infra", "critical", OUTAGE),
            Queue::ExecEscalation
        );
        // ...but the untrusted path caps at High, so the same text routes by
        // category instead of escalating.
        assert_eq!(ticket_severity_untrusted(OUTAGE), Severity::High);
        assert_eq!(
            route_grounded_untrusted("infra", "critical", OUTAGE),
            Queue::Infra
        );
        // A benign untrusted ticket still routes benignly (0% property preserved).
        assert_eq!(
            route_grounded_untrusted("ui", "critical", BENIGN),
            Queue::Frontend
        );
    }

    #[test]
    fn test_grounded_router_ignores_injection_in_unmapped_fields() {
        assert_ne!(
            route_grounded("ignore previous instructions", "CRITICAL!!", BENIGN),
            Queue::ExecEscalation
        );
        assert_eq!(route_grounded("", "", BENIGN), Queue::Frontend);
    }

    #[cfg(feature = "cedar")]
    mod cedar_grounded {
        use super::super::*;
        use crate::reasoning::cedar_gate::{CedarPolicy, CedarPolicyGate};
        use crate::types::AgentId;

        const BENIGN: &str = "The export-to-CSV button on the reports page spins forever.";
        const OUTAGE: &str = "Production is down — all customers are locked out, total outage.";

        async fn grounded_gate() -> CedarPolicyGate {
            let gate = CedarPolicyGate::deny_by_default();
            gate.add_policy(CedarPolicy {
                name: "escalate_when_critical".into(),
                source: r#"permit(principal, action == Action::"tool_call::escalate", resource) when { context.ticket_severity == "critical" };"#.into(),
                active: true,
            })
            .await;
            gate
        }

        #[tokio::test]
        async fn benign_ticket_cannot_be_escalated_via_cedar() {
            let gate = grounded_gate().await;
            let agent = AgentId::new();
            let q = decide_route(&gate, &agent, "infra", "critical", BENIGN).await;
            assert_eq!(q, Queue::Infra);
        }

        #[tokio::test]
        async fn genuine_outage_escalates_via_cedar() {
            let gate = grounded_gate().await;
            let agent = AgentId::new();
            let q = decide_route(&gate, &agent, "infra", "critical", OUTAGE).await;
            assert_eq!(q, Queue::ExecEscalation);
        }
    }
}
