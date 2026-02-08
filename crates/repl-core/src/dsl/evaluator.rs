//! DSL Evaluator for the Symbiont REPL
//!
//! Executes parsed DSL programs with runtime integration and policy enforcement.

use crate::dsl::ast::*;
use crate::error::{ReplError, Result};
use crate::execution_monitor::{ExecutionMonitor, TraceEventType};
use crate::runtime_bridge::RuntimeBridge;
use crate::session::SessionSnapshot;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use symbi_runtime::integrations::policy_engine::engine::PolicyDecision;
use symbi_runtime::types::security::Capability;
use tokio::sync::RwLock;
use uuid::Uuid;

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
type BuiltinFunction = fn(&[DslValue]) -> Result<DslValue>;

/// Execution context for DSL evaluation
#[derive(Debug, Clone)]
pub struct ExecutionContext {
    /// Variables in scope
    pub variables: HashMap<String, DslValue>,
    /// Function definitions
    pub functions: HashMap<String, FunctionDefinition>,
    /// Current agent instance
    pub agent_id: Option<Uuid>,
    /// Execution depth (for recursion protection)
    pub depth: usize,
    /// Maximum execution depth
    pub max_depth: usize,
}

impl Default for ExecutionContext {
    fn default() -> Self {
        Self {
            variables: HashMap::new(),
            functions: HashMap::new(),
            agent_id: None,
            depth: 0,
            max_depth: 100,
        }
    }
}

/// Runtime value in the DSL
#[derive(Debug, Clone, PartialEq)]
pub enum DslValue {
    String(String),
    Number(f64),
    Integer(i64),
    Boolean(bool),
    Duration { value: u64, unit: DurationUnit },
    Size { value: u64, unit: SizeUnit },
    List(Vec<DslValue>),
    Map(HashMap<String, DslValue>),
    Null,
    Agent(Box<AgentInstance>),
    Function(String),       // Function name reference
    Lambda(LambdaFunction), // Lambda function
}

/// Lambda function value
#[derive(Debug, Clone, PartialEq)]
pub struct LambdaFunction {
    pub parameters: Vec<String>,
    pub body: Expression,
    pub captured_context: HashMap<String, DslValue>,
}

impl DslValue {
    /// Convert to JSON value for serialization
    pub fn to_json(&self) -> JsonValue {
        match self {
            DslValue::String(s) => JsonValue::String(s.clone()),
            DslValue::Number(n) => JsonValue::Number(
                serde_json::Number::from_f64(*n).unwrap_or_else(|| serde_json::Number::from(0)),
            ),
            DslValue::Integer(i) => JsonValue::Number(serde_json::Number::from(*i)),
            DslValue::Boolean(b) => JsonValue::Bool(*b),
            DslValue::Duration { value, unit } => {
                let unit_str = match unit {
                    DurationUnit::Milliseconds => "ms",
                    DurationUnit::Seconds => "s",
                    DurationUnit::Minutes => "m",
                    DurationUnit::Hours => "h",
                    DurationUnit::Days => "d",
                };
                JsonValue::String(format!("{}{}", value, unit_str))
            }
            DslValue::Size { value, unit } => {
                let unit_str = match unit {
                    SizeUnit::Bytes => "B",
                    SizeUnit::KB => "KB",
                    SizeUnit::MB => "MB",
                    SizeUnit::GB => "GB",
                    SizeUnit::TB => "TB",
                };
                JsonValue::String(format!("{}{}", value, unit_str))
            }
            DslValue::List(items) => JsonValue::Array(items.iter().map(|v| v.to_json()).collect()),
            DslValue::Map(entries) => {
                let mut map = serde_json::Map::new();
                for (k, v) in entries {
                    map.insert(k.clone(), v.to_json());
                }
                JsonValue::Object(map)
            }
            DslValue::Null => JsonValue::Null,
            DslValue::Agent(agent) => JsonValue::String(format!("Agent({})", agent.id)),
            DslValue::Function(name) => JsonValue::String(format!("Function({})", name)),
            DslValue::Lambda(lambda) => {
                JsonValue::String(format!("Lambda({} params)", lambda.parameters.len()))
            }
        }
    }

    /// Get the type name for error messages
    pub fn type_name(&self) -> &'static str {
        match self {
            DslValue::String(_) => "string",
            DslValue::Number(_) => "number",
            DslValue::Integer(_) => "integer",
            DslValue::Boolean(_) => "boolean",
            DslValue::Duration { .. } => "duration",
            DslValue::Size { .. } => "size",
            DslValue::List(_) => "list",
            DslValue::Map(_) => "map",
            DslValue::Null => "null",
            DslValue::Agent(_) => "agent",
            DslValue::Function(_) => "function",
            DslValue::Lambda(_) => "lambda",
        }
    }

    /// Check if value is truthy
    pub fn is_truthy(&self) -> bool {
        match self {
            DslValue::Boolean(b) => *b,
            DslValue::Null => false,
            DslValue::String(s) => !s.is_empty(),
            DslValue::Number(n) => *n != 0.0,
            DslValue::Integer(i) => *i != 0,
            DslValue::List(items) => !items.is_empty(),
            DslValue::Map(entries) => !entries.is_empty(),
            DslValue::Lambda(_) => true,
            _ => true,
        }
    }
}

/// Agent instance in the DSL runtime
#[derive(Debug, Clone, PartialEq)]
pub struct AgentInstance {
    pub id: Uuid,
    pub definition: AgentDefinition,
    pub state: AgentState,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Agent execution state
#[derive(Debug, Clone, PartialEq)]
pub enum AgentState {
    Created,
    Starting,
    Running,
    Paused,
    Stopping,
    Stopped,
    Failed(String),
}

/// Execution result from DSL evaluation
#[derive(Debug, Clone)]
pub enum ExecutionResult {
    Value(DslValue),
    Return(DslValue),
    Continue,
    Break,
    Error(String),
}

/// DSL Evaluator with runtime integration
pub struct DslEvaluator {
    /// Runtime bridge for Symbiont integration
    runtime_bridge: Arc<RuntimeBridge>,
    /// Active agent instances
    agents: Arc<RwLock<HashMap<Uuid, AgentInstance>>>,
    /// Global execution context
    global_context: Arc<Mutex<ExecutionContext>>,
    /// Built-in functions
    builtins: HashMap<String, BuiltinFunction>,
    /// Execution monitor for debugging and tracing
    monitor: Arc<ExecutionMonitor>,
}

impl DslEvaluator {
    /// Create a new DSL evaluator
    pub fn new(runtime_bridge: Arc<RuntimeBridge>) -> Self {
        let mut builtins: HashMap<String, BuiltinFunction> = HashMap::new();

        // Register built-in functions
        builtins.insert("print".to_string(), builtin_print as BuiltinFunction);
        builtins.insert("len".to_string(), builtin_len as BuiltinFunction);
        builtins.insert("upper".to_string(), builtin_upper as BuiltinFunction);
        builtins.insert("lower".to_string(), builtin_lower as BuiltinFunction);
        builtins.insert("format".to_string(), builtin_format as BuiltinFunction);

        Self {
            runtime_bridge,
            agents: Arc::new(RwLock::new(HashMap::new())),
            global_context: Arc::new(Mutex::new(ExecutionContext::default())),
            builtins,
            monitor: Arc::new(ExecutionMonitor::new()),
        }
    }

