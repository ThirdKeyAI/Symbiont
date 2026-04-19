//! Syntax highlighting for Symbiont DSL, Cedar policies, and ToolClad TOML.
//!
//! - DSL: tree-sitter-symbiont grammar for full AST-based highlighting
//! - Cedar: keyword-based highlighting (permit/forbid/when/unless/principal/action/resource)
//! - TOML: structural highlighting (sections, keys, values, strings, comments)

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Highlight DSL source code into styled ratatui Lines.
///
/// Falls back to plain yellow text if tree-sitter parsing fails.
pub fn highlight_dsl(source: &str) -> Vec<Line<'static>> {
    match dsl::parse_dsl(source) {
        Ok(tree) => highlight_tree(&tree, source),
        Err(_) => {
            // Fallback: plain yellow for unparseable code
            source
                .lines()
                .map(|line| {
                    Line::from(Span::styled(
                        line.to_string(),
                        Style::default().fg(Color::Yellow),
                    ))
                })
                .collect()
        }
    }
}

fn highlight_tree(tree: &tree_sitter::Tree, source: &str) -> Vec<Line<'static>> {
    let source_bytes = source.as_bytes();
    let root = tree.root_node();

    // Collect all leaf-level styled spans with their byte positions
    let mut highlights: Vec<(usize, usize, Style)> = Vec::new();
    collect_highlights(root, &mut highlights);

    // Sort by start position
    highlights.sort_by_key(|(start, _, _)| *start);

    // Build lines by walking through source, applying highlights
    let mut result: Vec<Line<'static>> = Vec::new();
    let mut current_spans: Vec<Span<'static>> = Vec::new();

    let lines: Vec<&str> = source.lines().collect();
    let mut line_start: usize = 0;

    for line_text in &lines {
        let line_end = line_start + line_text.len();
        current_spans.clear();

        let mut pos = line_start;
        for &(hl_start, hl_end, style) in &highlights {
            // Skip highlights that don't overlap this line
            if hl_end <= line_start || hl_start >= line_end {
                continue;
            }

            // Clamp to line boundaries
            let span_start = hl_start.max(line_start);
            let span_end = hl_end.min(line_end);

            // Add unstyled gap before this highlight
            if span_start > pos {
                let gap = std::str::from_utf8(&source_bytes[pos..span_start]).unwrap_or("");
                if !gap.is_empty() {
                    current_spans.push(Span::raw(gap.to_string()));
                }
            }

            // Add highlighted span
            let text = std::str::from_utf8(&source_bytes[span_start..span_end]).unwrap_or("");
            if !text.is_empty() {
                current_spans.push(Span::styled(text.to_string(), style));
            }

            pos = span_end;
        }

        // Add remaining unstyled text on this line
        if pos < line_end {
            let remainder = std::str::from_utf8(&source_bytes[pos..line_end]).unwrap_or("");
            if !remainder.is_empty() {
                current_spans.push(Span::raw(remainder.to_string()));
            }
        }

        if current_spans.is_empty() {
            result.push(Line::from(Span::raw(line_text.to_string())));
        } else {
            result.push(Line::from(std::mem::take(&mut current_spans)));
        }

        // +1 for the newline character
        line_start = line_end + 1;
    }

    result
}

