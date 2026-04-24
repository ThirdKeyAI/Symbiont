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
        if query.is_empty() {
            // Bare `/` — user is browsing. Group by category and sort
            // alphabetically within each group so the list reads like a
            // table of contents rather than a flat blob.
            candidates.sort_by(|a, b| a.category.cmp(&b.category).then(a.display.cmp(&b.display)));
        } else {
            // Filtered — prioritise fuzzy rank so prefix/subsequence
            // hits surface first regardless of category.
            candidates.sort_by(|a, b| b.score.cmp(&a.score).then(a.display.cmp(&b.display)));
        }
        return (0, candidates);
    }

    // @mention fuzzy completion (works in both modes).
    //
    // Two sources, merged and ranked together:
    // - registered entities (agents / tools / etc.) — cheap, in-memory
    // - filesystem entries starting under the cwd — lets `@src/mo` suggest
    //   `src/mod.rs` / `src/modules/` without the user leaving the shell.
    //
    // Filesystem suggestion is only triggered when the query contains
    // a `/` (explicit path-like intent) or when no entity fuzzy-matches,
    // so typing `@spa` for "spawn_agent" still returns the entity first
    // without a scan of every file in cwd.
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

        let wants_files = query.contains('/') || candidates.is_empty();
        if wants_files {
            candidates.extend(file_path_candidates(query));
        }

        candidates.sort_by_key(|b| std::cmp::Reverse(b.score));
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

/// Maximum number of directory entries scanned per completion call.
/// Bounds the cost of `@src/` in a large crate without losing the
/// common case where the user is narrowing by prefix.
const FILE_SCAN_LIMIT: usize = 500;

/// Maximum number of matching file candidates returned.
const FILE_RESULT_LIMIT: usize = 20;

/// Produce filesystem completion candidates for an `@`-triggered query.
///
/// The query is split into a directory prefix and a final fragment:
/// - `src/mod` → directory `src/`, fragment `mod`
/// - `foo`    → directory `./`, fragment `foo`
/// - `~/.cfg` → directory `$HOME/`, fragment `.cfg`
///
/// Directories are suggested with a trailing `/` so the user can
/// continue typing (Tab-cycle deeper) without leaving `@` context.
fn file_path_candidates(query: &str) -> Vec<Candidate> {
    let (dir_part, fragment) = split_dir_fragment(query);
    let dir = resolve_dir(&dir_part);

    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut results: Vec<Candidate> = Vec::new();
    for entry in entries.take(FILE_SCAN_LIMIT).flatten() {
        let file_name = entry.file_name().to_string_lossy().to_string();
        // Hide dotfiles unless the user's fragment starts with '.'.
        if file_name.starts_with('.') && !fragment.starts_with('.') {
            continue;
        }
        let score = fuzzy_score(&file_name, &fragment);
        if score == 0 && !fragment.is_empty() {
            continue;
        }
        let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);

        let suffix = if is_dir { "/" } else { "" };
        let display_path = format!("{}{}{}", dir_part, file_name, suffix);
        let kind_tag = if is_dir { "dir" } else { "file" };

        results.push(Candidate {
            display: format!("{} ({})", display_path, kind_tag),
            replacement: display_path,
            // File matches rank slightly below entity matches with the
            // same fuzzy score so `@spawn` never bumps a known
            // `spawn_agent` entity out of first place.
            score: score.saturating_sub(1).max(1),
            summary: None,
            category: Some("files".to_string()),
        });
    }
    results.sort_by(|a, b| b.score.cmp(&a.score).then(a.display.cmp(&b.display)));
    results.truncate(FILE_RESULT_LIMIT);
    results
}

/// Split `"src/mod"` → `("src/", "mod")`; `"foo"` → `("", "foo")`.
fn split_dir_fragment(query: &str) -> (String, String) {
    match query.rfind('/') {
        Some(idx) => (query[..=idx].to_string(), query[idx + 1..].to_string()),
        None => (String::new(), query.to_string()),
    }
}

