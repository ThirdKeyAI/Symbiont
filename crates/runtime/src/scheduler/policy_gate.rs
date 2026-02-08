//! Policy gate for scheduled job execution.
//!
//! Before a cron-triggered agent runs, the `PolicyGate` evaluates relevant
//! policies and returns a decision: allow, deny, or require approval.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::cron_types::CronJobDefinition;

/// Decision from the policy gate about whether a scheduled job may run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SchedulePolicyDecision {
    /// The job is allowed to execute.
    Allow,
    /// The job is denied — skip this run.
    Deny { reason: String, policy_id: String },
    /// The job requires manual approval before executing.
    RequiresApproval {
        approver: String,
        reason: String,
        policy_id: String,
    },
}

/// Contextual state passed to the policy gate alongside the job definition.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScheduleContext {
    /// Number of consecutive failures for this job.
    pub consecutive_failures: u64,
    /// Total runs so far.
    pub total_runs: u64,
    /// Current system load factor (0.0 – 1.0).
    pub system_load: f64,
    /// Arbitrary key-value pairs for custom policy rules.
    pub extra: HashMap<String, String>,
}

/// A single policy rule evaluated by the gate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulePolicyRule {
    /// Unique identifier for this rule.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// The condition to evaluate.
    pub condition: SchedulePolicyCondition,
    /// What to do when the condition matches.
    pub effect: SchedulePolicyEffect,
    /// Priority (higher = evaluated first).
    pub priority: u32,
    /// Whether the rule is active.
    pub enabled: bool,
}

/// Conditions that can trigger a policy rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SchedulePolicyCondition {
    /// Job has failed more than N times consecutively.
    ConsecutiveFailures { threshold: u64 },
    /// System load exceeds a threshold.
    SystemLoadExceeds { threshold: f64 },
    /// Job matches a specific name pattern.
    JobNameMatches { pattern: String },
    /// Job has a specific policy ID in its policy_ids list.
    HasPolicy { policy_id: String },
    /// Always matches (useful for catch-all rules).
    Always,
}

/// Effect when a policy condition matches.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SchedulePolicyEffect {
    /// Allow execution.
    Allow,
    /// Deny execution with a reason.
    Deny { reason: String },
    /// Require approval from a specific role/person.
    RequireApproval { approver: String, reason: String },
}

/// The policy gate evaluates rules against a job and its context.
pub struct PolicyGate {
    rules: Vec<SchedulePolicyRule>,
    /// Default decision when no rules match.
    default_allow: bool,
}

impl PolicyGate {
    /// Create a new policy gate with the given rules.
    pub fn new(rules: Vec<SchedulePolicyRule>, default_allow: bool) -> Self {
        let mut sorted_rules = rules;
        sorted_rules.sort_by(|a, b| b.priority.cmp(&a.priority));
        Self {
            rules: sorted_rules,
            default_allow,
        }
    }

    /// Create a permissive gate that allows everything (for dev/testing).
    pub fn permissive() -> Self {
        Self {
            rules: Vec::new(),
            default_allow: true,
        }
    }

    /// Evaluate policies for a scheduled job.
    pub fn evaluate(
        &self,
        job: &CronJobDefinition,
        context: &ScheduleContext,
    ) -> SchedulePolicyDecision {
        for rule in &self.rules {
            if !rule.enabled {
                continue;
            }
            if self.condition_matches(&rule.condition, job, context) {
                return self.apply_effect(&rule.effect, &rule.id);
            }
        }

        if self.default_allow {
            SchedulePolicyDecision::Allow
        } else {
            SchedulePolicyDecision::Deny {
                reason: "No matching policy rule; default deny".to_string(),
                policy_id: "default".to_string(),
            }
        }
    }

