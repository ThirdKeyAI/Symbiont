//! Fuzzy @mention, /command, and DSL keyword completion for symbi shell.

use crate::commands::registry::REGISTRY;

/// A completion candidate.
#[derive(Debug, Clone)]
pub struct Candidate {
    /// Display text (the command name, agent name, DSL keyword).
    pub display: String,
    /// Replacement text (what gets inserted).
    pub replacement: String,
    /// Fuzzy score (higher is better).
    pub score: i64,
    /// One-line summary rendered next to the display in the popup.
    /// Only populated for slash commands today.
    pub summary: Option<String>,
    /// Category tag rendered in dim text (e.g. `(authoring)`).
    pub category: Option<String>,
}

/// DSL keywords for context-aware completion in /dsl mode.
const DSL_KEYWORDS: &[(&str, &str)] = &[
    // Definition forms
    ("agent", "definition"),
    ("behavior", "definition"),
    ("function", "definition"),
    ("metadata", "block"),
    ("policy", "definition"),
    ("schedule", "definition"),
    ("channel", "definition"),
    ("memory", "definition"),
    ("webhook", "definition"),
    ("capabilities", "declaration"),
    ("with", "block"),
    // Statements
    ("let", "statement"),
    ("if", "statement"),
    ("else", "statement"),
    ("return", "statement"),
    ("for", "statement"),
    ("while", "statement"),
    ("match", "statement"),
    ("try", "statement"),
    ("catch", "statement"),
    ("emit", "statement"),
    ("require", "statement"),
    ("invoke", "statement"),
    // Types
    ("String", "type"),
    ("int", "type"),
    ("float", "type"),
    ("bool", "type"),
    // Values
    ("true", "literal"),
    ("false", "literal"),
    ("null", "literal"),
    // Policy actions
    ("allow", "policy"),
    ("deny", "policy"),
    ("audit", "policy"),
    // Security
    ("sandbox", "config"),
    ("strict", "sandbox"),
    ("moderate", "sandbox"),
    ("permissive", "sandbox"),
    ("timeout", "config"),
    // Builtins
    ("reason", "async fn"),
    ("llm_call", "async fn"),
    ("chain", "async fn"),
    ("debate", "async fn"),
    ("spawn_agent", "async fn"),
    ("ask", "async fn"),
    ("send_to", "async fn"),
    ("parallel", "async fn"),
    ("race", "async fn"),
    ("tool_call", "async fn"),
    ("print", "fn"),
    ("len", "fn"),
    ("format", "fn"),
    ("parse_json", "fn"),
];

/// Complete the current input. Returns (start_position, candidates).
///
/// When `dsl_mode` is true, bare identifiers get DSL keyword/builtin completion.
pub fn complete(
    input: &str,
    cursor: usize,
    entities: &[(String, String)],
    dsl_mode: bool,
) -> (usize, Vec<Candidate>) {
    let text = &input[..cursor];

    // /command completion (works in both modes). Fuzzy match across the
    // full registry so typing `/ex` surfaces `/exit`, `/exec`, and any
    // other command whose name contains those characters in order. The
    // popup is sorted by score so prefix matches come first.
    //
    // Strip the leading '/' so the scorer doesn't reward every entry
    // for the slash that they all share.
    if let Some(query) = text.strip_prefix('/') {
        let mut candidates: Vec<Candidate> = REGISTRY
            .iter()
            .filter_map(|entry| {
                let candidate_body = &entry.name[1..];
                let score = fuzzy_score(candidate_body, query);
                if score == 0 && !query.is_empty() {
                    return None;
                }
                Some(Candidate {
                    display: entry.name.to_string(),
                    replacement: entry.name.to_string(),
                    score,
                    summary: Some(entry.summary.to_string()),
                    category: Some(entry.category.to_string()),
                })
            })
            .collect();
        candidates.sort_by(|a, b| b.score.cmp(&a.score).then(a.display.cmp(&b.display)));
        return (0, candidates);
    }

    // @mention fuzzy completion (works in both modes)
    if let Some(at_pos) = text.rfind('@') {
        let query = &text[at_pos + 1..];
        let mut candidates: Vec<Candidate> = entities
            .iter()
            .filter_map(|(name, kind)| {
                let score = fuzzy_score(name, query);
                if score > 0 {
                    Some(Candidate {
                        display: format!("{} ({})", name, kind),
                        replacement: name.clone(),
                        score,
                        summary: None,
                        category: None,
                    })
                } else {
                    None
                }
            })
            .collect();
        candidates.sort_by_key(|c| std::cmp::Reverse(c.score));
        return (at_pos, candidates);
    }

    // DSL keyword/builtin completion (only in DSL mode)
    if dsl_mode {
        // Find the start of the current word
        let word_start = text
            .rfind(|c: char| !c.is_alphanumeric() && c != '_')
            .map(|i| i + 1)
            .unwrap_or(0);
        let prefix = &text[word_start..];

        if !prefix.is_empty() {
            let mut candidates: Vec<Candidate> = DSL_KEYWORDS
                .iter()
                .filter(|(kw, _)| kw.starts_with(prefix))
                .map(|(kw, kind)| Candidate {
                    display: format!("{} ({})", kw, kind),
                    replacement: kw.to_string(),
                    score: 100,
                    summary: None,
                    category: None,
                })
                .collect();

            // Also include matching entities
            for (name, kind) in entities {
                if name.starts_with(prefix) {
                    candidates.push(Candidate {
                        display: format!("{} ({})", name, kind),
                        replacement: name.clone(),
                        score: 90,
                        summary: None,
                        category: None,
                    });
                }
            }

            if !candidates.is_empty() {
                return (word_start, candidates);
            }
        }
    }

    (cursor, vec![])
}

