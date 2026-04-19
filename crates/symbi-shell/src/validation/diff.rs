/// A line in a structured diff.
#[derive(Debug, Clone)]
pub struct DiffLine {
    pub kind: DiffKind,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffKind {
    Added,
    Removed,
    Unchanged,
    /// An escalation: something that grants more permission.
    Escalation,
}

/// Generate a line-by-line diff between old and new artifact text.
/// Marks permission-related additions as escalations.
pub fn artifact_diff(old: &str, new: &str) -> Vec<DiffLine> {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    let mut result = Vec::new();

    let mut oi = 0;
    let mut ni = 0;

    while oi < old_lines.len() && ni < new_lines.len() {
        if old_lines[oi] == new_lines[ni] {
            result.push(DiffLine {
                kind: DiffKind::Unchanged,
                content: old_lines[oi].to_string(),
            });
            oi += 1;
            ni += 1;
        } else {
            result.push(DiffLine {
                kind: DiffKind::Removed,
                content: old_lines[oi].to_string(),
            });
            oi += 1;
        }
    }

    while oi < old_lines.len() {
        result.push(DiffLine {
            kind: DiffKind::Removed,
            content: old_lines[oi].to_string(),
        });
        oi += 1;
    }

    while ni < new_lines.len() {
        let line = new_lines[ni].to_string();
        let kind = if is_escalation(&line) {
            DiffKind::Escalation
        } else {
            DiffKind::Added
        };
        result.push(DiffLine {
            kind,
            content: line,
        });
        ni += 1;
    }

    result
}

/// Heuristic: does this line represent a permission escalation?
fn is_escalation(line: &str) -> bool {
    let lower = line.to_lowercase();
    let escalation_patterns = [
        "permit",
        "allow",
        "risk_tier",
        "permissive",
        "network_raw",
        "filesystem_write_root",
        "delegate",
        "human_approval = false",
    ];
    escalation_patterns.iter().any(|p| lower.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identical_no_diff() {
        let text = "line 1\nline 2";
        let diff = artifact_diff(text, text);
        assert!(diff.iter().all(|d| d.kind == DiffKind::Unchanged));
    }

    #[test]
    fn test_addition_detected() {
        let diff = artifact_diff("line 1", "line 1\nline 2");
        assert!(diff.iter().any(|d| d.kind == DiffKind::Added));
    }

    #[test]
    fn test_escalation_detected() {
        let diff = artifact_diff("", "permit(principal, action, resource)");
        assert!(diff.iter().any(|d| d.kind == DiffKind::Escalation));
    }

    #[test]
    fn test_removal_detected() {
        let diff = artifact_diff("line 1\nline 2", "line 1");
        assert!(diff.iter().any(|d| d.kind == DiffKind::Removed));
    }
}
