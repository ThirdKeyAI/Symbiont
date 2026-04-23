use crate::app::App;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let t = super::theme::current();
    let prompt = app.prompt();
    let line = Line::from(vec![
        Span::styled(prompt, Style::default().fg(t.agent)),
        Span::styled(app.input.clone(), Style::default().fg(t.input_text)),
    ]);
    let widget = Paragraph::new(line);
    frame.render_widget(widget, area);

    // Cursor
    let cursor_x = area.x + prompt.len() as u16 + app.cursor as u16;
    frame.set_cursor_position((cursor_x, area.y));

    // Completion popup (renders above the input line).
    //
    // Sizing: grow up to `MAX_POPUP_CONTENT_ROWS` content rows, bounded
    // by the vertical space available above the input. Two extra rows
    // are reserved for the borders. When the candidate list is larger
    // than the visible window, `first_visible` tracks the scroll offset
    // so the highlighted candidate is always on-screen — previously the
    // popup was hard-capped to 8 rows (≈6 visible items after borders)
    // which hid the majority of the 40+ slash commands.
    if app.completion_visible && !app.completion_candidates.is_empty() {
        const MAX_POPUP_CONTENT_ROWS: u16 = 12;
        const BORDER_ROWS: u16 = 2;

        let space_above = area.y; // rows available above the input line
        let content_rows = space_above
            .saturating_sub(BORDER_ROWS)
            .clamp(1, MAX_POPUP_CONTENT_ROWS) as usize;
        let visible = content_rows.min(app.completion_candidates.len());

        // Scroll window anchored on the selected index: the highlighted
        // candidate is always in view.
        let selected = app
            .completion_index
            .min(app.completion_candidates.len() - 1);
        let first_visible = if selected < visible {
            0
        } else {
            selected + 1 - visible
        };

        let has_more_above = first_visible > 0;
        let has_more_below = first_visible + visible < app.completion_candidates.len();

        // Column widths: the name column is the widest visible name; the
        // category column is the widest visible category (or 0 when no
        // candidate has one — e.g. DSL / @mention lists). Remainder goes
        // to the summary column, which gets truncated with an ellipsis.
        let max_name = app
            .completion_candidates
            .iter()
            .skip(first_visible)
            .take(visible)
            .map(|c| c.display.chars().count())
            .max()
            .unwrap_or(0);
        let max_cat = app
            .completion_candidates
            .iter()
            .skip(first_visible)
            .take(visible)
            .map(|c| {
                c.category
                    .as_deref()
                    .map(|s| s.chars().count() + 2)
                    .unwrap_or(0)
            })
            .max()
            .unwrap_or(0);

        // Popup width: up to 80 chars, bounded by remaining horizontal
        // space from the popup origin.
        const POPUP_MAX_WIDTH: u16 = 80;
        let origin_x = area.x + app.completion_start as u16;
        let room = area.width.saturating_sub(origin_x.saturating_sub(area.x));
        let popup_width = room.min(POPUP_MAX_WIDTH);

        // Inner width (content) = popup_width - 2 (borders)
        let inner_width = popup_width.saturating_sub(2) as usize;
        let gap = 2usize; // spaces between columns
                          // Budget: name + gap + (category + gap if any) + summary
        let summary_width = inner_width
            .saturating_sub(max_name)
            .saturating_sub(gap)
            .saturating_sub(if max_cat > 0 { max_cat + gap } else { 0 });

        let popup_height = visible as u16 + BORDER_ROWS;
        let popup_y = area.y.saturating_sub(popup_height);
        let popup_area = Rect::new(origin_x, popup_y, popup_width, popup_height);

        let items: Vec<Line> = app
            .completion_candidates
            .iter()
            .enumerate()
            .skip(first_visible)
            .take(visible)
            .map(|(i, c)| {
                let selected_row = i == selected;
                let name_style = if selected_row {
                    Style::default().fg(Color::Black).bg(t.footer_accent)
                } else {
                    Style::default().fg(t.input_text)
                };
                let meta_style = if selected_row {
                    Style::default().fg(Color::Black).bg(t.footer_accent)
                } else {
                    Style::default().fg(t.dim).add_modifier(Modifier::DIM)
                };

                // Pad the name column so categories / summaries align.
                let padded_name = if c.display.chars().count() < max_name {
                    format!(
                        "{}{}",
                        c.display,
                        " ".repeat(max_name - c.display.chars().count())
                    )
                } else {
                    c.display.clone()
                };

                let mut spans = vec![Span::styled(padded_name, name_style)];

                if max_cat > 0 {
                    let cat_str = match c.category.as_deref() {
                        Some(cat) => format!("  ({})", cat),
                        None => " ".repeat(max_cat + gap),
                    };
                    let padded = if cat_str.chars().count() < max_cat + gap {
                        format!(
                            "{}{}",
                            cat_str,
                            " ".repeat(max_cat + gap - cat_str.chars().count())
                        )
                    } else {
                        cat_str
                    };
                    spans.push(Span::styled(padded, meta_style));
                }

                if summary_width > 0 {
                    if let Some(sum) = c.summary.as_deref() {
                        let truncated = truncate_ellipsis(sum, summary_width);
                        spans.push(Span::raw("  "));
                        spans.push(Span::styled(truncated, meta_style));
                    }
                }

                Line::from(spans)
            })
            .collect();

        // Overflow hints in the border so users know the list extends.
        let title_top = if has_more_above { " ▲ more " } else { "" };
        let title_bottom = if has_more_below { " ▼ more " } else { "" };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.input_border))
            .title(title_top)
            .title_bottom(title_bottom);
        let popup = Paragraph::new(items).block(block);
        frame.render_widget(Clear, popup_area);
        frame.render_widget(popup, popup_area);
    }
}

/// Truncate `s` to fit within `width` columns (character count), adding
/// an ellipsis when the string is cut. Returns a new String.
fn truncate_ellipsis(s: &str, width: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= width {
        let mut out: String = chars.into_iter().collect();
        // Pad to `width` so backgrounds (e.g. selected-row highlight)
        // fill the column cleanly.
        while out.chars().count() < width {
            out.push(' ');
        }
        return out;
    }
    if width == 0 {
        return String::new();
    }
    if width == 1 {
        return "…".to_string();
    }
    let take = width - 1;
    let truncated: String = chars.into_iter().take(take).collect();
    format!("{}…", truncated)
}
