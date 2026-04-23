//! Convert markdown text to ratatui styled Lines.
//!
//! Derived from openai/codex-rs (Apache-2.0) `tui/src/markdown_render.rs`,
//! heavily trimmed to drop codex-specific local-file-link rewriting and
//! adaptive-wrapping logic. Adapted for symbi-shell's cyan/gray theme and
//! the DSL/Cedar/TOML/Clad highlighter we already ship.

use pulldown_cmark::{CodeBlockKind, CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

struct MarkdownStyles {
    h1: Style,
    h2: Style,
    h3: Style,
    h4: Style,
    h5: Style,
    h6: Style,
    code: Style,
    emphasis: Style,
    strong: Style,
    strikethrough: Style,
    ordered_list_marker: Style,
    unordered_list_marker: Style,
    link: Style,
    blockquote_prefix: Style,
}

impl MarkdownStyles {
    fn from_theme() -> Self {
        let t = super::theme::current();
        Self {
            h1: Style::default()
                .fg(t.md_heading)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            h2: Style::default()
                .fg(t.md_heading)
                .add_modifier(Modifier::BOLD),
            h3: Style::default()
                .fg(t.md_heading)
                .add_modifier(Modifier::BOLD | Modifier::ITALIC),
            h4: Style::default()
                .fg(t.md_heading)
                .add_modifier(Modifier::ITALIC),
            h5: Style::default()
                .fg(t.md_heading)
                .add_modifier(Modifier::ITALIC),
            h6: Style::default()
                .fg(t.md_heading)
                .add_modifier(Modifier::ITALIC),
            code: Style::default().fg(t.md_code),
            emphasis: Style::default().add_modifier(Modifier::ITALIC),
            strong: Style::default().add_modifier(Modifier::BOLD),
            strikethrough: Style::default().add_modifier(Modifier::CROSSED_OUT),
            ordered_list_marker: Style::default().fg(t.md_list_ordered),
            unordered_list_marker: Style::default().fg(t.md_list_unordered),
            link: Style::default()
                .fg(t.md_link)
                .add_modifier(Modifier::UNDERLINED),
            blockquote_prefix: Style::default().fg(t.md_blockquote),
        }
    }
}

/// Render markdown text into a list of styled ratatui Lines.
pub fn render(text: &str) -> Vec<Line<'static>> {
    let options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES;
    let parser = Parser::new_ext(text, options);
    let styles = MarkdownStyles::from_theme();

    let mut out: Vec<Line<'static>> = Vec::new();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut style_stack: Vec<Style> = Vec::new();

    let mut in_code_block = false;
    let mut code_block_lang: Option<String> = None;
    let mut code_block_buffer = String::new();

    let mut heading_level: Option<HeadingLevel> = None;

    // list_indices parallels pulldown_cmark::Tag::List(Option<u64>) per depth.
    // `None` = unordered, `Some(n)` = next ordered index.
    let mut list_indices: Vec<Option<u64>> = Vec::new();
    let mut list_pending_marker = false;

    let mut in_blockquote = false;

    // Pending link destination to append after the label when the link is not
    // bare text — matches the codex-rs convention of showing " (dest)" when
    // the label and destination differ.
    let mut pending_link: Option<String> = None;

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    flush_line(&mut out, &mut spans);
                    heading_level = Some(level);
                    let style = style_for_heading(&styles, level);
                    spans.push(Span::styled(
                        format!("{} ", "#".repeat(level as usize)),
                        style,
                    ));
                    style_stack.push(style);
                }
                Tag::Paragraph => {
                    if !out.is_empty() && !spans.is_empty() {
                        flush_line(&mut out, &mut spans);
                    }
                    if in_blockquote {
                        spans.push(Span::styled("> ", styles.blockquote_prefix));
                    }
                }
                Tag::CodeBlock(kind) => {
                    flush_line(&mut out, &mut spans);
                    in_code_block = true;
                    code_block_buffer.clear();
                    code_block_lang = match kind {
                        CodeBlockKind::Fenced(info) => {
                            // CommonMark info strings can carry metadata after
                            // the language ("rust,no_run", "rust title=demo");
                            // take the first token so highlighter lookup hits.
                            info.split([',', ' ', '\t'])
                                .next()
                                .filter(|s| !s.is_empty())
                                .map(str::to_string)
                        }
                        CodeBlockKind::Indented => None,
                    };
                }
                Tag::Strong => style_stack.push(styles.strong),
                Tag::Emphasis => style_stack.push(styles.emphasis),
                Tag::Strikethrough => style_stack.push(styles.strikethrough),
                Tag::List(start) => list_indices.push(start),
                Tag::Item => {
                    flush_line(&mut out, &mut spans);
                    let depth = list_indices.len();
                    let indent = if depth > 0 {
                        "  ".repeat(depth - 1)
                    } else {
                        String::new()
                    };
                    let marker = match list_indices.last_mut() {
                        Some(Some(idx)) => {
                            let s = format!("{}{}. ", indent, *idx);
                            *idx += 1;
                            Span::styled(s, styles.ordered_list_marker)
                        }
                        Some(None) => {
                            Span::styled(format!("{}- ", indent), styles.unordered_list_marker)
                        }
                        None => Span::raw(String::new()),
                    };
                    spans.push(marker);
                    list_pending_marker = true;
                }
                Tag::BlockQuote(_) => {
                    flush_line(&mut out, &mut spans);
                    in_blockquote = true;
                    spans.push(Span::styled("> ", styles.blockquote_prefix));
                }
                Tag::Link { dest_url, .. } => {
                    pending_link = Some(dest_url.to_string());
                    style_stack.push(styles.link);
                }
                _ => {}
            },
            Event::End(tag_end) => match tag_end {
                TagEnd::Heading(_) => {
                    style_stack.pop();
                    heading_level = None;
                    flush_line(&mut out, &mut spans);
                }
                TagEnd::Paragraph => flush_line(&mut out, &mut spans),
                TagEnd::CodeBlock => {
                    if !code_block_buffer.is_empty() {
                        render_code_block(&code_block_buffer, code_block_lang.as_deref(), &mut out);
                    }
                    in_code_block = false;
                    code_block_buffer.clear();
                    code_block_lang = None;
                }
                TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough => {
                    style_stack.pop();
                }
                TagEnd::List(_) => {
                    list_indices.pop();
                }
                TagEnd::Item => {
                    flush_line(&mut out, &mut spans);
                    list_pending_marker = false;
                }
                TagEnd::BlockQuote(_) => {
                    flush_line(&mut out, &mut spans);
                    in_blockquote = false;
                }
                TagEnd::Link => {
                    style_stack.pop();
                    if let Some(dest) = pending_link.take() {
                        if should_show_link_destination(&dest) {
                            spans.push(Span::styled(
                                format!(" ({})", dest),
                                Style::default().fg(super::theme::current().dim),
                            ));
                        }
                    }
                }
                _ => {}
            },
            Event::Text(text) => {
                if in_code_block {
                    code_block_buffer.push_str(&text);
                    continue;
                }
                let style = current_style(&style_stack, heading_level, &styles);
                push_text_spans(&text, style, &mut out, &mut spans);
                list_pending_marker = false;
            }
            Event::Code(code) => {
                spans.push(Span::styled(format!("`{}`", code), styles.code));
            }
            Event::SoftBreak | Event::HardBreak => {
                flush_line(&mut out, &mut spans);
                if in_blockquote {
                    spans.push(Span::styled("> ", styles.blockquote_prefix));
                }
            }
            Event::Rule => {
                flush_line(&mut out, &mut spans);
                out.push(Line::from(Span::styled(
                    "────────────────────────────────",
                    Style::default().fg(super::theme::current().dim),
                )));
            }
            _ => {}
        }
        let _ = list_pending_marker;
    }

    flush_line(&mut out, &mut spans);
    out
}

