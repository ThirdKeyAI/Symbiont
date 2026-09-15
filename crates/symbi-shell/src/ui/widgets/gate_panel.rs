use crate::ui::theme;
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph},
};
use symbi_runtime::escalation::{render_approval_request, HeldAction, HeldStatus};

/// Complete immutable request retained while the operator reviews it.
#[derive(Debug, Clone)]
pub struct HeldActionView {
    pub id: String,
    action: HeldAction,
    snapshot: serde_json::Value,
    pub details: Result<String, String>,
}

impl HeldActionView {
    pub fn from_json(value: &serde_json::Value) -> Option<Self> {
        let known = [
            "id",
            "agent_id",
            "kind",
            "summary",
            "reason",
            "context_snapshot",
            "created_at",
            "expires_at",
            "status",
        ];
        if !value
            .as_object()?
            .keys()
            .all(|key| known.contains(&key.as_str()))
        {
            return None;
        }
        let action: HeldAction = serde_json::from_value(value.clone()).ok()?;
        if action.id.len() != 16
            || !action.id.bytes().all(|b| b.is_ascii_hexdigit())
            || action.status != HeldStatus::Pending
        {
            return None;
        }
        Some(Self {
            id: action.id.clone(),
            details: render_approval_request(&action),
            action,
            snapshot: value.clone(),
        })
    }
    pub fn same_request(&self, other: &Self) -> bool {
        self.snapshot == other.snapshot
    }
    pub fn reviewable(&self) -> bool {
        self.details.is_ok() && chrono::Utc::now() < self.action.expires_at
    }
}

/// An ambiguous or partially decoded queue cannot support an approval review.
pub fn parse_pending(value: &serde_json::Value) -> Result<Vec<HeldActionView>, String> {
    let rows = value.as_array().ok_or("approval response is not a list")?;
    if rows.len() > 1024 {
        return Err("approval response exceeds the queue display limit".into());
    }
    let mut ids = std::collections::HashSet::new();
    rows.iter()
        .map(|value| {
            let item = HeldActionView::from_json(value)
                .ok_or("approval response contains an unsupported request")?;
            if !ids.insert(item.id.clone()) {
                return Err("approval response contains duplicate request IDs".into());
            }
            Ok(item)
        })
        .collect()
}

pub fn safe_label(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch == ' ' || ch.is_ascii_graphic() {
                ch.to_string()
            } else {
                ch.escape_unicode().to_string()
            }
        })
        .collect()
}

/// Wrap every byte of the ASCII JSON, including long unbroken arguments.
fn detail_rows(details: &str, width: usize) -> Vec<String> {
    details
        .lines()
        .flat_map(|line| {
            if line.is_empty() {
                vec![String::new()]
            } else {
                line.as_bytes()
                    .chunks(width.max(1))
                    .map(|chunk| {
                        String::from_utf8(chunk.to_vec()).expect("approval details are ASCII")
                    })
                    .collect()
            }
        })
        .collect()
}

