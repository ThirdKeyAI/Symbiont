//! `symbi agents-md` subcommand — generate AGENTS.md from DSL agent definitions.

use std::path::Path;

const AUTO_START: &str = "<!-- agents-md:auto-start -->";
const AUTO_END: &str = "<!-- agents-md:auto-end -->";
const SENSITIVE_START: &str = "<!-- agents-md:sensitive-start -->";
const SENSITIVE_END: &str = "<!-- agents-md:sensitive-end -->";

/// Run the `agents-md generate` subcommand.
pub fn run(sub_matches: &clap::ArgMatches) {
    match sub_matches.subcommand() {
        Some(("generate", gen_matches)) => {
            let dir = gen_matches
                .get_one::<String>("dir")
                .map(|s| s.as_str())
                .unwrap_or(".");
            let output = gen_matches
                .get_one::<String>("output")
                .map(|s| s.as_str())
                .unwrap_or("AGENTS.md");
            generate(dir, output);
        }
        _ => {
            eprintln!("Usage: symbi agents-md generate [--dir <PATH>] [--output <FILE>]");
            std::process::exit(1);
        }
    }
}

/// Scan agent DSL files and generate the auto-generated section.
fn generate(dir: &str, output_path: &str) {
    let agents_dir = Path::new(dir).join("agents");
    if !agents_dir.exists() || !agents_dir.is_dir() {
        eprintln!(
            "No agents/ directory found in '{}'. Nothing to generate.",
            dir
        );
        std::process::exit(1);
    }

    // Collect and parse all .dsl files
    let mut agents = Vec::new();

    let mut entries: Vec<_> = std::fs::read_dir(&agents_dir)
        .unwrap_or_else(|e| {
            eprintln!("Failed to read agents/ directory: {}", e);
            std::process::exit(1);
        })
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "dsl"))
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        let filename = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let name = filename.strip_suffix(".dsl").unwrap_or(&filename);

        let source = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Warning: failed to read {}: {}", path.display(), e);
                continue;
            }
        };

        let tree = match dsl::parse_dsl(&source) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("Warning: failed to parse {}: {}", filename, e);
                continue;
            }
        };

        let metadata = dsl::extract_metadata(&tree, &source);
        let with_blocks = dsl::extract_with_blocks(&tree, &source).unwrap_or_default();
        let schedules = dsl::extract_schedule_definitions(&tree, &source).unwrap_or_default();
        let channels = dsl::extract_channel_definitions(&tree, &source).unwrap_or_default();
        let webhooks = dsl::extract_webhook_definitions(&tree, &source).unwrap_or_default();
        let memories = dsl::extract_memory_definitions(&tree, &source).unwrap_or_default();

        agents.push(AgentInfo {
            name: name.to_string(),
            source,
            metadata,
            with_blocks,
            schedules,
            channels,
            webhooks,
            memories,
        });
    }

    if agents.is_empty() {
        eprintln!("No .dsl files found in {}", agents_dir.display());
        std::process::exit(1);
    }

    // Build the auto-generated markdown
    let generated = build_generated_section(&agents);

    // Write or merge into existing file
    let output_file = Path::new(dir).join(output_path);
    if output_file.exists() {
        let existing = std::fs::read_to_string(&output_file).unwrap_or_default();
        let merged = merge_generated(&existing, &generated);
        std::fs::write(&output_file, merged).unwrap_or_else(|e| {
            eprintln!("Failed to write {}: {}", output_file.display(), e);
            std::process::exit(1);
        });
        println!(
            "Updated auto-generated section in {}",
            output_file.display()
        );
    } else {
        std::fs::write(&output_file, &generated).unwrap_or_else(|e| {
            eprintln!("Failed to write {}: {}", output_file.display(), e);
            std::process::exit(1);
        });
        println!("Generated {}", output_file.display());
    }
}

struct AgentInfo {
    name: String,
    source: String,
    metadata: std::collections::HashMap<String, String>,
    with_blocks: Vec<dsl::WithBlock>,
    schedules: Vec<dsl::ScheduleDefinition>,
    channels: Vec<dsl::ChannelDefinition>,
    webhooks: Vec<dsl::WebhookDefinition>,
    memories: Vec<dsl::MemoryDefinition>,
}