/// Subsequence fuzzy scoring with bonuses for consecutive and boundary matches.
fn fuzzy_score(candidate: &str, query: &str) -> i64 {
    if query.is_empty() {
        return 1;
    }

    let candidate_lower: Vec<char> = candidate.to_lowercase().chars().collect();
    let query_lower: Vec<char> = query.to_lowercase().chars().collect();

    let mut score: i64 = 0;
    let mut ci = 0;
    let mut prev_matched = false;

    for &qc in &query_lower {
        let mut found = false;
        while ci < candidate_lower.len() {
            if candidate_lower[ci] == qc {
                score += 1;
                if prev_matched {
                    score += 2;
                }
                if ci == 0 || candidate_lower[ci - 1] == '_' {
                    score += 3;
                }
                prev_matched = true;
                ci += 1;
                found = true;
                break;
            }
            prev_matched = false;
            ci += 1;
        }
        if !found {
            return 0;
        }
    }

    if candidate_lower.starts_with(&query_lower) {
        score += 5;
    }

    score
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_completion_prefix() {
        // Slash-command completion is fuzzy, but prefix matches still
        // rank first — typing /he must put /help at the top regardless
        // of how many other commands contain "h…e" as a subsequence.
        let (pos, candidates) = complete("/he", 3, &[], false);
        assert_eq!(pos, 0);
        assert!(!candidates.is_empty());
        assert_eq!(candidates[0].replacement, "/help");
        assert_eq!(candidates[0].category.as_deref(), Some("session"));
        assert!(candidates[0].summary.is_some());
    }

    #[test]
    fn test_command_completion_fuzzy_subsequence() {
        // "/ex" is a subsequence of /exec, /exit, /extra…. Non-prefix
        // matches (e.g. via 'x' in /exec) still appear and rank below
        // the prefix matches.
        let (_, candidates) = complete("/ex", 3, &[], false);
        assert!(candidates.iter().any(|c| c.replacement == "/exit"));
        assert!(candidates.iter().any(|c| c.replacement == "/exec"));
    }

    #[test]
    fn test_command_completion_bare_slash_lists_everything() {
        let (_, candidates) = complete("/", 1, &[], false);
        // The whole registry should appear when the user has typed
        // just the slash — that's the "browse all commands" UX.
        assert!(candidates.len() >= 40);
    }

    #[test]
    fn test_at_mention_completion() {
        let entities = vec![
            ("reason".to_string(), "async fn".to_string()),
            ("spawn_agent".to_string(), "async fn".to_string()),
        ];
        let (pos, candidates) = complete("@rea", 4, &entities, false);
        assert_eq!(pos, 0);
        assert_eq!(candidates[0].replacement, "reason");
    }

    #[test]
    fn test_at_mention_mid_line() {
        let entities = vec![("MyAgent".to_string(), "agent".to_string())];
        let (pos, candidates) = complete("ask @My", 7, &entities, false);
        assert_eq!(pos, 4);
        assert_eq!(candidates[0].replacement, "MyAgent");
    }

    #[test]
    fn test_no_completion_for_bare_text_orchestrator_mode() {
        let (_, candidates) = complete("hello", 5, &[], false);
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_fuzzy_subsequence() {
        let entities = vec![("spawn_agent".to_string(), "fn".to_string())];
        let (_, candidates) = complete("@sa", 3, &entities, false);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].replacement, "spawn_agent");
    }

    #[test]
    fn test_dsl_keyword_completion() {
        let (pos, candidates) = complete("ag", 2, &[], true);
        assert_eq!(pos, 0);
        assert!(candidates.iter().any(|c| c.replacement == "agent"));
    }

    #[test]
    fn test_dsl_builtin_completion() {
        let (pos, candidates) = complete("let x = rea", 11, &[], true);
        assert_eq!(pos, 8);
        assert!(candidates.iter().any(|c| c.replacement == "reason"));
    }

    #[test]
    fn test_dsl_no_keyword_completion_in_orchestrator_mode() {
        let (_, candidates) = complete("ag", 2, &[], false);
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_dsl_type_completion() {
        let (_, candidates) = complete("Str", 3, &[], true);
        assert!(candidates.iter().any(|c| c.replacement == "String"));
    }
}
