#![no_main]

//! Fuzz target for the DSL evaluator.
//!
//! Exercises expression evaluation, operator dispatch, recursion depth
//! protection, type coercion, and built-in function calls. The parser
//! is already fuzzed separately; this target feeds parsed ASTs into
//! the evaluator to find panics in semantic execution.

use futures::executor::block_on;
use libfuzzer_sys::{arbitrary::Arbitrary, fuzz_target};
use repl_core::dsl::{DslEvaluator, Lexer, Parser};
use std::sync::Arc;

/// Maximum source length to keep evaluation fast.
const MAX_SOURCE_LEN: usize = 4096;

#[derive(Arbitrary, Debug)]
struct Input {
    mode: FuzzMode,
}

#[derive(Arbitrary, Debug)]
enum FuzzMode {
    /// Raw source code → lex → parse → evaluate.
    RawSource(String),
    /// Structured expression snippets designed to exercise specific paths.
    Structured(StructuredExpr),
}

#[derive(Arbitrary, Debug)]
enum StructuredExpr {
    /// Binary operator with two operands.
    BinaryOp {
        left: OperandVariant,
        op: OpVariant,
        right: OperandVariant,
    },
    /// Deeply nested expression to test recursion depth limit.
    DeepNest { depth: u8 },
    /// Let binding + variable reference.
    LetAndRef { name: String, value: OperandVariant },
    /// Built-in function call.
    BuiltinCall { func: BuiltinVariant, arg: String },
    /// If/else expression.
    Conditional {
        condition: OperandVariant,
        then_val: OperandVariant,
        else_val: OperandVariant,
    },
    /// List operations.
    ListOps { elements: Vec<OperandVariant> },
    /// Map operations.
    MapOps { keys: Vec<String>, values: Vec<OperandVariant> },
    /// Division / modulo by zero.
    DivByZero { numerator: OperandVariant, use_modulo: bool },
}

