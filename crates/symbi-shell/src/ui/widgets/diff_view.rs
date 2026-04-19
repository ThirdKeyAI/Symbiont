use crate::validation::diff::{DiffKind, DiffLine};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
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
        let rendered_lines: Vec<Line> = self
            .lines
            .iter()
            .map(|dl| {
                let (prefix, style) = match dl.kind {
                    DiffKind::Added => ("+ ", Style::default().fg(Color::Green)),
                    DiffKind::Removed => ("- ", Style::default().fg(Color::Red)),
                    DiffKind::Unchanged => ("  ", Style::default().fg(Color::DarkGray)),
                    DiffKind::Escalation => (
                        "! ",
                        Style::default()
                            .fg(Color::Red)
                            .bg(Color::Yellow)
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
            .border_style(Style::default().fg(Color::DarkGray))
            .title(format!(" {} ", self.title));

        let paragraph = Paragraph::new(rendered_lines)
            .block(block)
            .wrap(Wrap { trim: false });

        paragraph.render(area, buf);
    }
}
