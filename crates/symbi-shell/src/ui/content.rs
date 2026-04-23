use super::markdown;
use crate::app::{App, EntrySource, NoticeKind, ToolCallEntry};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

/// Max body rows rendered before a tool-call card truncates with an
/// `… +N more (ctrl+o)` hint. When expanded the full body is shown.
const TOOL_CARD_COLLAPSED_ROWS: usize = 12;

/// Render the "live tail" of unfinished entries into the inline
/// viewport area. Finalized entries have already been flushed to
/// terminal scrollback via `insert_before` and do not appear here.
pub fn draw_live_tail(frame: &mut Frame, app: &App, area: Rect) {
    let lines = render_entries_to_lines(app.live_tail());
    let paragraph = Paragraph::new(lines)
        .block(Block::default().borders(Borders::NONE))
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

/// Render a slice of output entries as a flat `Vec<Line>`. Used both
/// by the inline viewport's live tail and by the main loop when it
/// flushes entries into terminal scrollback via `insert_before`.
///
/// The resulting `Line`s own their string data so the returned vec
/// can outlive the `entries` slice (necessary for `insert_before`
/// which consumes the lines inside its own closure).
pub fn render_entries_to_lines(entries: &[crate::app::OutputEntry]) -> Vec<Line<'static>> {
    let t = super::theme::current();
    let mut lines: Vec<Line<'static>> = Vec::new();

    for entry in entries {
        match &entry.source {
            EntrySource::User => {
                lines.push(Line::from(vec![
                    Span::styled("you: ", Style::default().fg(t.user)),
                    Span::raw(entry.content.clone()),
                ]));
            }
            EntrySource::System => {
                for text_line in entry.content.lines() {
                    lines.push(Line::from(Span::styled(
                        format!("sys: {}", text_line),
                        Style::default().fg(t.sys),
                    )));
                }
            }
            EntrySource::Agent(name) => {
                lines.push(Line::from(Span::styled(
                    format!("{}:", name),
                    Style::default().fg(t.agent),
                )));
                let md_lines = markdown::render(&entry.content);
                for md_line in md_lines {
                    let mut indented = vec![Span::raw("  ")];
                    indented.extend(md_line.spans);
                    lines.push(Line::from(indented));
                }
            }
            EntrySource::Error => {
                lines.push(Line::from(vec![
                    Span::styled("err: ", Style::default().fg(t.err)),
                    Span::styled(entry.content.clone(), Style::default().fg(t.err)),
                ]));
            }
            EntrySource::Meta => {
                // Dimmed, two-space indented so it visually groups under
                // the preceding agent reply.
                for text_line in entry.content.lines() {
                    lines.push(Line::from(Span::styled(
                        format!("  {}", text_line),
                        Style::default().fg(t.meta).add_modifier(Modifier::DIM),
                    )));
                }
            }
            EntrySource::ToolCall(card) => {
                render_tool_card(card, &mut lines);
            }
            EntrySource::Notice { kind, source_label } => {
                let (icon, color) = match kind {
                    NoticeKind::Info => ("ℹ", t.info),
                    NoticeKind::Success => ("✓", t.success),
                    NoticeKind::Warning => ("⚠", t.warning),
                    NoticeKind::Error => ("✗", t.error),
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("{} ", icon), Style::default().fg(color)),
                    Span::styled(format!("[{}] ", source_label), Style::default().fg(t.dim)),
                    Span::styled(entry.content.clone(), Style::default().fg(color)),
                ]));
            }
        }
    }

    lines
}