#[derive(Arbitrary, Debug)]
enum OperandVariant {
    Integer(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Null,
}

#[derive(Arbitrary, Debug)]
enum OpVariant {
    Add, Sub, Mul, Div, Mod,
    Eq, Neq, Lt, Gt, Lte, Gte,
    And, Or,
    BitwiseAnd, BitwiseOr, BitwiseXor,
    ShiftLeft, ShiftRight,
}

#[derive(Arbitrary, Debug)]
enum BuiltinVariant {
    Len, Upper, Lower, Format, Print,
}

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

fn operand_to_source(v: &OperandVariant) -> String {
    match v {
        OperandVariant::Integer(i) => i.to_string(),
        OperandVariant::Float(f) => {
            if f.is_nan() { "0.0".to_string() }
            else if f.is_infinite() { "999999999.0".to_string() }
            else { format!("{:.6}", f) }
        }
        OperandVariant::Str(s) => {
            let escaped = clamp(s.clone(), 64, "hello")
                .replace('\\', "\\\\")
                .replace('"', "\\\"");
            format!("\"{}\"", escaped)
        }
        OperandVariant::Bool(b) => b.to_string(),
        OperandVariant::Null => "null".to_string(),
    }
}

fn op_to_source(op: &OpVariant) -> &str {
    match op {
        OpVariant::Add => "+",
        OpVariant::Sub => "-",
        OpVariant::Mul => "*",
        OpVariant::Div => "/",
        OpVariant::Mod => "%",
        OpVariant::Eq => "==",
        OpVariant::Neq => "!=",
        OpVariant::Lt => "<",
        OpVariant::Gt => ">",
        OpVariant::Lte => "<=",
        OpVariant::Gte => ">=",
        OpVariant::And => "and",
        OpVariant::Or => "or",
        OpVariant::BitwiseAnd => "&",
        OpVariant::BitwiseOr => "|",
        OpVariant::BitwiseXor => "^",
        OpVariant::ShiftLeft => "<<",
        OpVariant::ShiftRight => ">>",
    }
}

fn builtin_to_source(func: &BuiltinVariant, arg: &str) -> String {
    let arg = clamp(arg.to_string(), 64, "test")
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    match func {
        BuiltinVariant::Len => format!("len(\"{}\")", arg),
        BuiltinVariant::Upper => format!("upper(\"{}\")", arg),
        BuiltinVariant::Lower => format!("lower(\"{}\")", arg),
        BuiltinVariant::Format => format!("format(\"{}\")", arg),
        BuiltinVariant::Print => format!("print(\"{}\")", arg),
    }
}

fn structured_to_source(expr: &StructuredExpr) -> String {
    match expr {
        StructuredExpr::BinaryOp { left, op, right } => {
            format!(
                "behavior test_eval\n  let result = {} {} {}\nend",
                operand_to_source(left),
                op_to_source(op),
                operand_to_source(right),
            )
        }
        StructuredExpr::DeepNest { depth } => {
            let d = (*depth).min(50) as usize;
            let mut s = "behavior test_eval\n  let result = ".to_string();
            for _ in 0..d {
                s.push_str("(1 + ");
            }
            s.push('1');
            for _ in 0..d {
                s.push(')');
            }
            s.push_str("\nend");
            s
        }
        StructuredExpr::LetAndRef { name, value } => {
            let name = clamp(name.clone(), 32, "x")
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_')
                .collect::<String>();
            let name = if name.is_empty() { "x".to_string() } else { name };
            format!(
                "behavior test_eval\n  let {} = {}\n  let result = {}\nend",
                name,
                operand_to_source(value),
                name,
            )
        }
        StructuredExpr::BuiltinCall { func, arg } => {
            format!(
                "behavior test_eval\n  let result = {}\nend",
                builtin_to_source(func, arg),
            )
        }
        StructuredExpr::Conditional { condition, then_val, else_val } => {
            format!(
                "behavior test_eval\n  let result = if {} then {} else {} end\nend",
                operand_to_source(condition),
                operand_to_source(then_val),
                operand_to_source(else_val),
            )
        }
        StructuredExpr::ListOps { elements } => {
            let elems: Vec<String> = elements.iter()
                .take(8)
                .map(|e| operand_to_source(e))
                .collect();
            format!(
                "behavior test_eval\n  let lst = [{}]\n  let result = len(lst)\nend",
                elems.join(", "),
            )
        }
        StructuredExpr::MapOps { keys, values } => {
            let pairs: Vec<String> = keys.iter()
                .zip(values.iter())
                .take(8)
                .map(|(k, v)| {
                    let k = clamp(k.clone(), 16, "key")
                        .replace('\\', "")
                        .replace('"', "");
                    let k = if k.is_empty() { "key".to_string() } else { k };
                    format!("\"{}\": {}", k, operand_to_source(v))
                })
                .collect();
            format!(
                "behavior test_eval\n  let m = {{{}}}\nend",
                pairs.join(", "),
            )
        }
        StructuredExpr::DivByZero { numerator, use_modulo } => {
            let op = if *use_modulo { "%" } else { "/" };
            format!(
                "behavior test_eval\n  let result = {} {} 0\nend",
                operand_to_source(numerator),
                op,
            )
        }
    }
}

fuzz_target!(|input: Input| {
    let source = match &input.mode {
        FuzzMode::RawSource(s) => {
            let mut s = s.clone();
            if s.len() > MAX_SOURCE_LEN {
                let mut end = MAX_SOURCE_LEN;
                while !s.is_char_boundary(end) {
                    end -= 1;
                }
                s.truncate(end);
            }
            s
        }
        FuzzMode::Structured(expr) => structured_to_source(expr),
    };

    // Lex
    let tokens = match Lexer::new(&source).tokenize() {
        Ok(t) => t,
        Err(_) => return,
    };

    // Parse
    let program = match Parser::new(tokens).parse() {
        Ok(p) => p,
        Err(_) => return,
    };

    // Evaluate — must never panic
    let bridge = Arc::new(repl_core::RuntimeBridge::new());
    let evaluator = DslEvaluator::new(bridge);
    let _ = block_on(evaluator.execute_program(program));
});
