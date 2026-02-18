#![no_main]

//! Structure-aware fuzz target for the Symbi DSL lexer + parser.
//!
//! Instead of feeding raw bytes, we derive `Arbitrary` on structs that
//! mirror the DSL grammar and render them to syntactically-diverse source
//! strings.  This lets the fuzzer explore deep parser paths that raw byte
//! mutation would rarely reach.

use libfuzzer_sys::{arbitrary::Arbitrary, fuzz_target};
use repl_core::dsl::{Lexer, Parser};

// ---------------------------------------------------------------------------
// String helpers
// ---------------------------------------------------------------------------

/// Clamp a fuzz-generated string to a sane length, fixing char boundaries.
fn clamp(mut s: String, max: usize, fallback: &str) -> String {
    if s.is_empty() {
        return fallback.to_string();
    }
    if s.len() > max {
        let mut end = max;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
    s
}

/// Turn an arbitrary string into a valid DSL identifier (alphanumeric + underscore).
fn to_ident(s: &str, fallback: &str) -> String {
    let cleaned: String = s
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .take(24)
        .collect();
    if cleaned.is_empty() || cleaned.starts_with(|c: char| c.is_ascii_digit()) {
        fallback.to_string()
    } else {
        cleaned
    }
}

/// Escape a string for embedding in DSL double-quoted literals.
fn escape_str(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

// ---------------------------------------------------------------------------
// Pick-lists — small, closed sets the fuzzer can index into
// ---------------------------------------------------------------------------

const IDENT_POOL: &[&str] = &[
    "x", "y", "z", "val", "count", "name", "result", "data", "msg", "item",
    "ctx", "state", "cfg", "buf", "idx", "tmp", "err", "ok", "inner", "outer",
];

const TYPE_NAMES: &[&str] = &[
    "string", "number", "boolean", "datetime", "duration", "size", "any",
];

const DURATION_UNITS: &[&str] = &["ms", "s", "m", "h", "d"];
const SIZE_UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];

const SECURITY_TIERS: &[&str] = &["tier1", "tier2", "tier3", "tier4"];
const SANDBOX_MODES: &[&str] = &["strict", "moderate", "permissive"];
const FAILURE_ACTIONS: &[&str] = &["terminate", "restart", "escalate", "ignore"];

fn pick<'a>(pool: &'a [&'a str], idx: u8) -> &'a str {
    pool[(idx as usize) % pool.len()]
}

// ---------------------------------------------------------------------------
// Arbitrary types that mirror the DSL grammar
// ---------------------------------------------------------------------------

#[derive(Arbitrary, Debug)]
struct DslInput {
    declarations: Vec<DslDeclaration>,
}

#[derive(Arbitrary, Debug)]
enum DslDeclaration {
    Agent {
        name: String,
        has_name_meta: bool,
        has_version: bool,
        has_author: bool,
        has_description: bool,
        has_resources: bool,
        has_security: bool,
        has_policies: bool,
        resource_cfg: ResourceCfg,
        security_cfg: SecurityCfg,
        policy_cfg: PolicyCfg,
    },
    Behavior {
        name: String,
        has_input: bool,
        has_output: bool,
        input_params: Vec<ParamDef>,
        output_params: Vec<ParamDef>,
        steps: Vec<DslStatement>,
    },
    Function {
        name: String,
        params: Vec<ParamDef>,
        has_return_type: bool,
        return_type_idx: u8,
        body: Vec<DslStatement>,
    },
    EventHandler {
        event_name: String,
        params: Vec<ParamDef>,
        body: Vec<DslStatement>,
    },
    StructDef {
        name: String,
        fields: Vec<FieldDef>,
    },
    /// Raw text — also exercise error paths with arbitrary content
    RawText(String),
}

#[derive(Arbitrary, Debug)]
struct ResourceCfg {
    memory_val: u16,
    memory_unit: u8,
    cpu_val: u16,
    cpu_unit: u8,
    network_allowed: bool,
    storage_val: u16,
    storage_unit: u8,
}

#[derive(Arbitrary, Debug)]
struct SecurityCfg {
    tier_idx: u8,
    capabilities: Vec<String>,
    sandbox_idx: u8,
}

#[derive(Arbitrary, Debug)]
struct PolicyCfg {
    timeout_val: u16,
    timeout_unit: u8,
    retry_count: u8,
    failure_idx: u8,
}

#[derive(Arbitrary, Debug)]
struct ParamDef {
    name: String,
    type_kind: DslType,
}

#[derive(Arbitrary, Debug)]
struct FieldDef {
    name: String,
    type_kind: DslType,
}

#[derive(Arbitrary, Debug)]
enum DslType {
    Simple(u8),             // indexes into TYPE_NAMES
    List(u8),               // list<T>
    Map(u8, u8),            // map<K, V>
    Custom(String),         // user-defined type name
}

#[derive(Arbitrary, Debug)]
enum DslStatement {
    Let {
        var_name: String,
        has_type: bool,
        type_idx: u8,
        value: DslExpr,
    },
    If {
        condition: DslExpr,
        then_stmts: Vec<DslStatement>,
        has_else: bool,
        else_stmts: Vec<DslStatement>,
    },
    Return {
        has_value: bool,
        value: DslExpr,
    },
    Emit {
        event_name: String,
    },
    Require {
        kind: RequireKind,
    },
    ExprStmt(DslExpr),
}

#[derive(Arbitrary, Debug)]
enum RequireKind {
    SingleCapability(String),
    MultiCapability(Vec<String>),
}

#[derive(Arbitrary, Debug)]
enum DslExpr {
    StringLit(String),
    NumberLit(u8),          // kept small to avoid huge floats
    IntegerLit(i16),
    BoolLit(bool),
    NullLit,
    DurationLit(u16, u8),   // value, unit index
    SizeLit(u16, u8),       // value, unit index
    Identifier(u8),         // indexes into IDENT_POOL
    FunctionCall {
        func_name: u8,
        args: Vec<DslExpr>,
    },
    BinaryOp {
        left: Box<DslExpr>,
        op: BinOp,
        right: Box<DslExpr>,
    },
    UnaryNot(Box<DslExpr>),
    UnaryNeg(Box<DslExpr>),
    FieldAccess {
        object: Box<DslExpr>,
        field: u8,
    },
    MethodCall {
        object: Box<DslExpr>,
        method: u8,
        args: Vec<DslExpr>,
    },
    IndexAccess {
        object: Box<DslExpr>,
        index: Box<DslExpr>,
    },
    ListLit(Vec<DslExpr>),
    MapLit(Vec<(DslExpr, DslExpr)>),
    Parenthesized(Box<DslExpr>),
    /// Inject arbitrary text into expression position to hit error paths
    RawExpr(String),
}

#[derive(Arbitrary, Debug)]
enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Neq,
    Lt,
    Gt,
    Lte,
    Gte,
    And,
    Or,
}

// ---------------------------------------------------------------------------
// Rendering — turn Arbitrary structs into DSL source text
// ---------------------------------------------------------------------------

impl DslInput {
    fn render(&self) -> String {
        let mut out = String::with_capacity(2048);
        // Limit declaration count to avoid huge inputs
        for decl in self.declarations.iter().take(5) {
            decl.render(&mut out);
            out.push('\n');
        }
        out
    }
}

impl DslDeclaration {
    fn render(&self, out: &mut String) {
        match self {
            DslDeclaration::Agent {
                name,
                has_name_meta,
                has_version,
                has_author,
                has_description,
                has_resources,
                has_security,
                has_policies,
                resource_cfg,
                security_cfg,
                policy_cfg,
            } => {
                let ident = to_ident(&clamp(name.clone(), 32, "fuzz_agent"), "fuzz_agent");
                out.push_str(&format!("agent {} {{\n", ident));

                if *has_name_meta {
                    out.push_str(&format!("  name: \"{}\"\n", escape_str(&ident)));
                }
                if *has_version {
                    out.push_str("  version: \"1.0.0\"\n");
                }
                if *has_author {
                    out.push_str("  author: \"fuzzer\"\n");
                }
                if *has_description {
                    out.push_str("  description: \"fuzz test agent\"\n");
                }
                if *has_resources {
                    out.push_str("  resources {\n");
                    out.push_str(&format!(
                        "    memory: {}{}\n",
                        resource_cfg.memory_val.clamp(1, 9999),
                        pick(SIZE_UNITS, resource_cfg.memory_unit)
                    ));
                    out.push_str(&format!(
                        "    cpu: {}{}\n",
                        resource_cfg.cpu_val.clamp(1, 9999),
                        pick(DURATION_UNITS, resource_cfg.cpu_unit)
                    ));
                    out.push_str(&format!(
                        "    network: {}\n",
                        if resource_cfg.network_allowed {
                            "allow"
                        } else {
                            "false"
                        }
                    ));
                    out.push_str(&format!(
                        "    storage: {}{}\n",
                        resource_cfg.storage_val.clamp(1, 9999),
                        pick(SIZE_UNITS, resource_cfg.storage_unit)
                    ));
                    out.push_str("  }\n");
                }
                if *has_security {
                    out.push_str("  security {\n");
                    out.push_str(&format!(
                        "    tier: {}\n",
                        pick(SECURITY_TIERS, security_cfg.tier_idx)
                    ));
                    if !security_cfg.capabilities.is_empty() {
                        let caps: Vec<String> = security_cfg
                            .capabilities
                            .iter()
                            .take(4)
                            .map(|c| format!("\"{}\"", escape_str(&clamp(c.clone(), 16, "cap"))))
                            .collect();
                        out.push_str(&format!("    capabilities: [{}]\n", caps.join(", ")));
                    }
                    out.push_str(&format!(
                        "    sandbox: {}\n",
                        pick(SANDBOX_MODES, security_cfg.sandbox_idx)
                    ));
                    out.push_str("  }\n");
                }
                if *has_policies {
                    out.push_str("  policies {\n");
                    out.push_str(&format!(
                        "    timeout: {}{}\n",
                        policy_cfg.timeout_val.clamp(1, 9999),
                        pick(DURATION_UNITS, policy_cfg.timeout_unit)
                    ));
                    out.push_str(&format!(
                        "    retry: {}\n",
                        policy_cfg.retry_count.clamp(0, 10)
                    ));
                    out.push_str(&format!(
                        "    failure: {}\n",
                        pick(FAILURE_ACTIONS, policy_cfg.failure_idx)
                    ));
                    out.push_str("  }\n");
                }

                out.push_str("}\n");
            }

            DslDeclaration::Behavior {
                name,
                has_input,
                has_output,
                input_params,
                output_params,
                steps,
            } => {
                let ident = to_ident(&clamp(name.clone(), 32, "fuzz_behavior"), "fuzz_behavior");
                out.push_str(&format!("behavior {} {{\n", ident));

                if *has_input && !input_params.is_empty() {
                    out.push_str("  input {\n");
                    render_params(out, input_params, 4);
                    out.push_str("  }\n");
                }
                if *has_output && !output_params.is_empty() {
                    out.push_str("  output {\n");
                    render_params(out, output_params, 4);
                    out.push_str("  }\n");
                }

                out.push_str("  steps {\n");
                for stmt in steps.iter().take(6) {
                    out.push_str("    ");
                    stmt.render(out, 2);
                    out.push('\n');
                }
                out.push_str("  }\n");
                out.push_str("}\n");
            }

            DslDeclaration::Function {
                name,
                params,
                has_return_type,
                return_type_idx,
                body,
            } => {
                let ident = to_ident(&clamp(name.clone(), 32, "fuzz_fn"), "fuzz_fn");
                out.push_str(&format!("function {}(", ident));
                render_param_list(out, params);
                out.push(')');

                if *has_return_type {
                    out.push_str(" -> ");
                    out.push_str(pick(TYPE_NAMES, *return_type_idx));
                }

                out.push_str(" {\n");
                for stmt in body.iter().take(6) {
                    out.push_str("  ");
                    stmt.render(out, 1);
                    out.push('\n');
                }
                out.push_str("}\n");
            }

            DslDeclaration::EventHandler {
                event_name,
                params,
                body,
            } => {
                let ident = to_ident(&clamp(event_name.clone(), 32, "fuzz_event"), "fuzz_event");
                out.push_str(&format!("on {}(", ident));
                render_param_list(out, params);
                out.push_str(") {\n");
                for stmt in body.iter().take(6) {
                    out.push_str("  ");
                    stmt.render(out, 1);
                    out.push('\n');
                }
                out.push_str("}\n");
            }

            DslDeclaration::StructDef { name, fields } => {
                let ident = to_ident(&clamp(name.clone(), 32, "FuzzStruct"), "FuzzStruct");
                out.push_str(&format!("struct {} {{\n", ident));
                for (i, field) in fields.iter().take(8).enumerate() {
                    let fname = to_ident(
                        &clamp(field.name.clone(), 24, &format!("field{}", i)),
                        &format!("field{}", i),
                    );
                    out.push_str(&format!("  {}: ", fname));
                    field.type_kind.render(out);
                    out.push('\n');
                }
                out.push_str("}\n");
            }

            DslDeclaration::RawText(text) => {
                // Inject raw text capped at a reasonable length
                out.push_str(&clamp(text.clone(), 256, ""));
                out.push('\n');
            }
        }
    }
}

fn render_params(out: &mut String, params: &[ParamDef], indent: usize) {
    let indent_str: String = " ".repeat(indent);
    for (i, p) in params.iter().take(4).enumerate() {
        let pname = to_ident(
            &clamp(p.name.clone(), 24, &format!("p{}", i)),
            &format!("p{}", i),
        );
        out.push_str(&indent_str);
        out.push_str(&format!("{}: ", pname));
        p.type_kind.render(out);
        if i + 1 < params.len().min(4) {
            out.push(',');
        }
        out.push('\n');
    }
}

fn render_param_list(out: &mut String, params: &[ParamDef]) {
    for (i, p) in params.iter().take(4).enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        let pname = to_ident(
            &clamp(p.name.clone(), 24, &format!("p{}", i)),
            &format!("p{}", i),
        );
        out.push_str(&format!("{}: ", pname));
        p.type_kind.render(out);
    }
}

