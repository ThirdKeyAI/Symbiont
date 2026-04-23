use crate::ui::theme;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

/// A single trace entry for display.
pub struct TraceEntry {
    pub timestamp: String,
    pub phase: String,
    pub agent: Option<String>,
    pub description: String,
    pub duration_ms: Option<u64>,
}

/// Widget for rendering an execution trace as a timeline.
pub struct TraceTimeline<'a> {
    entries: &'a [TraceEntry],
}

impl<'a> TraceTimeline<'a> {
    pub fn new(entries: &'a [TraceEntry]) -> Self {
        Self { entries }
    }
}

impl Widget for TraceTimeline<'_> {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        let t = theme::current();
        let lines: Vec<Line> = self
            .entries
            .iter()
            .map(|entry| {
                // Map ORGA phases onto existing notice roles so theme
                // choice keeps the perceptual distinction:
                //   Observe → info, Reason → keyword/magenta,
                //   Gate → warning, Act → success.
                let phase_color = match entry.phase.as_str() {
                    "Observe" => t.info,
                    "Reason" => t.syn_keyword,
                    "Gate" => t.warning,
                    "Act" => t.success,
                    _ => t.dim,
                };

                let mut spans = vec![
                    Span::styled(format!("{} ", entry.timestamp), Style::default().fg(t.dim)),
                    Span::styled(
                        format!("[{:>7}] ", entry.phase),
                        Style::default().fg(phase_color),
                    ),
                ];

                if let Some(ref agent) = entry.agent {
                    spans.push(Span::styled(
                        format!("@{} ", agent),
                        Style::default().fg(t.syn_type),
                    ));
                }

                spans.push(Span::raw(&entry.description));

                if let Some(ms) = entry.duration_ms {
                    spans.push(Span::styled(
                        format!(" ({}ms)", ms),
                        Style::default().fg(t.dim),
                    ));
                }

                Line::from(spans)
            })
            .collect();

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.input_border))
            .title(" Execution Trace ");

        let paragraph = Paragraph::new(lines).block(block);
        paragraph.render(area, buf);
    }
}
