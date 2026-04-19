//! Convert markdown text to ratatui styled Lines.

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Render markdown text into a list of styled ratatui Lines.
pub fn render(text: &str) -> Vec<Line<'static>> {
    let options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES;
    let parser = Parser::new_ext(text, options);

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current_spans: Vec<Span<'static>> = Vec::new();

    // Style stack for nested formatting
    let mut bold = false;
    let mut italic = false;
    let mut in_code_block = false;
    let mut code_block_lang = String::new();
    let mut code_block_content = String::new();
    let mut in_heading = false;
    let mut heading_level: u8 = 0;
    let mut in_list = false;
    let mut list_prefix = String::new();
    let mut line_started = false;

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    flush_line(&mut lines, &mut current_spans);
                    in_heading = true;
                    heading_level = level as u8;
                }
                Tag::Paragraph => {
                    if !lines.is_empty() && !in_list {
                        flush_line(&mut lines, &mut current_spans);
                    }
                }
                Tag::CodeBlock(kind) => {
                    flush_line(&mut lines, &mut current_spans);
                    in_code_block = true;
                    code_block_content.clear();
                    code_block_lang = match kind {
                        CodeBlockKind::Fenced(lang) => lang.to_string(),
                        CodeBlockKind::Indented => String::new(),
                    };
                }
                Tag::Strong => bold = true,
                Tag::Emphasis => italic = true,
                Tag::List(start) => {
                    in_list = true;
                    if let Some(n) = start {
                        list_prefix = format!("{}. ", n);
                    } else {
                        list_prefix = "  - ".to_string();
                    }
                }
                Tag::Item => {
                    flush_line(&mut lines, &mut current_spans);
                    current_spans.push(Span::styled(
                        list_prefix.clone(),
                        Style::default().fg(Color::DarkGray),
                    ));
                    line_started = true;
                }
                Tag::BlockQuote(_) => {
                    flush_line(&mut lines, &mut current_spans);
                    current_spans.push(Span::styled("  | ", Style::default().fg(Color::DarkGray)));
                }
                _ => {}
            },
            Event::End(tag_end) => match tag_end {
                TagEnd::Heading(_) => {
                    // Style the accumulated heading text
                    let styled: Vec<Span<'static>> = current_spans
                        .drain(..)
                        .map(|s| {
                            Span::styled(
                                s.content.to_string(),
                                Style::default()
                                    .fg(Color::Cyan)
                                    .add_modifier(Modifier::BOLD),
                            )
                        })
                        .collect();
                    let prefix = match heading_level {
                        1 => "# ",
                        2 => "## ",
                        3 => "### ",
                        _ => "#### ",
                    };
                    let mut all = vec![Span::styled(
                        prefix,
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )];
                    all.extend(styled);
                    lines.push(Line::from(all));
                    in_heading = false;
                }
                TagEnd::Paragraph => {
                    flush_line(&mut lines, &mut current_spans);
                }
                TagEnd::CodeBlock => {
                    // Apply syntax highlighting based on language tag
                    if !code_block_content.is_empty() {
                        let highlighted = match code_block_lang.as_str() {
                            "dsl" | "symbiont" | "symbi" => {
                                Some(super::syntax::highlight_dsl(&code_block_content))
                            }
                            "cedar" => Some(super::syntax::highlight_cedar(&code_block_content)),
                            "toml" | "clad" => {
                                Some(super::syntax::highlight_toml(&code_block_content))
                            }
                            _ => None,
                        };
                        if let Some(hl_lines) = highlighted {
                            flush_line(&mut lines, &mut current_spans);
                            for hl_line in hl_lines {
                                let mut indented = vec![Span::raw("  ".to_string())];
                                indented.extend(hl_line.spans);
                                lines.push(Line::from(indented));
                            }
                        }
                    }
                    in_code_block = false;
                    code_block_content.clear();
                }
                TagEnd::Strong => bold = false,
                TagEnd::Emphasis => italic = false,
                TagEnd::List(_) => {
                    in_list = false;
                }
                TagEnd::Item => {
                    flush_line(&mut lines, &mut current_spans);
                    line_started = false;
                }
                TagEnd::BlockQuote(_) => {
                    flush_line(&mut lines, &mut current_spans);
                }
                _ => {}
            },
            Event::Text(text) => {
                let style = if in_code_block {
                    Style::default().fg(Color::Yellow)
                } else if in_heading {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    let mut s = Style::default();
                    if bold {
                        s = s.add_modifier(Modifier::BOLD);
                    }
                    if italic {
                        s = s.add_modifier(Modifier::ITALIC);
                    }
                    s
                };

                if in_code_block {
                    if matches!(
                        code_block_lang.as_str(),
                        "dsl" | "symbiont" | "symbi" | "cedar" | "toml" | "clad"
                    ) {
                        // Accumulate content for syntax highlighting at block end
                        code_block_content.push_str(&text);
                    } else {
                        // Non-DSL code blocks: plain yellow, line by line
                        for code_line in text.split('\n') {
                            if line_started {
                                flush_line(&mut lines, &mut current_spans);
                            }
                            current_spans.push(Span::styled(format!("  {}", code_line), style));
                            line_started = true;
                        }
                    }
                } else {
                    current_spans.push(Span::styled(text.to_string(), style));
                    line_started = true;
                }
            }
            Event::Code(code) => {
                current_spans.push(Span::styled(
                    format!("`{}`", code),
                    Style::default().fg(Color::Yellow),
                ));
            }
            Event::SoftBreak | Event::HardBreak => {
                flush_line(&mut lines, &mut current_spans);
            }
            Event::Rule => {
                flush_line(&mut lines, &mut current_spans);
                lines.push(Line::from(Span::styled(
                    "────────────────────────────────",
                    Style::default().fg(Color::DarkGray),
                )));
            }
            _ => {}
        }
    }

    // Flush remaining
    flush_line(&mut lines, &mut current_spans);

    lines
}

fn flush_line(lines: &mut Vec<Line<'static>>, spans: &mut Vec<Span<'static>>) {
    if !spans.is_empty() {
        lines.push(Line::from(std::mem::take(spans)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_text() {
        let lines = render("hello world");
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn test_heading() {
        let lines = render("# Title\n\nBody text");
        assert!(lines.len() >= 2);
        // First line should contain "# " prefix
        let first = &lines[0];
        let text: String = first.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(text.contains("Title"));
    }

    #[test]
    fn test_code_block() {
        let lines = render("```\nlet x = 1;\n```");
        // Should have at least the code line
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_bullet_list() {
        let lines = render("- item one\n- item two");
        assert!(lines.len() >= 2);
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
}