fn build_generated_section(agents: &[AgentInfo]) -> String {
    let mut out = String::new();
    out.push_str(AUTO_START);
    out.push('\n');

    // --- Agents table ---
    out.push_str("\n## Agents\n\n");
    out.push_str("| Name | Description |\n");
    out.push_str("|------|-------------|\n");
    for agent in agents {
        let desc = agent
            .metadata
            .get("description")
            .map(|s| s.trim_matches('"').to_string())
            .or_else(|| extract_description_from_source(&agent.source))
            .unwrap_or_else(|| "-".to_string());
        out.push_str(&format!("| `{}` | {} |\n", agent.name, desc));
    }

    // --- Sensitive section: sandbox tiers, schedules, webhooks ---
    out.push('\n');
    out.push_str(SENSITIVE_START);
    out.push('\n');

    // Sandbox tiers (tree-sitter + source-text fallback)
    {
        let mut sandbox_rows: Vec<(&str, String, String)> = Vec::new();
        for agent in agents {
            let ts_sandbox = agent.with_blocks.iter().find_map(|wb| {
                wb.sandbox_tier.as_ref().map(|tier| {
                    let timeout = wb
                        .timeout
                        .map(|t| format!("{}s", t))
                        .unwrap_or_else(|| "-".to_string());
                    (format!("{}", tier), timeout)
                })
            });
            if let Some((tier, timeout)) = ts_sandbox {
                sandbox_rows.push((&agent.name, tier, timeout));
            } else {
                let (sb, to) = extract_with_info_from_source(&agent.source);
                if let Some(tier) = sb {
                    let timeout = to
                        .map(|t| format!("{}s", t))
                        .unwrap_or_else(|| "-".to_string());
                    sandbox_rows.push((&agent.name, tier, timeout));
                }
            }
        }
        if !sandbox_rows.is_empty() {
            out.push_str("\n### Sandbox Configuration\n\n");
            out.push_str("| Agent | Sandbox | Timeout |\n");
            out.push_str("|-------|---------|--------|\n");
            for (name, tier, timeout) in &sandbox_rows {
                out.push_str(&format!("| `{}` | {} | {} |\n", name, tier, timeout));
            }
        }
    }

    // Schedules (tree-sitter + source-text fallback)
    {
        let mut schedule_rows: Vec<(String, String, String, String)> = Vec::new();
        for agent in agents {
            if !agent.schedules.is_empty() {
                for sched in &agent.schedules {
                    let expr = sched.cron.as_deref().or(sched.at.as_deref()).unwrap_or("-");
                    let target = sched.agent.as_deref().unwrap_or(&agent.name);
                    schedule_rows.push((
                        sched.name.clone(),
                        target.to_string(),
                        expr.to_string(),
                        sched.timezone.clone(),
                    ));
                }
            } else {
                for (name, cron) in extract_schedules_from_source(&agent.source) {
                    schedule_rows.push((name, agent.name.clone(), cron, "UTC".to_string()));
                }
            }
        }
        if !schedule_rows.is_empty() {
            out.push_str("\n### Schedules\n\n");
            out.push_str("| Name | Agent | Expression | Timezone |\n");
            out.push_str("|------|-------|------------|----------|\n");
            for (name, target, expr, tz) in &schedule_rows {
                out.push_str(&format!(
                    "| `{}` | `{}` | `{}` | {} |\n",
                    name, target, expr, tz
                ));
            }
        }
    }

    // Webhooks (tree-sitter + source-text fallback)
    {
        let mut webhook_rows: Vec<(String, String, String, String)> = Vec::new();
        for agent in agents {
            if !agent.webhooks.is_empty() {
                for wh in &agent.webhooks {
                    let target = wh.agent.as_deref().unwrap_or(&agent.name);
                    webhook_rows.push((
                        wh.name.clone(),
                        wh.path.clone(),
                        format!("{:?}", wh.provider),
                        target.to_string(),
                    ));
                }
            } else {
                for (name, path, provider) in extract_webhooks_from_source(&agent.source) {
                    webhook_rows.push((name, path, provider, agent.name.clone()));
                }
            }
        }
        if !webhook_rows.is_empty() {
            out.push_str("\n### Webhooks\n\n");
            out.push_str("| Name | Path | Provider | Agent |\n");
            out.push_str("|------|------|----------|-------|\n");
            for (name, path, provider, target) in &webhook_rows {
                out.push_str(&format!(
                    "| `{}` | `{}` | {} | `{}` |\n",
                    name, path, provider, target
                ));
            }
        }
    }

    out.push_str(SENSITIVE_END);
    out.push('\n');

    // --- Non-sensitive sections ---

    // Channels (platform names only, no sensitive details)
    let all_channels: Vec<_> = agents.iter().flat_map(|a| a.channels.iter()).collect();
    if !all_channels.is_empty() {
        out.push_str("\n## Channels\n\n");
        for ch in &all_channels {
            let platform = ch.platform.as_deref().unwrap_or("unknown");
            let agent = ch
                .default_agent
                .as_deref()
                .map(|a| format!(" (agent: `{}`)", a))
                .unwrap_or_default();
            out.push_str(&format!("- **{}**: `{}`{}\n", ch.name, platform, agent));
        }
    }

    // Memory stores (filter garbled tree-sitter entries, fall back to source)
    {
        let mut memory_rows: Vec<(String, String, String)> = Vec::new();
        for agent in agents {
            // Filter tree-sitter memories: reject garbled names containing `=` or newlines
            let valid_ts: Vec<_> = agent
                .memories
                .iter()
                .filter(|m| !m.name.contains('=') && !m.name.contains('\n'))
                .collect();
            if !valid_ts.is_empty() {
                for mem in valid_ts {
                    memory_rows.push((
                        mem.name.clone(),
                        format!("{:?}", mem.store),
                        format!("{}", mem.path.display()),
                    ));
                }
            } else {
                for (name, store, path) in extract_memory_from_source(&agent.source) {
                    memory_rows.push((name, store, path));
                }
            }
        }
        if !memory_rows.is_empty() {
            out.push_str("\n## Memory Stores\n\n");
            for (name, store, path) in &memory_rows {
                out.push_str(&format!("- **{}**: `{}` at `{}`\n", name, store, path));
            }
        }
    }

    // Invocation
    out.push_str("\n## Invocation\n\n");
    out.push_str("```bash\n");
    out.push_str("# MCP (Claude Code, Cursor, etc.)\n");
    out.push_str("symbi mcp\n\n");
    out.push_str("# HTTP API\n");
    out.push_str("curl -X POST http://localhost:8080/api/v1/agents/<id>/execute \\\n");
    out.push_str("  -H 'Authorization: Bearer $TOKEN' \\\n");
    out.push_str("  -d '{\"input\": \"your prompt\"}'\n\n");
    out.push_str("# DSL parse\n");
    out.push_str("symbi dsl -f agents/<name>.dsl\n");
    out.push_str("```\n");

    out.push_str(AUTO_END);
    out.push('\n');

    out
}