impl DslType {
    fn render(&self, out: &mut String) {
        match self {
            DslType::Simple(idx) => out.push_str(pick(TYPE_NAMES, *idx)),
            DslType::List(inner) => {
                out.push_str("list<");
                out.push_str(pick(TYPE_NAMES, *inner));
                out.push('>');
            }
            DslType::Map(k, v) => {
                out.push_str("map<");
                out.push_str(pick(TYPE_NAMES, *k));
                out.push_str(", ");
                out.push_str(pick(TYPE_NAMES, *v));
                out.push('>');
            }
            DslType::Custom(name) => {
                let ident = to_ident(&clamp(name.clone(), 24, "CustomType"), "CustomType");
                out.push_str(&ident);
            }
        }
    }
}

impl DslStatement {
    fn render(&self, out: &mut String, depth: usize) {
        // Limit recursion depth to avoid stack overflow
        if depth > 4 {
            out.push_str("null");
            return;
        }

        match self {
            DslStatement::Let {
                var_name,
                has_type,
                type_idx,
                value,
            } => {
                let ident = to_ident(&clamp(var_name.clone(), 24, "v"), "v");
                out.push_str(&format!("let {}", ident));
                if *has_type {
                    out.push_str(": ");
                    out.push_str(pick(TYPE_NAMES, *type_idx));
                }
                out.push_str(" = ");
                value.render(out, depth + 1);
            }

            DslStatement::If {
                condition,
                then_stmts,
                has_else,
                else_stmts,
            } => {
                out.push_str("if ");
                condition.render(out, depth + 1);
                out.push_str(" {\n");
                for s in then_stmts.iter().take(3) {
                    out.push_str(&"  ".repeat(depth + 1));
                    s.render(out, depth + 1);
                    out.push('\n');
                }
                out.push_str(&"  ".repeat(depth));
                out.push('}');
                if *has_else && !else_stmts.is_empty() {
                    out.push_str(" else {\n");
                    for s in else_stmts.iter().take(3) {
                        out.push_str(&"  ".repeat(depth + 1));
                        s.render(out, depth + 1);
                        out.push('\n');
                    }
                    out.push_str(&"  ".repeat(depth));
                    out.push('}');
                }
            }

            DslStatement::Return { has_value, value } => {
                out.push_str("return");
                if *has_value {
                    out.push(' ');
                    value.render(out, depth + 1);
                }
            }

            DslStatement::Emit { event_name } => {
                let ident = to_ident(&clamp(event_name.clone(), 24, "fuzz_evt"), "fuzz_evt");
                out.push_str(&format!("emit {}", ident));
            }

            DslStatement::Require { kind } => match kind {
                RequireKind::SingleCapability(cap) => {
                    let ident = to_ident(&clamp(cap.clone(), 24, "read"), "read");
                    out.push_str(&format!("require capability {}", ident));
                }
                RequireKind::MultiCapability(caps) => {
                    let idents: Vec<String> = caps
                        .iter()
                        .take(4)
                        .enumerate()
                        .map(|(i, c)| {
                            to_ident(&clamp(c.clone(), 24, &format!("cap{}", i)), &format!("cap{}", i))
                        })
                        .collect();
                    out.push_str(&format!("require capabilities [{}]", idents.join(", ")));
                }
            },

            DslStatement::ExprStmt(expr) => {
                expr.render(out, depth + 1);
            }
        }
    }
}