    fn condition_matches(
        &self,
        condition: &SchedulePolicyCondition,
        job: &CronJobDefinition,
        context: &ScheduleContext,
    ) -> bool {
        match condition {
            SchedulePolicyCondition::ConsecutiveFailures { threshold } => {
                context.consecutive_failures >= *threshold
            }
            SchedulePolicyCondition::SystemLoadExceeds { threshold } => {
                context.system_load > *threshold
            }
            SchedulePolicyCondition::JobNameMatches { pattern } => {
                job.name.contains(pattern.as_str())
            }
            SchedulePolicyCondition::HasPolicy { policy_id } => job.policy_ids.contains(policy_id),
            SchedulePolicyCondition::Always => true,
        }
    }

    fn apply_effect(&self, effect: &SchedulePolicyEffect, rule_id: &str) -> SchedulePolicyDecision {
        match effect {
            SchedulePolicyEffect::Allow => SchedulePolicyDecision::Allow,
            SchedulePolicyEffect::Deny { reason } => SchedulePolicyDecision::Deny {
                reason: reason.clone(),
                policy_id: rule_id.to_string(),
            },
            SchedulePolicyEffect::RequireApproval { approver, reason } => {
                SchedulePolicyDecision::RequiresApproval {
                    approver: approver.clone(),
                    reason: reason.clone(),
                    policy_id: rule_id.to_string(),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        AgentConfig, AgentId, ExecutionMode, Priority, ResourceLimits, SecurityTier,
    };

    fn test_job(name: &str, policy_ids: Vec<String>) -> CronJobDefinition {
        let config = AgentConfig {
            id: AgentId::new(),
            name: name.to_string(),
            dsl_source: String::new(),
            execution_mode: ExecutionMode::Ephemeral,
            security_tier: SecurityTier::Tier1,
            resource_limits: ResourceLimits::default(),
            capabilities: vec![],
            policies: vec![],
            metadata: HashMap::new(),
            priority: Priority::Normal,
        };
        let mut job = CronJobDefinition::new(
            name.to_string(),
            "0 * * * *".to_string(),
            "UTC".to_string(),
            config,
        );
        job.policy_ids = policy_ids;
        job
    }

    #[test]
    fn permissive_gate_allows_all() {
        let gate = PolicyGate::permissive();
        let job = test_job("test", vec![]);
        let ctx = ScheduleContext::default();
        assert!(matches!(
            gate.evaluate(&job, &ctx),
            SchedulePolicyDecision::Allow
        ));
    }

    #[test]
    fn default_deny_when_no_rules_match() {
        let gate = PolicyGate::new(vec![], false);
        let job = test_job("test", vec![]);
        let ctx = ScheduleContext::default();
        assert!(matches!(
            gate.evaluate(&job, &ctx),
            SchedulePolicyDecision::Deny { .. }
        ));
    }

    #[test]
    fn consecutive_failures_triggers_deny() {
        let rules = vec![SchedulePolicyRule {
            id: "fail-guard".to_string(),
            name: "Failure guard".to_string(),
            condition: SchedulePolicyCondition::ConsecutiveFailures { threshold: 3 },
            effect: SchedulePolicyEffect::Deny {
                reason: "too many failures".to_string(),
            },
            priority: 100,
            enabled: true,
        }];
        let gate = PolicyGate::new(rules, true);
        let job = test_job("flaky_job", vec![]);

        // Under threshold → allow
        let ctx = ScheduleContext {
            consecutive_failures: 2,
            ..Default::default()
        };
        assert!(matches!(
            gate.evaluate(&job, &ctx),
            SchedulePolicyDecision::Allow
        ));

        // At threshold → deny
        let ctx = ScheduleContext {
            consecutive_failures: 3,
            ..Default::default()
        };
        assert!(matches!(
            gate.evaluate(&job, &ctx),
            SchedulePolicyDecision::Deny { .. }
        ));
    }

    #[test]
    fn system_load_triggers_approval() {
        let rules = vec![SchedulePolicyRule {
            id: "load-gate".to_string(),
            name: "High load gate".to_string(),
            condition: SchedulePolicyCondition::SystemLoadExceeds { threshold: 0.9 },
            effect: SchedulePolicyEffect::RequireApproval {
                approver: "ops-team".to_string(),
                reason: "system under heavy load".to_string(),
            },
            priority: 100,
            enabled: true,
        }];
        let gate = PolicyGate::new(rules, true);
        let job = test_job("report", vec![]);
        let ctx = ScheduleContext {
            system_load: 0.95,
            ..Default::default()
        };
        match gate.evaluate(&job, &ctx) {
            SchedulePolicyDecision::RequiresApproval { approver, .. } => {
                assert_eq!(approver, "ops-team");
            }
            other => panic!("expected RequiresApproval, got {:?}", other),
        }
    }

    #[test]
    fn has_policy_condition_matches() {
        let rules = vec![SchedulePolicyRule {
            id: "hipaa-check".to_string(),
            name: "HIPAA check".to_string(),
            condition: SchedulePolicyCondition::HasPolicy {
                policy_id: "hipaa_guard".to_string(),
            },
            effect: SchedulePolicyEffect::RequireApproval {
                approver: "compliance".to_string(),
                reason: "HIPAA policy requires review".to_string(),
            },
            priority: 200,
            enabled: true,
        }];
        let gate = PolicyGate::new(rules, true);

        let job_with = test_job("audit", vec!["hipaa_guard".to_string()]);
        assert!(matches!(
            gate.evaluate(&job_with, &ScheduleContext::default()),
            SchedulePolicyDecision::RequiresApproval { .. }
        ));

        let job_without = test_job("audit", vec![]);
        assert!(matches!(
            gate.evaluate(&job_without, &ScheduleContext::default()),
            SchedulePolicyDecision::Allow
        ));
    }

    #[test]
    fn disabled_rules_are_skipped() {
        let rules = vec![SchedulePolicyRule {
            id: "deny-all".to_string(),
            name: "Deny all".to_string(),
            condition: SchedulePolicyCondition::Always,
            effect: SchedulePolicyEffect::Deny {
                reason: "blocked".to_string(),
            },
            priority: 100,
            enabled: false,
        }];
        let gate = PolicyGate::new(rules, true);
        let job = test_job("test", vec![]);
        assert!(matches!(
            gate.evaluate(&job, &ScheduleContext::default()),
            SchedulePolicyDecision::Allow
        ));
    }

    #[test]
    fn higher_priority_rule_wins() {
        let rules = vec![
            SchedulePolicyRule {
                id: "allow-all".to_string(),
                name: "Allow all".to_string(),
                condition: SchedulePolicyCondition::Always,
                effect: SchedulePolicyEffect::Allow,
                priority: 50,
                enabled: true,
            },
            SchedulePolicyRule {
                id: "deny-all".to_string(),
                name: "Deny all".to_string(),
                condition: SchedulePolicyCondition::Always,
                effect: SchedulePolicyEffect::Deny {
                    reason: "global deny".to_string(),
                },
                priority: 100,
                enabled: true,
            },
        ];
        let gate = PolicyGate::new(rules, true);
        let job = test_job("test", vec![]);
        assert!(matches!(
            gate.evaluate(&job, &ScheduleContext::default()),
            SchedulePolicyDecision::Deny { .. }
        ));
    }

    #[test]
    fn decision_serialization_roundtrip() {
        let decisions = vec![
            SchedulePolicyDecision::Allow,
            SchedulePolicyDecision::Deny {
                reason: "test".to_string(),
                policy_id: "p1".to_string(),
            },
            SchedulePolicyDecision::RequiresApproval {
                approver: "admin".to_string(),
                reason: "needs review".to_string(),
                policy_id: "p2".to_string(),
            },
        ];
        for decision in &decisions {
            let json = serde_json::to_string(decision).unwrap();
            let _parsed: SchedulePolicyDecision = serde_json::from_str(&json).unwrap();
        }
    }
}