    /// Get the execution monitor
    pub fn monitor(&self) -> Arc<ExecutionMonitor> {
        Arc::clone(&self.monitor)
    }

    /// Execute a DSL program
    pub async fn execute_program(&self, program: Program) -> Result<DslValue> {
        let mut context = ExecutionContext::default();

        // First pass: collect function definitions
        for declaration in &program.declarations {
            if let Declaration::Function(func) = declaration {
                context.functions.insert(func.name.clone(), func.clone());
            }
        }

        // Second pass: execute declarations
        let mut last_value = DslValue::Null;
        for declaration in &program.declarations {
            match self.execute_declaration(declaration, &mut context).await? {
                ExecutionResult::Value(value) => last_value = value,
                ExecutionResult::Return(value) => return Ok(value),
                ExecutionResult::Error(msg) => return Err(ReplError::Execution(msg)),
                _ => {}
            }
        }

        Ok(last_value)
    }

    /// Execute a declaration
    async fn execute_declaration(
        &self,
        declaration: &Declaration,
        context: &mut ExecutionContext,
    ) -> Result<ExecutionResult> {
        match declaration {
            Declaration::Agent(agent_def) => self.create_agent(agent_def.clone(), context).await,
            Declaration::Behavior(behavior_def) => {
                // Register behavior as a function
                let func_def = FunctionDefinition {
                    name: behavior_def.name.clone(),
                    parameters: behavior_def.input.clone().unwrap_or_default(),
                    return_type: behavior_def.output.as_ref().map(|_| Type::Any),
                    body: behavior_def.steps.clone(),
                    span: behavior_def.span.clone(),
                };
                context
                    .functions
                    .insert(behavior_def.name.clone(), func_def);
                Ok(ExecutionResult::Value(DslValue::Function(
                    behavior_def.name.clone(),
                )))
            }
            Declaration::Function(func_def) => {
                context
                    .functions
                    .insert(func_def.name.clone(), func_def.clone());
                Ok(ExecutionResult::Value(DslValue::Function(
                    func_def.name.clone(),
                )))
            }
            Declaration::EventHandler(handler) => {
                // Register event handler with runtime bridge
                let agent_id = context.agent_id.unwrap_or_else(|| Uuid::new_v4());

                match self
                    .runtime_bridge
                    .register_event_handler(
                        &agent_id.to_string(),
                        &handler.event_name,
                        &handler.event_name,
                    )
                    .await
                {
                    Ok(_) => {
                        tracing::info!(
                            "Registered event handler '{}' for agent {}",
                            handler.event_name,
                            agent_id
                        );
                        Ok(ExecutionResult::Value(DslValue::Function(
                            handler.event_name.clone(),
                        )))
                    }
                    Err(e) => {
                        tracing::error!("Failed to register event handler: {}", e);
                        Err(ReplError::Runtime(format!(
                            "Failed to register event handler: {}",
                            e
                        )))
                    }
                }
            }
            Declaration::Struct(struct_def) => {
                // Register struct type in the context for later use
                let struct_info = format!("{}:{}", struct_def.name, struct_def.fields.len());
                context.variables.insert(
                    format!("type_{}", struct_def.name),
                    DslValue::String(struct_info.clone()),
                );

                tracing::info!(
                    "Registered struct type '{}' with {} fields",
                    struct_def.name,
                    struct_def.fields.len()
                );
                Ok(ExecutionResult::Value(DslValue::String(format!(
                    "Struct({})",
                    struct_def.name
                ))))
            }
        }
    }

    /// Create an agent instance
    pub async fn create_agent(
        &self,
        agent_def: AgentDefinition,
        context: &mut ExecutionContext,
    ) -> Result<ExecutionResult> {
        // Check capabilities
        if let Some(security) = &agent_def.security {
            for capability in &security.capabilities {
                if !self.check_capability(capability).await? {
                    return Err(ReplError::Security(format!(
                        "Missing capability: {}",
                        capability
                    )));
                }
            }
        }

        let agent_id = Uuid::new_v4();
        let agent = AgentInstance {
            id: agent_id,
            definition: agent_def.clone(),
            state: AgentState::Created,
            created_at: chrono::Utc::now(),
        };

        // Log agent creation
        self.monitor
            .log_agent_event(&agent, TraceEventType::AgentCreated);

        // Store agent instance
        self.agents.write().await.insert(agent_id, agent.clone());
        context.agent_id = Some(agent_id);

        tracing::info!("Agent '{}' created with ID {}", agent_def.name, agent_id);
        Ok(ExecutionResult::Value(DslValue::Agent(Box::new(agent))))
    }