fn current_style(stack: &[Style], heading: Option<HeadingLevel>, styles: &MarkdownStyles) -> Style {
    if let Some(top) = stack.last().copied() {
        return top;
    }
    if let Some(level) = heading {
        return style_for_heading(styles, level);
    }
    Style::default()
}

fn style_for_heading(styles: &MarkdownStyles, level: HeadingLevel) -> Style {
    match level {
        HeadingLevel::H1 => styles.h1,
        HeadingLevel::H2 => styles.h2,
        HeadingLevel::H3 => styles.h3,
        HeadingLevel::H4 => styles.h4,
        HeadingLevel::H5 => styles.h5,
        HeadingLevel::H6 => styles.h6,
    }
}

fn push_text_spans(
    text: &CowStr<'_>,
    style: Style,
    out: &mut Vec<Line<'static>>,
    spans: &mut Vec<Span<'static>>,
) {
    // pulldown-cmark splits soft/hard breaks into separate events, but literal
    // '\n' inside a Text chunk still means "line break" for our purposes.
    for (i, piece) in text.split('\n').enumerate() {
        if i > 0 {
            flush_line(out, spans);
        }
        if !piece.is_empty() {
            spans.push(Span::styled(piece.to_string(), style));
        }
    }
}

fn should_show_link_destination(dest: &str) -> bool {
    // Hide destination for obvious local refs; show URLs so the user can copy.
    let d = dest.trim();
    if d.is_empty() {
        return false;
    }
    !(d.starts_with('/')
        || d.starts_with("./")
        || d.starts_with("../")
        || (d.contains('/') && !d.contains("://")))
}

