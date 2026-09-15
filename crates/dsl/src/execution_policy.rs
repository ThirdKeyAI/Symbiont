//! Pure, bounded inline policy rules for normalized ORGA effects.
//!
//! These rules restrict an external authorization gate. They cannot grant
//! tools, waive approval, execute code, or infer properties of untrusted text.

use crate::{parse_dsl, resolve_execution_settings, AgentExecutionSettings};
use serde_json::Value;
use tree_sitter::Node;

const MAX_RULES: usize = 128;
const MAX_TOKENS: usize = 256;
const MAX_DEPTH: usize = 24;

#[derive(Debug, Clone)]
pub struct ExecutionPolicy {
    source: String,
    settings: AgentExecutionSettings,
    blocks: Vec<PolicyBlock>,
}

#[derive(Debug, Clone)]
struct PolicyBlock {
    name: String,
    rules: Vec<Rule>,
}

#[derive(Debug, Clone)]
struct Rule {
    allow: bool,
    selector: Selector,
    condition: Expr,
}

#[derive(Debug, Clone)]
enum Selector {
    All(bool),
    Names(Vec<String>),
}

#[derive(Debug, Clone)]
enum Expr {
    Literal(Value),
    Field(Vec<String>),
    Not(Box<Expr>),
    Binary(String, Box<Expr>, Box<Expr>),
}

impl ExecutionPolicy {
    /// Compile file-wide and selected-agent policies. Sibling policies do not
    /// apply. Every selected rule must belong to the supported pure subset.
    pub fn parse(source: &str, selector: &str) -> Result<Self, String> {
        let settings = resolve_execution_settings(source, selector)?;
        let mut blocks = Vec::new();
        let mut total_rules = 0;
        for (index, selected) in [source, settings.agent_source.as_str()]
            .into_iter()
            .enumerate()
        {
            let tree = parse_dsl(selected).map_err(|error| error.to_string())?;
            let root = if index == 0 {
                tree.root_node()
            } else {
                tree.root_node()
                    .named_child(0)
                    .ok_or("missing selected agent")?
            };
            let mut cursor = root.walk();
            for node in root
                .named_children(&mut cursor)
                .filter(|n| n.kind() == "policy_definition")
            {
                let mut children = node.walk();
                let name = node
                    .named_children(&mut children)
                    .find(|n| n.kind() == "identifier")
                    .ok_or("missing policy name")?;
                let name = selected[name.byte_range()].to_owned();
                if blocks.iter().any(|block: &PolicyBlock| block.name == name) {
                    return Err(format!("duplicate selected policy name {name}"));
                }
                let mut rules = Vec::new();
                let mut children = node.walk();
                for rule in node
                    .named_children(&mut children)
                    .filter(|n| n.kind() == "policy_rule")
                {
                    total_rules += 1;
                    if total_rules > MAX_RULES {
                        return Err("inline policy exceeds 128 rules".into());
                    }
                    let parsed = parse_rule(rule, selected)
                        .map_err(|reason| format!("inline policy {name}: {reason}"))?;
                    rules.push(parsed);
                }
                if rules.is_empty() {
                    return Err(format!("inline policy {name} has no rules"));
                }
                blocks.push(PolicyBlock { name, rules });
            }
        }
        Ok(Self {
            source: source.to_owned(),
            settings,
            blocks,
        })
    }

