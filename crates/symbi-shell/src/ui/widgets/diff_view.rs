use crate::ui::theme;
use crate::validation::diff::{DiffKind, DiffLine};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};

/// Widget for rendering an artifact diff with escalation highlighting.
pub struct DiffView<'a> {
    lines: &'a [DiffLine],
    title: &'a str,
}

impl<'a> DiffView<'a> {
    pub fn new(lines: &'a [DiffLine], title: &'a str) -> Self {
        Self { lines, title }
    }
}

impl Widget for DiffView<'_> {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        let t = theme::current();
        let rendered_lines: Vec<Line> = self
            .lines
            .iter()
            .map(|dl| {
                let (prefix, style) = match dl.kind {
                    DiffKind::Added => ("+ ", Style::default().fg(t.diff_add)),
                    DiffKind::Removed => ("- ", Style::default().fg(t.diff_del)),
                    DiffKind::Unchanged => ("  ", Style::default().fg(t.diff_context)),
                    DiffKind::Escalation => (
                        "! ",
                        Style::default()
                            .fg(t.error)
                            .bg(t.warning)
                            .add_modifier(Modifier::BOLD),
                    ),
                };
                Line::from(vec![
                    Span::styled(prefix, style),
                    Span::styled(&dl.content, style),
                ])
            })
            .collect();

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.input_border))
            .title(format!(" {} ", self.title));

        let paragraph = Paragraph::new(rendered_lines)
            .block(block)
            .wrap(Wrap { trim: false });

        paragraph.render(area, buf);
    }
}