fn render_code_block(content: &str, lang: Option<&str>, out: &mut Vec<Line<'static>>) {
    let highlighted = lang.and_then(|l| match l {
        "dsl" | "symbiont" | "symbi" => Some(super::syntax::highlight_dsl(content)),
        "cedar" => Some(super::syntax::highlight_cedar(content)),
        "toml" | "clad" => Some(super::syntax::highlight_toml(content)),
        _ => None,
    });

    if let Some(hl_lines) = highlighted {
        for hl_line in hl_lines {
            let mut indented = vec![Span::raw("  ".to_string())];
            indented.extend(hl_line.spans);
            out.push(Line::from(indented));
        }
        return;
    }

    // Fallback: plain yellow, indented two spaces.
    for code_line in content.split('\n') {
        out.push(Line::from(vec![
            Span::raw("  ".to_string()),
            Span::styled(
                code_line.to_string(),
                Style::default().fg(super::theme::current().md_code),
            ),
        ]));
    }
}

fn flush_line(lines: &mut Vec<Line<'static>>, spans: &mut Vec<Span<'static>>) {
    if !spans.is_empty() {
        lines.push(Line::from(std::mem::take(spans)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flatten(lines: &[Line<'static>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.to_string()).collect())
            .collect()
    }

    #[test]
    fn test_plain_text() {
        let lines = render("hello world");
        assert_eq!(lines.len(), 1);
        assert_eq!(flatten(&lines), vec!["hello world".to_string()]);
    }

    #[test]
    fn test_heading() {
        let lines = render("# Title\n\nBody text");
        assert!(lines.len() >= 2);
        let first: String = lines[0]
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        assert!(first.contains("Title"));
        assert!(first.starts_with("# "));
    }

    #[test]
    fn test_heading_levels_distinct_styles() {
        let h1 = render("# one");
        let h3 = render("### three");
        // H1 has bold+underlined, H3 has bold+italic.
        let h1_style = h1[0].spans[0].style;
        let h3_style = h3[0].spans[0].style;
        assert!(h1_style.add_modifier.contains(Modifier::UNDERLINED));
        assert!(h3_style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn test_code_block() {
        let lines = render("```\nlet x = 1;\n```");
        assert!(!lines.is_empty());
        let joined: Vec<String> = flatten(&lines);
        assert!(joined.iter().any(|l| l.contains("let x = 1;")));
    }

    #[test]
    fn test_bullet_list() {
        let lines = render("- item one\n- item two");
        let joined = flatten(&lines);
        assert!(joined.iter().any(|l| l.starts_with("- item one")));
        assert!(joined.iter().any(|l| l.starts_with("- item two")));
    }

    #[test]
    fn test_ordered_list_numbering_increments() {
        let lines = render("1. first\n2. second\n3. third");
        let joined = flatten(&lines);
        assert!(joined.iter().any(|l| l.starts_with("1. first")));
        assert!(joined.iter().any(|l| l.starts_with("2. second")));
        assert!(joined.iter().any(|l| l.starts_with("3. third")));
    }

    #[test]
    fn test_inline_code() {
        let lines = render("use `foo` here");
        assert_eq!(lines.len(), 1);
        let text: String = lines[0]
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        assert!(text.contains("`foo`"));
    }

    #[test]
    fn test_strikethrough() {
        let lines = render("normal ~~struck~~ end");
        let mut saw_struck = false;
        for span in &lines[0].spans {
            if span.content.contains("struck")
                && span.style.add_modifier.contains(Modifier::CROSSED_OUT)
            {
                saw_struck = true;
            }
        }
        assert!(saw_struck, "expected a strikethrough span for 'struck'");
    }

    #[test]
    fn test_link_destination_shown_for_url() {
        let lines = render("see [docs](https://example.com)");
        let joined: String = flatten(&lines).join("");
        assert!(joined.contains("docs"));
        assert!(joined.contains("(https://example.com)"));
    }

    #[test]
    fn test_link_destination_hidden_for_local_path() {
        let lines = render("see [README](./README.md)");
        let joined: String = flatten(&lines).join("");
        assert!(joined.contains("README"));
        assert!(!joined.contains("(./README.md)"));
    }

    #[test]
    fn test_blockquote_prefixed() {
        let lines = render("> quoted text");
        let joined: String = flatten(&lines).join("");
        assert!(joined.starts_with("> "));
        assert!(joined.contains("quoted text"));
    }

    #[test]
    fn test_code_block_language_strips_metadata() {
        // "rust,no_run" should be treated as lang="rust" — our highlighter
        // only knows dsl/cedar/toml, so this still falls back to plain yellow,
        // but we assert the render succeeds without crashing.
        let lines = render("```rust,no_run\nfn main() {}\n```");
        assert!(!lines.is_empty());
    }
}
