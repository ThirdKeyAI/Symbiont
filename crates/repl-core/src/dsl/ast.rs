//! Abstract Syntax Tree definitions for the Symbiont DSL

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

/// Source location information for error reporting and debugging
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceLocation {
    pub line: usize,
    pub column: usize,
    pub offset: usize,
}

/// Span of source code
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Span {
    pub start: SourceLocation,
    pub end: SourceLocation,
}

/// Root AST node representing a complete DSL program
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Program {
    pub declarations: Vec<Declaration>,
    pub span: Span,
}

/// Top-level declarations in the DSL
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Declaration {
    Agent(AgentDefinition),
    Behavior(BehaviorDefinition),
    Function(FunctionDefinition),
    EventHandler(EventHandler),
    Struct(StructDefinition),
}

/// Agent definition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentDefinition {
    pub name: String,
    pub metadata: AgentMetadata,
    pub resources: Option<ResourceConfig>,
    pub security: Option<SecurityConfig>,
    pub policies: Option<PolicyConfig>,
    pub span: Span,
}

/// Agent metadata
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentMetadata {
    pub name: Option<String>,
    pub version: Option<String>,
    pub author: Option<String>,
    pub description: Option<String>,
}

/// Resource configuration
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceConfig {
    pub memory: Option<SizeValue>,
    pub cpu: Option<DurationValue>,
    pub network: Option<bool>,
    pub storage: Option<SizeValue>,
}

/// Security configuration
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SecurityConfig {
    pub tier: Option<SecurityTier>,
    pub capabilities: Vec<String>,
    pub sandbox: Option<SandboxMode>,
}

/// Security tier levels
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SecurityTier {
    Tier1,
    Tier2,
    Tier3,
    Tier4,
}

/// Sandbox modes
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SandboxMode {
    Strict,
    Moderate,
    Permissive,
}

/// Policy configuration
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyConfig {
    pub execution_timeout: Option<DurationValue>,
    pub retry_count: Option<u32>,
    pub failure_action: Option<FailureAction>,
}

/// Failure actions
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FailureAction {
    Terminate,
    Restart,
    Escalate,
    Ignore,
}

/// Behavior definition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BehaviorDefinition {
    pub name: String,
    pub input: Option<ParameterList>,
    pub output: Option<ParameterList>,
    pub steps: Block,
    pub span: Span,
}

/// Function definition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionDefinition {
    pub name: String,
    pub parameters: ParameterList,
    pub return_type: Option<Type>,
    pub body: Block,
    pub span: Span,
}

/// Event handler definition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventHandler {
    pub event_name: String,
    pub parameters: ParameterList,
    pub body: Block,
    pub span: Span,
}

/// Struct definition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StructDefinition {
    pub name: String,
    pub fields: Vec<StructField>,
    pub span: Span,
}

/// Struct field
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StructField {
    pub name: String,
    pub field_type: Type,
    pub span: Span,
}

/// Parameter list for functions and behaviors
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ParameterList {
    pub parameters: Vec<Parameter>,
}

/// Parameter definition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    pub param_type: Type,
    pub default_value: Option<Expression>,
    pub span: Span,
}

/// Type definitions
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Type {
    String,
    Number,
    Boolean,
    DateTime,
    Duration,
    Size,
    List(Box<Type>),
    Map(Box<Type>, Box<Type>),
    Optional(Box<Type>),
    Custom(String),
    Any,
}

/// Block of statements
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub statements: Vec<Statement>,
    pub span: Span,
}

/// Statement types
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Statement {
    Let(LetStatement),
    If(IfStatement),
    Match(MatchStatement),
    For(ForStatement),
    While(WhileStatement),
    Try(TryStatement),
    Return(ReturnStatement),
    Emit(EmitStatement),
    Require(RequireStatement),
    Check(CheckStatement),
    Expression(Expression),
}

/// Let statement for variable binding
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LetStatement {
    pub name: String,
    pub var_type: Option<Type>,
    pub value: Expression,
    pub span: Span,
}

/// If statement
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IfStatement {
    pub condition: Expression,
    pub then_block: Block,
    pub else_ifs: Vec<ElseIf>,
    pub else_block: Option<Block>,
    pub span: Span,
}

/// Else-if clause
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ElseIf {
    pub condition: Expression,
    pub block: Block,
    pub span: Span,
}

/// Match statement for pattern matching
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchStatement {
    pub expression: Expression,
    pub arms: Vec<MatchArm>,
    pub span: Span,
}

/// Match arm
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub body: Expression,
    pub span: Span,
}

/// Pattern for matching
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Pattern {
    Literal(Literal),
    Wildcard,
    Identifier(String),
}

/// For loop statement
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForStatement {
    pub variable: String,
    pub iterable: Expression,
    pub body: Block,
    pub span: Span,
}

/// While loop statement
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WhileStatement {
    pub condition: Expression,
    pub body: Block,
    pub span: Span,
}

/// Try-catch statement
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TryStatement {
    pub try_block: Block,
    pub catch_variable: Option<String>,
    pub catch_block: Block,
    pub span: Span,
}

/// Return statement
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReturnStatement {
    pub value: Option<Expression>,
    pub span: Span,
}

/// Emit statement for events
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmitStatement {
    pub event_name: String,
    pub data: Option<Expression>,
    pub span: Span,
}

/// Require statement for capabilities
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequireStatement {
    pub requirement: RequirementType,
    pub span: Span,
}

