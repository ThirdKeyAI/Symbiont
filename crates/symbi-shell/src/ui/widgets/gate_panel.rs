use crate::ui::theme;
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph},
};

/// A single held action awaiting operator approval, projected from the
/// `GET /api/v1/approvals` JSON payload into a render-ready view with a
/// pre-computed countdown.
#[derive(Debug, Clone)]
pub struct HeldActionView {
    pub id: String,
    pub agent_id: String,
    pub summary: String,
    pub reason: String,
    pub seconds_left: i64,
}

impl HeldActionView {
    /// Parse one held-action object. Returns `None` if the required
    /// `id` / `expires_at` fields are missing or malformed.
    pub fn from_json(v: &serde_json::Value) -> Option<Self> {
        let expires = v.get("expires_at")?.as_str()?;
        let expires = chrono::DateTime::parse_from_rfc3339(expires).ok()?;
        let secs = (expires.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_seconds();
        Some(Self {
            id: v.get("id")?.as_str()?.to_string(),
            agent_id: v
                .get("agent_id")
                .and_then(|x| x.as_str())
                .unwrap_or("?")
                .to_string(),
            summary: v
                .get("summary")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            reason: v
                .get("reason")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            seconds_left: secs.max(0),
        })
    }
}

/// Bordered panel listing held actions with a live countdown and the
/// approve/deny key hints.
pub struct GatePanel<'a> {
    items: &'a [HeldActionView],
    selected: usize,
}

impl<'a> GatePanel<'a> {
    pub fn new(items: &'a [HeldActionView], selected: usize) -> Self {
        Self { items, selected }
    }
}

impl Widget for GatePanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let t = theme::current();
        let mut lines: Vec<Line> = Vec::new();
        if self.items.is_empty() {
            lines.push(Line::from("No held actions."));
        } else {
            for (i, it) in self.items.iter().enumerate() {
                let marker = if i == self.selected { "> " } else { "  " };
                let (mm, ss) = (it.seconds_left / 60, it.seconds_left % 60);
                lines.push(Line::from(format!(
                    "{marker}{}  {:<14} {:<28} ⏳ {}:{:02}",
                    it.id, it.agent_id, it.summary, mm, ss
                )));
            }
            if let Some(sel) = self.items.get(self.selected) {
                lines.push(Line::from(""));
                lines.push(Line::from(format!("reason: {}", sel.reason)));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(
                "[a] approve  [d] deny  ↑↓ select  Ctrl+G/Esc close",
            ));
        }
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.footer_accent))
            .title(format!(" Gate · {} held ", self.items.len()));
        Paragraph::new(lines).block(block).render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_held_action_json() {
        let v = serde_json::json!({ "id":"a3f1","agent_id":"api-monitor","summary":"tool_call http_post",
            "reason":"policy","expires_at": (chrono::Utc::now()+chrono::Duration::seconds(48)).to_rfc3339(), "status":"pending" });
        let h = HeldActionView::from_json(&v).unwrap();
        assert_eq!(h.id, "a3f1");
        assert!(h.seconds_left > 40 && h.seconds_left <= 48);
    }
}