pub struct GatePanel<'a> {
    items: &'a [HeldActionView],
    selected: usize,
    review: Option<&'a HeldActionView>,
    scroll: &'a mut usize,
    message: &'a str,
}
impl<'a> GatePanel<'a> {
    pub fn new(
        items: &'a [HeldActionView],
        selected: usize,
        review: Option<&'a HeldActionView>,
        scroll: &'a mut usize,
        message: &'a str,
    ) -> Self {
        Self {
            items,
            selected,
            review,
            scroll,
            message,
        }
    }
}
impl Widget for GatePanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::current().footer_accent))
            .title(match self.review {
                Some(review) => format!(" Review {} ", review.id),
                None => format!(" Gate · {} held ", self.items.len()),
            });
        let inner = block.inner(area);
        block.render(area, buf);
        let height = usize::from(inner.height.saturating_sub(2));
        let mut lines: Vec<Line> = Vec::new();
        if let Some(review) = self.review {
            match &review.details {
                Ok(details) => {
                    let rows = detail_rows(details, usize::from(inner.width));
                    *self.scroll = (*self.scroll).min(rows.len().saturating_sub(height));
                    lines.extend(
                        rows.into_iter()
                            .skip(*self.scroll)
                            .take(height)
                            .map(Line::from),
                    );
                }
                Err(error) => lines.push(Line::from(safe_label(error))),
            }
        } else if self.items.is_empty() {
            lines.push(Line::from("No held actions."));
        } else {
            let first = self.selected.saturating_sub(height.saturating_sub(1));
            for (index, item) in self.items.iter().enumerate().skip(first).take(height) {
                let marker = if index == self.selected { ">" } else { " " };
                let seconds = (item.action.expires_at - chrono::Utc::now())
                    .num_seconds()
                    .max(0);
                lines.push(Line::from(format!(
                    "{marker} {} {}s {} {}",
                    item.id,
                    seconds,
                    safe_label(&item.action.agent_id),
                    safe_label(&item.action.summary)
                )));
            }
        }
        while lines.len() < height {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(safe_label(self.message)));
        lines.push(Line::from(if self.review.is_some() {
            "[a] approve [d] deny | arrows/PgUp/PgDn scroll | Esc back"
        } else {
            "Enter review | arrows select | Ctrl+G/Esc close"
        }));
        Paragraph::new(lines).render(inner, buf);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub fn request(id: &str) -> serde_json::Value {
        serde_json::json!({"id":id,"agent_id":"operator-test","kind":"tool_call",
            "summary":"tool_call edit_file","reason":"approval required",
            "context_snapshot":{"invocation":{"arguments":{"path":"safe/file","note":"\u{1b}[2J\u{202e}hidden"}}},
            "created_at":chrono::Utc::now(),"expires_at":chrono::Utc::now()+chrono::Duration::seconds(60),"status":"pending"})
    }
    #[test]
    fn ambiguous_or_unknown_requests_reject_the_entire_queue() {
        let mut value = request("0123456789abcdef");
        assert!(parse_pending(&serde_json::json!([value.clone()])).is_ok());
        assert!(parse_pending(&serde_json::json!([value.clone(), value.clone()])).is_err());
        value["future_authority"] = "hidden".into();
        assert!(parse_pending(&serde_json::json!([value])).is_err());
        assert!(parse_pending(&serde_json::json!({"error":"not authorized"})).is_err());
    }

    #[test]
    fn review_preserves_complete_arguments_and_escapes_controls() {
        let value = request("0123456789abcdef");
        let view = HeldActionView::from_json(&value).unwrap();
        let rendered = view.details.as_ref().unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(rendered).unwrap(),
            value
        );
        assert!(rendered
            .bytes()
            .all(|b| b == b'\n' || (0x20..=0x7e).contains(&b)));
        assert!(rendered.contains("\\u202e"));
        assert!(view.reviewable());
        assert_eq!(
            detail_rows(rendered, 7).concat(),
            rendered.lines().collect::<String>()
        );
    }
    #[test]
    fn changed_expired_and_oversize_requests_cannot_reuse_a_review() {
        let mut value = request("0123456789abcdef");
        let original = HeldActionView::from_json(&value).unwrap();
        value["context_snapshot"]["invocation"]["arguments"]["path"] = "other/file".into();
        assert!(!original.same_request(&HeldActionView::from_json(&value).unwrap()));
        value["reason"] = "x".repeat(70_000).into();
        assert!(!HeldActionView::from_json(&value).unwrap().reviewable());
        value["expires_at"] = (chrono::Utc::now() - chrono::Duration::seconds(1))
            .to_rfc3339()
            .into();
        assert!(!HeldActionView::from_json(&value).unwrap().reviewable());
        value["id"] = "../../approve".into();
        assert!(HeldActionView::from_json(&value).is_none());
    }
    #[test]
    fn detail_view_can_reach_the_last_byte_of_a_long_argument() {
        let mut value = request("0123456789abcdef");
        value["context_snapshot"]["invocation"]["arguments"]["note"] =
            format!("{}END_OF_ARGUMENT", "x".repeat(500)).into();
        let view = HeldActionView::from_json(&value).unwrap();
        let mut offset = usize::MAX;
        let area = Rect::new(0, 0, 70, 16);
        let mut buffer = Buffer::empty(area);
        GatePanel::new(&[], 0, Some(&view), &mut offset, "").render(area, &mut buffer);
        let content = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(content.contains("END_OF_ARGUMENT"));
        assert!(offset < usize::MAX);
    }
}