/// Requirement types
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RequirementType {
    Capability(String),
    Capabilities(Vec<String>),
}

/// Check statement for policies
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckStatement {
    pub policy_name: String,
    pub span: Span,
}

/// Expression types
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Expression {
    Literal(Literal),
    Identifier(Identifier),
    FieldAccess(FieldAccess),
    IndexAccess(IndexAccess),
    FunctionCall(FunctionCall),
    MethodCall(MethodCall),
    BinaryOp(BinaryOperation),
    UnaryOp(UnaryOperation),
    Assignment(Assignment),
    List(ListExpression),
    Map(MapExpression),
    Invoke(InvokeExpression),
    Lambda(LambdaExpression),
    Conditional(ConditionalExpression),
}

/// Literal values
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Literal {
    String(String),
    Number(f64),
    Integer(i64),
    Boolean(bool),
    Null,
    Duration(DurationValue),
    Size(SizeValue),
}

/// Duration value with unit
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DurationValue {
    pub value: u64,
    pub unit: DurationUnit,
}

/// Duration units
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DurationUnit {
    Seconds,
    Minutes,
    Hours,
    Days,
    Milliseconds,
}

/// Size value with unit
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SizeValue {
    pub value: u64,
    pub unit: SizeUnit,
}

/// Size units
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SizeUnit {
    Bytes,
    KB,
    MB,
    GB,
    TB,
}

/// Identifier
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Identifier {
    pub name: String,
    pub span: Span,
}

/// Field access (e.g., obj.field)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldAccess {
    pub object: Box<Expression>,
    pub field: String,
    pub span: Span,
}

/// Index access (e.g., arr[0])
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexAccess {
    pub object: Box<Expression>,
    pub index: Box<Expression>,
    pub span: Span,
}

/// Function call
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionCall {
    pub function: String,
    pub arguments: Vec<Expression>,
    pub span: Span,
}

/// Method call (e.g., obj.method(args))
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MethodCall {
    pub object: Box<Expression>,
    pub method: String,
    pub arguments: Vec<Expression>,
    pub span: Span,
}

/// Binary operation
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BinaryOperation {
    pub left: Box<Expression>,
    pub operator: BinaryOperator,
    pub right: Box<Expression>,
    pub span: Span,
}

/// Binary operators
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BinaryOperator {
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    Equal,
    NotEqual,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
    And,
    Or,
    BitwiseAnd,
    BitwiseOr,
    BitwiseXor,
    LeftShift,
    RightShift,
}

/// Unary operation
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnaryOperation {
    pub operator: UnaryOperator,
    pub operand: Box<Expression>,
    pub span: Span,
}

/// Unary operators
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum UnaryOperator {
    Not,
    Negate,
    BitwiseNot,
}

/// Assignment expression
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Assignment {
    pub target: Box<Expression>,
    pub value: Box<Expression>,
    pub span: Span,
}

/// List expression
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListExpression {
    pub elements: Vec<Expression>,
    pub span: Span,
}

/// Map expression
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapExpression {
    pub entries: Vec<MapEntry>,
    pub span: Span,
}

/// Map entry
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapEntry {
    pub key: Expression,
    pub value: Expression,
    pub span: Span,
}

/// Invoke expression for behavior invocation
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InvokeExpression {
    pub behavior: String,
    pub arguments: HashMap<String, Expression>,
    pub span: Span,
}

/// Lambda expression
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LambdaExpression {
    pub parameters: Vec<String>,
    pub body: Box<Expression>,
    pub span: Span,
}

/// Conditional expression (ternary operator)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConditionalExpression {
    pub condition: Box<Expression>,
    pub if_true: Box<Expression>,
    pub if_false: Box<Expression>,
    pub span: Span,
}

impl DurationValue {
    /// Convert to standard Duration
    pub fn to_duration(&self) -> Duration {
        match self.unit {
            DurationUnit::Milliseconds => Duration::from_millis(self.value),
            DurationUnit::Seconds => Duration::from_secs(self.value),
            DurationUnit::Minutes => Duration::from_secs(self.value * 60),
            DurationUnit::Hours => Duration::from_secs(self.value * 3600),
            DurationUnit::Days => Duration::from_secs(self.value * 86400),
        }
    }
}

impl SizeValue {
    /// Convert to bytes
    pub fn to_bytes(&self) -> u64 {
        match self.unit {
            SizeUnit::Bytes => self.value,
            SizeUnit::KB => self.value * 1024,
            SizeUnit::MB => self.value * 1024 * 1024,
            SizeUnit::GB => self.value * 1024 * 1024 * 1024,
            SizeUnit::TB => self.value * 1024 * 1024 * 1024 * 1024,
        }
    }
}

/// Visitor trait for traversing the AST
pub trait AstVisitor {
    type Output;

    fn visit_program(&mut self, program: &Program) -> Self::Output;
    fn visit_declaration(&mut self, declaration: &Declaration) -> Self::Output;
    fn visit_statement(&mut self, statement: &Statement) -> Self::Output;
    fn visit_expression(&mut self, expression: &Expression) -> Self::Output;
}

/// Mutable visitor trait for transforming the AST
pub trait AstVisitorMut {
    fn visit_program_mut(&mut self, program: &mut Program);
    fn visit_declaration_mut(&mut self, declaration: &mut Declaration);
    fn visit_statement_mut(&mut self, statement: &mut Statement);
    fn visit_expression_mut(&mut self, expression: &mut Expression);
}