use crate::app::{App, InputMode};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use throbber_widgets_tui::{Throbber, WhichUse};

pub fn draw(frame: &mut Frame, app: &mut App, area: Rect) {
    let t = super::theme::current();
    let mode_str = match app.mode {
        InputMode::Orchestrator => "ORCH",
        InputMode::Dsl => "DSL",
    };

    // The mode pill is background-on-accent — keep Color::Black as the
    // foreground because the accent color is theme-variable and should
    // always be dark-on-light, not theme.sys on theme.footer_accent.
    let mode_pill = Style::default()
        .fg(Color::Black)
        .bg(t.footer_accent)
        .add_modifier(Modifier::BOLD);

    if app.is_busy() {
        // Split footer: left = status, right = throbber
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(30), Constraint::Length(25)])
            .split(area);

        let status = Line::from(vec![
            Span::styled(format!(" {} ", mode_str), mode_pill),
            Span::raw(" "),
            Span::styled(
                format!("model:{}", app.model_name),
                Style::default().fg(t.dim),
            ),
            Span::raw(" | "),
            Span::styled(
                format!("tokens:{}", app.tokens_used),
                Style::default().fg(t.dim),
            ),
        ]);
        frame.render_widget(Paragraph::new(status), chunks[0]);

        let throbber = Throbber::default()
            .label(app.busy_label.as_str())
            .style(Style::default().fg(t.warning))
            .throbber_style(Style::default().fg(t.footer_accent))
            .use_type(WhichUse::Spin);
        frame.render_stateful_widget(throbber, chunks[1], &mut app.throbber_state);
    } else {
        let footer = Line::from(vec![
            Span::styled(format!(" {} ", mode_str), mode_pill),
            Span::raw(" "),
            Span::styled(
                format!("model:{}", app.model_name),
                Style::default().fg(t.dim),
            ),
            Span::raw(" | "),
            Span::styled(
                format!("agents:{}", app.active_agents),
                Style::default().fg(t.warning),
            ),
            Span::raw(" | "),
            Span::styled(
                format!("tokens:{}", app.tokens_used),
                Style::default().fg(t.dim),
            ),
            Span::raw(" | "),
            Span::styled(
                if app.remote.is_some() {
                    "attached".to_string()
                } else {
                    "local".to_string()
                },
                Style::default().fg(if app.remote.is_some() {
                    t.success
                } else {
                    t.dim
                }),
            ),
        ]);
        frame.render_widget(Paragraph::new(footer), area);
    }
}
