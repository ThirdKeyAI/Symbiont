use crate::ui::theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

/// Data for rendering an agent card.
pub struct AgentCardData {
    pub name: String,
    pub id: String,
    pub version: String,
    pub state: String,
    pub capabilities: Vec<String>,
    pub sandbox: String,
    pub security_tier: String,
}

/// A bordered card widget showing agent metadata.
pub struct AgentCard<'a> {
    data: &'a AgentCardData,
}

impl<'a> AgentCard<'a> {
    pub fn new(data: &'a AgentCardData) -> Self {
        Self { data }
    }
}

impl Widget for AgentCard<'_> {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        let t = theme::current();
        let state_color = match self.data.state.as_str() {
            "Running" => t.success,
            "Paused" => t.warning,
            "Stopped" | "Failed" => t.error,
            _ => t.dim,
        };

        let lines = vec![
            Line::from(vec![
                Span::styled(
                    &self.data.name,
                    Style::default()
                        .fg(t.tool_name)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(
                    format!("v{}", self.data.version),
                    Style::default().fg(t.dim),
                ),
                Span::raw("  "),
                Span::styled(&self.data.state, Style::default().fg(state_color)),
            ]),
            Line::from(vec![
                Span::styled("id: ", Style::default().fg(t.dim)),
                Span::raw(&self.data.id[..8.min(self.data.id.len())]),
            ]),
            Line::from(vec![
                Span::styled("sandbox: ", Style::default().fg(t.dim)),
                Span::raw(&self.data.sandbox),
                Span::raw("  "),
                Span::styled("tier: ", Style::default().fg(t.dim)),
                Span::raw(&self.data.security_tier),
            ]),
            Line::from(vec![
                Span::styled("capabilities: ", Style::default().fg(t.dim)),
                Span::raw(self.data.capabilities.join(", ")),
            ]),
        ];

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.footer_accent))
            .title(" Agent ");

        let paragraph = Paragraph::new(lines).block(block);
        paragraph.render(area, buf);
    }
}