    pub fn source(&self) -> &str {
        &self.source
    }
    pub fn settings(&self) -> &AgentExecutionSettings {
        &self.settings
    }
    pub fn names(&self) -> Vec<&str> {
        self.blocks
            .iter()
            .map(|block| block.name.as_str())
            .collect()
    }
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Evaluate an effect name against runtime-built data. Each block is an
    /// independent restriction: deny wins; a block with allow rules requires
    /// at least one match. Missing fields and type errors deny the effect.
    pub fn evaluate(&self, effect: &str, data: &Value) -> Result<(), String> {
        for block in &self.blocks {
            let mut allowed = !block.rules.iter().any(|rule| rule.allow);
            for (index, rule) in block.rules.iter().enumerate() {
                let matches = match &rule.selector {
                    Selector::All(value) => *value,
                    Selector::Names(names) => names.iter().any(|name| name == effect),
                };
                if !matches {
                    continue;
                }
                let applies = rule.condition.boolean(data).map_err(|reason| {
                    format!("inline policy {} rule {}: {reason}", block.name, index + 1)
                })?;
                if applies {
                    if !rule.allow {
                        return Err(format!(
                            "inline policy {} rule {} denied effect",
                            block.name,
                            index + 1
                        ));
                    }
                    allowed = true;
                }
            }
            if !allowed {
                return Err(format!(
                    "inline policy {} has no matching allow",
                    block.name
                ));
            }
        }
        Ok(())
    }
}

fn parse_rule(node: Node<'_>, source: &str) -> Result<Rule, String> {
    let mut cursor = node.walk();
    let effect = node
        .children(&mut cursor)
        .find(|n| matches!(n.kind(), "allow" | "deny" | "require" | "audit"))
        .ok_or("missing policy effect")?;
    if !matches!(effect.kind(), "allow" | "deny") {
        return Err(format!(
            "{} rules require an unsupported interpreter",
            effect.kind()
        ));
    }
    let mut cursor = node.walk();
    let expressions: Vec<_> = node
        .named_children(&mut cursor)
        .filter(|n| n.kind() == "expression")
        .collect();
    if !(1..=2).contains(&expressions.len()) {
        return Err("invalid policy rule".into());
    }
    let tokens = expression_tokens(expressions[0], source)?;
    let expr = Parser::parse(&tokens)?;
    let selector = match expr {
        Expr::Literal(Value::Bool(value)) => Selector::All(value),
        Expr::Literal(Value::String(name)) => Selector::Names(vec![name]),
        Expr::Literal(Value::Array(values)) if !values.is_empty() => Selector::Names(
            values
                .into_iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| "policy selectors must be literal strings".to_owned())
                })
                .collect::<Result<_, _>>()?,
        ),
        _ => {
            return Err(
                "policy selector must be a literal string, nonempty string array, or boolean"
                    .into(),
            )
        }
    };
    if matches!(&selector, Selector::Names(names) if names.iter().any(|name| name.is_empty() || name.len() > 256))
    {
        return Err("policy effect names must contain 1 to 256 bytes".into());
    }
    let condition = expressions
        .get(1)
        .map(|node| Parser::parse(&expression_tokens(*node, source)?))
        .transpose()?
        .unwrap_or(Expr::Literal(Value::Bool(true)));
    Ok(Rule {
        allow: effect.kind() == "allow",
        selector,
        condition,
    })
}

/// Use grammar tokens, preserving string literals and discarding comments.
/// The grammar's hidden precedence nodes flatten binary expressions, so the
/// bounded parser below reconstructs only the explicitly supported operators.
fn expression_tokens<'a>(node: Node<'_>, source: &'a str) -> Result<Vec<&'a str>, String> {
    fn visit<'a>(
        node: Node<'_>,
        source: &'a str,
        depth: usize,
        out: &mut Vec<&'a str>,
    ) -> Result<(), String> {
        if depth > MAX_DEPTH {
            return Err("policy expression nesting exceeds 24".into());
        }
        if node.kind() == "comment" {
            return Ok(());
        }
        if node.child_count() == 0
            || matches!(node.kind(), "string" | "number" | "boolean" | "identifier")
        {
            if out.len() >= MAX_TOKENS {
                return Err("policy expression exceeds 256 tokens".into());
            }
            out.push(&source[node.byte_range()]);
        } else {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                visit(child, source, depth + 1, out)?;
            }
        }
        Ok(())
    }
    let mut tokens = Vec::new();
    visit(node, source, 0, &mut tokens)?;
    Ok(tokens)
}

