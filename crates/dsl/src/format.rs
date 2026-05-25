//! Canonical formatter for Symbi (`.symbi`) source files.
//!
//! The formatter canonicalizes the structural wrapper layer — metadata blocks,
//! agent headers, capabilities declarations, and `with`-block headers — and
//! preserves the verbatim source bytes for everything else (policy bodies,
//! function bodies, statement bodies, expressions). This keeps it useful as a
//! pre-commit / CI tool without depending on full canonical pretty-printing
//! of the entire DSL.
//!
//! Style summary:
//! - 4-space indent.
//! - One blank line between top-level items; a leading line comment is
//!   attached to the next item with no blank-line separator.
//! - `metadata_pair` and `capabilities` preserve the source's separator
//!   (`=` or `:`) so existing files round-trip with minimal diff.
//! - Empty `with` blocks render canonically as `{\n}`; non-empty bodies are
//!   echoed verbatim from the source.
//! - When the parser produces any error, the input is returned unchanged
//!   (tolerant fallback) — a future grammar bump can recover ground.

use crate::parse_dsl;
use tree_sitter::Node;

const INDENT: &str = "    ";

#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    #[error("parse error: {0}")]
    Parse(String),
}

pub fn format_source(input: &str) -> Result<String, FormatError> {
    let tree = parse_dsl(input).map_err(|e| FormatError::Parse(e.to_string()))?;
    let root = tree.root_node();

    // Tolerant: if any part of the source failed to parse, leave it alone.
    // The user can still rely on `--check` to surface drift in clean files,
    // and a follow-up grammar bump expands what we can canonicalize.
    if root.has_error() {
        return Ok(input.to_string());
    }

    let mut out = String::new();
    let printer = Printer::new(input);
    printer.print_program(root, &mut out);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

struct Printer<'src> {
    source: &'src str,
}

impl<'src> Printer<'src> {
    fn new(source: &'src str) -> Self {
        Self { source }
    }

