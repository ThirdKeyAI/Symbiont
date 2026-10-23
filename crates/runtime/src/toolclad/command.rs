//! Command construction for oneshot ToolClad manifests.
//!
//! The argv array is the only invocation form that reaches a process. `exec`
//! declares it directly; the deprecated `template` string is tokenized once,
//! on trusted manifest text, before any argument value exists. A placeholder
//! then resolves to exactly one argv entry and the result is never re-split.
//!
//! Two placeholder classes exist:
//!
//! * manifest-controlled, multi-token: `{_<arg>_flags}` from
//!   `[command.mappings.<arg>]` and `{_<cond>}` from
//!   `[command.conditionals.<cond>]`. They must stand alone as a whole token;
//! * agent-controlled, single-token: every `[args.*]` value and every
//!   `[command.defaults]` entry, substituted inside one argv entry.
//!
//! `{{` and `}}` are escapes for literal braces. Any other brace is a load
//! error, so a typo can never reach argv as a literal token.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use super::manifest::{ConditionalDef, Manifest};

/// One left-to-right pass over a token: a brace escape or a `{name}`
/// placeholder. Escapes are consumed first so `%{{http_code}}` is literal.
static PLACEHOLDER_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\{\{|\}\}|\{(\w+)\}").expect("placeholder pattern"));

/// Executor-injected single-token variables Symbiont supports.
pub const INJECTED_VARS: &[&str] = &["_scan_id"];

/// ToolClad's host-path evidence variables. A Symbiont tool runs inside an
/// isolated boundary where a host path is meaningless, so these are refused
/// at load; write the result to stdout, which is captured as evidence.
pub const UNSUPPORTED_INJECTED_VARS: &[&str] = &["_output_file", "_evidence_dir"];