struct Parser<'a> {
    tokens: &'a [&'a str],
    pos: usize,
}
impl<'a> Parser<'a> {
    fn parse(tokens: &'a [&'a str]) -> Result<Expr, String> {
        let mut parser = Self { tokens, pos: 0 };
        let expr = parser.expression(0, 0)?;
        if parser.pos != tokens.len() {
            return Err("unsupported policy expression or operator".into());
        }
        Ok(expr)
    }
    fn take(&mut self) -> Result<&'a str, String> {
        let token = *self
            .tokens
            .get(self.pos)
            .ok_or("incomplete policy expression")?;
        self.pos += 1;
        Ok(token)
    }
    fn expect(&mut self, expected: &str) -> Result<(), String> {
        if self.take()? == expected {
            Ok(())
        } else {
            Err(format!("expected {expected} in policy expression"))
        }
    }
    fn expression(&mut self, min: u8, depth: usize) -> Result<Expr, String> {
        if depth > MAX_DEPTH {
            return Err("policy expression nesting exceeds 24".into());
        }
        let token = self.take()?;
        let mut left = match token {
            "!" | "not" => Expr::Not(Box::new(self.expression(6, depth + 1)?)),
            "(" => {
                let expr = self.expression(0, depth + 1)?;
                self.expect(")")?;
                expr
            }
            "[" => {
                let mut values = Vec::new();
                while self.tokens.get(self.pos) != Some(&"]") {
                    let Expr::Literal(value) = self.expression(0, depth + 1)? else {
                        return Err("policy arrays must contain literals".into());
                    };
                    if value.is_array() {
                        return Err("nested policy arrays are unsupported".into());
                    }
                    values.push(value);
                    if self.tokens.get(self.pos) != Some(&",") {
                        break;
                    }
                    self.pos += 1;
                }
                self.expect("]")?;
                Expr::Literal(Value::Array(values))
            }
            "true" | "false" => Expr::Literal(Value::Bool(token == "true")),
            "-" => {
                let number = self.take()?.replace('_', "");
                let value = format!("-{number}")
                    .parse::<i64>()
                    .map_err(|_| "policy numbers must be signed 64-bit integers")?;
                Expr::Literal(Value::from(value))
            }
            value if value.starts_with('"') => Expr::Literal(Value::String(
                serde_json::from_str(value)
                    .map_err(|_| "policy strings must be valid JSON strings")?,
            )),
            value if value.as_bytes().first().is_some_and(u8::is_ascii_digit) => {
                Expr::Literal(Value::from(
                    value
                        .replace('_', "")
                        .parse::<i64>()
                        .map_err(|_| "policy numbers must be signed 64-bit integers")?,
                ))
            }
            "principal" | "invocation" | "context" => {
                let mut path = vec![token.to_owned()];
                while self.tokens.get(self.pos) == Some(&".") {
                    self.pos += 1;
                    let key = self.take()?;
                    if !key.bytes().enumerate().all(|(index, byte)| {
                        byte.is_ascii_alphabetic()
                            || byte == b'_'
                            || (index > 0 && byte.is_ascii_digit())
                    }) {
                        return Err("invalid policy field".into());
                    }
                    path.push(key.to_owned());
                }
                if token == "principal" && path.len() != 1 {
                    return Err("principal is the actual agent ID string".into());
                }
                if token == "context"
                    && path.get(1).is_some_and(|key| {
                        matches!(key.as_str(), "has_human_approval" | "approved_fingerprint")
                    })
                {
                    return Err("inline rules cannot establish or inspect approval receipts".into());
                }
                Expr::Field(path)
            }
            _ => return Err(format!("unsupported inline policy token {token:?}")),
        };
        while let Some(operator) = self.tokens.get(self.pos).copied() {
            let precedence = match operator {
                "||" => 1,
                "&&" => 2,
                "==" | "!=" | "in" => 3,
                "<" | ">" | "<=" | ">=" => 4,
                _ => break,
            };
            if precedence < min {
                break;
            }
            self.pos += 1;
            let right = self.expression(precedence + 1, depth + 1)?;
            left = Expr::Binary(operator.into(), Box::new(left), Box::new(right));
        }
        Ok(left)
    }
}

