#![allow(dead_code)]

pub mod content;
mod footer;
mod input;
mod markdown;
pub mod osc8;
pub mod sidebar;
pub mod syntax;
pub mod theme;
pub mod widgets;

use crate::app::App;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::Frame;

/// Render the inline-viewport UI layout.
///
/// The inline viewport is the bottom-N rows of the terminal. Finalized
/// transcript entries have already been pushed into the terminal's
/// scrollback via `Terminal::insert_before` — what lives in the
/// viewport is the "live tail": any still-in-progress tool card, the
/// footer, and the input line.
///
/// The sidebar is disabled in the inline model (there's no side space
/// to use), but the code is kept in the tree because /memory display
/// may move back when we add a sidebar-capable mode later.
pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    // Gate panel mode: live tail above, the held-action queue in a fixed
    // region, then footer + input.
    if app.gate_visible {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),     // live tail
                Constraint::Length(10), // gate panel
                Constraint::Length(1),  // footer
                Constraint::Length(1),  // input
            ])
            .split(area);
        content::draw_live_tail(frame, app, chunks[0]);
        frame.render_widget(
            widgets::gate_panel::GatePanel::new(&app.gate_items, app.gate_selected),
            chunks[1],
        );
        footer::draw(frame, app, chunks[2]);
        input::draw(frame, app, chunks[3]);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),    // live tail (pending tool cards)
            Constraint::Length(1), // footer
            Constraint::Length(1), // input
        ])
        .split(area);

    content::draw_live_tail(frame, app, chunks[0]);
    footer::draw(frame, app, chunks[1]);
    input::draw(frame, app, chunks[2]);
}