/// Merge the generated section into an existing file, replacing the auto-generated block.
fn merge_generated(existing: &str, generated: &str) -> String {
    if let (Some(start), Some(end)) = (existing.find(AUTO_START), existing.find(AUTO_END)) {
        let end_pos = end + AUTO_END.len();
        // Skip the trailing newline after the end marker if present
        let end_pos = if existing[end_pos..].starts_with('\n') {
            end_pos + 1
        } else {
            end_pos
        };
        format!(
            "{}{}{}",
            &existing[..start],
            generated,
            &existing[end_pos..]
        )
    } else {
        // No existing markers — append to end
        format!("{}\n{}", existing.trim_end(), generated)
    }
}

/// Extract the auto-generated section from AGENTS.md content.
/// Returns only the content between the markers, or None if not found.
pub fn extract_auto_section(content: &str) -> Option<&str> {
    let start = content.find(AUTO_START)?;
    let end = content.find(AUTO_END)?;
    if end <= start {
        return None;
    }
    let section_start = start + AUTO_START.len();
    Some(content[section_start..end].trim())
}

/// Strip sensitive content from AGENTS.md for filtered serving.
/// Removes content between sensitive markers.
#[allow(dead_code)]
pub fn strip_sensitive(content: &str) -> String {
    let mut result = content.to_string();
    while let (Some(start), Some(end)) = (result.find(SENSITIVE_START), result.find(SENSITIVE_END))
    {
        if end <= start {
            break;
        }
        let end_pos = end + SENSITIVE_END.len();
        let end_pos = if result[end_pos..].starts_with('\n') {
            end_pos + 1
        } else {
            end_pos
        };
        // Also remove the sensitive-start marker's preceding newline if present
        let start_pos = if start > 0 && result.as_bytes()[start - 1] == b'\n' {
            start - 1
        } else {
            start
        };
        result = format!("{}{}", &result[..start_pos], &result[end_pos..]);
    }
    result
}