impl Expr {
    fn boolean(&self, data: &Value) -> Result<bool, String> {
        self.value(data)?
            .as_bool()
            .ok_or_else(|| "policy condition is not boolean".into())
    }
    fn value(&self, data: &Value) -> Result<Value, String> {
        match self {
            Self::Literal(value) => Ok(value.clone()),
            Self::Field(path) => {
                let mut value = data;
                for key in path {
                    value = value
                        .as_object()
                        .and_then(|map| map.get(key))
                        .ok_or_else(|| format!("missing policy field {}", path.join(".")))?;
                }
                Ok(value.clone())
            }
            Self::Not(expr) => Ok(Value::Bool(!expr.boolean(data)?)),
            Self::Binary(op, left, right) => {
                if op == "&&" {
                    return Ok(Value::Bool(left.boolean(data)? && right.boolean(data)?));
                }
                if op == "||" {
                    return Ok(Value::Bool(left.boolean(data)? || right.boolean(data)?));
                }
                let left = left.value(data)?;
                let right = right.value(data)?;
                fn scalar_kind(value: &Value) -> Option<u8> {
                    match value {
                        Value::Bool(_) => Some(1),
                        Value::String(_) => Some(2),
                        Value::Number(n) if n.as_i64().is_some() => Some(3),
                        _ => None,
                    }
                }
                let result = if op == "in" {
                    let array = right
                        .as_array()
                        .ok_or("right side of in must be an array")?;
                    if scalar_kind(&left).is_none()
                        || array
                            .iter()
                            .any(|value| scalar_kind(value) != scalar_kind(&left))
                    {
                        return Err("policy membership types do not match".into());
                    }
                    array.contains(&left)
                } else {
                    if scalar_kind(&left).is_none() || scalar_kind(&left) != scalar_kind(&right) {
                        return Err("policy comparison requires matching scalar types".into());
                    }
                    match op.as_str() {
                        "==" => left == right,
                        "!=" => left != right,
                        _ => {
                            let (left, right) = (
                                left.as_i64()
                                    .ok_or("ordered policy comparisons require integers")?,
                                right
                                    .as_i64()
                                    .ok_or("ordered policy comparisons require integers")?,
                            );
                            match op.as_str() {
                                "<" => left < right,
                                ">" => left > right,
                                "<=" => left <= right,
                                ">=" => left >= right,
                                _ => return Err("unsupported policy operator".into()),
                            }
                        }
                    }
                };
                Ok(Value::Bool(result))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn compile(rules: &str) -> ExecutionPolicy {
        ExecutionPolicy::parse(
            &format!("agent worker() {{ policy scope {{ {rules} }} }}"),
            "worker",
        )
        .unwrap()
    }

    #[test]
    fn combines_file_and_selected_scopes_without_sibling_authority() {
        let source = r#"policy global { deny: "erase" }
agent a() { policy local { allow: ["read", "erase"] } }
agent b() { policy local { allow: "write" } }"#;
        let a = ExecutionPolicy::parse(source, "a").unwrap();
        assert_eq!(a.names(), ["global", "local"]);
        assert!(a.evaluate("read", &json!({})).is_ok());
        assert!(a.evaluate("erase", &json!({})).is_err());
        assert!(a.evaluate("write", &json!({})).is_err());
        assert!(ExecutionPolicy::parse(source, "b")
            .unwrap()
            .evaluate("write", &json!({}))
            .is_ok());
        assert!(ExecutionPolicy::parse(
            "policy p { deny: true } agent a() { policy p { allow: true } }",
            "a"
        )
        .is_err());
    }

    #[test]
    fn evaluates_pure_conditions_and_fails_closed_on_missing_or_wrong_types() {
        let policy = compile(
            r#"allow: "edit" if principal == "worker" && invocation.arguments.path in ["/workspace/report", "/workspace/notes"] && context.level >= 2
deny: "edit" if context.locked || !(context.region == "lab")"#,
        );
        let data = json!({"principal":"worker", "invocation":{"arguments":{"path":"/workspace/report"}}, "context":{"level":2,"locked":false,"region":"lab"}});
        assert!(policy.evaluate("edit", &data).is_ok());
        for change in [json!("2"), json!(1), Value::Null] {
            let mut changed = data.clone();
            changed["context"]["level"] = change;
            assert!(policy.evaluate("edit", &changed).is_err());
        }
        let mut wrong_principal = data.clone();
        wrong_principal["principal"] = json!("sibling");
        assert!(policy.evaluate("edit", &wrong_principal).is_err());
        let missing = compile("deny: true if context.missing != false");
        assert!(missing.evaluate("edit", &json!({"context":{}})).is_err());
        assert!(compile("allow: true if false || true && !false")
            .evaluate("edit", &json!({}))
            .is_ok());
        assert!(
            compile("allow: true if not (true || false) && context.missing")
                .evaluate("edit", &json!({}))
                .is_err()
        );
        assert!(compile("allow: true if true || context.missing")
            .evaluate("edit", &json!({}))
            .is_ok());
        assert!(compile("allow: true if -3 < 1_000 && 4 >= 4")
            .evaluate("edit", &json!({}))
            .is_ok());
    }

    #[test]
    fn deny_overrides_allow_and_each_allow_block_must_match() {
        assert!(compile("allow: true deny: true")
            .evaluate("edit", &json!({}))
            .is_err());
        assert!(compile("deny: \"erase\"")
            .evaluate("edit", &json!({}))
            .is_ok());
        assert!(compile("allow: false")
            .evaluate("edit", &json!({}))
            .is_err());
        let policy = ExecutionPolicy::parse(
            "policy global { allow: \"read\" } agent a() { policy local { allow: \"write\" } }",
            "a",
        )
        .unwrap();
        assert!(policy.evaluate("read", &json!({})).is_err());
        assert!(policy.evaluate("write", &json!({})).is_err());
    }

    #[test]
    fn rejects_unsupported_requirements_even_in_inactive_branches() {
        for rule in [
            "require: true",
            "audit: true",
            "allow: []",
            "allow: [1]",
            "allow: invoke_tool(tool)",
            "allow: \"edit\" if false && context.role.contains(\"x\")",
            "allow: true if context.level + 1 > 2",
            "allow: true if invocation.arguments[\"path\"] == \"x\"",
            "allow: true if user.role == \"admin\"",
            "allow: true if context.has_human_approval",
            "allow: true if principal.id == \"x\"",
            "allow: true if 1.5 > 1",
            "allow: true if 9223372036854775808 > 1",
            "allow: true if [true, [false]] == true",
        ] {
            let source = format!("agent a() {{ policy p {{ {rule} }} }}");
            assert!(ExecutionPolicy::parse(&source, "a").is_err(), "{rule}");
        }
    }

    #[test]
    fn bounds_parser_work_and_retains_comments_and_source() {
        let source = "policy /* invalid */ p { allow: true } agent a() {}";
        assert!(ExecutionPolicy::parse(source, "a").is_err());
        let source = "agent // selected\na() { policy p { allow: [\"read\", # selector\n \"write\"] if true # condition\n } }";
        let definition = crate::ConversationalAgent::parse(source, "a").unwrap();
        assert_eq!(crate::conversational_agent_names(source).unwrap(), ["a"]);
        assert_eq!(definition.policy().source(), source);
        assert!(definition.policy().evaluate("read", &json!({})).is_ok());
        let encoded = serde_json::to_value(&definition).unwrap();
        let decoded: crate::ConversationalAgent = serde_json::from_value(encoded).unwrap();
        assert!(decoded.policy().evaluate("erase", &json!({})).is_err());
        let excessive = format!(
            "agent a() {{ policy p {{ {} }} }}",
            "allow: true ".repeat(129)
        );
        assert!(ExecutionPolicy::parse(&excessive, "a").is_err());
        let deep = format!(
            "agent a() {{ policy p {{ allow: true if {}true{} }} }}",
            "(".repeat(30),
            ")".repeat(30)
        );
        assert!(ExecutionPolicy::parse(&deep, "a").is_err());
    }
}