/// Render a tool-call card into `lines`.
///
/// Layout:
///   ● tool_name(args_summary)  •  4ms       [header, always one row]
///     ⎿ first line of output                 [body, only when interesting]
///       ...
///       … +N more (ctrl+o)
///
/// The body is suppressed when the call is trivially fast and
/// uninteresting — see [`should_show_body`] for the heuristic. Users
/// can force the body via Ctrl+O (`expanded = true`).
fn render_tool_card(card: &ToolCallEntry, lines: &mut Vec<Line>) {
    let t = super::theme::current();
    // Header: ● name(args_summary)  •  <duration>
    let dot_color = if card.is_error {
        t.tool_error
    } else if !card.done {
        t.tool_running
    } else {
        t.tool_done
    };
    let mut header_spans: Vec<Span<'static>> = vec![
        Span::styled("● ", Style::default().fg(dot_color)),
        Span::styled(
            card.name.clone(),
            Style::default()
                .fg(t.tool_name)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("("),
        Span::styled(card.args_summary.clone(), Style::default().fg(t.tool_args)),
        Span::raw(")"),
    ];
    if let Some(ms) = card.duration_ms {
        header_spans.push(Span::styled(
            format!("  •  {}", format_duration_ms(ms)),
            Style::default().fg(t.dim).add_modifier(Modifier::DIM),
        ));
    }
    lines.push(Line::from(header_spans));

    if !should_show_body(card) {
        return;
    }

    // Body: diff if edit-shaped, otherwise plain ⎿-indented output.
    if card.is_edit {
        render_edit_body(card, lines);
    } else {
        render_plain_body(card, lines);
    }
}

/// Heuristic for deciding whether to render a tool card's body.
///
/// A call's body earns its pixels when it either:
/// - the user explicitly expanded the card (Ctrl+O);
/// - the call errored (important to see why);
/// - the call is an edit-shaped tool (the diff IS the value);
/// - it took a meaningful amount of time (≥ 500 ms — the model
///   probably did real work, user is likely to want context);
/// - the output is multi-line with something non-trivial;
/// - the call is still running (no duration yet).
///
/// Fast trivial calls like `list_agents()` that return a single line
/// collapse to just the `●` header to keep the transcript quiet.
pub fn should_show_body(card: &ToolCallEntry) -> bool {
    if card.expanded {
        return true;
    }
    if !card.done {
        return true;
    }
    if card.is_error || card.is_edit {
        return true;
    }
    let non_empty_lines = card.output.lines().filter(|l| !l.trim().is_empty()).count();
    if non_empty_lines == 0 {
        return false;
    }
    let slow = card.duration_ms.map(|ms| ms >= 500).unwrap_or(true);
    if slow {
        return true;
    }
    // Fast tool, short output — collapse to header only.
    non_empty_lines > 1
}

/// Human-readable duration for the header ("4ms" / "320ms" / "2.1s").
fn format_duration_ms(ms: u64) -> String {
    if ms >= 1_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        format!("{}ms", ms)
    }
}

fn render_plain_body(card: &ToolCallEntry, lines: &mut Vec<Line>) {
    let t = super::theme::current();
    if !card.done && card.output.is_empty() {
        // Running with no observation yet — show the spinner glyph so
        // the card doesn't look empty while the tool executes.
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("⎿ ", Style::default().fg(t.dim).add_modifier(Modifier::DIM)),
            Span::styled(
                "running…",
                Style::default().fg(t.dim).add_modifier(Modifier::ITALIC),
            ),
        ]));
        return;
    }

    let text_lines: Vec<&str> = card.output.lines().collect();
    let show_all = card.expanded || text_lines.len() <= TOOL_CARD_COLLAPSED_ROWS;
    let visible_rows = if show_all {
        text_lines.len()
    } else {
        TOOL_CARD_COLLAPSED_ROWS
    };
    let body_style = if card.is_error {
        Style::default().fg(t.err)
    } else {
        Style::default().fg(t.dim)
    };

    for (idx, line) in text_lines.iter().take(visible_rows).enumerate() {
        let prefix = if idx == 0 { "  ⎿ " } else { "    " };
        lines.push(Line::from(vec![
            Span::styled(prefix, Style::default().fg(t.dim)),
            Span::styled(line.to_string(), body_style),
        ]));
    }

    if !show_all {
        let remaining = text_lines.len() - visible_rows;
        lines.push(Line::from(Span::styled(
            format!("    … +{} more (ctrl+o)", remaining),
            Style::default().fg(t.dim).add_modifier(Modifier::DIM),
        )));
    }
}