// --- Source-text fallback extraction ---
//
// The tree-sitter grammar has known issues:
// - metadata_pair expects `:` but DSL files use `=`, producing ERROR nodes
// - `with memory = "persistent"` is mis-parsed as memory_definition nodes
// - Inline unnamed blocks (schedule/webhook/memory) inside agent bodies are not
//   in _agent_item, so they produce ERROR nodes
//
// These functions parse the raw DSL source as a reliable fallback.

/// Extract description from raw DSL source text.
/// Handles both `description = "..."` and `description: "..."` syntax.
fn extract_description_from_source(source: &str) -> Option<String> {
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("description") {
            let first = rest.chars().next()?;
            if first.is_alphanumeric() || first == '_' {
                continue; // part of a longer identifier
            }
            let rest = rest.trim_start();
            if let Some(after_sep) = rest.strip_prefix('=').or_else(|| rest.strip_prefix(':')) {
                let after_sep = after_sep.trim_start();
                if let Some(quoted) = after_sep.strip_prefix('"') {
                    if let Some(end) = quoted.find('"') {
                        return Some(quoted[..end].to_string());
                    }
                }
            }
        }
    }
    None
}

/// Extract sandbox tier and timeout from `with` attribute lines in source.
fn extract_with_info_from_source(source: &str) -> (Option<String>, Option<u64>) {
    let mut sandbox = None;
    let mut timeout = None;

    for line in source.lines() {
        let trimmed = line.trim();
        if sandbox.is_none() {
            if let Some(val) = extract_kv(trimmed, "sandbox") {
                sandbox = Some(val);
            }
        }
        if timeout.is_none() {
            if let Some(val) = extract_kv(trimmed, "timeout") {
                if let Ok(t) = val.parse::<u64>() {
                    timeout = Some(t);
                }
            }
        }
    }

    (sandbox, timeout)
}

/// Extract schedules from inline `schedule { cron "..." }` blocks in source.
/// Returns Vec of (name, cron_expression).
fn extract_schedules_from_source(source: &str) -> Vec<(String, String)> {
    let mut results = Vec::new();
    for (name, content) in find_keyword_blocks(source, "schedule") {
        if let Some(cron) = extract_block_value(&content, "cron") {
            results.push((name, cron));
        }
    }
    results
}

/// Extract webhooks from inline `webhook { provider X path "..." }` blocks.
/// Returns Vec of (name, path, provider).
fn extract_webhooks_from_source(source: &str) -> Vec<(String, String, String)> {
    let mut results = Vec::new();
    for (name, content) in find_keyword_blocks(source, "webhook") {
        let path = extract_block_value(&content, "path").unwrap_or_default();
        let provider = extract_block_value(&content, "provider").unwrap_or_else(|| "custom".into());
        if !path.is_empty() {
            results.push((name, path, provider));
        }
    }
    results
}

/// Extract memory stores from standalone `memory { store X path "..." }` blocks.
/// Skips `with memory = "..."` patterns (those are with-block attributes).
/// Returns Vec of (name, store_type, path).
fn extract_memory_from_source(source: &str) -> Vec<(String, String, String)> {
    let mut results = Vec::new();
    for (name, content) in find_keyword_blocks(source, "memory") {
        let store = extract_block_value(&content, "store").unwrap_or_else(|| "markdown".into());
        let path = extract_block_value(&content, "path").unwrap_or_default();
        if !path.is_empty() {
            results.push((name, store, path));
        }
    }
    results
}

