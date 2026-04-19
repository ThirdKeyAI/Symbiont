use super::markdown;
use crate::app::{App, EntrySource};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();

    for entry in &app.output {
        match &entry.source {
            EntrySource::User => {
                lines.push(Line::from(vec![
                    Span::styled("you: ", Style::default().fg(Color::Cyan)),
                    Span::raw(&entry.content),
                ]));
            }
            EntrySource::System => {
                for text_line in entry.content.lines() {
                    lines.push(Line::from(Span::styled(
                        format!("sys: {}", text_line),
                        Style::default().fg(Color::DarkGray),
                    )));
                }
            }
            EntrySource::Agent(name) => {
                lines.push(Line::from(Span::styled(
                    format!("{}:", name),
                    Style::default().fg(Color::Green),
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
                    Span::styled("err: ", Style::default().fg(Color::Red)),
                    Span::styled(&entry.content, Style::default().fg(Color::Red)),
                ]));
            }
        }
    }

    let total_lines = lines.len() as u16;
    let visible_height = area.height;

    // Calculate scroll: scroll_offset=0 means show the bottom (latest)
    let max_scroll = total_lines.saturating_sub(visible_height);
    let scroll_pos = max_scroll.saturating_sub(app.scroll_offset);

    let paragraph = Paragraph::new(lines)
        .block(Block::default().borders(Borders::NONE))
        .wrap(Wrap { trim: false })
        .scroll((scroll_pos, 0));

    frame.render_widget(paragraph, area);
}