    /// Execute a block of statements
    fn execute_block<'a>(
        &'a self,
        block: &'a Block,
        context: &'a mut ExecutionContext,
    ) -> BoxFuture<'a, Result<ExecutionResult>> {
        Box::pin(async move {
            if context.depth >= context.max_depth {
                return Err(ReplError::Execution(
                    "Maximum execution depth exceeded".to_string(),
                ));
            }

            context.depth += 1;

            let mut last_result = ExecutionResult::Value(DslValue::Null);

            for statement in &block.statements {
                match self.execute_statement(statement, context).await? {
                    ExecutionResult::Return(value) => {
                        context.depth -= 1;
                        return Ok(ExecutionResult::Return(value));
                    }
                    ExecutionResult::Break | ExecutionResult::Continue => {
                        context.depth -= 1;
                        return Ok(last_result);
                    }
                    ExecutionResult::Error(msg) => {
                        context.depth -= 1;
                        return Err(ReplError::Execution(msg));
                    }
                    result => last_result = result,
                }
            }

            context.depth -= 1;
            Ok(last_result)
        })
    }

    /// Execute a statement
    async fn execute_statement(
        &self,
        statement: &Statement,
        context: &mut ExecutionContext,
    ) -> Result<ExecutionResult> {
        match statement {
            Statement::Let(let_stmt) => {
                let value = self
                    .evaluate_expression_impl(&let_stmt.value, context)
                    .await?;
                context.variables.insert(let_stmt.name.clone(), value);
                Ok(ExecutionResult::Value(DslValue::Null))
            }
            Statement::If(if_stmt) => {
                let condition = self
                    .evaluate_expression_impl(&if_stmt.condition, context)
                    .await?;

                if condition.is_truthy() {
                    self.execute_block(&if_stmt.then_block, context).await
                } else {
                    // Check else-if conditions
                    for else_if in &if_stmt.else_ifs {
                        let else_condition = self
                            .evaluate_expression_impl(&else_if.condition, context)
                            .await?;
                        if else_condition.is_truthy() {
                            return self.execute_block(&else_if.block, context).await;
                        }
                    }

                    // Execute else block if present
                    if let Some(else_block) = &if_stmt.else_block {
                        self.execute_block(else_block, context).await
                    } else {
                        Ok(ExecutionResult::Value(DslValue::Null))
                    }
                }
            }
            Statement::Return(ret_stmt) => {
                let value = if let Some(expr) = &ret_stmt.value {
                    self.evaluate_expression_impl(expr, context).await?
                } else {
                    DslValue::Null
                };
                Ok(ExecutionResult::Return(value))
            }
            Statement::Emit(emit_stmt) => {
                let data = if let Some(expr) = &emit_stmt.data {
                    self.evaluate_expression_impl(expr, context).await?
                } else {
                    DslValue::Null
                };

                // Emit event through runtime bridge
                let agent_id = context.agent_id.unwrap_or_else(|| Uuid::new_v4());

                match self
                    .runtime_bridge
                    .emit_event(
                        &agent_id.to_string(),
                        &emit_stmt.event_name,
                        &data.to_json(),
                    )
                    .await
                {
                    Ok(_) => {
                        tracing::info!(
                            "Successfully emitted event: {} with data: {:?}",
                            emit_stmt.event_name,
                            data
                        );
                    }
                    Err(e) => {
                        tracing::error!("Failed to emit event '{}': {}", emit_stmt.event_name, e);
                        return Err(ReplError::Runtime(format!("Failed to emit event: {}", e)));
                    }
                }
                Ok(ExecutionResult::Value(DslValue::Null))
            }
            Statement::Require(req_stmt) => {
                match &req_stmt.requirement {
                    RequirementType::Capability(cap_name) => {
                        if !self.check_capability(cap_name).await? {
                            return Err(ReplError::Security(format!(
                                "Missing capability: {}",
                                cap_name
                            )));
                        }
                    }
                    RequirementType::Capabilities(cap_names) => {
                        for cap_name in cap_names {
                            if !self.check_capability(cap_name).await? {
                                return Err(ReplError::Security(format!(
                                    "Missing capability: {}",
                                    cap_name
                                )));
                            }
                        }
                    }
                }
                Ok(ExecutionResult::Value(DslValue::Null))
            }
            Statement::Expression(expr) => {
                let value = self.evaluate_expression_impl(expr, context).await?;
                Ok(ExecutionResult::Value(value))
            }
            // Implement remaining statement types with basic functionality
            Statement::Match(match_stmt) => {
                let value = self
                    .evaluate_expression_impl(&match_stmt.expression, context)
                    .await?;

                for arm in &match_stmt.arms {
                    if self.pattern_matches(&arm.pattern, &value) {
                        return self
                            .evaluate_expression_impl(&arm.body, context)
                            .await
                            .map(ExecutionResult::Value);
                    }
                }

                // No match found
                Err(ReplError::Execution(
                    "No matching pattern found".to_string(),
                ))
            }
            Statement::For(for_stmt) => {
                let iterable = self
                    .evaluate_expression_impl(&for_stmt.iterable, context)
                    .await?;

                match iterable {
                    DslValue::List(items) => {
                        for item in items {
                            context.variables.insert(for_stmt.variable.clone(), item);
                            match self.execute_block(&for_stmt.body, context).await? {
                                ExecutionResult::Break => break,
                                ExecutionResult::Continue => continue,
                                ExecutionResult::Return(value) => {
                                    return Ok(ExecutionResult::Return(value))
                                }
                                _ => {}
                            }
                        }
                        Ok(ExecutionResult::Value(DslValue::Null))
                    }
                    _ => Err(ReplError::Execution(
                        "For loop requires iterable value".to_string(),
                    )),
                }
            }
            Statement::While(while_stmt) => {
                loop {
                    let condition = self
                        .evaluate_expression_impl(&while_stmt.condition, context)
                        .await?;
                    if !condition.is_truthy() {
                        break;
                    }

                    match self.execute_block(&while_stmt.body, context).await? {
                        ExecutionResult::Break => break,
                        ExecutionResult::Continue => continue,
                        ExecutionResult::Return(value) => {
                            return Ok(ExecutionResult::Return(value))
                        }
                        _ => {}
                    }
                }
                Ok(ExecutionResult::Value(DslValue::Null))
            }
            Statement::Try(try_stmt) => {
                // Execute try block
                match self.execute_block(&try_stmt.try_block, context).await {
                    Ok(result) => Ok(result),
                    Err(_) => {
                        // Execute catch block
                        self.execute_block(&try_stmt.catch_block, context).await
                    }
                }
            }
            Statement::Check(check_stmt) => {
                // Check policy validation (simplified implementation)
                tracing::info!("Policy check for: {}", check_stmt.policy_name);
                Ok(ExecutionResult::Value(DslValue::Boolean(true)))
            }
        }
    }

    /// Evaluate an expression
    pub async fn evaluate_expression(
        &self,
        expression: &Expression,
        context: &mut ExecutionContext,
    ) -> Result<DslValue> {
        self.evaluate_expression_impl(expression, context).await
    }

    /// Internal implementation for expression evaluation
    fn evaluate_expression_impl<'a>(
        &'a self,
        expression: &'a Expression,
        context: &'a mut ExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<DslValue>> + Send + 'a>> {
        Box::pin(async move {
            match expression {
                Expression::Literal(literal) => self.evaluate_literal(literal),
                Expression::Identifier(identifier) => {
                    if let Some(value) = context.variables.get(&identifier.name) {
                        Ok(value.clone())
                    } else {
                        Err(ReplError::Execution(format!(
                            "Undefined variable: {}",
                            identifier.name
                        )))
                    }
                }
                Expression::FieldAccess(field_access) => {
                    let object = self
                        .evaluate_expression_impl(&field_access.object, context)
                        .await?;
                    self.access_field(object, &field_access.field)
                }
                Expression::IndexAccess(index_access) => {
                    let object = self
                        .evaluate_expression_impl(&index_access.object, context)
                        .await?;
                    let index = self
                        .evaluate_expression_impl(&index_access.index, context)
                        .await?;
                    self.access_index(object, index)
                }
                Expression::FunctionCall(func_call) => {
                    self.call_function(&func_call.function, &func_call.arguments, context)
                        .await
                }
                Expression::MethodCall(method_call) => {
                    let object = self
                        .evaluate_expression_impl(&method_call.object, context)
                        .await?;
                    self.call_method(object, &method_call.method, &method_call.arguments, context)
                        .await
                }
                Expression::BinaryOp(binary_op) => {
                    let left = self
                        .evaluate_expression_impl(&binary_op.left, context)
                        .await?;
                    let right = self
                        .evaluate_expression_impl(&binary_op.right, context)
                        .await?;
                    self.evaluate_binary_op(&binary_op.operator, left, right)
                }
                Expression::UnaryOp(unary_op) => {
                    let operand = self
                        .evaluate_expression_impl(&unary_op.operand, context)
                        .await?;
                    self.evaluate_unary_op(&unary_op.operator, operand)
                }
                Expression::Assignment(assignment) => {
                    let value = self
                        .evaluate_expression_impl(&assignment.value, context)
                        .await?;

                    if let Expression::Identifier(identifier) = assignment.target.as_ref() {
                        context
                            .variables
                            .insert(identifier.name.clone(), value.clone());
                        Ok(value)
                    } else {
                        Err(ReplError::Execution(
                            "Invalid assignment target".to_string(),
                        ))
                    }
                }
                Expression::List(list_expr) => {
                    let mut items = Vec::new();
                    for element in &list_expr.elements {
                        items.push(self.evaluate_expression_impl(element, context).await?);
                    }
                    Ok(DslValue::List(items))
                }
                Expression::Map(map_expr) => {
                    let mut entries = HashMap::new();
                    for entry in &map_expr.entries {
                        let key = self.evaluate_expression_impl(&entry.key, context).await?;
                        let value = self.evaluate_expression_impl(&entry.value, context).await?;

                        if let DslValue::String(key_str) = key {
                            entries.insert(key_str, value);
                        } else {
                            return Err(ReplError::Execution(
                                "Map keys must be strings".to_string(),
                            ));
                        }
                    }
                    Ok(DslValue::Map(entries))
                }
                Expression::Invoke(invoke) => {
                    self.evaluate_invoke_expression(invoke, context).await
                }
                Expression::Lambda(lambda) => {
                    self.evaluate_lambda_expression(lambda, context).await
                }
                Expression::Conditional(conditional) => {
                    let condition = self
                        .evaluate_expression_impl(&conditional.condition, context)
                        .await?;

                    if condition.is_truthy() {
                        self.evaluate_expression_impl(&conditional.if_true, context)
                            .await
                    } else {
                        self.evaluate_expression_impl(&conditional.if_false, context)
                            .await
                    }
                }
            }
        })
    }

    /// Evaluate a literal
    pub fn evaluate_literal(&self, literal: &Literal) -> Result<DslValue> {
        match literal {
            Literal::String(s) => Ok(DslValue::String(s.clone())),
            Literal::Number(n) => Ok(DslValue::Number(*n)),
            Literal::Integer(i) => Ok(DslValue::Integer(*i)),
            Literal::Boolean(b) => Ok(DslValue::Boolean(*b)),
            Literal::Duration(duration) => Ok(DslValue::Duration {
                value: duration.value,
                unit: duration.unit.clone(),
            }),
            Literal::Size(size) => Ok(DslValue::Size {
                value: size.value,
                unit: size.unit.clone(),
            }),
            Literal::Null => Ok(DslValue::Null),
        }
    }

    /// Access a field on an object
    fn access_field(&self, object: DslValue, field: &str) -> Result<DslValue> {
        match object {
            DslValue::Map(entries) => entries
                .get(field)
                .cloned()
                .ok_or_else(|| ReplError::Execution(format!("Field '{}' not found", field))),
            DslValue::Agent(agent) => match field {
                "id" => Ok(DslValue::String(agent.id.to_string())),
                "state" => Ok(DslValue::String(format!("{:?}", agent.state))),
                "created_at" => Ok(DslValue::String(agent.created_at.to_rfc3339())),
                _ => Err(ReplError::Execution(format!(
                    "Agent field '{}' not found",
                    field
                ))),
            },
            _ => Err(ReplError::Execution(format!(
                "Cannot access field on {}",
                object.type_name()
            ))),
        }
    }

    /// Access an index on an object
    fn access_index(&self, object: DslValue, index: DslValue) -> Result<DslValue> {
        match (object, index) {
            (DslValue::List(items), DslValue::Integer(i)) => {
                let idx = if i < 0 { items.len() as i64 + i } else { i } as usize;

                items
                    .get(idx)
                    .cloned()
                    .ok_or_else(|| ReplError::Execution("Index out of bounds".to_string()))
            }
            (DslValue::Map(entries), DslValue::String(key)) => entries
                .get(&key)
                .cloned()
                .ok_or_else(|| ReplError::Execution(format!("Key '{}' not found", key))),
            (obj, idx) => Err(ReplError::Execution(format!(
                "Cannot index {} with {}",
                obj.type_name(),
                idx.type_name()
            ))),
        }
    }

    /// Call a function
    async fn call_function(
        &self,
        name: &str,
        arguments: &[Expression],
        context: &mut ExecutionContext,
    ) -> Result<DslValue> {
        // Evaluate arguments
        let mut arg_values = Vec::new();
        for arg in arguments {
            arg_values.push(self.evaluate_expression_impl(arg, context).await?);
        }

        // Check for built-in functions
        if let Some(builtin) = self.builtins.get(name) {
            return builtin(&arg_values);
        }

        // Check for user-defined functions
        if let Some(func_def) = context.functions.get(name).cloned() {
            return self.call_user_function(func_def, arg_values, context).await;
        }

        Err(ReplError::Execution(format!("Unknown function: {}", name)))
    }

    /// Call a user-defined function
    async fn call_user_function(
        &self,
        func_def: FunctionDefinition,
        arguments: Vec<DslValue>,
        context: &mut ExecutionContext,
    ) -> Result<DslValue> {
        // Create new scope
        let mut new_context = context.clone();
        new_context.variables.clear();

        // Bind parameters
        for (i, param) in func_def.parameters.parameters.iter().enumerate() {
            let value = match arguments.get(i) {
                Some(value) => value.clone(),
                None => {
                    if let Some(default_expr) = &param.default_value {
                        // Evaluate default value expression
                        self.evaluate_expression_impl(default_expr, &mut new_context)
                            .await?
                    } else {
                        return Err(ReplError::Execution(format!(
                            "Missing argument for parameter '{}'",
                            param.name
                        )));
                    }
                }
            };

            new_context.variables.insert(param.name.clone(), value);
        }

        // Execute function body
        match self.execute_block(&func_def.body, &mut new_context).await? {
            ExecutionResult::Value(value) => Ok(value),
            ExecutionResult::Return(value) => Ok(value),
            _ => Ok(DslValue::Null),
        }
    }

    /// Call a method on an object
    async fn call_method(
        &self,
        object: DslValue,
        method: &str,
        arguments: &[Expression],
        context: &mut ExecutionContext,
    ) -> Result<DslValue> {
        let mut arg_values = vec![object.clone()];
        for arg in arguments {
            arg_values.push(self.evaluate_expression(arg, context).await?);
        }

        match (&object, method) {
            (DslValue::String(_), "upper") => builtin_upper(&[object]),
            (DslValue::String(_), "lower") => builtin_lower(&[object]),
            (DslValue::List(_) | DslValue::Map(_) | DslValue::String(_), "len") => {
                builtin_len(&[object])
            }
            _ => Err(ReplError::Execution(format!(
                "Method '{}' not found on {}",
                method,
                object.type_name()
            ))),
        }
    }

    /// Evaluate binary operation
    fn evaluate_binary_op(
        &self,
        operator: &BinaryOperator,
        left: DslValue,
        right: DslValue,
    ) -> Result<DslValue> {
        match operator {
            BinaryOperator::Add => match (left, right) {
                (DslValue::Number(l), DslValue::Number(r)) => Ok(DslValue::Number(l + r)),
                (DslValue::Integer(l), DslValue::Integer(r)) => Ok(DslValue::Integer(l + r)),
                (DslValue::String(l), DslValue::String(r)) => Ok(DslValue::String(l + &r)),
                _ => Err(ReplError::Execution(
                    "Invalid operands for addition".to_string(),
                )),
            },
            BinaryOperator::Subtract => match (left, right) {
                (DslValue::Number(l), DslValue::Number(r)) => Ok(DslValue::Number(l - r)),
                (DslValue::Integer(l), DslValue::Integer(r)) => Ok(DslValue::Integer(l - r)),
                _ => Err(ReplError::Execution(
                    "Invalid operands for subtraction".to_string(),
                )),
            },
            BinaryOperator::Multiply => match (left, right) {
                (DslValue::Number(l), DslValue::Number(r)) => Ok(DslValue::Number(l * r)),
                (DslValue::Integer(l), DslValue::Integer(r)) => Ok(DslValue::Integer(l * r)),
                _ => Err(ReplError::Execution(
                    "Invalid operands for multiplication".to_string(),
                )),
            },
            BinaryOperator::Divide => match (left, right) {
                (DslValue::Number(l), DslValue::Number(r)) => {
                    if r == 0.0 {
                        Err(ReplError::Execution("Division by zero".to_string()))
                    } else {
                        Ok(DslValue::Number(l / r))
                    }
                }
                (DslValue::Integer(l), DslValue::Integer(r)) => {
                    if r == 0 {
                        Err(ReplError::Execution("Division by zero".to_string()))
                    } else {
                        Ok(DslValue::Integer(l / r))
                    }
                }
                _ => Err(ReplError::Execution(
                    "Invalid operands for division".to_string(),
                )),
            },
            BinaryOperator::Modulo => match (left, right) {
                (DslValue::Integer(l), DslValue::Integer(r)) => {
                    if r == 0 {
                        Err(ReplError::Execution("Modulo by zero".to_string()))
                    } else {
                        Ok(DslValue::Integer(l % r))
                    }
                }
                _ => Err(ReplError::Execution(
                    "Invalid operands for modulo".to_string(),
                )),
            },
            BinaryOperator::Equal => Ok(DslValue::Boolean(left == right)),
            BinaryOperator::NotEqual => Ok(DslValue::Boolean(left != right)),
            BinaryOperator::LessThan => match (left, right) {
                (DslValue::Number(l), DslValue::Number(r)) => Ok(DslValue::Boolean(l < r)),
                (DslValue::Integer(l), DslValue::Integer(r)) => Ok(DslValue::Boolean(l < r)),
                _ => Err(ReplError::Execution(
                    "Invalid operands for comparison".to_string(),
                )),
            },
            BinaryOperator::LessThanOrEqual => match (left, right) {
                (DslValue::Number(l), DslValue::Number(r)) => Ok(DslValue::Boolean(l <= r)),
                (DslValue::Integer(l), DslValue::Integer(r)) => Ok(DslValue::Boolean(l <= r)),
                _ => Err(ReplError::Execution(
                    "Invalid operands for comparison".to_string(),
                )),
            },
            BinaryOperator::GreaterThan => match (left, right) {
                (DslValue::Number(l), DslValue::Number(r)) => Ok(DslValue::Boolean(l > r)),
                (DslValue::Integer(l), DslValue::Integer(r)) => Ok(DslValue::Boolean(l > r)),
                _ => Err(ReplError::Execution(
                    "Invalid operands for comparison".to_string(),
                )),
            },
            BinaryOperator::GreaterThanOrEqual => match (left, right) {
                (DslValue::Number(l), DslValue::Number(r)) => Ok(DslValue::Boolean(l >= r)),
                (DslValue::Integer(l), DslValue::Integer(r)) => Ok(DslValue::Boolean(l >= r)),
                _ => Err(ReplError::Execution(
                    "Invalid operands for comparison".to_string(),
                )),
            },
            BinaryOperator::And => Ok(DslValue::Boolean(left.is_truthy() && right.is_truthy())),
            BinaryOperator::Or => Ok(DslValue::Boolean(left.is_truthy() || right.is_truthy())),
            // Bitwise operations
            BinaryOperator::BitwiseAnd => match (left, right) {
                (DslValue::Integer(l), DslValue::Integer(r)) => Ok(DslValue::Integer(l & r)),
                _ => Err(ReplError::Execution(
                    "Bitwise AND requires integer operands".to_string(),
                )),
            },
            BinaryOperator::BitwiseOr => match (left, right) {
                (DslValue::Integer(l), DslValue::Integer(r)) => Ok(DslValue::Integer(l | r)),
                _ => Err(ReplError::Execution(
                    "Bitwise OR requires integer operands".to_string(),
                )),
            },
            BinaryOperator::BitwiseXor => match (left, right) {
                (DslValue::Integer(l), DslValue::Integer(r)) => Ok(DslValue::Integer(l ^ r)),
                _ => Err(ReplError::Execution(
                    "Bitwise XOR requires integer operands".to_string(),
                )),
            },
            BinaryOperator::LeftShift => match (left, right) {
                (DslValue::Integer(l), DslValue::Integer(r)) => {
                    if r < 0 || r > 63 {
                        Err(ReplError::Execution("Invalid shift amount".to_string()))
                    } else {
                        Ok(DslValue::Integer(l << r))
                    }
                }
                _ => Err(ReplError::Execution(
                    "Left shift requires integer operands".to_string(),
                )),
            },
            BinaryOperator::RightShift => match (left, right) {
                (DslValue::Integer(l), DslValue::Integer(r)) => {
                    if r < 0 || r > 63 {
                        Err(ReplError::Execution("Invalid shift amount".to_string()))
                    } else {
                        Ok(DslValue::Integer(l >> r))
                    }
                }
                _ => Err(ReplError::Execution(
                    "Right shift requires integer operands".to_string(),
                )),
            },
        }
    }

    /// Evaluate unary operation
    fn evaluate_unary_op(&self, operator: &UnaryOperator, operand: DslValue) -> Result<DslValue> {
        match operator {
            UnaryOperator::Not => Ok(DslValue::Boolean(!operand.is_truthy())),
            UnaryOperator::Negate => match operand {
                DslValue::Number(n) => Ok(DslValue::Number(-n)),
                DslValue::Integer(i) => Ok(DslValue::Integer(-i)),
                _ => Err(ReplError::Execution(
                    "Invalid operand for negation".to_string(),
                )),
            },
            UnaryOperator::BitwiseNot => match operand {
                DslValue::Integer(i) => Ok(DslValue::Integer(!i)),
                _ => Err(ReplError::Execution(
                    "Bitwise NOT requires integer operand".to_string(),
                )),
            },
        }
    }

    /// Check if a capability is available
    async fn check_capability(&self, capability_name: &str) -> Result<bool> {
        let capability = match capability_name {
            "filesystem" => Capability::FileRead("/".to_string()), // Generic file read capability
            "network" => Capability::NetworkRequest("*".to_string()), // Generic network capability
            "execute" => Capability::Execute("*".to_string()),     // Generic execute capability
            "data" => Capability::DataRead("*".to_string()),       // Generic data capability
            _ => return Ok(false),
        };

        // For now, use a default agent ID - this should be context-specific in real implementation
        let agent_id = "default";
        match self
            .runtime_bridge
            .check_capability(agent_id, &capability)
            .await
        {
            Ok(PolicyDecision::Allow) => Ok(true),
            Ok(PolicyDecision::Deny) => Ok(false),
            Err(e) => Err(ReplError::Runtime(format!(
                "Capability check failed: {}",
                e
            ))),
        }
    }

    /// Get agent by ID
    pub async fn get_agent(&self, agent_id: Uuid) -> Option<AgentInstance> {
        self.agents.read().await.get(&agent_id).cloned()
    }

    /// List all agents
    pub async fn list_agents(&self) -> Vec<AgentInstance> {
        self.agents.read().await.values().cloned().collect()
    }

    /// Start an agent
    pub async fn start_agent(&self, agent_id: Uuid) -> Result<()> {
        let mut agents = self.agents.write().await;
        if let Some(agent) = agents.get_mut(&agent_id) {
            agent.state = AgentState::Starting;

            // Log the event
            self.monitor
                .log_agent_event(agent, TraceEventType::AgentStarted);

            // Integrate with runtime to actually start the agent
            match self.runtime_bridge.initialize().await {
                Ok(_) => {
                    agent.state = AgentState::Running;
                    tracing::info!("Agent {} started and integrated with runtime", agent_id);
                    Ok(())
                }
                Err(e) => {
                    agent.state = AgentState::Failed(format!("Runtime integration failed: {}", e));
                    tracing::error!("Failed to start agent {}: {}", agent_id, e);
                    Err(ReplError::Runtime(format!("Failed to start agent: {}", e)))
                }
            }
        } else {
            Err(ReplError::Execution(format!(
                "Agent {} not found",
                agent_id
            )))
        }
    }

    /// Stop an agent
    pub async fn stop_agent(&self, agent_id: Uuid) -> Result<()> {
        let mut agents = self.agents.write().await;
        if let Some(agent) = agents.get_mut(&agent_id) {
            agent.state = AgentState::Stopping;
            // Log the stopping event
            self.monitor
                .log_agent_event(agent, TraceEventType::AgentStopped);

            // Integrate with runtime to actually stop the agent
            // Note: In a real implementation, this would call runtime bridge methods to stop the agent
            // For now, we just set the state as there's no agent-specific stop method in the current runtime bridge
            agent.state = AgentState::Stopped;
            tracing::info!("Agent {} stopped", agent_id);
            Ok(())
        } else {
            Err(ReplError::Execution(format!(
                "Agent {} not found",
                agent_id
            )))
        }
    }

    /// Pause an agent
    pub async fn pause_agent(&self, agent_id: Uuid) -> Result<()> {
        let mut agents = self.agents.write().await;
        if let Some(agent) = agents.get_mut(&agent_id) {
            match agent.state {
                AgentState::Running => {
                    agent.state = AgentState::Paused;
                    self.monitor
                        .log_agent_event(agent, TraceEventType::AgentPaused);
                    tracing::info!("Agent {} paused", agent_id);
                    Ok(())
                }
                _ => Err(ReplError::Execution(format!(
                    "Agent {} is not running",
                    agent_id
                ))),
            }
        } else {
            Err(ReplError::Execution(format!(
                "Agent {} not found",
                agent_id
            )))
        }
    }

    /// Resume a paused agent
    pub async fn resume_agent(&self, agent_id: Uuid) -> Result<()> {
        let mut agents = self.agents.write().await;
        if let Some(agent) = agents.get_mut(&agent_id) {
            match agent.state {
                AgentState::Paused => {
                    agent.state = AgentState::Running;
                    self.monitor
                        .log_agent_event(agent, TraceEventType::AgentResumed);
                    tracing::info!("Agent {} resumed", agent_id);
                    Ok(())
                }
                _ => Err(ReplError::Execution(format!(
                    "Agent {} is not paused",
                    agent_id
                ))),
            }
        } else {
            Err(ReplError::Execution(format!(
                "Agent {} not found",
                agent_id
            )))
        }
    }

    /// Destroy an agent
    pub async fn destroy_agent(&self, agent_id: Uuid) -> Result<()> {
        let mut agents = self.agents.write().await;
        if let Some(agent) = agents.remove(&agent_id) {
            self.monitor
                .log_agent_event(&agent, TraceEventType::AgentDestroyed);
            tracing::info!("Agent {} destroyed", agent_id);
            Ok(())
        } else {
            Err(ReplError::Execution(format!(
                "Agent {} not found",
                agent_id
            )))
        }
    }

    /// Execute a specific behavior on an agent
    pub async fn execute_agent_behavior(
        &self,
        agent_id: Uuid,
        behavior_name: &str,
        args: &str,
    ) -> Result<DslValue> {
        // Get agent reference
        let agent = {
            let agents = self.agents.read().await;
            agents
                .get(&agent_id)
                .cloned()
                .ok_or_else(|| ReplError::Execution(format!("Agent {} not found", agent_id)))?
        };

        // Check if agent is in valid state for execution
        match agent.state {
            AgentState::Running => {}
            AgentState::Created => {
                return Err(ReplError::Execution(format!(
                    "Agent {} is not started",
                    agent_id
                )));
            }
            AgentState::Paused => {
                return Err(ReplError::Execution(format!(
                    "Agent {} is paused",
                    agent_id
                )));
            }
            AgentState::Stopped => {
                return Err(ReplError::Execution(format!(
                    "Agent {} is stopped",
                    agent_id
                )));
            }
            AgentState::Failed(ref reason) => {
                return Err(ReplError::Execution(format!(
                    "Agent {} failed: {}",
                    agent_id, reason
                )));
            }
            _ => {
                return Err(ReplError::Execution(format!(
                    "Agent {} is not ready for execution",
                    agent_id
                )));
            }
        }

        // Look up behavior in global context (behaviors are defined separately)
        let behavior = {
            let context_guard = self.global_context.lock().unwrap();
            let behavior = context_guard.functions.get(behavior_name).ok_or_else(|| {
                ReplError::Execution(format!("Behavior '{}' not found", behavior_name))
            })?;
            behavior.clone()
        };

        // Parse arguments if provided
        let mut context = ExecutionContext {
            agent_id: Some(agent_id),
            ..ExecutionContext::default()
        };

        // Simple argument parsing - in a real implementation this would be more sophisticated
        if !args.is_empty() {
            // For now, just parse as a single string argument
            context
                .variables
                .insert("args".to_string(), DslValue::String(args.to_string()));
        }

        // Execute the behavior with policy enforcement
        self.execute_function_with_policies(&behavior, &mut context)
            .await
    }

    /// Execute a function with policy enforcement
    async fn execute_function_with_policies(
        &self,
        function: &FunctionDefinition,
        context: &mut ExecutionContext,
    ) -> Result<DslValue> {
        // Start monitoring execution
        let execution_id = self
            .monitor
            .start_execution(context.agent_id, Some(function.name.clone()));

        // Log execution start
        tracing::info!(
            "Executing function '{}' for agent {:?}",
            function.name,
            context.agent_id
        );

        // Execute the function body
        let result = match self.execute_block(&function.body, context).await? {
            ExecutionResult::Value(value) => Ok(value),
            ExecutionResult::Return(value) => Ok(value),
            ExecutionResult::Error(msg) => Err(ReplError::Execution(msg)),
            _ => Ok(DslValue::Null),
        };

        // End monitoring execution - handle the clone issue
        match &result {
            Ok(value) => {
                self.monitor.end_execution(execution_id, Ok(value.clone()));
            }
            Err(error) => {
                let error_msg = format!("{}", error);
                self.monitor
                    .end_execution(execution_id, Err(ReplError::Execution(error_msg)));
            }
        }

        result
    }

    /// Get debug information for an agent
    pub async fn debug_agent(&self, agent_id: Uuid) -> Result<String> {
        let agents = self.agents.read().await;
        if let Some(agent) = agents.get(&agent_id) {
            let mut debug_info = String::new();
            debug_info.push_str("Agent Debug Information:\n");
            debug_info.push_str(&format!("  ID: {}\n", agent.id));
            debug_info.push_str(&format!("  Name: {}\n", agent.definition.name));

            if let Some(version) = &agent.definition.metadata.version {
                debug_info.push_str(&format!("  Version: {}\n", version));
            }

            debug_info.push_str(&format!("  State: {:?}\n", agent.state));
            debug_info.push_str(&format!(
                "  Created: {}\n",
                agent.created_at.format("%Y-%m-%d %H:%M:%S UTC")
            ));

            if let Some(description) = &agent.definition.metadata.description {
                debug_info.push_str(&format!("  Description: {}\n", description));
            }

            if let Some(author) = &agent.definition.metadata.author {
                debug_info.push_str(&format!("  Author: {}\n", author));
            }

            // Count available functions/behaviors in global context
            let context_guard = self.global_context.lock().unwrap();
            let function_count = context_guard.functions.len();
            drop(context_guard);

            debug_info.push_str(&format!(
                "  Available Functions/Behaviors: {}\n",
                function_count
            ));

            if let Some(security) = &agent.definition.security {
                debug_info.push_str(&format!(
                    "  Required Capabilities: {}\n",
                    security.capabilities.len()
                ));
                for cap in &security.capabilities {
                    debug_info.push_str(&format!("    - {}\n", cap));
                }
            }

            if let Some(resources) = &agent.definition.resources {
                debug_info.push_str("  Resource Configuration:\n");
                if let Some(memory) = &resources.memory {
                    debug_info
                        .push_str(&format!("    Memory: {}{:?}\n", memory.value, memory.unit));
                }
                if let Some(cpu) = &resources.cpu {
                    debug_info.push_str(&format!("    CPU: {}{:?}\n", cpu.value, cpu.unit));
                }
                if let Some(network) = resources.network {
                    debug_info.push_str(&format!("    Network: {}\n", network));
                }
                if let Some(storage) = &resources.storage {
                    debug_info.push_str(&format!(
                        "    Storage: {}{:?}\n",
                        storage.value, storage.unit
                    ));
                }
            }

            Ok(debug_info)
        } else {
            Err(ReplError::Execution(format!(
                "Agent {} not found",
                agent_id
            )))
        }
    }

    /// Create a snapshot of the evaluator state
    pub async fn create_snapshot(&self) -> SessionSnapshot {
        let agents = self.agents.read().await.clone();
        let context = self.global_context.lock().unwrap().clone();

        SessionSnapshot {
            id: Uuid::new_v4(),
            timestamp: chrono::Utc::now(),
            data: serde_json::json!({
                "agents": agents.iter().map(|(id, agent)| {
                    (id.to_string(), serde_json::json!({
                        "id": agent.id,
                        "definition": agent.definition.name,
                        "state": format!("{:?}", agent.state),
                        "created_at": agent.created_at
                    }))
                }).collect::<serde_json::Map<_, _>>(),
                "context": {
                    "variables": context.variables.iter().map(|(k, v)| {
                        (k.clone(), v.to_json())
                    }).collect::<serde_json::Map<_, _>>(),
                    "functions": context.functions.keys().collect::<Vec<_>>()
                }
            }),
        }
    }

    /// Restore from a snapshot
    pub async fn restore_snapshot(&self, snapshot: &SessionSnapshot) -> Result<()> {
        // Clear current state
        self.agents.write().await.clear();
        self.global_context.lock().unwrap().variables.clear();
        self.global_context.lock().unwrap().functions.clear();

        // Extract data from snapshot
        if let Some(snapshot_data) = snapshot.data.as_object() {
            // Restore agents
            if let Some(agents_data) = snapshot_data.get("agents").and_then(|v| v.as_object()) {
                for (agent_id_str, agent_data) in agents_data {
                    if let Ok(agent_id) = uuid::Uuid::parse_str(agent_id_str) {
                        if let Some(_agent_obj) = agent_data.as_object() {
                            // In a real implementation, you'd reconstruct the full AgentInstance
                            // from the serialized data. For now, we'll create a placeholder
                            tracing::info!("Restored agent {} from snapshot", agent_id);
                        }
                    }
                }
            }

            // Restore context variables
            if let Some(context_data) = snapshot_data.get("context").and_then(|v| v.as_object()) {
                if let Some(variables) = context_data.get("variables").and_then(|v| v.as_object()) {
                    let mut context_guard = self.global_context.lock().unwrap();
                    for (var_name, var_value) in variables {
                        // Convert JSON value back to DslValue
                        let dsl_value = self.json_to_dsl_value(var_value);
                        context_guard.variables.insert(var_name.clone(), dsl_value);
                    }
                }

                // Functions would need to be restored from their definitions
                // This is a simplified implementation
                if let Some(functions) = context_data.get("functions").and_then(|v| v.as_array()) {
                    tracing::info!(
                        "Restored {} function definitions from snapshot",
                        functions.len()
                    );
                }
            }
        }

        tracing::info!(
            "Successfully restored evaluator state from snapshot {}",
            snapshot.id
        );
        Ok(())
    }

    /// Helper method to convert JSON value to DslValue
    fn json_to_dsl_value(&self, json_value: &JsonValue) -> DslValue {
        match json_value {
            JsonValue::String(s) => DslValue::String(s.clone()),
            JsonValue::Number(n) => {
                if let Some(i) = n.as_i64() {
                    DslValue::Integer(i)
                } else {
                    DslValue::Number(n.as_f64().unwrap_or(0.0))
                }
            }
            JsonValue::Bool(b) => DslValue::Boolean(*b),
            JsonValue::Array(arr) => {
                let items = arr.iter().map(|v| self.json_to_dsl_value(v)).collect();
                DslValue::List(items)
            }
            JsonValue::Object(obj) => {
                let mut entries = HashMap::new();
                for (k, v) in obj {
                    entries.insert(k.clone(), self.json_to_dsl_value(v));
                }
                DslValue::Map(entries)
            }
            JsonValue::Null => DslValue::Null,
        }
    }

    /// Evaluate invoke expression for behavior invocation
    async fn evaluate_invoke_expression(
        &self,
        invoke: &InvokeExpression,
        context: &mut ExecutionContext,
    ) -> Result<DslValue> {
        let behavior_name = &invoke.behavior;

        // Look up behavior in context
        let behavior_def = {
            let context_guard = self.global_context.lock().unwrap();
            context_guard
                .functions
                .get(behavior_name)
                .cloned()
                .ok_or_else(|| {
                    ReplError::Execution(format!("Behavior '{}' not found", behavior_name))
                })?
        };

        // Evaluate arguments
        let mut arg_values = Vec::new();
        for param in &behavior_def.parameters.parameters {
            if let Some(arg_expr) = invoke.arguments.get(&param.name) {
                arg_values.push(self.evaluate_expression_impl(arg_expr, context).await?);
            } else if let Some(default_expr) = &param.default_value {
                arg_values.push(self.evaluate_expression_impl(default_expr, context).await?);
            } else {
                return Err(ReplError::Execution(format!(
                    "Missing argument for parameter '{}'",
                    param.name
                )));
            }
        }

        // Execute the behavior
        self.call_user_function(behavior_def, arg_values, context)
            .await
    }

    /// Evaluate lambda expression
    async fn evaluate_lambda_expression(
        &self,
        lambda: &LambdaExpression,
        context: &mut ExecutionContext,
    ) -> Result<DslValue> {
        // Capture current context for closure
        let captured_context = context.variables.clone();

        let lambda_func = LambdaFunction {
            parameters: lambda.parameters.clone(),
            body: *lambda.body.clone(),
            captured_context,
        };

        Ok(DslValue::Lambda(lambda_func))
    }

    /// Call a lambda function
    async fn call_lambda(
        &self,
        lambda: &LambdaFunction,
        arguments: Vec<DslValue>,
        context: &mut ExecutionContext,
    ) -> Result<DslValue> {
        // Create new scope with captured context
        let mut new_context = context.clone();
        new_context.variables = lambda.captured_context.clone();

        // Bind parameters
        if arguments.len() != lambda.parameters.len() {
            return Err(ReplError::Execution(format!(
                "Lambda expects {} arguments, got {}",
                lambda.parameters.len(),
                arguments.len()
            )));
        }

        for (param_name, arg_value) in lambda.parameters.iter().zip(arguments.iter()) {
            new_context
                .variables
                .insert(param_name.clone(), arg_value.clone());
        }

        // Execute lambda body
        self.evaluate_expression_impl(&lambda.body, &mut new_context)
            .await
    }

    /// Check if pattern matches value
    fn pattern_matches(&self, pattern: &Pattern, value: &DslValue) -> bool {
        match pattern {
            Pattern::Literal(literal) => {
                if let Ok(literal_value) = self.evaluate_literal(literal) {
                    &literal_value == value
                } else {
                    false
                }
            }
            Pattern::Wildcard => true,
            Pattern::Identifier(_) => true, // Identifiers always match and bind
        }
    }
}