/// Tokenize a trusted manifest template into argv-shaped tokens.
///
/// POSIX-like: `'...'` and `"..."` group whitespace and `\` escapes the next
/// character outside single quotes. A placeholder inside quotes is refused,
/// because a quoted placeholder lets a value break out of its token.
pub fn tokenize_template(s: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut started = false;
    let mut in_single = false;
    let mut in_double = false;
    let mut chars = s.char_indices().peekable();

    while let Some((i, c)) = chars.next() {
        match c {
            '\\' if !in_single => match chars.next() {
                Some((_, next)) => {
                    cur.push(next);
                    started = true;
                }
                None => return Err("trailing backslash in command template".into()),
            },
            '\'' if !in_double => {
                in_single = !in_single;
                started = true;
            }
            '"' if !in_single => {
                in_double = !in_double;
                started = true;
            }
            '{' if in_single || in_double => {
                if chars.peek().map(|(_, n)| *n) == Some('{') {
                    chars.next();
                    cur.push_str("{{");
                    started = true;
                } else {
                    return Err(format!(
                        "command template places a placeholder inside quotes at offset {i}; \
                         placeholders must be whole, unquoted tokens (use exec = [...])"
                    ));
                }
            }
            c if c.is_whitespace() && !in_single && !in_double => {
                if started {
                    tokens.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            c => {
                cur.push(c);
                started = true;
            }
        }
    }
    if in_single || in_double {
        return Err("unbalanced quote in command template".into());
    }
    if started {
        tokens.push(cur);
    }
    Ok(tokens)
}

/// Placeholder names referenced by one token, skipping `{{`/`}}` escapes.
pub fn placeholder_names(token: &str) -> Vec<String> {
    PLACEHOLDER_RE
        .captures_iter(token)
        .filter_map(|cap| cap.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

/// A brace that is neither an escape nor a well-formed placeholder.
fn stray_brace(token: &str) -> bool {
    PLACEHOLDER_RE.replace_all(token, "").contains(['{', '}'])
}

/// `name` when `token` is exactly one `{name}` placeholder.
fn exact_placeholder(token: &str) -> Option<&str> {
    let cap = PLACEHOLDER_RE.captures(token)?;
    let whole = cap.get(0)?;
    (whole.start() == 0 && whole.end() == token.len())
        .then(|| cap.get(1).map(|m| m.as_str()))
        .flatten()
}

/// The canonical exec form: the declared `exec` array, else the tokenized
/// `template`. `None` when the manifest has neither.
pub fn compiled_exec(manifest: &Manifest) -> Result<Option<Vec<String>>, String> {
    if let Some(exec) = &manifest.command.exec {
        return Ok(Some(exec.clone()));
    }
    manifest
        .command
        .template
        .as_deref()
        .map(tokenize_template)
        .transpose()
}

/// True when the command form is a declared exec array. `allow_whitespace` is
/// honored only then: a template's argv boundaries are provable only after
/// compilation, so ToolClad keeps it strict and so does Symbiont.
pub fn exec_was_declared(manifest: &Manifest) -> bool {
    manifest.command.exec.is_some()
}

fn conditional_tokens(name: &str, cond: &ConditionalDef) -> Result<Vec<String>, String> {
    tokenize_template(&cond.template).map_err(|e| format!("conditional '{name}': {e}"))
}

/// Load-time checks on the command section: `end_of_options`, placeholder
/// resolution, whole-token multi-token variables, brace hygiene, and that
/// every mapping and conditional fragment tokenizes.
pub fn check_command(manifest: &Manifest) -> Result<(), String> {
    let cmd = &manifest.command;
    if cmd.exec.as_ref().is_some_and(Vec::is_empty) {
        return Err("command.exec must not be empty".into());
    }
    if let Some(eoo) = &cmd.end_of_options {
        if eoo != "--" {
            return Err(format!(
                "end_of_options must be exactly \"--\" (got '{eoo}')"
            ));
        }
    }
    let mut single: HashSet<String> = manifest.args.keys().cloned().collect();
    single.extend(cmd.defaults.keys().cloned());
    single.extend(INJECTED_VARS.iter().map(|s| s.to_string()));
    let mut trusted: HashSet<String> = cmd.mappings.keys().map(|k| format!("_{k}_flags")).collect();
    trusted.extend(cmd.conditionals.keys().map(|k| format!("_{k}")));

    let check = |tokens: &[String], context: &str| -> Result<(), String> {
        for token in tokens {
            if stray_brace(token) {
                return Err(format!(
                    "token '{token}' in {context} has a brace that is not a placeholder; \
                     use {{{{ and }}}} for literal braces"
                ));
            }
            for name in placeholder_names(token) {
                if UNSUPPORTED_INJECTED_VARS.contains(&name.as_str()) {
                    return Err(format!(
                        "'{{{name}}}' is not supported: tools run inside an isolated boundary, \
                         so write the result to stdout, which is captured as evidence"
                    ));
                }
                if trusted.contains(&name) {
                    if token != &format!("{{{name}}}") {
                        return Err(format!(
                            "placeholder '{{{name}}}' expands to multiple tokens and must be a whole token in {context}"
                        ));
                    }
                } else if !single.contains(&name) {
                    return Err(format!("unresolved placeholder '{{{name}}}' in {context}"));
                }
            }
        }
        Ok(())
    };

    if let Some(tokens) = compiled_exec(manifest)? {
        check(&tokens, "command")?;
        if let Some(argv0) = tokens.first() {
            if !placeholder_names(argv0).is_empty() {
                return Err("command executable must be fixed by the manifest".into());
            }
        }
    }
    for (name, cond) in &cmd.conditionals {
        let tokens = conditional_tokens(name, cond)?;
        // A conditional may reference single-token values only.
        for token in &tokens {
            for p in placeholder_names(token) {
                if trusted.contains(&p) {
                    return Err(format!(
                        "conditional '{name}' may not reference the multi-token variable '{{{p}}}'"
                    ));
                }
            }
        }
        check(&tokens, &format!("conditional '{name}'"))?;
    }
    for (arg, table) in &cmd.mappings {
        for value in table.values() {
            let tokens =
                tokenize_template(value).map_err(|e| format!("mapping for '{arg}': {e}"))?;
            if tokens
                .iter()
                .any(|t| !placeholder_names(t).is_empty() || stray_brace(t))
            {
                return Err(format!(
                    "mapping for '{arg}' may not contain placeholders or braces"
                ));
            }
        }
    }
    Ok(())
}

/// The invocation-scoped values injected by the executor.
#[derive(Debug, Clone)]
pub struct Injected {
    pub scan_id: String,
}

impl Injected {
    /// A fresh scan identifier with 128 bits of entropy.
    pub fn fresh() -> Self {
        Self {
            scan_id: new_scan_id(),
        }
    }
}

/// `<unix-seconds>-<32 hex>`: sortable, and unguessable.
pub fn new_scan_id() -> String {
    format!(
        "{}-{}",
        chrono::Utc::now().timestamp(),
        uuid::Uuid::new_v4().simple()
    )
}

/// Render a `[command.defaults]` or argument default for argv.
pub(crate) fn toml_value_to_string(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => s.clone(),
        toml::Value::Integer(n) => n.to_string(),
        toml::Value::Float(f) => f.to_string(),
        toml::Value::Boolean(b) => b.to_string(),
        other => other.to_string(),
    }
}

/// Build the argv for one invocation from validated arguments.
///
/// Divergence from the ToolClad reference, kept because it is stricter: a
/// token that is exactly one agent-controlled placeholder and resolves to the
/// empty string keeps its (empty) argv slot instead of being dropped.
/// Dropping it could turn the next operand into the preceding option's value
/// (`-H {headers} {url}` would become `-H <url>`). Optional syntax is omitted
/// through a manifest-controlled conditional instead.
pub fn build_argv(
    manifest: &Manifest,
    args: &HashMap<String, String>,
    injected: &Injected,
) -> Result<Vec<String>, String> {
    check_command(manifest)?;
    let tokens = compiled_exec(manifest)?
        .ok_or("No command exec array or template defined (and no custom executor)")?;

    let mut vars: HashMap<String, String> = HashMap::new();
    for name in manifest.args.keys() {
        if let Some(v) = args.get(name) {
            vars.insert(name.clone(), v.clone());
        }
    }
    for (key, value) in &manifest.command.defaults {
        vars.entry(key.clone())
            .or_insert_with(|| toml_value_to_string(value));
    }
    for (key, def) in &manifest.args {
        if !vars.contains_key(key) {
            if let Some(default) = &def.default {
                vars.insert(key.clone(), toml_value_to_string(default));
            }
        }
    }
    vars.insert("_scan_id".into(), injected.scan_id.clone());

    let mut trusted: HashMap<String, Vec<String>> = HashMap::new();
    for (arg, mapping) in &manifest.command.mappings {
        let tokens = match vars.get(arg).filter(|v| !v.is_empty()) {
            Some(value) => match mapping.get(value) {
                Some(mapped) => tokenize_template(mapped)?,
                None => {
                    return Err(format!(
                    "no mapping for argument '{arg}' value '{value}' in [command.mappings.{arg}]"
                ))
                }
            },
            None => Vec::new(),
        };
        trusted.insert(format!("_{arg}_flags"), tokens);
    }
    let mut names: Vec<&String> = manifest.command.conditionals.keys().collect();
    names.sort();
    for name in names {
        let cond = &manifest.command.conditionals[name];
        let tokens = if evaluate_condition(&cond.when, &vars) {
            let fragment = conditional_tokens(name, cond)?;
            substitute(&fragment, &vars, &HashMap::new(), &HashSet::new())?.0
        } else {
            Vec::new()
        };
        trusted.insert(format!("_{name}"), tokens);
    }

    let user_args: HashSet<&str> = manifest.args.keys().map(String::as_str).collect();
    let (mut argv, first_user_arg) = substitute(&tokens, &vars, &trusted, &user_args)?;
    if manifest.command.end_of_options.is_some() {
        if let Some(index) = first_user_arg {
            argv.insert(index, "--".into());
        }
    }
    if argv.first().is_none_or(String::is_empty) {
        return Err("Empty command after construction".into());
    }
    Ok(argv)
}

/// Substitute one token vector. A whole-token multi-token variable splices
/// its tokens; every other token becomes exactly one argv entry. Returns the
/// index of the first entry produced from a token that was solely an
/// agent-controlled placeholder (for `end_of_options`).
fn substitute(
    tokens: &[String],
    vars: &HashMap<String, String>,
    trusted: &HashMap<String, Vec<String>>,
    user_args: &HashSet<&str>,
) -> Result<(Vec<String>, Option<usize>), String> {
    let mut out = Vec::new();
    let mut first_user_arg = None;
    for token in tokens {
        let exact = exact_placeholder(token);
        if let Some(vec) = exact.and_then(|name| trusted.get(name)) {
            out.extend(vec.iter().cloned());
            continue;
        }
        let mut result = String::with_capacity(token.len());
        let mut last = 0;
        for cap in PLACEHOLDER_RE.captures_iter(token) {
            let whole = cap.get(0).expect("whole match");
            result.push_str(&token[last..whole.start()]);
            match whole.as_str() {
                "{{" => result.push('{'),
                "}}" => result.push('}'),
                _ => {
                    let name = &cap[1];
                    match vars.get(name) {
                        Some(v) => result.push_str(v),
                        None => return Err(format!("unresolved placeholder '{{{name}}}'")),
                    }
                }
            }
            last = whole.end();
        }
        result.push_str(&token[last..]);
        if let Some(name) = exact {
            if first_user_arg.is_none() && user_args.contains(name) {
                first_user_arg = Some(out.len());
            }
        }
        out.push(result);
    }
    Ok((out, first_user_arg))
}

/// Closed-vocabulary condition evaluator: `var == 'lit'`, `var != 'lit'`,
/// joined by `and` / `or` (`and` binds tighter). Never evaluates code.
pub(crate) fn evaluate_condition(when: &str, vars: &HashMap<String, String>) -> bool {
    let when = when.trim();
    if when.contains(" or ") {
        return when
            .split(" or ")
            .any(|part| evaluate_condition(part, vars));
    }
    if when.contains(" and ") {
        return when
            .split(" and ")
            .all(|part| evaluate_condition(part, vars));
    }
    for (op, equal) in [("!=", false), ("==", true)] {
        if let Some((key, literal)) = when.split_once(op) {
            let expected = literal.trim().trim_matches('\'').trim_matches('"');
            let actual = vars.get(key.trim()).map(String::as_str).unwrap_or("");
            return (actual == expected) == equal;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(toml_text: &str) -> Manifest {
        toml::from_str(toml_text).unwrap()
    }

    const HEAD: &str = "[tool]\nname = \"t\"\nversion = \"1\"\nbinary = \"echo\"\ndescription = \"d\"\n[output]\nformat = \"text\"\n";

    #[test]
    fn template_placeholder_inside_quotes_is_refused() {
        assert!(tokenize_template("curl -H '{headers}'").is_err());
        assert!(tokenize_template("curl -H \"{headers}\"").is_err());
        assert_eq!(
            tokenize_template("curl -w '%{{http_code}}' {url}").unwrap(),
            vec!["curl", "-w", "%{{http_code}}", "{url}"]
        );
    }

    #[test]
    fn literal_brace_escapes_reach_argv_as_single_braces() {
        let m = manifest(&format!(
            "{HEAD}[args.url]\ntype = \"string\"\n[command]\nexec = [\"echo\", \"-w\", \"%{{{{http_code}}}}\", \"{{url}}\"]\n"
        ));
        let mut args = HashMap::new();
        args.insert("url".into(), "x".into());
        let argv = build_argv(&m, &args, &Injected::fresh()).unwrap();
        assert_eq!(argv, vec!["echo", "-w", "%{http_code}", "x"]);
    }

    #[test]
    fn unresolved_and_stray_placeholders_are_load_errors() {
        let m = manifest(&format!(
            "{HEAD}[command]\nexec = [\"echo\", \"{{nope}}\"]\n"
        ));
        assert!(check_command(&m).unwrap_err().contains("unresolved"));
        let m = manifest(&format!(
            "{HEAD}[command]\nexec = [\"echo\", \"{{_secret:k}}\"]\n"
        ));
        assert!(check_command(&m).is_err());
        let m = manifest(&format!(
            "{HEAD}[command]\nexec = [\"echo\", \"{{_output_file}}\"]\n"
        ));
        assert!(check_command(&m).unwrap_err().contains("stdout"));
    }

    #[test]
    fn mapping_names_only_use_the_arg_flags_spelling() {
        let m = manifest(&format!(
            "{HEAD}[args.scan_type]\ntype = \"enum\"\nallowed = [\"a\"]\n[command]\nexec = [\"echo\", \"{{_scan_flags}}\"]\n[command.mappings.scan_type]\na = \"-x -y\"\n"
        ));
        assert!(check_command(&m).unwrap_err().contains("unresolved"));
        let m = manifest(&format!(
            "{HEAD}[args.scan_type]\ntype = \"enum\"\nallowed = [\"a\"]\n[command]\nexec = [\"echo\", \"{{_scan_type_flags}}\"]\n[command.mappings.scan_type]\na = \"-x -y\"\n"
        ));
        let mut args = HashMap::new();
        args.insert("scan_type".into(), "a".into());
        assert_eq!(
            build_argv(&m, &args, &Injected::fresh()).unwrap(),
            vec!["echo", "-x", "-y"]
        );
    }

    #[test]
    fn multi_token_variables_must_be_whole_tokens() {
        let m = manifest(&format!(
            "{HEAD}[args.a]\ntype = \"enum\"\nallowed = [\"x\"]\n[command]\nexec = [\"echo\", \"pre{{_a_flags}}\"]\n[command.mappings.a]\nx = \"-x\"\n"
        ));
        assert!(check_command(&m).unwrap_err().contains("whole token"));
    }

    #[test]
    fn end_of_options_precedes_the_first_agent_value() {
        let m = manifest(&format!(
            "{HEAD}[args.p]\ntype = \"string\"\n[command]\nexec = [\"echo\", \"-n\", \"{{p}}\", \"{{p}}\"]\nend_of_options = \"--\"\n"
        ));
        let mut args = HashMap::new();
        args.insert("p".into(), "v".into());
        assert_eq!(
            build_argv(&m, &args, &Injected::fresh()).unwrap(),
            vec!["echo", "-n", "--", "v", "v"]
        );
        let m = manifest(&format!(
            "{HEAD}[command]\nexec = [\"echo\"]\nend_of_options = \"++\"\n"
        ));
        assert!(check_command(&m).is_err());
    }

    #[test]
    fn values_are_never_resplit() {
        let m = manifest(&format!(
            "{HEAD}[args.p]\ntype = \"string\"\n[command]\ntemplate = \"echo {{p}}\"\n"
        ));
        let mut args = HashMap::new();
        args.insert("p".into(), "a b -c 'd'".into());
        assert_eq!(
            build_argv(&m, &args, &Injected::fresh()).unwrap(),
            vec!["echo", "a b -c 'd'"]
        );
    }

    #[test]
    fn executable_is_fixed() {
        let m = manifest(&format!(
            "{HEAD}[args.p]\ntype = \"string\"\n[command]\nexec = [\"{{p}}\"]\n"
        ));
        assert!(check_command(&m).is_err());
    }

    #[test]
    fn scan_ids_carry_128_bits() {
        let id = new_scan_id();
        let hex = id.split_once('-').unwrap().1;
        assert_eq!(hex.len(), 32);
        assert_ne!(id, new_scan_id());
    }

    #[test]
    fn conditions_support_and_or() {
        let mut vars = HashMap::new();
        vars.insert("a".to_string(), "1".to_string());
        assert!(evaluate_condition("a != ''", &vars));
        assert!(evaluate_condition("a == '1' and b == ''", &vars));
        assert!(evaluate_condition("a == '2' or b == ''", &vars));
        assert!(!evaluate_condition("a == '2' or b != ''", &vars));
    }
}
