//! Fuzzy `@`-mention completion for the Symbiont REPL.
//!
//! Supports:
//! - `@<query>` — fuzzy-match agent names, builtins, async builtins, user functions, variables
//! - `:<query>` at line start — complete REPL commands
//! - DSL keyword completion for bare identifiers

use repl_proto::{CompletionItem, CompletionKind};
use rustyline::completion::{Completer, Pair};
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use rustyline::Helper;
use std::sync::{Arc, Mutex};

/// REPL commands available for `:` completion.
const REPL_COMMANDS: &[&str] = &[
    ":help",
    ":h",
    ":agents",
    ":agent list",
    ":agent start",
    ":agent stop",
    ":agent pause",
    ":agent resume",
    ":agent destroy",
    ":agent execute",
    ":agent debug",
    ":snapshot",
    ":restore",
    ":record on",
    ":record off",
    ":memory inspect",
    ":memory compact",
    ":memory purge",
    ":webhook list",
    ":monitor stats",
    ":monitor traces",
    ":monitor report",
    ":monitor clear",
    ":clear",
    ":version",
];

/// DSL keywords available for bare-identifier completion.
const DSL_KEYWORDS: &[&str] = &[
    "agent",
    "behavior",
    "function",
    "struct",
    "let",
    "if",
    "else",
    "match",
    "for",
    "while",
    "try",
    "catch",
    "return",
    "emit",
    "require",
    "check",
    "on",
    "in",
    "invoke",
    "true",
    "false",
    "null",
    "capability",
    "capabilities",
    "policy",
    "input",
    "output",
    "steps",
];

/// Shared cache of dynamic completion items fetched from the server.
#[derive(Default)]
pub struct CompletionCache {
    pub items: Vec<CompletionItem>,
}

/// The rustyline helper that drives completion.
pub struct SymbiHelper {
    cache: Arc<Mutex<CompletionCache>>,
}

impl SymbiHelper {
    pub fn new(cache: Arc<Mutex<CompletionCache>>) -> Self {
        Self { cache }
    }
}

impl Helper for SymbiHelper {}
impl Highlighter for SymbiHelper {}
impl Hinter for SymbiHelper {
    type Hint = String;
}
impl Validator for SymbiHelper {}

impl Completer for SymbiHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let text = &line[..pos];

        // `@` prefix — fuzzy-match dynamic entities.
        // The `@` is a completion trigger only: the replacement starts AT the `@`
        // so both the `@` and the query are replaced by the entity name.
        if let Some(at_pos) = text.rfind('@') {
            let query = &text[at_pos + 1..];
            let items = self.cache.lock().unwrap();
            let mut matches: Vec<(i64, Pair)> = items
                .items
                .iter()
                .filter_map(|item| {
                    let score = fuzzy_score(&item.label, query);
                    if score > 0 {
                        let display = format_candidate(item);
                        Some((
                            score,
                            Pair {
                                display,
                                replacement: item.label.clone(),
                            },
                        ))
                    } else {
                        None
                    }
                })
                .collect();
            matches.sort_by(|a, b| b.0.cmp(&a.0));
            let pairs: Vec<Pair> = matches.into_iter().map(|(_, p)| p).collect();
            // Start replacement at the `@` itself so it gets consumed
            return Ok((at_pos, pairs));
        }

        // `:` at line start — REPL command completion
        if text.starts_with(':') {
            let pairs: Vec<Pair> = REPL_COMMANDS
                .iter()
                .filter(|cmd| cmd.starts_with(text))
                .map(|cmd| Pair {
                    display: cmd.to_string(),
                    replacement: cmd.to_string(),
                })
                .collect();
            return Ok((0, pairs));
        }

        // Bare identifier — keyword + cached entity prefix match
        if let Some(word_start) = text.rfind(|c: char| !c.is_alphanumeric() && c != '_') {
            let prefix = &text[word_start + 1..];
            if prefix.is_empty() {
                return Ok((pos, vec![]));
            }
            let start = word_start + 1;

            let mut pairs: Vec<Pair> = DSL_KEYWORDS
                .iter()
                .filter(|kw| kw.starts_with(prefix))
                .map(|kw| Pair {
                    display: kw.to_string(),
                    replacement: kw.to_string(),
                })
                .collect();

            // Also include cached entities for prefix match
            if let Ok(items) = self.cache.lock() {
                for item in &items.items {
                    if item.label.starts_with(prefix) {
                        pairs.push(Pair {
                            display: format_candidate(item),
                            replacement: item.label.clone(),
                        });
                    }
                }
            }

            return Ok((start, pairs));
        }

        // Line is a bare word from position 0
        let prefix = text;
        if prefix.is_empty() {
            return Ok((pos, vec![]));
        }

        let mut pairs: Vec<Pair> = DSL_KEYWORDS
            .iter()
            .filter(|kw| kw.starts_with(prefix))
            .map(|kw| Pair {
                display: kw.to_string(),
                replacement: kw.to_string(),
            })
            .collect();

        if let Ok(items) = self.cache.lock() {
            for item in &items.items {
                if item.label.starts_with(prefix) {
                    pairs.push(Pair {
                        display: format_candidate(item),
                        replacement: item.label.clone(),
                    });
                }
            }
        }

        Ok((0, pairs))
    }
}