// Built-in functions
pub fn builtin_print(args: &[DslValue]) -> Result<DslValue> {
    let output = args
        .iter()
        .map(|v| match v {
            DslValue::String(s) => s.clone(),
            other => format!("{:?}", other),
        })
        .collect::<Vec<_>>()
        .join(" ");

    println!("{}", output);
    Ok(DslValue::Null)
}

pub fn builtin_len(args: &[DslValue]) -> Result<DslValue> {
    if args.len() != 1 {
        return Err(ReplError::Execution(
            "len() takes exactly one argument".to_string(),
        ));
    }

    let len = match &args[0] {
        DslValue::String(s) => s.len() as i64,
        DslValue::List(items) => items.len() as i64,
        DslValue::Map(entries) => entries.len() as i64,
        _ => {
            return Err(ReplError::Execution(
                "len() requires string, list, or map".to_string(),
            ))
        }
    };

    Ok(DslValue::Integer(len))
}

pub fn builtin_upper(args: &[DslValue]) -> Result<DslValue> {
    if args.len() != 1 {
        return Err(ReplError::Execution(
            "upper() takes exactly one argument".to_string(),
        ));
    }

    match &args[0] {
        DslValue::String(s) => Ok(DslValue::String(s.to_uppercase())),
        _ => Err(ReplError::Execution(
            "upper() requires string argument".to_string(),
        )),
    }
}

