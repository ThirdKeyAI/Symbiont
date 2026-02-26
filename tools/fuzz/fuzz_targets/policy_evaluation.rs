#![no_main]

//! Fuzz target for policy parsing and evaluation.
//!
//! Extends the existing `policy_parser` fuzz target with structured inputs
//! and semantic validation of parsed policies. Tests all rule actions,
//! malformed policies, and edge cases in the policy DSL.

use libfuzzer_sys::{arbitrary::Arbitrary, fuzz_target};
use repl_core::parse_policy;

#[derive(Arbitrary, Debug)]
struct Input {
    mode: PolicyFuzzMode,
}

#[derive(Arbitrary, Debug)]
enum PolicyFuzzMode {
    /// Raw policy text (like the existing fuzz target but with assertions).
    Raw(String),
    /// Structured valid policy.
    Structured(StructuredPolicy),
    /// Edge cases designed to break the parser.
    EdgeCase(EdgeCaseVariant),
}

#[derive(Arbitrary, Debug)]
struct StructuredPolicy {
    name: String,
    rules: Vec<StructuredRule>,
}

#[derive(Arbitrary, Debug)]
struct StructuredRule {
    action: ActionVariant,
    target: String,
    condition: Option<String>,
}

#[derive(Arbitrary, Debug)]
enum ActionVariant {
    Allow,
    Deny,
    Audit,
    Limit,
    Require,
    Apply,
    /// Invalid action to test error handling.
    Invalid(String),
}

#[derive(Arbitrary, Debug)]
enum EdgeCaseVariant {
    /// Empty string.
    Empty,
    /// Only whitespace.
    Whitespace,
    /// Only comments.
    OnlyComments,
    /// Policy with no rules.
    NoRules(String),
    /// Very deeply indented rules.
    DeepIndent { depth: u8, rule: String },
    /// Extremely long policy name.
    LongName(String),
    /// Rule with no target.
    NoTarget(String),
    /// Policy with duplicate rule actions.
    DuplicateActions { action: String, count: u8 },
    /// Mixed comment styles.
    MixedComments,
    /// Unicode in policy name and targets.
    Unicode { name: String, target: String },
}

fn clamp(mut s: String, max: usize, fallback: &str) -> String {
    // Remove newlines and control chars that would split lines in the parser.
    s.retain(|c| !c.is_control());
    if s.is_empty() {
        return fallback.to_string();
    }
    if s.len() > max {
        let mut end = max;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
    s
}

fn action_to_str(action: &ActionVariant) -> String {
    match action {
        ActionVariant::Allow => "allow".to_string(),
        ActionVariant::Deny => "deny".to_string(),
        ActionVariant::Audit => "audit".to_string(),
        ActionVariant::Limit => "limit".to_string(),
        ActionVariant::Require => "require".to_string(),
        ActionVariant::Apply => "apply".to_string(),
        ActionVariant::Invalid(s) => clamp(s.clone(), 32, "bogus_action"),
    }
}

fn structured_to_source(policy: &StructuredPolicy) -> String {
    let name = clamp(policy.name.clone(), 64, "fuzz_policy");
    let mut lines = vec![format!("policy {}", name)];

    for rule in policy.rules.iter().take(10) {
        let action = action_to_str(&rule.action);
        let target = clamp(rule.target.clone(), 64, "network");

        if let Some(ref cond) = rule.condition {
            let cond = clamp(cond.clone(), 128, "agent.tier == high");
            lines.push(format!("  {} {} when {}", action, target, cond));
        } else {
            lines.push(format!("  {} {}", action, target));
        }
    }

    lines.push("end".to_string());
    lines.join("\n")
}

fn edge_case_to_source(edge: &EdgeCaseVariant) -> String {
    match edge {
        EdgeCaseVariant::Empty => String::new(),
        EdgeCaseVariant::Whitespace => "   \n\n\t  \n  ".to_string(),
        EdgeCaseVariant::OnlyComments => {
            "# This is a comment\n// Another comment\n# Third comment".to_string()
        }
        EdgeCaseVariant::NoRules(name) => {
            let name = clamp(name.clone(), 64, "empty_policy");
            format!("policy {}\nend", name)
        }
        EdgeCaseVariant::DeepIndent { depth, rule } => {
            let d = (*depth).min(50) as usize;
            let indent = " ".repeat(d * 2);
            let rule = clamp(rule.clone(), 64, "allow network");
            format!("policy deep\n{}  {}\nend", indent, rule)
        }
        EdgeCaseVariant::LongName(name) => {
            let name = clamp(name.clone(), 1024, &"a".repeat(500));
            format!("policy {}\n  allow network\nend", name)
        }
        EdgeCaseVariant::NoTarget(action) => {
            let action = clamp(action.clone(), 32, "allow");
            format!("policy test\n  {}\nend", action)
        }
        EdgeCaseVariant::DuplicateActions { action, count } => {
            let action = clamp(action.clone(), 32, "deny");
            let c = (*count).min(20) as usize;
            let mut lines = vec!["policy dupes".to_string()];
            for i in 0..c.max(1) {
                lines.push(format!("  {} target_{}", action, i));
            }
            lines.push("end".to_string());
            lines.join("\n")
        }
        EdgeCaseVariant::MixedComments => {
            "policy mixed\n  # hash comment\n  // slash comment\n  allow network\nend".to_string()
        }
        EdgeCaseVariant::Unicode { name, target } => {
            let name = clamp(name.clone(), 64, "politica");
            let target = clamp(target.clone(), 64, "red");
            format!("policy {}\n  allow {}\nend", name, target)
        }
    }
}

fuzz_target!(|input: Input| {
    let source = match &input.mode {
        PolicyFuzzMode::Raw(s) => {
            let s = clamp(s.clone(), 8192, "");
            if s.is_empty() {
                return;
            }
            s
        }
        PolicyFuzzMode::Structured(policy) => structured_to_source(policy),
        PolicyFuzzMode::EdgeCase(edge) => edge_case_to_source(edge),
    };

    // Must never panic.
    let result = parse_policy(&source);

    // --- Semantic assertions ---

    match &input.mode {
        PolicyFuzzMode::Structured(policy) => {
            // Count how many valid rules we expect.
            let valid_rules: Vec<_> = policy.rules.iter()
                .take(10)
                .filter(|r| !matches!(r.action, ActionVariant::Invalid(_)))
                .collect();

            if valid_rules.is_empty() {
                // No valid rules — parser may accept or reject depending on
                // whether invalid actions are treated as errors.
                return;
            }

            if let Ok(parsed) = &result {
                // Parsed policy name must match (parser trims whitespace).
                let expected_name = clamp(policy.name.clone(), 64, "fuzz_policy")
                    .trim()
                    .to_string();
                assert_eq!(
                    parsed.name, expected_name,
                    "policy name must match",
                );

                // Every parsed rule must have a non-empty action and target.
                for rule in &parsed.rules {
                    assert!(!rule.action.is_empty(), "rule action must not be empty");
                    assert!(!rule.target.is_empty(), "rule target must not be empty");
                }
            }
        }

        PolicyFuzzMode::EdgeCase(edge) => {
            match edge {
                EdgeCaseVariant::Empty | EdgeCaseVariant::Whitespace => {
                    assert!(
                        result.is_err(),
                        "empty/whitespace policy must be rejected",
                    );
                }
                EdgeCaseVariant::OnlyComments => {
                    // Comments-only input doesn't start with "policy" keyword,
                    // so the parser treats it as an implicit inline policy — valid.
                }
                _ => {}
            }
        }
        _ => {}
    }
});