/// Extract a value from `key = "value"`, `key: "value"`, or `key value` patterns.
/// Checks word boundaries around the key.
fn extract_kv(line: &str, key: &str) -> Option<String> {
    let idx = line.find(key)?;
    // Word boundary before
    if idx > 0 {
        let prev = line.as_bytes()[idx - 1];
        if prev.is_ascii_alphanumeric() || prev == b'_' {
            return None;
        }
    }
    // Word boundary after
    let after = idx + key.len();
    if after < line.len() {
        let next = line.as_bytes()[after];
        if next.is_ascii_alphanumeric() || next == b'_' {
            return None;
        }
    }

    let rest = line[after..].trim_start();
    let after_sep = rest
        .strip_prefix('=')
        .or_else(|| rest.strip_prefix(':'))
        .unwrap_or(rest)
        .trim_start();
    // Strip trailing comma
    let after_sep = after_sep.trim_end_matches(',').trim_end();

    if let Some(quoted) = after_sep.strip_prefix('"') {
        let end = quoted.find('"')?;
        return Some(quoted[..end].to_string());
    }
    // Bare value — take until comma or whitespace
    let end = after_sep
        .find(|c: char| c == ',' || c.is_whitespace())
        .unwrap_or(after_sep.len());
    let v = &after_sep[..end];
    if v.is_empty() {
        None
    } else {
        Some(v.to_string())
    }
}