/// Lines of context shown around each changed region in the diff.
const DIFF_CONTEXT_RADIUS: usize = 2;

/// Render an edit-shaped tool call as a unified LCS diff.
///
/// Extracts `old_string` / `new_string` from the args JSON, computes a
/// Myers diff via `similar`, and emits hunks with 2 lines of surrounding
/// context, each line prefixed by `-` / `+` / ` ` and colored
/// accordingly. Falls back to the plain body when the shape is
/// unexpected.
///
/// This approach is derived in spirit from openai/codex-rs (Apache-2.0)
/// `tui/src/diff_render.rs`, but the codex-rs renderer is ~2,500 lines
/// of terminal-palette-aware, syntax-highlighted, line-numbered output
/// with theme-adaptive backgrounds. For the in-transcript card preview
/// we only need interleaved -/+/space gutters and a compact hunk
/// header, so we use the `similar` crate directly instead of lifting
/// the full renderer.
fn render_edit_body(card: &ToolCallEntry, lines: &mut Vec<Line>) {
    let parsed = serde_json::from_str::<serde_json::Value>(&card.args).ok();
    let old_str = parsed
        .as_ref()
        .and_then(|v| v.get("old_string"))
        .and_then(|v| v.as_str());
    let new_str = parsed
        .as_ref()
        .and_then(|v| v.get("new_string"))
        .and_then(|v| v.as_str());

    let (Some(old), Some(new)) = (old_str, new_str) else {
        render_plain_body(card, lines);
        return;
    };

    let body = build_diff_rows(old, new, DIFF_CONTEXT_RADIUS);
    let t = super::theme::current();

    if body.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("  ⎿ ", Style::default().fg(t.dim)),
            Span::styled(
                "(no changes)",
                Style::default().fg(t.dim).add_modifier(Modifier::DIM),
            ),
        ]));
        return;
    }

    let show_all = card.expanded || body.len() <= TOOL_CARD_COLLAPSED_ROWS;
    let visible_rows = if show_all {
        body.len()
    } else {
        TOOL_CARD_COLLAPSED_ROWS
    };

    for (idx, row) in body.iter().take(visible_rows).enumerate() {
        let prefix = if idx == 0 { "  ⎿ " } else { "    " };
        let mut spans = vec![Span::styled(prefix, Style::default().fg(t.dim))];
        match row {
            DiffRow::HunkHeader(label) => {
                spans.push(Span::styled(
                    label.clone(),
                    Style::default().fg(t.diff_hunk).add_modifier(Modifier::DIM),
                ));
            }
            DiffRow::Change { marker, text } => {
                let (gutter_color, body_color) = match marker {
                    '-' => (t.diff_del, t.diff_del),
                    '+' => (t.diff_add, t.diff_add),
                    _ => (t.dim, t.diff_context),
                };
                spans.push(Span::styled(
                    format!("{} ", marker),
                    Style::default()
                        .fg(gutter_color)
                        .add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::styled(text.clone(), Style::default().fg(body_color)));
            }
        }
        lines.push(Line::from(spans));
    }

    if !show_all {
        let remaining = body.len() - visible_rows;
        lines.push(Line::from(Span::styled(
            format!("    … +{} more (ctrl+o)", remaining),
            Style::default().fg(t.dim).add_modifier(Modifier::DIM),
        )));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DiffRow {
    HunkHeader(String),
    Change { marker: char, text: String },
}

/// Compute a unified-diff-shaped flat list of rows from `old` → `new`.
///
/// Uses `similar::TextDiff` for the underlying Myers LCS. Output is a
/// sequence of hunks, each preceded by a `@@ -a,b +c,d @@`-style header
/// and containing `-` / `+` / ` ` rows. `context` controls how many
/// equal lines are kept around each change cluster.
fn build_diff_rows(old: &str, new: &str, context: usize) -> Vec<DiffRow> {
    use similar::{ChangeTag, TextDiff};

    let diff = TextDiff::from_lines(old, new);
    let mut rows: Vec<DiffRow> = Vec::new();
    for group in diff.grouped_ops(context) {
        let (old_range, new_range) = group_range(&group);
        rows.push(DiffRow::HunkHeader(format!(
            "@@ -{},{} +{},{} @@",
            old_range.0 + 1,
            old_range.1,
            new_range.0 + 1,
            new_range.1
        )));
        for op in &group {
            for change in diff.iter_changes(op) {
                let marker = match change.tag() {
                    ChangeTag::Delete => '-',
                    ChangeTag::Insert => '+',
                    ChangeTag::Equal => ' ',
                };
                // `similar` includes the trailing newline in change.value();
                // strip so each ratatui Line renders on exactly one row.
                let text = change.value().trim_end_matches('\n').to_string();
                rows.push(DiffRow::Change { marker, text });
            }
        }
    }
    rows
}

/// Collapse a group of DiffOps into (old_start, old_len) + (new_start, new_len).
fn group_range(ops: &[similar::DiffOp]) -> ((usize, usize), (usize, usize)) {
    use similar::DiffOp;
    let (mut old_start, mut old_end) = (usize::MAX, 0usize);
    let (mut new_start, mut new_end) = (usize::MAX, 0usize);
    for op in ops {
        let (os, ol, ns, nl) = match *op {
            DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => (old_index, len, new_index, len),
            DiffOp::Delete {
                old_index,
                old_len,
                new_index,
            } => (old_index, old_len, new_index, 0),
            DiffOp::Insert {
                old_index,
                new_index,
                new_len,
            } => (old_index, 0, new_index, new_len),
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => (old_index, old_len, new_index, new_len),
        };
        old_start = old_start.min(os);
        new_start = new_start.min(ns);
        old_end = old_end.max(os + ol);
        new_end = new_end.max(ns + nl);
    }
    if old_start == usize::MAX {
        old_start = 0;
    }
    if new_start == usize::MAX {
        new_start = 0;
    }
    (
        (old_start, old_end.saturating_sub(old_start)),
        (new_start, new_end.saturating_sub(new_start)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_diff_rows_interleaves_changes_with_context() {
        let old = "a\nb\nc\nd\ne\n";
        let new = "a\nb\nX\nd\ne\n";
        let rows = build_diff_rows(old, new, 2);
        assert!(matches!(rows.first(), Some(DiffRow::HunkHeader(_))));
        let changes: Vec<(char, String)> = rows
            .iter()
            .filter_map(|r| match r {
                DiffRow::Change { marker, text } => Some((*marker, text.clone())),
                _ => None,
            })
            .collect();
        assert!(changes.contains(&('-', "c".to_string())));
        assert!(changes.contains(&('+', "X".to_string())));
        assert!(changes.contains(&(' ', "a".to_string())));
    }

    #[test]
    fn build_diff_rows_empty_for_identical_input() {
        let rows = build_diff_rows("same\n", "same\n", 2);
        assert!(rows.is_empty());
    }

    #[test]
    fn build_diff_rows_handles_pure_addition() {
        let rows = build_diff_rows("", "added\n", 2);
        let inserts = rows
            .iter()
            .filter(|r| matches!(r, DiffRow::Change { marker: '+', .. }))
            .count();
        assert_eq!(inserts, 1);
    }

    #[test]
    fn build_diff_rows_handles_pure_deletion() {
        let rows = build_diff_rows("gone\n", "", 2);
        let deletes = rows
            .iter()
            .filter(|r| matches!(r, DiffRow::Change { marker: '-', .. }))
            .count();
        assert_eq!(deletes, 1);
    }
}