/// Map the raw directory fragment (`""`, `"./"`, `"src/"`, `"~/"`,
/// `"/etc/"`) to an actual path to scan.
fn resolve_dir(dir_part: &str) -> std::path::PathBuf {
    if dir_part.is_empty() || dir_part == "./" {
        return std::path::PathBuf::from(".");
    }
    if let Some(rest) = dir_part.strip_prefix("~/") {
        if let Some(mut home) = dirs::home_dir() {
            home.push(rest);
            return home;
        }
    }
    std::path::PathBuf::from(dir_part)
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
    fn test_bare_slash_groups_by_category() {
        // Unfiltered popup should cluster commands by category so the
        // list reads like a table of contents, not an alphabetic flat
        // blob. Check that all candidates with category X are
        // contiguous in the output.
        let (_, candidates) = complete("/", 1, &[], false);
        let cats: Vec<&str> = candidates
            .iter()
            .map(|c| c.category.as_deref().unwrap_or(""))
            .collect();

        // Once a category appears and then stops appearing, it must
        // not reappear. That's the "grouped" invariant.
        let mut seen_and_ended: std::collections::HashSet<&str> = std::collections::HashSet::new();
        let mut prev: Option<&str> = None;
        for cat in &cats {
            if let Some(p) = prev {
                if p != *cat {
                    seen_and_ended.insert(p);
                }
            }
            assert!(
                !seen_and_ended.contains(cat),
                "category {:?} reappeared after ending; candidates are not grouped: {:?}",
                cat,
                cats
            );
            prev = Some(cat);
        }
    }

    #[test]
    fn test_filtered_slash_prioritises_fuzzy_rank_over_category() {
        // When the query narrows the list, fuzzy rank should win —
        // `/he` must put `/help` at position 0 regardless of which
        // category /help belongs to.
        let (_, candidates) = complete("/he", 3, &[], false);
        assert_eq!(candidates[0].replacement, "/help");
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

    #[test]
    fn test_split_dir_fragment() {
        assert_eq!(
            split_dir_fragment("src/mod"),
            ("src/".to_string(), "mod".to_string())
        );
        assert_eq!(
            split_dir_fragment("foo"),
            ("".to_string(), "foo".to_string())
        );
        assert_eq!(
            split_dir_fragment("a/b/c"),
            ("a/b/".to_string(), "c".to_string())
        );
    }

    #[test]
    fn test_at_path_lists_cwd_subdir() {
        // Verify the filesystem branch triggers when the query
        // contains a '/' — pick a real subdir that every working tree
        // has so we don't depend on which exact files exist.
        let td = tempfile::tempdir().unwrap();
        std::fs::create_dir(td.path().join("alpha")).unwrap();
        std::fs::write(td.path().join("bravo.txt"), b"").unwrap();

        // Build query as if the user typed `@<tmp>/` — an absolute
        // path; file_path_candidates will read_dir the tempdir and
        // offer `alpha/` and `bravo.txt`.
        let prefix = format!("{}/", td.path().display());
        let results = file_path_candidates(&prefix);
        let names: Vec<String> = results.iter().map(|c| c.display.clone()).collect();
        assert!(
            names
                .iter()
                .any(|n| n.contains("alpha") && n.contains("dir")),
            "expected alpha/ in results: {:?}",
            names
        );
        assert!(
            names
                .iter()
                .any(|n| n.contains("bravo.txt") && n.contains("file")),
            "expected bravo.txt in results: {:?}",
            names
        );
    }

    #[test]
    fn test_at_path_hides_dotfiles_by_default() {
        let td = tempfile::tempdir().unwrap();
        std::fs::write(td.path().join(".hidden"), b"").unwrap();
        std::fs::write(td.path().join("visible"), b"").unwrap();
        let results = file_path_candidates(&format!("{}/", td.path().display()));
        let names: Vec<String> = results.iter().map(|c| c.display.clone()).collect();
        assert!(names.iter().any(|n| n.contains("visible")));
        assert!(!names.iter().any(|n| n.contains(".hidden")));
    }

    #[test]
    fn test_at_path_shows_dotfiles_when_fragment_starts_with_dot() {
        let td = tempfile::tempdir().unwrap();
        std::fs::write(td.path().join(".hidden"), b"").unwrap();
        std::fs::write(td.path().join("visible"), b"").unwrap();
        let results = file_path_candidates(&format!("{}/.h", td.path().display()));
        let names: Vec<String> = results.iter().map(|c| c.display.clone()).collect();
        assert!(names.iter().any(|n| n.contains(".hidden")));
    }
}