/// Extract a value for a key within block content.
/// Handles space-separated (`key "value"`), colon (`key: "value"`), and
/// equals (`key = "value"`) styles.
fn extract_block_value(content: &str, key: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(key) {
            // Ensure word boundary
            if let Some(first) = rest.chars().next() {
                if first.is_alphanumeric() || first == '_' {
                    continue;
                }
            }
            let rest = rest.trim_start();
            // Skip optional separator
            let rest = rest
                .strip_prefix('=')
                .or_else(|| rest.strip_prefix(':'))
                .unwrap_or(rest)
                .trim_start();

            if let Some(quoted) = rest.strip_prefix('"') {
                if let Some(end) = quoted.find('"') {
                    return Some(quoted[..end].to_string());
                }
            }
            // Bare value
            let end = rest
                .find(|c: char| c.is_whitespace() || c == ',')
                .unwrap_or(rest.len());
            let val = &rest[..end];
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

/// Find all brace-delimited blocks for a keyword in source text.
/// Returns Vec of (name, block_content).
///
/// Matches `keyword { ... }` (unnamed → name defaults to keyword)
/// and `keyword name { ... }` (named).
/// Skips `with keyword = ...` patterns where the keyword is a with-attribute.
fn find_keyword_blocks(source: &str, keyword: &str) -> Vec<(String, String)> {
    let mut results = Vec::new();
    let mut search_from = 0;
    let kw_len = keyword.len();

    while search_from < source.len() {
        let remaining = &source[search_from..];
        let rel_idx = match remaining.find(keyword) {
            Some(i) => i,
            None => break,
        };
        let abs_idx = search_from + rel_idx;
        search_from = abs_idx + kw_len;

        // Word boundary before
        if abs_idx > 0 {
            let prev = source.as_bytes()[abs_idx - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' {
                continue;
            }
        }
        // Word boundary after
        let after = abs_idx + kw_len;
        if after < source.len() {
            let next = source.as_bytes()[after];
            if next.is_ascii_alphanumeric() || next == b'_' {
                continue;
            }
        }

        // Skip if inside a comment (// ...)
        let line_start = source[..abs_idx].rfind('\n').map_or(0, |p| p + 1);
        let line_prefix = source[line_start..abs_idx].trim_start();
        if line_prefix.starts_with("//") {
            continue;
        }

        let rest = source[after..].trim_start();
        let rest_abs = source.len() - rest.len();

        // Skip with-attributes: keyword followed by `=` or `:`
        if rest.starts_with('=') || rest.starts_with(':') {
            continue;
        }

        let (name, brace_abs) = if rest.starts_with('{') {
            // Unnamed block
            (keyword.to_string(), rest_abs)
        } else {
            // Try to read identifier before `{`
            let name_end = rest
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .unwrap_or(0);
            if name_end == 0 {
                continue;
            }
            let n = &rest[..name_end];
            let after_name = rest[name_end..].trim_start();
            if !after_name.starts_with('{') {
                continue;
            }
            let brace = source.len() - after_name.len();
            (n.to_string(), brace)
        };

        // Track braces to find matching close
        let block_src = &source[brace_abs..];
        let mut depth = 0i32;
        let mut end_idx = None;
        for (i, ch) in block_src.char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end_idx = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }

        if let Some(end) = end_idx {
            let content = &block_src[1..end]; // skip opening brace
            results.push((name, content.to_string()));
            search_from = brace_abs + end + 1;
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_generated_appends_when_no_markers() {
        let existing = "# My Project\n\nSome hand-written content.";
        let generated = "<!-- agents-md:auto-start -->\n## Agents\n<!-- agents-md:auto-end -->\n";
        let result = merge_generated(existing, generated);
        assert!(result.starts_with("# My Project"));
        assert!(result.contains(AUTO_START));
        assert!(result.contains("## Agents"));
    }

    #[test]
    fn test_merge_generated_replaces_existing_markers() {
        let existing = "# Header\n\n<!-- agents-md:auto-start -->\nold content\n<!-- agents-md:auto-end -->\n\n# Footer\n";
        let generated = "<!-- agents-md:auto-start -->\nnew content\n<!-- agents-md:auto-end -->\n";
        let result = merge_generated(existing, generated);
        assert!(result.contains("new content"));
        assert!(!result.contains("old content"));
        assert!(result.contains("# Header"));
        assert!(result.contains("# Footer"));
    }

    #[test]
    fn test_extract_auto_section() {
        let content = "# Header\n<!-- agents-md:auto-start -->\n## Agents\nsome content\n<!-- agents-md:auto-end -->\n# Footer";
        let section = extract_auto_section(content).unwrap();
        assert!(section.contains("## Agents"));
        assert!(section.contains("some content"));
        assert!(!section.contains("# Header"));
    }

    #[test]
    fn test_extract_auto_section_missing() {
        assert!(extract_auto_section("no markers here").is_none());
    }

    #[test]
    fn test_strip_sensitive() {
        let content = "## Agents\n\n<!-- agents-md:sensitive-start -->\n### Sandbox\nsecret stuff\n<!-- agents-md:sensitive-end -->\n\n## Channels\npublic";
        let result = strip_sensitive(content);
        assert!(!result.contains("secret stuff"));
        assert!(!result.contains("Sandbox"));
        assert!(result.contains("## Agents"));
        assert!(result.contains("## Channels"));
    }

    #[test]
    fn test_extract_description_equals() {
        let src = r#"metadata {
    version = "1.0"
    description = "Schema validation engine"
    tags = ["validation"]
}"#;
        assert_eq!(
            extract_description_from_source(src),
            Some("Schema validation engine".to_string())
        );
    }

    #[test]
    fn test_extract_description_colon() {
        let src = r#"metadata {
    description: "My agent"
}"#;
        assert_eq!(
            extract_description_from_source(src),
            Some("My agent".to_string())
        );
    }

    #[test]
    fn test_extract_description_none() {
        let src = "agent foo(x: String) -> Result { }";
        assert_eq!(extract_description_from_source(src), None);
    }

    #[test]
    fn test_extract_description_word_boundary() {
        // "data_description" should NOT match as "description"
        let src = r#"data_description = "not this""#;
        assert_eq!(extract_description_from_source(src), None);
    }

    #[test]
    fn test_extract_with_info() {
        let src = r#"    with memory = "persistent", security = "high", sandbox = "Tier1",
         timeout = 300000, max_memory_mb = 512 { }"#;
        let (sb, to) = extract_with_info_from_source(src);
        assert_eq!(sb, Some("Tier1".to_string()));
        assert_eq!(to, Some(300000));
    }

    #[test]
    fn test_extract_with_info_multiline() {
        let src = r#"    with
        memory = "persistent",
        sandbox = "Tier2",
        timeout = 60000
    { }"#;
        let (sb, to) = extract_with_info_from_source(src);
        assert_eq!(sb, Some("Tier2".to_string()));
        assert_eq!(to, Some(60000));
    }

    #[test]
    fn test_extract_schedules_unnamed() {
        let src = r#"agent foo(body: JSON) -> Report {
    schedule {
        cron "0 6 * * *"
        max_jitter 300
    }
    with sandbox = "Tier1" { }
}"#;
        let schedules = extract_schedules_from_source(src);
        assert_eq!(schedules.len(), 1);
        assert_eq!(schedules[0].0, "schedule");
        assert_eq!(schedules[0].1, "0 6 * * *");
    }

    #[test]
    fn test_extract_schedules_named() {
        let src = r#"schedule daily_report {
    cron: "0 9 * * *"
    timezone: "UTC"
    agent: "reporter"
}"#;
        let schedules = extract_schedules_from_source(src);
        assert_eq!(schedules.len(), 1);
        assert_eq!(schedules[0].0, "daily_report");
        assert_eq!(schedules[0].1, "0 9 * * *");
    }

    #[test]
    fn test_extract_webhooks_unnamed() {
        let src = r#"    webhook {
        provider slack
        secret   $SLACK_SIGNING_SECRET
        path     "/hooks/slack"
        filter   ["message", "app_mention"]
    }"#;
        let webhooks = extract_webhooks_from_source(src);
        assert_eq!(webhooks.len(), 1);
        assert_eq!(webhooks[0].0, "webhook");
        assert_eq!(webhooks[0].1, "/hooks/slack");
        assert_eq!(webhooks[0].2, "slack");
    }

    #[test]
    fn test_extract_webhooks_named() {
        let src = r#"webhook github_events {
    path     "/hooks/github"
    provider github
    secret   "vault://webhooks/github/secret"
    agent    deployer
}"#;
        let webhooks = extract_webhooks_from_source(src);
        assert_eq!(webhooks.len(), 1);
        assert_eq!(webhooks[0].0, "github_events");
        assert_eq!(webhooks[0].1, "/hooks/github");
        assert_eq!(webhooks[0].2, "github");
    }

    #[test]
    fn test_extract_memory_standalone() {
        let src = r#"memory knowledge_store {
    store     markdown
    path      "data/knowledge"
    retention 365d
}"#;
        let mems = extract_memory_from_source(src);
        assert_eq!(mems.len(), 1);
        assert_eq!(mems[0].0, "knowledge_store");
        assert_eq!(mems[0].1, "markdown");
        assert_eq!(mems[0].2, "data/knowledge");
    }

    #[test]
    fn test_extract_memory_inline_unnamed() {
        let src = r#"agent foo(body: JSON) -> Report {
    memory {
        store markdown
        path  "/var/lib/symbi/memory/compliance"
        retention 2555d
    }
    with memory = "persistent", sandbox = "Tier1" { }
}"#;
        let mems = extract_memory_from_source(src);
        assert_eq!(mems.len(), 1);
        assert_eq!(mems[0].0, "memory");
        assert_eq!(mems[0].2, "/var/lib/symbi/memory/compliance");
    }

    #[test]
    fn test_extract_memory_skips_with_attribute() {
        // `with memory = "persistent"` should NOT be extracted as a memory block
        let src = r#"    with memory = "persistent", sandbox = "Tier1" { }"#;
        let mems = extract_memory_from_source(src);
        assert!(mems.is_empty());
    }

    #[test]
    fn test_find_keyword_blocks_skips_comments() {
        let src = r#"// memory block for testing
memory real_store {
    store markdown
    path "data/test"
}"#;
        let blocks = find_keyword_blocks(src, "memory");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].0, "real_store");
    }

    #[test]
    fn test_extract_kv_word_boundary() {
        assert!(extract_kv("max_timeout = 100", "timeout").is_none());
        assert_eq!(
            extract_kv("timeout = 100", "timeout"),
            Some("100".to_string())
        );
    }
}