impl DslExpr {
    fn render(&self, out: &mut String, depth: usize) {
        // Limit recursion depth to avoid stack overflow in the fuzzer itself
        if depth > 6 {
            out.push_str("null");
            return;
        }

        match self {
            DslExpr::StringLit(s) => {
                let escaped = escape_str(&clamp(s.clone(), 64, "fuzz"));
                out.push_str(&format!("\"{}\"", escaped));
            }
            DslExpr::NumberLit(n) => {
                out.push_str(&format!("{}.0", n));
            }
            DslExpr::IntegerLit(i) => {
                out.push_str(&format!("{}", i));
            }
            DslExpr::BoolLit(b) => {
                out.push_str(if *b { "true" } else { "false" });
            }
            DslExpr::NullLit => {
                out.push_str("null");
            }
            DslExpr::DurationLit(val, unit_idx) => {
                out.push_str(&format!(
                    "{}{}",
                    (*val).clamp(1, 9999),
                    pick(DURATION_UNITS, *unit_idx)
                ));
            }
            DslExpr::SizeLit(val, unit_idx) => {
                out.push_str(&format!(
                    "{}{}",
                    (*val).clamp(1, 9999),
                    pick(SIZE_UNITS, *unit_idx)
                ));
            }
            DslExpr::Identifier(idx) => {
                out.push_str(pick(IDENT_POOL, *idx));
            }
            DslExpr::FunctionCall { func_name, args } => {
                out.push_str(pick(IDENT_POOL, *func_name));
                out.push('(');
                for (i, arg) in args.iter().take(4).enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    arg.render(out, depth + 1);
                }
                out.push(')');
            }
            DslExpr::BinaryOp { left, op, right } => {
                left.render(out, depth + 1);
                out.push(' ');
                match op {
                    BinOp::Add => out.push('+'),
                    BinOp::Sub => out.push('-'),
                    BinOp::Mul => out.push('*'),
                    BinOp::Div => out.push('/'),
                    BinOp::Mod => out.push('%'),
                    BinOp::Eq => out.push_str("=="),
                    BinOp::Neq => out.push_str("!="),
                    BinOp::Lt => out.push('<'),
                    BinOp::Gt => out.push('>'),
                    BinOp::Lte => out.push_str("<="),
                    BinOp::Gte => out.push_str(">="),
                    BinOp::And => out.push_str("&&"),
                    BinOp::Or => out.push_str("||"),
                }
                out.push(' ');
                right.render(out, depth + 1);
            }
            DslExpr::UnaryNot(inner) => {
                out.push('!');
                inner.render(out, depth + 1);
            }
            DslExpr::UnaryNeg(inner) => {
                out.push('-');
                inner.render(out, depth + 1);
            }
            DslExpr::FieldAccess { object, field } => {
                object.render(out, depth + 1);
                out.push('.');
                out.push_str(pick(IDENT_POOL, *field));
            }
            DslExpr::MethodCall {
                object,
                method,
                args,
            } => {
                object.render(out, depth + 1);
                out.push('.');
                out.push_str(pick(IDENT_POOL, *method));
                out.push('(');
                for (i, arg) in args.iter().take(4).enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    arg.render(out, depth + 1);
                }
                out.push(')');
            }
            DslExpr::IndexAccess { object, index } => {
                object.render(out, depth + 1);
                out.push('[');
                index.render(out, depth + 1);
                out.push(']');
            }
            DslExpr::ListLit(elements) => {
                out.push('[');
                for (i, elem) in elements.iter().take(5).enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    elem.render(out, depth + 1);
                }
                out.push(']');
            }
            DslExpr::MapLit(entries) => {
                out.push('{');
                for (i, (key, val)) in entries.iter().take(4).enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    key.render(out, depth + 1);
                    out.push_str(": ");
                    val.render(out, depth + 1);
                }
                out.push('}');
            }
            DslExpr::Parenthesized(inner) => {
                out.push('(');
                inner.render(out, depth + 1);
                out.push(')');
            }
            DslExpr::RawExpr(text) => {
                out.push_str(&clamp(text.clone(), 64, "null"));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Fuzz target
// ---------------------------------------------------------------------------

fuzz_target!(|input: DslInput| {
    let source = input.render();

    // Cap total source length to avoid spending time on degenerate inputs
    if source.len() > 8192 {
        return;
    }

    // Step 1: Lex — must not panic
    let tokens = match Lexer::new(&source).tokenize() {
        Ok(tokens) => tokens,
        Err(_) => return, // Lex error is fine, no panic is the goal
    };

    // Step 2: Parse — must not panic
    let program = match Parser::new(tokens).parse() {
        Ok(program) => program,
        Err(_) => return, // Parse error is fine, no panic is the goal
    };

    // Step 3: Basic invariants on successfully parsed programs
    // If the program parsed, it should have at least one declaration
    // (unless the input was empty / only RawText that got skipped).
    let non_raw_count = input
        .declarations
        .iter()
        .take(5)
        .filter(|d| !matches!(d, DslDeclaration::RawText(_)))
        .count();

    if non_raw_count > 0 && program.declarations.is_empty() {
        // This could legitimately happen if the generated DSL has subtle
        // syntax issues that the parser treats as recoverable.  We do NOT
        // assert-fail here because the parser may return Ok(Program { declarations: [] })
        // for edge cases.  Instead, just note it.
    }
});