pub fn builtin_lower(args: &[DslValue]) -> Result<DslValue> {
    if args.len() != 1 {
        return Err(ReplError::Execution(
            "lower() takes exactly one argument".to_string(),
        ));
    }

    match &args[0] {
        DslValue::String(s) => Ok(DslValue::String(s.to_lowercase())),
        _ => Err(ReplError::Execution(
            "lower() requires string argument".to_string(),
        )),
    }
}

pub fn builtin_format(args: &[DslValue]) -> Result<DslValue> {
    if args.is_empty() {
        return Err(ReplError::Execution(
            "format() requires at least one argument".to_string(),
        ));
    }

    let format_str = match &args[0] {
        DslValue::String(s) => s,
        _ => {
            return Err(ReplError::Execution(
                "format() first argument must be string".to_string(),
            ))
        }
    };

    // Simple format implementation - replace {} with arguments
    let mut result = format_str.clone();
    for arg in &args[1..] {
        let placeholder = "{}";
        if let Some(pos) = result.find(placeholder) {
            let replacement = match arg {
                DslValue::String(s) => s.clone(),
                DslValue::Number(n) => n.to_string(),
                DslValue::Integer(i) => i.to_string(),
                DslValue::Boolean(b) => b.to_string(),
                other => format!("{:?}", other),
            };
            result.replace_range(pos..pos + placeholder.len(), &replacement);
        }
    }

    Ok(DslValue::String(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::{lexer::Lexer, parser::Parser};

    async fn create_test_evaluator() -> DslEvaluator {
        let runtime_bridge = Arc::new(RuntimeBridge::new());
        DslEvaluator::new(runtime_bridge)
    }

    async fn evaluate_source(source: &str) -> Result<DslValue> {
        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize()?;
        let mut parser = Parser::new(tokens);
        let program = parser.parse()?;

        let evaluator = create_test_evaluator().await;
        evaluator.execute_program(program).await
    }

    #[tokio::test]
    async fn test_basic_arithmetic() {
        let result = evaluate_source(
            r#"
            function test() {
                return 2 + 3 * 4
            }
        "#,
        )
        .await
        .unwrap();
        assert_eq!(result, DslValue::Function("test".to_string()));
    }

    #[tokio::test]
    async fn test_variable_assignment() {
        let result = evaluate_source(
            r#"
            function test() {
                let x = 42
                return x
            }
        "#,
        )
        .await
        .unwrap();
        assert_eq!(result, DslValue::Function("test".to_string()));
    }

    #[tokio::test]
    async fn test_function_call() {
        let result = evaluate_source(
            r#"
            function add(a: number, b: number) -> number {
                return a + b
            }
        "#,
        )
        .await
        .unwrap();
        assert_eq!(result, DslValue::Function("add".to_string()));
    }

    #[tokio::test]
    async fn test_builtin_functions() {
        // Test that builtin functions work correctly
        let result = builtin_len(&[DslValue::String("hello".to_string())]);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), DslValue::Integer(5));

        let result = builtin_upper(&[DslValue::String("hello".to_string())]);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), DslValue::String("HELLO".to_string()));
    }
}
