use crate::error::{ReplError, Result};

/// A parsed policy with validated structure.
#[derive(Debug, Clone)]
pub struct ParsedPolicy {
    /// Policy name or identifier.
    pub name: String,
    /// Individual rules extracted from the policy text.
    pub rules: Vec<PolicyRule>,
}

/// A single rule within a policy.
#[derive(Debug, Clone)]
pub struct PolicyRule {
    /// The action this rule governs (e.g., "allow", "deny", "audit").
    pub action: String,
    /// The resource or scope the rule applies to.
    pub target: String,
    /// Optional condition expression.
    pub condition: Option<String>,
}

/// Parse and validate a policy definition string.
///
/// The expected format is line-oriented:
/// ```text
/// policy <name>
///   <action> <target> [when <condition>]
///   ...
/// end
/// ```
///
/// Returns a `ParsedPolicy` on success, or a descriptive error.
pub fn parse_policy(policy: &str) -> Result<ParsedPolicy> {
    if policy.is_empty() {
        return Err(ReplError::PolicyParsing("Empty policy".to_string()));
    }

    let lines: Vec<&str> = policy
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return Err(ReplError::PolicyParsing(
            "Policy contains only whitespace".to_string(),
        ));
    }

    // Extract policy name from first line
    let first_line = lines[0];
    let name = if let Some(stripped) = first_line.strip_prefix("policy") {
        let name = stripped.trim();
        if name.is_empty() {
            return Err(ReplError::PolicyParsing(
                "Policy name missing after 'policy' keyword".to_string(),
            ));
        }
        name.to_string()
    } else {
        // If there's no "policy" keyword, treat the entire input as a
        // single-rule implicit policy (backwards-compatible with simple strings).
        return Ok(ParsedPolicy {
            name: "inline".to_string(),
            rules: vec![PolicyRule {
                action: "apply".to_string(),
                target: policy.to_string(),
                condition: None,
            }],
        });
    };

    let mut rules = Vec::new();
    for line in &lines[1..] {
        if *line == "end" {
            break;
        }

        // Skip comment lines
        if line.starts_with('#') || line.starts_with("//") {
            continue;
        }

        let rule = parse_rule_line(line)?;
        rules.push(rule);
    }

    if rules.is_empty() {
        return Err(ReplError::PolicyParsing(format!(
            "Policy '{}' has no rules",
            name
        )));
    }

    Ok(ParsedPolicy { name, rules })
}

/// Parse a single rule line: `<action> <target> [when <condition>]`
fn parse_rule_line(line: &str) -> Result<PolicyRule> {
    // Split on "when" to extract optional condition
    let (main_part, condition) = if let Some(idx) = line.find(" when ") {
        let (main, cond) = line.split_at(idx);
        (main.trim(), Some(cond[5..].trim().to_string())) // skip " when"
    } else {
        (line, None)
    };

    let mut parts = main_part.splitn(2, ' ');
    let action = parts
        .next()
        .ok_or_else(|| ReplError::PolicyParsing(format!("Empty rule line: '{}'", line)))?
        .to_string();

    let target = parts
        .next()
        .ok_or_else(|| {
            ReplError::PolicyParsing(format!(
                "Rule '{}' missing target (expected '<action> <target>')",
                action
            ))
        })?
        .to_string();

    // Validate action is a known keyword
    let valid_actions = ["allow", "deny", "audit", "limit", "require", "apply"];
    if !valid_actions.contains(&action.as_str()) {
        return Err(ReplError::PolicyParsing(format!(
            "Unknown policy action '{}'; expected one of: {}",
            action,
            valid_actions.join(", ")
        )));
    }

    Ok(PolicyRule {
        action,
        target,
        condition,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_policy_fails() {
        assert!(parse_policy("").is_err());
    }

    #[test]
    fn whitespace_only_fails() {
        assert!(parse_policy("   \n  \n  ").is_err());
    }

    #[test]
    fn simple_string_becomes_inline_policy() {
        let result = parse_policy("no_external_calls").unwrap();
        assert_eq!(result.name, "inline");
        assert_eq!(result.rules.len(), 1);
        assert_eq!(result.rules[0].action, "apply");
        assert_eq!(result.rules[0].target, "no_external_calls");
    }

    #[test]
    fn structured_policy_parses() {
        let input = r#"
policy hipaa_guard
  deny network_access when patient_data
  allow read /approved/*
  audit all_operations
end
"#;
        let result = parse_policy(input).unwrap();
        assert_eq!(result.name, "hipaa_guard");
        assert_eq!(result.rules.len(), 3);

        assert_eq!(result.rules[0].action, "deny");
        assert_eq!(result.rules[0].target, "network_access");
        assert_eq!(result.rules[0].condition.as_deref(), Some("patient_data"));

        assert_eq!(result.rules[1].action, "allow");
        assert_eq!(result.rules[1].target, "read /approved/*");
        assert!(result.rules[1].condition.is_none());

        assert_eq!(result.rules[2].action, "audit");
        assert_eq!(result.rules[2].target, "all_operations");
    }

    #[test]
    fn missing_policy_name_fails() {
        assert!(parse_policy("policy\n  allow all\nend").is_err());
    }

    #[test]
    fn no_rules_fails() {
        assert!(parse_policy("policy empty\nend").is_err());
    }

    #[test]
    fn unknown_action_fails() {
        let input = "policy test\n  explode everything\nend";
        assert!(parse_policy(input).is_err());
    }

    #[test]
    fn comments_are_skipped() {
        let input = r#"
policy test
  # This is a comment
  allow read
  // Another comment
  deny write
end
"#;
        let result = parse_policy(input).unwrap();
        assert_eq!(result.rules.len(), 2);
    }

    #[test]
    fn condition_parsing() {
        let input = "policy gate\n  limit api_calls when rate > 100\nend";
        let result = parse_policy(input).unwrap();
        assert_eq!(result.rules[0].condition.as_deref(), Some("rate > 100"));
    }
}
