#![allow(dead_code)]

mod content;
mod footer;
mod input;
mod markdown;
pub mod sidebar;
pub mod syntax;
pub mod widgets;

use crate::app::App;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::Frame;

/// Render the full UI layout.
pub fn draw(frame: &mut Frame, app: &mut App) {
    let main_chunks = if app.sidebar_visible {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(24), Constraint::Min(1)])
            .split(frame.area())
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(0), Constraint::Min(1)])
            .split(frame.area())
    };

    if app.sidebar_visible {
        sidebar::draw(frame, app, main_chunks[0]);
    }

    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(main_chunks[1]);

    content::draw(frame, app, right_chunks[0]);
    footer::draw(frame, app, right_chunks[1]);
    input::draw(frame, app, right_chunks[2]);
}
