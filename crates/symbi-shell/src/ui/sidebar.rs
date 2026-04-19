use crate::app::App;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    if app.sidebar_show_memory {
        // Split sidebar: top = project info, bottom = memory
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(10), Constraint::Min(1)])
            .split(area);

        draw_project_info(frame, app, chunks[0]);
        draw_memory(frame, app, chunks[1]);
    } else {
        draw_project_info(frame, app, area);
    }
}

fn draw_project_info(frame: &mut Frame, app: &App, area: Rect) {
    let mut lines = Vec::new();

    // Agents section
    lines.push(Line::from(Span::styled(
        " Agents",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));
    if app.active_agents == 0 {
        lines.push(Line::from(Span::styled(
            "   (none)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            format!("   {} active", app.active_agents),
            Style::default().fg(Color::Green),
        )));
    }

    lines.push(Line::from(""));

    // Entities section
    lines.push(Line::from(Span::styled(
        " Entities",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));
    let entity_counts = count_entity_kinds(&app.entities);
    for (kind, count) in entity_counts {
        lines.push(Line::from(Span::styled(
            format!("   {} {}", count, kind),
            Style::default().fg(Color::DarkGray),
        )));
    }

    // Memory toggle hint
    lines.push(Line::from(""));
    let mem_hint = if app.sidebar_show_memory {
        " Ctrl+M: hide memory"
    } else {
        " Ctrl+M: show memory"
    };
    lines.push(Line::from(Span::styled(
        mem_hint,
        Style::default().fg(Color::DarkGray),
    )));

    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(" Project ");

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn draw_memory(frame: &mut Frame, app: &App, area: Rect) {
    let lines: Vec<Line> = match &app.memory_content {
        Some(content) => content
            .lines()
            .map(|line| {
                if line.starts_with("## ") {
                    Line::from(Span::styled(
                        line.to_string(),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ))
                } else if let Some(item) = line.strip_prefix("- ") {
                    Line::from(vec![
                        Span::styled("  - ", Style::default().fg(Color::DarkGray)),
                        Span::raw(item.to_string()),
                    ])
                } else {
                    Line::from(Span::styled(
                        format!("  {}", line),
                        Style::default().fg(Color::DarkGray),
                    ))
                }
            })
            .collect(),
        None => vec![Line::from(Span::styled(
            "  (no memory.md found)",
            Style::default().fg(Color::DarkGray),
        ))],
    };

    let block = Block::default()
        .borders(Borders::RIGHT | Borders::TOP)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(" Memory ");

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn count_entity_kinds(entities: &[(String, String)]) -> Vec<(String, usize)> {
    let mut counts = std::collections::HashMap::new();
    for (_, kind) in entities {
        *counts.entry(kind.clone()).or_insert(0usize) += 1;
    }
    let mut sorted: Vec<_> = counts.into_iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    sorted
}