    fn text(&self, node: Node) -> &'src str {
        &self.source[node.start_byte()..node.end_byte()]
    }

    /// Walk all children including anonymous tokens (separators, keywords).
    fn iter_children<'a>(&self, node: Node<'a>) -> Vec<Node<'a>> {
        let mut out = Vec::new();
        for i in 0u32..node.child_count() as u32 {
            if let Some(c) = node.child(i) {
                out.push(c);
            }
        }
        out
    }

    /// Find the first occurrence of either `=` or `:` among the node's
    /// (possibly anonymous) direct children. Used to preserve metadata and
    /// capabilities separators across roundtrips.
    fn first_separator(&self, node: Node) -> &'static str {
        for c in self.iter_children(node) {
            match self.text(c) {
                "=" => return "=",
                ":" => return ":",
                _ => {}
            }
        }
        ":"
    }

    fn print_program(&self, root: Node, out: &mut String) {
        let mut cursor = root.walk();
        let mut prev_kind: Option<&str> = None;
        for child in root.named_children(&mut cursor) {
            if let Some(prev) = prev_kind {
                if prev != "comment" {
                    out.push('\n');
                }
            }
            prev_kind = Some(child.kind());
            self.print_top_item(child, 0, out);
        }
    }

    fn print_top_item(&self, node: Node, depth: usize, out: &mut String) {
        match node.kind() {
            "comment" => self.emit_comment(node, depth, out),
            "metadata_block" => {
                self.print_metadata_block(node, depth, out);
                out.push('\n');
            }
            "agent_definition" => {
                self.print_agent_definition(node, depth, out);
                out.push('\n');
            }
            // policy_definition, type_definition, function_definition,
            // schedule_definition, channel_definition, memory_definition,
            // webhook_definition — preserve verbatim source bytes.
            _ => self.emit_verbatim(node, depth, out),
        }
    }

    fn emit_comment(&self, node: Node, depth: usize, out: &mut String) {
        self.write_indent(out, depth);
        out.push_str(self.text(node));
        out.push('\n');
    }

    fn emit_verbatim(&self, node: Node, depth: usize, out: &mut String) {
        // The node's text() begins at its first non-whitespace token; we
        // prepend our depth indent. Continuation lines inside the byte span
        // retain their original column, which lines up with our depth tree
        // when the source is consistently 4-space indented.
        self.write_indent(out, depth);
        out.push_str(self.text(node));
        out.push('\n');
    }

    fn print_metadata_block(&self, node: Node, depth: usize, out: &mut String) {
        self.write_indent(out, depth);
        out.push_str("metadata {\n");
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "metadata_pair" => {
                    self.write_indent(out, depth + 1);
                    self.print_metadata_pair(child, out);
                    out.push_str(",\n");
                }
                "comment" => self.emit_comment(child, depth + 1, out),
                _ => {}
            }
        }
        self.write_indent(out, depth);
        out.push('}');
    }

    fn print_metadata_pair(&self, node: Node, out: &mut String) {
        // metadata_pair = identifier (= | :) (value | array | record) ,?
        let sep = self.first_separator(node);
        let mut key: Option<&str> = None;
        let mut value_node: Option<Node> = None;
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "identifier" if key.is_none() => key = Some(self.text(child)),
                "value" | "array" | "record" => value_node = Some(child),
                _ => {}
            }
        }
        match (key, value_node) {
            (Some(k), Some(v)) => {
                out.push_str(k);
                if sep == "=" {
                    out.push_str(" = ");
                } else {
                    out.push_str(": ");
                }
                out.push_str(self.text(v).trim());
            }
            _ => out.push_str(self.text(node).trim()),
        }
    }

    fn print_agent_definition(&self, node: Node, depth: usize, out: &mut String) {
        // agent_definition = 'agent' identifier ('(' parameter,* ')')?
        //                    ('->' type)? '{' _agent_item* '}'
        let mut cursor = node.walk();
        let mut name: Option<&str> = None;
        let mut params: Vec<Node> = Vec::new();
        let mut return_type: Option<Node> = None;
        let mut items: Vec<Node> = Vec::new();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "identifier" if name.is_none() => name = Some(self.text(child)),
                "parameter" => params.push(child),
                "type" => return_type = Some(child),
                "capabilities_declaration"
                | "policy_definition"
                | "function_definition"
                | "with_block"
                | "comment" => items.push(child),
                _ => {}
            }
        }

        self.write_indent(out, depth);
        out.push_str("agent ");
        out.push_str(name.unwrap_or(""));
        if !params.is_empty() {
            out.push('(');
            let parts: Vec<String> = params
                .iter()
                .map(|p| self.text(*p).trim().to_string())
                .collect();
            out.push_str(&parts.join(", "));
            out.push(')');
        }
        if let Some(rt) = return_type {
            out.push_str(" -> ");
            out.push_str(self.text(rt).trim());
        }
        out.push_str(" {\n");
        let mut prev_kind: Option<&str> = None;
        for item in items {
            if let Some(prev) = prev_kind {
                if prev != "comment" {
                    out.push('\n');
                }
            }
            prev_kind = Some(item.kind());
            self.print_agent_item(item, depth + 1, out);
        }
        self.write_indent(out, depth);
        out.push('}');
    }

    fn print_agent_item(&self, node: Node, depth: usize, out: &mut String) {
        match node.kind() {
            "capabilities_declaration" => self.print_capabilities(node, depth, out),
            "with_block" => self.print_with_block(node, depth, out),
            "comment" => self.emit_comment(node, depth, out),
            // policy_definition, function_definition: verbatim
            _ => self.emit_verbatim(node, depth, out),
        }
    }

    fn print_capabilities(&self, node: Node, depth: usize, out: &mut String) {
        // capabilities_declaration = 'capabilities' (= | :) array
        let sep = self.first_separator(node);
        let mut array_node: Option<Node> = None;
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() == "array" {
                array_node = Some(child);
            }
        }
        self.write_indent(out, depth);
        out.push_str("capabilities");
        if sep == "=" {
            out.push_str(" = ");
        } else {
            out.push_str(": ");
        }
        if let Some(arr) = array_node {
            let mut cursor2 = arr.walk();
            let parts: Vec<String> = arr
                .named_children(&mut cursor2)
                .map(|e| self.text(e).trim().to_string())
                .collect();
            out.push('[');
            out.push_str(&parts.join(", "));
            out.push(']');
        }
        out.push('\n');
    }

    fn print_with_block(&self, node: Node, depth: usize, out: &mut String) {
        // with_block = 'with' (with_attribute ','?)* block
        let mut cursor = node.walk();
        let mut attrs: Vec<Node> = Vec::new();
        let mut block: Option<Node> = None;
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "with_attribute" => attrs.push(child),
                "block" => block = Some(child),
                _ => {}
            }
        }
        self.write_indent(out, depth);
        out.push_str("with ");
        let attr_strings: Vec<String> = attrs
            .iter()
            .map(|a| self.format_with_attribute(*a))
            .collect();
        out.push_str(&attr_strings.join(", "));
        if let Some(b) = block {
            out.push(' ');
            // Strip braces and check whether the body has any non-whitespace
            // content. Empty blocks render canonically; non-empty bodies are
            // echoed verbatim from the source so we do not have to format
            // arbitrary statement / expression sequences.
            let body = self.text(b);
            let inner = body.trim_start_matches('{').trim_end_matches('}').trim();
            if inner.is_empty() {
                out.push_str("{\n");
                self.write_indent(out, depth);
                out.push('}');
            } else {
                out.push_str(body);
            }
        }
        out.push('\n');
    }

    fn format_with_attribute(&self, node: Node) -> String {
        // with_attribute = identifier '=' (value | array)
        let mut cursor = node.walk();
        let mut key: Option<&str> = None;
        let mut value: Option<&str> = None;
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "identifier" if key.is_none() => key = Some(self.text(child)),
                "value" | "array" => value = Some(self.text(child).trim()),
                _ => {}
            }
        }
        match (key, value) {
            (Some(k), Some(v)) => format!("{} = {}", k, v),
            _ => self.text(node).trim().to_string(),
        }
    }

    fn write_indent(&self, out: &mut String, depth: usize) {
        for _ in 0..depth {
            out.push_str(INDENT);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_trivial_metadata() {
        let input = "metadata{ version:\"1.0\" }\n";
        let out = format_source(input).unwrap();
        assert_eq!(out, "metadata {\n    version: \"1.0\",\n}\n");
    }

    #[test]
    fn preserves_equals_separator_in_metadata() {
        let input = "metadata { version = \"1.0\" }\n";
        let out = format_source(input).unwrap();
        assert!(out.contains("version = \"1.0\""), "output: {out}");
    }

    #[test]
    fn echoes_unparsable_input_unchanged() {
        // Stray closing brace at the top level is unparseable.
        let input = "metadata { version: \"1.0\" } }\n";
        let out = format_source(input).unwrap();
        assert_eq!(out, input);
    }
}