/// Format a completion candidate with its kind tag for display.
fn format_candidate(item: &CompletionItem) -> String {
    let tag = match item.kind {
        CompletionKind::Agent => "agent",
        CompletionKind::Builtin => "fn",
        CompletionKind::AsyncBuiltin => "async fn",
        CompletionKind::Function => "fn",
        CompletionKind::Variable => "var",
        CompletionKind::Keyword => "kw",
        CompletionKind::ReplCommand => "cmd",
    };
    format!("{} ({})", item.label, tag)
}

/// Simple subsequence fuzzy scoring.
///
/// Returns a positive score if every character in `query` appears in `candidate`
/// in order, with bonuses for consecutive and word-boundary matches.
/// Returns 0 for no match.
fn fuzzy_score(candidate: &str, query: &str) -> i64 {
    if query.is_empty() {
        return 1; // empty query matches everything
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
                // Bonus for consecutive match
                if prev_matched {
                    score += 2;
                }
                // Bonus for word-boundary match (start or after _ / uppercase transition)
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
            return 0; // query char not found — no match
        }
    }

    // Bonus for exact prefix match
    if candidate_lower.starts_with(&query_lower) {
        score += 5;
    }

    score
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fuzzy_score_exact() {
        assert!(fuzzy_score("reason", "reason") > 0);
    }

    #[test]
    fn test_fuzzy_score_prefix() {
        let full = fuzzy_score("reason", "reason");
        let prefix = fuzzy_score("reason", "rea");
        assert!(full > prefix);
        assert!(prefix > 0);
    }

    #[test]
    fn test_fuzzy_score_subsequence() {
        assert!(fuzzy_score("spawn_agent", "sa") > 0);
        assert!(fuzzy_score("spawn_agent", "spag") > 0);
    }

    #[test]
    fn test_fuzzy_score_no_match() {
        assert_eq!(fuzzy_score("reason", "xyz"), 0);
    }

    #[test]
    fn test_fuzzy_score_case_insensitive() {
        assert!(fuzzy_score("MyAgent", "myag") > 0);
    }

    #[test]
    fn test_fuzzy_score_empty_query() {
        assert!(fuzzy_score("anything", "") > 0);
    }

    #[test]
    fn test_format_candidate() {
        let item = CompletionItem {
            label: "reason".to_string(),
            kind: CompletionKind::AsyncBuiltin,
        };
        assert_eq!(format_candidate(&item), "reason (async fn)");
    }
}