fn collect_highlights(node: tree_sitter::Node, highlights: &mut Vec<(usize, usize, Style)>) {
    let kind = node.kind();
    let start = node.start_byte();
    let end = node.end_byte();

    // Map node kinds to styles
    if let Some(style) = style_for_node(kind, node.is_named()) {
        highlights.push((start, end, style));
        return; // Don't recurse into styled leaf nodes
    }

    // Recurse into children
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            collect_highlights(cursor.node(), highlights);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

fn style_for_node(kind: &str, is_named: bool) -> Option<Style> {
    match kind {
        // Keywords
        "agent"
        | "policy"
        | "function"
        | "type"
        | "schedule"
        | "channel"
        | "memory"
        | "webhook"
        | "metadata"
        | "capabilities"
        | "with"
        | "search"
        | "filter"
        | "data_classification"
        | "if"
        | "else"
        | "let"
        | "return" => Some(
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),

        // Policy actions
        "allow" | "deny" | "require" | "audit" => {
            Some(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
        }

        // Type builtins
        "String" | "int" | "float" | "bool" => Some(Style::default().fg(Color::Cyan)),

        // Boolean literals
        "true" | "false" => Some(Style::default().fg(Color::Yellow)),

        // Named node types
        "string" if is_named => Some(Style::default().fg(Color::Green)),
        "number" if is_named => Some(Style::default().fg(Color::Yellow)),
        "duration_literal" if is_named => Some(Style::default().fg(Color::Yellow)),
        "comment" if is_named => Some(Style::default().fg(Color::DarkGray)),
        "identifier" if is_named => None, // Style depends on parent context

        // Operators
        "=" | "->" => Some(Style::default().fg(Color::DarkGray)),

        // Punctuation
        "{" | "}" | "(" | ")" | "[" | "]" | ":" | "," | ";" => {
            Some(Style::default().fg(Color::DarkGray))
        }

        _ => None,
    }
}

/// Highlight Cedar policy text into styled ratatui Lines.
pub fn highlight_cedar(source: &str) -> Vec<Line<'static>> {
    source.lines().map(highlight_cedar_line).collect()
}

fn highlight_cedar_line(line: &str) -> Line<'static> {
    let trimmed = line.trim();

    // Comments
    if trimmed.starts_with("//") {
        return Line::from(Span::styled(
            line.to_string(),
            Style::default().fg(Color::DarkGray),
        ));
    }

    let mut spans: Vec<Span<'static>> = Vec::new();
    let remaining = line.to_string();
    let mut pos = 0;

    while pos < remaining.len() {
        let rest = &remaining[pos..];

        // Check for Cedar keywords at word boundary
        let keyword_match = CEDAR_KEYWORDS.iter().find(|&&kw| {
            rest.starts_with(kw)
                && rest[kw.len()..]
                    .chars()
                    .next()
                    .is_none_or(|c| !c.is_alphanumeric() && c != '_')
        });

        if let Some(&kw) = keyword_match {
            let style = match kw {
                "permit" => Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
                "forbid" => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                "when" | "unless" => Style::default().fg(Color::Magenta),
                "principal" | "action" | "resource" => Style::default().fg(Color::Cyan),
                "context" => Style::default().fg(Color::Yellow),
                _ => Style::default().fg(Color::Magenta),
            };
            spans.push(Span::styled(kw.to_string(), style));
            pos += kw.len();
            continue;
        }

        // Check for string literals
        if let Some(after_quote) = rest.strip_prefix('"') {
            if let Some(end) = after_quote.find('"') {
                let s = &remaining[pos..pos + end + 2];
                spans.push(Span::styled(
                    s.to_string(),
                    Style::default().fg(Color::Green),
                ));
                pos += end + 2;
                continue;
            }
        }

        // Check for entity refs (Type::"value")
        if rest.starts_with("::") {
            spans.push(Span::styled(
                "::".to_string(),
                Style::default().fg(Color::DarkGray),
            ));
            pos += 2;
            continue;
        }

        // Default: single character unstyled
        let ch = remaining[pos..].chars().next().unwrap();
        // Punctuation
        if matches!(ch, '(' | ')' | '{' | '}' | ',' | ';' | '=') {
            spans.push(Span::styled(
                ch.to_string(),
                Style::default().fg(Color::DarkGray),
            ));
        } else {
            spans.push(Span::raw(ch.to_string()));
        }
        pos += ch.len_utf8();
    }

    if spans.is_empty() {
        Line::from(Span::raw(line.to_string()))
    } else {
        Line::from(spans)
    }
}

const CEDAR_KEYWORDS: &[&str] = &[
    "permit",
    "forbid",
    "when",
    "unless",
    "principal",
    "action",
    "resource",
    "context",
    "true",
    "false",
    "if",
    "then",
    "else",
    "in",
    "has",
    "like",
    "is",
    "Action",
    "Agent",
    "Tool",
    "Resource",
    "User",
    "System",
];

/// Highlight TOML text (ToolClad manifests) into styled ratatui Lines.
pub fn highlight_toml(source: &str) -> Vec<Line<'static>> {
    source.lines().map(highlight_toml_line).collect()
}

fn highlight_toml_line(line: &str) -> Line<'static> {
    let trimmed = line.trim();

    // Comments
    if trimmed.starts_with('#') {
        return Line::from(Span::styled(
            line.to_string(),
            Style::default().fg(Color::DarkGray),
        ));
    }

    // Section headers [tool], [args.name], etc.
    if trimmed.starts_with('[') {
        return Line::from(Span::styled(
            line.to_string(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
    }

    // Key = value lines
    if let Some(eq_pos) = trimmed.find('=') {
        let indent = line.len() - line.trim_start().len();
        let key = trimmed[..eq_pos].trim_end();
        let value = trimmed[eq_pos + 1..].trim_start();

        let mut spans = Vec::new();
        if indent > 0 {
            spans.push(Span::raw(" ".repeat(indent)));
        }
        spans.push(Span::styled(
            key.to_string(),
            Style::default().fg(Color::Magenta),
        ));
        spans.push(Span::styled(
            " = ".to_string(),
            Style::default().fg(Color::DarkGray),
        ));

        // Style the value
        let value_style = if value.starts_with('"') {
            Style::default().fg(Color::Green)
        } else if value == "true" || value == "false" {
            Style::default().fg(Color::Yellow)
        } else if value.starts_with('[') {
            Style::default().fg(Color::White)
        } else if value.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default()
        };
        spans.push(Span::styled(value.to_string(), value_style));

        return Line::from(spans);
    }

    Line::from(Span::raw(line.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_highlight_simple_agent() {
        let dsl = r#"agent Monitor {
    capabilities: [read, observe]
}"#;
        let lines = highlight_dsl(dsl);
        assert!(!lines.is_empty());
        // First line should have the "agent" keyword highlighted
        let first = &lines[0];
        assert!(
            first.spans.len() > 1,
            "Expected multiple spans, got {:?}",
            first.spans
        );
    }

    #[test]
    fn test_highlight_with_string() {
        let dsl = r#"metadata {
    name: "My Agent",
    version: "1.0"
}"#;
        let lines = highlight_dsl(dsl);
        assert!(lines.len() >= 3);
    }

    #[test]
    fn test_highlight_invalid_falls_back() {
        let dsl = "this is completely invalid {{{{}}}}";
        let lines = highlight_dsl(dsl);
        // Should still produce output (fallback)
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_highlight_function() {
        let dsl = r#"function greet(name: String) -> String {
    return "hello";
}"#;
        let lines = highlight_dsl(dsl);
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_highlight_policy() {
        let dsl = r#"policy access_control {
    allow: read
    deny: write
}"#;
        let lines = highlight_dsl(dsl);
        assert!(lines.len() >= 3);
    }

    #[test]
    fn test_highlight_cedar_permit() {
        let cedar = r#"permit (
    principal == Agent::"Monitor",
    action == "read",
    resource == Tool::"healthcheck"
);"#;
        let lines = highlight_cedar(cedar);
        assert!(lines.len() >= 4);
        // First line should have "permit" highlighted
        let first = &lines[0];
        assert!(first.spans.len() > 1);
    }

    #[test]
    fn test_highlight_cedar_forbid() {
        let cedar = r#"forbid (principal, action, resource);"#;
        let lines = highlight_cedar(cedar);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn test_highlight_cedar_comment() {
        let cedar = "// This is a comment";
        let lines = highlight_cedar(cedar);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn test_highlight_toml_section() {
        let toml = r#"[tool]
name = "healthcheck"
risk_tier = "low"
timeout_seconds = 30
"#;
        let lines = highlight_toml(toml);
        assert!(lines.len() >= 3);
    }

    #[test]
    fn test_highlight_toml_values() {
        let toml = r#"enabled = true
count = 42
name = "test"
"#;
        let lines = highlight_toml(toml);
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn test_highlight_toml_comment() {
        let toml = "# This is a comment";
        let lines = highlight_toml(toml);
        assert_eq!(lines.len(), 1);
    }
}
